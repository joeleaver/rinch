#![cfg(not(target_arch = "wasm32"))]
//! Native round-trip test: a throwaway `TcpListener` serves canned HTTP/1.1
//! responses, and `rinch_http::fetch` must deliver them to the callback on the
//! **main** thread.
//!
//! Native `fetch` parks the callback main-thread-side and dispatches only the
//! result back via `rinch_core`'s cross-thread dispatcher, so the test simulates
//! the rinch runtime: it registers this thread as main, installs a queueing
//! dispatcher, and pumps that queue to drive completion. It is a single `#[test]`
//! on a single thread so the main-thread registration and the per-thread pending
//! registry stay consistent (parallel test threads would not).
//!
//! The key assertion is that a 404-with-body comes back as
//! `Ok(Response { status: 404, .. })` — non-2xx is NOT an error.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use rinch_core::{register_main_thread, set_cross_thread_dispatcher};
use rinch_http::{HttpError, Request, Response, fetch};

/// A closure the (simulated) runtime must run on the main thread.
type Job = Box<dyn FnOnce() + Send>;

/// Closures the (simulated) runtime must run on the main thread.
static QUEUE: OnceLock<Mutex<VecDeque<Job>>> = OnceLock::new();

fn queue() -> &'static Mutex<VecDeque<Job>> {
    QUEUE.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn dispatcher(f: Job) {
    queue().lock().unwrap().push_back(f);
}

/// Spawn a one-shot server on 127.0.0.1:0 that writes `response` to the first
/// connection, and return the URL to hit it.
fn serve_once(response: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    format!("http://{addr}/")
}

/// Spawn a one-shot server that reads the **full** request (headers + body, via
/// Content-Length) and echoes it back verbatim as the 200 body, so a test can
/// assert the request line / headers / body the client actually sent. Reading in a
/// loop (rather than a single `read`) is required: a POST's headers and body can
/// arrive in separate TCP segments, and ureq keep-alive means no EOF to wait on.
fn serve_echo() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            // Safety net so a malformed request can never hang the test thread.
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut data: Vec<u8> = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                // Once headers are in, wait for the declared body to arrive too.
                if let Some(hdr_end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&data[..hdr_end]);
                    let content_len = headers
                        .lines()
                        .find_map(|l| {
                            let l = l.to_ascii_lowercase();
                            l.strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if data.len() >= hdr_end + 4 + content_len {
                        break;
                    }
                }
                match stream.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => data.extend_from_slice(&buf[..n]),
                    Err(_) => break,
                }
            }
            let req = String::from_utf8_lossy(&data).to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                req.len(),
                req
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });
    format!("http://{addr}/echo")
}

fn response_str(status_line: &str, body: &str) -> &'static str {
    let raw = format!(
        "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status_line,
        body.len(),
        body
    );
    // Leak to get a 'static str for the one-shot server closure.
    Box::leak(raw.into_boxed_str())
}

/// Pump the dispatch queue (running queued main-thread closures on this thread)
/// until `rx` yields a value or the timeout elapses.
fn pump_recv(
    rx: &mpsc::Receiver<Result<Response, HttpError>>,
    timeout: Duration,
) -> Result<Response, HttpError> {
    let start = Instant::now();
    loop {
        while let Some(job) = queue().lock().unwrap().pop_front() {
            job();
        }
        if let Ok(v) = rx.try_recv() {
            return v;
        }
        assert!(
            start.elapsed() < timeout,
            "callback not delivered within timeout"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Serve two sequential connections on one port: the first response carries
/// `set_cookie`, the second echoes back whatever `Cookie:` header the client sent.
/// Returns the base URL.
fn serve_cookie_pair(set_cookie: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    std::thread::spawn(move || {
        // 1st request: hand out the cookie.
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            let resp = format!(
                "HTTP/1.1 200 OK\r\nSet-Cookie: {}\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                set_cookie
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
        // 2nd request: echo the Cookie header the agent sent back to us.
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let echoed = req
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("cookie:"))
                .unwrap_or("cookie: <none>")
                .to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                echoed.len(),
                echoed
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });
    format!("http://{addr}")
}

/// The shared agent must carry ureq's cookie jar across requests — without this a
/// cookie-session login can never authenticate a later request on native.
///
/// Not a separate `#[test]`: each test fn runs on its own thread, but the parked
/// callbacks live in a *main-thread-local* registry while the dispatcher queue is
/// global — so a second test's pump could drain (and drop) this one's completion.
/// Everything shares one test fn on one thread.
fn cookie_jar_persists_across_requests() {
    let base = serve_cookie_pair("sid=abc123; Path=/");

    // 1st request receives the Set-Cookie.
    let (tx1, rx1) = mpsc::channel();
    fetch(Request::get(format!("{base}/login")), move |res| {
        tx1.send(res).unwrap()
    });
    let first = pump_recv(&rx1, Duration::from_secs(5)).expect("first request ok");
    assert_eq!(first.status, 200);

    // 2nd request must send it back — proving the jar lives on the shared agent.
    let (tx2, rx2) = mpsc::channel();
    fetch(Request::get(format!("{base}/me")), move |res| {
        tx2.send(res).unwrap()
    });
    let second = pump_recv(&rx2, Duration::from_secs(5)).expect("second request ok");
    assert_eq!(second.status, 200);
    assert!(
        second.text().contains("sid=abc123"),
        "cookie was not resent on the follow-up request (got {:?}) — the agent's \
         jar is not persisting, so session auth would break on native",
        second.text()
    );
}

#[test]
fn native_roundtrip_delivers_on_main_thread() {
    // Simulate the rinch runtime: this thread is "main"; cross-thread dispatch
    // queues closures for us to pump.
    register_main_thread();
    set_cross_thread_dispatcher(dispatcher);

    // 200 with a JSON body.
    {
        let body = r#"{"hello":"world"}"#;
        let url = serve_once(response_str("200 OK", body));
        let (tx, rx) = mpsc::channel();
        fetch(Request::get(url), move |res| tx.send(res).unwrap());
        let resp = pump_recv(&rx, Duration::from_secs(5)).expect("transport ok");
        assert_eq!(resp.status, 200);
        assert!(resp.ok());
        assert_eq!(resp.text(), body);
        assert_eq!(resp.header("Content-Type"), Some("application/json"));
    }

    // 404 with a body must come back as Ok(Response), not Err.
    {
        let body = r#"{"error":"not found"}"#;
        let url = serve_once(response_str("404 Not Found", body));
        let (tx, rx) = mpsc::channel();
        fetch(Request::get(url), move |res| tx.send(res).unwrap());
        let resp = pump_recv(&rx, Duration::from_secs(5)).expect("404 must be Ok(Response)");
        assert_eq!(resp.status, 404);
        assert!(!resp.ok());
        assert_eq!(resp.text(), body);
    }

    // POST with a body and a custom header: the request line, header and body must
    // all reach the server (the echo server reflects the raw request back).
    {
        let url = serve_echo();
        let (tx, rx) = mpsc::channel();
        fetch(
            Request::post(url)
                .header("X-Test", "hello")
                .body_str(r#"{"k":"v"}"#),
            move |res| tx.send(res).unwrap(),
        );
        let resp = pump_recv(&rx, Duration::from_secs(5)).expect("post transport ok");
        assert_eq!(resp.status, 200);
        let echoed = resp.text();
        assert!(
            echoed.starts_with("POST "),
            "method reached server: {echoed:?}"
        );
        assert!(
            echoed.to_ascii_lowercase().contains("x-test: hello"),
            "custom header reached server: {echoed:?}"
        );
        assert!(
            echoed.contains(r#"{"k":"v"}"#),
            "request body reached server: {echoed:?}"
        );
    }

    // Session cookies must survive across requests (shared agent = shared jar).
    cookie_jar_persists_across_requests();
}
