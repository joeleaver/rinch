//! Compile tests for issues #391 and #394 — an `rsx!` binding the author uses
//! is not reported unused.
//!
//! `rsx!` builds several closures over one user-written pattern: an `if let`'s
//! condition (`matches!`) and its branch, a `for` loop's key closure and its
//! view closure, a `match`'s discriminant and its arms. A name one of them uses
//! is bound, unused, in the others, and rustc used to report it at the author's
//! own pattern — a warning they could not act on, and a hard error under
//! `-D warnings`.
//!
//! This file denies `unused_variables`, so **if it compiles, the tests pass**.
//! Every function below used to fail to compile (#391: the `if let` ones; #394:
//! the `for` ones).
//!
//! The opposite half — a binding the author genuinely never uses still warns —
//! cannot be a compile-pass test; it is pinned against the expansion in
//! `dom_codegen::control_flow`'s unit tests
//! (`a_for_item_used_nowhere_is_left_to_warn`,
//! `an_if_let_binding_is_reported_by_its_branch_alone`).

#![deny(unused_variables)]
#![allow(dead_code)]

use rinch::prelude::*;

// ============================================================
// #391 — `if let`
// ============================================================

#[component]
fn if_let_binding_used_in_branch() -> NodeHandle {
    let maybe: Option<String> = Some("hello".into());
    rsx! {
        div {
            if let Some(name) = maybe.clone() {
                p { {name} }
            } else {
                span { "none" }
            }
        }
    }
}

#[component]
fn if_let_binding_used_without_else() -> NodeHandle {
    let pair: Signal<Option<(u32, String)>> = Signal::new(Some((1, "a".into())));
    rsx! {
        div {
            if let Some((n, label)) = pair.get() {
                p { {format!("{n}: {label}")} }
            }
        }
    }
}

#[component]
fn else_if_let_binding_used() -> NodeHandle {
    let flag = Signal::new(false);
    let maybe: Signal<Option<u32>> = Signal::new(Some(3));
    rsx! {
        div {
            if flag.get() {
                p { "flag" }
            } else if let Some(count) = maybe.get() {
                p { {count.to_string()} }
            }
        }
    }
}

// ============================================================
// #394 — `for`
// ============================================================

#[component]
fn for_item_used_only_as_key() -> NodeHandle {
    let ids = vec![1u32, 2, 3];
    rsx! {
        div {
            for id in ids.clone() {
                span { key: id, "row" }
            }
        }
    }
}

/// A tuple pattern whose halves are split between the two closures: `id` is
/// the key closure's alone, `name` the view closure's alone.
#[component]
fn for_tuple_halves_split_between_key_and_view() -> NodeHandle {
    let rows = vec![(1u32, "a".to_string()), (2, "b".to_string())];
    rsx! {
        div {
            for (id, name) in rows.clone() {
                span { key: id, {name} }
            }
        }
    }
}

/// The key reads the item through a leading `let`, and the view never names it.
#[component]
fn for_item_used_only_through_a_key_let() -> NodeHandle {
    let ids = vec![10u32, 20];
    rsx! {
        div {
            for id in ids.clone() {
                let k = id * 2;
                span { key: k, "row" }
            }
        }
    }
}

/// The ordinary shape stays warning-free too: the item in both closures.
#[component]
fn for_item_used_in_key_and_view() -> NodeHandle {
    let ids = vec![1u32, 2];
    rsx! {
        div {
            for id in ids.clone() {
                span { key: id, {id.to_string()} }
            }
        }
    }
}

// ============================================================
// `match` — the same shape, one arm binding used in its body
// ============================================================

#[component]
fn match_arm_binding_used() -> NodeHandle {
    let maybe: Signal<Option<u32>> = Signal::new(Some(3));
    rsx! {
        div {
            match maybe.get() {
                Some(n) => p { {n.to_string()} },
                None => p { "none" },
            }
        }
    }
}

#[test]
fn the_file_compiles_under_deny_unused_variables() {}
