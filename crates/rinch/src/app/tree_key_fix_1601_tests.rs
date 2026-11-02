//! The Tree keyboard's behaviours the review of #1601 found untested or
//! wrong: a field inside a row keeps its keys, a redrawn cursor row takes
//! the keyboard back, the ring, `oncursor`'s rest, scrolling into view, a
//! deleted cursor row's neighbour, Space in a `multiple` tree, a tree that
//! does not select on click, the cursor moved by the app, and the ARIA a
//! screen reader reads on the focused row.

use super::*;
use std::cell::RefCell;

use rinch_components::{
    RenderTreeNode, RenderTreeNodePayload, Tree, TreeKey, TreeNodeData, UseTreeOptions,
    UseTreeReturn,
};
use rinch_core::Component;
use rinch_core::element::ValueCallback;
use rinch_core::reactive::Signal;
use rinch_platform::{KeyCode, KeyRepeat, Modifiers};

const VIEWPORT: (f32, f32) = (800.0, 600.0);

#[derive(Clone, Copy)]
struct Opts {
    multiple: bool,
    select_on_click: bool,
    delay: u32,
    /// The row whose label is an `<input>` (a rename box).
    input_on: Option<&'static str>,
    /// Put the tree in a 100px scroller.
    scroller: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            multiple: false,
            select_on_click: true,
            delay: 0,
            input_on: None,
            scroller: false,
        }
    }
}

struct F {
    app: RinchApp,
    state: UseTreeReturn,
    data: Signal<Vec<TreeNodeData>>,
    rested: Rc<RefCell<Vec<String>>>,
    seen: Rc<RefCell<Vec<String>>>,
}

/// a (open: a1, a2), b, c
fn data() -> Vec<TreeNodeData> {
    vec![
        TreeNodeData::new("a", "Apples").with_children(vec![
            TreeNodeData::new("a1", "Avocado"),
            TreeNodeData::new("a2", "Banana"),
        ]),
        TreeNodeData::new("b", "Beets"),
        TreeNodeData::new("c", "Carrots"),
    ]
}

fn build(opts: Opts, initial: Vec<TreeNodeData>) -> F {
    let rested: Rc<RefCell<Vec<String>>> = Rc::default();
    let seen: Rc<RefCell<Vec<String>>> = Rc::default();
    let signal: Rc<RefCell<Option<Signal<Vec<TreeNodeData>>>>> = Rc::default();
    let state = UseTreeReturn::new(UseTreeOptions {
        initial_expanded: ["a".to_string()].into_iter().collect(),
        multiple: opts.multiple,
        ..Default::default()
    });
    let (r, s, sig) = (rested.clone(), seen.clone(), signal.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let (r, s) = (r.clone(), s.clone());
        let data = Signal::new(initial.clone());
        *sig.borrow_mut() = Some(data);
        let render_node: Option<RenderTreeNode> = opts.input_on.map(|on| {
            let render: RenderTreeNode =
                Rc::new(move |p: &RenderTreeNodePayload, scope: &mut RenderScope| {
                    let wrap = scope.create_element("span");
                    if p.node_value == on {
                        let input = scope.create_element("input");
                        input.set_attribute("class", "rename");
                        input.set_attribute("style", "width: 120px");
                        wrap.append_child(&input);
                    } else {
                        wrap.append_child(&scope.create_text(&p.node.label));
                    }
                    wrap
                });
            render
        });
        let tree = Tree {
            data: data.get(),
            data_source: Some(Rc::new(move || data.get())),
            tree: Some(state),
            select_on_click: opts.select_on_click,
            keyboard: true,
            cursor_delay_ms: opts.delay,
            render_node,
            oncursor: Some(ValueCallback::new(move |v| r.borrow_mut().push(v))),
            handle_key: Some(Rc::new(move |key: &TreeKey| {
                s.borrow_mut().push(key.key.clone());
                false
            })),
            ..Default::default()
        }
        .render(scope, &[]);
        let root = scope.create_element("div");
        let holder = scope.create_element("div");
        holder.set_attribute("class", "scroller");
        holder.set_attribute(
            "style",
            if opts.scroller {
                "width: 300px; height: 100px; overflow: auto"
            } else {
                "width: 300px"
            },
        );
        holder.append_child(&tree);
        root.append_child(&holder);
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
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let data = signal.borrow().unwrap();
    F {
        app,
        state,
        data,
        rested,
        seen,
    }
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

/// The li of `value` (connected to the document).
fn li(app: &RinchApp, value: &str) -> u64 {
    find(app, |n| {
        n.attributes.get("data-value").map(String::as_str) == Some(value) && n.parent.is_some()
    })
}

/// The content box (the row) of `value`.
fn row(app: &RinchApp, value: &str) -> u64 {
    let li = li(app, value);
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.nodes[li]
        .children
        .iter()
        .copied()
        .find(|&c| has_class(&d.tree.nodes[c], "rinch-tree__node-content"))
        .unwrap()
}

fn attr(app: &RinchApp, id: u64, name: &str) -> Option<String> {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.nodes[id].attributes.get(name).cloned()
}

fn centre(app: &RinchApp, id: u64) -> (f32, f32) {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let (x, y, w, h) = hit_testing::painted_element_box(&d.tree, id);
    (x + w / 2.0, y + h / 2.0)
}

fn click(f: &mut F, (x, y): (f32, f32)) {
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

fn click_row(f: &mut F, value: &str) {
    let at = centre(&f.app, row(&f.app, value));
    click(f, at);
}

fn press(f: &mut F, key: KeyCode, text: Option<&str>) {
    f.app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Unknown,
        },
        (800, 600),
        1.0,
    );
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
}

fn focused(app: &RinchApp) -> Option<u64> {
    match app.focus_target {
        FocusTarget::Input(id) | FocusTarget::Node(id) => Some(id),
        _ => None,
    }
}

fn ring(app: &RinchApp) -> bool {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .nodes
        .iter()
        .any(|(_, n)| has_class(n, "rinch-tree--focused"))
}

/// A timer backend the test fires by hand, by delay.
mod timers {
    use std::cell::RefCell;
    thread_local! {
        static SCHEDULED: RefCell<Vec<(u64, u32)>> = const { RefCell::new(Vec::new()) };
    }
    pub fn manual(id: u64, delay_ms: u32) {
        SCHEDULED.with(|s| s.borrow_mut().push((id, delay_ms)));
    }
    /// Fire every armed timer of `delay_ms` (and any they arm).
    pub fn fire(delay_ms: u32) {
        for _ in 0..4 {
            let armed: Vec<(u64, u32)> = SCHEDULED.with(|s| std::mem::take(&mut *s.borrow_mut()));
            let (now, later): (Vec<_>, Vec<_>) =
                armed.into_iter().partition(|(_, ms)| *ms == delay_ms);
            SCHEDULED.with(|s| s.borrow_mut().extend(later));
            for (id, _) in now {
                rinch_core::fire_timeout(id);
            }
        }
    }
}

fn settle(f: &mut F) {
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    timers::fire(0);
    f.app
        .handle_event(PlatformEvent::AboutToWait, (800, 600), 1.0);
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    timers::fire(0);
    f.app
        .handle_event(PlatformEvent::AboutToWait, (800, 600), 1.0);
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
}

/// A rename box in a row keeps its keys: the tree neither moves its cursor
/// nor offers them to `handle_key`.
#[test]
fn a_field_inside_a_row_keeps_its_keys() {
    let mut f = build(
        Opts {
            input_on: Some("b"),
            ..Default::default()
        },
        data(),
    );
    let input = find(&f.app, |n| has_class(n, "rename"));
    // The row is the cursor row; the app opens its rename box and gives it
    // the keyboard.
    click_row(&mut f, "b");
    f.app.try_focus_input(input);
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(
        focused(&f.app),
        Some(input),
        "control: the field has the keyboard"
    );
    let seen = f.seen.borrow().len();
    press(&mut f, KeyCode::ArrowDown, None);
    press(&mut f, KeyCode::KeyC, Some("c"));
    assert_ne!(f.state.cursor.get().as_deref(), Some("c"), "type-ahead");
    assert_ne!(f.state.cursor.get().as_deref(), Some("a1"));
    assert_eq!(
        f.seen.borrow().len(),
        seen,
        "handle_key was offered a field's key"
    );
    assert_eq!(focused(&f.app), Some(input));
    // Positive control: the same keys on a row are the tree's.
    click_row(&mut f, "a1");
    press(&mut f, KeyCode::ArrowDown, None);
    assert_eq!(f.state.cursor.get().as_deref(), Some("a2"));
}

/// The cursor row redrawn while it has the keyboard (its data changed)
/// gives the keyboard to the row as drawn now.
#[test]
fn a_cursor_row_redrawn_while_it_has_the_keyboard_takes_it_back() {
    let _guard = crate::app::timer_backend_lock();
    rinch_core::set_timer_backend(timers::manual);
    let mut f = build(Opts::default(), data());
    click_row(&mut f, "b");
    let old = row(&f.app, "b");
    assert_eq!(
        focused(&f.app),
        Some(old),
        "control: the row has the keyboard"
    );
    f.data.update(|d| d[1].label = "Beetroot".into());
    settle(&mut f);
    let new = row(&f.app, "b");
    assert_ne!(new, old, "control: the row was drawn again");
    assert_eq!(
        focused(&f.app),
        Some(new),
        "the redrawn row took the keyboard back"
    );
    assert!(ring(&f.app));
}

/// The ring is on while the tree has the keyboard, and off once it leaves.
#[test]
fn the_ring_shows_while_the_tree_has_the_keyboard() {
    let _guard = crate::app::timer_backend_lock();
    rinch_core::set_timer_backend(timers::manual);
    let mut f = build(Opts::default(), data());
    assert!(!ring(&f.app));
    click_row(&mut f, "a1");
    assert!(ring(&f.app), "a clicked row gives the tree its ring");
    let away = find(&f.app, |n| has_class(n, "away"));
    let at = centre(&f.app, away);
    click(&mut f, at);
    settle(&mut f);
    assert!(!ring(&f.app), "the ring comes off with the keyboard");
}

/// `oncursor` waits for the cursor to rest: rows passed on the way are not
/// reported, the row it stops on is, once.
#[test]
fn oncursor_waits_for_the_cursor_to_rest() {
    let _guard = crate::app::timer_backend_lock();
    rinch_core::set_timer_backend(timers::manual);
    let mut f = build(
        Opts {
            delay: 120,
            ..Default::default()
        },
        data(),
    );
    click_row(&mut f, "a1");
    press(&mut f, KeyCode::ArrowDown, None);
    press(&mut f, KeyCode::ArrowDown, None);
    assert_eq!(f.state.cursor.get().as_deref(), Some("b"));
    assert!(
        f.rested.borrow().is_empty(),
        "told before resting: {:?}",
        f.rested.borrow()
    );
    timers::fire(120);
    assert_eq!(f.rested.borrow().as_slice(), ["b".to_string()]);
}

/// End in a tree in a short scroller brings the last row into view.
#[test]
fn the_cursor_row_is_scrolled_into_view() {
    let rows: Vec<TreeNodeData> = (0..50)
        .map(|i| TreeNodeData::new(format!("r{i}"), format!("Row {i}")))
        .collect();
    let mut f = build(
        Opts {
            scroller: true,
            ..Default::default()
        },
        rows,
    );
    click_row(&mut f, "r0");
    let scroller = find(&f.app, |n| has_class(n, "scroller"));
    let top = || {
        let doc = f.app.doc.as_ref().unwrap();
        let d = doc.borrow();
        d.tree.nodes[scroller].scroll_offset.1
    };
    assert_eq!(top(), 0.0, "control: not scrolled");
    press(&mut f, KeyCode::End, None);
    assert_eq!(f.state.cursor.get().as_deref(), Some("r49"));
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    assert!(
        d.tree.nodes[scroller].scroll_offset.1 > 0.0,
        "the last row was not scrolled into view"
    );
}

/// The app deletes the cursor row while it has the keyboard: the cursor and
/// the keyboard go to the row below it, and the ring stays.
#[test]
fn a_deleted_cursor_row_hands_cursor_and_keyboard_to_its_neighbour() {
    let _guard = crate::app::timer_backend_lock();
    rinch_core::set_timer_backend(timers::manual);
    let mut f = build(Opts::default(), data());
    click_row(&mut f, "b");
    f.data.update(|d| d.retain(|n| n.value != "b"));
    settle(&mut f);
    assert_eq!(f.state.cursor.get().as_deref(), Some("c"));
    assert_eq!(focused(&f.app), Some(row(&f.app, "c")));
    assert!(ring(&f.app));
    assert_eq!(
        attr(&f.app, row(&f.app, "c"), "tabindex").as_deref(),
        Some("0")
    );
    // The last row: the cursor goes up.
    f.data.update(|d| d.retain(|n| n.value != "c"));
    settle(&mut f);
    assert_eq!(f.state.cursor.get().as_deref(), Some("a2"));
    assert_eq!(focused(&f.app), Some(row(&f.app, "a2")));
}

/// In a `multiple` tree the arrows select nothing and Space toggles the
/// cursor row.
#[test]
fn space_toggles_the_cursor_row_in_a_multiple_tree() {
    let mut f = build(
        Opts {
            multiple: true,
            ..Default::default()
        },
        data(),
    );
    click_row(&mut f, "a1");
    press(&mut f, KeyCode::ArrowDown, None);
    assert_eq!(f.state.cursor.get().as_deref(), Some("a2"));
    let selected = |f: &F| {
        let mut v: Vec<String> = f.state.selected.get().into_iter().collect();
        v.sort();
        v
    };
    assert_eq!(selected(&f), ["a1"]);
    press(&mut f, KeyCode::Space, Some(" "));
    assert_eq!(selected(&f), ["a1", "a2"]);
    press(&mut f, KeyCode::Space, Some(" "));
    assert_eq!(selected(&f), ["a1"]);
}

/// A tree that does not select on click selects nothing from the keyboard.
#[test]
fn without_select_on_click_the_keys_select_nothing() {
    let mut f = build(
        Opts {
            select_on_click: false,
            ..Default::default()
        },
        data(),
    );
    click_row(&mut f, "a1");
    press(&mut f, KeyCode::ArrowDown, None);
    press(&mut f, KeyCode::Space, Some(" "));
    assert_eq!(
        f.state.cursor.get().as_deref(),
        Some("a2"),
        "control: the cursor moved"
    );
    assert!(
        f.state.selected.get().is_empty(),
        "{:?}",
        f.state.selected.get()
    );
}

/// The app moves the cursor while a row has the keyboard: the keyboard and
/// the Tab stop go with it.
#[test]
fn the_cursor_set_by_the_app_takes_the_keyboard_from_another_row() {
    let mut f = build(Opts::default(), data());
    click_row(&mut f, "a1");
    f.state.controller.set_cursor("c");
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    f.app
        .handle_event(PlatformEvent::AboutToWait, (800, 600), 1.0);
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(focused(&f.app), Some(row(&f.app, "c")));
    assert_eq!(
        attr(&f.app, row(&f.app, "c"), "tabindex").as_deref(),
        Some("0")
    );
    assert_eq!(
        attr(&f.app, row(&f.app, "a1"), "tabindex").as_deref(),
        Some("-1")
    );
}

/// The row that takes the keyboard is the treeitem, with its state; the
/// list item around it is only structure, and its group is its own.
#[test]
fn the_focused_row_is_the_treeitem() {
    let mut f = build(Opts::default(), data());
    click_row(&mut f, "a1");
    let a = row(&f.app, "a");
    let a1 = row(&f.app, "a1");
    assert_eq!(focused(&f.app), Some(a1));
    assert_eq!(attr(&f.app, a1, "role").as_deref(), Some("treeitem"));
    assert_eq!(attr(&f.app, a1, "aria-selected").as_deref(), Some("true"));
    assert_eq!(attr(&f.app, a1, "aria-level").as_deref(), Some("2"));
    assert_eq!(attr(&f.app, a, "aria-level").as_deref(), Some("1"));
    assert_eq!(attr(&f.app, a, "aria-expanded").as_deref(), Some("true"));
    assert_eq!(
        attr(&f.app, li(&f.app, "a1"), "role").as_deref(),
        Some("none")
    );
    let owns = attr(&f.app, a, "aria-owns").expect("a owns its group");
    let group = find(&f.app, |n| n.attributes.get("id") == Some(&owns));
    assert_eq!(attr(&f.app, group, "role").as_deref(), Some("group"));
}

/// The cursor row is deleted while it has the keyboard, and the user clicks
/// into a field outside the tree before the tree gives the keyboard to the
/// neighbour: the field keeps it.
#[test]
fn the_neighbour_never_takes_the_keyboard_from_a_field_the_user_went_to() {
    let _guard = crate::app::timer_backend_lock();
    rinch_core::set_timer_backend(timers::manual);
    let mut f = build(Opts::default(), data());
    click_row(&mut f, "b");
    f.data.update(|d| d.retain(|n| n.value != "b"));
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let away = find(&f.app, |n| has_class(n, "away"));
    let at = centre(&f.app, away);
    click(&mut f, at);
    assert_eq!(
        focused(&f.app),
        Some(away),
        "control: the field has the keyboard"
    );
    settle(&mut f);
    assert_eq!(
        focused(&f.app),
        Some(away),
        "the neighbour took the keyboard"
    );
    assert!(!ring(&f.app));
    assert_eq!(
        f.state.cursor.get().as_deref(),
        Some("c"),
        "the cursor still moved"
    );
}
