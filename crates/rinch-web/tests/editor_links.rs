//! Link activation and link hover in the browser editor
//! (`EditorHandle::on_link_click` / `on_link_hover`).
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_links
//! ```
//!
//! An editor link is a real `<a href>` in the page. Cancelling the `mousedown`
//! does not cancel the `click`'s default action, so before this a click on
//! one followed it. What is pinned here: a primary press on a link is offered
//! to the app before the caret moves, a claimed one changes nothing, the
//! character under the pointer decides (not the nearest caret boundary),
//! hover is reported once per enter, change and leave, and no click on an
//! editor link navigates.
//!
//! Every fixture mounts a real editor through `rinch_web::mount_into` and
//! presses it with dispatched events at the element under the point, as the
//! browser would; each asserts a positive control before it relies on one.
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::{Pos, Selection};
use rinch_web::{EditorHandle, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-test-host-links";

/// Paragraph text "go " 0..3, "here" 3..7 (its "er" bold), " and " 7..12,
/// "there" 12..17, " end" 17..21; a model position is the character + 1.
/// Fragment hrefs, so a navigation this suite fails to prevent changes only
/// the page's hash, which the suite checks, and never unloads the runner.
const CONTENT: &str = "<p>go <a href=\"#editor-link\" title=\"B\">h<strong>er</strong>e</a> and \
     <a href=\"#other-link\">there</a> end</p><p>second line</p>";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

impl Fixture {
    fn mount() -> Self {
        if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
            for i in 0..stale.length() {
                if let Some(node) = stale.item(i)
                    && let Ok(el) = node.dyn_into::<web_sys::Element>()
                {
                    el.remove();
                }
            }
        }
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        host.set_attribute(
            "style",
            "font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px; width: 500px;",
        )
        .unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let handle = create_editor();
        assert!(handle.load_html(CONTENT));
        let mounted = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| mounted.mount(scope),
        );
        Self { root, host, handle }
    }

    fn paragraph(&self) -> web_sys::Element {
        document()
            .query_selector("[data-pm-editor] p")
            .unwrap()
            .expect("the first paragraph")
    }

    /// The viewport point `frac` of the way across character `ch` of the first
    /// paragraph, on its line's middle.
    fn point(&self, ch: u32, frac: f64) -> (f32, f32) {
        let (text, off) = text_at(&self.paragraph(), ch);
        let range = document().create_range().unwrap();
        range.set_start(&text, off).unwrap();
        range.set_end(&text, off + 1).unwrap();
        let r = range.get_bounding_client_rect();
        assert!(
            r.width() > 2.0,
            "positive control: character {ch} has width"
        );
        (
            (r.x() + r.width() * frac) as f32,
            (r.y() + r.height() / 2.0) as f32,
        )
    }

    fn teardown(self) {
        rinch_editor_view::set_link_hover(None, None);
        self.root.unmount();
        self.host.remove();
    }
}

/// The text node holding character `ch` of `el`'s text, and `ch`'s offset in it.
fn text_at(el: &web_sys::Element, ch: u32) -> (web_sys::Node, u32) {
    fn walk(node: &web_sys::Node, ch: u32, seen: &mut u32) -> Option<(web_sys::Node, u32)> {
        let kids = node.child_nodes();
        for i in 0..kids.length() {
            let kid = kids.item(i)?;
            if kid.node_type() == web_sys::Node::TEXT_NODE {
                let len = kid
                    .text_content()
                    .unwrap_or_default()
                    .encode_utf16()
                    .count() as u32;
                if ch < *seen + len {
                    return Some((kid, ch - *seen));
                }
                *seen += len;
            } else if let Some(found) = walk(&kid, ch, seen) {
                return Some(found);
            }
        }
        None
    }
    walk(el, ch, &mut 0).unwrap_or_else(|| panic!("no character {ch}"))
}

fn under(x: f32, y: f32) -> web_sys::Element {
    document()
        .element_from_point(x, y)
        .unwrap_or_else(|| panic!("nothing under ({x}, {y})"))
}

/// Dispatch a mouse event at whatever is under `(x, y)`, as the browser would.
fn mouse(name: &str, (x, y): (f32, f32), buttons: u16, ctrl: bool) -> web_sys::MouseEvent {
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_button(0);
    init.set_buttons(buttons);
    init.set_detail(1);
    init.set_ctrl_key(ctrl);
    init.set_client_x(x as i32);
    init.set_client_y(y as i32);
    let ev = web_sys::MouseEvent::new_with_mouse_event_init_dict(name, &init).unwrap();
    under(x, y).dispatch_event(&ev).unwrap();
    ev
}

/// A whole primary click: `mousedown`, `mouseup`, `click`. Returns the `click`.
fn click(at: (f32, f32), ctrl: bool) -> web_sys::MouseEvent {
    mouse("mousedown", at, 1, ctrl);
    mouse("mouseup", at, 0, ctrl);
    mouse("click", at, 0, ctrl)
}

fn hover_to(at: (f32, f32)) {
    mouse("mousemove", at, 0, false);
}

/// Every link click as `(href, from, to, primary)`.
type Clicks = Rc<RefCell<Vec<(String, usize, usize, bool)>>>;

/// Record every link click, answering `claim`.
fn record_clicks(handle: &EditorHandle, claim: bool) -> Clicks {
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

fn location() -> js_sys::Object {
    js_sys::Reflect::get(&web_sys::window().unwrap(), &"location".into())
        .unwrap()
        .unchecked_into()
}

fn hash() -> String {
    js_sys::Reflect::get(&location(), &"hash".into())
        .unwrap()
        .as_string()
        .unwrap()
}

/// A click on an editor link does not follow it, with or without a callback
/// and whatever the callback answers. Control: the same click on a link
/// outside the editor is left to the browser.
#[wasm_bindgen_test]
fn a_click_on_an_editor_link_never_navigates() {
    let f = Fixture::mount();
    let before = hash();
    let at = f.point(4, 0.5);
    assert!(
        under(at.0, at.1).closest("a").unwrap().is_some(),
        "positive control: the point is on the link element"
    );
    let ev = click(at, false);
    assert!(
        ev.default_prevented(),
        "no callback: the click's default is prevented"
    );
    let seen = record_clicks(&f.handle, false);
    let ev = click(f.point(5, 0.5), false);
    assert!(ev.default_prevented(), "an unclaimed click neither");
    assert_eq!(seen.borrow().len(), 1);
    assert_eq!(hash(), before, "the page did not navigate");

    let outside = document().create_element("a").unwrap();
    outside.set_attribute("href", "#outside-link").unwrap();
    outside.set_text_content(Some("outside"));
    f.host.append_child(&outside).unwrap();
    let ev = mouse("click", centre(&outside), 0, false);
    assert!(
        !ev.default_prevented(),
        "control: a link outside any editor is the browser's"
    );
    js_sys::Reflect::set(&location(), &"hash".into(), &before.into()).unwrap();
    f.teardown();
}

fn centre(el: &web_sys::Element) -> (f32, f32) {
    let r = el.get_bounding_client_rect();
    (
        (r.x() + r.width() / 2.0) as f32,
        (r.y() + r.height() / 2.0) as f32,
    )
}

#[wasm_bindgen_test]
fn a_plain_press_on_a_link_is_reported_and_still_places_the_caret() {
    let f = Fixture::mount();
    let seen = record_clicks(&f.handle, false);
    click(f.point(4, 0.3), false); // the bold 'e'
    assert_eq!(
        *seen.borrow(),
        vec![("#editor-link".to_string(), 4, 8, false)],
        "one report, no modifier, the whole run"
    );
    assert_eq!(f.handle.selection(), Selection::cursor(Pos(5)));
    f.teardown();
}

#[wasm_bindgen_test]
fn a_claimed_ctrl_press_leaves_the_selection_and_arms_no_drag() {
    let f = Fixture::mount();
    // Focus and a selection somewhere else first.
    click(f.point(18, 0.3), false);
    f.handle.set_selection(Selection::text(Pos(19), Pos(21)));
    let seen = record_clicks(&f.handle, true);
    mouse("mousedown", f.point(5, 0.5), 1, true);
    // A drag from the press would select toward "there".
    mouse("mousemove", f.point(14, 0.5), 1, true);
    mouse("mouseup", f.point(14, 0.5), 0, true);
    assert_eq!(
        *seen.borrow(),
        vec![("#editor-link".to_string(), 4, 8, true)]
    );
    assert_eq!(
        f.handle.selection(),
        Selection::text(Pos(19), Pos(21)),
        "a claimed press moves nothing, and drags nothing"
    );
    f.teardown();
}

/// Over the right half of the link's last letter `caretRangeFromPoint`
/// answers the boundary *after* it, and over the left half of the following
/// space the same boundary: the first is on the link, the second is not.
#[wasm_bindgen_test]
fn the_character_under_the_pointer_decides_not_the_nearest_caret() {
    let f = Fixture::mount();
    let seen = record_clicks(&f.handle, true);
    click(f.point(6, 0.8), false);
    assert_eq!(seen.borrow().len(), 1, "the last letter's right half");
    let before = f.handle.selection();
    click(f.point(7, 0.2), false);
    assert_eq!(
        seen.borrow().len(),
        1,
        "the space after the link is not on it"
    );
    assert_ne!(
        f.handle.selection(),
        before,
        "and the press placed the caret"
    );
    // The boundary between the two links' neighbours: the first letter of
    // "there" after a space is its own link, reported with its own href.
    click(f.point(12, 0.2), false);
    assert_eq!(seen.borrow()[1].0, "#other-link");
    f.teardown();
}

type Hovers = Rc<RefCell<Vec<Option<(String, rinch_core::ElementBounds)>>>>;

fn record_hovers(handle: &EditorHandle) -> Hovers {
    let seen: Hovers = Rc::default();
    let seen_in = seen.clone();
    handle.on_link_hover(move |h| {
        seen_in
            .borrow_mut()
            .push(h.map(|h| (h.link.href.clone(), h.rect)));
    });
    seen
}

#[wasm_bindgen_test]
fn hover_reports_enter_change_and_leave_once_each() {
    let f = Fixture::mount();
    let seen = record_hovers(&f.handle);
    hover_to(f.point(1, 0.5));
    assert!(seen.borrow().is_empty(), "plain text is no link");
    hover_to(f.point(3, 0.5));
    hover_to(f.point(4, 0.5));
    hover_to(f.point(6, 0.8));
    assert_eq!(seen.borrow().len(), 1, "one enter along the whole link");
    hover_to(f.point(7, 0.2));
    hover_to(f.point(8, 0.5));
    hover_to(f.point(13, 0.5));
    hover_to(f.point(15, 0.5));
    hover_to(centre(&f.host)); // below the text, inside the host
    let hrefs: Vec<Option<String>> = seen
        .borrow()
        .iter()
        .map(|h| h.as_ref().map(|(href, _)| href.clone()))
        .collect();
    assert_eq!(
        hrefs,
        vec![
            Some("#editor-link".into()),
            None,
            Some("#other-link".into()),
            None
        ]
    );
    // The rect is the link's run: the `<a>`s' own boxes.
    let rect = seen.borrow()[0].as_ref().unwrap().1;
    let links = document()
        .query_selector_all("[data-pm-editor] a[href='#editor-link']")
        .unwrap();
    let (mut left, mut right) = (f64::MAX, f64::MIN);
    for i in 0..links.length() {
        let r = links
            .item(i)
            .unwrap()
            .dyn_into::<web_sys::Element>()
            .unwrap()
            .get_bounding_client_rect();
        left = left.min(r.left());
        right = right.max(r.right());
    }
    assert!(
        (rect.x as f64 - left).abs() < 1.0 && ((rect.x + rect.width) as f64 - right).abs() < 1.0,
        "{rect:?} spans the link's elements {left}..{right}"
    );
    f.teardown();
}
