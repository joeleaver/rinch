//! Floating controls over a picture in the browser editor: which source an
//! image's `<img>` is resolved from (`rinch_web::set_image_source`), that each
//! such source resolves and reloads on its own, and the image the pointer is
//! over (`EditorHandle::on_image_hover`).
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_image_hover
//! ```
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::{AttrValue, Pos, SetNodeAttrStep};
use rinch_web::{
    EditorHandle, ImageHover, LOGICAL_SRC_ATTR, RootHandle, clear_image_source, create_editor,
    register_image_url_scheme, set_image_source,
};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-test-host-image-hover";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

impl Fixture {
    fn mount(html: &str) -> Self {
        if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
            for i in 0..stale.length() {
                if let Some(el) = stale
                    .item(i)
                    .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
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
        let style = document().create_element("style").unwrap();
        style.set_text_content(Some(
            "[data-test-host-image-hover] [data-pm-editor] img { width: 40px; height: 30px; }",
        ));
        host.append_child(&style).unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let handle = create_editor();
        assert!(handle.load_html(html));
        let mounted = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| mounted.mount(scope),
        );
        Self { root, host, handle }
    }

    fn images(&self) -> Vec<web_sys::Element> {
        let list = self
            .host
            .query_selector_all("[data-pm-editor] img")
            .unwrap();
        (0..list.length())
            .map(|i| list.item(i).unwrap().dyn_into().unwrap())
            .collect()
    }

    fn teardown(self) {
        rinch_editor_view::set_image_hover(None, None);
        self.root.unmount();
        self.host.remove();
        rinch_web::__reset_image_sources();
    }
}

fn centre(el: &web_sys::Element) -> (f32, f32) {
    let r = el.get_bounding_client_rect();
    (
        (r.x() + r.width() / 2.0) as f32,
        (r.y() + r.height() / 2.0) as f32,
    )
}

fn hover_to((x, y): (f32, f32)) {
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_client_x(x as i32);
    init.set_client_y(y as i32);
    let ev = web_sys::MouseEvent::new_with_mouse_event_init_dict("mousemove", &init).unwrap();
    document()
        .element_from_point(x, y)
        .unwrap_or_else(|| panic!("nothing under ({x}, {y})"))
        .dispatch_event(&ev)
        .unwrap();
}

type Seen = Rc<RefCell<Vec<Option<ImageHover>>>>;

fn record(handle: &EditorHandle) -> Seen {
    let seen: Seen = Rc::default();
    let seen_in = seen.clone();
    handle.on_image_hover(move |h| seen_in.borrow_mut().push(h.cloned()));
    seen
}

#[wasm_bindgen_test]
fn hovering_an_image_reports_its_position_attrs_and_client_rect() {
    let f = Fixture::mount(
        r#"<p>ab<img src="x.png" alt="first">cd</p><p>ef<img src="y.png" alt="second"></p>"#,
    );
    let seen = record(&f.handle);
    let imgs = f.images();
    assert_eq!(imgs.len(), 2);
    let r = imgs[0].get_bounding_client_rect();
    assert_eq!((r.width(), r.height()), (40.0, 30.0), "control: sized");

    // Onto the first picture and around inside it: one report.
    hover_to(centre(&imgs[0]));
    hover_to(((r.x() + 3.0) as f32, (r.y() + 3.0) as f32));
    assert_eq!(seen.borrow().len(), 1);
    let first = seen.borrow()[0].clone().expect("entered");
    assert_eq!(first.pos, Pos(3));
    assert_eq!(first.attrs.get_str("alt"), Some("first"));
    assert_eq!(
        (
            first.rect.x,
            first.rect.y,
            first.rect.width,
            first.rect.height
        ),
        (r.x() as f32, r.y() as f32, 40.0, 30.0)
    );

    // An edit of its attrs is reported on the next move.
    assert!(f.handle.update(|state| {
        let mut tr = state.tr();
        tr.step(Box::new(SetNodeAttrStep::new(
            3,
            "alt",
            AttrValue::from("edited"),
        )))
        .ok()?;
        Some(tr)
    }));
    hover_to(centre(&f.images()[0]));
    assert_eq!(seen.borrow().len(), 2);
    assert_eq!(
        seen.borrow()[1].as_ref().unwrap().attrs.get_str("alt"),
        Some("edited")
    );

    // Onto the second picture, then off every picture.
    hover_to(centre(&f.images()[1]));
    let p = f
        .host
        .query_selector("[data-pm-editor] p")
        .unwrap()
        .unwrap();
    let pr = p.get_bounding_client_rect();
    hover_to(((pr.x() + 2.0) as f32, (pr.y() + pr.height() / 2.0) as f32));
    let got: Vec<Option<String>> = seen
        .borrow()
        .iter()
        .map(|h| {
            h.as_ref()
                .map(|h| h.attrs.get_str("alt").unwrap_or("").to_string())
        })
        .collect();
    assert_eq!(
        got,
        vec![
            Some("first".into()),
            Some("edited".into()),
            Some("second".into()),
            None
        ]
    );
    f.teardown();
}

/// Two pictures with one `src` and different attrs are two sources: each is
/// resolved once, `reload_image` of one asks for it alone, and an edit of an
/// image's attrs asks for its new source.
#[wasm_bindgen_test]
async fn each_source_an_app_chooses_resolves_and_reloads_on_its_own() {
    let asked: Rc<RefCell<Vec<String>>> = Rc::default();
    let asked_in = asked.clone();
    register_image_url_scheme("hover-web-blob", move |src: &str| {
        asked_in.borrow_mut().push(src.to_string());
        Some("data:,".to_string())
    });
    set_image_source(|attrs| {
        let title = attrs.get_str("title").filter(|t| !t.is_empty())?;
        Some(format!("{}#overlay={title}", attrs.get_str("src")?))
    });
    const S: &str = "hover-web-blob:store/blob";
    let f = Fixture::mount(&format!(
        r#"<p><img src="{S}" title="b1"> <img src="{S}"> <img src="{S}" title="b2"></p>"#
    ));
    let microtask = || {
        wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&wasm_bindgen::JsValue::NULL))
    };
    let _ = microtask().await;
    let logical: Vec<String> = f
        .images()
        .iter()
        .map(|i| i.get_attribute(LOGICAL_SRC_ATTR).unwrap_or_default())
        .collect();
    assert_eq!(
        logical,
        [
            format!("{S}#overlay=b1"),
            S.into(),
            format!("{S}#overlay=b2")
        ]
    );
    let sorted = |v: &RefCell<Vec<String>>| {
        let mut v = v.borrow().clone();
        v.sort();
        v
    };
    assert_eq!(
        sorted(&asked),
        [
            S.to_string(),
            format!("{S}#overlay=b1"),
            format!("{S}#overlay=b2")
        ]
    );

    asked.borrow_mut().clear();
    rinch::image::reload_image(&format!("{S}#overlay=b2"));
    assert_eq!(sorted(&asked), [format!("{S}#overlay=b2")]);

    asked.borrow_mut().clear();
    assert!(f.handle.update(|state| {
        let mut tr = state.tr();
        tr.step(Box::new(SetNodeAttrStep::new(
            3,
            "title",
            AttrValue::from("b3"),
        )))
        .ok()?;
        Some(tr)
    }));
    let _ = microtask().await;
    assert_eq!(
        f.images()[1].get_attribute(LOGICAL_SRC_ATTR).as_deref(),
        Some(format!("{S}#overlay=b3").as_str())
    );
    assert_eq!(sorted(&asked), [format!("{S}#overlay=b3")]);
    clear_image_source();
    rinch_web::unregister_image_url_scheme("hover-web-blob");
    f.teardown();
}
