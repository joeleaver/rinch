//! An embed `RinchContext` dropped is its document's teardown: what it left in
//! the registries keyed by its document goes with it, including what its own
//! cleanups registered as it went (issue #2797).
//!
//! Requires the `embed` (or `gpu`) feature:
//!     cargo test -p rinch --features embed --test embed_doc_forget_2797

#![cfg(any(feature = "gpu-renderer", feature = "embed"))]

use std::cell::Cell;
use std::rc::Rc;

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::prelude::*;

fn cfg() -> RinchContextConfig {
    RinchContextConfig {
        width: 200,
        height: 200,
        scale_factor: 1.0,
        theme: None,
        fonts: Vec::new(),
    }
}

/// The app keys document `doc` has registered.
fn app_keys(doc: u64) -> usize {
    rinch_core::doc_entries(doc)
        .into_iter()
        .find(|(name, _)| *name == "rinch_core::events::app_key::LISTENERS")
        .map_or(0, |(_, n)| n)
}

/// A context whose content registers an app key in its ordinary code,
/// unowned, so that only the document's sweep removes it, and, when
/// `in_cleanup`, another in its `on_cleanup`, which no scope owns either. Answers the context and its document's key.
fn context(in_cleanup: bool) -> (RinchContext, u64) {
    let doc = Rc::new(Cell::new(0));
    let seen = doc.clone();
    let ctx = RinchContext::new(cfg(), move |__scope: &mut RenderScope| {
        seen.set(
            rinch_core::current_dispatching_doc().expect("a context's mount is its document's"),
        );
        // Unowned: no scope's disposal removes it; only the sweep does.
        let _listener = rinch_core::reactive::unowned(|| rinch_core::on_app_key(|_| false));
        if in_cleanup {
            __scope.on_cleanup(|| {
                let _listener = rinch_core::on_app_key(|_| false);
            });
        }
        rsx! { div { "embedded" } }
    });
    let doc = doc.get();
    assert_ne!(doc, 0);
    (ctx, doc)
}

#[test]
fn a_dropped_contexts_app_key_is_forgotten() {
    let (ctx, doc) = context(false);
    assert_eq!(app_keys(doc), 1, "control: registered");
    drop(ctx);
    assert_eq!(app_keys(doc), 0);
}

#[test]
fn an_app_key_a_dropped_contexts_cleanup_registers_is_forgotten() {
    // Every listener on this thread, whatever its document: a cleanup run as
    // no document's code (after the sweep, in the fields' drop) would register
    // one under none, which no count by document sees.
    let before = rinch_core::events::app_key_listener_count();
    let (ctx, doc) = context(true);
    assert_eq!(app_keys(doc), 1, "control: the cleanup has not run yet");
    drop(ctx);
    assert_eq!(app_keys(doc), 0);
    assert_eq!(
        rinch_core::events::app_key_listener_count(),
        before,
        "nothing the context registered stays, under its document or none"
    );
}

#[test]
fn another_contexts_entries_survive_a_drop() {
    let (first, gone) = context(true);
    let (second, kept) = context(true);
    drop(first);
    assert_eq!(app_keys(gone), 0);
    assert_eq!(app_keys(kept), 1, "the other context's own");
    drop(second);
    assert_eq!(app_keys(kept), 0);
}
