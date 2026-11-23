//! rinch's spellchecker in the web editor (#2862): the rich-text `Editor` is
//! not `contenteditable`, so the browser cannot check it. With dictionaries
//! the app supplied (`.aff`/`.dic` bytes), rinch checks it on the page's own
//! thread in slices, and a misspelled word becomes a `<span
//! class="pm-spell-error">` drawn with the browser's own wavy underline.
//! With none supplied the editor is not checked and nothing runs.
//!
//! Every test presses the editor with a genuine `mousedown` through rinch's
//! document delegation; its positive control is that the press focused the
//! editor (the capture field holds the browser's focus), so an absent mark
//! is never a press that rinch did not see. The spellcheck configuration is
//! per thread, so each test installs the one it needs.
#![cfg(target_arch = "wasm32")]

use rinch::spellcheck::{HunspellChecker, SpellcheckConfig, configure};
use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_web::{EditorHandle, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const AFF: &[u8] = include_bytes!("../../rinch-spell/tests/fixtures/en_TEST.aff");
const DIC: &[u8] = include_bytes!("../../rinch-spell/tests/fixtures/en_TEST.dic");

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

/// The app's dictionaries, no pause after typing.
fn with_dictionaries() -> SpellcheckConfig {
    SpellcheckConfig::new()
        .checker(HunspellChecker::from_bytes("en_TEST", AFF, DIC).unwrap())
        .debounce_ms(0)
}

struct Fx {
    _root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

impl Drop for Fx {
    fn drop(&mut self) {
        self.host.remove();
    }
}

/// An editor holding `html` in a fresh host (`wrapper_attr` on a `<div>`
/// around the editor).
fn mount(html: &str, wrapper_attr: Option<(&str, &str)>) -> Fx {
    let host = document().create_element("div").unwrap();
    host.set_attribute(
        "style",
        "font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px; width: 600px;",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let handle = create_editor();
    assert!(handle.load_html(html));
    let mounted = handle.clone();
    let wrapper_attr = wrapper_attr.map(|(k, v)| (k.to_string(), v.to_string()));
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let wrapper = scope.create_element("div");
            if let Some((k, v)) = &wrapper_attr {
                wrapper.set_attribute(k, v);
            }
            wrapper.append_child(&mounted.mount(scope));
            wrapper
        },
    );
    Fx {
        _root: root,
        host,
        handle,
    }
}

impl Fx {
    fn editor(&self) -> web_sys::Element {
        self.host
            .query_selector("[data-pm-editor]")
            .unwrap()
            .expect("the editor is mounted")
    }

    /// A genuine primary press on the first paragraph's text, through the
    /// document's delegation. Asserts the positive control: rinch saw it and
    /// gave the editor the keyboard.
    fn press(&self) {
        let p = self.editor().query_selector("p").unwrap().unwrap();
        let r = p.get_bounding_client_rect();
        let (x, y) = (r.left() + 4.0, r.top() + r.height() / 2.0);
        let target = document().element_from_point(x as f32, y as f32).unwrap();
        for name in ["mousedown", "mouseup", "click"] {
            let init = web_sys::MouseEventInit::new();
            init.set_bubbles(true);
            init.set_cancelable(true);
            init.set_client_x(x as i32);
            init.set_client_y(y as i32);
            init.set_detail(1);
            init.set_buttons(if name == "mousedown" { 1 } else { 0 });
            let ev = web_sys::MouseEvent::new_with_mouse_event_init_dict(name, &init).unwrap();
            target.dispatch_event(&ev).unwrap();
        }
        let active = document().active_element().expect("something has focus");
        assert!(
            active.has_attribute("data-pm-capture"),
            "the press never reached rinch: <{}> has the focus",
            active.tag_name()
        );
    }

    /// The text of every misspelling span, in order.
    fn marked(&self) -> Vec<String> {
        let list = self.editor().query_selector_all(".pm-spell-error").unwrap();
        (0..list.length())
            .map(|i| list.item(i).unwrap().text_content().unwrap_or_default())
            .collect()
    }

    /// Wait for the slices to settle: the marks are what `want` says, or
    /// two seconds pass.
    async fn marked_when(&self, want: &[&str]) -> Vec<String> {
        for _ in 0..100 {
            if self.marked() == want {
                break;
            }
            sleep(20).await;
        }
        self.marked()
    }
}

fn style_of(el: &web_sys::Element, prop: &str) -> String {
    web_sys::window()
        .unwrap()
        .get_computed_style(el)
        .unwrap()
        .unwrap()
        .get_property_value(prop)
        .unwrap()
}

/// A press marks the misspelled words, after the press (in slices), as the
/// browser's own wavy red underline; the custom properties restyle it.
#[wasm_bindgen_test]
async fn a_press_marks_the_misspelled_words_with_a_wavy_underline() {
    configure(with_dictionaries());
    let fx = mount("<p>helo the wrld</p><p>a cta</p>", None);
    sleep(50).await;
    assert!(fx.marked().is_empty(), "marked before any interaction");
    fx.press();
    assert!(
        fx.marked().is_empty(),
        "the check ran inside the press; it is the slices' work"
    );
    assert_eq!(
        fx.marked_when(&["helo", "wrld", "cta"]).await,
        ["helo", "wrld", "cta"]
    );

    let span = fx
        .editor()
        .query_selector(".pm-spell-error")
        .unwrap()
        .unwrap();
    assert_eq!(style_of(&span, "text-decoration-line"), "underline");
    assert_eq!(style_of(&span, "text-decoration-style"), "wavy");
    assert_eq!(style_of(&span, "text-decoration-color"), "rgb(255, 0, 0)");
    // A correct word has none.
    let p = fx.editor().query_selector("p").unwrap().unwrap();
    assert_eq!(style_of(&p, "text-decoration-line"), "none");

    // The dark theme has its own default red, and the property still wins
    // over it (review of #2930: the dark rule hard-coded its colour).
    fx.editor().set_attribute("data-pm-theme", "dark").unwrap();
    assert_eq!(
        style_of(&span, "text-decoration-color"),
        "rgb(255, 123, 114)"
    );

    // Cascade-scoped, as on the desktop.
    fx.host
        .unchecked_ref::<web_sys::HtmlElement>()
        .style()
        .set_property("--rinch-spellcheck-color", "rgb(0, 0, 255)")
        .unwrap();
    fx.host
        .unchecked_ref::<web_sys::HtmlElement>()
        .style()
        .set_property("--rinch-spellcheck-style", "dotted")
        .unwrap();
    assert_eq!(style_of(&span, "text-decoration-color"), "rgb(0, 0, 255)");
    assert_eq!(style_of(&span, "text-decoration-style"), "dotted");

    // Read-only clears the marks; editable again checks again.
    fx.handle.set_read_only(true);
    assert!(fx.marked().is_empty(), "a read-only editor kept its marks");
    fx.handle.set_read_only(false);
    assert_eq!(
        fx.marked_when(&["helo", "wrld", "cta"]).await,
        ["helo", "wrld", "cta"]
    );
    // Turning it off clears them.
    rinch::spellcheck::set_enabled(false);
    assert!(
        fx.marked().is_empty(),
        "turning spellcheck off kept the marks"
    );
}

/// `spellcheck="false"` on an ancestor turns the editor off, as on the
/// desktop; turning it back on checks it.
#[wasm_bindgen_test]
async fn an_ancestors_spellcheck_false_is_honoured() {
    configure(with_dictionaries());
    let fx = mount("<p>helo wrld</p>", Some(("spellcheck", "false")));
    fx.press();
    sleep(100).await;
    assert!(fx.marked().is_empty(), "checked under spellcheck=false");
    // Positive control: the same editor, turned on, is marked.
    fx.editor()
        .parent_element()
        .unwrap()
        .set_attribute("spellcheck", "true")
        .unwrap();
    fx.handle.refresh_spellcheck();
    fx.press();
    assert_eq!(fx.marked_when(&["helo", "wrld"]).await, ["helo", "wrld"]);
}

/// With no dictionaries of the app's the editor is not checked and no slice
/// runs. Dictionaries fetched later (`configure` at run time) check an editor
/// the user already pressed in.
#[wasm_bindgen_test]
async fn no_dictionaries_no_work_until_they_arrive() {
    // The default configuration: the system's dictionaries, which a browser
    // has none of.
    configure(SpellcheckConfig::new().debounce_ms(0));
    rinch_editor_view::spellcheck::reset_slice_stats();
    let fx = mount("<p>helo wrld</p>", None);
    fx.press();
    sleep(100).await;
    assert!(fx.marked().is_empty());
    assert_eq!(
        rinch_editor_view::spellcheck::slice_stats().slices,
        0,
        "checking ran with no dictionaries"
    );
    // The app's fetch completes.
    configure(with_dictionaries());
    assert_eq!(fx.marked_when(&["helo", "wrld"]).await, ["helo", "wrld"]);
    assert!(rinch_editor_view::spellcheck::slice_stats().slices > 0);
}

/// Review of #2930: an editor mounted before the app configured anything
/// (an island whose app calls `configure` after its fetch) is checked once
/// the dictionaries arrive.
#[wasm_bindgen_test]
async fn an_editor_mounted_before_any_configuration_is_checked_later() {
    rinch::spellcheck::clear_config();
    let fx = mount("<p>helo wrld</p>", None);
    fx.press();
    sleep(50).await;
    assert!(fx.marked().is_empty());
    configure(with_dictionaries());
    assert_eq!(fx.marked_when(&["helo", "wrld"]).await, ["helo", "wrld"]);
}

/// A long document's first check is many slices, and ends with every word
/// marked. Prints what the slices cost (the PR's number).
#[wasm_bindgen_test]
async fn a_long_document_is_checked_in_slices() {
    configure(with_dictionaries());
    let line = "the cat and the dog helo ".repeat(40);
    let paragraphs = 400;
    let html: String = (0..paragraphs)
        .map(|_| format!("<p>{line}cta</p>"))
        .collect();
    let fx = mount(&html, None);
    rinch_editor_view::spellcheck::reset_slice_stats();
    fx.press();
    let want = paragraphs * 41;
    for _ in 0..1000 {
        if fx.marked().len() == want {
            break;
        }
        sleep(20).await;
    }
    assert_eq!(fx.marked().len(), want);
    let stats = rinch_editor_view::spellcheck::slice_stats();
    console_log!(
        "#2862 slices: {} slices, {} words, max {} us (applying a result: max {} us), mean {} us",
        stats.slices,
        stats.words,
        stats.max_us,
        stats.max_apply_us,
        stats.total_us / u64::from(stats.slices.max(1))
    );
    assert!(stats.slices > 10, "{stats:?}");
}
