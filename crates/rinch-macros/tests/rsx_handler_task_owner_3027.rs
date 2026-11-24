//! A task spawned from a component's handler belongs to the component that
//! wrote the handler, not to one render of the component it was handed to
//! (issue #3027).
//!
//! The shape from the report: a `Button` with a reactive prop
//! (`loading: {|| busy.get()}`) whose `onclick` sets `busy` and spawns a task.
//! The Button re-renders through `reactive_component_dom` when the click's
//! batch ends, and its per-render scope is disposed. The `Callback` used to
//! carry no owner, so the handler ran under the owner its *registration*
//! captured — that per-render scope — and the task was adopted by it and
//! aborted before its first poll.
//!
//! The rule the fix gives every app-callback kind: it runs under the owner that
//! was ambient where it was **created** (the caller's rsx!), so the task lives
//! as long as that component and is still cancelled when it unmounts. Every
//! test here samples both sides: the re-render must not cancel, the unmount (or
//! the row's removal) must.
//!
//! The browser twin is `rinch-web/tests/handler_task_owner_3027.rs`.

#[path = "support/task_owner.rs"]
mod task_owner;

use rinch::prelude::*;
use rinch_core::task::spawn_local;
use std::cell::RefCell;
use std::rc::Rc;
use task_owner::*;

/// The handler ids of every `.rinch-button` under `node`, in tree order.
fn button_handlers(node: &NodeHandle) -> Vec<rinch_core::events::EventHandlerId> {
    click_handlers(node, &|n| has_class(n, "rinch-button"))
}

fn click_only_button(node: &NodeHandle) {
    let ids = button_handlers(node);
    assert_eq!(ids.len(), 1, "one button in the tree");
    assert!(
        rinch_core::events::dispatch_event(ids[0]),
        "the click reached a handler"
    );
}

/// The report's component: a Button whose `loading` prop follows `busy`, and
/// whose click sets `busy` and spawns a request.
fn send_button(__scope: &mut RenderScope, busy: Signal<bool>, trace: Trace) -> NodeHandle {
    rsx! {
        Button {
            loading: {|| busy.get()},
            onclick: {
                let trace = trace.clone();
                move || {
                    busy.set(true);
                    spawn_local(Pending(trace.clone()));
                }
            },
            "Send"
        }
    }
}

/// (a) The report's repro: the click's own write re-renders the Button, and the
/// task it spawned must still get its first poll.
#[test]
fn a_task_from_a_click_survives_the_rerender_that_click_causes() {
    let busy = Signal::new(false);
    let trace = Trace::default();
    let t = trace.clone();
    let mut mounted = mount(move |__scope| {
        let div = __scope.create_element("div");
        let button = send_button(__scope, busy, t);
        div.append_child(&button);
        div
    });

    click_only_button(&mounted.root);
    assert!(busy.get(), "positive control: the handler ran");
    assert_eq!(
        button_handlers(&mounted.root).len(),
        1,
        "the Button re-rendered in place"
    );
    run_polls();
    assert!(
        !trace.dropped.get(),
        "the Button's re-render must not drop the task its click spawned"
    );
    assert_eq!(trace.polls.get(), 1, "the task got its first poll");

    // (b) Unmounting the component that wrote the handler still ends it.
    mounted.unmount();
    assert!(
        trace.dropped.get(),
        "the task is dropped when its component unmounts"
    );
}

/// (c) A handler inside a `for` row: re-rendering the row's Button (its own
/// reactive prop) keeps the task; removing the row cancels it.
#[test]
fn a_task_from_a_for_row_lives_with_the_row_not_with_one_render() {
    let rows = Signal::new(vec![1u32, 2]);
    let busy = Signal::new(false);
    let traces: Rc<RefCell<Vec<(u32, Trace)>>> = Rc::default();
    let tr = traces.clone();
    let mounted = mount(move |__scope| {
        rsx! {
            div {
                for id in rows.get() {
                    div {
                        key: id,
                        Button {
                            loading: {|| busy.get()},
                            onclick: {
                                let tr = tr.clone();
                                move || {
                                    busy.set(true);
                                    let trace = Trace::default();
                                    tr.borrow_mut().push((id, trace.clone()));
                                    spawn_local(Pending(trace));
                                }
                            },
                            "Send"
                        }
                    }
                }
            }
        }
    });

    // Click row 2's button: its write re-renders both rows' Buttons.
    let ids = button_handlers(&mounted.root);
    assert_eq!(ids.len(), 2);
    assert!(rinch_core::events::dispatch_event(ids[1]));
    run_polls();
    let trace = {
        let t = traces.borrow();
        assert_eq!(t.len(), 1, "positive control: the handler ran");
        assert_eq!(t[0].0, 2, "the click reached row 2");
        t[0].1.clone()
    };
    assert!(
        !trace.dropped.get(),
        "a re-render of the row's Button must not drop the row's task"
    );
    assert_eq!(trace.polls.get(), 1, "the task got its first poll");

    // Removing row 1 leaves row 2's task alone.
    rows.set(vec![2]);
    assert!(!trace.dropped.get(), "another row's removal leaves it");

    // Removing row 2 cancels it.
    rows.set(vec![]);
    assert!(trace.dropped.get(), "removing the row drops its task");
    drop(mounted);
}
