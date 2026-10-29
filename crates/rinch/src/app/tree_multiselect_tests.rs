//! Tree multi-select (#1598): clicks with Ctrl and Shift, Shift with the
//! keys, Ctrl+A, Enter and Space in a `multiple` tree, and what the app is
//! told. Started as the review probes of #1598; each was red before its fix.

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
    keyboard: bool,
}

/// What the app's `handle_key` saw: key, below, carried, drop_for(Down) slot.
type Seen = (
    String,
    Option<String>,
    Vec<String>,
    Option<(Option<String>, TreeSlot)>,
);

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
            keyboard: opts.keyboard,
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
fn row(app: &RinchApp, value: &str) -> u64 {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let li = d
        .tree
        .nodes
        .iter()
        .find(|(_, n)| {
            n.attributes.get("data-value").map(String::as_str) == Some(value) && n.parent.is_some()
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

fn centre(app: &RinchApp, id: u64) -> (f32, f32) {
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
    keyboard: true,
};

/// `multiple`, but not selecting on click.
const MULTI_NO_SELECT: Opts = Opts {
    select_on_click: false,
    ..MULTI
};

/// a (a1, a2), b, c.
fn nested() -> Vec<TreeNodeData> {
    vec![
        TreeNodeData::new("a", "A").with_children(flat(&["a1", "a2"])),
        TreeNodeData::new("b", "B"),
        TreeNodeData::new("c", "C"),
    ]
}

/// a, b (disabled), c, d.
fn with_disabled_b() -> Vec<TreeNodeData> {
    let mut data = flat(&["a", "b", "c", "d"]);
    data[1] = TreeNodeData::new("b", "B").disabled(true);
    data
}

fn last_report(p: &P) -> Vec<String> {
    p.changes.borrow().last().cloned().expect("a report")
}

// ---- Disabled rows (F2) ----------------------------------------------------

#[test]
fn a_shift_click_run_leaves_out_a_disabled_row() {
    let mut p = build(MULTI, with_disabled_b(), &[]);
    click(&mut p, "a", Modifiers::default());
    click(&mut p, "c", shift());
    assert_eq!(sel(&p), ["a", "c"]);
    assert_eq!(last_report(&p), ["a", "c"]);
}

#[test]
fn ctrl_a_leaves_out_a_disabled_row() {
    let mut p = build(MULTI, with_disabled_b(), &[]);
    click(&mut p, "a", Modifiers::default());
    press(&mut p, KeyCode::KeyA, Some("a"), ctrl());
    assert_eq!(sel(&p), ["a", "c", "d"]);
}

// ---- Without select_on_click (F3, #1601's rule) -----------------------------

#[test]
fn without_select_on_click_shift_arrows_only_move_the_cursor() {
    let mut p = build(MULTI_NO_SELECT, flat(&["a", "b", "c"]), &[]);
    click(&mut p, "a", Modifiers::default());
    assert!(sel(&p).is_empty());
    press(&mut p, KeyCode::ArrowDown, None, shift());
    assert_eq!(
        p.state.cursor.get().as_deref(),
        Some("b"),
        "the cursor moves"
    );
    assert!(sel(&p).is_empty(), "Shift+Down selected {:?}", sel(&p));
}

#[test]
fn without_select_on_click_ctrl_a_selects_nothing() {
    let mut p = build(MULTI_NO_SELECT, flat(&["a", "b", "c"]), &[]);
    click(&mut p, "a", Modifiers::default());
    press(&mut p, KeyCode::KeyA, Some("a"), ctrl());
    assert!(sel(&p).is_empty(), "Ctrl+A selected {:?}", sel(&p));
}

// ---- A deleted anchor (F5) and deleted rows (F6) ----------------------------

/// The data drops b, the anchor; the cursor (on b) moves on to c.
fn drop_the_anchor(p: &mut P) {
    click(p, "b", Modifiers::default());
    assert_eq!(sel(p), ["b"]);
    p.data.set(flat(&["a", "c", "d"]));
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    p.app
        .handle_event(PlatformEvent::AboutToWait, (800, 600), 1.0);
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
}

#[test]
fn a_shift_arrow_after_the_anchor_row_is_deleted_runs_from_the_cursor() {
    let mut p = build(MULTI, flat(&["a", "b", "c", "d"]), &[]);
    drop_the_anchor(&mut p);
    assert_eq!(
        p.state.cursor.get().as_deref(),
        Some("c"),
        "the cursor moved on"
    );
    press(&mut p, KeyCode::ArrowDown, None, shift());
    assert_eq!(p.state.cursor.get().as_deref(), Some("d"));
    assert_eq!(sel(&p), ["c", "d"]);
    assert_eq!(last_report(&p), ["c", "d"]);
}

#[test]
fn a_shift_click_after_the_anchor_row_is_deleted_selects_the_clicked_row() {
    let mut p = build(MULTI, flat(&["a", "b", "c", "d"]), &[]);
    drop_the_anchor(&mut p);
    click(&mut p, "d", shift());
    assert_eq!(sel(&p), ["d"]);
    // And the next run starts there.
    click(&mut p, "a", shift());
    assert_eq!(sel(&p), ["a", "c", "d"]);
}

#[test]
fn a_deleted_row_is_not_reported() {
    let mut p = build(MULTI, flat(&["a", "b", "c"]), &[]);
    click(&mut p, "b", Modifiers::default());
    p.data.set(flat(&["a", "c"]));
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    click(&mut p, "c", ctrl());
    assert_eq!(last_report(&p), ["c"]);
}

// ---- A hidden anchor (F11) and collapses (F12) ------------------------------

#[test]
fn a_run_from_an_anchor_hidden_by_a_collapse_starts_at_its_shown_parent() {
    let mut p = build(MULTI, nested(), &["a"]);
    click(&mut p, "a1", Modifiers::default());
    // The app collapses a: the anchor a1 is hidden (and stays selected:
    // only the tree's own collapses unselect).
    p.state.controller.collapse("a");
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    click(&mut p, "c", shift());
    assert_eq!(sel(&p), ["a", "b", "c"]);
}

#[test]
fn a_left_collapse_unselects_the_rows_it_hides() {
    let mut p = build(MULTI, nested(), &["a"]);
    click(&mut p, "a1", Modifiers::default());
    click(&mut p, "a2", ctrl());
    click(&mut p, "a", ctrl());
    assert_eq!(sel(&p), ["a", "a1", "a2"]);
    press(&mut p, KeyCode::ArrowLeft, None, Modifiers::default());
    assert!(!p.state.controller.is_expanded("a"), "collapsed");
    assert_eq!(sel(&p), ["a"]);
    assert_eq!(last_report(&p), ["a"]);
}

/// A tree that does not select leaves the selection to the app: no collapse
/// (row click, chevron or Left) unselects what it hides, and nothing is
/// reported.
#[test]
fn a_tree_that_does_not_select_keeps_the_apps_selection_on_collapse() {
    let mut p = build(MULTI_NO_SELECT, nested(), &["a"]);
    p.state
        .controller
        .set_selection(["a1".to_string(), "b".to_string()]);
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    click(&mut p, "a", Modifiers::default());
    assert!(
        !p.state.controller.is_expanded("a"),
        "the click collapsed a"
    );
    assert_eq!(sel(&p), ["a1", "b"]);
    p.state.controller.expand("a");
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    click_chevron(&mut p, "a");
    assert!(
        !p.state.controller.is_expanded("a"),
        "the chevron collapsed a"
    );
    p.state.controller.expand("a");
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    press(&mut p, KeyCode::ArrowLeft, None, Modifiers::default());
    assert!(!p.state.controller.is_expanded("a"), "Left collapsed a");
    assert_eq!(sel(&p), ["a1", "b"]);
    assert!(p.changes.borrow().is_empty(), "{:?}", p.changes.borrow());
}

/// The chevron's collapse unselects the rows it hides, as Left does.
#[test]
fn a_chevron_collapse_unselects_the_rows_it_hides() {
    let mut p = build(MULTI, nested(), &["a"]);
    click(&mut p, "a1", Modifiers::default());
    click(&mut p, "b", ctrl());
    assert_eq!(sel(&p), ["a1", "b"]);
    click_chevron(&mut p, "a");
    assert!(
        !p.state.controller.is_expanded("a"),
        "the chevron collapsed a"
    );
    assert_eq!(sel(&p), ["b"]);
    assert_eq!(last_report(&p), ["b"]);
}

/// A click that collapses a row and selects it is one report, with the
/// final selection.
#[test]
fn a_collapse_click_reports_once() {
    let mut p = build(MULTI, nested(), &["a"]);
    click(&mut p, "a1", Modifiers::default());
    click(&mut p, "b", ctrl());
    let before = p.changes.borrow().len();
    click(&mut p, "a", Modifiers::default());
    assert_eq!(sel(&p), ["a"]);
    assert_eq!(p.changes.borrow()[before..], [vec!["a".to_string()]]);
}

/// Enter on an open row collapses it: the rows it hides leave the
/// selection, the row joins it, and that is one report.
#[test]
fn an_enter_collapse_drops_the_hidden_rows_and_reports_once() {
    let mut p = build(MULTI, nested(), &["a"]);
    click(&mut p, "a1", Modifiers::default());
    click(&mut p, "b", ctrl());
    click(&mut p, "a", ctrl()); // the cursor on a
    assert_eq!(sel(&p), ["a", "a1", "b"]);
    p.state.controller.deselect("a");
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let before = p.changes.borrow().len();
    press(&mut p, KeyCode::Enter, None, Modifiers::default());
    assert!(!p.state.controller.is_expanded("a"), "Enter collapsed a");
    assert_eq!(sel(&p), ["a", "b"]);
    let after = p.changes.borrow()[before..].to_vec();
    assert_eq!(after, [vec!["a".to_string(), "b".to_string()]]);
}

/// Click the chevron of the row of `value`.
fn click_chevron(p: &mut P, value: &str) {
    let chevron = {
        let doc = p.app.doc.as_ref().unwrap();
        let d = doc.borrow();
        let content = row(&p.app, value);
        d.tree.nodes[content]
            .children
            .iter()
            .copied()
            .find(|&c| has_class(&d.tree.nodes[c], "rinch-tree__chevron"))
            .expect("the chevron")
    };
    let (x, y) = centre(&p.app, chevron);
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
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
}

// ---- Reports (F10) ----------------------------------------------------------

#[test]
fn nothing_is_reported_when_the_selection_did_not_change() {
    let mut p = build(MULTI, flat(&["a", "b", "c"]), &[]);
    click(&mut p, "a", Modifiers::default());
    let n = p.changes.borrow().len();
    assert_eq!(n, 1, "the first click is reported");
    click(&mut p, "a", Modifiers::default());
    press(&mut p, KeyCode::ArrowUp, None, shift()); // already at the top
    assert_eq!(
        p.changes.borrow().len(),
        n,
        "{:?}",
        &p.changes.borrow()[n..]
    );
    press(&mut p, KeyCode::KeyA, Some("a"), ctrl());
    assert_eq!(p.changes.borrow().len(), n + 1, "Ctrl+A changed it");
    press(&mut p, KeyCode::KeyA, Some("a"), ctrl());
    assert_eq!(p.changes.borrow().len(), n + 1, "a second Ctrl+A did not");
}

#[test]
fn the_report_is_top_to_bottom() {
    let rows = ["a", "b", "c", "d", "e", "f", "g", "h"];
    let mut p = build(MULTI, flat(&rows), &[]);
    click(&mut p, "h", Modifiers::default());
    for v in rows.iter().rev().skip(1) {
        click(&mut p, v, ctrl());
    }
    assert_eq!(last_report(&p), rows);
}

// ---- The cursor (F8) --------------------------------------------------------

#[test]
fn oncursor_hears_a_shift_arrow_move() {
    let mut p = build(MULTI, flat(&["a", "b", "c"]), &[]);
    click(&mut p, "a", Modifiers::default());
    p.rested.borrow_mut().clear();
    press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
    assert_eq!(*p.rested.borrow(), ["b"], "a plain arrow is reported");
    press(&mut p, KeyCode::ArrowDown, None, shift());
    assert_eq!(p.state.cursor.get().as_deref(), Some("c"));
    assert_eq!(*p.rested.borrow(), ["b", "c"]);
}

// ---- Space, Shift+Space and Enter (F4, F9) ----------------------------------

#[test]
fn space_tells_onselect_of_a_row_it_selects() {
    let mut p = build(MULTI, flat(&["a", "b", "c"]), &[]);
    click(&mut p, "a", Modifiers::default());
    assert_eq!(*p.selects.borrow(), ["a"], "a click tells onselect");
    press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
    press(&mut p, KeyCode::Space, Some(" "), Modifiers::default());
    assert_eq!(sel(&p), ["a", "b"]);
    assert_eq!(*p.selects.borrow(), ["a", "b"]);
    // Space again unselects b, and tells onselect nothing.
    press(&mut p, KeyCode::Space, Some(" "), Modifiers::default());
    assert_eq!(sel(&p), ["a"]);
    assert_eq!(*p.selects.borrow(), ["a", "b"]);
}

/// Shift+Space with Shift held as a real keyboard reports it.
fn shift_space(p: &mut P) {
    p.app
        .handle_event(PlatformEvent::ModifiersChanged(shift()), (800, 600), 1.0);
    press(p, KeyCode::Space, Some(" "), shift());
    p.app.handle_event(
        PlatformEvent::ModifiersChanged(Modifiers::default()),
        (800, 600),
        1.0,
    );
}

#[test]
fn shift_space_selects_the_run_from_the_row_last_clicked() {
    let mut p = build(MULTI, flat(&["a", "b", "c", "d"]), &[]);
    click(&mut p, "a", Modifiers::default());
    press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
    press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
    shift_space(&mut p);
    assert_eq!(
        sel(&p),
        ["a", "b", "c"],
        "a plain arrow does not move the anchor"
    );
}

#[test]
fn shift_space_selects_the_run_from_the_row_last_toggled() {
    let mut p = build(MULTI, flat(&["a", "b", "c", "d"]), &[]);
    click(&mut p, "a", Modifiers::default());
    press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
    press(&mut p, KeyCode::Space, Some(" "), Modifiers::default());
    assert_eq!(sel(&p), ["a", "b"]);
    press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
    p.selects.borrow_mut().clear();
    shift_space(&mut p);
    assert_eq!(sel(&p), ["b", "c"], "the run starts at b, where Space was");
    assert_eq!(last_report(&p), ["b", "c"]);
    assert!(
        p.selects.borrow().is_empty(),
        "no click: {:?}",
        p.selects.borrow()
    );
}

#[test]
fn enter_keeps_a_multi_selection_and_adds_the_cursor_row() {
    let mut p = build(MULTI, flat(&["a", "b", "c"]), &[]);
    click(&mut p, "a", Modifiers::default());
    click(&mut p, "c", ctrl());
    assert_eq!(sel(&p), ["a", "c"]);
    press(&mut p, KeyCode::ArrowUp, None, Modifiers::default());
    p.selects.borrow_mut().clear();
    press(&mut p, KeyCode::Enter, None, Modifiers::default());
    assert_eq!(sel(&p), ["a", "b", "c"]);
    assert_eq!(
        *p.selects.borrow(),
        ["b"],
        "Enter tells onselect, as a click does"
    );
}

// ---- Anchors and clicks (F15) -----------------------------------------------

#[test]
fn a_ctrl_click_moves_the_anchor() {
    let mut p = build(MULTI, flat(&["a", "b", "c", "d"]), &[]);
    click(&mut p, "a", Modifiers::default());
    click(&mut p, "c", ctrl());
    click(&mut p, "d", shift());
    assert_eq!(sel(&p), ["c", "d"]);
}

#[test]
fn a_plain_click_replaces_the_selection() {
    let mut p = build(MULTI, flat(&["a", "b", "c"]), &[]);
    click(&mut p, "a", Modifiers::default());
    click(&mut p, "b", ctrl());
    click(&mut p, "c", Modifiers::default());
    assert_eq!(sel(&p), ["c"]);
    assert_eq!(last_report(&p), ["c"]);
}

#[test]
fn a_tree_without_keyboard_or_drag_selects_a_run() {
    let mut p = build(
        Opts {
            keyboard: false,
            ..MULTI
        },
        flat(&["a", "b", "c", "d"]),
        &[],
    );
    click(&mut p, "a", Modifiers::default());
    click(&mut p, "c", shift());
    assert_eq!(sel(&p), ["a", "b", "c"]);
}

#[test]
fn ctrl_a_selects_the_cursor_rows_siblings_only() {
    let mut p = build(MULTI, nested(), &["a"]);
    click(&mut p, "a1", Modifiers::default());
    press(&mut p, KeyCode::KeyA, Some("a"), ctrl());
    assert_eq!(sel(&p), ["a1", "a2"]);
}

// ---- What handle_key is offered (F7) ----------------------------------------

/// #1601: `TreeKey::below` of a row alone is the row drawn right below it,
/// an open parent's first child.
#[test]
fn below_an_open_parent_is_its_first_child() {
    let data = vec![
        TreeNodeData::new("a", "A").with_children(flat(&["a1", "a2"])),
        TreeNodeData::new("b", "B"),
    ];
    let single = Opts {
        multiple: false,
        ..MULTI
    };
    for opts in [single, MULTI] {
        let mut p = build(opts, data.clone(), &["a"]);
        click(&mut p, "a", Modifiers::default()); // a click toggles a closed
        p.state.controller.expand("a");
        p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
        press(&mut p, KeyCode::F2, None, Modifiers::default());
        let seen = p.seen.borrow().last().cloned().unwrap();
        assert_eq!(seen.1.as_deref(), Some("a1"), "multiple: {}", opts.multiple);
    }
}

/// A block of selected rows: below is the row below the block, and Down
/// lands after it (a gap in the block closes).
#[test]
fn a_non_contiguous_block_moves_down_as_one() {
    let mut p = build(MULTI, flat(&["a", "b", "c", "d", "e"]), &[]);
    // The cursor on b, the first of the block: the row below the block
    // (e) is not the row drawn below b (c).
    click(&mut p, "d", Modifiers::default());
    click(&mut p, "b", ctrl());
    assert_eq!(p.state.cursor.get().as_deref(), Some("b"));
    press(&mut p, KeyCode::F2, None, Modifiers::default());
    let seen = p.seen.borrow().last().cloned().unwrap();
    assert_eq!(seen.1.as_deref(), Some("e"));
    assert_eq!(seen.2, ["b", "d"]);
    assert_eq!(seen.3, Some((None, TreeSlot::After("e".into()))));
}

/// Shift+Space is the tree's own key, not the runtime's activation click: on
/// a disabled cursor row (which takes no click) it still runs to it.
#[test]
fn shift_space_on_a_disabled_row_runs_up_to_it() {
    let mut data = flat(&["a", "b", "c", "d"]);
    data[2] = TreeNodeData::new("c", "C").disabled(true);
    let mut p = build(MULTI, data, &[]);
    click(&mut p, "a", Modifiers::default());
    press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
    press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
    assert_eq!(p.state.cursor.get().as_deref(), Some("c"));
    shift_space(&mut p);
    assert_eq!(sel(&p), ["a", "b"]);
}

/// Trees with an app's `drag_values` (the second review of #1598), and
/// single-select controls.
mod custom_trees {
    use super::*;
    use rinch_components::DragValues;

    type Customize = Rc<dyn Fn(&mut Tree, UseTreeReturn)>;

    fn build_with(
        opts: Opts,
        initial: Vec<TreeNodeData>,
        expanded: &[&str],
        custom: Customize,
    ) -> P {
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
            let mut tree = Tree {
                data: data.get(),
                data_source: Some(Rc::new(move || data.get())),
                tree: Some(state),
                select_on_click: opts.select_on_click,
                keyboard: opts.keyboard,
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
            };
            custom(&mut tree, state);
            let tree = tree.render(scope, &[]);
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

    const SINGLE: Opts = Opts {
        multiple: false,
        ..MULTI
    };

    /// The documented whole-selection `drag_values` ("an app that drags its
    /// whole selection") carries the selection once, not once a row.
    #[test]
    fn a_whole_selection_drag_values_carries_the_selection_once() {
        let custom: Customize = Rc::new(|tree, state| {
            let dv: DragValues = Rc::new(move |_v: &str| {
                let mut s: Vec<String> = state.selected.get().into_iter().collect();
                s.sort();
                s
            });
            tree.drag_values = Some(dv);
        });
        let mut p = build_with(MULTI, flat(&["a", "b", "c", "d"]), &[], custom);
        click(&mut p, "a", Modifiers::default());
        click(&mut p, "c", ctrl());
        press(&mut p, KeyCode::F2, None, Modifiers::default());
        let seen = p.seen.borrow();
        assert_eq!(seen[0].2, ["a", "c"], "carried");
    }

    /// A single-select tree whose `drag_values` names app ids (not row
    /// values) still moves the cursor row (#1601).
    #[test]
    fn drop_for_moves_the_cursor_row_whatever_drag_values_names() {
        let custom: Customize = Rc::new(|tree, _| {
            tree.drag_values = Some(Rc::new(|v: &str| vec![format!("id:{v}")]));
        });
        let mut p = build_with(SINGLE, flat(&["a", "b", "c", "d"]), &[], custom);
        click(&mut p, "b", Modifiers::default());
        press(&mut p, KeyCode::F2, None, Modifiers::default());
        let seen = p.seen.borrow();
        assert_eq!(seen[0].2, ["id:b"], "carried is the app's");
        assert_eq!(seen[0].3, Some((None, TreeSlot::After("c".into()))));
    }

    /// The block is the selected rows in tree order, whatever order
    /// `drag_values` hands back: Down of {b, c} lands after d.
    #[test]
    fn a_block_is_in_tree_order_whatever_drag_values_returns() {
        let custom: Customize = Rc::new(|tree, _| {
            tree.drag_values = Some(Rc::new(|v: &str| {
                if v == "b" || v == "c" {
                    vec!["c".into(), "b".into()]
                } else {
                    vec![v.into()]
                }
            }));
        });
        let mut p = build_with(MULTI, flat(&["a", "b", "c", "d", "e"]), &[], custom);
        click(&mut p, "b", Modifiers::default());
        click(&mut p, "c", ctrl());
        press(&mut p, KeyCode::F2, None, Modifiers::default());
        let seen = p.seen.borrow();
        assert_eq!(seen[0].2, ["c", "b"], "carried is the app's order");
        assert_eq!(seen[0].3, Some((None, TreeSlot::After("d".into()))));
    }

    /// Single-select: Shift+Space and Enter select as before, and
    /// `onselectionchange` is never called.
    #[test]
    fn single_select_enter_and_shift_space_are_unchanged() {
        let mut p = build(SINGLE, flat(&["a", "b", "c"]), &[]);
        click(&mut p, "a", Modifiers::default());
        press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
        press(&mut p, KeyCode::Space, Some(" "), shift());
        press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
        press(&mut p, KeyCode::Enter, None, Modifiers::default());
        assert_eq!(sel(&p), ["c"]);
        assert!(p.changes.borrow().is_empty());
    }

    /// Shift+letter is type-ahead of a capital: it moves the cursor and
    /// selects nothing.
    #[test]
    fn a_shift_letter_is_type_ahead() {
        let mut p = build(MULTI, flat(&["a", "b", "c"]), &[]);
        click(&mut p, "a", Modifiers::default());
        press(&mut p, KeyCode::KeyC, Some("C"), shift());
        assert_eq!(p.state.cursor.get().as_deref(), Some("c"));
        assert_eq!(sel(&p), ["a"]);
    }

    /// Ctrl- and Shift-clicks change the selection without telling
    /// `onselect`.
    #[test]
    fn ctrl_and_shift_clicks_do_not_tell_onselect() {
        let mut p = build(MULTI, flat(&["a", "b", "c"]), &[]);
        click(&mut p, "a", Modifiers::default());
        click(&mut p, "c", ctrl());
        click(&mut p, "b", shift());
        assert_eq!(sel(&p), ["b", "c"]);
        assert_eq!(*p.selects.borrow(), ["a"]);
    }
}

/// `TreeController::set_anchor`: an app whose selection follows the cursor
/// says where the next Shift run starts.
mod set_anchor {
    use super::*;

    /// a, b, c, d; a clicked (the anchor), then Down: the cursor is on b.
    fn clicked_a_cursor_on_b(opts: Opts) -> P {
        let mut p = build(opts, flat(&["a", "b", "c", "d"]), &[]);
        click(&mut p, "a", Modifiers::default());
        press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
        assert_eq!(p.state.cursor.get().as_deref(), Some("b"));
        assert_eq!(sel(&p), ["a"], "a plain arrow selects nothing");
        p
    }

    /// The file-manager model: the app selects the cursor row and moves
    /// the anchor there, so Shift+Down runs from it, not from the row last
    /// clicked. Kills `set_anchor` as a no-op, and `anchor()` answering
    /// `None`.
    #[test]
    fn shift_down_runs_from_the_anchor_the_app_set() {
        let mut p = clicked_a_cursor_on_b(MULTI);
        p.state.controller.set_selection(["b".to_string()]);
        p.state.controller.set_anchor(Some("b"));
        assert_eq!(p.state.controller.anchor().as_deref(), Some("b"));
        press(&mut p, KeyCode::ArrowDown, None, shift());
        assert_eq!(sel(&p), ["b", "c"]);
        assert_eq!(last_report(&p), ["b", "c"]);
    }

    /// The same for a Shift-click, in a tree with no keyboard: the anchor
    /// is not the keyboard's. Kills `set_anchor` as a no-op.
    #[test]
    fn a_shift_click_runs_from_the_anchor_the_app_set_without_a_keyboard() {
        let mut p = build(
            Opts {
                keyboard: false,
                ..MULTI
            },
            flat(&["a", "b", "c", "d"]),
            &[],
        );
        click(&mut p, "a", Modifiers::default());
        p.state.controller.set_selection(["c".to_string()]);
        p.state.controller.set_anchor(Some("c"));
        click(&mut p, "d", shift());
        assert_eq!(sel(&p), ["c", "d"]);
    }

    /// `None` clears the anchor: Shift+Down runs from the cursor row, the
    /// fallback, which becomes the anchor. Kills a `set_anchor` that only
    /// writes `Some`.
    #[test]
    fn a_cleared_anchor_runs_from_the_cursor() {
        let mut p = clicked_a_cursor_on_b(MULTI);
        p.state.controller.set_anchor(None);
        assert_eq!(p.state.controller.anchor(), None);
        press(&mut p, KeyCode::ArrowDown, None, shift());
        assert_eq!(sel(&p), ["b", "c"]);
        assert_eq!(p.state.controller.anchor().as_deref(), Some("b"));
    }

    /// A value not in the data is an anchor whose row was deleted: the run
    /// starts at the cursor row. Kills a `set_anchor` that refuses (keeps
    /// the old anchor for) a value it cannot find.
    #[test]
    fn an_anchor_not_in_the_data_runs_from_the_cursor() {
        let mut p = clicked_a_cursor_on_b(MULTI);
        p.state.controller.set_anchor(Some("nowhere"));
        assert_eq!(p.state.controller.anchor().as_deref(), Some("nowhere"));
        press(&mut p, KeyCode::ArrowDown, None, shift());
        assert_eq!(sel(&p), ["b", "c"]);
        assert_eq!(p.state.controller.anchor().as_deref(), Some("b"));
    }

    /// Setting the anchor touches nothing else: no selection change, no
    /// report, no `onselect`. Kills a `set_anchor` that also selects the
    /// row (or reports).
    #[test]
    fn set_anchor_changes_and_reports_nothing() {
        let mut p = clicked_a_cursor_on_b(MULTI);
        let (changes, selects) = (p.changes.borrow().len(), p.selects.borrow().len());
        p.state.controller.set_anchor(Some("c"));
        p.app
            .handle_event(PlatformEvent::AboutToWait, (800, 600), 1.0);
        p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
        assert_eq!(sel(&p), ["a"]);
        assert_eq!(p.changes.borrow().len(), changes, "onselectionchange");
        assert_eq!(p.selects.borrow().len(), selects, "onselect");
        assert_eq!(p.state.cursor.get().as_deref(), Some("b"), "the cursor");
    }
}
