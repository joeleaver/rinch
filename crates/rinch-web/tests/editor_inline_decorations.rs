//! Fixtures from the review of #836 (browser): a decorated editor under an app's
//! `data-oncontextmenu`, and caret hits through a decoration segment.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::decoration::{Decoration, DecorationSet};
use rinch_editor_core::{Attrs, EditorState, Plugin, PluginKey, Pos, Selection};
use rinch_web::{EditorHandle, RootHandle, create_editor};
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const CONTENT: &str = "<p>Hello world one</p><p>Second paragraph here</p>";

/// Decorates "world" (7..12) unless the caret is inside it — the shape a
/// spellchecker takes, and the one that discards the pressed segment.
struct Spell;
impl Plugin for Spell {
    fn key(&self) -> PluginKey {
        PluginKey("review836.spell")
    }
    fn decorations(&self, state: &EditorState) -> DecorationSet {
        let h = state.selection.head().0;
        if (7..=12).contains(&h) {
            return DecorationSet::new(vec![]);
        }
        DecorationSet::new(vec![Decoration::inline(
            Pos(7),
            Pos(12),
            Attrs::new().with("class", "pm-spell-error"),
        )])
    }
}

struct Fx {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
    count: Rc<Cell<u32>>,
}

fn mount(app_menu: bool) -> Fx {
    let host = document().create_element("div").unwrap();
    host.set_attribute(
        "style",
        "font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px;",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let handle = create_editor();
    assert!(handle.add_plugin(Rc::new(Spell)));
    assert!(handle.load_html(CONTENT));
    let count = Rc::new(Cell::new(0u32));
    let c2 = count.clone();
    let mounted = handle.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let editor = mounted.mount(scope);
            if app_menu {
                let wrapper = scope.create_element("div");
                let id = scope.register_handler(move || c2.set(c2.get() + 1));
                wrapper.set_attribute("data-oncontextmenu", &id.0.to_string());
                wrapper.append_child(&editor);
                wrapper
            } else {
                editor
            }
        },
    );
    Fx {
        root,
        host,
        handle,
        count,
    }
}

fn under(x: f32, y: f32) -> web_sys::Element {
    document().element_from_point(x, y).unwrap()
}

fn mouse_on(t: &web_sys::Element, name: &str, x: f32, y: f32, button: i16) -> web_sys::MouseEvent {
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_button(button);
    init.set_buttons(if button == 2 { 2 } else { 1 });
    init.set_client_x(x as i32);
    init.set_client_y(y as i32);
    let ev = web_sys::MouseEvent::new_with_mouse_event_init_dict(name, &init).unwrap();
    t.dispatch_event(&ev).unwrap();
    ev
}

fn mouse(name: &str, x: f32, y: f32, button: i16) -> web_sys::MouseEvent {
    let t = under(x, y);
    mouse_on(&t, name, x, y, button)
}

fn para0() -> web_sys::Element {
    document()
        .query_selector("[data-pm-editor] p")
        .unwrap()
        .unwrap()
}

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
            } else if let Some(f) = walk(&kid, ch, seen) {
                return Some(f);
            }
        }
        None
    }
    walk(el, ch, &mut 0).unwrap()
}

/// Just right of the caret boundary before character `ch` of paragraph 0.
fn at_boundary(ch: u32) -> (f32, f32) {
    let (t, off) = text_at(&para0(), ch);
    let r = document().create_range().unwrap();
    r.set_start(&t, off).unwrap();
    r.set_end(&t, off + 1).unwrap();
    let b = r.get_bounding_client_rect();
    ((b.x() + 1.0) as f32, (b.y() + b.height() / 2.0) as f32)
}

fn centre_of(ch: u32) -> (f32, f32) {
    let (t, off) = text_at(&para0(), ch);
    let r = document().create_range().unwrap();
    r.set_start(&t, off).unwrap();
    r.set_end(&t, off + 1).unwrap();
    let b = r.get_bounding_client_rect();
    (
        (b.x() + b.width() / 2.0) as f32,
        (b.y() + b.height() / 2.0) as f32,
    )
}

#[wasm_bindgen_test]
fn an_app_claim_over_a_squiggle_wins_on_the_web() {
    let f = mount(true);
    // Focus with a left press in "Hello".
    let (x, y) = centre_of(1);
    mouse("mousedown", x, y, 0);
    mouse("mouseup", x, y, 0);
    let (x, y) = centre_of(8); // inside "world"
    assert!(
        under(x, y).has_attribute("data-pm-deco"),
        "positive control: the press lands on the decoration segment"
    );
    mouse("mousedown", x, y, 2);
    let t = under(x, y);
    let tag = t.tag_name();
    let ev = mouse_on(&t, "contextmenu", x, y, 2);
    mouse("mouseup", x, y, 2);
    assert_ne!(
        tag, "TEXTAREA",
        "the capture textarea was parked over the app's claim"
    );
    assert!(ev.default_prevented());
    assert_eq!(f.count.get(), 1);
    let h = f.handle.selection().head().0;
    assert!((8..=10).contains(&h), "caret placed in the word: {h}");
    f.root.unmount();
    f.host.remove();
}

#[wasm_bindgen_test]
fn left_presses_through_a_segment_land_on_the_right_positions() {
    let f = mount(false);
    for ch in [2u32, 6, 13] {
        // Re-read geometry each time: a press can drop the decoration.
        let (x, y) = at_boundary(ch);
        mouse("mousedown", x, y, 0);
        mouse("mouseup", x, y, 0);
        assert_eq!(
            f.handle.selection(),
            Selection::cursor(Pos(ch as usize + 1)),
            "press before char {ch}"
        );
    }
    // One press straight onto the segment while it is mounted: caret before 'w'+1.
    f.handle.set_selection(Selection::cursor(Pos(2)));
    let (x, y) = at_boundary(7);
    assert!(
        under(x, y).has_attribute("data-pm-deco"),
        "positive control: segment mounted"
    );
    mouse("mousedown", x, y, 0);
    mouse("mouseup", x, y, 0);
    assert_eq!(f.handle.selection(), Selection::cursor(Pos(8)));
    f.root.unmount();
    f.host.remove();
}
