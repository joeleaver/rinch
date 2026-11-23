//! Read-only selection against Chrome 158 (headless, Playwright's mouse,
//! `getSelection().toString()`; the same markup in the same 600px root with
//! `font: 16px/20px sans-serif`): what a triple click takes and how a
//! Shift+click after a double or triple click extends (#2256), a double
//! click's word through the boxes touching it (#2259), what cannot be
//! selected in a copy (#2256), and which roots near an inline-block are
//! highlighted (#2245).

use super::text_selection_across_tests::{highlighted, mount, node};
use super::text_selection_chrome_tests::SEL;
use super::text_selection_copy_boxes_tests::copies_as_chrome;
use super::text_selection_copy_gesture_tests::{app_after, copy_after};
use super::*;

/// `(name, markup, ops, Chrome's string)`, ops as `copy_after` reads them.
type Case = (&'static str, &'static str, &'static str, &'static str);

/// Every case of `cases` copied after its gestures, checked together so a
/// failure names each case off.
fn after_gestures_as_chrome(cases: &[Case]) {
    let off: Vec<String> = cases
        .iter()
        .filter_map(|&(name, html, ops, chrome)| {
            let t = copy_after(html, ops);
            (t != chrome).then(|| format!("{name}: {t:?}, Chrome {chrome:?}"))
        })
        .collect();
    assert!(off.is_empty(), "{off:#?}");
}

#[test]
fn what_cannot_be_selected_writes_nothing_around_it() {
    // #2256: a `user-select: none` picture block still counted as written
    // (a blank line), and an unselectable inline table kept the spaces
    // around it. A selectable picture still ends its block with a line
    // break (the control).
    copies_as_chrome(&[
        (
            "unselectable picture block",
            "<div id=\"a\">first</div><div style=\"user-select:none\"><img style=\"display:block;width:20px;height:20px\" src=\"data:image/gif;base64,R0lGODlhAQABAAAAACw=\"></div><div id=\"b\">second</div>",
            "first\nsecond",
        ),
        (
            "selectable picture block",
            "<div id=\"a\">first</div><div><img style=\"display:block;width:20px;height:20px\" src=\"data:image/gif;base64,R0lGODlhAQABAAAAACw=\"></div><div id=\"b\">second</div>",
            "first\n\nsecond",
        ),
        (
            "unselectable inline table",
            "<div id=\"a\">first <table style=\"display:inline-table;user-select:none\"><tr><td>c</td></tr></table> after</div><div id=\"b\">second</div>",
            "first \n  after\nsecond",
        ),
        (
            "inline table in an unselectable span",
            "<div id=\"a\">first <span style=\"user-select:none\"><table style=\"display:inline-table\"><tr><td>c</td></tr></table></span> after</div><div id=\"b\">second</div>",
            "first \n  after\nsecond",
        ),
    ]);
}

#[test]
fn a_triple_click_takes_its_paragraph_and_the_break_after_it() {
    // #2256: Chrome's triple click selects the paragraph to the start of
    // the next one, so its copy ends with the paragraph's line breaks. A
    // paragraph ends at a `<br>` (its line break is in) and at a float
    // (block-level, so a paragraph edge: nothing after it is in); the last
    // paragraph of the document has nothing after it. Inside an
    // inline-block, the paragraph is the one the box stands in.
    after_gestures_as_chrome(&[
        (
            "p",
            "<p style=\"margin:0\"><span id=\"t\">text</span></p><p style=\"margin:0\">next</p>",
            "click:t:0.5:3",
            "text\n\n",
        ),
        (
            "div",
            "<div><span id=\"t\">text</span></div><div>next</div>",
            "click:t:0.5:3",
            "text\n",
        ),
        (
            "last paragraph",
            "<p style=\"margin:0\">zero</p><p style=\"margin:0\"><span id=\"t\">text</span></p>",
            "click:t:0.5:3",
            "text",
        ),
        (
            "nested containers",
            "<div><p style=\"margin:0\"><span id=\"t\">text</span></p></div><div><p style=\"margin:0\">next</p></div>",
            "click:t:0.5:3",
            "text\n\n",
        ),
        (
            "first line before a br",
            "<div><span id=\"t\">text</span><br>line2</div><div>next</div>",
            "click:t:0.5:3",
            "text\n",
        ),
        (
            "line between two brs",
            "<div>one<br><span id=\"t\">two</span><br>three</div><div>next</div>",
            "click:t:0.5:3",
            "two\n",
        ),
        (
            "last line after a br",
            "<div>one<br><span id=\"t\">two</span></div><div>next</div>",
            "click:t:0.5:3",
            "two\n",
        ),
        (
            "float in the middle",
            "<p style=\"margin:0\"><span id=\"t\">te</span><span style=\"float:right\">fr</span>xt</p><p style=\"margin:0\">next</p>",
            "click:t:0.5:3",
            "te",
        ),
        (
            "trailing float before a div",
            "<div><span id=\"t\">text</span> <span style=\"float:right\">fr</span></div><div>next</div>",
            "click:t:0.5:3",
            "text",
        ),
        (
            "unselectable paragraph next",
            "<p style=\"margin:0\"><span id=\"t\">text</span></p><p style=\"margin:0;user-select:none\">nope</p><p style=\"margin:0\">next</p>",
            "click:t:0.5:3",
            "text\n\n\n",
        ),
        (
            "inside an inline-block",
            "<p style=\"margin:0\">lead <span id=\"t\" style=\"display:inline-block\">mid</span> tail</p><p style=\"margin:0\">next</p>",
            "click:t:0.5:3",
            "lead mid tail\n\n",
        ),
        (
            "inside a nested inline-block",
            "<div><p style=\"margin:0\">lead <span style=\"display:inline-block\">x <span id=\"t\" style=\"display:inline-block\">deep</span> y</span> tail</p></div><p style=\"margin:0\">next</p>",
            "click:t:0.5:3",
            "lead x deep y tail\n\n",
        ),
        (
            "inline-block last in its paragraph",
            "<p style=\"margin:0\">a <span id=\"t\" style=\"display:inline-block\">mid</span></p><p style=\"margin:0\">next</p>",
            "click:t:0.5:3",
            "a mid\n\n",
        ),
    ]);
}

#[test]
fn a_shift_click_after_a_double_or_triple_click_extends_by_its_unit() {
    // #2256: after a triple click, Shift+click extends by paragraphs, and
    // after a double click by words; extending backward, the selection
    // runs from the clicked unit's start to the end of the first one (its
    // trailing break included), and a later Shift+click extends from the
    // first unit again. A drag's selection keeps its anchor (the control),
    // and Select All goes back to extending by characters (review of #2949).
    after_gestures_as_chrome(&[
        (
            "triple, shift before",
            "<p id=\"e0\" style=\"margin:0\"><span id=\"s\">zero</span></p><p style=\"margin:0\"><span id=\"t\">text</span> <span style=\"display:inline-block\">mid</span></p>",
            "click:t:0.5:3;shift:s:0.0",
            "zero\n\ntext mid",
        ),
        (
            "triple, shift before with a paragraph after",
            "<p style=\"margin:0\"><span id=\"s\">zero</span></p><p style=\"margin:0\"><span id=\"t\">text</span></p><p style=\"margin:0\">last</p>",
            "click:t:0.5:3;shift:s:0.5",
            "zero\n\ntext\n\n",
        ),
        (
            "triple, shift into a later paragraph",
            "<p style=\"margin:0\"><span id=\"t\">text</span></p><p style=\"margin:0\"><span id=\"s\">one two three</span></p><p style=\"margin:0\">last</p>",
            "click:t:0.5:3;shift:s:0.5",
            "text\n\none two three\n\n",
        ),
        (
            "triple, shift before, then shift after",
            "<p style=\"margin:0\"><span id=\"s\">zero</span></p><p style=\"margin:0\"><span id=\"t\">text</span></p><p style=\"margin:0\"><span id=\"u\">three</span></p><p style=\"margin:0\">last</p>",
            "click:t:0.5:3;shift:s:0.5;shift:u:0.5",
            "text\n\nthree\n\n",
        ),
        (
            "triple, Select All, shift",
            "<p style=\"margin:0\"><span id=\"s\">zero one</span></p><p style=\"margin:0\"><span id=\"t\">text</span></p><p style=\"margin:0\"><span id=\"u\">three four</span></p>",
            "click:t:0.5:3;all;shift:u:0.5",
            "zero one\n\ntext\n\nthree",
        ),
        (
            "double, shift after",
            "<p style=\"margin:0\"><span id=\"t\">alpha beta</span> gamma</p><p style=\"margin:0\"><span id=\"s\">next more</span></p>",
            "click:t:0.2:2;shift:s:0.2",
            "alpha beta gamma\n\nnext",
        ),
        (
            "double, shift before",
            "<p style=\"margin:0\"><span id=\"s\">zero one</span></p><p style=\"margin:0\"><span id=\"t\">alpha beta gamma</span></p>",
            "click:t:0.5:2;shift:s:0.3",
            "zero one\n\nalpha beta",
        ),
        (
            "double, shift earlier in its paragraph",
            "<p style=\"margin:0\"><span id=\"t\">alpha beta gamma delta</span></p>",
            "click:t:0.5:2;shift:t:0.05",
            "alpha beta gamma",
        ),
    ]);
}

#[test]
fn a_double_click_takes_the_boxes_its_word_touches() {
    // #2259: Chrome's word runs on through an inline-block, an inline-flex
    // or a float touching it with no break between, into the box's text
    // up to a break there; a picture or an inline table ends it, and a
    // space keeps the boxes out (the controls).
    after_gestures_as_chrome(&[
        (
            "inline-blocks",
            "<p style=\"margin:0\"><span style=\"display:inline-block\">m1</span><span id=\"t\">word</span><span style=\"display:inline-block\">m2</span> tail</p><p style=\"margin:0\">next</p>",
            "click:t:0.5:2",
            "m1wordm2",
        ),
        (
            "floats",
            "<p style=\"margin:0\"><span style=\"float:left\">fl</span><span id=\"t\">word</span><span style=\"float:right\">fr</span> tail</p><p style=\"margin:0\">next</p>",
            "click:t:0.5:2",
            "flwordfr",
        ),
        (
            "spaced",
            "<p style=\"margin:0\"><span style=\"display:inline-block\">m1</span> <span id=\"t\">word</span> <span style=\"display:inline-block\">m2</span></p>",
            "click:t:0.5:2",
            "word",
        ),
        (
            "text, box, word",
            "<p style=\"margin:0\">foo<span style=\"display:inline-block\">m1</span><span id=\"t\">word</span> tail</p>",
            "click:t:0.5:2",
            "foom1word",
        ),
        (
            "box with two words before",
            "<p style=\"margin:0\"><span style=\"display:inline-block\">a b</span><span id=\"t\">word</span> tail</p>",
            "click:t:0.5:2",
            "bword",
        ),
        (
            "box with two words after",
            "<p style=\"margin:0\">x <span id=\"t\">word</span><span style=\"display:inline-block\">m2 more</span></p>",
            "click:t:0.5:2",
            "wordm2",
        ),
        (
            "inline-flex",
            "<p style=\"margin:0\"><span id=\"t\">word</span><span style=\"display:inline-flex\">fx</span> tail</p>",
            "click:t:0.5:2",
            "wordfx",
        ),
        (
            "picture",
            "<p style=\"margin:0\"><img style=\"width:10px;height:10px\" src=\"data:image/gif;base64,R0lGODlhAQABAAAAACw=\"><span id=\"t\">word</span> tail</p>",
            "click:t:0.5:2",
            "word",
        ),
        (
            "inline table",
            "<p style=\"margin:0\"><span id=\"t\">word</span><table style=\"display:inline-table\"><tr><td>cell</td></tr></table> tail</p>",
            "click:t:0.5:2",
            "word",
        ),
        (
            "button after",
            "<p style=\"margin:0\"><span id=\"t\">word</span><button>OK</button> tail</p>",
            "click:t:0.5:2",
            "word",
        ),
        (
            "button before",
            "<p style=\"margin:0\"><button>OK</button><span id=\"t\">word</span> tail</p>",
            "click:t:0.5:2",
            "OKword",
        ),
        (
            "then shift",
            "<p style=\"margin:0\"><span style=\"display:inline-block\">m1</span><span id=\"t\">word</span> tail</p><p style=\"margin:0\"><span id=\"s\">next</span></p>",
            "click:t:0.5:2;shift:s:0.5",
            "m1word tail\n\nnext",
        ),
    ]);
}

#[test]
fn a_press_drag_on_a_picture_selects_nothing() {
    // #2259: Chrome starts an image drag there, and selects nothing.
    let t = copy_after(
        "<div id=\"a\"><img id=\"i\" style=\"width:40px;height:20px\" src=\"data:image/gif;base64,R0lGODlhAQABAAAAACw=\"> text after</div><div id=\"b\">second</div>",
        "drag:i:0.5:b:0.9",
    );
    assert_eq!(t, "", "Chrome selects nothing");
    // The control: the same drag from the text beside it selects.
    let t = copy_after(
        "<div id=\"a\"><img id=\"i\" style=\"width:40px;height:20px\" src=\"data:image/gif;base64,R0lGODlhAQABAAAAACw=\"> <span id=\"x\">text after</span></div><div id=\"b\">second</div>",
        "drag:x:0.0:b:0.9",
    );
    assert!(t.ends_with("second"), "{t:?}");
}

/// The highlighted part of `#id` after `ops` on `html`.
fn highlight_after(html: &'static str, ops: &str, ids: &[&str]) -> Vec<Option<(usize, usize)>> {
    let app = app_after(html, ops);
    ids.iter().map(|id| highlighted(&app, id)).collect()
}

/// A selection from `#a`'s text start to `#b`'s text end, highlighted.
fn highlight_range(html: &'static str, ids: &[&str]) -> Vec<Option<(usize, usize)>> {
    let mut app = mount(html, SEL);
    let (a, b) = (node(&app, "a"), node(&app, "b"));
    {
        let d = app.doc.as_ref().unwrap().borrow();
        let first_text = |id: u64| {
            let n = d.tree.get(id).unwrap();
            n.children[0]
        };
        let root_at = |text: u64| {
            let mut id = text;
            loop {
                let n = d.tree.get(id).unwrap();
                if id != text && n.text_layout.is_some() {
                    break id;
                }
                if let Some(b) = n.run_box {
                    break b;
                }
                id = n.parent.unwrap();
            }
        };
        let range = |text: u64| {
            let root = root_at(text);
            let l = d.tree.get(root).unwrap().text_layout.as_ref().unwrap();
            let r = l.text_ranges.iter().find(|r| r.node_id == text).unwrap();
            (root, r.flat_start, r.flat_end)
        };
        let (ra, sa, _) = range(first_text(a));
        let (rb, _, eb) = range(first_text(b));
        let mut sel = TextSelection::at(ra, sa);
        sel.focus_node = rb;
        sel.focus_offset = eb;
        sel.anchor_side.after_inline = true;
        drop(d);
        app.text_selection = Some(sel);
    }
    app.paint_text_selection();
    ids.iter().map(|id| highlighted(&app, id)).collect()
}

#[test]
fn an_inline_block_is_highlighted_where_it_stands_in_its_line() {
    // #2245: the highlight followed root order (a paragraph before the
    // inline-blocks in it), not where each box stands. Chrome highlights
    // a box the selection reaches past and no other.
    assert_eq!(
        highlight_range(
            "<div>before <span id=\"ib\" style=\"display:inline-block\">mid</span> <span id=\"a\">after</span></div><div id=\"b\">second</div>",
            &["ib"],
        ),
        [None],
        "a box before the start: Chrome does not highlight it"
    );
    assert_eq!(
        highlight_range(
            "<div id=\"a\">first</div><div>before <span id=\"ib\" style=\"display:inline-block\">mid</span> <span id=\"b\">after</span></div>",
            &["ib"],
        ),
        [Some((0, 3))],
        "a box before the end: Chrome highlights it"
    );
    assert_eq!(
        highlight_range(
            "<div id=\"a\">first</div><div><span id=\"b\">before</span> <span id=\"ib\" style=\"display:inline-block\">mid</span> after</div>",
            &["ib"],
        ),
        [None],
        "a box after the end (the control)"
    );
}

#[test]
fn a_selection_from_an_inline_block_into_its_paragraph_is_highlighted_in_order() {
    // #2245: from inside an inline-block to later text in its paragraph,
    // the paragraph was highlighted from its start to the focus and the
    // box from its start to the anchor; Chrome ("id aft") highlights the
    // box from the anchor to its end and the paragraph from the box on.
    let html = "<div id=\"p\">before <span id=\"ib\" style=\"display:inline-block\"><span id=\"a\">mid</span></span> <span id=\"b\">after</span> tail</div>";
    let app = app_after(html, "drag:a:0.3:b:0.6");
    let copy = app_after(html, "drag:a:0.3:b:0.6")
        .text_selection_text()
        .unwrap_or_default();
    assert_eq!(copy, "id aft", "the copy, as Chrome's");
    let p = highlighted(&app, "p").expect("the paragraph is highlighted");
    let ib = highlighted(&app, "ib").expect("the box is highlighted");
    assert_eq!(ib, (1, 3), "the box, from the anchor to its end");
    assert_eq!(
        p,
        ("before ".len(), "before  aft".len()),
        "the paragraph, from where the box stands to the focus"
    );
}

#[test]
fn an_inline_block_in_an_anonymous_boxs_run_is_highlighted() {
    // #2245: an inline-block inside a run an anonymous box lays out was no
    // selectable root, so it was never highlighted. Chrome: "mid ta".
    let html = "<div><span>lead <span id=\"ib\" style=\"display:inline-block\">mid</span> <span id=\"b\">tail</span></span><div>block</div></div>";
    assert_eq!(copy_after(html, "drag:ib:0.0:b:0.5"), "mid ta");
    assert_eq!(
        highlight_after(html, "drag:ib:0.0:b:0.5", &["ib"]),
        [Some((0, 3))]
    );
}

/// `side_at` for a point `fx` of the way across `#i`'s box, in `#d`.
fn side_across(html: &'static str, fx: f32) -> bool {
    let app = mount(html, SEL);
    let (d_id, i_id) = (node(&app, "d"), node(&app, "i"));
    let d = app.doc.as_ref().unwrap().borrow();
    let (x, y, w, h) = super::hit_testing::painted_element_box(&d.tree, i_id);
    let layout = d.tree.get(d_id).unwrap().text_layout.as_ref().unwrap();
    let at = layout
        .atomic_inlines()
        .find(|&(id, _)| id == i_id)
        .map(|(_, at)| at)
        .expect("#i is an atomic inline of #d");
    RinchApp::side_at(&d.tree, d_id, x + w * fx, y + h / 2.0, at).after_inline
}

#[test]
fn a_boxs_after_side_is_its_left_on_a_right_to_left_line() {
    // #2259: `side_at` read a box's right half as its after side on every
    // line; on a right-to-left line the after side is the left
    // (`inline_box_caret`). Left to right is the control.
    let rtl = "<div id=\"d\" dir=\"rtl\">שלום <span id=\"i\" style=\"display:inline-block;width:40px;height:16px\"></span> סוף</div>";
    let ltr = "<div id=\"d\">abc <span id=\"i\" style=\"display:inline-block;width:40px;height:16px\"></span> def</div>";
    assert!(
        side_across(rtl, 0.2),
        "the left half of a box on an RTL line is after it"
    );
    assert!(!side_across(rtl, 0.8), "and its right half before it");
    assert!(
        side_across(ltr, 0.8),
        "left to right, the right half is after"
    );
    assert!(!side_across(ltr, 0.2));
}

/// The steps (`text_selection_copy::WORK`) one paint of a selection takes,
/// from the start of a paragraph holding `boxes` inline-blocks into the
/// next block.
fn paint_work(boxes: usize) -> (u64, std::time::Duration) {
    let mut html = String::from("<div id=\"a\">start ");
    for i in 0..boxes {
        html.push_str(&format!(
            "<span style=\"display:inline-block\">c{i}</span> "
        ));
    }
    html.push_str("</div><div id=\"b\">next</div>");
    let html: &'static str = Box::leak(html.into_boxed_str());
    let mut app = mount(html, SEL);
    let (a, b) = (node(&app, "a"), node(&app, "b"));
    let mut sel = TextSelection::at(a, 0);
    sel.focus_node = b;
    sel.focus_offset = 4;
    app.text_selection = Some(sel);
    // The roots cache is built on the first paint; the second is a move's.
    app.paint_text_selection();
    let work = || super::text_selection_copy::WORK.with(|w| w.get());
    let before = work();
    if let Some(s) = &mut app.text_selection {
        s.focus_offset = 3;
    }
    let t = std::time::Instant::now();
    app.paint_text_selection();
    let took = t.elapsed();
    let w = work() - before;
    assert_eq!(highlighted(&app, "b"), Some((0, 3)));
    (w, took)
}

#[test]
fn a_paint_is_linear_in_the_boxes_of_an_end_root() {
    // Review of #2949: each inline-block in an end's paragraph looked up
    // where it stands with a walk of the paragraph's text, comparing paths
    // and sibling positions, so 800 boxes took 2 s a paint in debug. Four
    // times the boxes must be about four times the work.
    let (small, big) = (paint_work(100).0, paint_work(400).0);
    assert!(
        big <= 5 * small,
        "{small} steps for 100 boxes, {big} for 400"
    );
}

#[test]
#[ignore = "timing probe for the review of #2949"]
fn probe_paint_time_2949() {
    for n in [200, 400, 800] {
        let (w, took) = paint_work(n);
        eprintln!("{n} boxes: {took:?} a paint, {w} steps");
    }
}
