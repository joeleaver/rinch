//! Browser-driven test for issue #754: a reactive `muted:` binding on the
//! web cannot mute or unmute a media element.
//!
//! `muted` is in `rinch_core::dom::is_boolean_attribute`, so `rsx!` renders a
//! reactive `muted: {|| m.get()}` as presence/absence of the `muted` content
//! attribute (issue #551). But HTML gives that attribute no dynamic effect —
//! it only seeds the media element's `defaultMuted` once, at the load
//! algorithm — so writing or removing it after that point moves nothing the
//! user can hear or see muted.
//!
//! rinch mirrors the attribute's presence onto the live `.muted` IDL property
//! instead (option 1 of the issue's three), the way `checked`/`selected` are
//! mirrored (`sync_presence_property`, issues #100/#622). That makes rinch's
//! `muted` more dynamic than plain HTML's, deliberately, because nothing else
//! lets a reactive binding mute/unmute a media element at all.
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown
//! ```
#![cfg(target_arch = "wasm32")]

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use rinch_web::RootHandle;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-media-muted-754-test-host";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
}

impl Fixture {
    fn mount(build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> Self {
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
        document().body().unwrap().append_child(&host).unwrap();
        let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), build);
        Self { root, host }
    }

    fn media(&self, id: &str) -> web_sys::HtmlMediaElement {
        document()
            .get_element_by_id(id)
            .unwrap_or_else(|| panic!("no element #{id}"))
            .dyn_into()
            .unwrap()
    }

    fn attr(&self, id: &str, name: &str) -> Option<String> {
        document()
            .get_element_by_id(id)
            .unwrap()
            .get_attribute(name)
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
}

#[component]
fn reactive_video(muted: Signal<bool>) -> NodeHandle {
    rsx! {
        div {
            video { id: "vid", muted: {move || muted.get()} }
        }
    }
}

/// The measured HTML fact the issue rests on: writing the `muted` *content*
/// attribute on an already-attached, not-yet-loaded `<video>` (no `src`, so no
/// load algorithm ever runs) moves neither `defaultMuted` nor the live
/// `.muted` — measured outside rinch, raw DOM calls, so nothing in the
/// framework can be what produced the answer. This is the oracle the fix
/// deliberately diverges from (option 1 of #754's three): rinch's own
/// `muted:` binding mirrors the attribute's presence onto `.muted` anyway.
#[wasm_bindgen_test]
fn html_muted_attribute_alone_does_not_move_the_live_property() {
    let el = document().create_element("video").unwrap();
    document().body().unwrap().append_child(&el).unwrap();
    let media: web_sys::HtmlMediaElement = el.clone().dyn_into().unwrap();

    el.set_attribute("muted", "").unwrap();
    assert!(
        !media.muted(),
        "HTML gives the `muted` content attribute no dynamic effect — \
         `setAttribute(\"muted\", \"\")` alone must not mute a raw <video> \
         (measured, Chrome)"
    );

    el.remove();
}

/// A reactive `muted:` binding must mute and unmute the element through its
/// live `.muted` property.
///
/// Fails at HEAD on the first assertion after `muted.set(true)`:
/// `WebDocument::set_attribute`'s match on `"muted"` had no arm at all (unlike
/// `"checked"`/`"selected"`), so the attribute toggled but `sync_presence_property`
/// was never consulted for it and `.muted` never moved — which is exactly the
/// raw-DOM measurement above, reached through rinch's own reactive binding
/// instead of a literal `setAttribute` call.
#[wasm_bindgen_test]
fn a_reactive_muted_binding_mutes_and_unmutes_the_element() {
    let muted = Signal::new(false);
    let f = Fixture::mount(move |scope: &mut RenderScope| reactive_video(scope, muted));

    assert!(!f.media("vid").muted(), "starts unmuted");
    assert_eq!(f.attr("vid", "muted"), None, "and with no attribute");

    muted.set(true);
    assert_eq!(
        f.attr("vid", "muted").as_deref(),
        Some(""),
        "a true boolean attribute is written in the bare presence form"
    );
    assert!(
        f.media("vid").muted(),
        "a reactive `muted: {{|| true}}` must mute the live element (#754)"
    );

    muted.set(false);
    assert_eq!(
        f.attr("vid", "muted"),
        None,
        "a false boolean attribute is removed, not written as \"false\""
    );
    assert!(
        !f.media("vid").muted(),
        "and a reactive `muted: {{|| false}}` must unmute it back (#754)"
    );

    f.teardown();
}

/// Build a bare `<video>`, handing back its `NodeHandle` so the test can call
/// `set_attribute`/`remove_attribute`/`write_attribute` directly, exercising
/// the same guard `is_presence_reflected_attribute` closes for `checked` /
/// `selected` (#687) — off the fixed point where the attribute's presence and
/// `.muted` already agree.
fn video_fixture(
    out: std::rc::Rc<std::cell::RefCell<Option<NodeHandle>>>,
) -> impl FnOnce(&mut RenderScope) -> NodeHandle + 'static {
    move |scope: &mut RenderScope| {
        let video = scope.create_element("video");
        video.set_attribute("id", "lit-vid");
        *out.borrow_mut() = Some(video.clone());
        video
    }
}

/// A falsey `write_attribute("muted", ...)` must reach the backend — and
/// un-mute the element — even when the content attribute is already absent.
///
/// That is only observable once something has moved `.muted` without the
/// attribute, which native media controls do (and which this fixture
/// reproduces directly, since driving the real controls needs a loaded
/// media resource). Without `muted` in `is_presence_reflected_attribute`,
/// `write_attribute`'s removal guard reads "attribute already absent" as
/// "already unmuted" and skips the call — exactly the #687 shape one
/// attribute over.
#[wasm_bindgen_test]
fn a_falsey_muted_write_unmutes_a_dirtied_element() {
    let out = std::rc::Rc::new(std::cell::RefCell::new(None));
    let f = Fixture::mount(video_fixture(out.clone()));
    let video = out.borrow().clone().expect("fixture built");

    let media = f.media("lit-vid");
    media.set_muted(true);
    assert!(media.muted(), "the element is muted …");
    assert_eq!(
        f.attr("lit-vid", "muted"),
        None,
        "… with no `muted` attribute anywhere — same shape native media \
         controls leave behind"
    );

    video.write_attribute("muted", "false");
    assert!(
        !f.media("lit-vid").muted(),
        "a falsey `muted` write must unmute the element even with no \
         attribute to remove (#754, mirroring #687's `checked`/`selected` fix)"
    );

    f.teardown();
}
