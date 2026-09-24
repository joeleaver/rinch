//! Named twin fixtures for the style-invalidation holes the review of #894
//! found (T1–T21, by the reviewer's numbering), and the cost pin for its
//! blocker (D1): an insertion restyles the siblings a structural selector
//! ties to it **once per frame**, not once per insertion.
//!
//! Each twin fixture compares an incrementally changed document with a fresh
//! one built in the final state (`style_twin`). The ones that failed on the
//! PR's first cut, with the defect each pins:
//!
//! | fixture | defect |
//! |---|---|
//! | T1, T1b, T17 | D2: a structural selector used only in a `::before`/`::after` rule set no selector flags (Breadcrumbs' `:not(:last-child)::after`) |
//! | T3, T18 | D4: a snapshot whose element lost its style to a resize in the same frame was skipped, leaving later siblings stale |
//! | T4 | D5: `set_inner_html(node, "")` noted no child-list change (`:empty`) |
//! | T9 | D3: `content: attr(x)` did not appear when `x` was *added* and no selector names `x` |
//! | r1 | round 2, S1: `set_text_content(el, "")` judged `:empty` on the children it was about to replace |
//! | r14, r14b | round 2, S3: a row inserted and displaced in one frame started a transition, though never rendered |
//!
//! The rest passed on the first cut and pin behaviour it got right (T6, T11,
//! T14 and T15 fail on the invalidation before it). T10 reports only. r15b pins
//! the ancestor-snapshot check in `resolve_inserted_subtree` (the reviewer's
//! mutant N9), which r15 can no longer see once S3 is fixed.
mod style_twin;

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use style_twin::{assert_twin, assert_twin_opts};

fn doc(css: &str) -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css(css);
    d
}
fn el(d: &mut RinchDocument, tag: &str, class: &str) -> NodeId {
    let n = d.create_element(tag);
    if !class.is_empty() {
        d.set_attribute(n, "class", class);
    }
    n
}
fn settle(d: &mut RinchDocument) {
    d.resolve_layout(800.0, 600.0);
    d.resolve_layout(800.0, 600.0);
}

/// T1: a structural selector used ONLY in a ::after rule.
#[test]
fn t1_last_child_only_in_pseudo_rule() {
    let css = "li:last-child::after { content: '!'; }";
    let mut a = doc(css);
    let body = a.body();
    let ul = el(&mut a, "ul", "");
    a.append_child(body, ul);
    for _ in 0..2 {
        let li = el(&mut a, "li", "");
        let t = a.create_text("x");
        a.append_child(li, t);
        a.append_child(ul, li);
    }
    settle(&mut a);
    let li = el(&mut a, "li", "");
    let t = a.create_text("x");
    a.append_child(li, t);
    a.append_child(ul, li);
    settle(&mut a);

    let mut b = doc(css);
    let body = b.body();
    let ul = el(&mut b, "ul", "");
    for _ in 0..3 {
        let li = el(&mut b, "li", "");
        let t = b.create_text("x");
        b.append_child(li, t);
        b.append_child(ul, li);
    }
    b.append_child(body, ul);
    settle(&mut b);
    assert_twin_opts(&a, &b, "t1", false);
}

/// T1b: Breadcrumbs' own rule shape.
#[test]
fn t1b_not_last_child_pseudo_rule_on_removal() {
    let css = ".i:not(:last-child)::after { content: '/'; }";
    let build = |n: usize| {
        let mut d = doc(css);
        let body = d.body();
        let ul = el(&mut d, "div", "");
        for _ in 0..n {
            let li = el(&mut d, "span", "i");
            let t = d.create_text("x");
            d.append_child(li, t);
            d.append_child(ul, li);
        }
        d.append_child(body, ul);
        settle(&mut d);
        (d, ul)
    };
    let (mut a, ul) = build(3);
    let last = *a.tree.nodes[ul.0].children.last().unwrap();
    a.remove_node(NodeId(last));
    settle(&mut a);
    let (b, _) = build(2);
    assert_twin_opts(&a, &b, "t1b", false);
}

fn row(css: &str, classes: &[&str]) -> (RinchDocument, Vec<NodeId>) {
    let mut d = doc(css);
    let body = d.body();
    let p = el(&mut d, "div", "");
    let mut v = vec![];
    for c in classes {
        let n = el(&mut d, "div", c);
        d.append_child(p, n);
        v.push(n);
    }
    d.append_child(body, p);
    settle(&mut d);
    (d, v)
}

/// T2: `:nth-last-child(… of S)` — membership change of a LATER sibling moves
/// an EARLIER sibling's index.
#[test]
fn t2_nth_last_child_of_membership() {
    let css = ":nth-last-child(1 of .b) { --r: 1; } :nth-last-child(2 of .b) { --q: 1; }";
    let (mut a, v) = row(css, &["b", "b", "b"]);
    a.set_attribute(v[2], "class", "x");
    settle(&mut a);
    let (b, _) = row(css, &["b", "b", "x"]);
    assert_twin(&a, &b, "t2");
}

/// T2b: `:nth-child(… of S)` — membership change of an earlier sibling.
#[test]
fn t2b_nth_child_of_membership() {
    let css = ":nth-child(2 of .b) { --r: 1; }";
    let (mut a, v) = row(css, &["b", "b", "b", "b"]);
    a.set_attribute(v[0], "class", "x");
    settle(&mut a);
    let (b, _) = row(css, &["x", "b", "b", "b"]);
    assert_twin(&a, &b, "t2b");
}

/// T3: a class change on a viewport-unit user and a resize in one frame.
#[test]
fn t3_viewport_resize_and_sibling_class_change_same_frame() {
    let css = ".u { width: 10vw; } .a + .b { --r: 1; color: rgb(200,0,0); }";
    let build = |first: &str, w: f32| {
        let mut d = doc(css);
        let body = d.body();
        let p = el(&mut d, "div", "");
        let a = el(&mut d, "div", first);
        let b = el(&mut d, "div", "b");
        d.append_child(p, a);
        d.append_child(p, b);
        d.append_child(body, p);
        d.resolve_layout(w, 600.0);
        d.resolve_layout(w, 600.0);
        (d, a)
    };
    let (mut a, first) = build("u a", 800.0);
    a.set_attribute(first, "class", "u");
    a.resolve_layout(900.0, 600.0);
    a.resolve_layout(900.0, 600.0);
    let (b, _) = build("u", 900.0);
    assert_twin(&a, &b, "t3");
}

/// T4: `set_inner_html("")` empties an element that `:empty` styles.
#[test]
fn t4_set_inner_html_to_empty() {
    let css = ".d:empty { padding-left: 3px; --r: 1; } .d:empty + .s { --q: 1; }";
    let build = |kids: bool| {
        let mut d = doc(css);
        let body = d.body();
        let p = el(&mut d, "div", "");
        let x = el(&mut d, "div", "d");
        if kids {
            let k = el(&mut d, "span", "");
            d.append_child(x, k);
        }
        let s = el(&mut d, "div", "s");
        d.append_child(p, x);
        d.append_child(p, s);
        d.append_child(body, p);
        settle(&mut d);
        (d, x)
    };
    let (mut a, x) = build(true);
    a.set_inner_html(x, "");
    settle(&mut a);
    let (b, _) = build(false);
    assert_twin(&a, &b, "t4");
}

/// T5: `rem` under an intermediate with a fixed font-size, root font-size
/// changed by a class on <html>.
#[test]
fn t5_rem_under_fixed_font_intermediate() {
    let css = "html.big { font-size: 24px; } .fx { font-size: 16px; } .box { width: 2rem; height: 1rem; }";
    let build = |big: bool| {
        let mut d = doc(css);
        let html = NodeId(d.tree.html_id);
        if big {
            d.set_attribute(html, "class", "big");
        }
        let body = d.body();
        let f = el(&mut d, "div", "fx");
        let bx = el(&mut d, "div", "box");
        d.append_child(f, bx);
        d.append_child(body, f);
        settle(&mut d);
        d
    };
    let mut a = build(false);
    let html = NodeId(a.tree.html_id);
    a.set_attribute(html, "class", "big");
    settle(&mut a);
    let b = build(true);
    assert_twin(&a, &b, "t5");
}

/// T6: bulk insertion under `suppress_inline_restyle` next to a sibling rule.
#[test]
fn t6_suppressed_insertion_sibling() {
    let css = ".a + .b { --r: 1; } .b:first-child { --f: 1; } :empty { --e: 1; }";
    let mut a = doc(css);
    let body = a.body();
    let p = el(&mut a, "div", "");
    let b0 = el(&mut a, "div", "b");
    a.append_child(p, b0);
    a.append_child(body, p);
    settle(&mut a);
    a.tree.suppress_inline_restyle = true;
    let n = el(&mut a, "div", "a");
    a.insert_before(p, n, b0);
    a.tree.suppress_inline_restyle = false;
    settle(&mut a);

    let mut b = doc(css);
    let body = b.body();
    let p = el(&mut b, "div", "");
    let n = el(&mut b, "div", "a");
    let b0 = el(&mut b, "div", "b");
    b.append_child(p, n);
    b.append_child(p, b0);
    b.append_child(body, p);
    settle(&mut b);
    assert_twin(&a, &b, "t6");
}

/// T7: grandparent display flips under a `display: contents` parent.
#[test]
fn t7_display_contents_blockification() {
    let css = ".g.flex { display: flex; } .c { display: contents; }";
    let build = |flex: bool| {
        let mut d = doc(css);
        let body = d.body();
        let g = el(&mut d, "div", if flex { "g flex" } else { "g" });
        let c = el(&mut d, "div", "c");
        let s = el(&mut d, "span", "");
        let t = d.create_text("x");
        d.append_child(s, t);
        d.append_child(c, s);
        d.append_child(g, c);
        d.append_child(body, g);
        settle(&mut d);
        (d, g)
    };
    let (mut a, g) = build(false);
    a.set_attribute(g, "class", "g flex");
    settle(&mut a);
    let (b, _) = build(true);
    assert_twin(&a, &b, "t7");
}

/// T8: root attribute theme switch `[data-theme=dark] .x`.
#[test]
fn t8_root_attribute_descendant() {
    let css = "[data-theme=dark] .x { color: rgb(250,250,250); --r: 1; } [data-theme=dark] { background-color: rgb(0,0,0); }";
    let build = |dark: bool| {
        let mut d = doc(css);
        let html = NodeId(d.tree.html_id);
        if dark {
            d.set_attribute(html, "data-theme", "dark");
        }
        let body = d.body();
        let w = el(&mut d, "div", "");
        let x = el(&mut d, "div", "x");
        d.append_child(w, x);
        d.append_child(body, w);
        settle(&mut d);
        d
    };
    let mut a = build(false);
    let html = NodeId(a.tree.html_id);
    a.set_attribute(html, "data-theme", "dark");
    settle(&mut a);
    let b = build(true);
    assert_twin(&a, &b, "t8");
}

/// T9: attr() with no attribute-selector rule, attribute ADDED.
#[test]
fn t9_attr_content_added_without_attr_selector() {
    let css = ".a::before { content: attr(data-x); }";
    let build = |x: Option<&str>| {
        let mut d = doc(css);
        let body = d.body();
        let n = el(&mut d, "div", "a");
        if let Some(x) = x {
            d.set_attribute(n, "data-x", x);
        }
        d.append_child(body, n);
        settle(&mut d);
        (d, n)
    };
    let (mut a, n) = build(None);
    a.set_attribute(n, "data-x", "hi");
    settle(&mut a);
    let (b, _) = build(Some("hi"));
    assert_twin(&a, &b, "t9");
}

/// T10: `[type="CHECKBOX"]` — Chrome matches (type is a case-insensitive
/// attribute in HTML). Report only.
#[test]
fn t10_type_attribute_case() {
    let mut d = doc("input[type=\"CHECKBOX\"] { --r: 1; } div[data-x=\"Y\"] { --q: 1; }");
    let body = d.body();
    let i = d.create_element("input");
    d.set_attribute(i, "type", "checkbox");
    let v = d.create_element("div");
    d.set_attribute(v, "data-x", "y");
    d.append_child(body, i);
    d.append_child(body, v);
    settle(&mut d);
    let f = |n: NodeId| {
        let data = d.tree.nodes[n.0].stylo_element_data.borrow();
        let cv = data.as_ref().unwrap().styles.primary.clone().unwrap();
        style_twin::stylo_fingerprint(&cv)
            .lines()
            .last()
            .unwrap()
            .to_string()
    };
    eprintln!("T10 input: {}", f(i));
    eprintln!("T10 div: {}", f(v));
}

/// T11: change then revert, plus a removal, in one frame.
#[test]
fn t11_change_revert_and_remove_same_frame() {
    let css = ".a + .b { --r: 1; } .a ~ .c { --s: 1; } .b:last-child { --l: 1; }";
    let (mut a, v) = row(css, &["a", "x", "b", "c"]);
    a.set_attribute(v[0], "class", "z");
    a.set_attribute(v[0], "class", "a");
    a.remove_node(v[1]);
    a.set_attribute(v[3], "class", "b");
    settle(&mut a);
    let (b, _) = row(css, &["a", "b", "b"]);
    assert_twin(&a, &b, "t11");
}

/// T12: an inherited custom property through an element that does not change.
#[test]
fn t12_custom_property_chain() {
    let css = ".p { --c: 1px; } .p.on { --c: 7px; } .k { padding-left: var(--c); }";
    let build = |on: bool| {
        let mut d = doc(css);
        let body = d.body();
        let p = el(&mut d, "div", if on { "p on" } else { "p" });
        let m = el(&mut d, "div", "m");
        let k = el(&mut d, "div", "k");
        d.append_child(m, k);
        d.append_child(p, m);
        d.append_child(body, p);
        settle(&mut d);
        (d, p)
    };
    let (mut a, p) = build(false);
    a.set_attribute(p, "class", "p on");
    settle(&mut a);
    let (b, _) = build(true);
    assert_twin(&a, &b, "t12");
}

/// T13: parent's non-inherited `width` change, child and grandchild
/// `width: inherit`.
#[test]
fn t13_inherit_reset_chain() {
    let css = ".p { width: 100px; } .p.w { width: 300px; } .i { width: inherit; }";
    let build = |w: bool| {
        let mut d = doc(css);
        let body = d.body();
        let p = el(&mut d, "div", if w { "p w" } else { "p" });
        let c = el(&mut d, "div", "i");
        let g = el(&mut d, "div", "i");
        d.append_child(c, g);
        d.append_child(p, c);
        d.append_child(body, p);
        settle(&mut d);
        (d, p)
    };
    let (mut a, p) = build(false);
    a.set_attribute(p, "class", "p w");
    settle(&mut a);
    let (b, _) = build(true);
    assert_twin(&a, &b, "t13");
}

/// T14: set_style of an inherited property (#508's shape).
#[test]
fn t14_inline_inherited_set_style() {
    let css = ".k { padding-left: 1em; }";
    let build = |big: bool| {
        let mut d = doc(css);
        let body = d.body();
        let p = el(&mut d, "div", "");
        if big {
            d.set_style(p, "font-size", "30px");
        }
        let m = el(&mut d, "div", "");
        let k = el(&mut d, "div", "k");
        let t = d.create_text("hello");
        d.append_child(k, t);
        d.append_child(m, k);
        d.append_child(p, m);
        d.append_child(body, p);
        settle(&mut d);
        (d, p)
    };
    let (mut a, p) = build(false);
    a.set_style(p, "font-size", "30px");
    settle(&mut a);
    let (b, _) = build(true);
    assert_twin(&a, &b, "t14");
}

/// T15: a snapshotted element re-inserted in the same frame (its stylo data
/// dropped before the invalidator runs) — sibling selector.
#[test]
fn t15_snapshot_then_reinserted() {
    let css = ".a + .b { --r: 1; }";
    let (mut a, v) = row(css, &["a", "b"]);
    a.set_attribute(v[0], "class", "x");
    let p = NodeId(a.tree.nodes[v[0].0].parent.unwrap());
    a.insert_before(p, v[0], v[1]);
    settle(&mut a);
    let (b, _) = row(css, &["x", "b"]);
    assert_twin(&a, &b, "t15");
}

/// T16: ol reversed attribute.
#[test]
fn t16_ol_reversed() {
    let build = |rev: bool| {
        let mut d = doc("");
        let body = d.body();
        let ol = d.create_element("ol");
        if rev {
            d.set_attribute(ol, "reversed", "");
        }
        for _ in 0..3 {
            let li = d.create_element("li");
            let t = d.create_text("x");
            d.append_child(li, t);
            d.append_child(ol, li);
        }
        d.append_child(body, ol);
        settle(&mut d);
        (d, ol)
    };
    let (mut a, ol) = build(false);
    a.set_attribute(ol, "reversed", "");
    settle(&mut a);
    let (b, _) = build(true);
    assert_twin(&a, &b, "t16");
}

/// T17: `:empty::before` — :empty used only in a pseudo rule; text added.
#[test]
fn t17_empty_only_in_pseudo_rule() {
    let css = ".d:empty::before { content: 'empty'; }";
    let build = |text: &str| {
        let mut d = doc(css);
        let body = d.body();
        let x = el(&mut d, "div", "d");
        let t = d.create_text(text);
        d.append_child(x, t);
        d.append_child(body, x);
        settle(&mut d);
        (d, t)
    };
    let (mut a, t) = build("");
    a.set_text_content(t, "hi");
    settle(&mut a);
    let (b, _) = build("hi");
    assert_twin_opts(&a, &b, "t17", false);
}

/// T18: resize reached only through a viewport-unit sibling, while another
/// sibling's class changed — the viewport-unit element is the sibling-rule
/// anchor. (T3 variant: the snapshot owner is NOT the viewport user.)
#[test]
fn t18_hover_then_resize() {
    let css = ".h:hover { width: 10vw; } .h:hover + .b { --r: 1; }";
    let build = |hover: bool, w: f32| {
        let mut d = doc(css);
        let body = d.body();
        let p = el(&mut d, "div", "");
        let a = el(&mut d, "div", "h");
        let b = el(&mut d, "div", "b");
        d.append_child(p, a);
        d.append_child(p, b);
        d.append_child(body, p);
        d.resolve_layout(w, 600.0);
        if hover {
            let mut c = false;
            d.update_hover(Some(a.0), &mut c);
        }
        d.resolve_layout(w, 600.0);
        d.resolve_layout(w, 600.0);
        (d, a)
    };
    let (mut a, _) = build(true, 800.0);
    let mut c = false;
    a.update_hover(None, &mut c);
    a.resolve_layout(900.0, 600.0);
    a.resolve_layout(900.0, 600.0);
    let (b, _) = build(false, 900.0);
    assert_twin(&a, &b, "t18");
}

/// T19: parent display flips to flex; its inline child must blockify.
#[test]
fn t19_parent_display_flex_blockifies_child() {
    let css = ".p.flex { display: flex; }";
    let build = |flex: bool| {
        let mut d = doc(css);
        let body = d.body();
        let p = el(&mut d, "div", if flex { "p flex" } else { "p" });
        let s = el(&mut d, "span", "");
        let t = d.create_text("x");
        d.append_child(s, t);
        d.append_child(p, s);
        d.append_child(body, p);
        settle(&mut d);
        (d, p)
    };
    let (mut a, p) = build(false);
    a.set_attribute(p, "class", "p flex");
    settle(&mut a);
    let (b, _) = build(true);
    assert_twin(&a, &b, "t19");
}

/// T20: an element inserted under `suppress_inline_restyle` (unstyled) has
/// its class changed before the resolve; a later sibling depends on it.
#[test]
fn t20_unstyled_class_change_reaches_sibling() {
    let css = ".a + .b { --r: 1; }";
    let mut a = doc(css);
    let body = a.body();
    let p = el(&mut a, "div", "");
    let x = el(&mut a, "div", "a");
    let b0 = el(&mut a, "div", "b");
    a.append_child(p, x);
    a.append_child(p, b0);
    a.append_child(body, p);
    settle(&mut a);
    // Re-insert x under suppression (unstyled), then change its class.
    a.tree.suppress_inline_restyle = true;
    a.insert_before(p, x, b0);
    a.tree.suppress_inline_restyle = false;
    a.set_attribute(x, "class", "z");
    settle(&mut a);
    let (b, _) = row(css, &["z", "b"]);
    assert_twin(&a, &b, "t20");
}

/// T21: prepend under a `:first-child`-only stylesheet. The displaced first
/// child is the one that must lose the rule — the element *after* the inserted
/// one (`note_child_list_changed`'s `after_next`). The rule is scoped to the
/// row and sets a non-inherited property: `:first-child { --f: 1 }` also
/// matched the row's own container, so the displaced child inherited `--f`
/// either way and the fixture could not see that branch dropped.
#[test]
fn t21_prepend_first_child() {
    let css = "div > div:first-child { padding-left: 3px; --f: 1; } \
               div > div { --f: 0; }";
    let (mut a, v) = row(css, &["x", "y"]);
    let p = NodeId(a.tree.nodes[v[0].0].parent.unwrap());
    let n = el(&mut a, "div", "w");
    a.insert_before(p, n, v[0]);
    settle(&mut a);
    let (b, _) = row(css, &["w", "x", "y"]);
    assert_twin(&a, &b, "t21");
}

/// T22: a generated `::before` box is not a child for selector matching.
/// rinch keeps it in the child list, and `:first-child` / `:nth-child` / `+`
/// used to count it — so in a fresh document the first real child was not
/// `:first-child`, and in an incremental one it was until something
/// restyled it (found by the random stylesheet-subset differential). Chrome:
/// `.x::before { content: "z" } .x > span:first-child` matches the span.
#[test]
fn t22_generated_content_is_not_a_sibling() {
    let mut d = doc(
        ".x::before { content: 'z'; } .x > span:first-child { --f: 1; } \
                     .x > span + span { --n: 1; } .x > span:nth-child(2) { --two: 1; }",
    );
    let body = d.body();
    let x = el(&mut d, "div", "x");
    let a = el(&mut d, "span", "");
    let b = el(&mut d, "span", "");
    d.append_child(x, a);
    d.append_child(x, b);
    d.append_child(body, x);
    settle(&mut d);
    let vars = |n: NodeId| {
        let data = d.tree.nodes[n.0].stylo_element_data.borrow();
        let cv = data.as_ref().unwrap().styles.primary.clone().unwrap();
        style_twin::stylo_fingerprint(&cv)
            .lines()
            .find(|l| l.starts_with("custom:"))
            .unwrap_or("")
            .to_string()
    };
    assert!(
        vars(a).contains("--f=1"),
        "the first span is :first-child: {}",
        vars(a)
    );
    assert!(
        vars(b).contains("--n=1"),
        "the second span follows the first: {}",
        vars(b)
    );
    assert!(
        vars(b).contains("--two=1"),
        "and is :nth-child(2): {}",
        vars(b)
    );
}

/// T23: the inset fast path (`set_styles` of `left`/`top`/… on an absolute
/// box) writes the inset straight to layout without a cascade, but a
/// `[style*=…]` selector still has to see the new attribute: the fast path
/// takes the invalidation snapshot too.
#[test]
fn t23_inset_fast_path_reaches_a_style_attribute_selector() {
    let mut d = doc(
        ".rel { position: relative; } .p { position: absolute; left: 0; top: 0; } \
                     .p[style*=\"40px\"] { --q: 1; }",
    );
    let body = d.body();
    let rel = el(&mut d, "div", "rel");
    let p = el(&mut d, "div", "p");
    d.append_child(rel, p);
    d.append_child(body, rel);
    settle(&mut d);
    d.set_style(p, "left", "40px");
    settle(&mut d);
    let data = d.tree.nodes[p.0].stylo_element_data.borrow();
    let cv = data.as_ref().unwrap().styles.primary.clone().unwrap();
    let fp = style_twin::stylo_fingerprint(&cv);
    assert!(
        fp.contains("--q=1"),
        "[style*=40px] must match after the write:\n{fp}"
    );
}

/// Build a mounted `<ul>` of one `li.r`, then insert `n` more one at a time
/// (prepending or appending) and lay out once. Answers the elements cascaded
/// across the inserts and that one layout.
fn cascades_for_inserts(css: &str, n: usize, prepend: bool) -> u64 {
    let mut d = doc(css);
    let body = d.body();
    let ul = el(&mut d, "ul", "");
    d.append_child(body, ul);
    let first = el(&mut d, "li", "r");
    d.append_child(ul, first);
    settle(&mut d);
    d.tree.perf.reset();
    for _ in 0..n {
        let li = el(&mut d, "li", "r");
        if prepend {
            let f = NodeId(d.tree.nodes[ul.0].children[0]);
            d.insert_before(ul, li, f);
        } else {
            d.append_child(ul, li);
        }
    }
    d.resolve_layout(800.0, 600.0);
    d.tree
        .perf
        .end_frame()
        .get(rinch_dom::perf::Counter::ElementsCascaded)
}

/// **D1.** N insertions into a mounted list whose rows a structural selector
/// ties together cost O(N) cascades — each new row once on insertion, each
/// marked sibling once at the frame's resolve — not O(N²). Every insertion
/// used to resolve all pending style work synchronously, which re-cascaded
/// every marked sibling on every insertion: 36 s (against 57 ms on main) for
/// 1000 appends under `li:last-of-type`.
///
/// The bound is 5N: each `<li>` carries a list-marker box, which the walk
/// cascades beside it (4N measured — two at insertion, two at the resolve).
#[test]
fn structural_inserts_cascade_linearly() {
    const N: u64 = 100;
    for (name, css, prepend) in [
        (
            "append, li:last-of-type",
            "li:last-of-type { --x: 1; }",
            false,
        ),
        (
            "append, li:nth-last-child(2)",
            "li:nth-last-child(2) { --x: 1; }",
            false,
        ),
        (
            "prepend, li:nth-child(odd)",
            "li:nth-child(odd) { --x: 1; }",
            true,
        ),
        ("prepend, .r + .r", ".r + .r { --x: 1; }", true),
        (
            "append, li:first-child",
            "li:first-child { --x: 1; }",
            false,
        ),
        ("prepend, li:last-child", "li:last-child { --x: 1; }", true),
        (
            "append, ul:empty",
            "ul:empty { --x: 1; } ul:empty + p { --y: 1; }",
            false,
        ),
        ("prepend, ul:empty", "ul:empty li { --x: 1; }", true),
    ] {
        let c = cascades_for_inserts(css, N as usize, prepend);
        assert!(
            c <= 5 * N,
            "{name}: {c} elements cascaded for {N} insertions — quadratic"
        );
        eprintln!("{name}: {c} elements cascaded for {N} insertions");
    }
}

fn bench_list(css: &str, n: usize, prepend: bool) -> std::time::Duration {
    let mut d = doc(css);
    let body = d.body();
    let ul = el(&mut d, "ul", "");
    d.append_child(body, ul);
    let first = el(&mut d, "li", "r");
    d.append_child(ul, first);
    settle(&mut d);
    let t = std::time::Instant::now();
    for _ in 0..n {
        let li = el(&mut d, "li", "r");
        let tx = d.create_text("row");
        d.append_child(li, tx);
        if prepend {
            let f = NodeId(d.tree.nodes[ul.0].children[0]);
            d.insert_before(ul, li, f);
        } else {
            d.append_child(ul, li);
        }
    }
    d.resolve_layout(800.0, 600.0);
    t.elapsed()
}

#[test]
#[ignore]
fn bench_structural_inserts() {
    for (name, css, prepend) in [
        ("append, no structural rule", "li { color: red; }", false),
        (
            "append, li:last-of-type",
            "li:last-of-type { --x: 1; }",
            false,
        ),
        (
            "append, li:nth-last-child(2)",
            "li:nth-last-child(2) { --x: 1; }",
            false,
        ),
        ("append, li:last-child", "li:last-child { --x: 1; }", false),
        (
            "prepend, li:nth-child(odd)",
            "li:nth-child(odd) { --x: 1; }",
            true,
        ),
        ("prepend, .r + .r", ".r + .r { --x: 1; }", true),
        ("prepend, no structural rule", "li { color: red; }", true),
    ] {
        let mut best = std::time::Duration::MAX;
        for _ in 0..3 {
            best = best.min(bench_list(css, 1000, prepend));
        }
        println!("BENCH {name}: {:?}", best);
    }
}

#[test]
#[ignore]
fn bench_bulk_attr_writes() {
    let css = ".row.on { color: rgb(1,2,3); } .row[data-sel] .cell { --s: 1; }";
    let mut best = std::time::Duration::MAX;
    for _ in 0..5 {
        let mut d = doc(css);
        let body = d.body();
        let list = el(&mut d, "div", "");
        let mut rows = vec![];
        for _ in 0..1000 {
            let r = el(&mut d, "div", "row");
            let c = el(&mut d, "span", "cell");
            let t = d.create_text("x");
            d.append_child(c, t);
            d.append_child(r, c);
            d.append_child(list, r);
            rows.push(r);
        }
        d.append_child(body, list);
        settle(&mut d);
        let t = std::time::Instant::now();
        for (i, &r) in rows.iter().enumerate() {
            d.set_attribute(r, "data-foo", &i.to_string());
            d.set_attribute(r, "class", "row on");
            if i % 2 == 0 {
                d.set_attribute(r, "data-sel", "");
            }
        }
        d.resolve_layout(800.0, 600.0);
        best = best.min(t.elapsed());
    }
    println!("BENCH bulk 1000x3 attr writes + resolve: {best:?}");
}

/// T24: `replace_node` swapping a non-empty child for an empty text node
/// flips the parent's `:empty`, while a non-empty-for-non-empty swap cannot
/// and skips the restyle (the editor's view diff does that for every mark).
/// This is the half that must still restyle.
#[test]
fn t24_replace_node_to_empty_text_flips_empty() {
    let css = ".d:empty { padding-left: 3px; } .d:empty + .s { --q: 1; }";
    let mut a = doc(css);
    let body = a.body();
    let p = el(&mut a, "div", "");
    let d = el(&mut a, "div", "d");
    let s = el(&mut a, "div", "s");
    let t = a.create_text("x");
    a.append_child(d, t);
    a.append_child(p, d);
    a.append_child(p, s);
    a.append_child(body, p);
    settle(&mut a);
    let empty = a.create_text("");
    a.replace_node(t, empty);
    settle(&mut a);
    let mut b = doc(css);
    let body = b.body();
    let p = el(&mut b, "div", "");
    let d = el(&mut b, "div", "d");
    let s = el(&mut b, "div", "s");
    let t = b.create_text("");
    b.append_child(d, t);
    b.append_child(p, d);
    b.append_child(p, s);
    b.append_child(body, p);
    settle(&mut b);
    assert_twin(&a, &b, "t24");
}

// Round 2 of the #894 review (fixtures by the reviewer).

const EMPTY_CSS: &str = ".d:empty { padding-left: 3px; --r: 1; } .d:empty + .s { --q: 1; } \
     .d:empty ~ .t { --w: 1; } .d:not(:empty) + .s { --n: 1; }";

/// Build `<div><div.d>{kids}</div><div.s/><div.t/></div>` where kids is a list
/// of ("text", content) or ("span", "").
fn empty_doc(css: &str, kids: &[(&str, &str)]) -> (RinchDocument, NodeId, Vec<NodeId>) {
    let mut d = doc(css);
    let body = d.body();
    let p = el(&mut d, "div", "");
    let x = el(&mut d, "div", "d");
    let mut ks = vec![];
    for (k, c) in kids {
        let n = if *k == "text" {
            d.create_text(c)
        } else {
            el(&mut d, k, "")
        };
        d.append_child(x, n);
        ks.push(n);
    }
    let s = el(&mut d, "div", "s");
    let t = el(&mut d, "div", "t");
    d.append_child(p, x);
    d.append_child(p, s);
    d.append_child(p, t);
    d.append_child(body, p);
    settle(&mut d);
    (d, x, ks)
}

/// R1: set_text_content(el, "") on an element with TWO non-empty children.
#[test]
fn r1_set_text_content_empty_on_multi_child_element() {
    let (mut a, x, _) = empty_doc(EMPTY_CSS, &[("span", ""), ("span", "")]);
    a.set_text_content(x, "");
    settle(&mut a);
    let (b, _, _) = empty_doc(EMPTY_CSS, &[("text", "")]);
    assert_twin(&a, &b, "r1");
}

/// R1b: same, with exactly one old child (the case the count allows).
#[test]
fn r1b_set_text_content_empty_on_single_child_element() {
    let (mut a, x, _) = empty_doc(EMPTY_CSS, &[("span", "")]);
    a.set_text_content(x, "");
    settle(&mut a);
    let (b, _, _) = empty_doc(EMPTY_CSS, &[("text", "")]);
    assert_twin(&a, &b, "r1b");
}

/// R14: two prepends in one frame under a transitioned :nth-child rule. The
/// first prepended row is cascaded at insertion as :nth-child(1), then
/// displaced to :nth-child(2) by the second prepend in the SAME frame. A
/// browser never gives it a before-change style (it was never rendered), so
/// no transition may start.
#[test]
fn r14_row_displaced_in_its_insertion_frame_does_not_transition() {
    let css =
        "li { transition: padding-left 1s linear; } li:nth-child(odd) { padding-left: 20px; }";
    let mut d = doc(css);
    let body = d.body();
    let ul = el(&mut d, "ul", "");
    d.append_child(body, ul);
    let first = el(&mut d, "li", "");
    d.append_child(ul, first);
    settle(&mut d);
    let a1 = el(&mut d, "li", "");
    d.insert_before(ul, a1, first);
    let a2 = el(&mut d, "li", "");
    d.insert_before(ul, a2, a1);
    settle(&mut d);
    let trans: Vec<_> = d.tree.active_transitions.keys().copied().collect();
    eprintln!(
        "r14 transitions on {:?} (a1={}, a2={}, first={}); a1 pad={:?}",
        trans, a1.0, a2.0, first.0, d.tree.nodes[a1.0].computed_style.padding_left
    );
    assert!(
        !trans.contains(&a1.0),
        "a row inserted and displaced in one frame started a transition"
    );
}

/// R14b: same shape with appends under `li:last-child` (a keyed `for`
/// appending two rows in one frame).
#[test]
fn r14b_two_appends_last_child_transition() {
    let css = "li { transition: padding-left 1s linear; } li:last-child { padding-left: 20px; }";
    let mut d = doc(css);
    let body = d.body();
    let ul = el(&mut d, "ul", "");
    d.append_child(body, ul);
    let first = el(&mut d, "li", "");
    d.append_child(ul, first);
    settle(&mut d);
    let a1 = el(&mut d, "li", "");
    d.append_child(ul, a1);
    let a2 = el(&mut d, "li", "");
    d.append_child(ul, a2);
    settle(&mut d);
    let trans: Vec<_> = d.tree.active_transitions.keys().copied().collect();
    eprintln!(
        "r14b transitions on {:?} (a1={}, a2={}, first={})",
        trans, a1.0, a2.0, first.0
    );
    assert!(
        !trans.contains(&a1.0),
        "a row inserted and displaced in one frame started a transition"
    );
}

/// R15 (kills N9): insert a child under an ancestor whose class changed this
/// frame (snapshot pending, no hint yet). The child must not be cascaded
/// against the ancestor's old style and then transition at the frame resolve.
#[test]
fn r15_insert_under_snapshotted_ancestor_does_not_transition() {
    let css = ".p { color: rgb(255, 0, 0); } .p.on { color: rgb(0, 0, 255); } \
               .c { transition: color 1s linear; }";
    let mut d = doc(css);
    let body = d.body();
    let p = el(&mut d, "div", "p");
    d.append_child(body, p);
    settle(&mut d);
    d.set_attribute(p, "class", "p on");
    let c = el(&mut d, "div", "c");
    d.append_child(p, c);
    settle(&mut d);
    let trans: Vec<_> = d.tree.active_transitions.keys().copied().collect();
    assert!(
        !trans.contains(&c.0),
        "freshly inserted child transitioned: {trans:?}"
    );
}

/// R15b: what `resolve_inserted_subtree`'s ancestor-snapshot check buys. An
/// insertion under an ancestor whose class changed this frame (a pending
/// snapshot, no restyle hint yet) must not cascade the new child against the
/// ancestor's *old* style: the check sends it through the full resolve,
/// which processes the snapshot first. R15 cannot see a stale insertion-time
/// cascade any more, because the frame's resolve corrects the child and a
/// node never rendered starts no transition; this reads the style the
/// insertion itself produced.
#[test]
fn r15b_insert_under_snapshotted_ancestor_cascades_against_the_new_style() {
    let css = ".p { color: rgb(255, 0, 0); } .p.on { color: rgb(0, 0, 255); }";
    let mut d = doc(css);
    let body = d.body();
    let p = el(&mut d, "div", "p");
    d.append_child(body, p);
    settle(&mut d);
    d.set_attribute(p, "class", "p on");
    let c = el(&mut d, "div", "c");
    d.append_child(p, c);
    let got = format!("{:?}", d.tree.nodes[c.0].computed_style.color);

    let mut e = doc(css);
    let body = e.body();
    let p = el(&mut e, "div", "p on");
    let c = el(&mut e, "div", "c");
    e.append_child(p, c);
    e.append_child(body, p);
    settle(&mut e);
    let want = format!("{:?}", e.tree.nodes[c.0].computed_style.color);
    assert_eq!(
        got, want,
        "the insertion cascaded against the old ancestor style"
    );
}
