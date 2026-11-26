//! In the browser, the editor container's children start with its blocks, in
//! the model's order, and the caret and selection overlays follow them: Enter
//! at the end of the last paragraph puts the new paragraph after the last
//! block, not after the overlays (`ViewDesc::block_end_anchor`). An app's
//! sibling selectors over the blocks (`p + p` for a first-line indent) then
//! read the document. Before the fix the new paragraph was appended to the
//! container, after the caret and the selection boxes.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_block_order
//! ```
//!
//! The fixture mounts through `rinch_web::mount_into`, focuses with a genuine
//! press and checks the capture textarea holds focus before asserting anything.
//! Positions: `<p>one</p>` 0..5, `<p>two</p>` 5..10, "two" ends at 9.
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

const HOST_MARKER: &str = "data-test-host-block-order";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

impl Fixture {
    /// One editor over `content`, focused by a real press on the first
    /// paragraph's first character.
    fn focused(content: &str) -> Self {
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        host.set_attribute(
            "style",
            "font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px;",
        )
        .unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let handle = create_editor();
        assert!(handle.load_html(content));
        let mounted = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| mounted.mount(scope),
        );
        let f = Self { root, host, handle };
        let para = f
            .editor_el()
            .query_selector("p")
            .unwrap()
            .expect("a paragraph");
        let text = para.first_child().expect("the paragraph has a text node");
        let range = document().create_range().unwrap();
        range.set_start(&text, 0).unwrap();
        range.set_end(&text, 1).unwrap();
        let r = range.get_bounding_client_rect();
        let (x, y) = (
            (r.x() + r.width() / 2.0) as f32,
            (r.y() + r.height() / 2.0) as f32,
        );
        mouse("mousedown", x, y);
        mouse("mouseup", x, y);
        assert!(
            document().active_element().as_deref() == Some(f.capture().as_ref()),
            "positive control: a left press must focus the capture textarea"
        );
        f
    }

    fn editor_el(&self) -> web_sys::Element {
        self.host
            .query_selector("[data-pm-editor]")
            .unwrap()
            .expect("an editor")
    }

    fn capture(&self) -> web_sys::HtmlTextAreaElement {
        document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .expect("the capture textarea exists once an editor was focused")
            .dyn_into()
            .unwrap()
    }

    /// The editor container's `childNodes`, each as `p:<text>` for a block
    /// (`data-pm-type`), its overlay marker (`data-pm-caret`,
    /// `data-pm-selection`, …) for anything else, or `#node` for a non-element.
    fn children(&self) -> Vec<String> {
        let nodes = self.editor_el().child_nodes();
        (0..nodes.length())
            .map(|i| {
                let node = nodes.item(i).unwrap();
                let Ok(el) = node.dyn_into::<web_sys::Element>() else {
                    return "#node".to_string();
                };
                if el.has_attribute("data-pm-type") {
                    return format!(
                        "{}:{}",
                        el.tag_name().to_ascii_lowercase(),
                        el.text_content().unwrap_or_default()
                    );
                }
                let names = el.get_attribute_names();
                (0..names.length())
                    .filter_map(|i| names.get(i).as_string())
                    .find(|n| n.starts_with("data-pm-"))
                    .unwrap_or_else(|| el.tag_name().to_ascii_lowercase())
            })
            .collect()
    }

    /// An Enter `keydown` on the capture textarea; answers whether the editor
    /// consumed it (`preventDefault`ed).
    fn enter(&self) -> bool {
        let init = web_sys::KeyboardEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_key("Enter");
        init.set_code("Enter");
        let ev =
            web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        self.capture().dispatch_event(&ev).unwrap();
        ev.default_prevented()
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

async fn next_task() {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(resolve.unchecked_ref(), 0)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

/// The blocks lead the container's children, in the model's order; nothing
/// that is not a block comes before or between them.
fn assert_blocks_first(children: &[String], blocks: &[&str], context: &str) {
    let leading: Vec<&str> = children
        .iter()
        .take(blocks.len())
        .map(String::as_str)
        .collect();
    assert_eq!(
        leading, blocks,
        "{context}: the blocks lead, in order: {children:?}"
    );
    assert!(
        children[blocks.len()..]
            .iter()
            .all(|c| !c.starts_with("p:")),
        "{context}: no block after the overlays: {children:?}"
    );
}

/// Enter at the end of the last paragraph: the new paragraph goes in after
/// "two", before the caret and the selection boxes.
#[wasm_bindgen_test]
async fn enter_at_the_end_keeps_the_blocks_before_the_overlays() {
    let f = Fixture::focused("<p>one</p><p>two</p>");
    // A selection over "ne" draws selection boxes, then the caret at the end.
    f.handle.set_selection(Selection::text(Pos(2), Pos(4)));
    next_task().await;
    f.handle.set_selection(Selection::cursor(Pos(9)));
    next_task().await;
    let before = f.children();
    assert_blocks_first(&before, &["p:one", "p:two"], "before Enter");
    assert!(
        before.iter().any(|c| c == "data-pm-caret"),
        "positive control: the caret overlay is in the container: {before:?}"
    );
    assert!(f.enter(), "Enter is the editor's");
    next_task().await;
    let after = f.children();
    assert_blocks_first(&after, &["p:one", "p:two", "p:"], "after Enter");
    assert!(
        after.iter().any(|c| c == "data-pm-caret"),
        "the caret overlay is still in the container: {after:?}"
    );
    // And typing in the new paragraph keeps it there.
    assert!(f.handle.insert_text("three"));
    next_task().await;
    assert_blocks_first(
        &f.children(),
        &["p:one", "p:two", "p:three"],
        "after typing",
    );
    f.teardown();
}
