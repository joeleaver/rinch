//! #1116 (review of #1710): the caret between a claimed image (a node view's
//! host `<span>`) and an unclaimed `<img>`. The web answers a caret against
//! a node view's host span and not against an `<img>`, so a downstream caret
//! between the two, which asks for the leaf after it first (#1116), falls
//! back to the span's right edge. Asked for the `<img>` alone it fell back to
//! the byte query, which drew it at the span's LEFT edge, 40px off (Chrome
//! 155 and Firefox alike).
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::Pos;
use rinch_web::{NodeViewSpec, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST: &str = "data-test-host-caret-between-leaves-1116";

thread_local! {
    static LIVE: std::cell::Cell<Option<RootHandle>> = const { std::cell::Cell::new(None) };
}

/// Mount `html` in a 400px editor with images claimed by `data-badge`, wait
/// for layout, and answer the caret x at char `pos` and the leaves' boxes
/// `(tag, left, right)` in document order.
async fn caret_between(html: &str, pos: usize) -> (f64, Vec<(String, f64, f64)>) {
    if let Some(r) = LIVE.with(|r| r.take()) {
        r.unmount();
    }
    if let Ok(stale) = document().query_selector_all(&format!("[{HOST}]")) {
        for i in 0..stale.length() {
            if let Some(el) = stale
                .item(i)
                .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
            {
                el.remove();
            }
        }
    }
    rinch_web::__reset_activation_state();
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST, "").unwrap();
    host.set_attribute(
        "style",
        "position: fixed; top: 0; left: 0; width: 400px; font-family: monospace; \
         font-size: 16px; line-height: 24px; background: white; z-index: 10;",
    )
    .unwrap();
    let style = document().create_element("style").unwrap();
    style.set_text_content(Some(&format!(
        "[{HOST}] img {{ width: 40px; height: 16px; }}"
    )));
    host.append_child(&style).unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let handle = create_editor();
    assert!(handle.load_html(html));
    handle
        .register_node_view(
            NodeViewSpec::new("image", |s: &mut RenderScope, _ctx| {
                let chip = s.create_element("b");
                chip.set_attribute(
                    "style",
                    "display: inline-block; width: 40px; height: 16px; background: red",
                );
                chip
            })
            .matches(|a| a.get_str("data-badge").is_some()),
        )
        .keep();
    let h = handle.clone();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
        h.mount(scope)
    });
    LIVE.with(|r| r.set(Some(root)));
    for _ in 0..5 {
        let p = js_sys::Promise::new(&mut |res, _| {
            web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&res, 30)
                .unwrap();
        });
        wasm_bindgen_futures::JsFuture::from(p).await.unwrap();
    }
    let leaves = document()
        .query_selector_all(&format!("[{HOST}] [data-pm-type=\"image\"]"))
        .unwrap();
    let mut boxes = Vec::new();
    for i in 0..leaves.length() {
        let el: web_sys::Element = leaves.item(i).unwrap().dyn_into().unwrap();
        let r = el.get_bounding_client_rect();
        boxes.push((el.tag_name().to_lowercase(), r.left(), r.right()));
    }
    let c = handle.caret_rect(Pos(pos + 1)).expect("a caret rect");
    (c.x as f64, boxes)
}

const SRC: &str = "x.png";

#[wasm_bindgen_test]
async fn the_caret_between_a_claimed_and_an_unclaimed_image_is_at_their_boundary() {
    for html in [
        format!(r#"<p>ab<img src="{SRC}" data-badge="n"><img src="{SRC}">cd</p>"#),
        format!(r#"<p>ab<img src="{SRC}"><img src="{SRC}" data-badge="n">cd</p>"#),
    ] {
        let (x, boxes) = caret_between(&html, 3).await;
        assert_eq!(boxes.len(), 2, "positive control: two leaves: {boxes:?}");
        assert!(
            boxes.iter().any(|(tag, _, _)| tag == "span"),
            "positive control: one is a node view's host span: {boxes:?}"
        );
        let boundary = boxes[0].2;
        assert!(
            (boxes[1].1 - boundary).abs() < 1.0,
            "positive control: side by side: {boxes:?}"
        );
        assert!(
            (x - boundary).abs() < 1.5,
            "{html}: caret at {x}, the boundary is {boundary}: {boxes:?}"
        );
    }
}
