//! Compile tests for issue #223 — a branch may capture what its sibling captures.
//!
//! `show_dom`'s `then_fn` and `else_fn` are both plain `Fn`, so **both** branches
//! are constructed no matter which condition holds; the same is true of every
//! `match` arm, and of every closure a repeatable body (a branch, a `for` view,
//! an effect) builds on each run. Without a shadow clone at each of those
//! construction sites a non-`Copy` value referenced twice is a genuine E0382 /
//! E0507 — correct Rust, but a rule the author of an `if` has no reason to
//! expect, since only one branch ever renders.
//!
//! Every function here fails to compile without the auto-clone shadows in
//! `dom_codegen::captures`, except the four under "Over-cloning guards" — those
//! are the opposite assertion: values that must **not** be cloned, so the fix
//! cannot turn working code into an error. They pass before and after.
//!
//! If this file compiles, the tests pass.

// `unused_variables`: an `rsx!` `if let` binding is reported unused because the
// generated condition's `matches!` doesn't use it — issue #391, unrelated.
#![allow(dead_code, unused_imports, unused_variables)]

use rinch::prelude::*;

/// `Clone` but not `Copy`: moving it into one closure leaves the next one with
/// nothing.
#[derive(Clone)]
struct Row {
    label: String,
}

/// Not `Clone` at all — the over-cloning guard. Any auto-clone that fires on
/// this type turns a compiling program into an E0599.
struct NotClone {
    label: String,
}

impl NotClone {
    fn label(&self) -> String {
        self.label.clone()
    }
}

// ============================================================
// The issue's headline shape: sibling branches of one if/else
// ============================================================

#[component]
fn if_else_branches_share_a_string() -> NodeHandle {
    let row = Row {
        label: "hello".into(),
    };
    rsx! {
        div {
            if row.label.is_empty() {
                p { {row.label.clone()} }
            } else {
                span { {row.label.clone()} }
            }
        }
    }
}

#[component]
fn else_if_chain_shares_a_string() -> NodeHandle {
    let row = Row {
        label: "hello".into(),
    };
    rsx! {
        div {
            if row.label.is_empty() {
                p { {row.label.clone()} }
            } else if row.label.len() > 3 {
                span { {row.label.clone()} }
            } else {
                em { {row.label.clone()} }
            }
        }
    }
}

#[component]
fn if_let_scrutinee_is_reused_by_its_own_branch() -> NodeHandle {
    // The `if let` scrutinee is emitted twice: `matches!(..)` in the condition
    // closure and a re-destructure in the then closure.
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

// ============================================================
// The nesting layers under one branch
// ============================================================

#[component]
fn reactive_attribute_inside_a_branch() -> NodeHandle {
    let row = Row {
        label: "hello".into(),
    };
    let shown = Signal::new(true);
    rsx! {
        div {
            if shown.get() {
                p { title: {move || row.label.clone()}, "row" }
            }
        }
    }
}

#[component]
fn reactive_text_inside_a_branch() -> NodeHandle {
    let row = Row {
        label: "hello".into(),
    };
    let shown = Signal::new(true);
    rsx! {
        div {
            if shown.get() {
                p { {move || row.label.clone()} }
            }
        }
    }
}

#[component]
fn reactive_style_shorthand_inside_a_branch() -> NodeHandle {
    let width = String::from("120px");
    let shown = Signal::new(true);
    rsx! {
        div {
            if shown.get() {
                p { w: {move || width.clone()}, "row" }
            }
        }
    }
}

#[component]
fn for_body_inside_a_branch_captures_an_ancestor_string() -> NodeHandle {
    // Every iteration's own handler competes with every other iteration's for
    // the ancestor-scope `row`, the same way two sibling branches do.
    let row = Row {
        label: "hello".into(),
    };
    let items = Signal::new(vec![1u32, 2, 3]);
    rsx! {
        div {
            if !items.get().is_empty() {
                for id in items.get() {
                    button {
                        key: id,
                        onclick: move || { let _ = (row.label.clone(), id); },
                        "row"
                    }
                }
            }
        }
    }
}

#[component]
fn a_for_iterates_and_renders_the_same_value() -> NodeHandle {
    // `names` is read by the collection closure AND by the per-item view.
    let names: Vec<String> = vec!["a".into(), "b".into()];
    rsx! {
        div {
            for name in names.clone() {
                p { key: name.clone(), {format!("{} of {}", name, names.len())} }
            }
        }
    }
}

// ============================================================
// match — the same defect, plus a scrutinee re-emitted per arm
// ============================================================

#[component]
fn match_arms_share_the_scrutinee() -> NodeHandle {
    let row = Row { label: "b".into() };
    rsx! {
        div {
            match row.label.as_str() {
                "a" => p { {row.label.clone()} },
                _ => span { {row.label.clone()} },
            }
        }
    }
}

// ============================================================
// Over-cloning guards — these compile today and must keep compiling
// ============================================================

/// A non-`Clone` value used by exactly one branch is moved, not cloned.
#[component]
fn a_single_branch_may_use_a_non_clone_value() -> NodeHandle {
    let thing = NotClone { label: "x".into() };
    let shown = Signal::new(true);
    rsx! {
        div {
            if shown.get() {
                p { {thing.label()} }
            } else {
                p { "none" }
            }
        }
    }
}

/// A value bound *inside* a repeatable body is a fresh local on every run, so
/// the one closure that consumes it may still move it.
#[component]
fn a_body_local_non_clone_value_is_not_cloned() -> NodeHandle {
    let shown = Signal::new(true);
    rsx! {
        div {
            if shown.get() {
                let thing = NotClone { label: "x".into() };
                button { onclick: move || { let _ = thing.label(); }, "go" }
            }
        }
    }
}

/// Same, one level deeper: the per-item local belongs to the `for` view body.
#[component]
fn a_for_body_local_non_clone_value_is_not_cloned() -> NodeHandle {
    let items = Signal::new(vec![1u32, 2]);
    rsx! {
        div {
            for id in items.get() {
                let thing = NotClone { label: id.to_string() };
                button { key: id, onclick: move || { let _ = thing.label(); }, "go" }
            }
        }
    }
}

/// A real `for` statement inside a branch binds its own loop variable — the
/// capture scan must not mistake it for a value from the enclosing scope.
#[component]
fn a_rust_for_loop_in_a_branch_binds_its_own_variable() -> NodeHandle {
    let shown = Signal::new(true);
    rsx! {
        div {
            if shown.get() {
                let total = { let mut sum = 0u32; for n in 0..4u32 { sum += n; } sum };
                p { {total.to_string()} }
            }
        }
    }
}

// ============================================================
// Components — the render closure is rebuilt on every prop change
// ============================================================

/// A reactive component prop rebuilds its closure inside the render closure
/// that owns the captured value, on every render.
#[component]
fn a_reactive_component_prop_closure_is_rebuilt_per_render() -> NodeHandle {
    let variant = String::from("filled");
    let active = Signal::new(false);
    rsx! {
        Button {
            variant: {move || { let _ = active.get(); variant.clone() }},
            "Click"
        }
    }
}

/// The whole render closure is built inside the branch, so what it names is
/// moved out of the branch's captures on every render.
#[component]
fn a_reactive_component_inside_a_branch() -> NodeHandle {
    let variant = String::from("filled");
    let shown = Signal::new(true);
    let active = Signal::new(false);
    rsx! {
        div {
            if shown.get() {
                Button {
                    variant: {move || { let _ = active.get(); variant.clone() }},
                    "Click"
                }
            }
        }
    }
}

/// A component's reactive `style:`/`class:` are effects like any other.
#[component]
fn a_component_style_closure_inside_a_branch() -> NodeHandle {
    let width = String::from("width: 40px");
    let shown = Signal::new(true);
    rsx! {
        div {
            if shown.get() {
                Button { style: {move || width.clone()}, "Click" }
            }
        }
    }
}

/// The example printed in `docs/src/guide/rsx-syntax.md` under "Capturing the
/// same value in more than one branch" — including the sibling that uses the
/// value *after* the `if`, which only works because each branch took a copy.
#[component]
fn the_guides_worked_example_compiles() -> NodeHandle {
    let label = String::from("hello");
    rsx! {
        div {
            if label.is_empty() {
                p { "empty" }
            } else {
                p { {label.clone()} }
            }
            Text { {label.clone()} }
        }
    }
}

fn main() {}
