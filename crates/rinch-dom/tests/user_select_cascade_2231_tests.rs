//! Issue #2231 — `user-select` from a style sheet cascades.
//!
//! Stylo 0.22's servo build parses `user-select` (with its `-webkit-` and
//! `-moz-` aliases) and computes it into the `ui` struct; `from_stylo` used to
//! write `Auto` whatever it said and re-read the inline `style` attribute
//! alone, so a rule in any sheet (the UA's, a component's, the app's) did
//! nothing on desktop.
//!
//! The property is not inherited: a child that declares nothing computes
//! `auto`, and `auto` is resolved from the parent where selection reads it
//! (`rinch::app::text_selection::resolved_user_select`). These fixtures read
//! the computed value; the selection consequences are pinned in the shell.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::UserSelectValue as U;

fn doc_with(css: &str, html: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let style = doc.create_element("style");
    let text = doc.create_text(css);
    doc.append_child(style, text);
    doc.append_child(body, style);
    let host = doc.create_element("div");
    doc.append_child(body, host);
    doc.set_inner_html(host, html);
    doc.resolve_layout(800.0, 600.0);
    doc
}

fn by_id(doc: &RinchDocument, id: &str) -> NodeId {
    doc.query_selector(&format!("#{id}"))
        .unwrap_or_else(|| panic!("no #{id}"))
}

fn us(doc: &RinchDocument, id: &str) -> U {
    doc.tree
        .get(by_id(doc, id).0)
        .unwrap()
        .computed_style
        .user_select
}

/// Every keyword, from a class rule. `text` and `all` are off the initial
/// `auto` and off the old tag default, so a reader that ignores the sheet
/// fails each of them.
#[test]
fn a_style_sheet_rule_sets_user_select() {
    let doc = doc_with(
        ".n { user-select: none } .t { user-select: text } \
         .a { user-select: all } .c { user-select: contain } \
         .au { user-select: auto }",
        r#"<div id="n" class="n">n</div><div id="t" class="t">t</div>
           <div id="a" class="a">a</div><div id="c" class="c">c</div>
           <div id="au" class="au">au</div><div id="plain">p</div>"#,
    );
    assert_eq!(us(&doc, "n"), U::None);
    assert_eq!(us(&doc, "t"), U::Text);
    assert_eq!(us(&doc, "a"), U::All);
    assert_eq!(us(&doc, "c"), U::Contain);
    assert_eq!(us(&doc, "au"), U::Auto);
    assert_eq!(us(&doc, "plain"), U::Auto, "no rule: the initial `auto`");
}

/// The prefixed spellings are aliases of the one property, so the later
/// declaration in a block wins whichever spelling it uses.
#[test]
fn the_prefixed_spellings_are_the_same_property() {
    let doc = doc_with(
        ".w { -webkit-user-select: none } \
         .wt { user-select: none; -webkit-user-select: text } \
         .tw { -webkit-user-select: text; user-select: all } \
         .m { -moz-user-select: none }",
        r#"<div id="w" class="w">w</div><div id="wt" class="wt">wt</div>
           <div id="tw" class="tw">tw</div><div id="m" class="m">m</div>"#,
    );
    assert_eq!(us(&doc, "w"), U::None);
    assert_eq!(us(&doc, "wt"), U::Text, "the later, prefixed, declaration");
    assert_eq!(us(&doc, "tw"), U::All, "the later, unprefixed, declaration");
    assert_eq!(us(&doc, "m"), U::None);
}

/// Specificity and source order decide between rules; inline `style` beats a
/// normal rule; an `!important` rule beats inline `style`.
#[test]
fn the_cascade_decides_between_declarations() {
    let doc = doc_with(
        "#s { user-select: text } .s { user-select: none } \
         .o1 { user-select: none } .o2 { user-select: all } \
         .i { user-select: none } \
         .imp { user-select: none !important }",
        r#"<div id="s" class="s">s</div>
           <div id="o" class="o2 o1">o</div>
           <div id="i" class="i" style="user-select: text">i</div>
           <div id="imp" class="imp" style="user-select: text">imp</div>"#,
    );
    assert_eq!(us(&doc, "s"), U::Text, "an id beats a class");
    assert_eq!(us(&doc, "o"), U::All, "the later rule of equal specificity");
    assert_eq!(us(&doc, "i"), U::Text, "inline beats a normal rule");
    assert_eq!(us(&doc, "imp"), U::None, "`!important` beats inline");
}

/// `user-select` is not inherited: under a `none` parent a child that says
/// nothing computes `auto`, and one that says `text` keeps it.
#[test]
fn user_select_is_not_inherited() {
    let doc = doc_with(
        ".p { user-select: none } .t { user-select: text }",
        r#"<div id="p" class="p"><p id="child">c</p><span id="t" class="t">t</span></div>"#,
    );
    assert_eq!(us(&doc, "p"), U::None);
    assert_eq!(us(&doc, "child"), U::Auto);
    assert_eq!(us(&doc, "t"), U::Text);
}

/// The code-ish elements are selectable by default (rinch's UA sheet), and an
/// author rule — not only an inline style — overrides that per element.
#[test]
fn an_author_rule_overrides_the_code_default() {
    let doc = doc_with(
        ".off { user-select: none }",
        r#"<pre id="pre">a</pre><code id="code">b</code>
           <code id="off" class="off">c</code><kbd id="kbd">d</kbd><samp id="samp">e</samp>"#,
    );
    for id in ["pre", "code", "kbd", "samp"] {
        assert_eq!(us(&doc, id), U::Text, "<{id}>");
    }
    assert_eq!(
        us(&doc, "off"),
        U::None,
        "the author rule beats the UA rule"
    );
}

/// A class change restyles it: the next resolve reads the new rule.
#[test]
fn a_class_change_restyles_user_select() {
    let mut doc = doc_with(".t { user-select: text }", r#"<div id="x">x</div>"#);
    let x = by_id(&doc, "x");
    assert_eq!(us(&doc, "x"), U::Auto);
    doc.set_attribute(x, "class", "t");
    doc.resolve_layout(801.0, 600.0);
    assert_eq!(us(&doc, "x"), U::Text);
    doc.remove_attribute(x, "class");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(us(&doc, "x"), U::Auto);
}
