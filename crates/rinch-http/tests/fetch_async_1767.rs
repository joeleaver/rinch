#![cfg(not(target_arch = "wasm32"))]
//! `fetch_async` (issue #1767): sequential requests awaited in one task, which
//! resumes on the main thread after each. The test plays the runtime as
//! `native_roundtrip.rs` does — this thread registered as main, a queueing
//! dispatcher pumped by hand — and is one `#[test]` for the same reason (the
//! registrations are process-global).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use rinch_core::{register_main_thread, set_cross_thread_dispatcher, spawn_local};
use rinch_http::{Request, fetch_async};

type Job = Box<dyn FnOnce() + Send>;
static QUEUE: OnceLock<Mutex<VecDeque<Job>>> = OnceLock::new();

fn queue() -> &'static Mutex<VecDeque<Job>> {
    QUEUE.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn dispatcher(f: Job) {
    queue().lock().unwrap().push_back(f);
}

/// Serve `bodies` in order, one connection each (`Connection: close`).
fn serve(bodies: &'static [&'static str]) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for body in bodies {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://{addr}/")
}

#[test]
fn awaited_requests_resume_the_task_on_the_main_thread() {
    register_main_thread();
    set_cross_thread_dispatcher(dispatcher);
    let main = std::thread::current().id();

    let url = serve(&["first", "second"]);
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let l = log.clone();
    let handle = spawn_local(async move {
        let first = fetch_async(Request::get(url.clone())).await.unwrap();
        assert_eq!(std::thread::current().id(), main);
        l.borrow_mut().push(first.text());
        let second = fetch_async(Request::get(url)).await.unwrap();
        assert_eq!(std::thread::current().id(), main);
        l.borrow_mut().push(second.text());
    });

    let deadline = Instant::now() + Duration::from_secs(10);
    while !handle.is_finished() {
        assert!(
            Instant::now() < deadline,
            "task did not finish: {:?}",
            log.borrow()
        );
        let jobs: Vec<Job> = queue().lock().unwrap().drain(..).collect();
        for job in jobs {
            job();
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(*log.borrow(), ["first", "second"]);
}
