//! `HtmlView`'s user sheet on the web: an element the sheet matches in the
//! document as written (by a class the cleaner strips) gets the rule's
//! declarations inside the view's shadow root, and changing only the
//! declarations restyles it without cleaning again.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test html_view_user_sheet
//! ```
#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rinch::html::Policy;
use rinch::prelude::*;
use rinch_core::Component;
use rinch_core::element::ThemeProviderProps;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const MAIL: &str = r#"<p>Sounds good.</p><div class="gmail_quote"><blockquote type="cite">Shall we meet at noon?</blockquote></div><img src="data:image/png;base64,AAAA">"#;

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

#[wasm_bindgen_test]
fn the_sheet_styles_matched_elements_and_toggles_without_a_new_clean() {
    rinch_web::__reset_activation_state();
    let host = document().create_element("div").unwrap();
    host.set_attribute("data-test-host-user-sheet", "").unwrap();
    document().body().unwrap().append_child(&host).unwrap();

    let sheet: Rc<Cell<Option<Signal<String>>>> = Rc::new(Cell::new(None));
    let cleans = Arc::new(AtomicUsize::new(0));
    let reported: Rc<RefCell<Vec<Vec<usize>>>> = Rc::new(RefCell::new(Vec::new()));
    let (s, c, r) = (sheet.clone(), cleans.clone(), reported.clone());
    let handle = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
        let css = Signal::new(".gmail_quote { display: none }".to_owned());
        s.set(Some(css));
        let counted = c.clone();
        let r = r.clone();
        HtmlView {
            document: MAIL.into(),
            policy: Some(Policy {
                picture: Arc::new(move |src| {
                    counted.fetch_add(1, Ordering::Relaxed);
                    Some(src.to_owned())
                }),
                ..Policy::default()
            }),
            user_css_fn: Some(Rc::new(move || css.get())),
            onusermatches: Some((move |n: Vec<usize>| r.borrow_mut().push(n)).into()),
            ..Default::default()
        }
        .render(scope, &[])
    });
    let css = sheet.get().unwrap();

    let view = host.query_selector(".rinch-html-view").unwrap().unwrap();
    let shadow = view.shadow_root().expect("isolated in a shadow root");
    let quote = shadow
        .query_selector("[data-rinch-user]")
        .unwrap()
        .expect("the quote is marked");
    let display = || {
        web_sys::window()
            .unwrap()
            .get_computed_style(&quote)
            .unwrap()
            .unwrap()
            .get_property_value("display")
            .unwrap()
    };
    assert_eq!(display(), "none");
    assert_eq!(*reported.borrow(), [vec![1]]);
    let reply = shadow.query_selector("p").unwrap().unwrap();
    assert!(reply.get_bounding_client_rect().height() > 0.0);

    css.set(".gmail_quote { }".into());
    assert_eq!(display(), "block");
    assert!(
        quote.get_bounding_client_rect().height() > 0.0,
        "the quote shows"
    );
    css.set(".gmail_quote { display: none }".into());
    assert_eq!(display(), "none");
    assert_eq!(
        cleans.load(Ordering::Relaxed),
        1,
        "toggling never cleaned again"
    );

    handle.unmount();
    host.remove();
}
