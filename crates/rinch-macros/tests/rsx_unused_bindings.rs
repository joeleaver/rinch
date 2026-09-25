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
//! The opposite half — a binding the author genuinely never uses must **still**
//! warn, or the fix is a blanket `#[allow]` that hides real mistakes — is the
//! "Genuinely unused" section at the bottom. Each of those functions carries
//! `#[expect(unused_variables)]`, and this file denies
//! `unfulfilled_lint_expectations`, so it compiles only if rustc does report
//! the unused binding.

#![deny(unused_variables, unfulfilled_lint_expectations)]
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

/// A leading `let` that shares its name with a field the key reads: the key
/// closure keeps `let id = row.id;` (its filter goes by spelling) but reads
/// `row.id`, not `id` — only the view's click handler reads `id`.
#[component]
fn for_let_kept_by_the_key_but_read_by_the_view() -> NodeHandle {
    #[derive(Clone, PartialEq)]
    struct Row {
        id: u32,
    }
    let rows = Signal::new(vec![Row { id: 1 }]);
    rsx! {
        div {
            for row in rows.get() {
                let id = row.id;
                button {
                    key: row.id,
                    onclick: move || rows.update(|r| r.retain(|x| x.id != id)),
                    "delete"
                }
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

// ============================================================
// Genuinely unused — each of these must still warn
// ============================================================

#[expect(unused_variables)]
#[component]
fn if_let_binding_the_branch_never_reads() -> NodeHandle {
    let maybe: Option<String> = Some("hello".into());
    rsx! {
        div {
            if let Some(name) = maybe.clone() {
                p { "has a name" }
            }
        }
    }
}

/// `name` is read by neither the key nor the view.
#[expect(unused_variables)]
#[component]
fn for_item_half_read_by_neither_closure() -> NodeHandle {
    let rows = vec![(1u32, "a".to_string())];
    rsx! {
        div {
            for (id, name) in rows.clone() {
                span { key: id, "row" }
            }
        }
    }
}

/// `id` is read by neither closure, but the key spells it — as a *path
/// segment*, `keys::id`. The view's acknowledgement of what the key reads is a
/// token scan, so it takes `id` as read and stays silent; the key closure is
/// what must still report it. (A key closure acknowledging every name would
/// pass the fixture above, since the view still reports `name` there, and
/// fail this.)
#[expect(unused_variables)]
#[component]
fn for_item_named_by_the_key_only_as_a_path_segment() -> NodeHandle {
    mod keys {
        pub fn id(label: &str) -> String {
            label.to_string()
        }
    }
    let rows = vec![(1u32, "a".to_string())];
    rsx! {
        div {
            for (id, label) in rows.clone() {
                span { key: keys::id(label), "row" }
            }
        }
    }
}

/// The same `let id = row.id;` as above, but read by neither closure. The key
/// closure keeps the `let` (its filter goes by spelling) and the view binds it
/// too; neither may acknowledge it on the other's behalf — `row.id` is a field,
/// not a read of `id`.
#[expect(unused_variables)]
#[component]
fn for_let_kept_by_the_key_and_read_by_neither() -> NodeHandle {
    #[derive(Clone, PartialEq)]
    struct Row {
        id: u32,
    }
    let rows = vec![Row { id: 1 }];
    rsx! {
        div {
            for row in rows.clone() {
                let id = row.id;
                span { key: row.id, "row" }
            }
        }
    }
}

/// A leading `let` read by neither closure, whose name the key spells as a path
/// segment: the view acknowledges it (a token scan takes `keys::id` as a read
/// of `id`), so the key closure must report it — which it can only do if its
/// view's own `let id = …` is not mistaken for a read.
#[expect(unused_variables)]
#[component]
fn for_let_the_key_spells_as_a_path_and_nobody_reads() -> NodeHandle {
    mod keys {
        pub fn id(label: &str) -> String {
            label.to_string()
        }
    }
    let rows = vec!["a".to_string()];
    rsx! {
        div {
            for label in rows.clone() {
                let id = label.len();
                span { key: keys::id(label), "row" }
            }
        }
    }
}

/// No `key:` — the fabricated `format!("{:?}", tag)` key reads the item, but
/// the author never does, so the view still reports it.
#[expect(unused_variables)]
#[component]
fn for_item_never_named_with_a_fallback_key() -> NodeHandle {
    let tags = vec!["a".to_string()];
    rsx! {
        div {
            for tag in tags.clone() {
                span { "row" }
            }
        }
    }
}

/// A leading `let` neither the key nor the view reads.
#[expect(unused_variables)]
#[component]
fn for_body_let_read_by_neither_closure() -> NodeHandle {
    let ids = vec![1u32];
    rsx! {
        div {
            for id in ids.clone() {
                let doubled = id * 2;
                span { key: id, "row" }
            }
        }
    }
}

#[expect(unused_variables)]
#[component]
fn match_arm_binding_the_arm_never_reads() -> NodeHandle {
    let maybe: Signal<Option<u32>> = Signal::new(Some(3));
    rsx! {
        div {
            match maybe.get() {
                Some(n) => p { "some" },
                None => p { "none" },
            }
        }
    }
}

#[test]
fn the_file_compiles_under_deny_unused_variables() {}
