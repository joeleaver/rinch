//! #1120 on the web: the selection highlight beside an image. The web editor
//! draws its highlight from `Range.getClientRects` over the flat byte range,
//! and an image has no bytes there either, so a selection from right after an
//! image covered it (the range started at the end of the text before it) and
//! an image selected alone drew nothing. Desktop's twin is the #1120 section
//! of `rinch/src/app/editor_inline_leaf_tests.rs`.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_image_selection_1120
//! ```
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

const HOST_MARKER: &str = "data-test-host-image-selection-1120";
/// A 1x1 GIF; the sheet sizes every image 40x16.
const IMG: &str = "<img src=\"data:image/gif;base64,R0lGODlhAQABAIAAAP///wAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw==\" alt=\"\">";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

impl Fixture {
    fn mount(html: &str) -> Self {
        if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
            for i in 0..stale.length() {
                if let Some(node) = stale.item(i)
                    && let Ok(el) = node.dyn_into::<web_sys::Element>()
                {
                    el.remove();
                }
            }
        }
        rinch_web::__reset_activation_state();
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        host.set_attribute(
            "style",
            "font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px;",
        )
        .unwrap();
        let sheet = document().create_element("style").unwrap();
        sheet.set_text_content(Some(&format!(
            "[{HOST_MARKER}] img {{ width: 40px; height: 16px; }}"
        )));
        host.append_child(&sheet).unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let handle = create_editor();
        assert!(handle.load_html(html));
        let mounted = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| {
                let wrap = scope.create_element("div");
                wrap.set_attribute("style", "width: 400px");
                wrap.append_child(&mounted.mount(scope));
                wrap
            },
        );
        handle.focus();
        Self { root, host, handle }
    }

    /// The images' `(left, right)` in client px, in document order.
    fn images(&self) -> Vec<(f64, f64)> {
        let list = self.host.query_selector_all("img").unwrap();
        (0..list.length())
            .filter_map(|i| list.item(i)?.dyn_into::<web_sys::Element>().ok())
            .map(|el| {
                let r = el.get_bounding_client_rect();
                (r.left(), r.right())
            })
            .collect()
    }

    /// Select chars `i..j` of the paragraph; the highlight rects'
    /// `(left, right)` in client px.
    async fn select(&self, i: usize, j: usize) -> Vec<(f64, f64)> {
        self.handle
            .set_selection(Selection::text(Pos(i + 1), Pos(j + 1)));
        next_task().await;
        let list = document()
            .query_selector_all("[data-pm-editor] [data-pm-selection]")
            .unwrap();
        (0..list.length())
            .filter_map(|i| list.item(i)?.dyn_into::<web_sys::Element>().ok())
            .map(|el| el.get_bounding_client_rect())
            .filter(|r| r.width() > 0.0 && r.height() > 0.0)
            .map(|r| (r.left(), r.right()))
            .collect()
    }

    /// Where the caret at char `i` is drawn, in client px.
    fn caret_x(&self, i: usize) -> f64 {
        self.handle.caret_rect(Pos(i + 1)).expect("laid out").x as f64
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
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

#[track_caller]
fn assert_covers(hl: &[(f64, f64)], left: f64, right: f64, what: &str) {
    assert!(!hl.is_empty(), "{what}: nothing drawn");
    let l = hl.iter().map(|r| r.0).fold(f64::INFINITY, f64::min);
    let r = hl.iter().map(|r| r.1).fold(f64::NEG_INFINITY, f64::max);
    assert!(
        (l - left).abs() < 1.5 && (r - right).abs() < 1.5,
        "{what}: {l}..{r}, want {left}..{right} ({hl:?})"
    );
}

#[wasm_bindgen_test]
async fn the_highlight_beside_an_image_is_on_its_side() {
    let f = Fixture::mount(&format!("<p>alpha{IMG}bravo</p>"));
    next_task().await;
    let img = f.images()[0];
    assert!(img.1 >= img.0 + 39.0, "positive control: the image {img:?}");
    let (a1, b2) = (f.caret_x(1), f.caret_x(8));
    assert_covers(
        &f.select(6, 8).await,
        img.1,
        b2,
        "from after the image over `br`",
    );
    assert_covers(&f.select(5, 6).await, img.0, img.1, "the image alone");
    assert_covers(&f.select(1, 6).await, a1, img.1, "`lpha` and the image");
    assert_covers(&f.select(1, 5).await, a1, img.0, "`lpha`, not the image");
    assert_covers(&f.select(3, 8).await, f.caret_x(3), b2, "across the image");
    f.teardown();
}

#[wasm_bindgen_test]
async fn two_adjacent_images_are_each_their_own() {
    let f = Fixture::mount(&format!("<p>ab{IMG}{IMG}cd</p>"));
    next_task().await;
    let imgs = f.images();
    assert_eq!(imgs.len(), 2, "positive control");
    let (a, b) = (imgs[0], imgs[1]);
    assert_covers(&f.select(2, 3).await, a.0, a.1, "the first image");
    assert_covers(&f.select(3, 4).await, b.0, b.1, "the second image");
    assert_covers(&f.select(2, 4).await, a.0, b.1, "both");
    assert_covers(
        &f.select(3, 5).await,
        b.0,
        f.caret_x(5),
        "the second and `c`",
    );
    f.teardown();
}

// From the review of PR #1730: a block's edge. With no text before (after)
// the images, the start (end) is the box list's, never the text beyond the
// other images.

#[wasm_bindgen_test]
async fn adjacent_images_at_a_paragraphs_start_are_each_their_own() {
    let f = Fixture::mount(&format!("<p>{IMG}{IMG}cd</p>"));
    next_task().await;
    let imgs = f.images();
    assert_eq!(imgs.len(), 2);
    assert_covers(
        &f.select(0, 1).await,
        imgs[0].0,
        imgs[0].1,
        "first leading image",
    );
    assert_covers(
        &f.select(1, 2).await,
        imgs[1].0,
        imgs[1].1,
        "second leading image",
    );
    f.teardown();
}

#[wasm_bindgen_test]
async fn adjacent_images_at_a_paragraphs_end_are_each_their_own() {
    let f = Fixture::mount(&format!("<p>ab{IMG}{IMG}</p>"));
    next_task().await;
    let imgs = f.images();
    assert_covers(
        &f.select(2, 3).await,
        imgs[0].0,
        imgs[0].1,
        "first trailing image",
    );
    assert_covers(
        &f.select(3, 4).await,
        imgs[1].0,
        imgs[1].1,
        "second trailing image",
    );
    f.teardown();
}

#[wasm_bindgen_test]
async fn the_middle_of_three_images_alone() {
    let f = Fixture::mount(&format!("<p>{IMG}{IMG}{IMG}</p>"));
    next_task().await;
    let imgs = f.images();
    assert_covers(&f.select(1, 2).await, imgs[1].0, imgs[1].1, "middle image");
    f.teardown();
}

#[wasm_bindgen_test]
async fn an_image_after_a_hard_break_alone() {
    let f = Fixture::mount(&format!("<p>ab<br>{IMG}cd</p>"));
    next_task().await;
    let imgs = f.images();
    assert_covers(
        &f.select(3, 4).await,
        imgs[0].0,
        imgs[0].1,
        "image after break",
    );
    f.teardown();
}

#[wasm_bindgen_test]
async fn a_linked_image_alone() {
    let f = Fixture::mount(&format!("<p>ab<a href=\"https://e.x\">{IMG}</a>cd</p>"));
    next_task().await;
    let imgs = f.images();
    assert_covers(&f.select(2, 3).await, imgs[0].0, imgs[0].1, "linked image");
    f.teardown();
}

#[wasm_bindgen_test]
async fn a_hard_break_alone_draws_nothing() {
    let f = Fixture::mount("<p>ab<br>cd</p>");
    next_task().await;
    let hl = f.select(2, 3).await;
    assert!(hl.is_empty(), "br alone: {hl:?}");
    f.teardown();
}
