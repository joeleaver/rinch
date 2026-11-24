//! Tab reaches **inline** focusables (issue #2963): an `<a href>` in a
//! paragraph, a `<span tabindex="0">`, a link nested in `<b>`, an empty link.
//!
//! An inline element in an inline formatting context owns no Taffy box — its
//! `layout` is a `0x0` ghost while its text is painted — and the Tab walk's
//! visibility test read that box, so Tab skipped every one of them.
//!
//! The expected order was **measured in Chrome 158** (headless, CDP
//! `Input.dispatchKeyEvent` Tab, reading `document.activeElement.id`) on the
//! same markup, `font: 16px/20px sans-serif`, in a `300px` box:
//!
//! ```html
//! <button id=A>A</button>
//! <p>text <a id=L1 href=/x>link</a> more <span id=S tabindex=0>span</span>
//!    and <a id=E href=/y></a> <b><a id=N href=/z>nested</a></b></p>
//! <div style="display:none"><a id=H href=/h>hidden</a></div>
//! <p><a id=V href=/v style="visibility:hidden">vis</a>
//!    <span style="visibility:hidden"><a id=VI href=/vi>vi</a></span></p>
//! <p style="width:80px"><a id=W href=/w>wrapping link text here</a></p>
//! <button id=B>B</button>
//! ```
//!
//! Tab from nothing: `A L1 S E N W B`, then (past the page) `A` again.
//! `E` is **empty** — Chrome's `getBoundingClientRect()` is `0x17` — and is
//! still a Tab stop: an inline element with a line fragment is rendered,
//! whatever its width. `W` wraps onto three lines (three client rects).
//! `H` (in a `display: none` subtree), `V` and `VI` (`visibility: hidden`,
//! own and inherited) are skipped.

use super::*;
use rinch_core::reactive::Signal;

const W: f32 = 800.0;
const H: f32 = 600.0;

fn tab(app: &mut RinchApp, shift: bool) {
    let modifiers = Modifiers {
        shift,
        ..Default::default()
    };
    app.handle_event(
        PlatformEvent::KeyDown {
            key: KeyCode::Tab,
            logical_key: None,
            text: None,
            modifiers,
            repeat: KeyRepeat::Unknown,
        },
        (W as u32, H as u32),
        1.0,
    );
    app.handle_event(
        PlatformEvent::KeyUp {
            key: KeyCode::Tab,
            logical_key: None,
            modifiers,
        },
        (W as u32, H as u32),
        1.0,
    );
}

fn focused(app: &RinchApp) -> Option<u64> {
    match app.focus_target {
        FocusTarget::Input(id) | FocusTarget::Node(id) => Some(id),
        _ => None,
    }
}

type Ids = std::collections::HashMap<&'static str, u64>;

fn mount() -> (RinchApp, Ids) {
    mount_with(Signal::new(false), Signal::new(false))
}

/// `hide_first` puts the first paragraph (`L1 S E N`) under `display: none`,
/// `hide_b` the inline `<b>` around `N`.
fn mount_with(hide_first: Signal<bool>, hide_b: Signal<bool>) -> (RinchApp, Ids) {
    let ids: Rc<RefCell<Ids>> = Rc::new(RefCell::new(Ids::new()));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let reg = |name: &'static str, n: &NodeHandle| {
            ids_in.borrow_mut().insert(name, n.node_id().0);
        };
        let text = |scope: &mut RenderScope, parent: &NodeHandle, s: &str| {
            parent.append_child(&scope.create_text(s));
        };
        let el = |scope: &mut RenderScope,
                  parent: &NodeHandle,
                  tag: &str,
                  attrs: &[(&str, &str)],
                  content: &str| {
            let n = scope.create_element(tag);
            for (k, v) in attrs {
                n.set_attribute(k, v);
            }
            if !content.is_empty() {
                n.append_child(&scope.create_text(content));
            }
            parent.append_child(&n);
            n
        };

        let root = scope.create_element("div");
        root.set_attribute("style", "width: 300px; font: 16px/20px sans-serif");

        let a = el(scope, &root, "button", &[], "A");
        reg("A", &a);

        let p = el(scope, &root, "p", &[], "");
        {
            let p = p.clone();
            scope.create_effect(move || {
                p.set_attribute(
                    "style",
                    if hide_first.get() {
                        "display: none"
                    } else {
                        ""
                    },
                );
            });
        }
        text(scope, &p, "text ");
        let l1 = el(scope, &p, "a", &[("href", "/x")], "link");
        reg("L1", &l1);
        text(scope, &p, " more ");
        let s = el(scope, &p, "span", &[("tabindex", "0")], "span");
        reg("S", &s);
        text(scope, &p, " and ");
        let e = el(scope, &p, "a", &[("href", "/y")], "");
        reg("E", &e);
        text(scope, &p, " ");
        let b = el(scope, &p, "b", &[], "");
        {
            let b = b.clone();
            scope.create_effect(move || {
                b.set_attribute("style", if hide_b.get() { "display: none" } else { "" });
            });
        }
        let n = el(scope, &b, "a", &[("href", "/z")], "nested");
        reg("N", &n);

        let none = el(scope, &root, "div", &[("style", "display: none")], "");
        let h = el(scope, &none, "a", &[("href", "/h")], "hidden");
        reg("H", &h);

        let p2 = el(scope, &root, "p", &[], "");
        let v = el(
            scope,
            &p2,
            "a",
            &[("href", "/v"), ("style", "visibility: hidden")],
            "vis",
        );
        reg("V", &v);
        text(scope, &p2, " ");
        let vs = el(scope, &p2, "span", &[("style", "visibility: hidden")], "");
        let vi = el(scope, &vs, "a", &[("href", "/vi")], "vi");
        reg("VI", &vi);

        let p3 = el(scope, &root, "p", &[("style", "width: 80px")], "");
        let w = el(
            scope,
            &p3,
            "a",
            &[("href", "/w")],
            "wrapping link text here",
        );
        reg("W", &w);

        let bb = el(scope, &root, "button", &[], "B");
        reg("B", &bb);
        root
    });
    app.mount_component(W, H);
    app.resolve_and_repaint(W, H);
    let map = ids.borrow().clone();
    (app, map)
}

fn tour(app: &mut RinchApp, ids: &Ids, presses: usize, shift: bool) -> Vec<&'static str> {
    let by_id: std::collections::HashMap<u64, &'static str> =
        ids.iter().map(|(&k, &v)| (v, k)).collect();
    (0..presses)
        .map(|_| {
            tab(app, shift);
            focused(app)
                .and_then(|id| by_id.get(&id).copied())
                .unwrap_or("?")
        })
        .collect()
}

/// The fixture is what the issue says it is: every inline focusable here owns
/// a `0x0` layout box. Without this, a fixture whose links had been given a
/// box some other way would pass with the bug in place.
#[test]
fn the_inline_focusables_have_no_layout_box() {
    let (app, ids) = mount();
    let doc = app.doc.as_ref().unwrap().borrow();
    for name in ["L1", "S", "E", "N", "W"] {
        let node = doc.tree.get(ids[name]).unwrap();
        assert!(
            node.is_flowed_inline_element(),
            "{name} should be a flowed inline element"
        );
        assert_eq!(
            (node.layout.width, node.layout.height),
            (0.0, 0.0),
            "{name}'s layout should be the 0x0 ghost"
        );
    }
}

/// **The bug:** at HEAD Tab went `A B A …`, skipping every inline stop.
/// **Mutant: visibility tested after the inline arm** (an inline element
/// counted as rendered before `visibility` is read) — stops on `V` and `VI`.
/// **Mutant: any `display: inline` element counts** (`ifc_root` not required)
/// — every test that hides a subtree goes red.
#[test]
fn tab_stops_on_inline_focusables_in_tree_order_as_chrome_does() {
    let (mut app, ids) = mount();
    assert_eq!(
        tour(&mut app, &ids, 8, false),
        ["A", "L1", "S", "E", "N", "W", "B", "A"],
        "Chrome 158's Tab order for this markup, wrap included"
    );
}

#[test]
fn shift_tab_walks_the_inline_stops_backwards() {
    let (mut app, ids) = mount();
    assert_eq!(
        tour(&mut app, &ids, 8, true),
        ["B", "W", "N", "E", "S", "L1", "A", "B"],
    );
}

/// The collection a dialog's focus delegate is chosen from sees them too.
#[test]
fn the_focusable_collection_holds_the_inline_stops() {
    let (app, ids) = mount();
    let names = ["A", "L1", "S", "E", "N", "W", "B"];
    let want: Vec<u64> = names.iter().map(|n| ids[n]).collect();
    assert_eq!(app.collect_focusable_nodes(), want);
}

/// A link that stops being rendered stops being a Tab stop: the fragments are
/// read from the layout as it is now, not from lines shaped before the
/// subtree was hidden.
#[test]
fn a_link_hidden_after_layout_leaves_the_tab_order() {
    let hide = Signal::new(false);
    let (mut app, ids) = mount_with(hide, Signal::new(false));
    assert_eq!(app.collect_focusable_nodes().len(), 7, "positive control");
    hide.set(true);
    // A new viewport too, so the resolve cannot early-return on a clean tree.
    app.resolve_and_repaint(W - 10.0, H);
    let names = ["A", "W", "B"];
    let want: Vec<u64> = names.iter().map(|n| ids[n]).collect();
    assert_eq!(app.collect_focusable_nodes(), want);
}

/// The same for a `display: none` written on an **inline** ancestor after
/// layout: the paragraph's lines are rebuilt without `N`, so `N` has no
/// fragment, though its own computed `display` was never touched.
#[test]
fn a_link_under_an_inline_ancestor_hidden_after_layout_leaves_the_tab_order() {
    let hide_b = Signal::new(false);
    let (mut app, ids) = mount_with(Signal::new(false), hide_b);
    assert_eq!(app.collect_focusable_nodes().len(), 7, "positive control");
    hide_b.set(true);
    app.resolve_and_repaint(W - 10.0, H);
    let names = ["A", "L1", "S", "E", "W", "B"];
    let want: Vec<u64> = names.iter().map(|n| ids[n]).collect();
    assert_eq!(app.collect_focusable_nodes(), want);
}
