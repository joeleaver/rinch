//! Behavioural tests for the `rsx!` `for` loop's duplicate-key handling
//! (issue #185).
//!
//! `rinch-core`'s own tests mirror the macro's fallback key by hand — the macro
//! expands to `rinch::core::…` paths and cannot be invoked from inside
//! `rinch-core`. These run the real expansion, so they also pin the half the
//! mirror cannot see: that the macro passes the right
//! [`rinch::core::KeySource`] for each shape of `for`.

use rinch::prelude::*;
use rinch_core::dom::DomDocument;
use rinch_core::dom::mock::MockDomDocument;
use std::cell::RefCell;
use std::rc::Rc;

/// Rows of the `for` loop under `root`, in sibling order. The `<!-- for -->`
/// marker is a comment node (`node_type() == Some(8)`) and is filtered out.
fn rows(root: &NodeHandle) -> Vec<String> {
    root.children()
        .into_iter()
        .filter(|c| c.node_type() != Some(8))
        .map(|c| c.text_content().unwrap_or_default())
        .collect()
}

/// Mount a component into a headless document.
///
/// Returns the document and the scope alongside the root: a [`NodeHandle`] holds
/// only a `Weak` to the document, so letting either go out of scope would leave
/// every handle answering as if the tree were empty.
fn mount(
    build: impl FnOnce(&mut RenderScope) -> NodeHandle,
) -> (Rc<RefCell<MockDomDocument>>, RenderScope, NodeHandle) {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let root = build(&mut scope);
    (doc, scope, root)
}

#[component]
fn unkeyed_tags() -> NodeHandle {
    let tags = vec!["rust".to_string(), "rust".to_string(), "gui".to_string()];
    rsx! {
        div {
            for tag in tags.clone() {
                span { {tag.clone()} }
            }
        }
    }
}

/// A `key:`-less `for` over `Debug`-equal items renders every row.
///
/// The macro fabricates the key from `format!("{:?}", item)`, so a repeated
/// value would collide. That is not a user error, so the macro passes
/// `KeySource::Fallback` and the fabricated key is uniquified by occurrence
/// ordinal rather than the row being dropped.
#[test]
fn an_unkeyed_rsx_for_renders_every_row_even_when_two_items_are_equal() {
    let (_doc, _scope, root) = mount(unkeyed_tags);
    assert_eq!(rows(&root), ["rust", "rust", "gui"]);
}

#[component]
fn keyed_collision() -> NodeHandle {
    // Both rows claim the same id — a mistake in the caller's key.
    let entries = vec![("a", "First"), ("a", "Second")];
    rsx! {
        div {
            for entry in entries.clone() {
                span { key: entry.0, {entry.1.to_string()} }
            }
        }
    }
}

/// An explicit `key:` that repeats is a user error, and keeps the React rule:
/// the repeat is dropped, first occurrence wins.
#[test]
fn an_explicitly_keyed_rsx_for_still_drops_a_repeated_key() {
    let (_doc, _scope, root) = mount(keyed_collision);
    assert_eq!(rows(&root), ["First"]);
}
