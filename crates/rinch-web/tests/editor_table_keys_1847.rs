//! #1847, review of #1931: Shift+arrow and Shift+click across table cells on
//! the web input path (keydown on the capture textarea, a press with Shift
//! held), as prosemirror-tables' `shiftArrow` selects cells.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_table_keys_1847
//! ```
//!
//! Positions: `p(x)` 0..3; the table's cells at 5 ("ab", text 7..9), 11
//! ("cd", 13..15), 19 ("ef", 21..23), 25 ("gh", 27..29); `p(zw)` 33..37.
#![cfg(target_arch = "wasm32")]

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

const GRID: &str = "<p>x</p><table><tr><td><p>ab</p></td><td><p>cd</p></td></tr>\
                    <tr><td><p>ef</p></td><td><p>gh</p></td></tr></table><p>zw</p>";

struct F {
    _root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

impl Drop for F {
    fn drop(&mut self) {
        self.host.remove();
    }
}

impl F {
    fn new() -> Self {
        let host = document().create_element("div").unwrap();
        host.set_attribute(
            "style",
            "font-family: monospace; font-size: 16px; line-height: 24px; \
             padding: 20px; width: 300px",
        )
        .unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let handle = create_editor();
        assert!(handle.load_html(GRID));
        let m = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |s: &mut RenderScope| m.mount(s),
        );
        let f = Self {
            _root: root,
            host,
            handle,
        };
        // Focus: a press in the "ab" cell.
        f.press(0, false);
        assert!(
            document().active_element().as_deref() == Some(f.capture().as_ref()),
            "focus control"
        );
        f
    }
    fn capture(&self) -> web_sys::HtmlTextAreaElement {
        document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap()
    }
    /// A press near the start of the `i`th cell's paragraph.
    fn press(&self, i: u32, shift: bool) {
        let cell = self
            .host
            .query_selector_all("td p")
            .unwrap()
            .item(i)
            .unwrap()
            .dyn_into::<web_sys::Element>()
            .unwrap();
        let r = cell.get_bounding_client_rect();
        let (x, y) = ((r.x() + 1.0) as f32, (r.y() + r.height() / 2.0) as f32);
        mouse("mousedown", x, y, shift);
        mouse("mouseup", x, y, shift);
    }
    fn shift_key(&self, key: &str) {
        let init = web_sys::KeyboardEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_key(key);
        init.set_code(key);
        init.set_shift_key(true);
        let ev =
            web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        self.capture().dispatch_event(&ev).unwrap();
    }
}

fn mouse(name: &str, x: f32, y: f32, shift: bool) {
    let target = document().element_from_point(x, y).unwrap();
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_button(0);
    init.set_buttons(if name == "mouseup" { 0 } else { 1 });
    init.set_detail(1);
    init.set_shift_key(shift);
    init.set_client_x(x as i32);
    init.set_client_y(y as i32);
    let ev = web_sys::MouseEvent::new_with_mouse_event_init_dict(name, &init).unwrap();
    target.dispatch_event(&ev).unwrap();
}

#[wasm_bindgen_test]
fn shift_arrows_select_cells_on_the_web() {
    let f = F::new();
    f.handle.set_selection(Selection::cursor(Pos(8)));
    f.shift_key("ArrowRight");
    assert_eq!(f.handle.selection(), Selection::text(Pos(8), Pos(9)));
    f.shift_key("ArrowRight");
    assert_eq!(f.handle.selection(), Selection::cell(Pos(5), Pos(11)));
    f.shift_key("ArrowDown");
    assert_eq!(f.handle.selection(), Selection::cell(Pos(5), Pos(25)));
    f.shift_key("ArrowLeft");
    assert_eq!(f.handle.selection(), Selection::cell(Pos(5), Pos(19)));
    f.shift_key("ArrowUp");
    assert_eq!(f.handle.selection(), Selection::cell(Pos(5), Pos(5)));
}

#[wasm_bindgen_test]
fn shift_down_from_a_cells_only_line_selects_the_cell_below_on_the_web() {
    let f = F::new();
    f.handle.set_selection(Selection::cursor(Pos(8)));
    f.shift_key("ArrowDown");
    assert_eq!(f.handle.selection(), Selection::cell(Pos(5), Pos(19)));
    // From the bottom row, down leaves the table: a text selection.
    f.handle.set_selection(Selection::cursor(Pos(22)));
    f.shift_key("ArrowDown");
    let sel = f.handle.selection();
    assert!(
        matches!(sel, Selection::Text(t) if t.anchor == Pos(22) && t.head.0 > 33),
        "{sel:?}"
    );
}

#[wasm_bindgen_test]
fn shift_click_from_a_cell_selection_keeps_its_anchor_cell_on_the_web() {
    let f = F::new();
    f.handle.set_selection(Selection::cell(Pos(11), Pos(11)));
    f.press(0, true);
    assert_eq!(f.handle.selection(), Selection::cell(Pos(11), Pos(5)));
}
