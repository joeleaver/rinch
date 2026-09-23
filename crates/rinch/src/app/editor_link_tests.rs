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
    page_with_css("")
}

/// [`page`] with an author stylesheet `css` in front of the editor.
fn page_with_css(css: &'static str) -> Page {
    let slot: Rc<RefCell<Option<(usize, crate::editor::EditorHandle)>>> =
        Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        if !css.is_empty() {
            let style = scope.create_element("style");
            style.append_child(&scope.create_text(css));
            root.append_child(&style);
        }
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

/// The debug port's `click` with the primary modifier's name follows a link:
/// the editor reads the click's modifiers from the app's held state, which
/// only `ModifiersChanged` sets, so before `click` took `modifiers` a
/// Ctrl/Cmd+click could not be driven over the port at all.
#[cfg(feature = "debug")]
#[test]
fn a_debug_click_with_the_primary_modifier_is_a_primary_link_click() {
    use rinch_debug::{DebugCommandKind, DebugResult};
    let mut p = page();
    let seen = record_clicks(&p.handle, true);
    let (x, y) = over_char(&p, 5, 0.5);
    let primary_name = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    let mut actions = Vec::new();
    let result = p.app.execute_debug_command(
        DebugCommandKind::Click {
            x,
            y,
            button: None,
            modifiers: Some(vec![primary_name.to_string()]),
        },
        &mut actions,
        1.0,
        VP,
    );
    assert!(matches!(result, DebugResult::Json { .. }));
    assert_eq!(*seen.borrow(), vec![("pimble:a/b".to_string(), 4, 8, true)]);
    assert_eq!(p.app.modifiers, Modifiers::default(), "released afterwards");

    // Control: the same click without the field is a plain one.
    p.app.last_click_pos = (-1000.0, -1000.0);
    p.app.execute_debug_command(
        DebugCommandKind::Click {
            x,
            y,
            button: None,
            modifiers: None,
        },
        &mut actions,
        1.0,
        VP,
    );
    assert_eq!(seen.borrow()[1], ("pimble:a/b".to_string(), 4, 8, false));
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

/// The pointer in a textblock's padding, above or below its text, is over no
/// character — even directly above or below a link. Parley's
/// `Cluster::from_point_exact` clamps `y` onto the first or last line, so
/// without `cluster_range_at_point`'s height check a padded block reported the
/// link on its edge line. Hovered at the link first (positive control), then
/// in the top padding, then the bottom one: two leaves, no stay.
#[test]
fn the_padding_above_and_below_a_link_is_not_on_it() {
    let mut p = page_with_css("[data-pm-editor] p { padding: 20px 0; }");
    let seen = record_hovers(&p.handle);
    let (x, y, h) = p.app.editor_caret_point(&p.handle, Pos(5)).unwrap();
    let x = x + 2.0;
    // Positive control: the padding points are inside the paragraph's box, so
    // the hit lands on the textblock (not the editor root's own padding).
    {
        let doc = p.app.doc.clone().unwrap();
        let d = doc.borrow();
        for probe in [y - 10.0, y + h + 10.0] {
            let hit = hit_test(&d.tree, x, probe).expect("a hit");
            let pm_type = d
                .tree
                .get(hit)
                .and_then(|n| n.attributes.get("data-pm-type"));
            assert!(
                pm_type.map(String::as_str) == Some("paragraph"),
                "the probe at y={probe} is in the paragraph's padding"
            );
        }
    }
    pointer_move(&mut p.app, (x, y + h / 2.0));
    assert_eq!(hrefs(&seen), vec![Some("pimble:a/b".into())], "on the link");
    pointer_move(&mut p.app, (x, y - 10.0)); // top padding, above the link
    assert_eq!(hrefs(&seen).len(), 2, "the top padding is a leave");
    pointer_move(&mut p.app, (x, y + h / 2.0));
    pointer_move(&mut p.app, (x, y + h + 10.0)); // bottom padding, below it
    assert_eq!(
        hrefs(&seen),
        vec![
            Some("pimble:a/b".into()),
            None,
            Some("pimble:a/b".into()),
            None
        ],
        "the bottom padding is a leave too"
    );
}

/// Link hover is kept per document: two `RinchApp`s on one thread (a window
/// and its DevTools panel, two embed contexts) each report their own pointer.
/// A move in B — over no link — is not a leave of the link A's pointer is on.
#[test]
fn a_move_in_another_document_does_not_leave_this_ones_link() {
    let mut a = page();
    let mut b = page();
    let seen_a = record_hovers(&a.handle);
    let at = over_char(&a, 5, 0.5);
    pointer_move(&mut a.app, at);
    assert_eq!(hrefs(&seen_a), vec![Some("pimble:a/b".into())]);
    pointer_move(&mut b.app, (700.0, 500.0)); // B's pointer, on nothing
    assert_eq!(
        hrefs(&seen_a),
        vec![Some("pimble:a/b".into())],
        "B's move is not A's leave"
    );
    pointer_move(&mut a.app, (700.0, 500.0));
    assert_eq!(hrefs(&seen_a).len(), 2, "positive control: A's own move is");
}

/// An editor inside a branch that unmounts, its hover callback reading a
/// signal the branch owns (as a tooltip's does), and one of its links hovered.
/// Hovers the link, unmounts, moves away; returns the callback's calls and
/// whether `link_hover_wanted` still answered `true` right after the unmount.
/// `strict` reads the signal with `get` (which panics once it is freed);
/// otherwise with `try_get`, so the call log itself can be inspected.
fn unmount_while_hovered(strict: bool) -> (Vec<Option<String>>, bool) {
    let visible = rinch_core::Signal::new(true);
    let calls: Rc<RefCell<Vec<Option<String>>>> = Rc::default();
    let slot: Rc<RefCell<Option<crate::editor::EditorHandle>>> = Rc::default();
    let (calls_in, slot_in) = (calls.clone(), slot.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let parent = NodeHandle::new(root.node_id(), scope.doc_weak());
        let (calls_in, slot_in) = (calls_in.clone(), slot_in.clone());
        rinch_core::show_dom(
            scope,
            &parent,
            move || visible.get(),
            move |s: &mut RenderScope| {
                let tip = rinch_core::Signal::new(0u32); // owned by this branch
                let handle = crate::editor::create_editor();
                handle.load_html(HTML);
                let container = rinch_core::Component::render(
                    &crate::editor::Editor {
                        editor: Some(handle.clone()),
                        ..Default::default()
                    },
                    s,
                    &[],
                );
                container.set_attribute(
                    "style",
                    "width: 400px; height: 200px; font-size: 16px; line-height: 24px; \
                     font-family: sans-serif",
                );
                let calls_in = calls_in.clone();
                handle.on_link_hover(move |h| {
                    if strict {
                        let _ = tip.get();
                    } else {
                        let _ = tip.try_get();
                    }
                    calls_in.borrow_mut().push(h.map(|h| h.link.href.clone()));
                });
                *slot_in.borrow_mut() = Some(handle);
                container
            },
            None::<fn(&mut RenderScope) -> NodeHandle>,
        );
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let handle = slot.borrow_mut().take().unwrap();
    let (x0, y, h) = app.editor_caret_point(&handle, Pos(5)).unwrap();
    drop(handle);
    pointer_move(&mut app, (x0 + 2.0, y + h / 2.0));
    assert_eq!(calls.borrow().len(), 1, "positive control: hover entered");

    visible.set(false);
    app.resolve_and_repaint(800.0, 600.0);
    let wanted_after_unmount = crate::editor::link_hover_wanted();
    pointer_move(&mut app, (700.0, 500.0));
    let calls = calls.borrow().clone();
    (calls, wanted_after_unmount)
}

/// Unmount is silent (#147/#183): an editor unmounted while one of its links
/// is hovered is not called back on the next move. Before the fix the
/// registry's strong handle delivered `on_link_hover(None)` into the disposed
/// scope, and this read of a freed signal panicked (review of #892, D1).
#[test]
fn unmount_while_hovered_does_not_call_back_into_freed_state() {
    let (calls, _) = unmount_while_hovered(true);
    assert_eq!(calls, vec![Some("pimble:a/b".to_string())]);
}

/// The same with a `try_get` read, so the log is visible: exactly the enter,
/// no `None` after the unmount, and the unmounted editor no longer counted in
/// `link_hover_wanted` from the unmount on — not from the next move.
#[test]
fn unmount_while_hovered_delivers_nothing_and_releases_the_count() {
    let (calls, wanted_after_unmount) = unmount_while_hovered(false);
    assert_eq!(
        calls,
        vec![Some("pimble:a/b".to_string())],
        "no `None` delivered to the unmounted editor"
    );
    assert!(
        !wanted_after_unmount,
        "the unmount itself released the editor's hover count"
    );
}
