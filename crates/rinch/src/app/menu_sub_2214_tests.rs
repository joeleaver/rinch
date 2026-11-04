//! Issue #2214: cascading submenus (`DropdownMenuSub`) in `DropdownMenu` and
//! `ContextMenu`, and the menus' own keyboard navigation and Escape (#2169,
//! #2170), on the desktop backend.
//!
//! Driven through the real event path (`handle_event`) under the real theme
//! and component stylesheets, with a manual timer backend so the delays are
//! read, not waited for. Every row is declared 30px tall and every panel's
//! width is declared, so no geometry here depends on the host's fonts.

use super::hit_testing::painted_element_box;
use super::*;
use rinch_components::{
    ContextMenu, DropdownMenu, DropdownMenuDropdown, DropdownMenuItem, DropdownMenuSub,
    DropdownMenuTarget,
};
use rinch_core::dom::NodeHandle;
use rinch_core::{Callback, Component, Signal};
use std::cell::{Cell, RefCell};
use std::sync::MutexGuard;

const VP: (u32, u32) = (800, 480);

/// Rows 30px tall with no font dependence, panels 200px wide, flyouts 170px.
const FIXTURE_CSS: &str = "
.rinch-dropdown-menu__item { height: 30px; padding: 0 8px; font-size: 12px; line-height: 30px; }
.rinch-dropdown-menu__dropdown { --rinch-dropdown-menu-width: 200px; }
.fixture-root { --rinch-dropdown-menu-sub-width: 170px; }
";

thread_local! {
    static SCHEDULED: RefCell<Vec<(u64, u32)>> = const { RefCell::new(Vec::new()) };
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn manual(id: u64, delay_ms: u32) {
    SCHEDULED.with(|s| s.borrow_mut().push((id, delay_ms)));
}

fn timer_lock() -> MutexGuard<'static, ()> {
    let guard = super::timer_backend_lock();
    rinch_core::set_timer_backend(manual);
    SCHEDULED.with(|s| s.borrow_mut().clear());
    LOG.with(|l| l.borrow_mut().clear());
    guard
}

/// The delays armed since the last call (and forget them unfired).
fn armed() -> Vec<u32> {
    SCHEDULED.with(|s| s.borrow().iter().map(|(_, d)| *d).collect())
}

/// Fire everything armed so far: the deadline arriving.
fn deadline(app: &mut RinchApp) {
    let ids: Vec<u64> = SCHEDULED.with(|s| s.borrow_mut().drain(..).map(|(id, _)| id).collect());
    for id in ids {
        rinch_core::fire_timeout(id);
    }
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
}

fn log() -> Vec<String> {
    LOG.with(|l| l.borrow().clone())
}

fn item(scope: &mut RenderScope, label: &str, class: &str) -> NodeHandle {
    let text = scope.create_text(label);
    let name = label.to_string();
    let node = DropdownMenuItem {
        onclick: Some(Callback::new(move || {
            LOG.with(|l| l.borrow_mut().push(name.clone()))
        })),
        ..Default::default()
    }
    .render(scope, &[text]);
    node.add_class(class);
    node
}

fn sub(
    scope: &mut RenderScope,
    label: &str,
    class: &str,
    props: DropdownMenuSub,
    kids: &[NodeHandle],
) -> NodeHandle {
    let node = DropdownMenuSub {
        label: label.into(),
        ..props
    }
    .render(scope, kids);
    node.add_class(class);
    node
}

/// The rows of every fixture: Alpha, New ▸ (File, Deep ▸ (Leaf, Lemma, Lx,
/// Ly, Lz), Folder), Off ▸ (disabled), Beta.
fn rows(scope: &mut RenderScope, props: impl Fn() -> DropdownMenuSub) -> Vec<NodeHandle> {
    let leaf = item(scope, "Leaf", "i-leaf");
    let leaf2 = item(scope, "Lemma", "i-leaf2");
    let lx = item(scope, "Lx", "i-lx");
    let ly = item(scope, "Ly", "i-ly");
    let lz = item(scope, "Lz", "i-lz");
    let deep = sub(scope, "Deep", "s-deep", props(), &[leaf, leaf2, lx, ly, lz]);
    let file = item(scope, "File", "i-file");
    let folder = item(scope, "Folder", "i-folder");
    let new = sub(scope, "New", "s-new", props(), &[file, deep, folder]);
    let x = item(scope, "Inner", "i-inner");
    let off = sub(
        scope,
        "Off",
        "s-off",
        DropdownMenuSub {
            disabled: true,
            ..props()
        },
        &[x],
    );
    vec![
        item(scope, "Alpha", "i-alpha"),
        new,
        off,
        item(scope, "Beta", "i-beta"),
    ]
}

fn style(app: &mut RinchApp) {
    let doc = app.doc.as_ref().unwrap();
    let mut d = doc.borrow_mut();
    d.load_css(&rinch_theme::generate_theme_css(
        &rinch_theme::Theme::default(),
    ));
    d.load_css(&rinch_components::generate_component_css());
    d.load_css(FIXTURE_CSS);
    d.recompute_all_styles_full();
}

/// A `DropdownMenu` whose 40x20 trigger is at `(x, y)`, open from the start.
fn dropdown_at(
    x: f32,
    y: f32,
    props: fn() -> DropdownMenuSub,
    panel_style: Option<&'static str>,
) -> (RinchApp, Signal<bool>) {
    let open = Signal::new(true);
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let trigger = scope.create_element("div");
        trigger.set_attribute("style", "width: 40px; height: 20px;");
        let target = DropdownMenuTarget.render(scope, &[trigger]);
        let kids = rows(scope, props);
        let dropdown = DropdownMenuDropdown.render(scope, &kids);
        if let Some(s) = panel_style {
            dropdown.set_attribute("style", s);
        }
        let menu = DropdownMenu {
            opened_fn: Some(Rc::new(move || open.get())),
            on_close: Some(Callback::new(move || {
                LOG.with(|l| l.borrow_mut().push("on_close".into()));
                open.set(false);
            })),
            ..Default::default()
        }
        .render(scope, &[target, dropdown]);
        let wrapper = scope.create_element("div");
        wrapper.set_attribute(
            "style",
            &format!("position: absolute; left: {x}px; top: {y}px; line-height: 0;"),
        );
        wrapper.set_attribute("class", "fixture-root");
        wrapper.append_child(&menu);
        wrapper
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    style(&mut app);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    (app, open)
}

fn defaults() -> DropdownMenuSub {
    DropdownMenuSub::default()
}

fn find(app: &RinchApp, class: &str) -> u64 {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .nodes
        .iter()
        .find(|(_, n)| {
            n.attributes
                .get("class")
                .is_some_and(|c| c.split_whitespace().any(|one| one == class))
        })
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no node carries {class}"))
}

/// A submenu's row (`s-new` → its trigger button).
fn row(app: &RinchApp, sub_class: &str) -> u64 {
    let s = find(app, sub_class);
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree.nodes[s].children[0]
}

fn flyout(app: &RinchApp, sub_class: &str) -> u64 {
    let s = find(app, sub_class);
    let d = app.doc.as_ref().unwrap().borrow();
    *d.tree.nodes[s]
        .children
        .iter()
        .find(|&&c| {
            d.tree.nodes[c]
                .attributes
                .get("class")
                .is_some_and(|k| k.contains("rinch-dropdown-menu__sub-dropdown"))
        })
        .unwrap()
}

fn is_open(app: &RinchApp, sub_class: &str) -> bool {
    let s = find(app, sub_class);
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree.nodes[s]
        .attributes
        .get("class")
        .is_some_and(|c| c.contains("rinch-dropdown-menu__sub--opened"))
}

/// `(left, top, right, bottom)` as painted.
fn painted(app: &RinchApp, id: u64) -> (f32, f32, f32, f32) {
    let d = app.doc.as_ref().unwrap().borrow();
    let (x, y, w, h) = painted_element_box(&d.tree, id);
    (x, y, x + w, y + h)
}

fn center(app: &RinchApp, id: u64) -> (f32, f32) {
    let (l, t, r, b) = painted(app, id);
    ((l + r) / 2.0, (t + b) / 2.0)
}

fn move_to(app: &mut RinchApp, (x, y): (f32, f32)) {
    app.handle_event(PlatformEvent::MouseMove { x, y }, VP, 1.0);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
}

fn press_at(app: &mut RinchApp, (x, y): (f32, f32), button: MouseButton) {
    app.handle_event(PlatformEvent::MouseMove { x, y }, VP, 1.0);
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
}

fn click(app: &mut RinchApp, id: u64) {
    let c = center(app, id);
    press_at(app, c, MouseButton::Left);
}

fn key(app: &mut RinchApp, key: KeyCode, text: Option<&str>) {
    let modifiers = Modifiers::default();
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers,
            repeat: KeyRepeat::Unknown,
        },
        VP,
        1.0,
    );
    app.handle_event(
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers,
        },
        VP,
        1.0,
    );
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
}

fn focused(app: &RinchApp) -> Option<u64> {
    let d = app.doc.as_ref().unwrap().borrow();
    rinch_core::dom::DomDocument::active_element(&*d).map(|n| n.0)
}

/// Hover opens after Chrome's 400 ms, not before; the flyout overlaps the
/// panel by 3px on its right, its first row level with the row.
#[test]
fn hovering_a_row_opens_its_submenu_after_the_open_delay() {
    let _t = timer_lock();
    let (mut app, _open) = dropdown_at(100.0, 40.0, defaults, None);
    let r = row(&app, "s-new");
    let at = center(&app, r);
    move_to(&mut app, at);
    assert_eq!(armed(), vec![400], "one open timer, Chrome's show_delay");
    assert!(!is_open(&app, "s-new"), "not before the delay");
    deadline(&mut app);
    assert!(is_open(&app, "s-new"), "open once the delay ran out");

    let panel = painted(&app, find(&app, "rinch-dropdown-menu__dropdown"));
    let fly = painted(&app, flyout(&app, "s-new"));
    let first = painted(&app, find(&app, "i-file"));
    let rowb = painted(&app, r);
    assert_eq!(
        fly.0,
        panel.2 - 3.0,
        "3px overlap with the panel: {fly:?} {panel:?}"
    );
    assert_eq!(fly.2 - fly.0, 170.0, "its declared width");
    assert_eq!(
        first.1, rowb.1,
        "first row level with the row that opened it"
    );
}

/// Moving off the row before the delay cancels the open.
#[test]
fn leaving_the_row_before_the_delay_cancels_the_open() {
    let _t = timer_lock();
    let (mut app, _open) = dropdown_at(100.0, 40.0, defaults, None);
    let at = center(&app, row(&app, "s-new"));
    move_to(&mut app, at);
    let at = center(&app, find(&app, "i-beta"));
    move_to(&mut app, at);
    deadline(&mut app);
    assert!(!is_open(&app, "s-new"));
}

/// A press opens at once, with no timer.
#[test]
fn pressing_a_row_opens_its_submenu_at_once() {
    let _t = timer_lock();
    let (mut app, open) = dropdown_at(100.0, 40.0, defaults, None);
    let at = row(&app, "s-new");
    click(&mut app, at);
    assert!(is_open(&app, "s-new"));
    assert!(open.get(), "the row is not an item: the menu stays open");
    assert!(log().is_empty(), "{:?}", log());
}

/// The diagonal: the pointer crossing a sibling row on its way into the
/// flyout does not close it (the close waits 400 ms and checks again); a
/// pointer that stays on the sibling does close it.
#[test]
fn a_diagonal_move_across_a_sibling_keeps_the_submenu() {
    let _t = timer_lock();
    let (mut app, _open) = dropdown_at(100.0, 40.0, defaults, None);
    let at = row(&app, "s-new");
    click(&mut app, at);
    SCHEDULED.with(|s| s.borrow_mut().clear());
    let at = center(&app, find(&app, "i-alpha"));
    move_to(&mut app, at);
    assert_eq!(armed(), vec![400], "one close timer");
    assert!(is_open(&app, "s-new"), "not closed at once");
    let at = center(&app, find(&app, "i-folder"));
    move_to(&mut app, at);
    deadline(&mut app);
    assert!(
        is_open(&app, "s-new"),
        "the pointer reached the flyout in time"
    );

    // Positive control: staying on the sibling closes it.
    let at = center(&app, find(&app, "i-beta"));
    move_to(&mut app, at);
    deadline(&mut app);
    assert!(
        !is_open(&app, "s-new"),
        "a pointer that stays on a sibling closes it"
    );
}

/// Leaving every menu keeps the submenu open (Chrome's `HandleMouseLocation`).
#[test]
fn the_pointer_leaving_the_menu_keeps_the_submenu_open() {
    let _t = timer_lock();
    let (mut app, _open) = dropdown_at(100.0, 40.0, defaults, None);
    let at = row(&app, "s-new");
    click(&mut app, at);
    move_to(&mut app, (700.0, 400.0));
    deadline(&mut app);
    assert!(is_open(&app, "s-new"));
}

/// With `safe_triangle` and no close delay, a move inside the triangle toward
/// the flyout keeps it, and one outside it closes it at once.
#[test]
fn the_safe_triangle_holds_the_submenu_with_no_close_delay() {
    let _t = timer_lock();
    fn tri() -> DropdownMenuSub {
        DropdownMenuSub {
            safe_triangle: true,
            close_delay: 0,
            ..Default::default()
        }
    }
    let (mut app, _open) = dropdown_at(100.0, 40.0, tri, None);
    let r = row(&app, "s-new");
    let (l, t, rr, b) = painted(&app, r);
    click(&mut app, r);
    move_to(&mut app, (rr - 20.0, (t + b) / 2.0));
    // Below the row, heading right toward the flyout's tall near edge.
    let fly = painted(&app, flyout(&app, "s-new"));
    assert!(
        fly.3 > b + 40.0,
        "premise: the flyout reaches below the row"
    );
    move_to(&mut app, (rr - 8.0, b + 6.0));
    assert!(is_open(&app, "s-new"), "inside the triangle");
    // Back down-left, far from the flyout: outside the triangle.
    move_to(&mut app, (l + 4.0, b + 40.0));
    assert!(!is_open(&app, "s-new"), "outside the triangle, no delay");
}

/// The keyboard: ArrowRight opens and focuses the first row; Up/Down move
/// inside the flyout; ArrowLeft/Escape close only the innermost and refocus
/// its row; three levels deep; then Escape closes the menu.
#[test]
fn the_keyboard_opens_walks_and_closes_submenus() {
    let _t = timer_lock();
    let (mut app, open) = dropdown_at(100.0, 40.0, defaults, None);
    let new_row = row(&app, "s-new");
    // Focus by the keyboard: Down picks Alpha, Down again the row.
    key(&mut app, KeyCode::ArrowDown, None);
    key(&mut app, KeyCode::ArrowDown, None);
    assert_eq!(
        focused(&app),
        Some(new_row),
        "positive control: focus landed"
    );
    key(&mut app, KeyCode::ArrowRight, None);
    assert!(is_open(&app, "s-new"));
    assert_eq!(
        focused(&app),
        Some(find(&app, "i-file")),
        "first row focused"
    );
    key(&mut app, KeyCode::ArrowDown, None);
    let deep_row = row(&app, "s-deep");
    assert_eq!(focused(&app), Some(deep_row), "Down within the flyout");
    key(&mut app, KeyCode::Enter, None);
    assert!(is_open(&app, "s-deep"), "Enter opens the third level");
    assert_eq!(focused(&app), Some(find(&app, "i-leaf")));
    key(&mut app, KeyCode::ArrowUp, None);
    assert_eq!(focused(&app), Some(find(&app, "i-lz")), "Up wraps");
    key(&mut app, KeyCode::ArrowLeft, None);
    assert!(
        !is_open(&app, "s-deep") && is_open(&app, "s-new"),
        "only the innermost"
    );
    assert_eq!(focused(&app), Some(deep_row), "focus back on its row");
    key(&mut app, KeyCode::Escape, None);
    assert!(!is_open(&app, "s-new"), "Escape closes the submenu");
    assert!(open.get(), "and only the submenu");
    assert_eq!(focused(&app), Some(new_row));
    key(&mut app, KeyCode::Escape, None);
    assert!(!open.get(), "the next Escape closes the menu");
    assert_eq!(log(), vec!["on_close".to_string()]);
}

/// The root panel's own arrows, Home/End and type-ahead (#2169).
#[test]
fn the_root_panel_answers_arrows_and_typeahead() {
    let _t = timer_lock();
    let (mut app, _open) = dropdown_at(100.0, 40.0, defaults, None);
    key(&mut app, KeyCode::ArrowDown, None);
    assert_eq!(
        focused(&app),
        Some(find(&app, "i-alpha")),
        "Down with no focus: first"
    );
    key(&mut app, KeyCode::ArrowDown, None);
    assert_eq!(focused(&app), Some(row(&app, "s-new")));
    key(&mut app, KeyCode::ArrowDown, None);
    assert_eq!(
        focused(&app),
        Some(find(&app, "i-beta")),
        "the disabled submenu row is skipped"
    );
    key(&mut app, KeyCode::ArrowDown, None);
    assert_eq!(focused(&app), Some(find(&app, "i-alpha")), "wraps");
    key(&mut app, KeyCode::End, None);
    assert_eq!(focused(&app), Some(find(&app, "i-beta")));
    key(&mut app, KeyCode::KeyA, Some("a"));
    assert_eq!(
        log(),
        vec!["Alpha".to_string(), "on_close".to_string()],
        "a single match is activated, and an item closes the menu"
    );
}

/// A disabled submenu opens by no route.
#[test]
fn a_disabled_submenu_does_not_open() {
    let _t = timer_lock();
    let (mut app, _open) = dropdown_at(100.0, 40.0, defaults, None);
    let off = row(&app, "s-off");
    let at = center(&app, off);
    move_to(&mut app, at);
    deadline(&mut app);
    click(&mut app, off);
    // A disabled row takes no focus, so its keys never reach it either.
    key(&mut app, KeyCode::ArrowRight, None);
    assert!(!is_open(&app, "s-off"));
    // Positive control: the same moves open an enabled one.
    let at = center(&app, row(&app, "s-new"));
    move_to(&mut app, at);
    deadline(&mut app);
    assert!(is_open(&app, "s-new"));
}

/// Choosing an item three levels down closes the whole tree.
#[test]
fn choosing_a_deep_item_closes_every_level() {
    let _t = timer_lock();
    let (mut app, open) = dropdown_at(100.0, 40.0, defaults, None);
    let at = row(&app, "s-new");
    click(&mut app, at);
    let at = row(&app, "s-deep");
    click(&mut app, at);
    assert!(is_open(&app, "s-deep"), "positive control: three deep");
    let at = find(&app, "i-leaf");
    click(&mut app, at);
    assert_eq!(log(), vec!["Leaf".to_string(), "on_close".to_string()]);
    assert!(!open.get());
    assert!(!is_open(&app, "s-new") && !is_open(&app, "s-deep"));
}

/// At the window's right and bottom edges the flyout flips to the panel's
/// left and shifts up, 4px inside.
#[test]
fn at_the_window_edge_the_flyout_flips_left_and_shifts_up() {
    let _t = timer_lock();
    let (mut app, _open) = dropdown_at(560.0, 300.0, defaults, None);
    // Root panel 324..466 (fits below); Deep's five rows would end at 556.
    let at = row(&app, "s-new");
    click(&mut app, at);
    let at = row(&app, "s-deep");
    click(&mut app, at);
    let panel = painted(&app, find(&app, "rinch-dropdown-menu__dropdown"));
    let fly = painted(&app, flyout(&app, "s-new"));
    assert!(panel.2 + 167.0 > 800.0, "premise: no room on the right");
    assert_eq!(
        fly.2,
        panel.0 + 3.0,
        "flipped: its right edge 3px into the panel"
    );
    let deep = painted(&app, flyout(&app, "s-deep"));
    assert!(deep.3 <= 480.0 - 4.0 + 0.5, "shifted up inside: {deep:?}");
    assert!(
        deep.0 >= 0.0 && deep.2 <= 800.0,
        "inside horizontally: {deep:?}"
    );
}

/// The parent panel clips its overflow (a capped, scrolling panel); the
/// flyout is outside that clip, painted and hit.
#[test]
fn a_clipping_panel_does_not_clip_the_flyout() {
    let _t = timer_lock();
    let (mut app, _open) = dropdown_at(
        100.0,
        40.0,
        defaults,
        Some("max-height: 100px; overflow-y: auto;"),
    );
    let at = row(&app, "s-new");
    click(&mut app, at);
    let panel = painted(&app, find(&app, "rinch-dropdown-menu__dropdown"));
    let folder = find(&app, "i-folder");
    let (fx, fy) = center(&app, folder);
    assert!(fx > panel.2, "premise: the row is outside the panel's clip");
    let hit = {
        let d = app.doc.as_ref().unwrap().borrow();
        super::hit_testing::hit_test(&d.tree, fx, fy)
    };
    let d = app.doc.as_ref().unwrap().borrow();
    let mut cur = hit;
    let mut found = false;
    while let Some(id) = cur {
        if id == folder {
            found = true;
            break;
        }
        cur = d.tree.nodes[id].parent;
    }
    assert!(found, "a point on the flyout's row hits that row: {hit:?}");
}

/// `ContextMenu`: a submenu opens in it, a deep item closes the whole menu,
/// and the menu's keys and Escape work with the focus outside it (#2170).
#[test]
fn a_context_menu_holds_submenus_and_answers_the_keyboard() {
    let _t = timer_lock();
    let opened: Rc<Cell<Option<Signal<bool>>>> = Rc::new(Cell::new(None));
    let opened_in = opened.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("class", "fixture-root");
        let target = scope.create_element("div");
        target.set_attribute("style", "width: 800px; height: 480px");
        let list = scope.create_element("div");
        for r in rows(scope, defaults) {
            list.append_child(&r);
        }
        let cm = ContextMenu::default();
        opened_in.set(Some(cm.opened));
        root.append_child(&cm.render(scope, &[target, list]));
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    style(&mut app);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let opened = opened.get().unwrap();
    // Panels in a context menu are not `DropdownMenu`'s; give them a width.
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(".rinch-context-menu__dropdown { width: 200px; }");
        d.recompute_all_styles_full();
    }

    press_at(&mut app, (100.0, 60.0), MouseButton::Right);
    assert!(opened.get(), "positive control: the right press opened it");
    key(&mut app, KeyCode::ArrowDown, None);
    assert_eq!(focused(&app), Some(find(&app, "i-alpha")));
    key(&mut app, KeyCode::ArrowDown, None);
    key(&mut app, KeyCode::ArrowRight, None);
    assert!(is_open(&app, "s-new"));
    assert_eq!(focused(&app), Some(find(&app, "i-file")));
    key(&mut app, KeyCode::Escape, None);
    assert!(!is_open(&app, "s-new") && opened.get(), "only the submenu");
    key(&mut app, KeyCode::Escape, None);
    assert!(!opened.get(), "Escape closes the context menu");

    press_at(&mut app, (100.0, 60.0), MouseButton::Right);
    let at = row(&app, "s-new");
    click(&mut app, at);
    let at = row(&app, "s-deep");
    click(&mut app, at);
    let at = find(&app, "i-leaf2");
    click(&mut app, at);
    assert_eq!(log(), vec!["Lemma".to_string()]);
    assert!(!opened.get(), "a deep item closes the context menu");
    assert!(!is_open(&app, "s-new") && !is_open(&app, "s-deep"));
}
