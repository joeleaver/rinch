//! Review of PR #1601 (Tree keyboard): fixtures for what the PR's own tests
//! do not reach. Each `#[test]` asserts the behaviour a general tree should
//! have (WAI-ARIA tree view pattern); the ones that fail on the PR head are
//! the findings.

use super::*;
use std::cell::RefCell;

use rinch_components::{Tree, TreeNodeData, UseTreeOptions, UseTreeReturn};
use rinch_core::Component;
use rinch_core::element::ValueCallback;
use rinch_dom::perf::Counter;
use rinch_platform::{KeyCode, KeyRepeat, Modifiers};

const VIEWPORT: (f32, f32) = (800.0, 600.0);

struct Fixture {
    app: RinchApp,
    state: UseTreeReturn,
    selects: Rc<RefCell<Vec<String>>>,
}

fn data() -> Vec<TreeNodeData> {
    vec![
        TreeNodeData::new("a", "Apples").with_children(vec![
            TreeNodeData::new("a1", "Avocado"),
            TreeNodeData::new("a2", "Banana").disabled(true),
        ]),
        TreeNodeData::new("b", "Beets"),
        TreeNodeData::new("c", "Carrots").with_child(TreeNodeData::new("c1", "Celery")),
    ]
}

fn fixture_with(data: Vec<TreeNodeData>, multiple: bool, expanded: &[&str]) -> Fixture {
    let selects: Rc<RefCell<Vec<String>>> = Rc::default();
    let state = UseTreeReturn::new(UseTreeOptions {
        initial_expanded: expanded.iter().map(|s| s.to_string()).collect(),
        multiple,
        ..Default::default()
    });
    let s = selects.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let s = s.clone();
        let tree = Tree {
            data: data.clone(),
            tree: Some(state),
            select_on_click: true,
            keyboard: true,
            cursor_delay_ms: 0,
            onselect: Some(ValueCallback::new(move |v| s.borrow_mut().push(v))),
            ..Default::default()
        }
        .render(scope, &[]);
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 300px; height: 400px; overflow: auto");
        root.append_child(&tree);
        let away = scope.create_element("div");
        away.set_attribute("tabindex", "0");
        away.set_attribute("class", "away");
        away.set_attribute("style", "height: 40px");
        let outer = scope.create_element("div");
        outer.append_child(&root);
        outer.append_child(&away);
        outer
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
    Fixture {
        app,
        state,
        selects,
    }
}

fn fixture() -> Fixture {
    fixture_with(data(), false, &["a"])
}

fn has_class(n: &rinch_dom::Node, class: &str) -> bool {
    n.attributes
        .get("class")
        .is_some_and(|c| c.split_whitespace().any(|one| one == class))
}

fn find(app: &RinchApp, pred: impl Fn(&rinch_dom::Node) -> bool) -> u64 {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .nodes
        .iter()
        .find(|(_, n)| pred(n))
        .map(|(id, _)| id)
        .expect("the node")
}

fn row(app: &RinchApp, value: &str) -> u64 {
    let li = find(app, |n| {
        n.attributes.get("data-value").map(String::as_str) == Some(value)
    });
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
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

fn click(f: &mut Fixture, (x, y): (f32, f32)) {
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
        f.app.handle_event(event, (800, 600), 1.0);
    }
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
}

fn press(f: &mut Fixture, key: KeyCode, text: Option<&str>, modifiers: Modifiers) {
    f.app.handle_event(
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
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
}

fn shift() -> Modifiers {
    Modifiers {
        shift: true,
        ..Default::default()
    }
}

/// R1: `*` is Shift+8 on a US keyboard (and Shift+something on most
/// layouts). WAI-ARIA: `*` expands all siblings. The tree skips `nav` when
/// Shift is held, so the main-keyboard `*` is taken as a type-ahead letter.
#[test]
fn r1_shift_eight_star_expands_the_siblings() {
    let mut f = fixture();
    let b = centre(&f.app, row(&f.app, "b"));
    click(&mut f, b);
    press(&mut f, KeyCode::Digit8, Some("*"), shift());
    assert!(
        f.state.expanded.get().contains("c"),
        "Shift+8 (`*`) expands the cursor row's siblings: {:?}",
        f.state.expanded.get()
    );
}

/// R2: in a `multiple` tree an arrow press must not add every row it
/// passes to the selection (`controller.select` does not clear when
/// `multiple`).
#[test]
fn r2_arrows_in_a_multiple_tree_do_not_accumulate_a_selection() {
    let mut f = fixture_with(data(), true, &["a"]);
    let a1 = centre(&f.app, row(&f.app, "a1"));
    click(&mut f, a1);
    press(&mut f, KeyCode::ArrowDown, None, Modifiers::default());
    press(&mut f, KeyCode::ArrowDown, None, Modifiers::default());
    press(&mut f, KeyCode::ArrowDown, None, Modifiers::default());
    assert_eq!(f.state.cursor.get().as_deref(), Some("c"));
    assert!(
        f.state.selected.get().len() <= 1,
        "the arrows selected every row they passed: {:?}",
        f.state.selected.get()
    );
}

/// R3: a disabled row is not selectable by a click (no handler); the
/// keyboard selects it.
#[test]
fn r3_the_cursor_does_not_select_a_disabled_row() {
    let mut f = fixture();
    let a1 = centre(&f.app, row(&f.app, "a1"));
    click(&mut f, a1);
    press(&mut f, KeyCode::ArrowDown, None, Modifiers::default());
    assert_eq!(f.state.cursor.get().as_deref(), Some("a2"));
    assert!(
        !f.state.selected.get().contains("a2"),
        "a2 is disabled and was selected by the keyboard"
    );
}

/// R4: the keyboard selects, but `onselect` is not called, so an app that
/// listens to `onselect` (the documented selection callback) misses every
/// keyboard selection.
#[test]
fn r4_a_keyboard_selection_tells_onselect() {
    let mut f = fixture();
    let b = centre(&f.app, row(&f.app, "b"));
    click(&mut f, b);
    assert_eq!(f.selects.borrow().as_slice(), ["b".to_string()]);
    press(&mut f, KeyCode::ArrowDown, None, Modifiers::default());
    assert!(f.state.selected.get().contains("c"));
    assert_eq!(
        f.selects.borrow().last().map(String::as_str),
        Some("c"),
        "onselect was not told of the keyboard's selection"
    );
}

/// R5: WAI-ARIA tree: one Tab stop (roving tabindex or
/// aria-activedescendant). Every row of a keyboard tree is `tabindex=0`.
#[test]
fn r5_a_keyboard_tree_is_one_tab_stop() {
    let f = fixture();
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let stops = d
        .tree
        .nodes
        .iter()
        .filter(|(_, n)| {
            has_class(n, "rinch-tree__node-content")
                && n.attributes.get("tabindex").map(String::as_str) == Some("0")
        })
        .count();
    assert_eq!(stops, 1, "{stops} rows are Tab stops");
}

/// R6: focus on one row, cursor on another (a scripted focus, or a click
/// on a row's own control): the arrows act from the focused row. (Tab can
/// no longer do it: a keyboard tree is one Tab stop, R5, so Tab leaves.)
#[test]
fn r6_an_arrow_acts_from_the_focused_row() {
    let mut f = fixture();
    let a1 = centre(&f.app, row(&f.app, "a1"));
    click(&mut f, a1);
    // Tab leaves the tree: the next stop is the box after it.
    press(&mut f, KeyCode::Tab, None, Modifiers::default());
    let away = find(&f.app, |n| has_class(n, "away"));
    assert!(
        matches!(f.app.focus_target, FocusTarget::Node(id) if id == away),
        "Tab left the tree"
    );
    // A scripted focus puts the keyboard on a2; the cursor stays on a1.
    let a2 = row(&f.app, "a2");
    f.app.try_focus_input(a2);
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(f.state.cursor.get().as_deref(), Some("a1"));
    press(&mut f, KeyCode::ArrowDown, None, Modifiers::default());
    assert_eq!(
        f.state.cursor.get().as_deref(),
        Some("b"),
        "ArrowDown from the focused row a2 goes to b"
    );
}

/// R7 (perf): one arrow press in a 1000-row tree. Reported, not a
/// pass/fail on its own: prints the effect runs.
#[test]
fn r7_arrow_cost_in_a_thousand_row_tree() {
    let rows: Vec<TreeNodeData> = (0..1000)
        .map(|i| TreeNodeData::new(format!("r{i}"), format!("Row {i}")))
        .collect();
    let mut f = fixture_with(rows, false, &[]);
    let r0 = centre(&f.app, row(&f.app, "r0"));
    click(&mut f, r0);
    let _ = f.app.end_perf_frame();
    press(&mut f, KeyCode::ArrowDown, None, Modifiers::default());
    let s = f.app.end_perf_frame().unwrap();
    eprintln!(
        "R7 effect_runs={} cascades={:?}",
        s.get(Counter::EffectRuns),
        s
    );
    assert_eq!(f.state.cursor.get().as_deref(), Some("r1"));
    // Two per row (aria-selected + the selected class), the selection
    // effects every row has; the same as a click selection on main.
    assert_eq!(s.get(Counter::EffectRuns), 2001);
}

/// R8: the cursor row is removed for good while it has the keyboard (the
/// app deleted it). `lost_row` is set and never cleared; when a row with
/// that value is drawn again later, the tree takes the keyboard from
/// wherever it is now (a field outside the tree). Also: until then the
/// focused ring (`rinch-tree--focused`) never comes off.
mod removed_cursor_row {
    use super::*;
    use rinch_core::reactive::Signal;

    thread_local! {
        static SCHEDULED: RefCell<Vec<(u64, u32)>> = const { RefCell::new(Vec::new()) };
    }
    fn manual(id: u64, delay_ms: u32) {
        SCHEDULED.with(|s| s.borrow_mut().push((id, delay_ms)));
    }
    fn fire_all() {
        for _ in 0..4 {
            let armed: Vec<(u64, u32)> = SCHEDULED.with(|s| std::mem::take(&mut *s.borrow_mut()));
            for (id, ms) in armed {
                if ms == 0 {
                    rinch_core::fire_timeout(id);
                }
            }
        }
    }

    fn focused(app: &RinchApp) -> Option<u64> {
        match app.focus_target {
            FocusTarget::Input(id) | FocusTarget::Node(id) => Some(id),
            _ => None,
        }
    }

    #[test]
    fn r8_control_without_the_delete_the_ring_comes_off() {
        let (ring, _, _) = run(false);
        assert!(!ring, "control: the ring comes off when focus leaves");
    }

    #[test]
    fn r8_a_deleted_cursor_row_drawn_again_later_does_not_steal_the_keyboard() {
        let (ring, before, after) = run(true);
        assert_eq!(
            after, before,
            "the re-added row took the keyboard from the field"
        );
        assert!(
            !ring,
            "the focused ring stayed on after the keyboard left the tree"
        );
    }

    fn settle(f: &mut Fixture) {
        f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
        fire_all();
        f.app
            .handle_event(PlatformEvent::AboutToWait, (800, 600), 1.0);
        f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
        f.app
            .handle_event(PlatformEvent::AboutToWait, (800, 600), 1.0);
        fire_all();
        f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    }

    fn run(delete: bool) -> (bool, Option<u64>, Option<u64>) {
        let _guard = crate::app::timer_backend_lock();
        rinch_core::set_timer_backend(manual);
        let rows: Rc<RefCell<Option<Signal<Vec<TreeNodeData>>>>> = Rc::default();
        let state = UseTreeReturn::new(UseTreeOptions::default());
        let r_out = rows.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let data = Signal::new(vec![
                TreeNodeData::new("a", "Apples"),
                TreeNodeData::new("b", "Beets"),
                TreeNodeData::new("c", "Carrots"),
            ]);
            *r_out.borrow_mut() = Some(data);
            let tree = Tree {
                data: data.get(),
                data_source: Some(Rc::new(move || data.get())),
                tree: Some(state),
                select_on_click: true,
                keyboard: true,
                cursor_delay_ms: 0,
                ..Default::default()
            }
            .render(scope, &[]);
            let root = scope.create_element("div");
            root.set_attribute("style", "width: 300px");
            root.append_child(&tree);
            let away = scope.create_element("div");
            away.set_attribute("tabindex", "0");
            away.set_attribute("class", "away");
            away.set_attribute("style", "height: 40px; margin-top: 20px");
            root.append_child(&away);
            root
        });
        app.mount_component(VIEWPORT.0, VIEWPORT.1);
        {
            let doc = app.doc.as_ref().unwrap();
            let mut d = doc.borrow_mut();
            d.load_css(&rinch_components::generate_component_css());
            d.recompute_all_styles_full();
        }
        app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
        let mut f = Fixture {
            app,
            state,
            selects: Rc::default(),
        };
        let data = rows.borrow().unwrap();
        let b = centre(&f.app, row(&f.app, "b"));
        click(&mut f, b);
        assert_eq!(f.state.cursor.get().as_deref(), Some("b"));

        // The app deletes b while it has the keyboard.
        if delete {
            data.update(|d| d.retain(|n| n.value != "b"));
        }
        settle(&mut f);

        // The user clicks into a field outside the tree.
        let away = find(&f.app, |n| has_class(n, "away"));
        let away_c = centre(&f.app, away);
        click(&mut f, away_c);
        settle(&mut f);
        let tree_focused = {
            let doc = f.app.doc.as_ref().unwrap();
            let d = doc.borrow();
            d.tree
                .nodes
                .iter()
                .any(|(_, n)| has_class(n, "rinch-tree--focused"))
        };
        let focused_before = focused(&f.app);
        eprintln!(
            "R8 focus on away? {:?} (away={away}) ring={tree_focused}",
            focused_before
        );

        // Later, a row with b's value comes back (undo, a sync).
        if delete {
            data.update(|d| d.insert(1, TreeNodeData::new("b", "Beets")));
        }
        settle(&mut f);
        let focused_after = focused(&f.app);
        eprintln!(
            "R8 delete={delete}: ring={tree_focused} {focused_before:?} -> {focused_after:?}"
        );
        (tree_focused, focused_before, focused_after)
    }
}
