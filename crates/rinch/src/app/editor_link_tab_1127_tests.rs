//! #1127: a press or hover on a tab in an editor textblock is on the tab, not
//! on the character after it.
//!
//! `RinchApp::editor_link_at_hit` finds the character under the pointer with
//! `cluster_range_at_point` and maps the cluster's start byte through the
//! caret map (`EditorHandle::pos_at`). While rinch-dom laid a tab out as four
//! spaces (four flat bytes), the caret map rounded a byte inside the tab to
//! its nearer side, so the right half of a tab answered for the character
//! after it: a press on a plain tab before a link followed the link, and a
//! press on a link's own trailing tab did not. Since #1122 a tab is one flat
//! byte, one cluster widened to its stop, and its start byte is the tab's own
//! byte, which the caret map puts before the tab — the tab's own character.
//!
//! These fixtures pin that: presses at fractions across the tab's painted
//! width (measured from the caret before and after it) reach the link only
//! when the tab is the link's. Chrome 153 offers a link on a press over a
//! character of the link and on no other character.

use super::hit_testing::painted_element_box;
use super::*;
use rinch_editor_core::Pos;
use std::cell::RefCell;
use std::rc::Rc;

const VP: (u32, u32) = (800, 600);
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const LINE: f32 = 24.0;
const FRACS: [f32; 6] = [0.1, 0.3, 0.45, 0.55, 0.7, 0.9];

fn idle(app: &mut RinchApp) {
    for _ in 0..3 {
        let actions = app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
        if actions.contains(&AppAction::RequestRedraw) {
            if app.has_pending_layout() {
                app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
            }
            let _ = app.build_pixels(1.0, VP, false);
        }
    }
}

struct Page {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
}

/// One paragraph in a 400px editor, Inter 16px / 24px, focused and settled.
fn page(html: &str) -> Page {
    let handle = crate::editor::create_editor();
    assert!(handle.load_html(html));
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let editor = handle_in.mount(scope);
        editor.set_attribute(
            "style",
            "width: 400px; height: 400px; font-size: 16px; line-height: 24px; \
             font-family: sans-serif",
        );
        root.append_child(&editor);
        root
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let mut page = Page { app, handle };
    page.handle.focus();
    idle(&mut page.app);
    page
}

impl Page {
    fn block(&self) -> u64 {
        self.handle.caret_address(Pos(1)).unwrap().0
    }
    /// The block's painted origin.
    fn origin(&self) -> (f32, f32) {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let b = painted_element_box(&doc.tree, self.block());
        (b.0, b.1)
    }
    /// The layout-local caret x for char offset `i`.
    fn local_x(&self, i: usize) -> f32 {
        let (block, byte) = self.handle.caret_address(Pos(i + 1)).unwrap();
        let doc = self.app.doc.as_ref().unwrap().borrow();
        doc.query_caret_position(block, byte).expect("laid out").0
    }
    /// The window point at fraction `frac` across char `i`'s advance.
    fn across(&self, i: usize, frac: f32) -> (f32, f32) {
        let (x0, x1) = (self.local_x(i), self.local_x(i + 1));
        let (bx, by) = self.origin();
        (bx + x0 + (x1 - x0) * frac, by + LINE / 2.0)
    }
    fn press(&mut self, (x, y): (f32, f32)) {
        let button = MouseButton::Left;
        self.app
            .handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
        self.app
            .handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
        idle(&mut self.app);
    }
    fn hover(&mut self, (x, y): (f32, f32)) {
        self.app
            .handle_event(PlatformEvent::MouseMove { x, y }, VP, 1.0);
        self.app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
    }
}

fn record_clicks(handle: &crate::editor::EditorHandle) -> Rc<RefCell<Vec<String>>> {
    let seen: Rc<RefCell<Vec<String>>> = Rc::default();
    let s = seen.clone();
    handle.on_link_click(move |c| {
        s.borrow_mut().push(c.link.href.clone());
        true
    });
    seen
}

/// Whether a hover at the point is on a link (the last hover callback).
fn hover_on_link(html: &str, char_index: usize, frac: f32) -> bool {
    let mut t = page(html);
    let on: Rc<RefCell<Option<bool>>> = Rc::default();
    let o = on.clone();
    t.handle
        .on_link_hover(move |h| *o.borrow_mut() = Some(h.is_some()));
    let at = t.across(char_index, frac);
    t.hover(at);
    let answer = *on.borrow();
    answer.unwrap_or(false)
}

const BEFORE_LINK: &str = "<p>ab\t<a href=\"https://x\">link</a></p>";
const LINK_TAB: &str = "<p><a href=\"https://x\">ab\t</a>cd</p>";

/// The tab is widened to its stop (17px here, about four spaces of Inter
/// 16px), so the presses across it sample both halves of a wide cluster
/// rather than one narrow space where either side's answer would agree.
#[test]
fn the_tab_is_wide() {
    let t = page(BEFORE_LINK);
    let tab = t.local_x(3) - t.local_x(2);
    assert!(tab > 12.0, "tab {tab}px");
}

/// A link right after a plain tab: no press or hover on the tab is on it.
#[test]
fn a_press_on_a_tab_before_a_link_is_not_on_the_link() {
    let mut hits = Vec::new();
    for frac in FRACS {
        let mut t = page(BEFORE_LINK);
        let seen = record_clicks(&t.handle);
        let at = t.across(2, frac);
        t.press(at);
        hits.push((frac, seen.borrow().len()));
    }
    // Positive control: a press on the link's first letter is on it.
    let mut t = page(BEFORE_LINK);
    let seen = record_clicks(&t.handle);
    let at = t.across(3, 0.5);
    t.press(at);
    assert_eq!(seen.borrow().len(), 1, "positive control: on the link");
    assert!(
        hits.iter().all(|h| h.1 == 0),
        "presses on the tab: {hits:?}"
    );
}

/// A link that ends with a tab: every press on the tab is on the link.
#[test]
fn a_press_on_a_links_trailing_tab_is_on_the_link() {
    let mut hits = Vec::new();
    for frac in FRACS {
        let mut t = page(LINK_TAB);
        let seen = record_clicks(&t.handle);
        let at = t.across(2, frac);
        t.press(at);
        hits.push((frac, seen.borrow().len()));
    }
    // Positive control: a press on the `c` after the link is not on it.
    let mut t = page(LINK_TAB);
    let seen = record_clicks(&t.handle);
    let at = t.across(3, 0.5);
    t.press(at);
    assert_eq!(seen.borrow().len(), 0, "positive control: off the link");
    assert!(
        hits.iter().all(|h| h.1 == 1),
        "presses on the tab: {hits:?}"
    );
}

/// Hover shares the path: on a plain tab before a link it is off the link,
/// on a link's own tab it is on it, across the whole tab.
#[test]
fn a_hover_on_a_tab_follows_the_tabs_own_link() {
    assert!(hover_on_link(BEFORE_LINK, 3, 0.5), "positive control");
    for frac in FRACS {
        assert!(!hover_on_link(BEFORE_LINK, 2, frac), "plain tab at {frac}");
        assert!(hover_on_link(LINK_TAB, 2, frac), "link's tab at {frac}");
    }
}
