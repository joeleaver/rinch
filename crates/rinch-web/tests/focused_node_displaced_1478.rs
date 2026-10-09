//! Taking the focused element out of place does not run the browser's focus
//! listeners with the document borrowed (issue #1478).
//!
//! Chrome fires `focusout` / `blur` (and `change`, for a modified field)
//! synchronously inside `removeChild`, `appendChild`, `insertBefore`,
//! `replaceChild`, `innerHTML =` and `textContent =` when the node that goes
//! holds the focus. rinch made each of those calls under the `borrow_mut` its
//! `NodeHandle` verb held, and rinch's own `focusout` listener asks every live
//! key entry (an open `Select` list) for `active_element()`, which borrows the
//! document again: "RefCell already mutably borrowed". The panic aborted the
//! listener, so the entry never heard the focus leave and the list stayed open.
//!
//! Every such verb now releases the focus first, with no borrow held
//! (`DomDocument::release_focus_within`): the listeners run against a free
//! document in which the node is still where it was, and then the node goes.
//!
//! Chrome also blurs the focused element inside three attribute writes
//! (`hidden` on it; `tabindex` / `contenteditable` removed from an element
//! focusable only through it). Only the browser knows which write is one, so
//! for those names the browser write itself is made first, with no borrow held
//! (`DomDocument::before_attribute_write`).
//!
//! Firefox fires nothing when a focused node is removed. Because the release
//! is rinch's own `blur()`, which fires `focusout` in every engine, none of
//! these fixtures depends on which engine it runs in.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test focused_node_displaced_1478
//! ```
#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

#[path = "support/engine.rs"]
mod engine;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn host() -> web_sys::Element {
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    host
}

fn active_id() -> String {
    document()
        .active_element()
        .map(|el| el.id())
        .unwrap_or_default()
}

fn by_id(id: &str) -> web_sys::HtmlElement {
    document()
        .get_element_by_id(id)
        .unwrap_or_else(|| panic!("no #{id}"))
        .dyn_into()
        .unwrap()
}

/// What the key entry's `on_focus_leave` saw when it ran: it is handler code,
/// so it reads the document through `NodeHandle`s.
#[derive(Default)]
struct Seen {
    /// How many times focus left the owner.
    left: Cell<u32>,
    /// Whether the owner still had a parent in the document then.
    attached: Cell<bool>,
    /// Whether a write through a handle landed (the node was not yet retired).
    wrote: Cell<bool>,
}

struct Fx {
    root: rinch_web::RootHandle,
    host: web_sys::Element,
    /// `<div id="wrap-{n}">`, holding the trigger.
    wrap: NodeHandle,
    /// `<{tag} id="trigger-{n}">`, focused, the owner of a live key entry.
    trigger: NodeHandle,
    /// `<button id="other-{n}">`, a sibling of `wrap`.
    other: NodeHandle,
    page: NodeHandle,
    seen: Rc<Seen>,
    n: &'static str,
    _keep: Rc<RefCell<Vec<rinch_core::DismissHandle>>>,
}

impl Fx {
    fn done(self) {
        self.root.unmount();
        self.host.remove();
    }

    /// The document is free again and nothing is left half done.
    fn assert_usable(&self) {
        self.other.set_attribute("data-after", "1");
        assert_eq!(
            by_id(&format!("other-{}", self.n))
                .get_attribute("data-after")
                .as_deref(),
            Some("1"),
            "a later write through a handle lands"
        );
    }

    fn assert_left_once(&self) {
        assert_eq!(
            self.seen.left.get(),
            1,
            "the entry heard focus leave its owner"
        );
        assert!(
            self.seen.attached.get(),
            "and its owner was still in the document when it heard"
        );
        assert!(
            self.seen.wrote.get(),
            "and a handle write from the listener landed"
        );
    }
}

/// `page > [wrap > trigger, other]`, the trigger focused and owning a live key
/// entry — an open `Select` list's shape, bare. `n` keeps ids unique per test,
/// so one red test cannot hand its stale nodes to the next.
fn fixture(tag: &str, n: &'static str) -> Fx {
    let host = host();
    let seen = Rc::new(Seen::default());
    let keep: Rc<RefCell<Vec<rinch_core::DismissHandle>>> = Default::default();
    type Slots = (NodeHandle, NodeHandle, NodeHandle, NodeHandle);
    let slots: Rc<RefCell<Option<Slots>>> = Default::default();
    let (sn, k, sl) = (seen.clone(), keep.clone(), slots.clone());
    let tag = tag.to_string();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |s| {
        let page = s.create_element("div");
        let wrap = s.create_element("div");
        wrap.set_attribute("id", &format!("wrap-{n}"));
        // `tag` is a tag name, optionally followed by an attribute that makes
        // it focusable.
        let mut parts = tag.split(' ');
        let trigger = s.create_element(parts.next().unwrap());
        match parts.next() {
            Some("tabindex") => trigger.set_attribute("tabindex", "0"),
            Some(attr) => trigger.set_attribute(attr, "true"),
            None => {}
        }
        trigger.set_attribute("id", &format!("trigger-{n}"));
        wrap.append_child(&trigger);
        page.append_child(&wrap);
        let other = s.create_element("button");
        other.set_attribute("id", &format!("other-{n}"));
        page.append_child(&other);
        let (sn, t) = (sn.clone(), trigger.clone());
        k.borrow_mut().push(rinch_core::push_key_handler(
            &trigger,
            |_| false,
            move || {
                sn.left.set(sn.left.get() + 1);
                sn.attached.set(
                    t.parent_node()
                        .and_then(|p| p.parent_node())
                        .is_some_and(|_| t.get_attribute("id").is_some()),
                );
                t.set_attribute("data-left", "1");
                sn.wrote
                    .set(t.get_attribute("data-left").as_deref() == Some("1"));
            },
        ));
        *sl.borrow_mut() = Some((page.clone(), wrap, trigger, other));
        page
    });
    let (page, wrap, trigger, other) = slots.borrow_mut().take().unwrap();
    by_id(&format!("trigger-{n}")).focus().unwrap();
    assert_eq!(
        active_id(),
        format!("trigger-{n}"),
        "positive control: the trigger holds the focus"
    );
    assert_eq!(seen.left.get(), 0, "positive control: nothing left yet");
    Fx {
        root,
        host,
        wrap,
        trigger,
        other,
        page,
        seen,
        n,
        _keep: keep,
    }
}

fn gone(id: &str) -> bool {
    document().get_element_by_id(id).is_none()
}

#[wasm_bindgen_test]
fn removing_the_focused_owner() {
    let f = fixture("button", "rm");
    f.trigger.remove();
    assert!(gone("trigger-rm"), "it is out of the document");
    f.assert_left_once();
    f.assert_usable();
    // A removed node comes back (#719), unfocused.
    f.wrap.append_child(&f.trigger);
    assert!(!gone("trigger-rm"));
    assert_ne!(active_id(), "trigger-rm");
    f.done();
}

#[wasm_bindgen_test]
fn removing_an_ancestor_of_the_focused_owner() {
    let f = fixture("button", "rma");
    f.wrap.remove();
    assert!(gone("trigger-rma"));
    f.assert_left_once();
    f.assert_usable();
    f.done();
}

#[wasm_bindgen_test]
fn remove_child_of_the_focused_owner() {
    let f = fixture("button", "rmc");
    f.page.remove_child(&f.wrap);
    assert!(gone("trigger-rmc"));
    f.assert_left_once();
    f.assert_usable();
    f.done();
}

/// A discard retires the node. The listener ran before that, so its handle
/// still worked.
#[wasm_bindgen_test]
fn discarding_the_focused_owner() {
    let f = fixture("button", "dis");
    f.trigger.discard();
    assert!(gone("trigger-dis"));
    f.assert_left_once();
    f.assert_usable();
    f.done();
}

#[wasm_bindgen_test]
fn inner_html_over_the_focused_owner() {
    let f = fixture("button", "html");
    f.wrap.set_inner_html("<p id=\"fresh-html\">gone</p>");
    assert!(gone("trigger-html"));
    assert!(!gone("fresh-html"), "the new markup is in");
    f.assert_left_once();
    f.assert_usable();
    f.done();
}

/// `set_text` on an element replaces its children.
#[wasm_bindgen_test]
fn text_written_over_the_focused_owner() {
    let f = fixture("button", "txt");
    f.wrap.set_text("gone");
    assert!(gone("trigger-txt"));
    assert_eq!(by_id("wrap-txt").text_content().as_deref(), Some("gone"));
    f.assert_left_once();
    f.assert_usable();
    f.done();
}

/// `set_inner_html` and `set_text` on the focused element itself take nothing
/// out of place: it keeps the focus.
#[wasm_bindgen_test]
fn content_written_into_the_focused_owner_keeps_its_focus() {
    let f = fixture("button", "self");
    f.trigger.set_text("label");
    f.trigger.set_inner_html("<b>label</b>");
    assert_eq!(active_id(), "trigger-self", "still focused");
    assert_eq!(f.seen.left.get(), 0, "focus did not leave");
    f.assert_usable();
    f.done();
}

#[wasm_bindgen_test]
fn replacing_the_focused_owner() {
    let f = fixture("button", "rep");
    let fresh = f.other.clone();
    f.wrap.replace_with(&fresh);
    assert!(gone("trigger-rep"));
    assert!(!gone("other-rep"));
    f.assert_left_once();
    f.assert_usable();
    f.done();
}

/// The replacement is the node that moves.
#[wasm_bindgen_test]
fn replacing_another_node_with_the_focused_owner() {
    let f = fixture("button", "repby");
    f.other.replace_with(&f.wrap);
    assert!(gone("other-repby"));
    assert!(!gone("trigger-repby"), "the trigger is where `other` was");
    f.assert_left_once();
    f.trigger.set_attribute("data-after", "1");
    f.done();
}

/// A move: `append_child` of a node that is already in the document. A browser
/// drops the focus of a node it moves.
#[wasm_bindgen_test]
fn appending_the_focused_owner_somewhere_else() {
    let f = fixture("button", "mv");
    f.page.append_child(&f.wrap);
    let last = by_id("wrap-mv")
        .parent_element()
        .unwrap()
        .last_element_child()
        .unwrap();
    assert_eq!(last.id(), "wrap-mv", "it moved to the end");
    assert_ne!(active_id(), "trigger-mv", "a moved node loses the focus");
    f.assert_left_once();
    f.assert_usable();
    f.done();
}

/// The keyed `for` reorder's verbs.
#[wasm_bindgen_test]
fn inserting_the_focused_owner_before_and_after_a_sibling() {
    let f = fixture("button", "ins");
    // [wrap, other] -> [other, wrap]
    f.other.insert_after(&f.wrap);
    assert_eq!(
        by_id("other-ins").next_element_sibling().unwrap().id(),
        "wrap-ins"
    );
    f.assert_left_once();
    // Focus it again and move it back: [wrap, other].
    by_id("trigger-ins").focus().unwrap();
    assert_eq!(
        active_id(),
        "trigger-ins",
        "positive control: focused again"
    );
    f.page.insert_before(&f.wrap, &f.other);
    assert_eq!(
        by_id("wrap-ins").next_element_sibling().unwrap().id(),
        "other-ins"
    );
    assert_eq!(f.seen.left.get(), 2, "the second move was heard too");
    f.assert_usable();
    f.done();
}

/// An unfocused node moves with no focus event at all.
#[wasm_bindgen_test]
fn moving_a_node_that_does_not_hold_the_focus_leaves_focus_alone() {
    let f = fixture("button", "calm");
    f.page.insert_before(&f.other, &f.wrap);
    f.other.remove();
    f.page.append_child(&f.other);
    assert_eq!(active_id(), "trigger-calm", "the trigger kept the focus");
    assert_eq!(f.seen.left.get(), 0);
    f.done();
}

/// A modified field fires `change` when it loses the focus: an `onchange`
/// that writes through a handle runs before the field goes.
#[wasm_bindgen_test]
fn removing_a_modified_focused_field_runs_onchange() {
    let host = host();
    let show = Signal::new(true);
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |__scope: &mut RenderScope| {
            let echo = __scope.create_element("p");
            echo.set_attribute("id", "echo-chg");
            let e = echo.clone();
            // A captured field: the hide only removes it.
            let field = rsx! {
                input { id: "field-chg", onchange: move |v: String| e.set_attribute("data-v", &v) }
            };
            let page = rsx! { div { if show.get() { {field.clone()} } } };
            page.append_child(&echo);
            page
        },
    );
    by_id("field-chg").focus().unwrap();
    let exec: js_sys::Function = js_sys::Reflect::get(&document(), &"execCommand".into())
        .unwrap()
        .dyn_into()
        .unwrap();
    let ok = exec
        .call3(
            &document(),
            &"insertText".into(),
            &false.into(),
            &"typed".into(),
        )
        .unwrap();
    assert!(ok.as_bool().unwrap_or(false), "positive control: typed");
    show.set(false);
    assert!(gone("field-chg"));
    assert_eq!(
        by_id("echo-chg").get_attribute("data-v").as_deref(),
        Some("typed"),
        "onchange's handle write landed"
    );
    root.unmount();
    host.remove();
}

/// The realistic shape (the #654 one): `if show { {panel} }` with a captured
/// panel holding the focused owner of a key entry the *outer* scope registered.
/// The hide only removes the panel, so the entry is still live when it goes.
#[wasm_bindgen_test]
fn hiding_a_captured_panel_that_holds_the_focused_owner() {
    let host = host();
    let show = Signal::new(true);
    let left = Rc::new(Cell::new(0u32));
    let keep: Rc<RefCell<Vec<rinch_core::DismissHandle>>> = Default::default();
    let (l, k) = (left.clone(), keep.clone());
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |__scope: &mut RenderScope| {
            let panel = __scope.create_element("div");
            let t = __scope.create_element("button");
            t.set_attribute("id", "trigger-panel");
            panel.append_child(&t);
            let l = l.clone();
            k.borrow_mut().push(rinch_core::push_key_handler(
                &t,
                |_| false,
                move || l.set(l.get() + 1),
            ));
            rsx! { div { if show.get() { {panel.clone()} } } }
        },
    );
    by_id("trigger-panel").focus().unwrap();
    assert_eq!(active_id(), "trigger-panel", "positive control");
    show.set(false);
    assert!(gone("trigger-panel"), "hidden");
    assert_eq!(left.get(), 1, "the entry heard focus leave");
    show.set(true);
    assert!(!gone("trigger-panel"), "shown again");
    root.unmount();
    host.remove();
}

/// The same hide with a real `Select` whose list is open: the list closes.
#[wasm_bindgen_test]
fn hiding_a_captured_panel_closes_the_open_select_inside_it() {
    use rinch::components::{Select, SelectOption};
    let host = host();
    let show = Signal::new(true);
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |__scope: &mut RenderScope| {
            let panel = __scope.create_element("div");
            let sel = Select {
                value: "a".into(),
                data: vec![SelectOption::new("a", "A"), SelectOption::new("b", "B")],
                ..Default::default()
            }
            .render(__scope, &[]);
            panel.append_child(&sel);
            rsx! { div { if show.get() { {panel.clone()} } } }
        },
    );
    let trigger: web_sys::HtmlElement = document()
        .query_selector(".rinch-select__input")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    trigger.focus().unwrap();
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key("Enter");
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    trigger.dispatch_event(&ev).unwrap();
    assert_eq!(
        trigger.get_attribute("aria-expanded").as_deref(),
        Some("true"),
        "positive control: the key reached rinch and opened the list"
    );
    show.set(false);
    assert!(!trigger.is_connected(), "hidden");
    assert_eq!(
        trigger.get_attribute("aria-expanded").as_deref(),
        Some("false"),
        "focus left the trigger, so the list closed"
    );
    show.set(true);
    assert!(trigger.is_connected(), "shown again");
    root.unmount();
    host.remove();
}

/// Unmounting a root whose focused node owns a key entry that outlives the
/// root (registered outside any render).
#[wasm_bindgen_test]
fn unmounting_a_root_that_holds_the_focused_owner() {
    let host = host();
    let slot: Rc<RefCell<Option<NodeHandle>>> = Default::default();
    let sl = slot.clone();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |s| {
        let page = s.create_element("div");
        let t = s.create_element("button");
        t.set_attribute("id", "trigger-unmount");
        page.append_child(&t);
        *sl.borrow_mut() = Some(t);
        page
    });
    let t = slot.borrow_mut().take().unwrap();
    let left = Rc::new(Cell::new(0u32));
    let l = left.clone();
    let _entry = rinch_core::push_key_handler(&t, |_| false, move || l.set(l.get() + 1));
    by_id("trigger-unmount").focus().unwrap();
    assert_eq!(active_id(), "trigger-unmount", "positive control");
    root.unmount();
    assert!(gone("trigger-unmount"));
    assert_eq!(left.get(), 1, "the entry heard focus leave");
    host.remove();
}

/// A listener that puts the focus back inside the subtree that is going. The
/// verb lets go of that focus too before it removes anything.
#[wasm_bindgen_test]
fn a_listener_that_refocuses_inside_the_subtree_being_removed() {
    let host = host();
    let keep: Rc<RefCell<Vec<rinch_core::DismissHandle>>> = Default::default();
    let slots: Rc<RefCell<Option<(NodeHandle, NodeHandle)>>> = Default::default();
    let calls = Rc::new(Cell::new(0u32));
    let (k, sl, c) = (keep.clone(), slots.clone(), calls.clone());
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |s| {
        let page = s.create_element("div");
        let wrap = s.create_element("div");
        let mut mk = |id: &str, into: &NodeHandle| {
            let b = s.create_element("button");
            b.set_attribute("id", id);
            into.append_child(&b);
            b
        };
        let (t, z) = (mk("trigger-stubborn", &wrap), mk("z-stubborn", &wrap));
        page.append_child(&wrap);
        let other = mk("other-stubborn", &page);
        let c = c.clone();
        k.borrow_mut().push(rinch_core::push_key_handler(
            &t,
            |_| false,
            move || {
                c.set(c.get() + 1);
                // Once: the first time focus leaves, send it to a sibling
                // that is going as well.
                if c.get() == 1 {
                    z.focus();
                }
            },
        ));
        *sl.borrow_mut() = Some((wrap, other));
        page
    });
    let (wrap, other) = slots.borrow_mut().take().unwrap();
    by_id("trigger-stubborn").focus().unwrap();
    assert_eq!(active_id(), "trigger-stubborn", "positive control");
    wrap.remove();
    assert!(gone("trigger-stubborn") && gone("z-stubborn"), "removed");
    // An entry is told on every focus move that leaves its owner unfocused:
    // the trigger's blur, the `focusin` of the sibling the listener focused,
    // and that sibling's own blur, which is the second release. Without it
    // the sibling goes with the focus and its `focusout` runs under the
    // removal's borrow, so the third never arrives.
    // Chrome 153's count. No Firefox was measured for a `focus()` made from
    // inside a `blur()`'s own `focusout`, so there only the first is pinned.
    if engine::is_gecko() {
        assert!(
            calls.get() >= 1,
            "the entry heard the trigger lose the focus"
        );
    } else {
        assert_eq!(
            calls.get(),
            3,
            "the sibling the listener focused was released too"
        );
    }
    other.set_attribute("data-after", "1");
    assert_eq!(
        by_id("other-stubborn")
            .get_attribute("data-after")
            .as_deref(),
        Some("1")
    );
    root.unmount();
    host.remove();
}

/// What a write in place did to the focus, per engine.
///
/// Chrome 153 blurs the focused element **inside** three attribute writes
/// (`hidden` on it; `tabindex` or `contenteditable` removed from an element
/// focusable only through it), so there the entry must have heard it, which it
/// did not while the listener ran under the write's borrow. No Firefox was
/// measured: there the fixture pins only that the write lands and the document
/// is usable.
fn assert_blurred_inside_the_write(f: &Fx) {
    if engine::is_gecko() {
        return;
    }
    assert_eq!(
        f.seen.left.get(),
        1,
        "Chrome blurs inside this write, and the entry heard it"
    );
    assert!(f.seen.wrote.get(), "and its handle write landed");
}

/// Chrome 153 fires nothing inside these writes, nor inside the forced layout
/// after them: it drops such a focus later, or (`disabled`) not at all.
fn assert_no_blur_inside_the_write(f: &Fx) {
    if engine::is_gecko() {
        return;
    }
    assert_eq!(f.seen.left.get(), 0, "Chrome fires no focusout here");
}

/// A write that makes the focused element unfocusable in place, then a
/// geometry read (which forces style and layout), with a live key entry.
fn in_place(n: &'static str, tag: &str, write: impl FnOnce(&Fx), check: fn(&Fx)) {
    let f = fixture(tag, n);
    write(&f);
    let _ = f.trigger.get_layout_bounds();
    let _ = f.other.get_layout_bounds();
    check(&f);
    f.assert_usable();
    f.done();
}

#[wasm_bindgen_test]
fn hiding_the_focused_owner_with_the_hidden_attribute() {
    in_place(
        "hidden",
        "button",
        |f| {
            f.trigger.write_attribute("hidden", "true");
            assert!(by_id("trigger-hidden").has_attribute("hidden"));
        },
        assert_blurred_inside_the_write,
    );
}

/// A `<div>` is focusable only through its `tabindex`.
#[wasm_bindgen_test]
fn removing_the_tabindex_that_made_the_owner_focusable() {
    in_place(
        "tabindex",
        "div tabindex",
        |f| {
            f.trigger.remove_attribute("tabindex");
            assert!(!by_id("trigger-tabindex").has_attribute("tabindex"));
        },
        assert_blurred_inside_the_write,
    );
}

#[wasm_bindgen_test]
fn removing_the_contenteditable_that_made_the_owner_focusable() {
    in_place(
        "editable",
        "div contenteditable",
        |f| {
            f.trigger.remove_attribute("contenteditable");
            assert!(!by_id("trigger-editable").has_attribute("contenteditable"));
        },
        assert_blurred_inside_the_write,
    );
}

/// The roving-tabindex write: the focused item stays focusable, so it keeps
/// the focus in every engine. (Blurring before any `tabindex` write would
/// lose it.)
#[wasm_bindgen_test]
fn rewriting_the_tabindex_of_the_focused_owner_keeps_its_focus() {
    let f = fixture("div tabindex", "roving");
    f.trigger.set_attribute("tabindex", "-1");
    f.trigger.set_attribute("tabindex", "0");
    assert_eq!(active_id(), "trigger-roving", "still focused");
    assert_eq!(f.seen.left.get(), 0, "focus did not leave");
    f.assert_usable();
    f.done();
}

#[wasm_bindgen_test]
fn disabling_the_focused_owner() {
    in_place(
        "disabled",
        "button",
        |f| f.trigger.write_attribute("disabled", "true"),
        assert_no_blur_inside_the_write,
    );
}

#[wasm_bindgen_test]
fn hiding_an_ancestor_of_the_focused_owner() {
    in_place(
        "displaynone",
        "button",
        |f| f.wrap.set_style("display", "none"),
        assert_no_blur_inside_the_write,
    );
    in_place(
        "hiddenwrap",
        "button",
        |f| f.wrap.write_attribute("hidden", "true"),
        assert_no_blur_inside_the_write,
    );
}

thread_local! {
    static AFTER: Cell<Signal<u32>> = Cell::new(Signal::new(0));
}

/// A listener that puts the focus back inside the departing subtree **every**
/// time (a field that refocuses itself on blur). The last release keeps it
/// from coming back, so no listener runs inside the removal. One that did was
/// aborted there with the runtime's flush guard set, and from then on a plain
/// signal write no longer reached the DOM.
#[wasm_bindgen_test]
fn a_listener_that_always_refocuses_inside_the_subtree_being_removed() {
    let host = host();
    let keep: Rc<RefCell<Vec<rinch_core::DismissHandle>>> = Default::default();
    let slots: Rc<RefCell<Option<(NodeHandle, NodeHandle)>>> = Default::default();
    let calls = Rc::new(Cell::new(0u32));
    let (k, sl, c) = (keep.clone(), slots.clone(), calls.clone());
    let after = AFTER.with(|a| a.get());
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |s| {
        let page = s.create_element("div");
        let wrap = s.create_element("div");
        wrap.set_attribute("id", "wrap-always");
        let mut mk = |id: &str, into: &NodeHandle| {
            let b = s.create_element("button");
            b.set_attribute("id", id);
            into.append_child(&b);
            b
        };
        let (t, z) = (mk("trigger-always", &wrap), mk("z-always", &wrap));
        page.append_child(&wrap);
        let other = mk("other-always", &page);
        // Asked afterwards: do effects still flush?
        let live = other.clone();
        s.create_effect(move || live.set_attribute("data-n", &after.get().to_string()));
        let c = c.clone();
        k.borrow_mut().push(rinch_core::push_key_handler(
            &t,
            |_| false,
            move || {
                c.set(c.get() + 1);
                if c.get() < 50 {
                    z.focus();
                }
            },
        ));
        *sl.borrow_mut() = Some((wrap, other));
        page
    });
    let (wrap, other) = slots.borrow_mut().take().unwrap();
    by_id("trigger-always").focus().unwrap();
    assert_eq!(active_id(), "trigger-always", "positive control");
    wrap.remove();
    assert!(gone("trigger-always") && gone("z-always"), "removed");
    assert!(calls.get() >= 1, "the entry heard the focus leave");
    assert!(calls.get() < 50, "and the listener was not still at it");
    assert!(
        !wrap.get_attribute("inert").is_some(),
        "the node is not left inert"
    );
    other.set_attribute("data-after", "1");
    after.set(7);
    assert_eq!(
        by_id("other-always").get_attribute("data-n").as_deref(),
        Some("7"),
        "a signal write after the removal still reaches the DOM"
    );
    // It comes back usable (#719): something in it takes the focus again.
    other.insert_after(&wrap);
    by_id("z-always").focus().unwrap();
    assert_eq!(active_id(), "z-always", "not left inert");
    root.unmount();
    host.remove();
}

/// A verb the browser refuses moves nothing, so it does not cost the focus.
#[wasm_bindgen_test]
fn a_move_that_cannot_happen_keeps_the_focus() {
    let f = fixture("button", "refused");
    // `other` is not a child of `wrap`: `insertBefore` throws.
    f.wrap.insert_before(&f.trigger, &f.other);
    assert_eq!(active_id(), "trigger-refused", "refused insert_before");
    // A node into its own descendant: a hierarchy error.
    f.trigger.append_child(&f.wrap);
    assert_eq!(active_id(), "trigger-refused", "refused cyclic append");
    f.trigger.insert_after(&f.page);
    assert_eq!(
        active_id(),
        "trigger-refused",
        "refused cyclic insert_after"
    );
    // Not `other`'s child.
    f.other.remove_child(&f.wrap);
    assert_eq!(active_id(), "trigger-refused", "refused remove_child");
    // A node replaced by itself, and by its own ancestor.
    f.wrap.replace_with(&f.wrap);
    f.trigger.replace_with(&f.page);
    assert_eq!(active_id(), "trigger-refused", "refused replace_with");
    assert_eq!(f.seen.left.get(), 0, "focus never left");
    assert_eq!(
        by_id("trigger-refused").parent_element().unwrap().id(),
        "wrap-refused",
        "and nothing moved"
    );
    // Positive control: the same verb, when it can run, does take the focus.
    f.page.insert_before(&f.wrap, &f.other);
    f.page.append_child(&f.wrap);
    assert_ne!(active_id(), "trigger-refused");
    f.done();
}

/// What the guide does **not** promise: a field the hidden branch *built* has
/// had its handlers freed before it goes (the branch's scope is disposed
/// first), so its `onchange` commits nothing. Only a captured field, or one
/// removed directly, still has a handler to run
/// (`removing_a_modified_focused_field_runs_onchange`).
#[wasm_bindgen_test]
fn a_field_the_hidden_branch_built_commits_nothing() {
    let host = host();
    let show = Signal::new(true);
    let got = Signal::new(String::from("untouched"));
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |__scope: &mut RenderScope| {
            rsx! {
                div {
                    if show.get() {
                        input { id: "field-built", onchange: move |v: String| got.set(v) }
                    }
                }
            }
        },
    );
    by_id("field-built").focus().unwrap();
    let exec: js_sys::Function = js_sys::Reflect::get(&document(), &"execCommand".into())
        .unwrap()
        .dyn_into()
        .unwrap();
    let ok = exec
        .call3(
            &document(),
            &"insertText".into(),
            &false.into(),
            &"typed".into(),
        )
        .unwrap();
    assert!(ok.as_bool().unwrap_or(false), "positive control: typed");
    show.set(false);
    assert!(gone("field-built"), "hidden");
    assert_eq!(got.get(), "untouched", "its handler was already freed");
    root.unmount();
    host.remove();
}

/// Counts `document.activeElement` reads and `Node.contains` calls.
const COUNTERS: &str = r#"(function(){
 if(window.__ae!==undefined){window.__ae=0;window.__ct=0;return 0;}
 const d=Object.getOwnPropertyDescriptor(Document.prototype,'activeElement');
 window.__ae=0;window.__ct=0;
 Object.defineProperty(Document.prototype,'activeElement',{get(){window.__ae++;return d.get.call(this)},configurable:true});
 const c=Node.prototype.contains;
 Node.prototype.contains=function(o){window.__ct++;return c.call(this,o)};
 return 0;})()"#;

fn js(src: &str) -> f64 {
    js_sys::eval(src).unwrap().as_f64().unwrap_or(-1.0)
}

/// The cost side: asking "is the focus in there" before every structural verb
/// and text write is free while nothing holds the focus, and a text write to a
/// text node never asks where the focus is.
#[wasm_bindgen_test]
fn asking_costs_no_browser_call_while_nothing_is_focused() {
    let host = host();
    let rows = Signal::new(Vec::<u32>::new());
    let tick = Signal::new(0u32);
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |__scope: &mut RenderScope| {
            rsx! {
                div {
                    input { id: "cost-in" }
                    p { id: "cost-tick", {|| tick.get().to_string()} }
                    ul { id: "cost-ul",
                        for n in rows.get() {
                            li { key: n, span { {n.to_string()} } }
                        }
                    }
                }
            }
        },
    );
    // Whatever an earlier test left focused, let it go.
    by_id("cost-in").focus().unwrap();
    by_id("cost-in").blur().unwrap();
    js(COUNTERS);
    let _ = document().active_element();
    let _ = document().body().unwrap().contains(None);
    assert_eq!(
        (js("window.__ae"), js("window.__ct")),
        (1.0, 1.0),
        "positive control: the counters count"
    );

    js(COUNTERS);
    rows.set((0..200).collect());
    for i in 1..=100 {
        tick.set(i);
    }
    rows.set((0..200).rev().collect());
    rows.set(Vec::new());
    assert_eq!(
        by_id("cost-tick").text_content().as_deref(),
        Some("100"),
        "positive control: the updates ran"
    );
    assert_eq!(
        (js("window.__ae"), js("window.__ct")),
        (0.0, 0.0),
        "nothing focused: no activeElement read, no contains call"
    );

    // With a field focused elsewhere on the page, a reactive text update (a
    // write to a text node) still asks neither.
    by_id("cost-in").focus().unwrap();
    assert_eq!(active_id(), "cost-in", "positive control: focused");
    js(COUNTERS);
    for i in 101..=200 {
        tick.set(i);
    }
    assert_eq!(by_id("cost-tick").text_content().as_deref(), Some("200"));
    assert_eq!(
        (js("window.__ae"), js("window.__ct")),
        (0.0, 0.0),
        "a text node holds no focused child"
    );
    // And a structural verb does ask, so the flag is not simply stuck off.
    rows.set(vec![1]);
    assert!(js("window.__ae") >= 1.0, "positive control: it asks now");
    assert_eq!(active_id(), "cost-in", "and the field kept the focus");
    root.unmount();
    host.remove();
}
