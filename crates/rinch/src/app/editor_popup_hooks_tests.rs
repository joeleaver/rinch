//! What an app needs to drive an autocomplete popup from the rich-text editor
//! (`EditorHandle::on_key`, `on_selection_change`, `caret_rect`), driven
//! through the real `PlatformEvent` path.
//!
//! The rules themselves are the handle's and are pinned in
//! `rinch-editor-view`. What is pinned here is the desktop half: every key a
//! window delivers to a focused editor is offered first, in the browser's
//! spelling, and a consumed one leaves the editor untouched; the selection
//! callback hears real clicks and keys once each; and `caret_rect` is where the
//! caret is painted, in window coordinates.
//!
//! Every consumed key has an unconsumed control beside it, run through the same
//! events: a key that would have done nothing anyway proves nothing.

use super::hit_testing::painted_element_box;
use super::*;
use rinch_editor_core::{Pos, Selection};

const VP: (u32, u32) = (800, 600);

struct Page {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
}

/// One editor over `<p>hello world</p><p>second</p><p></p>` (the last one
/// empty), focused by a real press after "hel". "hello world" is 1..12,
/// "second" is 14..20, the empty paragraph's caret is 22.
fn page() -> Page {
    let slot: Rc<RefCell<Option<crate::editor::EditorHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "padding: 30px 0 0 40px");
        let (container, handle) = crate::editor::mount_editor(scope);
        handle.load_html("<p>hello world</p><p>second</p><p></p>");
        container.set_attribute(
            "style",
            "width: 400px; height: 200px; font-size: 16px; line-height: 24px; \
             font-family: sans-serif",
        );
        root.append_child(&container);
        *slot_in.borrow_mut() = Some(handle);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let handle = slot.borrow_mut().take().expect("captured at mount");
    let mut page = Page { app, handle };
    let (x, y) = point_at(&page, 4);
    press(&mut page.app, x, y);
    assert_eq!(page.handle.selection(), Selection::cursor(Pos(4)));
    page
}

/// A window point just after the caret for `pos`.
fn point_at(page: &Page, pos: usize) -> (f32, f32) {
    let (x, y, h) = page
        .app
        .editor_caret_point(&page.handle, Pos(pos))
        .expect("the position has a caret");
    (x + 1.0, y + h / 2.0)
}

fn ev(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
    app.handle_event(event, VP, 1.0)
}

fn press(app: &mut RinchApp, x: f32, y: f32) {
    let button = MouseButton::Left;
    ev(app, PlatformEvent::MouseDown { x, y, button });
    ev(app, PlatformEvent::MouseUp { x, y, button });
}

fn key_with(app: &mut RinchApp, key: KeyCode, text: Option<&str>, modifiers: Modifiers) {
    ev(
        app,
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers,
            repeat: KeyRepeat::Fresh,
        },
    );
    ev(
        app,
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers,
        },
    );
}

fn key(app: &mut RinchApp, key: KeyCode) {
    key_with(app, key, None, Modifiers::default());
}

fn primary() -> Modifiers {
    Modifiers {
        ctrl: !cfg!(target_os = "macos"),
        meta: cfg!(target_os = "macos"),
        ..Default::default()
    }
}

/// Every key offered, as `(key, primary, shift)`, answering `consume(key)`.
type Offered = Rc<RefCell<Vec<(String, bool, bool)>>>;

fn offer_keys(page: &Page, consume: impl Fn(&str) -> bool + 'static) -> Offered {
    let seen: Offered = Rc::default();
    page.handle.on_key({
        let seen = seen.clone();
        move |k| {
            seen.borrow_mut()
                .push((k.key.to_string(), k.primary, k.shift));
            consume(k.key)
        }
    });
    seen
}

/// The caret overlay's painted box, in window coordinates.
fn painted_caret(app: &RinchApp) -> (f32, f32, f32, f32) {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let id = (0..d.tree.nodes.len())
        .find(|&id| {
            d.tree
                .nodes
                .get(id)
                .is_some_and(|n| n.attributes.contains_key("data-pm-caret"))
        })
        .expect("a caret overlay");
    painted_element_box(&d.tree, id)
}

#[test]
fn a_consumed_arrow_down_and_enter_leave_the_editor_alone() {
    let mut p = page();
    let seen = offer_keys(&p, |k| matches!(k, "ArrowDown" | "Enter"));
    let before = p.handle.doc();

    key(&mut p.app, KeyCode::ArrowDown);
    key(&mut p.app, KeyCode::Enter);

    assert_eq!(
        p.handle.selection(),
        Selection::cursor(Pos(4)),
        "ArrowDown did not move the caret"
    );
    assert!(
        p.handle.doc().same_ref(&before),
        "Enter did not split the paragraph"
    );
    let keys: Vec<String> = seen.borrow().iter().map(|(k, ..)| k.clone()).collect();
    assert_eq!(keys, ["ArrowDown", "Enter"]);
    assert_eq!(
        p.app.focus_target,
        FocusTarget::Editor(p.handle.container_id())
    );
}

#[test]
fn a_declined_key_is_the_editors_as_before() {
    let mut p = page();
    let seen = offer_keys(&p, |_| false);
    let before = p.handle.doc();

    key(&mut p.app, KeyCode::ArrowDown);
    assert!(
        (14..=20).contains(&p.handle.selection().head().0),
        "ArrowDown moved the caret into the second paragraph: {:?}",
        p.handle.selection()
    );
    key(&mut p.app, KeyCode::Enter);
    assert_eq!(
        p.handle.doc().child_count(),
        before.child_count() + 1,
        "Enter split the paragraph"
    );
    assert_eq!(seen.borrow().len(), 2, "both were offered first");
}

/// Printable keys arrive as the text they type, named keys by name, and the
/// space bar as `" "`, as in the browser. Consuming a printable key types
/// nothing.
#[test]
fn keys_are_offered_in_the_browsers_spelling() {
    let mut p = page();
    let seen = offer_keys(&p, |k| k == "[");
    let before = p.handle.doc();

    key_with(&mut p.app, KeyCode::Other, Some("["), Modifiers::default());
    assert!(
        p.handle.doc().same_ref(&before),
        "a consumed `[` types nothing"
    );

    let shift = Modifiers {
        shift: true,
        ..Default::default()
    };
    key_with(&mut p.app, KeyCode::KeyA, Some("A"), shift);
    key_with(&mut p.app, KeyCode::Space, Some(" "), Modifiers::default());
    key_with(&mut p.app, KeyCode::KeyB, None, primary());
    // The other of Ctrl and Meta is not the accelerator.
    let other = Modifiers {
        ctrl: cfg!(target_os = "macos"),
        meta: !cfg!(target_os = "macos"),
        ..Default::default()
    };
    key_with(&mut p.app, KeyCode::KeyK, None, other);
    key(&mut p.app, KeyCode::Escape);
    key(&mut p.app, KeyCode::Tab);

    assert_eq!(
        *seen.borrow(),
        [
            ("[".to_string(), false, false),
            ("A".to_string(), false, true),
            (" ".to_string(), false, false),
            ("b".to_string(), true, false),
            ("k".to_string(), false, false),
            ("Escape".to_string(), false, false),
            ("Tab".to_string(), false, false),
        ]
    );
    let text: String = (0..p.handle.doc().child(0).child_count())
        .filter_map(|i| p.handle.doc().child(0).child(i).text().map(str::to_string))
        .collect();
    assert_eq!(text, "helA lo world", "the declined keys typed as usual");
}

/// The link picker's shape: Enter replaces what was typed with a link from
/// inside the key callback, which also reads the geometry. Nothing may be
/// borrowed while it runs.
#[test]
fn the_key_callback_may_edit_the_editor_it_came_from() {
    let mut p = page();
    let h = p.handle.clone();
    p.handle.on_key(move |k| {
        if k.key != "Enter" {
            return false;
        }
        let head = h.selection().head();
        assert!(
            h.caret_rect(head).is_some(),
            "geometry from inside the callback"
        );
        h.set_selection(Selection::text(Pos(1), Pos(6)));
        assert!(h.toggle_link("pimble:a/b"));
        h.set_selection(Selection::cursor(Pos(6)));
        true
    });
    key(&mut p.app, KeyCode::Enter);
    assert_eq!(p.handle.doc().child_count(), 3, "Enter split nothing");
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(6)));
    p.handle.set_selection(Selection::cursor(Pos(3)));
    assert_eq!(p.handle.active_link_href().as_deref(), Some("pimble:a/b"));
}

/// Real keys and clicks report the selection once per change, and a consumed
/// key, which changes nothing, reports nothing.
#[test]
fn the_selection_callback_hears_real_keys_and_clicks_once_each() {
    let mut p = page();
    let seen: Rc<RefCell<Vec<Selection>>> = Rc::default();
    p.handle.on_selection_change({
        let (seen, h) = (seen.clone(), p.handle.clone());
        move |sel| {
            // Asking for geometry from inside the callback must not panic,
            // whatever the runtime is doing.
            let _ = h.caret_rect(sel.head());
            seen.borrow_mut().push(sel.clone());
        }
    });

    key_with(&mut p.app, KeyCode::KeyX, Some("x"), Modifiers::default());
    assert_eq!(*seen.borrow(), vec![Selection::cursor(Pos(5))], "typing");

    key(&mut p.app, KeyCode::ArrowRight);
    assert_eq!(seen.borrow().len(), 2, "an arrow");

    let (x, y) = point_at(&p, 16);
    press(&mut p.app, x, y);
    assert_eq!(seen.borrow().len(), 3, "a click");
    // A fresh single click (not the second half of a double click, which
    // would select the word).
    p.app.click_count = 0;
    let (x, y) = point_at(&p, 16);
    press(&mut p.app, x, y);
    assert_eq!(seen.borrow().len(), 3, "a click where the caret already is");

    let _keys = offer_keys(&p, |k| k == "ArrowDown");
    key(&mut p.app, KeyCode::ArrowDown);
    assert_eq!(seen.borrow().len(), 3, "a consumed key");
}

/// `caret_rect` is where the caret overlay is painted, in window coordinates,
/// for a caret in text and on a blank line; and it answers for a position the
/// caret is not at (a typed trigger's start) as well.
#[test]
fn caret_rect_is_where_the_caret_is_painted() {
    let mut p = page();
    for pos in [4, 16, 22] {
        p.handle.set_selection(Selection::cursor(Pos(pos)));
        p.app.refresh_editor_overlays();
        p.app.resolve_and_repaint(800.0, 600.0);
        let (x, y, _, h) = painted_caret(&p.app);
        let r = p.handle.caret_rect(Pos(pos)).expect("a laid-out caret");
        // Within the editor's 1px border: the desktop overlay is placed one
        // border width in from the text (it anchors to the padding box, and the
        // desktop `content_origin_inset` is zero), and boxes snap to whole
        // pixels. `caret_rect` is the text's own caret.
        assert!(
            (r.x - x).abs() <= 1.5 && (r.y - y).abs() <= 1.5 && (r.height - h).abs() < 0.5,
            "pos {pos}: caret_rect {r:?} vs painted ({x}, {y}, h {h})"
        );
        assert_eq!(r.width, 0.0);
    }
    // Not at the caret: the start of "second", left of the painted caret at 22.
    let start = p.handle.caret_rect(Pos(14)).unwrap();
    let later = p.handle.caret_rect(Pos(16)).unwrap();
    assert!(start.x < later.x && (start.y - later.y).abs() < 0.5);
    assert!(
        start.x >= 40.0 && start.y >= 30.0,
        "window, not editor, coordinates: {start:?}"
    );
    assert_eq!(p.handle.caret_rect(Pos(0)), None, "between blocks");
}

/// The link picker's timing on desktop: typing `[[` reports the selection
/// before the edited paragraph is laid out again, so `caret_rect` there has
/// no answer yet; `on_caret_moved` follows once it is laid out, and there it
/// answers where the caret is painted. A frame that moves nothing calls
/// nothing.
#[test]
fn caret_moved_comes_after_layout_with_the_painted_geometry() {
    let mut p = page();
    let h = p.handle.clone();
    let in_selection: Rc<RefCell<Vec<Option<rinch_core::ElementBounds>>>> = Rc::default();
    p.handle.on_selection_change({
        let (h, seen) = (h.clone(), in_selection.clone());
        move |sel| seen.borrow_mut().push(h.caret_rect(sel.head()))
    });
    let moved: Rc<RefCell<Vec<Option<rinch_core::ElementBounds>>>> = Rc::default();
    p.handle.on_caret_moved({
        let (h, seen) = (h.clone(), moved.clone());
        move || seen.borrow_mut().push(h.caret_rect(h.selection().head()))
    });

    key_with(&mut p.app, KeyCode::Other, Some("["), Modifiers::default());
    key_with(&mut p.app, KeyCode::Other, Some("["), Modifiers::default());
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(6)));
    assert_eq!(
        *in_selection.borrow(),
        [None, None],
        "reported before the edited paragraph is laid out again"
    );
    let last = moved
        .borrow()
        .last()
        .copied()
        .flatten()
        .expect("a caret-moved call after the layout, with geometry");
    let (x, y, _, _) = painted_caret(&p.app);
    assert!(
        (last.x - x).abs() <= 1.5 && (last.y - y).abs() <= 1.5,
        "{last:?} vs the painted caret ({x}, {y})"
    );
    // The trigger's start, in the laid-out paragraph, from the same callback.
    assert!(p.handle.caret_rect(Pos(4)).unwrap().x < last.x);

    let calls = moved.borrow().len();
    p.app.refresh_editor_overlays();
    p.app.resolve_and_repaint(800.0, 600.0);
    assert_eq!(moved.borrow().len(), calls, "a pass that moves nothing");
}

/// A dismiss-stack entry standing in for a `Modal`'s `close_on_escape`,
/// counting how often it closed.
fn modal_entry(page: &Page) -> (Rc<std::cell::Cell<u32>>, rinch_core::DismissHandle) {
    let dismissed = Rc::new(std::cell::Cell::new(0u32));
    let handle = rinch_core::push_dismiss_handler(page.app.doc_key(), {
        let dismissed = dismissed.clone();
        move || {
            dismissed.set(dismissed.get() + 1);
            true
        }
    });
    (dismissed, handle)
}

/// An autocomplete popup in an editor inside a `Modal`: the popup's `on_key`
/// claims Escape, so the popup closes and the modal stays open. That is the
/// browser's order — rinch-web's editor listener is a `document` capture
/// listener, ahead of the bubble-phase delegate that runs the dismiss stack —
/// and desktop offers the key before step 1 to match. It used to run the
/// dismiss stack first and never offer Escape at all.
#[test]
fn on_key_sees_escape_before_the_dismiss_stack() {
    let mut page = page();
    let (dismissed, _entry) = modal_entry(&page);
    let seen = offer_keys(&page, |k| k == "Escape");
    key(&mut page.app, KeyCode::Escape);
    assert_eq!(
        (seen.borrow().len(), dismissed.get()),
        (1, 0),
        "the popup takes Escape and the modal stays open"
    );
}

/// The other half: an Escape `on_key` leaves (no popup open) falls through to
/// the dismiss stack and closes the modal, and is offered exactly once.
#[test]
fn an_escape_on_key_leaves_still_closes_the_modal() {
    let mut page = page();
    let (dismissed, _entry) = modal_entry(&page);
    let seen = offer_keys(&page, |_| false);
    key(&mut page.app, KeyCode::Escape);
    assert_eq!(
        (seen.borrow().len(), dismissed.get()),
        (1, 1),
        "offered once, then the modal closes"
    );
}
