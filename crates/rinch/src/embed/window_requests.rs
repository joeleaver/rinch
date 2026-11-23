//! The window requests an embedded [`RinchContext`](super::RinchContext)'s
//! code makes (#2801): kept for its host, which reads them with
//! [`RinchContext::take_window_requests`](super::RinchContext::take_window_requests).
//!
//! A context has no window of rinch's: its "current window" is its host's
//! surface. So a request its code makes (`set_window_title`, show and hide,
//! `rinch::exit`, …) is the host's to honour, and never a desktop window's,
//! even when a desktop runtime runs in the same process.

use std::cell::RefCell;

use crate::shell::rinch_runtime::{EventFor, RinchNativeEvent};
use crate::window_request::WindowRequest;

rinch_core::doc_keyed! {
    /// Each live context's requests not yet taken, by its document. A context
    /// joins at its mount and leaves with its document (the sweep, #2797).
    static REQUESTS: rinch_core::DocMap<Vec<WindowRequest>> = rinch_core::DocMap::new();
}

thread_local! {
    /// What a context's code asks for while the context is being mounted,
    /// before its document's key is in [`REQUESTS`]: `Some` only during
    /// [`mounting`]'s `build`.
    static MOUNTING: RefCell<Option<Vec<WindowRequest>>> = const { RefCell::new(None) };
}

/// Run `build` (a context's mount) with what its code asks for kept, then
/// make the document `build` answers a context, holding those requests.
pub(super) fn mounting(build: impl FnOnce() -> u64) {
    let outer = MOUNTING.with(|m| m.borrow_mut().replace(Vec::new()));
    let doc = build();
    let early = MOUNTING
        .with(|m| std::mem::replace(&mut *m.borrow_mut(), outer))
        .unwrap_or_default();
    if doc != 0 {
        REQUESTS.with_mut(|r| r.insert(doc, early));
    }
}

/// Take context `doc`'s requests.
pub(super) fn take(doc: u64) -> Vec<WindowRequest> {
    REQUESTS
        .with_mut(|r| r.get_mut(doc).map(std::mem::take))
        .unwrap_or_default()
}

/// Keep `request` for the context whose code is asking: the document `to`
/// names, else the one dispatching. `Err(request)` when that is no context.
fn keep(to: EventFor, request: WindowRequest) -> Result<(), WindowRequest> {
    let doc = match to {
        EventFor::Sender(Some(doc)) => Some(doc),
        EventFor::Sender(None) => rinch_core::current_dispatching_doc(),
        // A window a handle names is a window of rinch's, never a context.
        EventFor::Window(_) => return Err(request),
    };
    let mut request = Some(request);
    if let Some(doc) = doc {
        REQUESTS.with_mut(|r| {
            if let Some(list) = r.get_mut(doc) {
                list.extend(request.take());
            }
        });
    }
    let Some(request) = request else {
        return Ok(());
    };
    // A context being mounted: its document has no key here yet.
    MOUNTING.with(|m| match m.borrow_mut().as_mut() {
        Some(early) => {
            early.push(request);
            Ok(())
        }
        None => Err(request),
    })
}

/// The native event `send_native_event_for` hands on: `None` when it is a
/// window request of a context's code, now kept for its host. Anything that
/// is no window request (a re-render, a debug command) goes on.
pub(crate) fn capture(to: EventFor, event: RinchNativeEvent) -> Option<RinchNativeEvent> {
    // No context on this thread: nothing to look up, nothing to clone.
    let none = REQUESTS.try_with(|r| r.is_empty()).unwrap_or(true)
        && MOUNTING.try_with(|m| m.borrow().is_none()).unwrap_or(true);
    if none {
        return Some(event);
    }
    let Some(request) = WindowRequest::from_native(event.clone()) else {
        return Some(event);
    };
    keep(to, request).err().map(|_| event)
}

/// Keep a title a context's code set (`set_window_title`, which stores a
/// title rather than sending it). Whether `doc` is a context's.
pub(crate) fn capture_title(doc: Option<u64>, title: &str) -> bool {
    keep(
        EventFor::Sender(doc),
        WindowRequest::SetTitle(title.to_string()),
    )
    .is_ok()
}
