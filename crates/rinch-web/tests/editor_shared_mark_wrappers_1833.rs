//! Adjacent text runs share the wrapper of a mark that continues across them,
//! on the web (#1833): inline code split by bold is one `<code>` holding a
//! `<strong>`, so the browser draws one pill, not one per run.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_shared_mark_wrappers_1833
//! ```
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::model::Fragment;
use rinch_editor_core::{Attrs, Mark, Node, Pos, Selection};
use rinch_web::{EditorHandle, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST: &str = "data-test-host-1833";

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn mount() -> (RootHandle, web_sys::Element, EditorHandle) {
    if let Ok(stale) = document().query_selector_all(&format!("[{HOST}]")) {
        for i in 0..stale.length() {
            if let Some(n) = stale.item(i) {
                n.dyn_into::<web_sys::Element>().unwrap().remove();
            }
        }
    }
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST, "").unwrap();
    host.set_attribute("style", "font-size: 16px; line-height: 24px; width: 400px;")
        .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let handle = create_editor();
    let m = handle.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |s: &mut RenderScope| m.mount(s),
    );
    (root, host, handle)
}

fn mark(handle: &EditorHandle, name: &str) -> Mark {
    let st = handle.state();
    Mark::new(
        st.schema().mark_type(name).unwrap().clone(),
        Attrs::default(),
    )
}

/// `"x" code("ab") code+bold("cd") code("ef")`, built directly: the starter
/// kit's `code` excludes bold, but an app's schema need not.
fn load(handle: &EditorHandle) {
    let st = handle.state();
    let s = st.schema();
    let code = mark(handle, "code");
    let runs: Vec<Node> = vec![
        s.text("x").unwrap(),
        s.text_with_marks("ab", vec![code.clone()]).unwrap(),
        s.text_with_marks("cd", vec![mark(handle, "bold"), code.clone()])
            .unwrap(),
        s.text_with_marks("ef", vec![code]).unwrap(),
    ];
    let para = s
        .branch("paragraph", Fragment::from_children(runs))
        .unwrap();
    handle.load_doc(s.branch("doc", Fragment::from_node(para)).unwrap());
}

fn paragraph(host: &web_sys::Element) -> web_sys::Element {
    host.query_selector("[data-pm-editor] p")
        .unwrap()
        .expect("the paragraph")
}

#[wasm_bindgen_test]
fn code_split_by_bold_is_one_code_element() {
    let (root, host, handle) = mount();
    load(&handle);
    let p = paragraph(&host);
    // Positive control: the editor projected the document into the page.
    assert_eq!(p.text_content().as_deref(), Some("xabcdef"));
    let codes = p.query_selector_all("code").unwrap();
    assert_eq!(codes.length(), 1, "one <code>: {}", p.inner_html());
    let code: web_sys::Element = codes.item(0).unwrap().dyn_into().unwrap();
    assert_eq!(
        code.inner_html(),
        "ab<strong data-pm-mark=\"bold\">cd</strong>ef"
    );

    // A keystroke at the seam joins the run before and keeps the one pill.
    handle.set_selection(Selection::cursor(Pos(4)));
    assert!(handle.insert_text("Z"));
    let p = paragraph(&host);
    assert_eq!(p.text_content().as_deref(), Some("xabZcdef"));
    assert_eq!(p.query_selector_all("code").unwrap().length(), 1);
    // And the browser draws one box: the <code> has a single client rect
    // (two elements, or one split across a line, would give more).
    let code: web_sys::Element = p.query_selector("code").unwrap().unwrap();
    assert_eq!(code.get_client_rects().length(), 1);
    root.unmount();
    host.remove();
}
