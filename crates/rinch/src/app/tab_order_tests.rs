//! The Tab **sequence**: positive `tabindex` first, ascending, ties in tree
//! order, then everything at `0` in tree order (issue #435).
//!
//! Before this, `handle_tab` cycled the focusable set in plain DOM pre-order,
//! so `tabindex="3"` made a node focusable and ordered it nowhere. `rinch-web`
//! gets the browser's order for free outside a trap, so this was also a
//! desktop/web divergence — and inside a trap `rinch-web` cycles a list of its
//! own, which had the same flaw.
//!
//! Every expected sequence here was **measured in Chrome 153** on the same
//! markup (buttons and a `tabindex="0"` div, pressing Tab / Shift+Tab and
//! reading `document.activeElement.id`):
//!
//! | markup (tree order) | Tab from nothing |
//! |---|---|
//! | `A B[2] C[1] D[0] E[2] F[-1] G` | `C B E A D G`, then wraps to `C` |
//!
//! A start that is **not** in the sequence (a clicked `tabindex="-1"` node)
//! goes to the next Tab stop after it in **tree order** — any tabindex — and
//! Shift+Tab to the previous one; the sequence then continues from there:
//!
//! | markup | start | Tab | Shift+Tab |
//! |---|---|---|---|
//! | `A F[-1] B[2] C[1] D[0] E[2] G` | `F` | `B`, then `E A D` | `A`, then `E B C` |
//! | `A B[2] C[1] F[-1] D E[2] G` | `F` | `D`, then `G` | `C` |
//! | `B[2] A C[1] D F[-1]` (nothing after `F`) | `F` | `A` — the first stop at `0`, not the sequence's first | `D` |
//!
//! And an open modal `<dialog>` holding `X Y[1] Z` focuses `X` on open (tree
//! order: the focus delegate is not the sequence's first) and Tab cycles
//! `Z … Y X Z` — the sequence, confined.
//!
//! **Off the fixed point.** A document whose positive stops happen to sit first
//! in tree order, or whose values are distinct and ascending in tree order,
//! gives the same answer sorted and unsorted. The fixture here puts a `0` stop
//! first, a `2` before a `1`, and two `2`s apart with a `1` between them, so
//! only a stable sort by `(positive-first, value)` produces the measured order.
//!
//! Every geometry is declared, never measured.

use super::*;

const W: f32 = 800.0;
const H: f32 = 600.0;

fn key(app: &mut RinchApp, key: KeyCode, modifiers: Modifiers) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
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
            key,
            logical_key: None,
            modifiers,
        },
        (W as u32, H as u32),
        1.0,
    );
}

fn tab(app: &mut RinchApp) {
    key(app, KeyCode::Tab, Modifiers::default());
}

fn shift_tab(app: &mut RinchApp) {
    key(
        app,
        KeyCode::Tab,
        Modifiers {
            shift: true,
            ..Default::default()
        },
    );
}

fn focused(app: &RinchApp) -> Option<usize> {
    match app.focus_target {
        FocusTarget::Input(id) | FocusTarget::Node(id) => Some(id),
        _ => None,
    }
}

/// One stop per `(name, tag, tabindex)` under a root `<div>`, returned by name.
/// `tabindex: None` leaves the attribute off (a `<button>` is then focusable by
/// its tag, at `0`). `trap` wraps them all in a `data-trap-focus` box, after
/// `outside` stops that sit in the page before it.
fn mount_stops(
    outside: &'static [(&'static str, &'static str, Option<&'static str>)],
    stops: &'static [(&'static str, &'static str, Option<&'static str>)],
    trap: bool,
) -> (RinchApp, Rc<std::collections::HashMap<&'static str, usize>>) {
    let ids: Rc<RefCell<std::collections::HashMap<&'static str, usize>>> =
        Rc::new(RefCell::new(std::collections::HashMap::new()));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let make =
            |scope: &mut RenderScope,
             parent: &NodeHandle,
             (name, tag, tabindex): (&'static str, &'static str, Option<&str>)| {
                let n = scope.create_element(tag);
                n.set_attribute("style", "display: block; width: 120px; height: 20px");
                if let Some(t) = tabindex {
                    n.set_attribute("tabindex", t);
                }
                parent.append_child(&n);
                ids_in.borrow_mut().insert(name, n.node_id().0);
            };
        for &s in outside {
            make(scope, &root, s);
        }
        let host = if trap {
            let t = scope.create_element("div");
            t.set_attribute("data-trap-focus", "");
            t.set_attribute("style", "display: block; width: 200px");
            root.append_child(&t);
            t
        } else {
            root.clone()
        };
        for &s in stops {
            make(scope, &host, s);
        }
        root
    });
    app.mount_component(W, H);
    app.resolve_and_repaint(W, H);
    let map = ids.borrow().clone();
    (app, Rc::new(map))
}

/// Names the arbiter lands on, one per press.
fn tour(
    app: &mut RinchApp,
    ids: &std::collections::HashMap<&'static str, usize>,
    presses: usize,
    shift: bool,
) -> Vec<&'static str> {
    let by_id: std::collections::HashMap<usize, &'static str> =
        ids.iter().map(|(&k, &v)| (v, k)).collect();
    (0..presses)
        .map(|_| {
            if shift {
                shift_tab(app)
            } else {
                tab(app)
            }
            focused(app)
                .and_then(|id| by_id.get(&id).copied())
                .unwrap_or("?")
        })
        .collect()
}

const DOC: &[(&str, &str, Option<&str>)] = &[
    ("A", "button", None),
    ("B", "button", Some("2")),
    ("C", "button", Some("1")),
    ("D", "div", Some("0")),
    ("E", "button", Some("2")),
    ("F", "button", Some("-1")),
    ("G", "button", None),
];

/// **Mutant: no sort** (the pre-#435 walk handed straight to the cycle) —
/// `A B C D E G`. **Mutant: sort by value alone**, zeros included — the zeros
/// would sort *first* (`A D G C B E`). **Mutant: an unstable or
/// tree-order-reversed tie** — `E` before `B`.
#[test]
fn tab_walks_positive_tabindex_ascending_then_the_zeros_in_tree_order() {
    let (mut app, ids) = mount_stops(&[], DOC, false);
    assert_eq!(
        tour(&mut app, &ids, 7, false),
        ["C", "B", "E", "A", "D", "G", "C"],
        "Chrome 153's order for this markup, wrap included"
    );
}

#[test]
fn shift_tab_walks_the_same_sequence_backwards() {
    let (mut app, ids) = mount_stops(&[], DOC, false);
    assert_eq!(
        tour(&mut app, &ids, 7, true),
        ["G", "D", "A", "E", "B", "C", "G"],
    );
}

/// The collection itself stays in **tree order**: it is also what an opening
/// dialog's focus delegate is chosen from, and HTML chooses that in tree order.
#[test]
fn the_focusable_collection_itself_stays_in_tree_order() {
    let (app, ids) = mount_stops(&[], DOC, false);
    let names = ["A", "B", "C", "D", "E", "G"];
    let expected: Vec<usize> = names.iter().map(|n| ids[n]).collect();
    assert_eq!(app.collect_focusable_nodes(), expected);
}

/// A trap cycles its own stops in sequence order, and a positive stop
/// **outside** it (`P`, `tabindex="1"`) is neither entered nor allowed to
/// displace the trap's own first stop. Chrome's `showModal()` gives
/// `X Y[1] Z` the cycle `Y X Z`.
///
/// **Mutant: sorting only the whole-document list** (the `None` arm of
/// `tab_trap_root`) leaves this tree-ordered: `X Y Z`.
#[test]
fn a_trap_cycles_its_own_stops_in_sequence_order() {
    const OUTSIDE: &[(&str, &str, Option<&str>)] = &[("P", "button", Some("1"))];
    const INSIDE: &[(&str, &str, Option<&str>)] = &[
        ("X", "button", None),
        ("Y", "button", Some("1")),
        ("Z", "button", None),
    ];
    let (mut app, ids) = mount_stops(OUTSIDE, INSIDE, true);
    assert_eq!(tour(&mut app, &ids, 4, false), ["Y", "X", "Z", "Y"]);
    assert_eq!(tour(&mut app, &ids, 3, true), ["Z", "X", "Y"]);
}

/// An opening dialog's focus goes to its first stop in **tree order** (Chrome:
/// `X`), not the sequence's first (`Y`) — so `focus_into_subtree` must not be
/// handed the sorted list.
#[test]
fn focus_into_a_subtree_picks_tree_order_not_tab_order() {
    const INSIDE: &[(&str, &str, Option<&str>)] = &[
        ("X", "button", None),
        ("Y", "button", Some("1")),
        ("Z", "button", None),
    ];
    let (mut app, ids) = mount_stops(&[], INSIDE, true);
    let trap = {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        d.tree.get(ids["X"]).unwrap().parent.unwrap()
    };
    app.focus_into_subtree(trap, rinch_core::dom::FocusIntoPolicy::FirstFocusable);
    assert_eq!(focused(&app), Some(ids["X"]));
}

/// Start on a `tabindex="-1"` node focused by click or script: Tab goes to the
/// next stop **after it in tree order**, whatever that stop's tabindex, and the
/// sequence continues from there (Chrome: `F` → `B`, then `E A D`; Shift+Tab
/// `F` → `A`, then `E B C`).
///
/// **Mutant: the pre-#435 fallback** (a start not in the list enters at the
/// sequence's first / last) gives `C` forward and `G` backward.
#[test]
fn a_start_outside_the_sequence_resumes_at_its_tree_neighbour() {
    const MARKUP: &[(&str, &str, Option<&str>)] = &[
        ("A", "button", None),
        ("F", "button", Some("-1")),
        ("B", "button", Some("2")),
        ("C", "button", Some("1")),
        ("D", "div", Some("0")),
        ("E", "button", Some("2")),
        ("G", "button", None),
    ];
    let (mut app, ids) = mount_stops(&[], MARKUP, false);
    app.focus_element(ids["F"]);
    assert_eq!(focused(&app), Some(ids["F"]), "precondition: F holds focus");
    assert_eq!(tour(&mut app, &ids, 4, false), ["B", "E", "A", "D"]);

    app.focus_element(ids["F"]);
    assert_eq!(tour(&mut app, &ids, 4, true), ["A", "E", "B", "C"]);
}

/// Same start, later in the tree: Chrome goes `F` → `D` (the next stop after it)
/// and Shift+Tab `F` → `C` (the previous one), even though `C` is the
/// sequence's *first* and `D` sits in the middle of it.
#[test]
fn a_start_between_stops_takes_the_tree_neighbour_on_either_side() {
    const MARKUP: &[(&str, &str, Option<&str>)] = &[
        ("A", "button", None),
        ("B", "button", Some("2")),
        ("C", "button", Some("1")),
        ("F", "button", Some("-1")),
        ("D", "button", None),
        ("E", "button", Some("2")),
        ("G", "button", None),
    ];
    let (mut app, ids) = mount_stops(&[], MARKUP, false);
    app.focus_element(ids["F"]);
    assert_eq!(tour(&mut app, &ids, 2, false), ["D", "G"]);
    app.focus_element(ids["F"]);
    assert_eq!(tour(&mut app, &ids, 1, true), ["C"]);
}

/// Nothing after the start in tree order: Chrome's Tab goes to the first stop
/// **at `0`** (`A`), not the sequence's first (`C`); Shift+Tab takes the
/// previous stop in tree order (`D`).
#[test]
fn a_start_after_every_stop_resumes_at_the_first_zero() {
    const MARKUP: &[(&str, &str, Option<&str>)] = &[
        ("B", "button", Some("2")),
        ("A", "button", None),
        ("C", "button", Some("1")),
        ("D", "button", None),
        ("F", "button", Some("-1")),
    ];
    let (mut app, ids) = mount_stops(&[], MARKUP, false);
    app.focus_element(ids["F"]);
    assert_eq!(tour(&mut app, &ids, 2, false), ["A", "D"]);
    app.focus_element(ids["F"]);
    assert_eq!(tour(&mut app, &ids, 1, true), ["D"]);
}

/// `(name, tag, tabindex, parent)` — a stop whose `parent` names an earlier
/// entry is nested inside it; `None` puts it under the root.
type Nested = (
    &'static str,
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
);

fn mount_nested(
    markup: &'static [Nested],
) -> (RinchApp, std::collections::HashMap<&'static str, usize>) {
    let ids: Rc<RefCell<std::collections::HashMap<&'static str, usize>>> =
        Rc::new(RefCell::new(std::collections::HashMap::new()));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let mut handles: std::collections::HashMap<&'static str, NodeHandle> =
            std::collections::HashMap::new();
        for &(name, tag, tabindex, parent) in markup {
            let n = scope.create_element(tag);
            // `height: auto` on a container so its nested stops give it a box.
            n.set_attribute("style", "display: block; width: 120px; min-height: 20px");
            if let Some(t) = tabindex {
                n.set_attribute("tabindex", t);
            }
            parent.map_or(&root, |p| &handles[p]).append_child(&n);
            ids_in.borrow_mut().insert(name, n.node_id().0);
            handles.insert(name, n);
        }
        root
    });
    app.mount_component(W, H);
    app.resolve_and_repaint(W, H);
    let map = ids.borrow().clone();
    (app, map)
}

/// A `tabindex="-1"` start **inside** a Tab stop still resumes at its tree
/// neighbour — not at the enclosing stop's place in the sequence. Chrome 153:
/// `D[2]{F[-1]} A E[2]`, F focused — Tab → `A`, Shift+Tab → `D`.
///
/// **Mutant: the ancestor walk ahead of the tree-neighbour rule** (the first
/// cut of #435) anchors on `D` and Tab goes to `E`, `D`'s successor in the
/// sequence.
#[test]
fn a_negative_start_inside_a_stop_resumes_at_its_tree_neighbour() {
    const MARKUP: &[Nested] = &[
        ("D", "div", Some("2"), None),
        ("F", "div", Some("-1"), Some("D")),
        ("A", "button", None, None),
        ("E", "button", Some("2"), None),
    ];
    let (mut app, ids) = mount_nested(MARKUP);
    app.focus_element(ids["F"]);
    assert_eq!(focused(&app), Some(ids["F"]), "precondition: F holds focus");
    assert_eq!(tour(&mut app, &ids, 1, false), ["A"], "Tab from F");
    app.focus_element(ids["F"]);
    assert_eq!(tour(&mut app, &ids, 1, true), ["D"], "Shift+Tab from F");
}

/// The Shift+Tab half off the ancestor's own neighbour: Chrome 153,
/// `A D[0]{F[-1] X} Q`, F focused — Shift+Tab → `D` (the stop before F in tree
/// order is its own container), Tab → `X`. Anchoring on `D` gives `A` backwards.
#[test]
fn shift_tab_from_a_negative_start_inside_a_stop_reaches_the_stop() {
    const MARKUP: &[Nested] = &[
        ("A", "button", None, None),
        ("D", "div", Some("0"), None),
        ("F", "div", Some("-1"), Some("D")),
        ("X", "button", None, Some("D")),
        ("Q", "button", None, None),
    ];
    let (mut app, ids) = mount_nested(MARKUP);
    app.focus_element(ids["F"]);
    assert_eq!(tour(&mut app, &ids, 1, true), ["D"], "Shift+Tab from F");
    app.focus_element(ids["F"]);
    assert_eq!(tour(&mut app, &ids, 1, false), ["X"], "Tab from F");
}

/// The arbiter holds nothing, but the DOM's own focus sits on a
/// `tabindex="-1"` node: Tab starts from that node, not from the top.
///
/// **Mutant: the probe is the arbiter's claim only** (no `focused_node`
/// fallback) — Tab enters at the sequence's first (`C`), Shift+Tab at its last
/// (`G`).
#[test]
fn with_no_claim_tab_starts_from_the_doms_focused_node() {
    const MARKUP: &[(&str, &str, Option<&str>)] = &[
        ("A", "button", None),
        ("B", "button", Some("2")),
        ("C", "button", Some("1")),
        ("F", "button", Some("-1")),
        ("D", "button", None),
        ("E", "button", Some("2")),
        ("G", "button", None),
    ];
    let (mut app, ids) = mount_stops(&[], MARKUP, false);
    for (shift, expected) in [(false, "D"), (true, "C")] {
        app.set_focus_target(FocusTarget::None);
        app.doc.as_ref().unwrap().borrow_mut().tree.focused_node = Some(ids["F"]);
        assert_eq!(
            focused(&app),
            None,
            "precondition: the arbiter holds nothing"
        );
        assert_eq!(tour(&mut app, &ids, 1, shift), [expected], "shift: {shift}");
    }
}

/// `tabindex` is parsed with HTML's integer rules in a browser (Chrome 153:
/// `" 3"` → 3, `"2.5"` → 2, `"+2"` → 2), so `A S1[" 3"] S2["2.5"] S3["+2"]`
/// tours `S2 S3 S1 A` there. Desktop's `i32::from_str` rejects the first two.
#[test]
#[ignore = "#1138: desktop parses tabindex with i32::from_str, not HTML's rules"]
fn tabindex_parses_like_html() {
    const MARKUP: &[(&str, &str, Option<&str>)] = &[
        ("A", "button", None),
        ("S1", "div", Some(" 3")),
        ("S2", "div", Some("2.5")),
        ("S3", "div", Some("+2")),
    ];
    let (mut app, ids) = mount_stops(&[], MARKUP, false);
    assert_eq!(tour(&mut app, &ids, 4, false), ["S2", "S3", "S1", "A"]);
}
