//! A task spawned from a handler the caller wrote among a re-rendering
//! component's **children** belongs to the caller, not to one render of that
//! component (issue #3030; #3027 covered the component's own `on*` props).
//!
//! The shape: `Paper { shadow: {|| ..busy..}, <handler> }`. Paper re-renders
//! for its reactive prop, and its children are rebuilt inside that render's
//! scope. A handler there — an element's `onclick`, a plain component's `on*`
//! prop, a nested re-rendering component's `on*` prop — used to run under that
//! scope, so the click's own `busy.set(true)` re-rendered Paper when the batch
//! ended and dropped the task before its first poll. Each case is also tested
//! nested two deep (a re-rendering parent inside another), and each samples
//! both sides: the re-render must not drop the task, the unmount must.
//!
//! The boundaries: a `for` row is the creator of a handler written in it, and
//! a handler holding what one render made (a signal its own expression
//! creates) stays that render's.

#[path = "support/task_owner.rs"]
mod task_owner;

use rinch::prelude::*;
use rinch_core::events::dispatch_event;
use rinch_core::task::spawn_local;
use std::cell::RefCell;
use std::rc::Rc;
use task_owner::*;

/// `"md"` while `busy`, else `"sm"`: the reactive prop that re-renders Paper.
fn shadow(busy: Signal<bool>) -> String {
    if busy.get() { "md" } else { "sm" }.to_string()
}

/// Whether `node` or anything under it carries `class`.
fn any_has_class(node: &NodeHandle, class: &str) -> bool {
    has_class(node, class) || node.children().iter().any(|c| any_has_class(c, class))
}

/// The first node under `node`, in tree order, carrying `attr`, and its value.
fn find_attribute(node: &NodeHandle, attr: &str) -> Option<String> {
    node.get_attribute(attr)
        .or_else(|| node.children().iter().find_map(|c| find_attribute(c, attr)))
}

/// Every click handler in the tree.
fn all_clicks(node: &NodeHandle) -> Vec<rinch_core::events::EventHandlerId> {
    click_handlers(node, &|_| true)
}

/// Click the tree's one click handler, which sets `busy` and spawns a task:
/// the re-render that write causes must leave the task running, and the
/// unmount must drop it.
fn click_survives_rerender_then_unmount_drops(
    mut mounted: Mounted,
    busy: Signal<bool>,
    trace: &Trace,
) {
    let ids = all_clicks(&mounted.root);
    assert_eq!(ids.len(), 1, "one click handler in the tree");
    assert!(dispatch_event(ids[0]), "the click reached a handler");
    assert!(busy.get(), "positive control: the handler ran");
    assert!(
        any_has_class(&mounted.root, "rinch-paper--shadow-md"),
        "positive control: the parent re-rendered"
    );
    run_polls();
    assert!(
        !trace.dropped.get(),
        "the parent's re-render must not drop the task the caller's handler spawned"
    );
    assert_eq!(trace.polls.get(), 1, "the task got its first poll");

    mounted.unmount();
    assert!(trace.dropped.get(), "unmounting the caller drops the task");
}

#[test]
fn an_element_handler_among_a_rerendering_parents_children() {
    let busy = Signal::new(false);
    let trace = Trace::default();
    let t = trace.clone();
    let mounted = mount(move |__scope| {
        rsx! {
            div {
                Paper {
                    shadow: {|| shadow(busy)},
                    button {
                        onclick: {
                            let t = t.clone();
                            move || {
                                busy.set(true);
                                spawn_local(Pending(t.clone()));
                            }
                        },
                        "go"
                    }
                }
            }
        }
    });
    click_survives_rerender_then_unmount_drops(mounted, busy, &trace);
}

#[test]
fn an_element_handler_two_rerendering_parents_deep() {
    let busy = Signal::new(false);
    let trace = Trace::default();
    let t = trace.clone();
    let mounted = mount(move |__scope| {
        rsx! {
            div {
                Paper {
                    shadow: {|| shadow(busy)},
                    Paper {
                        shadow: {|| shadow(busy)},
                        button {
                            onclick: {
                                let t = t.clone();
                                move || {
                                    busy.set(true);
                                    spawn_local(Pending(t.clone()));
                                }
                            },
                            "go"
                        }
                    }
                }
            }
        }
    });
    click_survives_rerender_then_unmount_drops(mounted, busy, &trace);
}

#[test]
fn a_plain_components_handler_among_a_rerendering_parents_children() {
    let busy = Signal::new(false);
    let trace = Trace::default();
    let t = trace.clone();
    let mounted = mount(move |__scope| {
        rsx! {
            div {
                Paper {
                    shadow: {|| shadow(busy)},
                    Button {
                        onclick: {
                            let t = t.clone();
                            move || {
                                busy.set(true);
                                spawn_local(Pending(t.clone()));
                            }
                        },
                        "go"
                    }
                }
            }
        }
    });
    click_survives_rerender_then_unmount_drops(mounted, busy, &trace);
}

#[test]
fn a_plain_components_handler_two_rerendering_parents_deep() {
    let busy = Signal::new(false);
    let trace = Trace::default();
    let t = trace.clone();
    let mounted = mount(move |__scope| {
        rsx! {
            div {
                Paper {
                    shadow: {|| shadow(busy)},
                    Paper {
                        shadow: {|| shadow(busy)},
                        Button {
                            onclick: {
                                let t = t.clone();
                                move || {
                                    busy.set(true);
                                    spawn_local(Pending(t.clone()));
                                }
                            },
                            "go"
                        }
                    }
                }
            }
        }
    });
    click_survives_rerender_then_unmount_drops(mounted, busy, &trace);
}

#[test]
fn a_rerendering_components_handler_among_a_rerendering_parents_children() {
    let busy = Signal::new(false);
    let trace = Trace::default();
    let t = trace.clone();
    let mounted = mount(move |__scope| {
        rsx! {
            div {
                Paper {
                    shadow: {|| shadow(busy)},
                    Button {
                        loading: {|| busy.get()},
                        onclick: {
                            let t = t.clone();
                            move || {
                                busy.set(true);
                                spawn_local(Pending(t.clone()));
                            }
                        },
                        "go"
                    }
                }
            }
        }
    });
    click_survives_rerender_then_unmount_drops(mounted, busy, &trace);
}

#[test]
fn a_rerendering_components_handler_two_rerendering_parents_deep() {
    let busy = Signal::new(false);
    let trace = Trace::default();
    let t = trace.clone();
    let mounted = mount(move |__scope| {
        rsx! {
            div {
                Paper {
                    shadow: {|| shadow(busy)},
                    Paper {
                        shadow: {|| shadow(busy)},
                        Button {
                            loading: {|| busy.get()},
                            onclick: {
                                let t = t.clone();
                                move || {
                                    busy.set(true);
                                    spawn_local(Pending(t.clone()));
                                }
                            },
                            "go"
                        }
                    }
                }
            }
        }
    });
    click_survives_rerender_then_unmount_drops(mounted, busy, &trace);
}

/// An `oninput` (a handler taking a value) among the children follows the
/// same rule; the other argument shapes compile in the same position.
#[test]
fn an_input_handler_among_a_rerendering_parents_children() {
    let busy = Signal::new(false);
    let trace = Trace::default();
    let t = trace.clone();
    let mut mounted = mount(move |__scope| {
        rsx! {
            div {
                Paper {
                    shadow: {|| shadow(busy)},
                    input {
                        oninput: {
                            let t = t.clone();
                            move |value: String| {
                                busy.set(!value.is_empty());
                                spawn_local(Pending(t.clone()));
                            }
                        },
                        onscroll: |_| {},
                        onkeydown: |_| false,
                        onwheel: |_| false,
                        onfiledrop: |_| {},
                        ondropfiles: |_| {},
                    }
                }
            }
        }
    });
    let id = find_attribute(&mounted.root, "data-oninput").expect("an input handler");
    let id = rinch_core::events::EventHandlerId(id.parse().unwrap());
    assert!(rinch_core::events::dispatch_input_event(id, "x".into()));
    assert!(busy.get(), "positive control: the handler ran");
    run_polls();
    assert!(!trace.dropped.get(), "the re-render must not drop the task");
    assert_eq!(trace.polls.get(), 1, "the task got its first poll");
    mounted.unmount();
    assert!(trace.dropped.get(), "unmounting the caller drops the task");
}

/// A `for` row among the children is the creator of a handler written in it:
/// removing the row drops the task, and the parent's re-render — which
/// rebuilds the row — does too.
#[test]
fn a_for_rows_handler_belongs_to_the_row() {
    let busy = Signal::new(false);
    let rows = Signal::new(vec![1u32, 2]);
    let traces: Rc<RefCell<Vec<(u32, Trace)>>> = Rc::default();
    let tr = traces.clone();
    let mounted = mount(move |__scope| {
        rsx! {
            div {
                Paper {
                    shadow: {|| shadow(busy)},
                    for id in rows.get() {
                        button {
                            key: id,
                            onclick: {
                                let tr = tr.clone();
                                move || {
                                    let trace = Trace::default();
                                    tr.borrow_mut().push((id, trace.clone()));
                                    spawn_local(Pending(trace));
                                }
                            },
                            "go"
                        }
                    }
                }
            }
        }
    });
    let click_row = |row: usize| {
        let ids = all_clicks(&mounted.root);
        assert!(dispatch_event(ids[row]), "the click reached a handler");
        run_polls();
        traces
            .borrow()
            .last()
            .expect("positive control: the handler ran")
            .1
            .clone()
    };

    let second = click_row(1);
    assert_eq!(second.polls.get(), 1, "the task got its first poll");
    rows.set(vec![1]);
    assert!(second.dropped.get(), "removing the row drops its task");

    let first = click_row(0);
    busy.set(true);
    assert!(
        any_has_class(&mounted.root, "rinch-paper--shadow-md"),
        "positive control: the parent re-rendered"
    );
    assert!(
        first.dropped.get(),
        "the parent's re-render rebuilds the row, and drops the row's task"
    );
    drop(mounted);
}

/// A handler whose own expression creates a signal holds what one render made
/// — a signal the next render frees — so it stays that render's: the task
/// ends with the render rather than outliving the signal it reads.
#[test]
fn a_handler_whose_expression_creates_a_signal_stays_the_renders() {
    let busy = Signal::new(false);
    let trace = Trace::default();
    let t = trace.clone();
    let mounted = mount(move |__scope| {
        rsx! {
            div {
                Paper {
                    shadow: {|| shadow(busy)},
                    button {
                        onclick: {
                            let clicks = Signal::new(0u32);
                            let t = t.clone();
                            move || {
                                clicks.update(|n| *n += 1);
                                busy.set(true);
                                spawn_local(Pending(t.clone()));
                            }
                        },
                        "go"
                    }
                }
            }
        }
    });
    let ids = all_clicks(&mounted.root);
    assert!(dispatch_event(ids[0]));
    assert!(busy.get(), "positive control: the handler ran");
    assert!(
        trace.dropped.get(),
        "the render that made `clicks` ended, and the task holding it with it"
    );
    drop(mounted);
}

/// A re-rendering component's `on*` prop is evaluated in each render, so what
/// its expression creates belongs to that render: re-rendering does not pile a
/// signal per render onto the caller (#3027 review, low 4).
#[test]
fn a_handler_props_signals_do_not_accumulate_on_the_caller() {
    let busy = Signal::new(false);
    let mounted = mount(move |__scope| {
        rsx! {
            div {
                Button {
                    loading: {|| busy.get()},
                    onclick: {
                        let clicks = Signal::new(0u32);
                        move || clicks.update(|n| *n += 1)
                    },
                    "go"
                }
            }
        }
    });
    let caller = mounted.scope.as_ref().unwrap().owner();
    let before = caller.owned_counts().unwrap();
    for i in 0..4 {
        busy.set(i % 2 == 0);
    }
    assert_eq!(
        caller.owned_counts().unwrap(),
        before,
        "four re-renders left the caller's owned resources as they were"
    );
    drop(mounted);
}
