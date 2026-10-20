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
fn row(app: &RinchApp, value: &str) -> usize {
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

#[test]
fn a_row_click_collapse_unselects_the_rows_it_hides() {
    let mut p = build(MULTI_NO_SELECT, nested(), &["a"]);
    // Selected by the app; the tree does not select on click.
    p.state
        .controller
        .set_selection(["a1".to_string(), "b".to_string()]);
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    click(&mut p, "a", Modifiers::default());
    assert!(
        !p.state.controller.is_expanded("a"),
        "the click collapsed a"
    );
    assert_eq!(sel(&p), ["b"]);
    assert_eq!(last_report(&p), ["b"]);
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

// ---- Review 2 probes (local only) --------------------------------------------
mod review2_probes {
    use super::*;
    use rinch_components::DragValues;

    type Customize = Rc<dyn Fn(&mut Tree, UseTreeReturn)>;

    fn build2(opts: Opts, initial: Vec<TreeNodeData>, expanded: &[&str], custom: Customize) -> P {
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

    /// R1: the documented `drag_values` use ("an app that drags its whole
    /// selection") now carries each selected row's whole selection.
    #[test]
    fn r1_drag_values_of_the_selection_is_not_doubled() {
        let custom: Customize = Rc::new(|tree, state| {
            let dv: DragValues = Rc::new(move |_v: &str| {
                let mut s: Vec<String> = state.selected.get().into_iter().collect();
                s.sort();
                s
            });
            tree.drag_values = Some(dv);
        });
        let mut p = build2(MULTI, flat(&["a", "b", "c", "d"]), &[], custom);
        click(&mut p, "a", Modifiers::default());
        click(&mut p, "c", ctrl());
        press(&mut p, KeyCode::ArrowUp, None, Modifiers::default()); // handle_key sees c's carried
        let seen = p.seen.borrow();
        eprintln!("R1 seen: {:?}", seen);
        assert_eq!(seen[0].2, ["a", "c"], "carried");
    }

    /// R2: a single-select tree whose drag_values names app ids (not row
    /// values): main's drop_for moved the cursor row; now nothing moves.
    #[test]
    fn r2_single_select_drop_for_with_app_ids() {
        let custom: Customize = Rc::new(|tree, _| {
            tree.drag_values = Some(Rc::new(|v: &str| vec![format!("id:{v}")]));
        });
        let opts = Opts {
            multiple: false,
            select_on_click: true,
            keyboard: true,
        };
        let mut p = build2(opts, flat(&["a", "b", "c", "d"]), &[], custom);
        click(&mut p, "b", Modifiers::default());
        press(&mut p, KeyCode::ArrowLeft, None, Modifiers::default());
        let seen = p.seen.borrow();
        eprintln!("R2 seen: {:?}", seen);
        assert_eq!(seen[0].3, Some((None, TreeSlot::After("c".into()))));
    }

    /// R3: drag_values in drag order (not tree order) — Down of {b,c} must
    /// land after d.
    #[test]
    fn r3_block_order_from_drag_values() {
        let custom: Customize = Rc::new(|tree, _| {
            tree.drag_values = Some(Rc::new(|v: &str| {
                if v == "b" || v == "c" {
                    vec!["c".into(), "b".into()]
                } else {
                    vec![v.into()]
                }
            }));
        });
        let opts = Opts {
            multiple: false,
            select_on_click: true,
            keyboard: true,
        };
        let mut p = build2(opts, flat(&["a", "b", "c", "d", "e"]), &[], custom);
        click(&mut p, "b", Modifiers::default());
        press(&mut p, KeyCode::ArrowLeft, None, Modifiers::default());
        let seen = p.seen.borrow();
        eprintln!("R3 seen: {:?}", seen);
        assert_eq!(seen[0].3, Some((None, TreeSlot::After("d".into()))));
    }

    /// R4: a tree that does not select (the app owns the selection) keeps
    /// what the app selected when a Left collapse hides it, as on main.
    #[test]
    fn r4_no_select_tree_keeps_the_apps_selection_on_collapse() {
        let mut p = build(MULTI_NO_SELECT, nested(), &["a"]);
        p.state
            .controller
            .set_selection(["a1".to_string(), "b".to_string()]);
        p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
        // Chevron-free: focus a via click (no select), Left collapses it.
        click(&mut p, "b", Modifiers::default());
        press(&mut p, KeyCode::ArrowUp, None, Modifiers::default());
        press(&mut p, KeyCode::ArrowUp, None, Modifiers::default());
        press(&mut p, KeyCode::ArrowUp, None, Modifiers::default());
        eprintln!("R4 cursor {:?}", p.state.cursor.get());
        press(&mut p, KeyCode::ArrowLeft, None, Modifiers::default());
        eprintln!(
            "R4 expanded a: {} sel {:?} reports {:?}",
            p.state.controller.is_expanded("a"),
            sel(&p),
            p.changes.borrow()
        );
        assert!(!p.state.controller.is_expanded("a"));
        assert_eq!(sel(&p), ["a1", "b"]);
        assert!(
            p.changes.borrow().is_empty(),
            "a tree that selects nothing reports nothing"
        );
    }

    /// R5: one click is one report (a collapse click that also selects).
    #[test]
    fn r5_a_collapse_click_reports_once() {
        let mut p = build(MULTI, nested(), &["a"]);
        click(&mut p, "a1", Modifiers::default());
        click(&mut p, "b", ctrl());
        let before = p.changes.borrow().len();
        click(&mut p, "a", Modifiers::default());
        let after: Vec<_> = p.changes.borrow()[before..].to_vec();
        eprintln!("R5 reports {:?}", after);
        assert_eq!(sel(&p), ["a"]);
        assert_eq!(after, [vec!["a".to_string()]]);
    }

    /// R6: Enter on an open parent (expand_on_click) — one report.
    #[test]
    fn r6_enter_collapse_reports_once() {
        let mut p = build(MULTI, nested(), &["a"]);
        click(&mut p, "a1", Modifiers::default());
        click(&mut p, "a", ctrl());
        let before = p.changes.borrow().len();
        press(&mut p, KeyCode::Enter, None, Modifiers::default());
        let after: Vec<_> = p.changes.borrow()[before..].to_vec();
        eprintln!(
            "R6 sel {:?} reports {:?} expanded {}",
            sel(&p),
            after,
            p.state.controller.is_expanded("a")
        );
        assert!(after.len() <= 1, "{after:?}");
    }

    /// R7: single-select click/Enter/Space unchanged (control).
    #[test]
    fn r7_single_select_enter_and_shift_space() {
        let opts = Opts {
            multiple: false,
            select_on_click: true,
            keyboard: true,
        };
        let mut p = build(opts, flat(&["a", "b", "c"]), &[]);
        click(&mut p, "a", Modifiers::default());
        press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
        press(&mut p, KeyCode::Space, Some(" "), shift());
        press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
        press(&mut p, KeyCode::Enter, None, Modifiers::default());
        eprintln!(
            "R7 sel {:?} selects {:?} changes {:?}",
            sel(&p),
            p.selects.borrow(),
            p.changes.borrow()
        );
        assert_eq!(sel(&p), ["c"]);
        assert!(p.changes.borrow().is_empty());
    }

    /// R8: Shift+letter (type-ahead of a capital) in a multiple tree.
    #[test]
    fn r8_shift_letter_typeahead() {
        let mut p = build(MULTI, flat(&["a", "b", "c"]), &[]);
        click(&mut p, "a", Modifiers::default());
        press(&mut p, KeyCode::KeyC, Some("C"), shift());
        eprintln!("R8 cursor {:?} sel {:?}", p.state.cursor.get(), sel(&p));
        assert_eq!(p.state.cursor.get().as_deref(), Some("c"));
        assert_eq!(sel(&p), ["a"]);
    }

    /// R9: Ctrl-click selecting a row: is onselect told? (main: yes, a click)
    #[test]
    fn r9_ctrl_click_onselect() {
        let mut p = build(MULTI, flat(&["a", "b", "c"]), &[]);
        click(&mut p, "a", Modifiers::default());
        click(&mut p, "c", ctrl());
        click(&mut p, "b", shift());
        eprintln!("R9 sel {:?} selects {:?}", sel(&p), p.selects.borrow());
    }
}
