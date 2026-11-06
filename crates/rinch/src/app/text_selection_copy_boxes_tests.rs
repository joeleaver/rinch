//! Copies of text in flex containers (#2241) and around atomic inlines
//! (#2242), and a corpus of #2240's cases, against Chrome 156: each case's
//! string is `getSelection().toString()` after `setBaseAndExtent(#a, 0, #b,
//! #b.childNodes.length)` on the same markup, in the same 600px root with
//! `font: 16px/20px sans-serif`.

use super::TextSelection;
use super::text_selection_across_tests::{mount, node};
use super::text_selection_chrome_tests::SEL;

/// The IFC root that lays out text node `text`, and where its text stands
/// in the root's.
fn root_of(tree: &rinch_dom::NodeTree, text: u64) -> (u64, usize, usize) {
    let mut id = text;
    let root = loop {
        let n = tree.get(id).expect("in the tree");
        if let Some(b) = n.run_box {
            break b;
        }
        if id != text && n.text_layout.is_some() {
            break id;
        }
        id = n.parent.expect("a root above");
    };
    let r = tree.get(root).unwrap().text_layout.as_ref().unwrap();
    let range = r
        .text_ranges
        .iter()
        .find(|r| r.node_id == text)
        .expect("the text is the root's");
    (root, range.flat_start, range.flat_end)
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
    let mut app = mount(html, SEL);
    let (a, b) = (node(&app, "a"), node(&app, "b"));
    let sel = {
        let d = app.doc.as_ref().unwrap().borrow();
        let (mut ta, mut tb) = (Vec::new(), Vec::new());
        texts(&d.tree, a, &mut ta);
        texts(&d.tree, b, &mut tb);
        let (ra, sa, _) = root_of(&d.tree, ta[0]);
        let (rb, _, eb) = root_of(&d.tree, *tb.last().unwrap());
        let mut sel = TextSelection::at(ra, sa);
        sel.focus_node = rb;
        sel.focus_offset = eb;
        sel
    };
    app.text_selection = Some(sel);
    app.text_selection_text().unwrap_or_default()
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

/// The copy of all of `#p`'s root, as a triple click selects it.
fn copy_whole(html: &'static str) -> String {
    let mut app = mount(html, SEL);
    let p = node(&app, "p");
    let len = {
        let d = app.doc.as_ref().unwrap().borrow();
        d.tree
            .get(p)
            .unwrap()
            .text_layout
            .as_ref()
            .unwrap()
            .text_content
            .len()
    };
    let mut sel = TextSelection::at(p, 0);
    sel.focus_offset = len;
    app.text_selection = Some(sel);
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
