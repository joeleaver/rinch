//! A plain `<input>` paste through a real host (issue #328): an embedded
//! `RinchContext`, a registered main thread and its queue-only dispatcher, and
//! the clipboard worker (the in-memory test backend).
//!
//! The read runs on the worker; its completion is sent to the app's deferred
//! work inbox and wakes the host through `run_on_main_thread`, and the host
//! runs it at the top of its next `update`. This pins that last step — the one
//! `RinchContext::update` has to take — which the `--lib` fixtures cannot
//! (`app::input_paste_async_tests` call `run_deferred_work` themselves, with no
//! main thread registered).
//!
//! Its own process because registering a main thread is process-wide, and
//! every body runs on one long-lived "UI" thread, as in
//! `tests/embed_cross_thread.rs`.
//!
//!     cargo test -p rinch --features embed,clipboard --test embed_input_paste

#![cfg(all(any(feature = "gpu", feature = "embed"), feature = "clipboard"))]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::platform::{KeyCode, KeyRepeat, Modifiers, MouseButton, PlatformEvent};
use rinch::prelude::*;

type Job = Box<dyn FnOnce() + Send>;

fn on_ui_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    static SENDER: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    let sender = SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("rinch-test-ui".into())
            .spawn(move || {
                rinch_clipboard::use_in_memory_clipboard();
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

fn key(key: KeyCode, modifiers: Modifiers) -> [PlatformEvent; 2] {
    [
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: None,
            modifiers,
            repeat: KeyRepeat::Unknown,
        },
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers,
        },
    ]
}

fn primary() -> Modifiers {
    Modifiers {
        ctrl: !cfg!(target_os = "macos"),
        meta: cfg!(target_os = "macos"),
        ..Default::default()
    }
}

#[test]
fn a_ctrl_v_in_an_embedded_input_lands_on_a_later_update() {
    let (before, after, updates) = on_ui_thread(|| {
        rinch_clipboard::copy_text("XYZ").unwrap();
        let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let log_in = log.clone();
        let mut ctx = RinchContext::new(cfg(), move |__scope: &mut RenderScope| {
            let log = log_in.clone();
            rsx! {
                input {
                    style: "display: block; width: 300px; height: 30px; padding: 0; margin: 0; \
                            font-size: 16px; line-height: 20px",
                    value: "hello world",
                    oninput: move |v: String| log.borrow_mut().push(v),
                }
            }
        });
        let button = MouseButton::Left;
        ctx.update(&[
            PlatformEvent::MouseDown {
                x: 2.0,
                y: 15.0,
                button,
            },
            PlatformEvent::MouseUp {
                x: 2.0,
                y: 15.0,
                button,
            },
        ]);
        ctx.update(&key(KeyCode::End, Modifiers::default()));
        ctx.update(&key(KeyCode::KeyV, primary()));
        let before = log.borrow().clone();

        // The worker answers; each `update` drains the wake and runs the
        // deferred completion.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut updates = 0;
        while log.borrow().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
            ctx.update(&[]);
            updates += 1;
        }
        let after = log.borrow().clone();
        (before, after, updates)
    });
    assert!(
        before.is_empty(),
        "Ctrl+V returned before the read: {before:?}"
    );
    assert_eq!(
        after,
        vec!["hello worldXYZ".to_string()],
        "the paste landed through `update` ({updates} updates)"
    );
}
