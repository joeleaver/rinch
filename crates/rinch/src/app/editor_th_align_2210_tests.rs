//! Alignment inside an editor's header cells, through a real cascade (#2210).
//!
//! The view writes no `text-align` for an unset alignment, so the stylesheet
//! decides it. Editor cells are `display: block`, and the UA sheet's
//! `th { text-align: center }` reached a header cell's paragraphs: every saved
//! table showed centred headers, and align-left (which stored `left` and wrote
//! nothing) could not undo it. The editor's sheet now start-aligns a `th`, and
//! an explicit `left` is written inline, so it holds in a `th` and in an RTL
//! editor.
//!
//! This mounts a real `Editor`, so the stylesheet under test is the shipped
//! `DEFAULT_EDITOR_CSS`, and reads each paragraph's computed `text-align`.

use super::*;

use rinch_core::Component;
use rinch_dom::computed_style::TextAlignValue;

const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// A real `Editor` holding `content`, with the author `css` loaded after it.
fn mount(content: &'static str, css: &'static str) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        crate::editor::Editor {
            content: content.into(),
            ..Default::default()
        }
        .render(scope, &[])
    });
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    if !css.is_empty() {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(css);
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    app
}

/// The computed `text-align` of the `<p>` whose parent is a `parent_tag`
/// element, in document order: `(the cell's own, the paragraph's)`.
fn aligns_in(app: &RinchApp, parent_tag: &str) -> Vec<(TextAlignValue, TextAlignValue)> {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let mut out: Vec<(u64, TextAlignValue, TextAlignValue)> = d
        .tree
        .nodes
        .iter()
        .filter(|(_, n)| n.tag() == Some("p"))
        .filter_map(|(id, n)| {
            let parent = d.tree.get(n.parent?)?;
            (parent.tag() == Some(parent_tag)).then_some((
                id,
                parent.computed_style.text_align,
                n.computed_style.text_align,
            ))
        })
        .collect();
    out.sort_by_key(|(id, ..)| *id);
    out.into_iter().map(|(_, c, p)| (c, p)).collect()
}

const TABLE: &str = "<table><tr>\
    <th><p>unset</p></th>\
    <th><p style=\"text-align:left\">left</p></th>\
    <th><p style=\"text-align:center\">center</p></th>\
    </tr><tr>\
    <td><p>a</p></td><td><p>b</p></td><td><p>c</p></td>\
    </tr></table>";

/// A header cell's unset paragraph starts like a body cell's; an explicit
/// left or center holds.
#[test]
fn a_header_cells_paragraph_starts_unless_it_says_otherwise() {
    let app = mount(TABLE, "");
    let th = aligns_in(&app, "th");
    assert_eq!(th.len(), 3, "three header paragraphs: {th:?}");
    // The cell itself: the editor sheet's `start`, not the UA's centre.
    assert_eq!(th[0].0, TextAlignValue::Start, "the th's own text-align");
    assert_eq!(th[0].1, TextAlignValue::Start, "unset in a th");
    assert_eq!(th[1].1, TextAlignValue::Left, "explicit left in a th");
    assert_eq!(th[2].1, TextAlignValue::Center, "explicit center in a th");
    // Positive control: a body cell's unset paragraph is `start` too.
    let td = aligns_in(&app, "td");
    assert_eq!(td[0].1, TextAlignValue::Start, "unset in a td");
}

/// In an RTL editor an explicit left is still left, and an unset alignment
/// is the inherited `start` (the right edge).
#[test]
fn an_explicit_left_holds_in_an_rtl_editor() {
    let app = mount(TABLE, "[data-pm-editor] { direction: rtl; }");
    let th = aligns_in(&app, "th");
    assert_eq!(th[0].1, TextAlignValue::Start, "unset in an RTL th");
    assert_eq!(th[1].1, TextAlignValue::Left, "explicit left in an RTL th");
}

/// A top-level paragraph: explicit left is written and computes `left`; the
/// unset one beside it computes `start`.
#[test]
fn an_explicit_left_paragraph_computes_left() {
    let app = mount(
        "<p>unset</p><p style=\"text-align:left\">left</p>",
        "[data-pm-editor] { direction: rtl; }",
    );
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let mut ps: Vec<(u64, TextAlignValue)> = d
        .tree
        .nodes
        .iter()
        .filter(|(_, n)| n.tag() == Some("p"))
        .map(|(id, n)| (id, n.computed_style.text_align))
        .collect();
    ps.sort_by_key(|(id, _)| *id);
    let aligns: Vec<TextAlignValue> = ps.into_iter().map(|(_, a)| a).collect();
    assert_eq!(aligns, [TextAlignValue::Start, TextAlignValue::Left]);
}
