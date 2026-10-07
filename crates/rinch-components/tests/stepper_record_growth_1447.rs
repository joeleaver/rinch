//! Review of PR #1447 (#1428): a step's click-handler record and the handlers
//! it names stay bounded, and a handler a stepper registers is that stepper
//! render's to free, whoever made the insertion that led to it.
//!
//! Each fixture prints what it measured (`-- --nocapture`).

use rinch_components as rinch;

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::stepper::{Stepper, StepperStep};
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::events::{EventHandlerId, dispatch_event, handler_count};
use rinch_core::{Component, IntoEventHandler, Signal, ValueCallback};
use rinch_macros::rsx;

mod common;
use common::{find_by_class, has_class};

const REC: &str = "data-stepper-click-handler";
const CLICKABLE: &str = "rinch-stepper__step--clickable";

struct Tree {
    _doc: Rc<RefCell<MockDomDocument>>,
    scope: RenderScope,
    root: NodeHandle,
}

impl Tree {
    fn build(build: impl FnOnce(&mut RenderScope) -> NodeHandle) -> Self {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let root = build(&mut scope);
        scope.body_handle().append_child(&root);
        Self {
            _doc: doc,
            scope,
            root,
        }
    }
}

fn steps_of(node: &NodeHandle) -> Vec<NodeHandle> {
    fn walk(node: &NodeHandle, out: &mut Vec<NodeHandle>) {
        if has_class(node, "rinch-stepper__step") {
            out.push(node.clone());
            return;
        }
        for child in node.children() {
            walk(&child, out);
        }
    }
    let mut out = Vec::new();
    walk(node, &mut out);
    out
}

fn entries(step: &NodeHandle) -> usize {
    step.get_attribute(REC)
        .map(|r| r.split_whitespace().count())
        .unwrap_or(0)
}

fn bytes(step: &NodeHandle) -> usize {
    step.get_attribute(REC).map(|r| r.len()).unwrap_or(0)
}

fn click(step: &NodeHandle) -> Option<bool> {
    let rid = step.get_attribute("data-rid")?;
    let id: usize = rid.parse().expect("data-rid is a handler id");
    Some(dispatch_event(EventHandlerId(id)))
}

fn move_into(step: &NodeHandle, stepper: &NodeHandle, at: usize) {
    let container = find_by_class(stepper, "rinch-stepper__steps").expect("steps container");
    let before = steps_of(stepper)[at].clone();
    container.insert_before(step, &before);
}

fn rounds() -> usize {
    std::env::var("PROBE_ROUNDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200)
}

/// P1: an rsx `Stepper` with a reactive prop, whose children are step nodes
/// the caller built once and keeps (captured handles).
#[test]
fn p1_reactive_stepper_over_captured_steps() {
    let active = Signal::new(9u32);
    let seen: Signal<Vec<u32>> = Signal::new(Vec::new());
    let kept: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let k = kept.clone();
    let tree = Tree::build(move |__scope| {
        let s0 = StepperStep::default().render(__scope, &[]);
        let s1 = StepperStep::default().render(__scope, &[]);
        let s2 = StepperStep::default().render(__scope, &[]);
        k.borrow_mut().extend([s0.clone(), s1.clone(), s2.clone()]);
        rsx! {
            div {
                Stepper { active: {move || active.get()}, on_step_click: move |i: u32| seen.update(|v| v.push(i)),
                    {s0.clone()}
                    {s1.clone()}
                    {s2.clone()}
                }
            }
        }
    });
    let kept = kept.borrow().clone();
    assert_eq!(
        steps_of(&tree.root).len(),
        3,
        "control: the kept steps are mounted"
    );
    assert_eq!(entries(&kept[0]), 1);
    let h0 = handler_count();
    let n = rounds();
    let t = std::time::Instant::now();
    for i in 0..n {
        active.set(9 + (i as u32 % 2) + 1);
    }
    let dt = t.elapsed();
    let same_nodes = steps_of(&tree.root)
        .iter()
        .zip(&kept)
        .all(|(a, b)| a.node_id() == b.node_id());
    eprintln!(
        "P1 rounds={n} same_nodes={same_nodes} entries={} bytes={} handlers {h0}->{} time={dt:?}",
        entries(&kept[0]),
        bytes(&kept[0]),
        handler_count()
    );
    assert_eq!(click(&kept[1]), Some(true));
    assert_eq!(seen.get(), vec![1]);
    assert!(
        same_nodes,
        "control: every render was handed the same step nodes"
    );
    assert_eq!(
        entries(&kept[0]),
        1,
        "a render that is gone leaves no entry: the record does not grow with re-renders"
    );
    assert_eq!(
        handler_count(),
        h0,
        "and each render's handlers went with it"
    );
}

/// P1b: the same stepper with a keyed `for` of steps.
#[test]
fn p1b_reactive_stepper_over_a_keyed_for() {
    let active = Signal::new(9u32);
    let items = Signal::new(vec!["a", "b", "c"]);
    let tree = Tree::build(move |__scope| {
        rsx! {
            div {
                Stepper { active: {move || active.get()}, on_step_click: move |_i: u32| {},
                    for it in items.get() { StepperStep { key: it, label: it } }
                }
            }
        }
    });
    let first = steps_of(&tree.root)[0].clone();
    let h0 = handler_count();
    for i in 0..rounds() {
        active.set(10 + (i as u32 % 2));
    }
    let now = steps_of(&tree.root);
    eprintln!(
        "P1b same_node={} max_entries={} handlers {h0}->{}",
        now[0].node_id() == first.node_id(),
        now.iter().map(entries).max().unwrap(),
        handler_count()
    );
    assert_eq!(now.iter().map(entries).max(), Some(1));
    assert_eq!(handler_count(), h0);
}

/// P2: one step shuttled between two steppers.
#[test]
fn p2_a_step_shuttled_between_two_steppers() {
    let mut tree = Tree::build(|scope| scope.create_element("div"));
    let mk = |scope: &mut RenderScope| {
        let steps: Vec<NodeHandle> = (0..3)
            .map(|_| StepperStep::default().render(scope, &[]))
            .collect();
        Stepper {
            active: 9,
            on_step_click: Some(ValueCallback::new(|_: u32| {})),
            ..Default::default()
        }
        .render(scope, &steps)
    };
    let a = mk(&mut tree.scope);
    let b = mk(&mut tree.scope);
    tree.root.append_child(&a);
    tree.root.append_child(&b);
    let moving = steps_of(&a)[0].clone();
    let h0 = handler_count();
    for _ in 0..1000 {
        move_into(&moving, &b, 1);
        move_into(&moving, &a, 1);
    }
    eprintln!(
        "P2 entries={} handlers {h0}->{}",
        entries(&moving),
        handler_count()
    );
    assert_eq!(entries(&moving), 2);
    assert_eq!(handler_count(), h0 + 1);
}

/// P3: a stepper unmounted and a new one mounted, over kept step nodes.
#[test]
fn p3_a_stepper_remounted_over_kept_steps() {
    let show = Signal::new(true);
    let seen: Signal<Vec<u32>> = Signal::new(Vec::new());
    let kept: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let k = kept.clone();
    let tree = Tree::build(move |__scope| {
        let s0 = StepperStep::default().render(__scope, &[]);
        let s1 = StepperStep::default().render(__scope, &[]);
        k.borrow_mut().extend([s0.clone(), s1.clone()]);
        rsx! {
            div {
                if show.get() {
                    Stepper { active: 9u32, on_step_click: move |i: u32| seen.update(|v| v.push(i)),
                        {s0.clone()}
                        {s1.clone()}
                    }
                }
            }
        }
    });
    let kept = kept.borrow().clone();
    assert_eq!(steps_of(&tree.root).len(), 2);
    let h0 = handler_count();
    for _ in 0..rounds() {
        show.set(false);
        show.set(true);
    }
    eprintln!(
        "P3 entries={} bytes={} handlers {h0}->{} mounted={}",
        entries(&kept[0]),
        bytes(&kept[0]),
        handler_count(),
        steps_of(&tree.root).len()
    );
    assert_eq!(click(&kept[1]), Some(true));
    assert_eq!(seen.get(), vec![1]);
    assert_eq!(entries(&kept[0]), 1, "one entry: the mounted stepper's");
    assert_eq!(handler_count(), h0);

    // P5: hidden — what does a kept step still carry, and does a click reach
    // anything?
    show.set(false);
    eprintln!(
        "P5 after hide: parent={:?} data-rid={:?} tabindex={:?} class_clickable={} click={:?} seen={:?}",
        kept[1].parent_node().map(|p| p.node_id()),
        kept[1].get_attribute("data-rid"),
        kept[1].get_attribute("tabindex"),
        has_class(&kept[1], CLICKABLE),
        click(&kept[1]),
        seen.get()
    );
}

/// P4: a kept step appended **late**, from code with no owner, into each new
/// mount of a stepper. Who owns the handler the late pass registers?
#[test]
fn p4_a_kept_step_appended_late_into_each_remount() {
    let show = Signal::new(true);
    let seen: Signal<Vec<u32>> = Signal::new(Vec::new());
    let mut tree = Tree::build(move |__scope| {
        rsx! {
            div {
                if show.get() {
                    Stepper { active: 9u32, on_step_click: move |i: u32| seen.update(|v| v.push(i)),
                        StepperStep {}
                    }
                }
            }
        }
    });
    let kept = StepperStep::default().render(&mut tree.scope, &[]);
    let h0 = handler_count();
    let n = rounds();
    for _ in 0..n {
        let c = find_by_class(&tree.root, "rinch-stepper__steps").unwrap();
        c.append_child(&kept);
        show.set(false);
        show.set(true);
    }
    let c = find_by_class(&tree.root, "rinch-stepper__steps").unwrap();
    c.append_child(&kept);
    eprintln!(
        "P4 rounds={n} entries={} handlers {h0}->{}",
        entries(&kept),
        handler_count()
    );
    assert_eq!(click(&kept), Some(true));
    assert_eq!(seen.get(), vec![1]);
    assert_eq!(
        handler_count(),
        h0 + 1,
        "a handler registered by a late pass is the stepper render's, and goes when it unmounts"
    );
    assert_eq!(entries(&kept), 1);
    seen.set(Vec::new());
    show.set(false);
    eprintln!(
        "P4 after hide: parent={:?} data-rid={:?} click={:?} seen={:?}",
        kept.parent_node().map(|p| p.node_id()),
        kept.get_attribute("data-rid"),
        click(&kept),
        seen.get()
    );
    assert_ne!(
        click(&kept),
        Some(true),
        "no live handler behind the kept step"
    );
    assert!(
        seen.get().is_empty(),
        "an unmounted stepper's on_step_click is not called"
    );
}

/// P6: a step that leaves stepper X, is settled by Y, and returns to X at the
/// **same** position it left (the #748 skip's fixed point).
#[test]
fn p6_a_step_returning_to_the_position_it_left() {
    let hits: Rc<RefCell<Vec<(&'static str, u32)>>> = Rc::new(RefCell::new(Vec::new()));
    let mut tree = Tree::build(|scope| scope.create_element("div"));
    let mut mk = |name: Option<&'static str>| {
        let steps: Vec<NodeHandle> = (0..3)
            .map(|_| StepperStep::default().render(&mut tree.scope, &[]))
            .collect();
        let hits = hits.clone();
        let s = Stepper {
            active: 9,
            on_step_click: name
                .map(|name| ValueCallback::new(move |p: u32| hits.borrow_mut().push((name, p)))),
            ..Default::default()
        }
        .render(&mut tree.scope, &steps);
        tree.root.append_child(&s);
        s
    };
    let x = mk(Some("X"));
    let y = mk(Some("Y"));
    let z = mk(None);

    let moving = steps_of(&x)[1].clone();
    let rid_x = moving.get_attribute("data-rid").unwrap();
    // To Y at position 1 as well, and back to X at position 1.
    move_into(&moving, &y, 1);
    assert_eq!(
        moving.get_attribute("data-step-position").as_deref(),
        Some("1")
    );
    assert_ne!(moving.get_attribute("data-rid").unwrap(), rid_x);
    move_into(&moving, &x, 1);
    assert_eq!(
        moving.get_attribute("data-step-position").as_deref(),
        Some("1")
    );
    assert_eq!(
        moving.get_attribute("data-rid").unwrap(),
        rid_x,
        "X's rid again"
    );
    assert_eq!(click(&moving), Some(true));
    assert_eq!(std::mem::take(&mut *hits.borrow_mut()), vec![("X", 1)]);

    // Through the callback-less Z at the same position, and back.
    move_into(&moving, &z, 1);
    assert_eq!(moving.get_attribute("data-rid"), None);
    assert!(!has_class(&moving, CLICKABLE));
    move_into(&moving, &x, 1);
    assert_eq!(moving.get_attribute("data-rid").unwrap(), rid_x);
    assert!(has_class(&moving, CLICKABLE));
    assert_eq!(moving.get_attribute("tabindex").as_deref(), Some("0"));
    assert_eq!(moving.get_attribute("role").as_deref(), Some("button"));
    assert_eq!(click(&moving), Some(true));
    assert_eq!(std::mem::take(&mut *hits.borrow_mut()), vec![("X", 1)]);

    // Out of every stepper, into a plain div, and back to X at 1: nobody
    // re-settled it in between.
    let plain = tree.scope.create_element("div");
    tree.root.append_child(&plain);
    plain.append_child(&moving);
    eprintln!(
        "P6 in a plain div: data-rid={:?} click={:?} hits={:?}",
        moving.get_attribute("data-rid"),
        click(&moving),
        hits.borrow()
    );
    hits.borrow_mut().clear();
    move_into(&moving, &x, 1);
    assert_eq!(click(&moving), Some(true));
    assert_eq!(std::mem::take(&mut *hits.borrow_mut()), vec![("X", 1)]);
}

/// P7: the class a callback-less stepper takes off a once-wired step, when the
/// class is on the step for other reasons.
#[test]
fn p7_what_a_callbackless_stepper_takes_off() {
    let mut tree = Tree::build(|scope| scope.create_element("div"));
    let mut mk = |cb: bool, props: Stepper| {
        let steps: Vec<NodeHandle> = (0..3)
            .map(|_| StepperStep::default().render(&mut tree.scope, &[]))
            .collect();
        let s = Stepper {
            on_step_click: cb.then(|| ValueCallback::new(|_: u32| {})),
            ..props
        }
        .render(&mut tree.scope, &steps);
        tree.root.append_child(&s);
        s
    };
    let a = mk(
        true,
        Stepper {
            active: 9,
            ..Default::default()
        },
    );
    // callback-less, grants the class past `active`
    let b = mk(
        false,
        Stepper {
            active: 0,
            allow_next_steps_select: true,
            ..Default::default()
        },
    );
    // callback-less, grants nothing
    let c = mk(
        false,
        Stepper {
            active: 0,
            ..Default::default()
        },
    );

    // (a) once-wired, B grants it the class at position 2, then C.
    let never = steps_of(&b)[2].clone();
    let wired = steps_of(&a)[0].clone();
    move_into(&wired, &b, 2);
    let in_b = has_class(&wired, CLICKABLE);
    move_into(&wired, &c, 2);
    let in_c = has_class(&wired, CLICKABLE);
    // (b) never-wired, same trip.
    let n_in_b = has_class(&never, CLICKABLE);
    move_into(&never, &c, 2);
    let n_in_c = has_class(&never, CLICKABLE);
    eprintln!("P7 wired: B={in_b} C={in_c}; never-wired: B={n_in_b} C={n_in_c}");

    // (c) once-wired step sitting AT `active` in B (the grant is `> active`).
    let wired2 = steps_of(&a)[0].clone();
    move_into(&wired2, &b, 0);
    eprintln!(
        "P7 wired at position {:?} == active in allow_next B: class={}",
        wired2.get_attribute("data-step-position"),
        has_class(&wired2, CLICKABLE)
    );
    assert!(in_b, "B grants the class past `active`");
    assert!(!in_c, "C grants nothing, and the class was A's wiring's");
    assert!(n_in_b && n_in_c, "a never-wired step is not touched");
    assert!(
        !has_class(&wired2, CLICKABLE),
        "B's grant is for positions past `active`, not `active` itself"
    );
}

/// P8: an ordinary app — a mounted/unmounted stepper with a keyed `for`, a row
/// arriving late each time. Are the late pass's handlers the stepper's?
#[test]
fn p8_late_rows_of_a_remounted_stepper() {
    let show = Signal::new(true);
    let items = Signal::new(vec![0u32]);
    let tree = Tree::build(move |__scope| {
        rsx! {
            div {
                if show.get() {
                    Stepper { active: 99u32, on_step_click: move |_i: u32| {},
                        for it in items.get() { StepperStep { key: it } }
                    }
                }
            }
        }
    });
    let h0 = handler_count();
    let n = rounds();
    for i in 0..n {
        items.update(|v| v.push(i as u32 + 1));
        items.update(|v| {
            v.pop();
        });
        show.set(false);
        show.set(true);
    }
    items.update(|v| v.push(1000));
    eprintln!(
        "P8 rounds={n} steps={} handlers {h0}->{}",
        steps_of(&tree.root).len(),
        handler_count()
    );
    assert_eq!(handler_count(), h0 + 1, "the one late row mounted now");
}
