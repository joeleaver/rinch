//! Link activation and link hover in the desktop editor
//! (`EditorHandle::on_link_click` / `on_link_hover`), driven through the real
//! `PlatformEvent` path.
//!
//! The model half — which run a character belongs to — is pinned in
//! `rinch-editor-core` (`links.rs`), and the change-only hover bookkeeping in
//! `rinch-editor-view`. What is pinned here is the desktop half: a press or a
//! move is judged by the **character under the pointer** (never the nearest
//! caret boundary), a claimed press changes nothing, and a pointer move keeps
//! its one hit test.

use super::*;
use rinch_dom::perf::Counter;
use rinch_editor_core::{Pos, Selection};

const VP: (u32, u32) = (800, 600);

/// "go " is 1..4, "here" 4..8 (its "er" bold), " and " 8..13, "there" 13..18,
/// " end" 18..22.
const HTML: &str = r#"<p>go <a href="pimble:a/b" title="B">h<strong>er</strong>e</a> and <a href="https://x.test/">there</a> end</p><p>second line</p>"#;

struct Page {
    app: RinchApp,
    container: usize,
    handle: crate::editor::EditorHandle,
}

fn page() -> Page {
    let slot: Rc<RefCell<Option<(usize, crate::editor::EditorHandle)>>> =
        Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let (container, handle) = crate::editor::mount_editor(scope);
        handle.load_html(HTML);
        container.set_attribute(
            "style",
            "width: 400px; height: 200px; font-size: 16px; line-height: 24px; \
             font-family: sans-serif",
        );
        root.append_child(&container);
        *slot_in.borrow_mut() = Some((container.node_id().0, handle));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (container, handle) = slot.borrow_mut().take().expect("captured at mount");
    Page {
        app,
        container,
        handle,
    }
}

/// A window point `frac` of the way across the character starting at `pos`,
/// on its line's middle.
fn over_char(page: &Page, pos: usize, frac: f32) -> (f32, f32) {
    let (x0, y, h) = page
        .app
        .editor_caret_point(&page.handle, Pos(pos))
        .expect("the position has a caret");
    let (x1, _, _) = page
        .app
        .editor_caret_point(&page.handle, Pos(pos + 1))
        .expect("the next position has a caret");
    assert!(x1 > x0 + 2.0, "positive control: the character has width");
    (x0 + (x1 - x0) * frac, y + h / 2.0)
}

fn ev(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
    app.handle_event(event, VP, 1.0)
}

fn modifiers(m: Modifiers) -> PlatformEvent {
    PlatformEvent::ModifiersChanged(m)
}

fn primary() -> Modifiers {
    Modifiers {
        ctrl: !cfg!(target_os = "macos"),
        meta: cfg!(target_os = "macos"),
        ..Default::default()
    }
}

fn down(app: &mut RinchApp, (x, y): (f32, f32)) {
    let button = MouseButton::Left;
    ev(app, PlatformEvent::MouseDown { x, y, button });
}

fn up(app: &mut RinchApp, (x, y): (f32, f32)) {
    let button = MouseButton::Left;
    ev(app, PlatformEvent::MouseUp { x, y, button });
}

/// A single click: a press far from the last one, so two clicks a few pixels
/// apart are not a double-click.
fn click(app: &mut RinchApp, at: (f32, f32)) {
    app.last_click_pos = (-1000.0, -1000.0);
    down(app, at);
    up(app, at);
}

fn pointer_move(app: &mut RinchApp, (x, y): (f32, f32)) {
    ev(app, PlatformEvent::MouseMove { x, y });
}

/// Every link click as `(href, from, to, primary)`.
type Clicks = Rc<RefCell<Vec<(String, usize, usize, bool)>>>;

/// Record every link click, answering `claim`.
fn record_clicks(handle: &crate::editor::EditorHandle, claim: bool) -> Clicks {
    let seen: Clicks = Rc::default();
    let seen_in = seen.clone();
    handle.on_link_click(move |c| {
        seen_in
            .borrow_mut()
            .push((c.link.href.clone(), c.link.from.0, c.link.to.0, c.primary));
        claim
    });
    seen
}

#[test]
fn a_plain_click_on_a_link_is_reported_and_still_places_the_caret() {
    let mut p = page();
    let seen = record_clicks(&p.handle, false);
    let at = over_char(&p, 5, 0.3);
    click(&mut p.app, at);
    assert_eq!(
        *seen.borrow(),
        vec![("pimble:a/b".to_string(), 4, 8, false)],
        "one report, no modifier, the whole run (its bold middle included)"
    );
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(5)));
    assert_eq!(p.app.focus_target, FocusTarget::Editor(p.container));
}

#[test]
fn a_claimed_ctrl_click_leaves_the_selection_and_arms_no_drag() {
    let mut p = page();
    // Somewhere else first, so "unchanged" is not also "where the click was".
    p.handle.set_selection(Selection::text(Pos(19), Pos(21)));
    let seen = record_clicks(&p.handle, true);
    ev(&mut p.app, modifiers(primary()));
    let at = over_char(&p, 6, 0.5);
    down(&mut p.app, at);
    // A drag from the press would extend a selection over "there".
    let at = over_char(&p, 15, 0.5);
    pointer_move(&mut p.app, at);
    let at = over_char(&p, 15, 0.5);
    up(&mut p.app, at);
    assert_eq!(*seen.borrow(), vec![("pimble:a/b".to_string(), 4, 8, true)]);
    assert_eq!(
        p.handle.selection(),
        Selection::text(Pos(19), Pos(21)),
        "a claimed press moves nothing, and drags nothing"
    );
    assert_eq!(p.app.focus_target, FocusTarget::Editor(p.container));

    // Control: the same gesture unclaimed places the caret and drags.
    let mut c = page();
    let seen = record_clicks(&c.handle, false);
    ev(&mut c.app, modifiers(primary()));
    let at = over_char(&c, 6, 0.3);
    down(&mut c.app, at);
    let at = over_char(&c, 15, 0.5);
    pointer_move(&mut c.app, at);
    let at = over_char(&c, 15, 0.5);
    up(&mut c.app, at);
    assert_eq!(seen.borrow().len(), 1);
    assert_eq!(c.handle.selection().anchor(), Pos(6));
    assert_ne!(c.handle.selection().head(), Pos(6), "control: it dragged");
}

/// The right half of a link's last letter is on the link, and the left half
/// of the space after it is not — although the nearest caret boundary is the
/// link's end in both cases. That boundary is where a caret-based lookup
/// looks, and every mark is inclusive there.
#[test]
fn the_character_under_the_pointer_decides_not_the_nearest_caret() {
    let mut p = page();
    let seen = record_clicks(&p.handle, true);
    let at = over_char(&p, 7, 0.8);
    click(&mut p.app, at);
    assert_eq!(seen.borrow().len(), 1, "the last letter's right half");
    let at = over_char(&p, 8, 0.2);
    click(&mut p.app, at);
    assert_eq!(
        seen.borrow().len(),
        1,
        "the space after the link is not on it"
    );
    assert_eq!(
        p.handle.selection(),
        Selection::cursor(Pos(8)),
        "and a press there places the caret as always"
    );
    // Beside the end of the line: no glyph, so no link, though the caret
    // snaps to the line's end.
    let (x, y) = over_char(&p, 21, 0.5);
    click(&mut p.app, (x + 150.0, y));
    assert_eq!(seen.borrow().len(), 1, "past the line's end");
}

#[test]
fn a_double_press_selects_the_word_and_is_not_offered() {
    let mut p = page();
    let seen = record_clicks(&p.handle, true);
    let at = over_char(&p, 14, 0.5);
    click(&mut p.app, at);
    assert_eq!(seen.borrow().len(), 1, "the first press is offered");
    down(&mut p.app, at);
    up(&mut p.app, at);
    assert_eq!(seen.borrow().len(), 1, "the second is a word selection");
    assert_eq!(p.handle.selection(), Selection::text(Pos(13), Pos(18)));
}

/// Every hover call as `Some(href)` / `None`, plus the last rect.
type Hovers = Rc<RefCell<Vec<Option<(String, rinch_core::ElementBounds)>>>>;

fn record_hovers(handle: &crate::editor::EditorHandle) -> Hovers {
    let seen: Hovers = Rc::default();
    let seen_in = seen.clone();
    handle.on_link_hover(move |h| {
        seen_in
            .borrow_mut()
            .push(h.map(|h| (h.link.href.clone(), h.rect)));
    });
    seen
}

fn hrefs(seen: &Hovers) -> Vec<Option<String>> {
    seen.borrow()
        .iter()
        .map(|h| h.as_ref().map(|(href, _)| href.clone()))
        .collect()
}

#[test]
fn hover_reports_enter_change_and_leave_once_each() {
    let mut p = page();
    let seen = record_hovers(&p.handle);
    let at = over_char(&p, 1, 0.5);
    pointer_move(&mut p.app, at);
    assert!(seen.borrow().is_empty(), "plain text is no link");
    let at = over_char(&p, 4, 0.5);
    pointer_move(&mut p.app, at);
    let at = over_char(&p, 5, 0.5);
    pointer_move(&mut p.app, at);
    let at = over_char(&p, 7, 0.8);
    pointer_move(&mut p.app, at);
    assert_eq!(hrefs(&seen), vec![Some("pimble:a/b".into())], "one enter");
    let at = over_char(&p, 8, 0.2);
    pointer_move(&mut p.app, at);
    assert_eq!(hrefs(&seen).len(), 2, "one leave");
    let at = over_char(&p, 9, 0.5);
    pointer_move(&mut p.app, at);
    let at = over_char(&p, 13, 0.5);
    pointer_move(&mut p.app, at);
    let at = over_char(&p, 16, 0.5);
    pointer_move(&mut p.app, at);
    pointer_move(&mut p.app, (700.0, 500.0)); // outside the editor
    assert_eq!(
        hrefs(&seen),
        vec![
            Some("pimble:a/b".into()),
            None,
            Some("https://x.test/".into()),
            None
        ]
    );
}

#[test]
fn the_hover_rect_is_the_links_painted_run() {
    let mut p = page();
    let seen = record_hovers(&p.handle);
    let at = over_char(&p, 5, 0.5);
    pointer_move(&mut p.app, at);
    let rect = seen.borrow()[0].as_ref().expect("entered").1;
    let (start, _) = over_char(&p, 4, 0.0);
    let (end, _) = over_char(&p, 7, 1.0);
    assert!(
        (rect.x - start).abs() < 0.5 && (rect.x + rect.width - end).abs() < 0.5,
        "from the first letter's left edge to the last one's right: {rect:?} vs {start}..{end}"
    );
    assert!(
        rect.y <= at.1 && at.1 <= rect.y + rect.height && rect.height >= 16.0,
        "the line's box, around the pointer: {rect:?}"
    );
}

#[test]
fn a_move_keeps_its_one_hit_test_with_or_without_hover() {
    for with_hover in [false, true] {
        let mut p = page();
        let calls = with_hover.then(|| record_hovers(&p.handle));
        let over = over_char(&p, 5, 0.5);
        p.app.end_perf_frame();
        pointer_move(&mut p.app, over);
        let s = p.app.end_perf_frame().unwrap();
        assert_eq!(s.get(Counter::HitTests), 1, "hover {with_hover}: {s:?}");
        if let Some(calls) = calls {
            assert_eq!(calls.borrow().len(), 1, "positive control: it hovered");
        }
    }
}
