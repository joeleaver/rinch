//! Browser-driven tests for a `<textarea>`'s text children on rinch-web
//! (issue #1206).
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test textarea_text_child
//! ```
//!
//! The rule, the same on both backends (desktop since #1186/PR #1208): a
//! textarea's text children are its **default value**. The shown value follows
//! them until the textarea's *dirty value flag* is set — by a user edit or by a
//! write of its `value` — and from then on a child change reaches the default
//! value (`defaultValue`, `textContent`) and not the shown one. That is the
//! browser's own behaviour for a child-text change, so rinch-web now leaves it
//! to the browser. It used to write `.value` from the children on every change
//! (#100), which made `textarea { {|| draft.get()} }` a controlled field; a
//! controlled textarea is spelled `value_fn` (or a reactive `value:`), and
//! [`value_fn_clears_a_typed_textarea_after_submit`] pins that route.
#![cfg(target_arch = "wasm32")]

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use rinch_web::RootHandle;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Marks a fixture's host so a later test can purge one a failed test left in
/// the body (a failed assertion never reaches its own `teardown`).
const HOST_MARKER: &str = "data-test-host-1206";

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

    /// The fixture's one `<textarea>`.
    fn textarea(&self) -> web_sys::HtmlTextAreaElement {
        self.host
            .query_selector("textarea")
            .unwrap()
            .expect("no <textarea> in the fixture")
            .dyn_into()
            .unwrap()
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
}

/// A real edit, as the user's typing makes it: focus the field, select its
/// text and insert `text` over it through the browser's editing machinery.
/// That sets the dirty value flag and fires a genuine `input` event, which is
/// what rinch's `oninput` delegation listens for.
fn type_over(ta: &web_sys::HtmlTextAreaElement, text: &str) {
    ta.focus().unwrap();
    ta.select();
    let exec: js_sys::Function = js_sys::Reflect::get(&document(), &"execCommand".into())
        .unwrap()
        .dyn_into()
        .unwrap();
    let done = exec
        .call3(
            &document(),
            &"insertText".into(),
            &false.into(),
            &text.into(),
        )
        .unwrap();
    assert_eq!(done.as_bool(), Some(true), "execCommand(insertText) ran");
    assert_eq!(
        ta.value(),
        text,
        "positive control: the edit reached the field"
    );
}

/// A textarea whose one text child is driven by `draft` — the reactive-child
/// spelling, with the handle kept so a test can write `value` through rinch.
fn reactive_child(draft: Signal<String>, slot: Rc<RefCell<Option<NodeHandle>>>) -> Fixture {
    Fixture::mount(move |__scope: &mut RenderScope| {
        let ta = rsx! { textarea { {|| draft.get()} } };
        *slot.borrow_mut() = Some(ta.clone());
        ta
    })
}

/// The mount shape and the positive control for everything below: before any
/// edit the field shows its children, and a child change is shown.
#[wasm_bindgen_test]
fn a_child_change_before_any_edit_is_shown() {
    let draft = Signal::new(String::from("first"));
    let fixture = reactive_child(draft, Rc::new(RefCell::new(None)));
    let ta = fixture.textarea();
    assert_eq!(ta.value(), "first", "the text child is the initial value");

    draft.set("second draft".into());
    assert_eq!(ta.default_value().unwrap(), "second draft");
    assert_eq!(
        ta.value(),
        "second draft",
        "a pristine textarea follows its children"
    );

    // Focus is not an edit: the field still follows its children.
    ta.focus().unwrap();
    draft.set("third".into());
    assert_eq!(
        ta.value(),
        "third",
        "focusing does not set the dirty value flag"
    );
    fixture.teardown();
}

/// #100's shape, and what this issue changes about it: `oninput` writes the
/// draft that feeds the child, the user types, and the app then clears the
/// draft. The child — the default value — is cleared; what the user typed
/// stays in the field, as in Chrome and on desktop.
#[wasm_bindgen_test]
fn a_child_change_after_a_user_edit_is_not_shown() {
    let draft = Signal::new(String::from("seed"));
    let fixture = Fixture::mount(move |__scope: &mut RenderScope| {
        rsx! {
            textarea {
                oninput: move |v: String| draft.set(v),
                {|| draft.get()}
            }
        }
    });
    let ta = fixture.textarea();

    type_over(&ta, "typed by the user");
    assert_eq!(
        draft.get(),
        "typed by the user",
        "positive control: the edit reached rinch's oninput"
    );

    // The #100 flow: type, click Send (which blurs), then the app clears.
    ta.blur().unwrap();
    draft.set(String::new());
    assert_eq!(
        ta.default_value().unwrap(),
        "",
        "the child change reaches the default value"
    );
    assert_eq!(
        ta.value(),
        "typed by the user",
        "an edited textarea keeps what the user typed when its child changes"
    );

    draft.set("something else".into());
    assert_eq!(ta.default_value().unwrap(), "something else");
    assert_eq!(ta.value(), "typed by the user");
    fixture.teardown();
}

/// A `value` write sets the dirty value flag too: after it, a child change
/// is not shown.
#[wasm_bindgen_test]
fn a_child_change_after_a_value_write_is_not_shown() {
    let draft = Signal::new(String::from("child"));
    let slot = Rc::new(RefCell::new(None));
    let fixture = reactive_child(draft, slot.clone());
    let ta = fixture.textarea();
    let handle = slot.borrow().clone().expect("the textarea's handle");

    handle.set_attribute("value", "written");
    assert_eq!(ta.value(), "written", "a value write reaches .value");

    draft.set("changed child".into());
    assert_eq!(ta.default_value().unwrap(), "changed child");
    assert_eq!(
        ta.value(),
        "written",
        "a textarea whose value was written does not follow its children"
    );
    fixture.teardown();
}

/// Issue #1222 case 2: a `value` write **equal** to the shown text sets the
/// dirty value flag too, as a script `.value` write does in Chrome 153
/// ([`chrome_sets_the_flag_on_an_equal_script_write`]). rinch-web skipped the
/// equal write, so the flag stayed clear and the field went on following its
/// children — where desktop, whose flag is the attribute, froze.
#[wasm_bindgen_test]
fn an_equal_value_write_sets_the_dirty_flag_1222() {
    let draft = Signal::new(String::from("same"));
    let slot = Rc::new(RefCell::new(None));
    let fixture = reactive_child(draft, slot.clone());
    let ta = fixture.textarea();
    let handle = slot.borrow().clone().expect("the textarea's handle");

    handle.set_attribute("value", "same");
    draft.set("changed".into());
    assert_eq!(ta.default_value().unwrap(), "changed", "positive control");
    assert_eq!(ta.value(), "same", "an equal write froze the field");
    fixture.teardown();
}

/// The equal write leaves a focused field's caret where it was: per HTML
/// the setter moves the cursor only when the value changed.
#[wasm_bindgen_test]
fn an_equal_value_write_keeps_the_caret_1222() {
    let draft = Signal::new(String::from("abcdef"));
    let slot = Rc::new(RefCell::new(None));
    let fixture = reactive_child(draft, slot.clone());
    let ta = fixture.textarea();
    let handle = slot.borrow().clone().expect("the textarea's handle");

    ta.focus().unwrap();
    ta.set_selection_range(2, 4).unwrap();
    handle.set_attribute("value", "abcdef");
    assert_eq!(ta.selection_start().unwrap(), Some(2));
    assert_eq!(ta.selection_end().unwrap(), Some(4));
    fixture.teardown();
}

/// The oracle for the two fixtures above: Chrome sets the flag on a script
/// `.value` write equal to the current value.
#[wasm_bindgen_test]
fn chrome_sets_the_flag_on_an_equal_script_write() {
    let ta: web_sys::HtmlTextAreaElement = document()
        .create_element("textarea")
        .unwrap()
        .dyn_into()
        .unwrap();
    document().body().unwrap().append_child(&ta).unwrap();
    ta.set_text_content(Some("same"));
    ta.set_value("same");
    ta.set_text_content(Some("changed"));
    assert_eq!(ta.value(), "same");
    ta.remove();
}

/// Issue #1222 case 1: removing the `value` attribute writes `.value = ""`,
/// which leaves the flag set; a page cannot clear it. Desktop now agrees
/// (it used to fall back to the children). A parity pin: this was the web's
/// behaviour before #1222.
#[wasm_bindgen_test]
fn removing_a_written_value_leaves_the_field_empty_and_dirty_1222() {
    let draft = Signal::new(String::from("child"));
    let slot = Rc::new(RefCell::new(None));
    let fixture = reactive_child(draft, slot.clone());
    let ta = fixture.textarea();
    let handle = slot.borrow().clone().expect("the textarea's handle");

    handle.set_attribute("value", "written");
    handle.remove_attribute("value");
    assert_eq!(ta.value(), "");
    assert_eq!(ta.get_attribute("value"), None);
    draft.set("changed".into());
    assert_eq!(ta.default_value().unwrap(), "changed", "positive control");
    assert_eq!(ta.value(), "");
    fixture.teardown();
}

/// The same from a pristine field that never had the attribute.
#[wasm_bindgen_test]
fn removing_an_absent_value_from_a_pristine_textarea_empties_it_1222() {
    let draft = Signal::new(String::from("child"));
    let slot = Rc::new(RefCell::new(None));
    let fixture = reactive_child(draft, slot.clone());
    let ta = fixture.textarea();
    let handle = slot.borrow().clone().expect("the textarea's handle");

    assert_eq!(ta.value(), "child", "positive control");
    handle.remove_attribute("value");
    assert_eq!(ta.value(), "");
    draft.set("changed".into());
    assert_eq!(ta.value(), "");
    fixture.teardown();
}

/// A child set directly on the textarea element (`set_text`, i.e. `set_text_content`, on the
/// element, not on its text node) follows the same rule.
#[wasm_bindgen_test]
fn set_text_content_on_an_edited_textarea_changes_only_its_default() {
    let slot = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let fixture = Fixture::mount(move |__scope: &mut RenderScope| {
        let ta = rsx! { textarea { "initial" } };
        *slot_in.borrow_mut() = Some(ta.clone());
        ta
    });
    let ta = fixture.textarea();
    let handle = slot.borrow().clone().expect("the textarea's handle");

    handle.set_text("pristine change");
    assert_eq!(
        ta.value(),
        "pristine change",
        "positive control: pristine follows"
    );

    type_over(&ta, "edited");
    handle.set_text("after the edit");
    assert_eq!(ta.default_value().unwrap(), "after the edit");
    assert_eq!(ta.value(), "edited");
    fixture.teardown();
}

/// The controlled textarea: `value_fn` + `oninput`, cleared after a submit.
/// This is the route the #100 shape moves to, and it reaches a field the user
/// has typed into.
#[wasm_bindgen_test]
fn value_fn_clears_a_typed_textarea_after_submit() {
    let draft = Signal::new(String::new());
    let fixture = Fixture::mount(move |__scope: &mut RenderScope| {
        rsx! {
            div {
                Textarea {
                    value_fn: move || draft.get(),
                    oninput: move |v: String| draft.set(v),
                }
            }
        }
    });
    let ta = fixture.textarea();

    type_over(&ta, "a message");
    assert_eq!(
        draft.get(),
        "a message",
        "positive control: oninput reached rinch"
    );

    // The submit handler clears the draft.
    draft.set(String::new());
    assert_eq!(ta.value(), "", "value_fn clears an edited textarea");

    draft.set("restored".into());
    assert_eq!(ta.value(), "restored");
    fixture.teardown();
}

/// The raw-element spelling of a controlled textarea: a reactive `value:`.
#[wasm_bindgen_test]
fn a_reactive_value_attribute_clears_a_typed_textarea() {
    let draft = Signal::new(String::new());
    let fixture = Fixture::mount(move |__scope: &mut RenderScope| {
        rsx! {
            textarea {
                value: {|| draft.get()},
                oninput: move |v: String| draft.set(v),
            }
        }
    });
    let ta = fixture.textarea();

    type_over(&ta, "typed");
    assert_eq!(draft.get(), "typed");
    draft.set(String::new());
    assert_eq!(
        ta.value(),
        "",
        "a reactive value: reaches an edited textarea"
    );
    fixture.teardown();
}

/// The `value_fn` example in `docs/src/guide/rsx-syntax.md` ("Sizing a
/// `<textarea>`"), kept here so it is known to compile. Keep the two in step.
#[allow(dead_code)]
#[component]
fn guide_example() -> NodeHandle {
    fn send(_: String) {}
    let draft = Signal::new(String::new());
    rsx! {
        div {
            Textarea {
                value_fn: move || draft.get(),
                oninput: move |v: String| draft.set(v),
            }
            textarea { value: {|| draft.get()}, oninput: move |v: String| draft.set(v) }
            button { onclick: move || { send(draft.get()); draft.set(String::new()); }, "Send" }
        }
    }
}
