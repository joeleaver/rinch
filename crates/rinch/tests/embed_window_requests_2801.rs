//! Issue #2801: in a process that embeds rinch through `RinchContext` and
//! never makes a desktop runtime, a window request is not queued where
//! nothing drains it, and what a context's code asks of its window reaches
//! its host through `RinchContext::take_window_requests`.
//!
//! Its own test binary, so no desktop runtime is ever made in the process.
//! Needs the `embed` and `windowing` features (`desktop` includes it):
//!     cargo test -p rinch --features desktop,embed --test embed_window_requests_2801

#![cfg(all(feature = "embed", feature = "windowing"))]

use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

use rinch::embed::{RinchContext, RinchContextConfig, WindowRequest};
use rinch::platform::{MouseButton, PlatformEvent};
use rinch::prelude::*;
use rinch::shell::rinch_runtime::__native_events_queued as queued;

// `RinchContext::new` registers its thread as the main thread process-wide,
// so every test runs on one worker thread.
type Job = Box<dyn FnOnce() + Send>;

fn on_ui_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    static SENDER: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    let sender = SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("rinch-test-ui".into())
            .spawn(move || {
                for job in rx {
                    job();
                }
            })
            .expect("spawn ui worker");
        Mutex::new(tx)
    });
    let (result_tx, result_rx) = mpsc::channel();
    let job: Job = Box::new(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        let _ = result_tx.send(result);
    });
    sender
        .lock()
        .expect("ui sender lock")
        .send(job)
        .expect("ui worker alive");
    match result_rx.recv().expect("ui worker responded") {
        Ok(v) => v,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

fn cfg() -> RinchContextConfig {
    RinchContextConfig {
        width: 800,
        height: 600,
        scale_factor: 1.0,
        theme: None,
        fonts: Vec::new(),
    }
}

/// A context whose content asks for the title `title` as it is built, and
/// whose one button (the whole 800x600 surface) asks to be hidden, sized and
/// titled `title` again, then to exit.
fn context(title: &'static str) -> RinchContext {
    RinchContext::new(cfg(), move |__scope: &mut RenderScope| {
        rinch::window::set_window_title(title);
        rsx! {
            button {
                style: "display: block; width: 800px; height: 600px; border: 0; padding: 0;",
                onclick: move || {
                    rinch::windows::hide_current_window();
                    rinch::windows::set_current_window_size(320.0, 240.0);
                    rinch::window::set_window_title(title);
                    rinch::exit();
                },
            }
        }
    })
}

fn click(ctx: &mut RinchContext) {
    ctx.update(&[
        PlatformEvent::MouseMove { x: 400.0, y: 300.0 },
        PlatformEvent::MouseDown {
            x: 400.0,
            y: 300.0,
            button: MouseButton::Left,
        },
        PlatformEvent::MouseUp {
            x: 400.0,
            y: 300.0,
            button: MouseButton::Left,
        },
    ]);
}

/// Requests made outside any context's code, with no desktop runtime in the
/// process, are not queued: nothing would ever drain them.
#[test]
fn an_embed_only_process_queues_no_request() {
    let left = on_ui_thread(|| {
        let _ctx = context("unused");
        for i in 0..1000 {
            rinch::window::set_window_title(&format!("title {i}"));
            rinch::windows::hide_current_window();
            rinch::windows::show_current_window();
        }
        queued()
    });
    assert_eq!(left, 0, "requests queued where nothing drains them");
}

/// What a context's code asks of its window, from its first line and from a
/// handler, is its host's to read, oldest first, and taken once.
#[test]
fn a_contexts_requests_reach_its_host() {
    let (mounted, clicked, again, left) = on_ui_thread(|| {
        let mut ctx = context("Mounted");
        ctx.update(&[]);
        let mounted = ctx.take_window_requests();
        click(&mut ctx);
        let clicked = ctx.take_window_requests();
        let again = ctx.take_window_requests();
        (mounted, clicked, again, queued())
    });
    assert_eq!(mounted, [WindowRequest::SetTitle("Mounted".into())]);
    assert_eq!(
        clicked,
        [
            WindowRequest::Hide,
            WindowRequest::SetSize(320.0, 240.0),
            WindowRequest::SetTitle("Mounted".into()),
            WindowRequest::Exit,
        ]
    );
    assert_eq!(again, [], "taken once");
    assert_eq!(left, 0, "nothing queued for a desktop runtime");
}

/// Two contexts on one thread each read their own requests only.
#[test]
fn two_contexts_do_not_see_each_others_requests() {
    let (a, b) = on_ui_thread(|| {
        let mut a = context("A");
        let mut b = context("B");
        let _ = (a.take_window_requests(), b.take_window_requests());
        click(&mut a);
        (a.take_window_requests(), b.take_window_requests())
    });
    assert_eq!(a.len(), 4, "A's click: {a:?}");
    assert_eq!(b, [], "B saw A's requests");
}

/// Many rounds of requests from a context's code, each taken by its host,
/// leave nothing behind and queue nothing.
#[test]
fn a_contexts_requests_do_not_accumulate_once_taken() {
    let (sizes, left) = on_ui_thread(|| {
        let mut ctx = context("Loop");
        let _mounted = ctx.take_window_requests();
        let mut sizes = Vec::new();
        for _ in 0..100 {
            click(&mut ctx);
            sizes.push(ctx.take_window_requests().len());
        }
        (sizes, queued())
    });
    assert!(sizes.iter().all(|&n| n == 4), "{sizes:?}");
    assert_eq!(left, 0);
}
