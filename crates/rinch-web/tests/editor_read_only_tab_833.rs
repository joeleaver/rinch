//! Browser-driven tests for Tab in a read-only rich-text editor (#833).
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_read_only_tab_833
//! ```
//!
//! The editor's keymap binds `Tab`→`sinkListItem` and `Shift-Tab`→
//! `liftListItem`, and a matched binding is consumed whether or not it
//! applied. A read-only editor refuses both, and consuming the key trapped a
//! keyboard user inside it. It now behaves like a `readonly` field: Tab and
//! Shift+Tab move the focus to the next and the previous Tab stop **from the
//! editor's place in the page** — not from the hidden capture `<textarea>`,
//! which holds the browser's focus but sits at the end of `<body>`, so the
//! browser's own Tab from there would leave the page and its Shift+Tab land on
//! the page's last control.
//!
//! Every fixture mounts through `rinch_web::mount_into`, focuses the editor
//! with a genuine `mousedown` and checks the capture textarea holds focus
//! (the positive control) before pressing anything.
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

const HOST_MARKER: &str = "data-test-host-read-only-tab";
const CONTENT: &str = "<ul><li><p>one</p></li><li><p>two</p></li></ul>";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

/// Where the editor sits.
#[derive(Clone, Copy)]
enum Layout {
    /// `outer` · `before` · editor · `after`, no trap.
    Page,
    /// `outer` · trap{ editor · `inside` } · `after`: the editor is the
    /// trap's first stop, so Shift+Tab wraps to `inside` instead of reaching
    /// `outer`.
    Trapped,
    /// `before` · editor: no Tab stop after the editor.
    LastStop,
    /// editor · `after`: no Tab stop before the editor.
    FirstStop,
    /// trap{ editor · `a1` } · trap{ `b1` · `b2` }: two visible traps, the
    /// editor in the first.
    TwoTraps,
}

fn button(scope: &mut RenderScope, name: &str) -> rinch_core::dom::NodeHandle {
    let b = scope.create_element("button");
    b.set_attribute("data-test-button", name);
    b.set_attribute("style", "width: 80px; height: 24px");
    b
}

impl Fixture {
    fn new(layout: Layout, read_only: bool) -> Self {
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
            "font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px;",
        )
        .unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let handle = create_editor();
        assert!(handle.load_html(CONTENT));
        let mounted = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| {
                let wrapper = scope.create_element("div");
                match layout {
                    Layout::Page | Layout::Trapped => {
                        wrapper.append_child(&button(scope, "outer"));
                        let inner = scope.create_element("div");
                        if matches!(layout, Layout::Trapped) {
                            inner.set_attribute("data-trap-focus", "");
                            inner.append_child(&mounted.mount(scope));
                            inner.append_child(&button(scope, "inside"));
                        } else {
                            inner.append_child(&button(scope, "before"));
                            inner.append_child(&mounted.mount(scope));
                        }
                        wrapper.append_child(&inner);
                        wrapper.append_child(&button(scope, "after"));
                    }
                    Layout::LastStop => {
                        wrapper.append_child(&button(scope, "before"));
                        wrapper.append_child(&mounted.mount(scope));
                    }
                    Layout::FirstStop => {
                        wrapper.append_child(&mounted.mount(scope));
                        wrapper.append_child(&button(scope, "after"));
                    }
                    Layout::TwoTraps => {
                        let a = scope.create_element("div");
                        a.set_attribute("data-trap-focus", "");
                        a.append_child(&mounted.mount(scope));
                        a.append_child(&button(scope, "a1"));
                        wrapper.append_child(&a);
                        let b = scope.create_element("div");
                        b.set_attribute("data-trap-focus", "");
                        b.append_child(&button(scope, "b1"));
                        b.append_child(&button(scope, "b2"));
                        wrapper.append_child(&b);
                    }
                }
                wrapper
            },
        );
        let f = Self { root, host, handle };
        f.focus_in_two();
        f.handle.set_read_only(read_only);
        f
    }

    fn button(&self, name: &str) -> web_sys::Element {
        self.host
            .query_selector(&format!("[data-test-button={name}]"))
            .unwrap()
            .unwrap_or_else(|| panic!("no button {name}"))
    }

    fn capture(&self) -> web_sys::HtmlTextAreaElement {
        document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .expect("the capture textarea exists once an editor was focused")
            .dyn_into()
            .unwrap()
    }

    fn active_is(&self, el: &web_sys::Element) -> bool {
        document().active_element().as_ref() == Some(el)
    }

    /// A genuine press on the "o" of "two", proved to give the capture
    /// textarea the browser's focus.
    fn focus_in_two(&self) {
        let para = self
            .host
            .query_selector_all("[data-pm-editor] p")
            .unwrap()
            .item(1)
            .expect("the second item's paragraph");
        let text = para.first_child().expect("the paragraph has a text node");
        let range = document().create_range().unwrap();
        range.set_start(&text, 1).unwrap();
        range.set_end(&text, 2).unwrap();
        let r = range.get_bounding_client_rect();
        let (x, y) = (r.x() + r.width() / 2.0, r.y() + r.height() / 2.0);
        mouse("mousedown", x as f32, y as f32);
        mouse("mouseup", x as f32, y as f32);
        assert!(
            self.active_is(self.capture().as_ref()),
            "positive control: a left press must focus the capture textarea"
        );
    }

    /// The list's top-level item count: 2 as loaded, 1 once Tab nested "two".
    fn top_level_items(&self) -> usize {
        self.handle.doc().child(0).child_count()
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
}

fn mouse(name: &str, x: f32, y: f32) {
    let target = document()
        .element_from_point(x, y)
        .unwrap_or_else(|| panic!("nothing under ({x}, {y})"));
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_button(0);
    init.set_buttons(1);
    init.set_detail(1);
    init.set_client_x(x as i32);
    init.set_client_y(y as i32);
    let ev = web_sys::MouseEvent::new_with_mouse_event_init_dict(name, &init).unwrap();
    target.dispatch_event(&ev).unwrap();
}

/// A Tab `keydown` on whatever has the browser's focus; answers whether it was
/// `preventDefault`ed. A synthetic event has no default action, so a Tab that
/// is not consumed moves nothing here — the fixtures that expect a move expect
/// rinch to have made it, and to say so by consuming the key.
fn tab(shift: bool) -> bool {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key("Tab");
    init.set_code("Tab");
    init.set_shift_key(shift);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    let target = document().active_element().expect("something has focus");
    target.dispatch_event(&ev).unwrap();
    ev.default_prevented()
}

#[wasm_bindgen_test]
fn tab_and_shift_tab_leave_a_read_only_editor_from_its_place_in_the_page() {
    for (shift, want) in [(false, "after"), (true, "before")] {
        let f = Fixture::new(Layout::Page, true);
        let doc = f.handle.doc();
        assert!(tab(shift), "shift={shift}: rinch moves the focus itself");
        assert!(
            f.active_is(&f.button(want)),
            "shift={shift}: focus went to `{want}`, is on {:?}",
            document().active_element().map(|e| e.outer_html())
        );
        assert!(f.handle.doc().same_ref(&doc), "shift={shift}: no edit");
        f.teardown();
    }
}

/// Inside a focus trap the move stays in the trap and wraps there, as every
/// other Tab inside it does: backward from the trap's first stop is its last,
/// not the `outer` button before the trap in the page.
#[wasm_bindgen_test]
fn inside_a_trap_tab_from_a_read_only_editor_wraps_within_the_trap() {
    for (shift, want) in [(false, "inside"), (true, "inside")] {
        let f = Fixture::new(Layout::Trapped, true);
        assert!(tab(shift), "shift={shift}: consumed");
        assert!(
            f.active_is(&f.button(want)),
            "shift={shift}: focus went to `{want}`, is on {:?}",
            document().active_element().map(|e| e.outer_html())
        );
        f.teardown();
    }
}

/// The control, and the switch going off again: an editable editor keeps Tab
/// and nests the item, and Shift+Tab lifts it back.
#[wasm_bindgen_test]
fn an_editable_editor_still_indents_on_tab_also_after_read_only_is_switched_off() {
    for was_read_only in [false, true] {
        let f = Fixture::new(Layout::Page, was_read_only);
        f.handle.set_read_only(false);
        f.handle.set_selection(Selection::cursor(Pos(11)));
        assert_eq!(f.top_level_items(), 2, "precondition");
        assert!(tab(false), "was_read_only={was_read_only}: consumed");
        assert!(
            f.active_is(f.capture().as_ref()),
            "was_read_only={was_read_only}: the editor keeps the keyboard"
        );
        assert_eq!(f.top_level_items(), 1, "was_read_only={was_read_only}");
        assert!(tab(true), "was_read_only={was_read_only}: consumed");
        assert!(f.active_is(f.capture().as_ref()));
        assert_eq!(f.top_level_items(), 2, "was_read_only={was_read_only}");
        f.teardown();
    }
}

/// The editor is the page's last Tab stop: Tab is left to the browser (not
/// consumed, nothing moved here), whose own Tab from the capture textarea at
/// the end of `<body>` then leaves the page, as from a native last control.
/// Not a wrap to `before`, which a trap's Tab would do.
#[wasm_bindgen_test]
fn tab_past_the_last_stop_is_left_to_the_browser() {
    let f = Fixture::new(Layout::LastStop, true);
    assert!(
        !tab(false),
        "not consumed: the browser's Tab leaves the page"
    );
    assert!(
        f.active_is(f.capture().as_ref()),
        "nothing moved, is on {:?}",
        document().active_element().map(|e| e.outer_html())
    );
    f.teardown();
}

/// The editor is the page's first Tab stop: Shift+Tab leaves the page
/// backward, as from a native `<textarea readonly>` there — the focus goes to
/// the document (`<body>`), measured with trusted keys in Chrome 155. Left to
/// the browser, its Shift+Tab from the capture textarea at the end of
/// `<body>` would land on `after`, the page's last control: a wrap.
#[wasm_bindgen_test]
fn shift_tab_before_the_first_stop_leaves_the_focus_to_the_document() {
    let f = Fixture::new(Layout::FirstStop, true);
    assert!(
        tab(true),
        "consumed: rinch blurs the capture textarea itself"
    );
    assert_eq!(
        document().active_element(),
        document().body().map(Into::into),
        "the focus is on the document, is on {:?}",
        document().active_element().map(|e| e.outer_html())
    );
    assert!(
        !f.active_is(&f.button("after")),
        "no wrap to the last control"
    );
    f.teardown();
}

/// Two visible traps, the editor in the first: the Tab stays in the trap the
/// *editor* is in (desktop's `tab_trap_root` from the focused node), not the
/// last visible trap in the page, which is the answer from the capture
/// textarea at the end of `<body>`.
#[wasm_bindgen_test]
fn with_two_traps_tab_stays_in_the_editors_trap() {
    for shift in [false, true] {
        let f = Fixture::new(Layout::TwoTraps, true);
        assert!(tab(shift), "shift={shift}: consumed");
        assert!(
            f.active_is(&f.button("a1")),
            "shift={shift}: focus went to `a1`, is on {:?}",
            document().active_element().map(|e| e.outer_html())
        );
        f.teardown();
    }
}
