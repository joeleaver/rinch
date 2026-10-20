//! Review probes for PR #1598 (Tree multi-select). Each asserts the behaviour
//! the reviewer expects; a FAILING probe is a finding.

use super::*;
use std::cell::RefCell;

use rinch_components::{
    Tree, TreeKey, TreeMove, TreeNodeData, TreeSlot, UseTreeOptions, UseTreeReturn,
};
use rinch_core::Component;
use rinch_core::element::ValueCallback;
use rinch_core::reactive::Signal;
use rinch_platform::{KeyCode, KeyRepeat, Modifiers};

const VIEWPORT: (f32, f32) = (800.0, 600.0);

type Log<T> = Rc<RefCell<Vec<T>>>;

#[derive(Clone, Copy)]
struct Opts {
    multiple: bool,
    select_on_click: bool,
}

/// What the app's `handle_key` saw: key, below, carried, drop_for(Down) slot.
type Seen = (String, Option<String>, Vec<String>, Option<(Option<String>, TreeSlot)>);

struct P {
    app: RinchApp,
    state: UseTreeReturn,
    data: Signal<Vec<TreeNodeData>>,
    changes: Log<Vec<String>>,
    selects: Log<String>,
    rested: Log<String>,
    seen: Log<Seen>,
}

fn build(opts: Opts, initial: Vec<TreeNodeData>, expanded: &[&str]) -> P {
    let changes: Log<Vec<String>> = Rc::default();
    let selects: Log<String> = Rc::default();
    let rested: Log<String> = Rc::default();
    let seen: Log<Seen> = Rc::default();
    let signal: Rc<RefCell<Option<Signal<Vec<TreeNodeData>>>>> = Rc::default();
    let state = UseTreeReturn::new(UseTreeOptions {
        initial_expanded: expanded.iter().map(|s| s.to_string()).collect(),
        multiple: opts.multiple,
        ..Default::default()
    });
    let (c, o, r, s, sig) = (
        changes.clone(),
        selects.clone(),
        rested.clone(),
        seen.clone(),
        signal.clone(),
    );
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let (c, o, r, s) = (c.clone(), o.clone(), r.clone(), s.clone());
        let data = Signal::new(initial.clone());
        *sig.borrow_mut() = Some(data);
        let tree = Tree {
            data: data.get(),
            data_source: Some(Rc::new(move || data.get())),
            tree: Some(state),
            select_on_click: opts.select_on_click,
            keyboard: true,
            cursor_delay_ms: 0,
            onselect: Some(ValueCallback::new(move |v| o.borrow_mut().push(v))),
            oncursor: Some(ValueCallback::new(move |v| r.borrow_mut().push(v))),
            onselectionchange: Some(ValueCallback::new(move |v| c.borrow_mut().push(v))),
            handle_key: Some(Rc::new(move |key: &TreeKey| {
                let drop = key.drop_for(TreeMove::Down).map(|d| (d.parent, d.slot));
                s.borrow_mut().push((
                    key.key.clone(),
                    key.below.clone(),
                    key.carried().to_vec(),
                    drop,
                ));
                false
            })),
            ..Default::default()
        }
        .render(scope, &[]);
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 300px");
        root.append_child(&tree);
        root
    });
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let data = signal.borrow().unwrap();
    P {
        app,
        state,
        data,
        changes,
        selects,
        rested,
        seen,
    }
}

fn flat(values: &[&str]) -> Vec<TreeNodeData> {
    values
        .iter()
        .map(|v| TreeNodeData::new(*v, v.to_uppercase()))
        .collect()
}

fn has_class(n: &rinch_dom::Node, class: &str) -> bool {
    n.attributes
        .get("class")
        .is_some_and(|c| c.split_whitespace().any(|one| one == class))
}

/// The connected content box of `value`.
fn row(app: &RinchApp, value: &str) -> usize {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let li = d
        .tree
        .nodes
        .iter()
        .find(|(_, n)| {
            n.attributes.get("data-value").map(String::as_str) == Some(value)
                && n.parent.is_some()
        })
        .map(|(id, _)| id)
        .expect("the row");
    d.tree.nodes[li]
        .children
        .iter()
        .copied()
        .find(|&c| has_class(&d.tree.nodes[c], "rinch-tree__node-content"))
        .unwrap()
}

fn centre(app: &RinchApp, id: usize) -> (f32, f32) {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let (x, y, w, h) = hit_testing::painted_element_box(&d.tree, id);
    (x + w / 2.0, y + h / 2.0)
}

fn click(p: &mut P, value: &str, modifiers: Modifiers) {
    let (x, y) = centre(&p.app, row(&p.app, value));
    p.app
        .handle_event(PlatformEvent::ModifiersChanged(modifiers), (800, 600), 1.0);
    for event in [
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        },
    ] {
        p.app.handle_event(event, (800, 600), 1.0);
    }
    p.app.handle_event(
        PlatformEvent::ModifiersChanged(Modifiers::default()),
        (800, 600),
        1.0,
    );
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
}

fn press(p: &mut P, key: KeyCode, text: Option<&str>, modifiers: Modifiers) {
    p.app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers,
            repeat: KeyRepeat::Unknown,
        },
        (800, 600),
        1.0,
    );
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
}

fn sel(p: &P) -> Vec<String> {
    let mut s: Vec<String> = p.state.selected.get().into_iter().collect();
    s.sort();
    s
}

fn shift() -> Modifiers {
    Modifiers {
        shift: true,
        ..Default::default()
    }
}

fn ctrl() -> Modifiers {
    Modifiers {
        ctrl: true,
        ..Default::default()
    }
}

const MULTI: Opts = Opts {
    multiple: true,
    select_on_click: true,
};

/// P1: a run (Shift-click, Shift+arrow) and Ctrl+A never select a disabled
/// row; Space and a click refuse one.
#[test]
fn p1_shift_click_run_skips_a_disabled_row() {
    let mut data = flat(&["a", "b", "c", "d"]);
    data[1] = TreeNodeData::new("b", "B").disabled(true);
    let mut p = build(MULTI, data, &[]);
    click(&mut p, "a", Modifiers::default());
    click(&mut p, "c", shift());
    assert_eq!(sel(&p), ["a", "c"], "disabled b selected by a Shift-click run");
}

#[test]
fn p1b_ctrl_a_skips_a_disabled_row() {
    let mut data = flat(&["a", "b", "c", "d"]);
    data[1] = TreeNodeData::new("b", "B").disabled(true);
    let mut p = build(MULTI, data, &[]);
    click(&mut p, "a", Modifiers::default());
    press(&mut p, KeyCode::KeyA, Some("a"), ctrl());
    assert_eq!(sel(&p), ["a", "c", "d"], "disabled b selected by Ctrl+A");
}

/// P2: #1601's rule — a tree that does not select on click selects nothing
/// from the keyboard — in a `multiple` tree.
#[test]
fn p2_without_select_on_click_shift_arrows_select_nothing() {
    let mut p = build(
        Opts {
            multiple: true,
            select_on_click: false,
        },
        flat(&["a", "b", "c"]),
        &[],
    );
    click(&mut p, "a", Modifiers::default());
    assert!(sel(&p).is_empty(), "control");
    press(&mut p, KeyCode::ArrowDown, None, shift());
    assert_eq!(p.state.cursor.get().as_deref(), Some("b"), "control: moved");
    assert!(sel(&p).is_empty(), "Shift+Down selected {:?}", sel(&p));
}

#[test]
fn p2b_without_select_on_click_ctrl_a_selects_nothing() {
    let mut p = build(
        Opts {
            multiple: true,
            select_on_click: false,
        },
        flat(&["a", "b", "c"]),
        &[],
    );
    click(&mut p, "a", Modifiers::default());
    press(&mut p, KeyCode::KeyA, Some("a"), ctrl());
    assert!(sel(&p).is_empty(), "Ctrl+A selected {:?}", sel(&p));
}

/// P3: the anchor row is deleted. A Shift run must still do something
/// (from the cursor), not silently nothing.
#[test]
fn p3_shift_arrow_after_the_anchor_row_is_deleted() {
    let mut p = build(MULTI, flat(&["a", "b", "c", "d"]), &[]);
    click(&mut p, "b", Modifiers::default());
    assert_eq!(sel(&p), ["b"]);
    p.data.set(flat(&["a", "c", "d"]));
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    p.app
        .handle_event(PlatformEvent::AboutToWait, (800, 600), 1.0);
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(p.state.cursor.get().as_deref(), Some("c"), "control: cursor moved on");
    let before = p.changes.borrow().len();
    press(&mut p, KeyCode::ArrowDown, None, shift());
    assert_eq!(p.state.cursor.get().as_deref(), Some("d"), "control: cursor moved");
    assert_eq!(
        sel(&p),
        ["c", "d"],
        "Shift+Down after the anchor went: selection {:?}, changes fired {}",
        sel(&p),
        p.changes.borrow().len() - before
    );
}

#[test]
fn p3b_shift_click_after_the_anchor_row_is_deleted() {
    let mut p = build(MULTI, flat(&["a", "b", "c", "d"]), &[]);
    click(&mut p, "b", Modifiers::default());
    p.data.set(flat(&["a", "c", "d"]));
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    click(&mut p, "d", shift());
    assert!(
        sel(&p).contains(&"d".to_string()),
        "a Shift-click on d after the anchor went selected {:?}",
        sel(&p)
    );
}

/// P4: anchor hidden by a collapse. A Shift-click run from a hidden anchor.
#[test]
fn p4_shift_click_with_the_anchor_hidden_by_a_collapse() {
    let data = vec![
        TreeNodeData::new("a", "A").with_children(flat(&["a1", "a2"])),
        TreeNodeData::new("b", "B"),
        TreeNodeData::new("c", "C"),
    ];
    let mut p = build(MULTI, data, &["a"]);
    click(&mut p, "a1", Modifiers::default());
    // Collapse a from the controller (the anchor a1 is hidden; the app's own
    // collapse button, say).
    p.state.controller.collapse("a");
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    click(&mut p, "c", shift());
    // Reported, not asserted as a bug: what the run is.
    eprintln!("P4 selection after Shift-click c with hidden anchor a1: {:?}", sel(&p));
    assert!(sel(&p).len() > 1, "run from a hidden anchor is {:?}", sel(&p));
}

/// P5: onselectionchange fires only when the selection changed.
#[test]
fn p5_no_change_no_report() {
    let mut p = build(MULTI, flat(&["a", "b", "c"]), &[]);
    click(&mut p, "a", Modifiers::default());
    let n = p.changes.borrow().len();
    click(&mut p, "a", Modifiers::default());
    press(&mut p, KeyCode::ArrowUp, None, shift()); // already at the top
    assert_eq!(
        p.changes.borrow().len(),
        n,
        "reports with no change: {:?}",
        &p.changes.borrow()[n..]
    );
}

/// P6: oncursor hears every cursor move, Shift+arrow included.
#[test]
fn p6_oncursor_hears_shift_arrow_moves() {
    let mut p = build(MULTI, flat(&["a", "b", "c"]), &[]);
    click(&mut p, "a", Modifiers::default());
    p.rested.borrow_mut().clear();
    press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
    assert_eq!(*p.rested.borrow(), ["b"], "control: plain arrow reports");
    press(&mut p, KeyCode::ArrowDown, None, shift());
    assert_eq!(p.state.cursor.get().as_deref(), Some("c"));
    assert_eq!(*p.rested.borrow(), ["b", "c"], "Shift+Down not reported to oncursor");
}

/// P7: Space selecting a row in a multiple tree tells `onselect`, as a
/// click does (and as it did on main after #1601).
#[test]
fn p7_space_tells_onselect_as_a_click_does() {
    let mut p = build(MULTI, flat(&["a", "b", "c"]), &[]);
    click(&mut p, "a", Modifiers::default());
    assert_eq!(*p.selects.borrow(), ["a"], "control: a click tells onselect");
    press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
    press(&mut p, KeyCode::Space, Some(" "), Modifiers::default());
    assert_eq!(sel(&p), ["a", "b"], "control");
    assert_eq!(*p.selects.borrow(), ["a", "b"], "Space did not tell onselect");
}

/// P8: TreeKey::below of an open parent in a single-select tree: on main
/// it was the first child (the row drawn right below the cursor row).
#[test]
fn p8_below_an_open_parent_single_select() {
    let data = vec![
        TreeNodeData::new("a", "A").with_children(flat(&["a1", "a2"])),
        TreeNodeData::new("b", "B"),
    ];
    let mut p = build(
        Opts {
            multiple: false,
            select_on_click: true,
        },
        data,
        &["a"],
    );
    click(&mut p, "a", Modifiers::default());
    // `a` was toggled closed by the click? expand_on_click default.
    p.state.controller.expand("a");
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    press(&mut p, KeyCode::F2, None, Modifiers::default());
    let seen = p.seen.borrow().last().cloned().unwrap();
    assert_eq!(seen.1.as_deref(), Some("a1"), "below = {:?}", seen.1);
}

/// P9: Shift+Space selects the run from the anchor (APG).
#[test]
fn p9_shift_space_runs_from_the_anchor() {
    let mut p = build(MULTI, flat(&["a", "b", "c"]), &[]);
    click(&mut p, "a", Modifiers::default());
    press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
    press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
    press(&mut p, KeyCode::Space, Some(" "), shift());
    eprintln!("P9 selection after Shift+Space: {:?}", sel(&p));
    assert!(sel(&p).contains(&"c".to_string()), "Shift+Space did nothing: {:?}", sel(&p));
}

/// P10: the block move of a non-contiguous selection (report what it is).
#[test]
fn p10_non_contiguous_block_down() {
    let mut p = build(MULTI, flat(&["a", "b", "c", "d", "e"]), &[]);
    click(&mut p, "b", Modifiers::default());
    click(&mut p, "d", ctrl());
    press(&mut p, KeyCode::F2, None, Modifiers::default());
    let seen = p.seen.borrow().last().cloned().unwrap();
    eprintln!("P10 carried {:?} below {:?} drop_for(Down) {:?}", seen.2, seen.1, seen.3);
    assert_eq!(seen.2, ["b", "d"]);
    assert_eq!(seen.3, Some((None, TreeSlot::After("e".into()))));
}

/// P11: a row-click collapse (expand_on_click) drops hidden rows from the
/// selection as the chevron and Left do.
#[test]
fn p11_row_click_collapse_unselects_hidden_children() {
    let data = vec![
        TreeNodeData::new("a", "A").with_children(flat(&["a1", "a2"])),
        TreeNodeData::new("b", "B"),
    ];
    let mut p = build(MULTI, data, &["a"]);
    click(&mut p, "a1", Modifiers::default());
    click(&mut p, "a2", ctrl());
    assert_eq!(sel(&p), ["a1", "a2"]);
    // Collapse with Left from a (a is open): the key path unselects.
    click(&mut p, "a", ctrl()); // add a; anchor a; cursor a
    press(&mut p, KeyCode::ArrowLeft, None, Modifiers::default());
    assert!(!p.state.controller.is_expanded("a"), "control: collapsed");
    assert_eq!(sel(&p), ["a"], "Left collapse keeps hidden {:?}", sel(&p));
}

/// P9b: Shift+Space with Shift held as a real keyboard reports it
/// (ModifiersChanged first): the multi-selection must not collapse.
#[test]
fn p9b_shift_space_keeps_the_selection() {
    let mut p = build(MULTI, flat(&["a", "b", "c"]), &[]);
    click(&mut p, "a", Modifiers::default());
    press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
    press(&mut p, KeyCode::Space, Some(" "), Modifiers::default());
    assert_eq!(sel(&p), ["a", "b"], "control");
    press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
    p.selects.borrow_mut().clear();
    p.app
        .handle_event(PlatformEvent::ModifiersChanged(shift()), (800, 600), 1.0);
    press(&mut p, KeyCode::Space, Some(" "), shift());
    p.app.handle_event(
        PlatformEvent::ModifiersChanged(Modifiers::default()),
        (800, 600),
        1.0,
    );
    eprintln!(
        "P9b selection {:?} onselect {:?} changes {:?}",
        sel(&p),
        p.selects.borrow(),
        p.changes.borrow().last()
    );
    assert!(
        sel(&p).len() >= 2,
        "Shift+Space collapsed the selection to {:?}",
        sel(&p)
    );
}
