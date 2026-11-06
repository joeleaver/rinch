//! Copies of text in flex containers (#2241) and around atomic inlines
//! (#2242), and a corpus of #2240's cases, against Chrome 156: each case's
//! string is `getSelection().toString()` after `setBaseAndExtent(#a, 0, #b,
//! #b.childNodes.length)` on the same markup, in the same 600px root with
//! `font: 16px/20px sans-serif`.

use super::text_selection_across_tests::{mount, node};
use super::text_selection_chrome_tests::SEL;
use super::{RinchApp, TextSelection};

/// The IFC root that lays out text node `text`, and where its text stands
/// in the root's; `None` for text no root lays out (a flex container's).
fn root_of(tree: &rinch_dom::NodeTree, text: u64) -> Option<(u64, usize, usize)> {
    let mut id = text;
    let root = loop {
        let n = tree.get(id)?;
        if id != text && n.text_layout.is_some() {
            break id;
        }
        if let Some(b) = n.run_box {
            break b;
        }
        id = n.parent?;
    };
    let r = tree.get(root)?.text_layout.as_ref()?;
    let range = r.text_ranges.iter().find(|r| r.node_id == text)?;
    Some((root, range.flat_start, range.flat_end))
}

/// The text nodes under `id`, in document order.
fn texts(tree: &rinch_dom::NodeTree, id: u64, out: &mut Vec<u64>) {
    let n = tree.get(id).unwrap();
    if n.tag().is_none() {
        out.push(id);
    }
    for &c in &n.children {
        texts(tree, c, out);
    }
}

/// The copy of a selection from the start of `#a`'s first text to the end
/// of `#b`'s last, as `setBaseAndExtent(#a, 0, #b, end)` makes it.
fn copy_range(html: &'static str) -> String {
    select_range(html).text_selection_text().unwrap_or_default()
}

/// An app holding `html` with that selection.
fn select_range(html: &'static str) -> RinchApp {
    let mut app = mount(html, SEL);
    let (a, b) = (node(&app, "a"), node(&app, "b"));
    let sel = {
        let d = app.doc.as_ref().unwrap().borrow();
        let (mut ta, mut tb) = (Vec::new(), Vec::new());
        texts(&d.tree, a, &mut ta);
        texts(&d.tree, b, &mut tb);
        let (ra, sa, _) = ta
            .iter()
            .find_map(|&t| root_of(&d.tree, t))
            .expect("#a holds text a root lays out");
        let (rb, _, eb) = tb
            .iter()
            .rev()
            .find_map(|&t| root_of(&d.tree, t))
            .expect("#b holds text a root lays out");
        let mut sel = TextSelection::at(ra, sa);
        sel.focus_node = rb;
        sel.focus_offset = eb;
        // As `setBaseAndExtent` puts them: the start of `#a`'s text is after
        // the boxes before it, the end of `#b`'s before the boxes after it.
        sel.anchor_side.after_inline = true;
        sel
    };
    app.text_selection = Some(sel);
    app
}

/// Copies of `cases` (name, markup, Chrome 156's string), checked together
/// so a failure names every case off.
fn copies_as_chrome(cases: &[(&str, &'static str, &str)]) {
    let off: Vec<String> = cases
        .iter()
        .filter_map(|&(name, html, chrome)| {
            let t = copy_range(html);
            (t != chrome).then(|| format!("{name}: {t:?}, Chrome {chrome:?}"))
        })
        .collect();
    assert!(off.is_empty(), "{off:#?}");
}

#[test]
fn text_in_a_flex_container_is_copied() {
    // #2241: a flex or grid container's own text is an anonymous item,
    // laid out as a text leaf with no IFC root; the copy passed it.
    copies_as_chrome(&[
        (
            "flex loose",
            "<div id=\"a\">first</div><div style=\"display:flex\">loose</div><div id=\"b\">second</div>",
            "first\nloose\nsecond",
        ),
        (
            "flex loose mixed",
            "<div id=\"a\">first</div><div style=\"display:flex\">loose <span>span</span> tail</div><div id=\"b\">second</div>",
            "first\nloose\nspan\ntail\nsecond",
        ),
        (
            "flex loose around item",
            "<div id=\"a\">first</div><div style=\"display:flex\">loose<div>item</div>more</div><div id=\"b\">second</div>",
            "first\nloose\nitem\nmore\nsecond",
        ),
        (
            "inline-flex link",
            "<div id=\"a\">first</div><div><a href=\"#\" style=\"display:inline-flex\">Click</a></div><div id=\"b\">second</div>",
            "first\nClick\nsecond",
        ),
        (
            "inline-flex mid",
            "<div id=\"a\">first</div><div>before <span style=\"display:inline-flex\">mid</span> after</div><div id=\"b\">second</div>",
            "first\nbefore mid after\nsecond",
        ),
        (
            "inline-grid mid",
            "<div id=\"a\">first</div><div>before <span style=\"display:inline-grid\">mid</span> after</div><div id=\"b\">second</div>",
            "first\nbefore mid after\nsecond",
        ),
        (
            "inline-flex items",
            "<div id=\"a\">first</div><div>before <span style=\"display:inline-flex\">m1<b>m2</b>m3</span> after</div><div id=\"b\">second</div>",
            "first\nbefore m1\nm2\nm3 after\nsecond",
        ),
        (
            "inline-flex block child",
            "<div id=\"a\">first</div><div>before <span style=\"display:inline-flex\"><div>m1</div></span> after</div><div id=\"b\">second</div>",
            "first\nbefore \nm1\n after\nsecond",
        ),
        (
            "flex white space",
            "<div id=\"a\">first</div><div style=\"display:flex\">  loose   text  </div><div id=\"b\">second</div>",
            "first\nloose text\nsecond",
        ),
        (
            "flex pre",
            "<div id=\"a\">first</div><div style=\"display:flex;white-space:pre\">  a  b </div><div id=\"b\">second</div>",
            "first\n  a  b \nsecond",
        ),
        (
            "flex unselectable",
            "<div id=\"a\">first</div><div style=\"display:flex;user-select:none\">loose</div><div id=\"b\">second</div>",
            "first\nsecond",
        ),
        (
            "flex hidden",
            "<div id=\"a\">first</div><div style=\"display:flex;visibility:hidden\">loose</div><div id=\"b\">second</div>",
            "first\nsecond",
        ),
        (
            "flex icon label",
            "<div id=\"a\">first</div><div style=\"display:flex\"><span>icon</span>label</div><div id=\"b\">second</div>",
            "first\nicon\nlabel\nsecond",
        ),
        (
            "flex only white space",
            "<div id=\"a\">first</div><div style=\"display:flex\"> <div>item</div> </div><div id=\"b\">second</div>",
            "first\nitem\nsecond",
        ),
        (
            "inline-flex white space",
            "<div id=\"a\">first</div><div>before <span style=\"display:inline-flex\"> mid </span> after</div><div id=\"b\">second</div>",
            "first\nbefore mid after\nsecond",
        ),
        (
            "flex in a cell",
            "<table><tr><td id=\"a\">first</td><td><a href=\"#\" style=\"display:inline-flex\">Click</a></td><td id=\"b\">second</td></tr></table>",
            "first\tClick\tsecond",
        ),
    ]);
}

#[test]
fn what_stands_in_a_line_is_copied_where_it_stands() {
    // #2242: an inline-block, float, inline table or positioned box is
    // written between the text before it and the text after it, and only
    // when the root's selected part reaches where it stands.
    copies_as_chrome(&[
        (
            "inline-block mid",
            "<div id=\"a\">first</div><div>before <span style=\"display:inline-block\">mid</span> after</div><div id=\"b\">second</div>",
            "first\nbefore mid after\nsecond",
        ),
        (
            "float mid",
            "<div id=\"a\">first</div><div>first <span style=\"float:right\">fl</span> more</div><div id=\"b\">second</div>",
            "first\nfirst flmore\nsecond",
        ),
        (
            "float left mid",
            "<div id=\"a\">first</div><div>first <span style=\"float:left\">fl</span> more</div><div id=\"b\">second</div>",
            "first\nfirst flmore\nsecond",
        ),
        (
            "inline table",
            "<div id=\"a\">first</div><div><table style=\"display:inline-table\"><tr><td>x</td></tr></table> </div><div id=\"b\">second</div>",
            "first\nx\n \nsecond",
        ),
        (
            "inline table mid",
            "<div id=\"a\">first</div><div>before <table style=\"display:inline-table\"><tr><td>x</td></tr></table> after</div><div id=\"b\">second</div>",
            "first\nbefore  \nx\n  after\nsecond",
        ),
        (
            "starts after inline-block",
            "<div>before <span style=\"display:inline-block\">mid</span> <span id=\"a\">after</span></div><div id=\"b\">second</div>",
            "after\nsecond",
        ),
        (
            "ends before inline-block",
            "<div id=\"a\">first</div><div><span id=\"b\">before</span> <span style=\"display:inline-block\">mid</span> after</div>",
            "first\nbefore",
        ),
        (
            "ends at the space before inline-block",
            "<div id=\"a\">first</div><div><span id=\"b\">before </span><span style=\"display:inline-block\">mid</span> after</div>",
            "first\nbefore ",
        ),
        (
            "starts at the space after inline-block",
            "<div>before <span style=\"display:inline-block\">mid</span><span id=\"a\"> after</span></div><div id=\"b\">second</div>",
            " after\nsecond",
        ),
        (
            "inline-block ends the line",
            "<div id=\"a\">first</div><div>before <span style=\"display:inline-block\">mid</span> </div><div id=\"b\">second</div>",
            "first\nbefore mid\nsecond",
        ),
        (
            "ends after inline-block",
            "<div id=\"a\">first</div><div>before <span style=\"display:inline-block\">mid</span> <span id=\"b\">after</span></div>",
            "first\nbefore mid after",
        ),
        (
            "starts in inline-block",
            "<div>before <span id=\"a\" style=\"display:inline-block\">mid</span> after</div><div id=\"b\">second</div>",
            "mid after\nsecond",
        ),
        (
            "ends in inline-block",
            "<div id=\"a\">first</div><div>before <span id=\"b\" style=\"display:inline-block\">mid</span> after</div>",
            "first\nbefore mid",
        ),
        (
            "inline-block first on line",
            "<div id=\"a\">first</div><div><span style=\"display:inline-block\">mid</span> after</div><div id=\"b\">second</div>",
            "first\nmid after\nsecond",
        ),
        (
            "two inline-blocks",
            "<div id=\"a\">first</div><div><span style=\"display:inline-block\">one</span> <span style=\"display:inline-block\">two</span></div><div id=\"b\">second</div>",
            "first\none two\nsecond",
        ),
        (
            "inline-block columns",
            "<div id=\"a\">first</div><div><div style=\"display:inline-block;width:40%\">left</div> <div style=\"display:inline-block;width:40%\">right</div></div><div id=\"b\">second</div>",
            "first\nleft right\nsecond",
        ),
        (
            "inline-block columns of paragraphs",
            "<div id=\"a\">first</div><div><div style=\"display:inline-block;width:40%\"><p style=\"margin:0\">left</p></div> <div style=\"display:inline-block;width:40%\"><p style=\"margin:0\">right</p></div></div><div id=\"b\">second</div>",
            "first\nleft\n\n \nright\n\nsecond",
        ),
        (
            "inline-block columns font-size 0",
            "<div id=\"a\">first</div><div style=\"font-size:0\"> <div style=\"display:inline-block;width:40%;font-size:16px\">left</div> <div style=\"display:inline-block;width:40%;font-size:16px\">right</div> </div><div id=\"b\">second</div>",
            "first\nleft right\nsecond",
        ),
        (
            "inline-block in anonymous run",
            "<div id=\"a\">first</div><div><div>block</div>loose <span style=\"display:inline-block\">mid</span> tail</div><div id=\"b\">second</div>",
            "first\nblock\nloose mid tail\nsecond",
        ),
        (
            "inline-block in a span",
            "<div id=\"a\">first</div><div>before <b>bold <span style=\"display:inline-block\">mid</span> on</b> after</div><div id=\"b\">second</div>",
            "first\nbefore bold mid on after\nsecond",
        ),
        (
            "unselectable inline-block",
            "<div id=\"a\">first</div><div>before <span style=\"display:inline-block;user-select:none\">mid</span> after</div><div id=\"b\">second</div>",
            "first\nbefore  after\nsecond",
        ),
        (
            "inline-block block child",
            "<div id=\"a\">first</div><div>before <span style=\"display:inline-block\"><div>m1</div></span> after</div><div id=\"b\">second</div>",
            "first\nbefore \nm1\n after\nsecond",
        ),
        (
            "inline-block two blocks",
            "<div id=\"a\">first</div><div>before <span style=\"display:inline-block\"><div>m1</div><div>m2</div></span> after</div><div id=\"b\">second</div>",
            "first\nbefore \nm1\nm2\n after\nsecond",
        ),
        (
            "absolute mid",
            "<div id=\"a\">first</div><div>before <span style=\"position:absolute\">abs</span> after</div><div id=\"b\">second</div>",
            "first\nbefore absafter\nsecond",
        ),
        (
            "pictures in a line",
            "<div id=\"a\">first</div><div><img width=\"10\" height=\"10\" src=\"data:image/gif;base64,R0lGODlhAQABAAAAACw=\"> <img width=\"10\" height=\"10\" src=\"data:image/gif;base64,R0lGODlhAQABAAAAACw=\"></div><div id=\"b\">second</div>",
            "first\n \nsecond",
        ),
        (
            "float block in text",
            "<div id=\"a\">first</div><div><div style=\"float:left\">fl</div>text</div><div id=\"b\">second</div>",
            "first\nfltext\nsecond",
        ),
        (
            "inline picture mid",
            "<div id=\"a\">first</div><div>before <img width=\"10\" height=\"10\" src=\"data:image/gif;base64,R0lGODlhAQABAAAAACw=\"> after</div><div id=\"b\">second</div>",
            "first\nbefore  after\nsecond",
        ),
    ]);
}

/// #2240's cases, around the ones above: tables, `<br>`, pictures,
/// `pre-line`, `user-select: none`, right-to-left text and floats.
const CORPUS: &[(&str, &str, &str)] = &[
    (
        "grid loose",
        "<div id=\"a\">first</div><div style=\"display:grid\">loose</div><div id=\"b\">second</div>",
        "first\nloose\nsecond",
    ),
    (
        "table",
        "<table><tr><td id=\"a\">a1</td><td>a2</td></tr><tr><td>b1</td><td id=\"b\">b2</td></tr></table>",
        "a1\ta2\nb1\tb2",
    ),
    (
        "table between blocks",
        "<div id=\"a\">first</div><table><tr><td>a1</td><td>a2</td></tr></table><div id=\"b\">second</div>",
        "first\na1\ta2\nsecond",
    ),
    (
        "nested table",
        "<div id=\"a\">first</div><table><tr><td><table><tr><td>in1</td><td>in2</td></tr></table></td><td>out</td></tr></table><div id=\"b\">second</div>",
        "first\nin1\tin2\nout\nsecond",
    ),
    (
        "br",
        "<div id=\"a\">first</div><div>x<br>y<br><br>z</div><div id=\"b\">second</div>",
        "first\nx\ny\n\nz\nsecond",
    ),
    (
        "br at end",
        "<div id=\"a\">first</div><div>x<br></div><div id=\"b\">second</div>",
        "first\nx\nsecond",
    ),
    (
        "paragraphs",
        "<p id=\"a\">first</p><p>mid</p><p id=\"b\">second</p>",
        "first\n\nmid\n\nsecond",
    ),
    (
        "list",
        "<div id=\"a\">first</div><ul><li>one</li><li>two</li></ul><div id=\"b\">second</div>",
        "first\none\ntwo\nsecond",
    ),
    (
        "block picture",
        "<div id=\"a\">first</div><img style=\"display:block\" width=\"10\" height=\"10\" src=\"data:image/gif;base64,R0lGODlhAQABAAAAACw=\"><div id=\"b\">second</div>",
        "first\n\nsecond",
    ),
    (
        "pre-line",
        "<div id=\"a\">first</div><div style=\"white-space:pre-line\">x\n  y\n</div><div id=\"b\">second</div>",
        "first\nx\ny\nsecond",
    ),
    (
        "pre",
        "<div id=\"a\">first</div><pre>  x\n  y</pre><div id=\"b\">second</div>",
        "first\n  x\n  y\nsecond",
    ),
    (
        "user-select none block",
        "<div id=\"a\">first</div><div style=\"user-select:none\">skip</div><div id=\"b\">second</div>",
        "first\nsecond",
    ),
    (
        "user-select none span",
        "<div id=\"a\">first</div><div>keep <span style=\"user-select:none\">skip</span> keep</div><div id=\"b\">second</div>",
        "first\nkeep  keep\nsecond",
    ),
    (
        "rtl",
        "<div id=\"a\">first</div><div dir=\"rtl\">שלום עולם</div><div id=\"b\">second</div>",
        "first\nשלום עולם\nsecond",
    ),
    (
        "rtl mixed",
        "<div id=\"a\">first</div><div>abc שלום def</div><div id=\"b\">second</div>",
        "first\nabc שלום def\nsecond",
    ),
    (
        "float block",
        "<div id=\"a\">first</div><div style=\"float:left\">fl</div><div id=\"b\">second</div>",
        "first\nfl\nsecond",
    ),
    (
        "hidden span",
        "<div id=\"a\">first</div><div>a <span style=\"visibility:hidden\">hid</span> b</div><div id=\"b\">second</div>",
        "first\na  b\nsecond",
    ),
    (
        "display none",
        "<div id=\"a\">first</div><div style=\"display:none\">gone</div><div id=\"b\">second</div>",
        "first\nsecond",
    ),
    (
        "nbsp spacer",
        "<div id=\"a\">first</div><div>&nbsp;</div><div id=\"b\">second</div>",
        "first\n \nsecond",
    ),
    (
        "empty cell",
        "<table><tr><td id=\"a\">first</td><td></td><td id=\"b\">second</td></tr></table>",
        "first\t\tsecond",
    ),
    (
        "unselectable cell",
        "<table><tr><td id=\"a\">first</td><td style=\"user-select:none\">mid</td><td id=\"b\">second</td></tr></table>",
        "first\tsecond",
    ),
    (
        "mjml columns",
        "<table width=\"100%\"><tr><td style=\"font-size:0\"><div style=\"display:inline-block;width:100%\"><table width=\"100%\"><tr><td><div><p id=\"a\" style=\"margin:0\">first paragraph</p></div></td></tr></table></div></td></tr></table> <table width=\"100%\"><tr><td style=\"font-size:0\"> <div style=\"display:inline-block;width:48%\"><table width=\"100%\"><tr><td><div><a href=\"#\"><img style=\"display:block\" width=\"40\" height=\"20\"></a></div></td></tr><tr><td><div><p id=\"b\" style=\"margin:0\">second paragraph</p></div></td></tr></table></div> </td></tr></table>",
        "first paragraph\n\n\nsecond paragraph",
    ),
];

#[test]
fn the_2240_corpus_still_copies_as_chrome() {
    copies_as_chrome(CORPUS);
}

/// The copy of all of `#p`'s root, as a triple click on it selects it.
fn copy_whole(html: &'static str) -> String {
    let mut app = mount(html, SEL);
    let p = node(&app, "p");
    let at = super::text_selection_across_tests::at(&app, "p", 0.9);
    app.start_text_selection(p, 0, 3, false, at);
    app.text_selection_text().unwrap_or_default()
}

#[test]
fn a_root_selected_whole_has_the_boxes_at_its_ends() {
    // An offset where a box stands cannot say which side of it it is on;
    // at a root's own start or end the box is in (Chrome 156 selects a
    // paragraph's contents with them).
    let starts =
        copy_whole(r#"<div id="p"><span style="display:inline-block">mid</span> after</div>"#);
    let ends =
        copy_whole(r#"<div id="p">before <span style="display:inline-block">mid</span></div>"#);
    assert_eq!(
        (starts.as_str(), ends.as_str()),
        ("mid after", "before mid")
    );
}

/// The review of #2249's cases.
const REVIEW: &[(&str, &str, &str)] = &[
    (
        "nested ib",
        "<div id=\"a\">first</div><div>x <span style=\"display:inline-block\">o1 <span style=\"display:inline-block\">in</span> o2</span> y</div><div id=\"b\">second</div>",
        "first\nx o1 in o2 y\nsecond",
    ),
    (
        "nested ib start inner",
        "<div>x <span style=\"display:inline-block\">o1 <span id=\"a\" style=\"display:inline-block\">in</span> o2</span> y</div><div id=\"b\">second</div>",
        "in o2 y\nsecond",
    ),
    (
        "nested ib end inner",
        "<div id=\"a\">first</div><div>x <span style=\"display:inline-block\">o1 <span id=\"b\" style=\"display:inline-block\">in</span> o2</span> y</div>",
        "first\nx o1 in",
    ),
    (
        "nested ib start outer text",
        "<div>x <span style=\"display:inline-block\"><span id=\"a\">o1</span> <span style=\"display:inline-block\">in</span> o2</span> y</div><div id=\"b\">second</div>",
        "o1 in o2 y\nsecond",
    ),
    (
        "nested ib end outer text",
        "<div id=\"a\">first</div><div>x <span style=\"display:inline-block\">o1 <span style=\"display:inline-block\">in</span> <span id=\"b\">o2</span></span> y</div>",
        "first\nx o1 in o2",
    ),
    (
        "same line a before b after",
        "<div><span id=\"a\">x</span> <span style=\"display:inline-block\">mid</span> <span id=\"b\">y</span></div>",
        "x mid y",
    ),
    (
        "a in ib b after same para",
        "<div>x <span id=\"a\" style=\"display:inline-block\">mid</span> <span id=\"b\">y</span> z</div>",
        "mid y",
    ),
    (
        "a before b in ib same para",
        "<div>x <span id=\"a\">w</span> <span id=\"b\" style=\"display:inline-block\">mid</span> y</div>",
        "w mid",
    ),
    (
        "a b in same ib",
        "<div>x <span style=\"display:inline-block\"><span id=\"a\">m1</span> <span id=\"b\">m2</span> m3</span> y</div>",
        "m1 m2",
    ),
    (
        "a in ib1 b in ib2",
        "<div>x <span id=\"a\" style=\"display:inline-block\">one</span> mid <span id=\"b\" style=\"display:inline-block\">two</span> y</div>",
        "one mid two",
    ),
    (
        "a in ib1 b in later block",
        "<div>x <span id=\"a\" style=\"display:inline-block\">one</span> mid <span style=\"display:inline-block\">two</span> y</div><div id=\"b\">second</div>",
        "one mid two y\nsecond",
    ),
    (
        "ib holds p",
        "<div id=\"a\">first</div><div>before <span style=\"display:inline-block\"><p>para</p></span> after</div><div id=\"b\">second</div>",
        "first\nbefore \npara\n\n after\nsecond",
    ),
    (
        "ib holds two p start second",
        "<div>before <span style=\"display:inline-block\"><p>p1</p><p id=\"a\">p2</p></span> after</div><div id=\"b\">second</div>",
        "p2\n\n after\nsecond",
    ),
    (
        "ib holds table",
        "<div id=\"a\">first</div><div>before <span style=\"display:inline-block\"><table><tr><td>c1</td><td>c2</td></tr></table></span> after</div><div id=\"b\">second</div>",
        "first\nbefore \nc1\tc2\n after\nsecond",
    ),
    (
        "ib holds table start in cell",
        "<div>before <span style=\"display:inline-block\"><table><tr><td>c1</td><td id=\"a\">c2</td></tr></table></span> after</div><div id=\"b\">second</div>",
        "c2\n after\nsecond",
    ),
    (
        "inline-flex mixed",
        "<div id=\"a\">first</div><div>before <span style=\"display:inline-flex\">t1 <b>bold</b> <div>blk</div> t2</span> after</div><div id=\"b\">second</div>",
        "first\nbefore t1\nbold\nblk\nt2 after\nsecond",
    ),
    (
        "inline-flex holds ib",
        "<div id=\"a\">first</div><div>before <span style=\"display:inline-flex\"><span style=\"display:inline-block\">ib</span>txt</span> after</div><div id=\"b\">second</div>",
        "first\nbefore \nib\ntxt after\nsecond",
    ),
    (
        "inline-flex start inside item",
        "<div>before <span style=\"display:inline-flex\">t1 <b id=\"a\">bold</b> t2</span> after</div><div id=\"b\">second</div>",
        "bold\nt2 after\nsecond",
    ),
    (
        "grid loose",
        "<div id=\"a\">first</div><div style=\"display:grid;grid-template-columns:1fr 1fr\">g1<div>g2</div>g3 <span>g4</span></div><div id=\"b\">second</div>",
        "first\ng1\ng2\ng3\ng4\nsecond",
    ),
    (
        "grid cells",
        "<div id=\"a\">first</div><div style=\"display:grid;grid-template-columns:1fr 1fr\"><div>c1</div><div>c2</div><div>c3</div></div><div id=\"b\">second</div>",
        "first\nc1\nc2\nc3\nsecond",
    ),
    (
        "grid start in item",
        "<div style=\"display:grid;grid-template-columns:1fr 1fr\"><div id=\"a\">c1</div>loose<div>c3</div></div><div id=\"b\">second</div>",
        "c1\nloose\nc3\nsecond",
    ),
    (
        "flex end in item",
        "<div id=\"a\">first</div><div style=\"display:flex\">loose<div id=\"b\">item</div>tail</div>",
        "first\nloose\nitem",
    ),
    (
        "float at line start",
        "<div id=\"a\">first</div><div><span style=\"float:left\">fl</span>text after</div><div id=\"b\">second</div>",
        "first\nfltext after\nsecond",
    ),
    (
        "float at line end",
        "<div id=\"a\">first</div><div>text before<span style=\"float:right\">fr</span></div><div id=\"b\">second</div>",
        "first\ntext beforefr\nsecond",
    ),
    (
        "float block div mid",
        "<div id=\"a\">first</div><div>text <div style=\"float:right;width:100px\">fdiv</div> more</div><div id=\"b\">second</div>",
        "first\ntext fdivmore\nsecond",
    ),
    (
        "start after float",
        "<div><span style=\"float:left\">fl</span><span id=\"a\">after</span></div><div id=\"b\">second</div>",
        "after\nsecond",
    ),
    (
        "end before float",
        "<div id=\"a\">first</div><div><span id=\"b\">before</span><span style=\"float:right\">fr</span></div>",
        "first\nbefore",
    ),
    (
        "start in float",
        "<div>text <span id=\"a\" style=\"float:right\">fl</span> more</div><div id=\"b\">second</div>",
        "flmore\nsecond",
    ),
    (
        "abs in line",
        "<div id=\"a\">first</div><div style=\"position:relative\">before <span style=\"position:absolute;left:0;top:30px\">abs</span> after</div><div id=\"b\">second</div>",
        "first\nbefore absafter\nsecond",
    ),
    (
        "fixed in line",
        "<div id=\"a\">first</div><div>before <span style=\"position:fixed;left:0;top:300px\">fix</span> after</div><div id=\"b\">second</div>",
        "first\nbefore fixafter\nsecond",
    ),
    (
        "abs div in line",
        "<div id=\"a\">first</div><div style=\"position:relative\">before <div style=\"position:absolute;top:40px\">absdiv</div> after</div><div id=\"b\">second</div>",
        "first\nbefore absdivafter\nsecond",
    ),
    (
        "abs at line start",
        "<div id=\"a\">first</div><div style=\"position:relative\"><span style=\"position:absolute;top:30px\">abs</span>after</div><div id=\"b\">second</div>",
        "first\nabsafter\nsecond",
    ),
    (
        "contents in flex",
        "<div id=\"a\">first</div><div style=\"display:flex\"><div style=\"display:contents\">c1 <span>c2</span></div>loose</div><div id=\"b\">second</div>",
        "first\nc1\nc2\nloose\nsecond",
    ),
    (
        "contents spans in flex",
        "<div id=\"a\">first</div><div style=\"display:flex\"><span style=\"display:contents\">x</span><span style=\"display:contents\"><b>y</b></span></div><div id=\"b\">second</div>",
        "first\nx\ny\nsecond",
    ),
    (
        "rtl ib",
        "<div id=\"a\">first</div><div dir=\"rtl\">שלום <span style=\"display:inline-block\">mid</span> עולם</div><div id=\"b\">second</div>",
        "first\nשלום mid עולם\nsecond",
    ),
    (
        "rtl start in ib",
        "<div dir=\"rtl\">שלום <span id=\"a\" style=\"display:inline-block\">mid</span> עולם</div><div id=\"b\">second</div>",
        "mid עולם\nsecond",
    ),
    (
        "rtl latin ib",
        "<div id=\"a\">first</div><div dir=\"rtl\">abc <span style=\"display:inline-block\">אב</span> def</div><div id=\"b\">second</div>",
        "first\nabc אב def\nsecond",
    ),
    (
        "unsel ib selectable child",
        "<div id=\"a\">first</div><div>before <span style=\"display:inline-block;user-select:none\">no <span style=\"user-select:text\">yes</span></span> after</div><div id=\"b\">second</div>",
        "first\nbefore yes after\nsecond",
    ),
    (
        "unsel ib nested ib",
        "<div id=\"a\">first</div><div>before <span style=\"display:inline-block;user-select:none\">no <span style=\"display:inline-block\">nest</span></span> after</div><div id=\"b\">second</div>",
        "first\nbefore  after\nsecond",
    ),
    (
        "unsel para selectable ib",
        "<div id=\"a\">first</div><div style=\"user-select:none\">before <span style=\"display:inline-block;user-select:text\">mid</span> after</div><div id=\"b\">second</div>",
        "first\nmid\nsecond",
    ),
    (
        "unsel ib between ibs",
        "<div id=\"a\">first</div><div><span style=\"display:inline-block\">one</span> <span style=\"display:inline-block;user-select:none\">no</span> <span style=\"display:inline-block\">two</span></div><div id=\"b\">second</div>",
        "first\none  two\nsecond",
    ),
    (
        "br around ib",
        "<div id=\"a\">first</div><div>before<br><span style=\"display:inline-block\">mid</span><br>after</div><div id=\"b\">second</div>",
        "first\nbefore\nmid\nafter\nsecond",
    ),
    (
        "br inside ib",
        "<div id=\"a\">first</div><div>before <span style=\"display:inline-block\">m1<br>m2</span> after</div><div id=\"b\">second</div>",
        "first\nbefore m1\nm2 after\nsecond",
    ),
    (
        "br then float",
        "<div id=\"a\">first</div><div>before<br><span style=\"float:left\">fl</span>after</div><div id=\"b\">second</div>",
        "first\nbefore\nflafter\nsecond",
    ),
    (
        "ib then br at end",
        "<div id=\"a\">first</div><div>before <span style=\"display:inline-block\">mid</span><br></div><div id=\"b\">second</div>",
        "first\nbefore mid\nsecond",
    ),
    (
        "br then ib at end",
        "<div id=\"a\">first</div><div>before<br><span style=\"display:inline-block\">mid</span></div><div id=\"b\">second</div>",
        "first\nbefore\nmid\nsecond",
    ),
    (
        "pre-wrap ib",
        "<div id=\"a\">first</div><div style=\"white-space:pre-wrap\">  before  <span style=\"display:inline-block\">mid</span>  after  </div><div id=\"b\">second</div>",
        "first\n  before  mid  after  \nsecond",
    ),
    (
        "pre-wrap two ibs",
        "<div id=\"a\">first</div><div style=\"white-space:pre-wrap\"><span style=\"display:inline-block\">one</span>   <span style=\"display:inline-block\">two</span></div><div id=\"b\">second</div>",
        "first\none   two\nsecond",
    ),
    (
        "pre-wrap only ib with spaces",
        "<div id=\"a\">first</div><div style=\"white-space:pre-wrap\">  <span style=\"display:inline-block\">mid</span>  </div><div id=\"b\">second</div>",
        "first\n  mid  \nsecond",
    ),
    (
        "pre-line ib",
        "<div id=\"a\">first</div><div style=\"white-space:pre-line\">  a  <span style=\"display:inline-block\">mid</span>  b  </div><div id=\"b\">second</div>",
        "first\na mid b\nsecond",
    ),
    (
        "normal only ib with spaces",
        "<div id=\"a\">first</div><div>  <span style=\"display:inline-block\">mid</span>  </div><div id=\"b\">second</div>",
        "first\nmid\nsecond",
    ),
    (
        "empty ib between text",
        "<div id=\"a\">first</div><div>a<span style=\"display:inline-block;width:10px;height:10px\"></span>b</div><div id=\"b\">second</div>",
        "first\nab\nsecond",
    ),
    (
        "empty ibs spaced",
        "<div id=\"a\">first</div><div><span style=\"display:inline-block;width:10px;height:10px\"></span> <span style=\"display:inline-block;width:10px;height:10px\"></span></div><div id=\"b\">second</div>",
        "first\n \nsecond",
    ),
    (
        "ib of spaces",
        "<div id=\"a\">first</div><div>before<span style=\"display:inline-block\"> </span>after</div><div id=\"b\">second</div>",
        "first\nbeforeafter\nsecond",
    ),
    (
        "img and ib",
        "<div id=\"a\">first</div><div><img style=\"width:10px;height:10px\"> <span style=\"display:inline-block\">mid</span></div><div id=\"b\">second</div>",
        "first\n mid\nsecond",
    ),
    (
        "ib in anon run start",
        "<div><div>blk</div>loose <span id=\"a\" style=\"display:inline-block\">mid</span> tail</div><div id=\"b\">second</div>",
        "mid tail\nsecond",
    ),
    (
        "anon run two ibs",
        "<div id=\"a\">first</div><div>loose <span style=\"display:inline-block\">ib</span><div>blk</div>tail <span style=\"display:inline-block\">ib2</span></div><div id=\"b\">second</div>",
        "first\nloose ib\nblk\ntail ib2\nsecond",
    ),
    (
        "ib in cell",
        "<table><tr><td id=\"a\">c1</td><td><span style=\"display:inline-block\">ib</span> t</td><td id=\"b\">c3</td></tr></table>",
        "c1\tib t\tc3",
    ),
    (
        "table regression",
        "<div id=\"a\">first</div><table><tr><td>a1</td><td>a2</td></tr><tr><td>b1</td><td>b2</td></tr></table><div id=\"b\">second</div>",
        "first\na1\ta2\nb1\tb2\nsecond",
    ),
    (
        "table start in cell",
        "<table><tr><td>a1</td><td id=\"a\">a2</td></tr><tr><td>b1</td><td id=\"b\">b2</td></tr></table>",
        "a2\nb1\tb2",
    ),
    (
        "br regression",
        "<div><span id=\"a\">l1</span><br>l2<br><br>l3 <span id=\"b\">end</span></div>",
        "l1\nl2\n\nl3 end",
    ),
    (
        "picture regression",
        "<div id=\"a\">first</div><div><img style=\"width:10px;height:10px\"></div><div id=\"b\">second</div>",
        "first\n\nsecond",
    ),
    (
        "pre-line regression",
        "<div id=\"a\">first</div><div style=\"white-space:pre-line\">  x  &#10;  y  </div><div id=\"b\">second</div>",
        "first\nx\ny\nsecond",
    ),
    (
        "nested table regression",
        "<div id=\"a\">first</div><table><tr><td><table><tr><td>i1</td><td>i2</td></tr></table></td><td>o2</td></tr></table><div id=\"b\">second</div>",
        "first\ni1\ti2\no2\nsecond",
    ),
    (
        "inline-table at start",
        "<div id=\"a\">first</div><div><table style=\"display:inline-table\"><tr><td>x</td></tr></table>after</div><div id=\"b\">second</div>",
        "first\nx\n after\nsecond",
    ),
    (
        "two inline-tables",
        "<div id=\"a\">first</div><div><table style=\"display:inline-table\"><tr><td>x</td></tr></table><table style=\"display:inline-table\"><tr><td>y</td></tr></table></div><div id=\"b\">second</div>",
        "first\nx\n  \ny\n \nsecond",
    ),
    (
        "start in inline-table",
        "<div>before <table style=\"display:inline-table\"><tr><td id=\"a\">x</td></tr></table> after</div><div id=\"b\">second</div>",
        "x\n  after\nsecond",
    ),
    (
        "a in ib b after then more",
        "<div>x <span id=\"a\" style=\"display:inline-block\">mid</span> <span id=\"b\">y</span> z</div><div>after</div><div style=\"display:flex\">leaf</div><table><tr><td>t1</td><td>t2</td></tr></table>",
        "mid y",
    ),
    (
        "a in ib b after in span tail",
        "<div>x <span id=\"a\" style=\"display:inline-block\">mid</span> w <b id=\"b\">y</b> z</div><div>next</div>",
        "mid w y",
    ),
    (
        "a in nested ib b after in outer ib",
        "<div>x <span style=\"display:inline-block\">o1 <span id=\"a\" style=\"display:inline-block\">in</span> <span id=\"b\">o2</span> o3</span> y</div><div>next</div>",
        "in o2",
    ),
    (
        "a in ib b at para end",
        "<div>x <span id=\"a\" style=\"display:inline-block\">mid</span> <span id=\"b\">y</span></div><div>next</div>",
        "mid y",
    ),
    (
        "start after ib at root start",
        "<div><span style=\"display:inline-block\">mid</span><span id=\"a\">after</span></div><div id=\"b\">second</div>",
        "after\nsecond",
    ),
    (
        "end before ib at root end",
        "<div id=\"a\">first</div><div><span id=\"b\">before</span><span style=\"display:inline-block\">mid</span></div>",
        "first\nbefore",
    ),
    (
        "flex leaf before start in same flex",
        "<div style=\"display:flex\">leaf<div id=\"a\">item</div></div><div id=\"b\">second</div>",
        "item\nsecond",
    ),
    (
        "flex leaf after end in same flex",
        "<div id=\"a\">first</div><div style=\"display:flex\"><div id=\"b\">item</div>leaf</div><div>tail</div>",
        "first\nitem",
    ),
    (
        "inline-flex leaf before start same para",
        "<div><span style=\"display:inline-flex\">leaf</span> <span id=\"a\">after</span></div><div id=\"b\">second</div>",
        "after\nsecond",
    ),
    (
        "inline-flex leaf after end same para",
        "<div id=\"a\">first</div><div><span id=\"b\">before</span> <span style=\"display:inline-flex\">leaf</span></div><div>tail</div>",
        "first\nbefore",
    ),
    (
        "unsel inline-flex leaf",
        "<div id=\"a\">first</div><div>before <span style=\"display:inline-flex;user-select:none\">leaf</span> after</div><div id=\"b\">second</div>",
        "first\nbefore  after\nsecond",
    ),
    (
        "hidden child of visible flex",
        "<div id=\"a\">first</div><div style=\"display:flex\"><span style=\"visibility:hidden\">h</span>vis</div><div id=\"b\">second</div>",
        "first\nvis\nsecond",
    ),
    (
        "visible leaf of hidden flex",
        "<div id=\"a\">first</div><div style=\"display:flex;visibility:hidden\"><span style=\"visibility:visible\">v</span>leaf</div><div id=\"b\">second</div>",
        "first\nv\nsecond",
    ),
    (
        "flex nowrap pre-line leaf",
        "<div id=\"a\">first</div><div style=\"display:flex;white-space:pre-line\">  a  &#10;  b  </div><div id=\"b\">second</div>",
        "first\na\nb\nsecond",
    ),
    (
        "pre-line regression2",
        "<div id=\"a\">first</div><div style=\"white-space:pre-line\">  x  &#10;  y  </div><div id=\"b\">second</div>",
        "first\nx\ny\nsecond",
    ),
    (
        "ib in anon run start2",
        "<div><div>blk</div>loose <span id=\"a\" style=\"display:inline-block\">mid</span> tail</div><div id=\"b\">second</div>",
        "mid tail\nsecond",
    ),
    (
        "abs before start same para",
        "<div style=\"position:relative\"><span style=\"position:absolute;top:30px\">abs</span>x <span id=\"a\">after</span></div><div id=\"b\">second</div>",
        "after\nsecond",
    ),
];

/// The cases of [`REVIEW`] named `names`, checked as [`copies_as_chrome`].
fn review_cases(names: &[&str]) {
    let cases: Vec<(&str, &'static str, &str)> = names
        .iter()
        .map(|&n| {
            *REVIEW
                .iter()
                .find(|c| c.0 == n)
                .unwrap_or_else(|| panic!("no case {n}"))
        })
        .collect();
    copies_as_chrome(&cases);
}

#[test]
fn a_selection_from_an_inline_block_into_its_paragraph_is_in_document_order() {
    // Review of #2249: the root order puts a paragraph before the
    // inline-blocks in it, so a selection from inside one to the text after
    // it took the paragraph as its start, refused the box and ran on to the
    // end of the document. The ends are ordered by where the box stands.
    review_cases(&[
        "a in ib b after same para",
        "a in ib b at para end",
        "a in ib b after then more",
        "a in ib b after in span tail",
        "a in nested ib b after in outer ib",
    ]);
}

/// The work ([`super::text_selection_copy::WORK`]) a copy of a paragraph of
/// `spans` spans, between two blocks, takes.
fn copy_work(spans: usize) -> u64 {
    let mut html = String::from(r#"<div id="a">first</div><div>"#);
    for i in 0..spans {
        html.push_str(&format!("<span>w{i} </span>"));
    }
    html.push_str(r#"</div><div id="b">last</div>"#);
    let html: &'static str = Box::leak(html.into_boxed_str());
    let mut app = select_range(html);
    let work = || super::text_selection_copy::WORK.with(|w| w.get());
    let before = work();
    let t = app.text_selection_text().unwrap_or_default();
    assert!(t.ends_with("\nlast"), "{t:?}");
    work() - before
}

#[test]
fn a_copy_is_linear_in_its_roots_text() {
    // Review of #2249: writing a root's part per text node rebuilt the
    // root's drawn ranges for each one, so a paragraph of 4,000 spans took
    // 2 s. Four times the spans must be about four times the work.
    let (small, big) = (copy_work(250), copy_work(1000));
    assert!(
        big <= 5 * small,
        "{small} steps for 250 spans, {big} for 1000"
    );
}

#[test]
fn a_selection_beside_a_box_at_its_roots_edge_leaves_the_box_out() {
    // Review of #2249: a selection that ends in the text before a box at
    // its root's end, or starts in the text after a box at its root's
    // start, stands on the text's side of the box (Chrome 156). Only a
    // selection made whole (a triple click, Select All) takes such a box.
    review_cases(&[
        "start after float",
        "end before float",
        "start after ib at root start",
        "end before ib at root end",
    ]);
}

#[test]
fn selectable_text_in_an_unselectable_box_is_copied() {
    // Review of #2249: `user-select: text` inside a `user-select: none`
    // inline-block is selectable (Chrome 156); the copy passed the whole
    // box because the box's root could not be selected.
    review_cases(&[
        "unsel ib selectable child",
        "unsel ib nested ib",
        "unsel para selectable ib",
        "unsel ib between ibs",
    ]);
}

/// The review's cases whose markup holds `&#10;`, which `set_inner_html`
/// does not decode yet (#2239): Chrome reads a line break there.
const NOT_DECODED: &[&str] = &[
    "pre-line regression",
    "flex nowrap pre-line leaf",
    "pre-line regression2",
];

#[test]
fn the_review_corpus_copies_as_chrome() {
    // The review of #2249's 84 cases against Chrome 156, all but the three
    // the HTML reader cannot give the copy yet.
    let cases: Vec<(&str, &'static str, &str)> = REVIEW
        .iter()
        .copied()
        .filter(|c| !NOT_DECODED.contains(&c.0))
        .collect();
    copies_as_chrome(&cases);
}
