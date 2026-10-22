//! `copy_text_then` / `copy_html_then` on native (#1780): the callback is called
//! once, with the clipboard worker's answer, after the write. The in-memory test backend stands in
//! for the system clipboard; its own process because that backend is
//! process-wide.
//!
//!     cargo test -p rinch --features clipboard --test clipboard_copy_then

#![cfg(feature = "clipboard")]

use std::sync::mpsc;
use std::time::Duration;

use rinch::clipboard::{copy_html_then, copy_text_then, paste_html, paste_text, ClipboardResult};

#[test]
fn a_copy_reports_the_workers_answer_after_the_write() {
    rinch_clipboard::use_in_memory_clipboard();

    // Text: the callback runs (positive control: the channel receives) with Ok.
    let (tx, rx) = mpsc::channel::<ClipboardResult<()>>();
    copy_text_then("copied 1780", move |result| {
        let _ = tx.send(result);
    });
    let result = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("copy_text_then called back");
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(paste_text().ok().as_deref(), Some("copied 1780"));

    // Html, with its plain-text fallback.
    let (tx, rx) = mpsc::channel::<ClipboardResult<()>>();
    copy_html_then("<b>bold</b>", Some("bold"), move |result| {
        let _ = tx.send(result);
    });
    let result = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("copy_html_then called back");
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(paste_html().ok().as_deref(), Some("<b>bold</b>"));
    assert_eq!(paste_text().ok().as_deref(), Some("bold"));
}
