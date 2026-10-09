//! A node moved inside the document keeps its state where the browser has
//! `moveBefore` (issue #1483).
//!
//! `insertBefore` / `appendChild` of a node that is already in the document is
//! a removal and an insertion: the browser drops the focus inside it (firing
//! `blur` / `focusout` inside the call), reloads an `<iframe>` and resets a
//! scroll position. A keyed `for` reorder is exactly that call, so typing in a
//! list that re-sorted under the user lost the keyboard on the web alone;
//! desktop's focus is the runtime's own state and a move never touched it.
//!
//! `Node.moveBefore` is the DOM's state-preserving move. Measured in plain
//! Chrome 153, on a row holding the focused `<input>`:
//!
//! | call | `activeElement` after | focus events inside the call |
//! |---|---|---|
//! | `insertBefore(row, ref)` / `appendChild(row)` | `BODY` | `blur`, `focusout` |
//! | `moveBefore(row, ref)` / `moveBefore(row, null)` | the same input | none |
//!
//! `WebDocument`'s insertion verbs use it when the method exists and the child
//! and the parent are both connected, in one document; the `NodeHandle` verbs
//! then ask for no focus release (`DomDocument::moves_keeping_focus`). Anything
//! else — a browser without the method, a move into or out of a detached
//! subtree — takes the #1478 path: blur first, then `insertBefore`.
//!
//! **Engines.** Each fixture branches on the *feature*, read from the page
//! (`Element.prototype.moveBefore`), never on an engine name. The "kept"
//! answers are Chrome 153's, measured here; the method is specified to keep
//! the focus, so an engine that has it is held to the same answers, and one
//! that lacks it is held to #1478's (the focus goes, and its listeners hear
//! it). Firefox 157 (CI) has the method and gives Chrome's answers for the
//! focus, the events, the selection and the `<iframe>`; it does **not** keep a
//! moved scroller's offset, which the scroll fixture states.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test move_keeps_focus_1483
//! ```
#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch::components::{Modal, Select, SelectOption};
use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use rinch_core::for_each_dom_typed;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_test::*;

#[path = "support/engine.rs"]
mod engine;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn host() -> web_sys::Element {
    // A test that left focus on a control of its own would hand it to the next.
    if let Some(active) = document()
        .active_element()
        .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
    {
        active.blur().ok();
    }
    install_counters();
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    host
}

/// Wraps the three DOM calls that can move a node, once per page and before
/// the first mount (the product looks `moveBefore` up once, at its first
/// insertion): `__mb` counts `moveBefore` calls, `__mbThrew` the ones that
/// threw, and `__oldMoves` the `insertBefore` / `appendChild` calls whose
/// child was connected, which is a move made the state-dropping way.
fn install_counters() {
    js_sys::eval(
        "(function () {
            if (window.__mb !== undefined) return;
            window.__mb = 0; window.__mbThrew = 0; window.__oldMoves = 0;
            const mb = Element.prototype.moveBefore;
            if (typeof mb === 'function') {
                Element.prototype.moveBefore = function (n, r) {
                    window.__mb++;
                    try { return mb.call(this, n, r); }
                    catch (e) { window.__mbThrew++; throw e; }
                };
            }
            const ib = Node.prototype.insertBefore;
            Node.prototype.insertBefore = function (n, r) {
                if (n && n.isConnected) window.__oldMoves++;
                return ib.call(this, n, r);
            };
            const ac = Node.prototype.appendChild;
            Node.prototype.appendChild = function (n) {
                if (n && n.isConnected) window.__oldMoves++;
                return ac.call(this, n);
            };
        })()",
    )
    .unwrap();
}

fn counter(name: &str) -> u32 {
    js_sys::eval(&format!("window.{name}"))
        .unwrap()
        .as_f64()
        .unwrap() as u32
}

fn reset_counters() {
    js_sys::eval("window.__mb = 0; window.__mbThrew = 0; window.__oldMoves = 0;").unwrap();
}

/// The focused element's id, or its tag name when it has none (`BODY`).
fn active() -> String {
    document()
        .active_element()
        .map(|el| {
            if el.id().is_empty() {
                el.tag_name()
            } else {
                el.id()
            }
        })
        .unwrap_or_default()
}

fn by_id(id: &str) -> web_sys::HtmlElement {
    document()
        .get_element_by_id(id)
        .unwrap_or_else(|| panic!("no #{id}"))
        .dyn_into()
        .unwrap()
}

/// Whether this browser has the state-preserving move.
fn has_move_before() -> bool {
    js_sys::eval("typeof Element.prototype.moveBefore === 'function'")
        .unwrap()
        .as_bool()
        .unwrap()
}

/// Counts `blur` and `focusout` reaching the document (capture), until dropped.
struct FocusEvents {
    n: Rc<Cell<u32>>,
    cb: Closure<dyn FnMut(web_sys::Event)>,
}

impl FocusEvents {
    fn watch() -> Self {
        let n = Rc::new(Cell::new(0));
        let c = n.clone();
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| c.set(c.get() + 1));
        for name in ["blur", "focusout"] {
            document()
                .add_event_listener_with_callback_and_bool(name, cb.as_ref().unchecked_ref(), true)
                .unwrap();
        }
        Self { n, cb }
    }

    fn count(&self) -> u32 {
        self.n.get()
    }
}

impl Drop for FocusEvents {
    fn drop(&mut self) {
        for name in ["blur", "focusout"] {
            document()
                .remove_event_listener_with_callback_and_bool(
                    name,
                    self.cb.as_ref().unchecked_ref(),
                    true,
                )
                .unwrap();
        }
    }
}

fn keydown(el: &web_sys::Element, key: &str) {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(key);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    el.dispatch_event(&ev).unwrap();
}

/// The order of the `[data-row]` elements under `#list_id`.
fn order(list_id: &str) -> String {
    let rows = by_id(list_id).query_selector_all("[data-row]").unwrap();
    (0..rows.length())
        .map(|i| {
            rows.item(i)
                .unwrap()
                .unchecked_into::<web_sys::Element>()
                .get_attribute("data-row")
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// A keyed `for` reorder, `[1,2,3] → [3,2,1]`, with each row's `<input>`
/// focused in turn. The reconcile leaves one row where it is and moves the
/// other two, so at least two of the three runs move the focused row.
///
/// Red before the fix (Chrome 153): rows 2 and 3 end at `BODY`.
#[wasm_bindgen_test]
fn a_keyed_for_reorder_keeps_the_focused_row_focused() {
    let host = host();
    let rows = Signal::new(vec![1u32, 2, 3]);
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |__scope: &mut RenderScope| {
            rsx! {
                ul { id: "kf-list",
                    for n in rows.get() {
                        li { key: n, data-row: {n.to_string()},
                            input { id: {format!("kf-in-{n}")} }
                        }
                    }
                }
            }
        },
    );
    assert_eq!(order("kf-list"), "1,2,3", "positive control: rendered");
    let mut lost = 0;
    for k in 1..=3u32 {
        rows.set(vec![1, 2, 3]);
        assert_eq!(order("kf-list"), "1,2,3", "positive control: reset");
        let id = format!("kf-in-{k}");
        let input: web_sys::HtmlInputElement = by_id(&id).dyn_into().unwrap();
        input.focus().unwrap();
        assert_eq!(active(), id, "positive control: row {k} focused");
        // A caret that is not at an end, to see the field's own state survive.
        input.set_value("abcdef");
        input.set_selection_range(2, 4).unwrap();
        let events = FocusEvents::watch();
        rows.set(vec![3, 2, 1]);
        assert_eq!(order("kf-list"), "3,2,1", "positive control: reversed");
        if has_move_before() {
            assert_eq!(active(), id, "row {k}: the moved row kept the keyboard");
            assert_eq!(events.count(), 0, "row {k}: and no focus event fired");
            assert_eq!(
                (
                    input.selection_start().unwrap(),
                    input.selection_end().unwrap()
                ),
                (Some(2), Some(4)),
                "row {k}: with its selection"
            );
        } else if active() != id {
            lost += 1;
        }
    }
    if !has_move_before() {
        // Without the method the browser drops the focus of a row it moves.
        assert!(lost >= 2, "no moveBefore: the moved rows lost the focus");
    }
    root.unmount();
    host.remove();
}

/// `page > [wrap > button, other]`, the button focused and owning a live key
/// entry. `left` counts the entry's `on_focus_leave`.
struct Verbs {
    root: rinch_web::RootHandle,
    host: web_sys::Element,
    page: NodeHandle,
    wrap: NodeHandle,
    other: NodeHandle,
    /// A `<div>` of the same document that is attached to nothing.
    detached: NodeHandle,
    left: Rc<Cell<u32>>,
    n: &'static str,
    _keep: rinch_core::DismissHandle,
}

fn verbs(n: &'static str) -> Verbs {
    let host = host();
    let left = Rc::new(Cell::new(0));
    type Slots = (
        NodeHandle,
        NodeHandle,
        NodeHandle,
        NodeHandle,
        rinch_core::DismissHandle,
    );
    let slots: Rc<RefCell<Option<Slots>>> = Default::default();
    let (l, sl) = (left.clone(), slots.clone());
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |s| {
        let page = s.create_element("div");
        let wrap = s.create_element("div");
        wrap.set_attribute("id", &format!("wrap-{n}"));
        let trigger = s.create_element("button");
        trigger.set_attribute("id", &format!("trigger-{n}"));
        wrap.append_child(&trigger);
        page.append_child(&wrap);
        let other = s.create_element("button");
        other.set_attribute("id", &format!("other-{n}"));
        page.append_child(&other);
        let l = l.clone();
        let keep = rinch_core::push_key_handler(&trigger, |_| false, move || l.set(l.get() + 1));
        let detached = s.create_element("div");
        *sl.borrow_mut() = Some((page.clone(), wrap, other, detached, keep));
        page
    });
    let (page, wrap, other, detached, keep) = slots.borrow_mut().take().unwrap();
    by_id(&format!("trigger-{n}")).focus().unwrap();
    assert_eq!(
        active(),
        format!("trigger-{n}"),
        "positive control: focused"
    );
    Verbs {
        root,
        host,
        page,
        wrap,
        other,
        detached,
        left,
        n,
        _keep: keep,
    }
}

impl Verbs {
    /// After a move inside the document: kept with `moveBefore`, and otherwise
    /// gone with the entry told once (#1478).
    fn assert_after_move(&self, moves: u32) {
        let trigger = format!("trigger-{}", self.n);
        if has_move_before() {
            assert_eq!(active(), trigger, "the moved owner kept the focus");
            assert_eq!(self.left.get(), 0, "and its key entry heard no leave");
        } else {
            assert_ne!(active(), trigger, "no moveBefore: the focus went");
            assert_eq!(self.left.get(), moves, "and the entry heard it");
        }
        // The document is free and nothing is left half done.
        self.other.set_attribute("data-after", "1");
        assert_eq!(
            by_id(&format!("other-{}", self.n))
                .get_attribute("data-after")
                .as_deref(),
            Some("1")
        );
    }

    fn done(self) {
        self.root.unmount();
        self.host.remove();
    }
}

/// The three insertion verbs, each moving the focused owner's parent.
#[wasm_bindgen_test]
fn each_insertion_verb_moves_the_focused_owner_with_its_focus() {
    // `append_child`: [wrap, other] -> [other, wrap].
    let f = verbs("ap");
    f.page.append_child(&f.wrap);
    assert_eq!(
        by_id("other-ap").next_element_sibling().unwrap().id(),
        "wrap-ap",
        "it moved to the end"
    );
    f.assert_after_move(1);
    f.done();

    // `insert_after`, then `insert_before` back.
    let f = verbs("ia");
    f.other.insert_after(&f.wrap);
    assert_eq!(
        by_id("other-ia").next_element_sibling().unwrap().id(),
        "wrap-ia"
    );
    f.assert_after_move(1);
    by_id("trigger-ia").focus().unwrap();
    f.page.insert_before(&f.wrap, &f.other);
    assert_eq!(
        by_id("wrap-ia").next_element_sibling().unwrap().id(),
        "other-ia"
    );
    f.assert_after_move(2);
    f.done();
}

/// A move the state-preserving call does not cover: out of the document, into
/// a parent that is in no document. The focus goes as before, with the entry
/// told outside the borrow (#1478), whatever the browser has.
#[wasm_bindgen_test]
fn a_move_out_of_the_document_still_releases_the_focus_first() {
    let f = verbs("out");
    f.detached.append_child(&f.wrap);
    assert!(!by_id_opt("trigger-out"), "it left the document");
    assert_ne!(active(), "trigger-out");
    assert_eq!(f.left.get(), 1, "the entry heard the focus leave, once");
    // And back in: an insertion of a node that is in no document, not a move.
    f.page.append_child(&f.wrap);
    assert!(by_id_opt("trigger-out"), "it is back");
    assert_eq!(f.left.get(), 1);
    f.done();
}

fn by_id_opt(id: &str) -> bool {
    document().get_element_by_id(id).is_some()
}

/// A text node and a comment are moved by the same call (`moveBefore` takes
/// an element or character data), and end where they were sent.
#[wasm_bindgen_test]
fn text_and_comment_nodes_move_too() {
    let host = host();
    type Slots = (NodeHandle, NodeHandle, NodeHandle, NodeHandle);
    let slots: Rc<RefCell<Option<Slots>>> = Default::default();
    let sl = slots.clone();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |s| {
        let p = s.create_element("p");
        p.set_attribute("id", "tc-p");
        let a = s.create_text("a");
        let c = s.create_comment("c");
        let b = s.create_element("b");
        let t = s.create_text("B");
        b.append_child(&t);
        p.append_child(&a);
        p.append_child(&c);
        p.append_child(&b);
        *sl.borrow_mut() = Some((p.clone(), a, c, b));
        p
    });
    let (p, a, c, b) = slots.borrow_mut().take().unwrap();
    let html = || by_id("tc-p").inner_html();
    assert_eq!(html(), "a<!--c--><b>B</b>", "positive control");
    p.append_child(&a);
    assert_eq!(html(), "<!--c--><b>B</b>a");
    p.insert_before(&c, &a);
    assert_eq!(html(), "<b>B</b><!--c-->a");
    b.insert_after(&a);
    assert_eq!(html(), "<b>B</b>a<!--c-->");
    root.unmount();
    host.remove();
}

/// An open `Select` in a row a keyed reorder moves: its trigger holds the
/// focus and its list's key entry is live. With `moveBefore` the list stays
/// open and still takes the keyboard; without it the focus leaves the trigger
/// and the list closes (the #1478 behaviour).
#[wasm_bindgen_test]
fn an_open_select_in_a_moved_row_stays_open_and_keeps_its_keys() {
    let host = host();
    let rows = Signal::new(vec![1u32, 2, 3]);
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |s| {
        let list = s.create_element("div");
        list.set_attribute("id", "sel-list");
        for_each_dom_typed(
            s,
            &list,
            move || rows.get(),
            |n: &u32| n.to_string(),
            move |n: u32, s: &mut RenderScope| {
                let row = s.create_element("section");
                row.set_attribute("data-row", &n.to_string());
                let sel = Select {
                    value: "a".into(),
                    data: vec![
                        SelectOption::new("a", "A"),
                        SelectOption::new("b", "B"),
                        SelectOption::new("c", "C"),
                    ],
                    ..Default::default()
                }
                .render(s, &[]);
                row.append_child(&sel);
                row
            },
        );
        list
    });
    assert_eq!(order("sel-list"), "1,2,3", "positive control: rendered");
    // Row 3 is moved to the front by the reversal.
    let trigger: web_sys::HtmlElement = by_id("sel-list")
        .query_selector("[data-row='3'] .rinch-select__input")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    trigger.focus().unwrap();
    keydown(&trigger, "Enter");
    assert_eq!(
        trigger.get_attribute("aria-expanded").as_deref(),
        Some("true"),
        "positive control: the key reached rinch and opened the list"
    );
    let highlighted = || trigger.get_attribute("aria-activedescendant");
    keydown(&trigger, "ArrowDown");
    let before = highlighted();
    assert!(before.is_some(), "positive control: a row is highlighted");

    let moved_before = trigger.parent_element().map(|_| order("sel-list"));
    rows.set(vec![3, 1, 2]);
    assert_eq!(order("sel-list"), "3,1,2", "positive control: reordered");
    assert_ne!(moved_before.as_deref(), Some("3,1,2"));
    assert!(trigger.is_connected());

    let on_trigger = document()
        .active_element()
        .is_some_and(|el| el.is_same_node(Some(trigger.unchecked_ref())));
    if has_move_before() {
        assert!(on_trigger, "the trigger kept the focus");
        assert_eq!(
            trigger.get_attribute("aria-expanded").as_deref(),
            Some("true"),
            "so the list stayed open"
        );
        keydown(&trigger, "ArrowDown");
        assert_ne!(
            highlighted(),
            before,
            "and its key entry still takes the keyboard"
        );
        // Escape goes to the dismiss stack's entry for the list.
        keydown(&trigger, "Escape");
        assert_eq!(
            trigger.get_attribute("aria-expanded").as_deref(),
            Some("false"),
            "Escape still closes it"
        );
    } else {
        assert!(!on_trigger, "no moveBefore: the focus left the trigger");
        assert_eq!(
            trigger.get_attribute("aria-expanded").as_deref(),
            Some("false"),
            "and the list closed"
        );
    }
    root.unmount();
    host.remove();
}

/// An editor in a row a keyed reorder moves keeps the keyboard. Its capture
/// `<textarea>` is on `<body>`, outside the row, so this held before the fix
/// too (a control: it must not regress).
#[wasm_bindgen_test]
fn an_editor_in_a_moved_row_keeps_the_keyboard() {
    let host = host();
    let rows = Signal::new(vec![1u32, 2, 3]);
    let handles: Rc<RefCell<Vec<rinch_web::EditorHandle>>> = Default::default();
    let hs = handles.clone();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |s| {
        let list = s.create_element("div");
        list.set_attribute("id", "ed-list");
        let hs = hs.clone();
        for_each_dom_typed(
            s,
            &list,
            move || rows.get(),
            |n: &u32| n.to_string(),
            move |n: u32, s: &mut RenderScope| {
                let row = s.create_element("section");
                row.set_attribute("data-row", &n.to_string());
                let h = rinch_web::create_editor();
                hs.borrow_mut().push(h.clone());
                let ed = rinch_web::Editor {
                    editor: Some(h),
                    content: format!("<p>row {n}</p>"),
                    ..Default::default()
                };
                let node = ed.render(s, &[]);
                row.append_child(&node);
                row
            },
        );
        list
    });
    assert_eq!(order("ed-list"), "1,2,3", "positive control: rendered");
    for k in 0..3usize {
        rows.set(vec![1, 2, 3]);
        assert_eq!(order("ed-list"), "1,2,3", "positive control: reset");
        let h = handles.borrow()[k].clone();
        h.focus();
        assert_eq!(active(), "TEXTAREA", "positive control: editor focused");
        let events = FocusEvents::watch();
        rows.set(vec![3, 2, 1]);
        assert_eq!(order("ed-list"), "3,2,1", "positive control: reversed");
        assert_eq!(
            active(),
            "TEXTAREA",
            "editor {k}: the capture field kept the focus"
        );
        assert_eq!(events.count(), 0, "editor {k}: no focus event");
        assert!(h.insert_text("Z"), "editor {k}: and typing still lands");
        let typed = by_id("ed-list").text_content().unwrap_or_default();
        assert_eq!(
            typed.matches('Z').count(),
            k + 1,
            "editor {k}: in its document"
        );
    }
    root.unmount();
    host.remove();
}

/// A `Modal` that lives in a row: opened from a button in another row, it
/// takes the focus inside (#695). Reordering the rows while it is open moves
/// the modal's row (focus inside it) and the opener's row; closing then hands
/// the focus back to the opener.
#[wasm_bindgen_test]
fn an_open_modal_in_a_moved_row_keeps_focus_and_restores_it_on_close() {
    let host = host();
    let rows = Signal::new(vec![1u32, 2, 3]);
    let open = Signal::new(false);
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |s| {
        let list = s.create_element("div");
        list.set_attribute("id", "mo-list");
        for_each_dom_typed(
            s,
            &list,
            move || rows.get(),
            |n: &u32| n.to_string(),
            move |n: u32, s: &mut RenderScope| {
                let row = s.create_element("section");
                row.set_attribute("data-row", &n.to_string());
                let b = s.create_element("button");
                b.set_attribute("id", &format!("mo-btn-{n}"));
                b.set_attribute("style", "display: block; width: 120px; height: 28px");
                row.append_child(&b);
                if n == 3 {
                    let inside = s.create_element("input");
                    inside.set_attribute("id", "mo-inside");
                    let modal = Modal {
                        opened_fn: Some(Rc::new(move || open.get())),
                        trap_focus: true,
                        with_close_button: false,
                        ..Default::default()
                    }
                    .render(s, &[inside]);
                    row.append_child(&modal);
                }
                row
            },
        );
        list
    });
    assert_eq!(order("mo-list"), "1,2,3", "positive control: rendered");
    by_id("mo-btn-2").focus().unwrap();
    assert_eq!(
        active(),
        "mo-btn-2",
        "positive control: the opener is focused"
    );
    open.set(true);
    assert_eq!(
        active(),
        "mo-inside",
        "positive control: the modal took the focus"
    );

    // [1,2,3] -> [3,2,1]: the reconcile keeps one row and moves two, so the
    // modal's row or the opener's (or both) move; do it both ways round.
    for target in [vec![3u32, 2, 1], vec![2, 3, 1], vec![1, 2, 3]] {
        if active() != "mo-inside" {
            by_id("mo-inside").focus().unwrap();
        }
        let want = target
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",");
        rows.set(target);
        assert_eq!(order("mo-list"), want, "positive control: reordered");
        if has_move_before() {
            assert_eq!(active(), "mo-inside", "{want}: focus stayed in the modal");
        }
    }
    by_id("mo-inside").focus().unwrap();
    open.set(false);
    assert_eq!(
        active(),
        "mo-btn-2",
        "closing gives the focus back to the opener, wherever its row is now"
    );
    root.unmount();
    host.remove();
}

/// What else the state-preserving move keeps: a same-origin `<iframe>`'s
/// window (`insertBefore` unloads it and loads a new one) and a scroller's
/// offset.
///
/// Measured in Chrome 153, plain page: after `insertBefore` of the row the
/// frame's window is a new one (a mark on the old window is gone) and
/// `scrollTop` is 0; after `moveBefore` the mark is still there and
/// `scrollTop` is unchanged. Firefox 157 (CI) keeps the frame's window and
/// resets `scrollTop` to 0, so the offset is asserted outside Gecko only and
/// Gecko's is left unpinned (it is the browser's to change).
#[wasm_bindgen_test]
fn a_moved_row_keeps_its_iframe_and_its_scroll_offset() {
    let host = host();
    type Slots = (NodeHandle, NodeHandle, NodeHandle);
    let slots: Rc<RefCell<Option<Slots>>> = Default::default();
    let sl = slots.clone();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |s| {
        let page = s.create_element("div");
        let row = s.create_element("div");
        let frame = s.create_element("iframe");
        frame.set_attribute("id", "st-frame");
        row.append_child(&frame);
        let scroller = s.create_element("div");
        scroller.set_attribute("id", "st-scroll");
        scroller.set_attribute("style", "height: 40px; overflow: auto");
        let tall = s.create_element("div");
        tall.set_attribute("style", "height: 400px");
        scroller.append_child(&tall);
        row.append_child(&scroller);
        page.append_child(&row);
        let other = s.create_element("p");
        page.append_child(&other);
        *sl.borrow_mut() = Some((page.clone(), row, other));
        page
    });
    let (_page, row, other) = slots.borrow_mut().take().unwrap();
    let mark = || {
        js_sys::eval("document.getElementById('st-frame').contentWindow.__mark === 7")
            .unwrap()
            .as_bool()
            .unwrap()
    };
    js_sys::eval("document.getElementById('st-frame').contentWindow.__mark = 7").unwrap();
    by_id("st-scroll").set_scroll_top(120);
    assert!(mark(), "positive control: the frame's window is marked");
    assert_eq!(by_id("st-scroll").scroll_top(), 120, "positive control");

    other.insert_after(&row);
    assert_eq!(
        by_id("st-frame")
            .parent_element()
            .unwrap()
            .previous_element_sibling()
            .map(|el| el.tag_name()),
        Some("P".to_string()),
        "positive control: the row is after the paragraph"
    );
    if has_move_before() {
        assert!(mark(), "the iframe was not reloaded");
        if !engine::is_gecko() {
            assert_eq!(
                by_id("st-scroll").scroll_top(),
                120,
                "the scroll offset held"
            );
        }
    } else {
        assert!(!mark(), "no moveBefore: the iframe's window is a new one");
    }
    root.unmount();
    host.remove();
}

/// A mount inserts nodes that are in no document: it never tries `moveBefore`
/// (which would throw for them). A reorder moves each row it moves with
/// `moveBefore`, once, with no throw and no `insertBefore` / `appendChild` of
/// a connected node. Counted by wrapping the prototypes' methods.
#[wasm_bindgen_test]
fn only_a_move_of_a_connected_node_uses_move_before() {
    // The product's one look at the method (at the page's first insertion)
    // calls it once on two scratch nodes, to tell a polyfill apart. Have that
    // happen before the counters are read, whichever test runs first.
    let warm = host();
    rinch_web::mount_into(&warm, ThemeProviderProps::default(), |s| {
        let d = s.create_element("div");
        let c = s.create_element("span");
        d.append_child(&c);
        d
    })
    .unmount();
    warm.remove();
    let host = host();
    reset_counters();
    let rows = Signal::new(vec![1u32, 2, 3]);
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |__scope: &mut RenderScope| {
            rsx! {
                ul { id: "mb-list",
                    for n in rows.get() {
                        li { key: n, data-row: {n.to_string()}, {n.to_string()} }
                    }
                }
            }
        },
    );
    assert_eq!(order("mb-list"), "1,2,3", "positive control: mounted");
    assert_eq!(counter("__mb"), 0, "a mount moves nothing");
    reset_counters();
    rows.set(vec![3, 2, 1, 4]);
    assert_eq!(order("mb-list"), "3,2,1,4", "two rows moved, one added");
    if has_move_before() {
        assert_eq!(counter("__mb"), 2, "each moved row is one moveBefore");
        assert_eq!(counter("__mbThrew"), 0, "none of which threw");
        assert_eq!(counter("__oldMoves"), 0, "and nothing moved the old way");
    } else {
        assert_eq!(counter("__mb"), 0);
        assert_eq!(counter("__oldMoves"), 2, "no moveBefore: moved the old way");
    }
    root.unmount();
    host.remove();
}
