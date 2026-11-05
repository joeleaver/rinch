//! The layout review of #2213: each history compared with a fresh layout of
//! the final document, and the shapes Chrome was measured on (Chrome 153).
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const CSS: &str =
    "body { margin: 0; font-family: sans-serif; font-size: 16px; line-height: 20px; }";

fn doc_at(html: &str, w: f32) -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css(CSS);
    let body = d.body();
    d.set_inner_html(body, html);
    d.resolve_layout(w, 800.0);
    d
}
fn doc(html: &str) -> RinchDocument {
    doc_at(html, 600.0)
}
fn q(d: &RinchDocument, id: &str) -> NodeId {
    d.query_selector(&format!("#{id}")).expect(id)
}
fn abs(d: &RinchDocument, id: &str) -> (f32, f32, f32, f32) {
    let n = q(d, id);
    let mut cur = Some(n.0);
    let l = d.tree.get(n.0).unwrap().layout;
    let (mut x, mut y) = (0.0, 0.0);
    while let Some(i) = cur {
        let node = d.tree.get(i).unwrap();
        x += node.layout.x;
        y += node.layout.y;
        cur = node.parent;
    }
    (x, y, l.width, l.height)
}
/// Append the nodes `html` parses to under `parent`.
fn add(d: &mut RinchDocument, parent: NodeId, html: &str) {
    let tmp = d.create_element("div");
    d.set_inner_html(tmp, html);
    for c in d.get_children(tmp) {
        d.append_child(parent, c);
    }
}
/// Compare every `ids` box of the edited document with a fresh one of `html`.
fn same(d: &RinchDocument, html: &str, w: f32, ids: &[&str], step: &str) -> Vec<String> {
    let fresh = doc_at(html, w);
    let mut bad = vec![];
    for id in ids {
        let (a, b) = (abs(d, id), abs(&fresh, id));
        if a != b {
            bad.push(format!("{step}: #{id} edited {a:?} fresh {b:?}"));
        }
    }
    bad
}
fn report(bad: Vec<String>) {
    for b in &bad {
        eprintln!("MISMATCH {b}");
    }
    assert!(bad.is_empty(), "{} mismatches", bad.len());
}

const ROW: &str = "<div id=r style='display:table-row'><div id=c1 style='display:table-cell;width:50px;height:10px'></div><div id=c2 style='display:table-cell;width:70px;height:10px'></div></div>";
const P: &str = "<div id=p style='height:12px'></div>";

fn host(inner: &str, host_style: &str) -> String {
    format!(
        "<div id=w style='width:300px'><div id=h style='{host_style}'>{inner}</div><div id=after style='height:5px'></div></div>"
    )
}

#[test]
fn f_host_loses_its_rows() {
    let mut d = doc(&host(ROW, "background:red"));
    eprintln!("start h={:?}", abs(&d, "h"));
    let r = q(&d, "r");
    let h = q(&d, "h");
    d.remove_child(h, r);
    add(&mut d, h, P);
    d.resolve_layout(610.0, 800.0);
    let mut bad = same(
        &d,
        &host(P, "background:red"),
        610.0,
        &["h", "p", "after"],
        "rows->p",
    );
    d.set_attribute(h, "style", "background:blue");
    d.resolve_layout(620.0, 800.0);
    bad.extend(same(
        &d,
        &host(P, "background:blue"),
        620.0,
        &["h", "p", "after"],
        "host restyled",
    ));
    report(bad);
}

#[test]
fn f_host_gains_then_loses_a_row() {
    let mut d = doc(&host(P, ""));
    let h = q(&d, "h");
    add(&mut d, h, ROW);
    d.resolve_layout(610.0, 800.0);
    let mut bad = same(
        &d,
        &host(&format!("{P}{ROW}"), ""),
        610.0,
        &["h", "p", "c1", "c2", "after"],
        "p+row",
    );
    let r = q(&d, "r");
    d.remove_child(h, r);
    d.resolve_layout(620.0, 800.0);
    bad.extend(same(
        &d,
        &host(P, ""),
        620.0,
        &["h", "p", "after"],
        "row removed again",
    ));
    report(bad);
}

#[test]
fn f_row_display_toggled() {
    // r a block with cell children (so r hosts); then a row (h hosts); then back.
    let blockrow = ROW.replace("display:table-row", "display:block");
    let mut d = doc(&host(&blockrow, ""));
    let r = q(&d, "r");
    d.set_attribute(r, "style", "display:table-row");
    d.resolve_layout(610.0, 800.0);
    let mut bad = same(
        &d,
        &host(ROW, ""),
        610.0,
        &["h", "r", "c1", "c2", "after"],
        "block->row",
    );
    d.set_attribute(r, "style", "display:block");
    d.resolve_layout(620.0, 800.0);
    bad.extend(same(
        &d,
        &host(&blockrow, ""),
        620.0,
        &["h", "r", "c1", "c2", "after"],
        "row->block",
    ));
    d.set_attribute(r, "style", "display:none");
    d.resolve_layout(630.0, 800.0);
    bad.extend(same(
        &d,
        &host(&ROW.replace("display:table-row", "display:none"), ""),
        630.0,
        &["h", "after"],
        "row->none",
    ));
    report(bad);
}

#[test]
fn f_host_display_toggled() {
    let mut d = doc(&host(ROW, ""));
    let h = q(&d, "h");
    d.set_attribute(h, "style", "display:flex");
    d.resolve_layout(610.0, 800.0);
    let mut bad = same(
        &d,
        &host(ROW, "display:flex"),
        610.0,
        &["h", "c1", "c2", "after"],
        "host->flex",
    );
    d.set_attribute(h, "style", "display:block");
    d.resolve_layout(620.0, 800.0);
    bad.extend(same(
        &d,
        &host(ROW, "display:block"),
        620.0,
        &["h", "c1", "c2", "after"],
        "host->block",
    ));
    report(bad);
}

#[test]
fn f_host_moved() {
    let html = format!(
        "<div id=a style='width:300px'>{}</div><div id=b style='width:500px;padding-left:7px'></div>",
        host(ROW, "")
    );
    let mut d = doc(&html);
    let w = q(&d, "w");
    let b = q(&d, "b");
    d.append_child(b, w);
    d.resolve_layout(610.0, 800.0);
    let fin = format!(
        "<div id=a style='width:300px'></div><div id=b style='width:500px;padding-left:7px'>{}</div>",
        host(ROW, "")
    );
    report(same(&d, &fin, 610.0, &["h", "c1", "c2", "after"], "moved"));
}

fn host_table(a: &str, w: &str) -> String {
    format!(
        "<div style='width:500px'><table id=t style='display:block;{w}border-spacing:0'><tbody><tr>
        <td id=a style='padding:0'>{a}</td><td id=b style='padding:0'>xx yy</td></tr></tbody></table><div id=after style='height:3px'></div></div>"
    )
}

#[test]
fn f_host_cell_edit_updates_columns() {
    for w in ["", "width:400px;"] {
        let mut d = doc(&host_table("short", w));
        let a = q(&d, "a");
        let text = d.get_children(a)[0];
        let long = "a much longer text for the first column";
        d.set_text_content(text, long);
        d.resolve_layout(610.0, 800.0);
        let mut bad = same(
            &d,
            &host_table(long, w),
            610.0,
            &["t", "a", "b", "after"],
            &format!("edit [{w}]"),
        );
        d.set_text_content(text, "short");
        d.resolve_layout(620.0, 800.0);
        bad.extend(same(
            &d,
            &host_table("short", w),
            620.0,
            &["t", "a", "b", "after"],
            &format!("edit back [{w}]"),
        ));
        report(bad);
    }
}

#[test]
fn f_host_vertical_align_shift_applies_once() {
    let html = "<div style='width:500px'><div id=h><div style='display:table-row'>
        <div id=m style='display:table-cell;vertical-align:middle'><div id=k style='height:10px;width:30px'></div><span id=s style='display:inline-block;width:20px;height:10px'></span></div>
        <div style='display:table-cell'><div style='height:100px;width:40px'></div></div></div></div></div>";
    let mut d = doc(html);
    let h = q(&d, "h");
    let mut bad = vec![];
    for (i, w) in [610.0, 620.0, 630.0].into_iter().enumerate() {
        d.set_attribute(h, "data-x", &i.to_string());
        d.resolve_layout(w, 800.0);
        bad.extend(same(
            &d,
            html,
            w,
            &["h", "m", "k", "s"],
            &format!("layout {i}"),
        ));
    }
    eprintln!("k={:?} s={:?}", abs(&d, "k"), abs(&d, "s"));
    report(bad);
}

fn aligned(align: &str) -> String {
    format!(
        "<table style='width:400px;border-spacing:0'><tr><td id=c {align} style='padding:0'>
        <table id=in style='border-spacing:0'><tr><td style='padding:0'><div style='width:100px;height:10px'></div></td></tr></table>
        <div id=d style='width:50px;height:10px'></div></td></tr></table>"
    )
}

#[test]
fn e_align_changed_after_layout() {
    let mut d = doc(&aligned("align=center"));
    let c = q(&d, "c");
    d.set_attribute(c, "align", "right");
    d.resolve_layout(610.0, 800.0);
    let mut bad = same(
        &d,
        &aligned("align=right"),
        610.0,
        &["in", "d"],
        "center->right",
    );
    d.remove_attribute(c, "align");
    d.resolve_layout(620.0, 800.0);
    bad.extend(same(&d, &aligned(""), 620.0, &["in", "d"], "removed"));
    d.set_attribute(c, "align", "center");
    d.resolve_layout(630.0, 800.0);
    bad.extend(same(
        &d,
        &aligned("align=center"),
        630.0,
        &["in", "d"],
        "added",
    ));
    report(bad);
}

#[test]
fn e_chrome_shapes() {
    // Chrome: -webkit-center centres a max-width block (its used width 200).
    let d = doc(
        "<table style='width:400px;border-spacing:0'><tr><td align=center style='padding:0'>
        <div id=m style='max-width:200px;height:10px'></div></td></tr></table>",
    );
    assert_eq!(
        abs(&d, "m").0,
        100.0,
        "a max-width block under align=center"
    );
    // Chrome: unequal margins still centre the margin box: 20 + (400-120)/2 = 160.
    let d = doc(
        "<div align=center style='width:400px'><div id=u style='width:100px;margin-left:20px;height:10px'></div></div>",
    );
    assert_eq!(
        abs(&d, "u").0,
        160.0,
        "unequal margins: the margin box is centred"
    );
    // Chrome: text centred in a div align=center.
    let d = doc(
        "<div align=center style='width:400px'><span id=s style='display:inline-block;width:20px;height:10px'></span></div>",
    );
    assert_eq!(abs(&d, "s").0, 190.0, "inline content in div align=center");
    // Inherited through a plain div.
    let d = doc(
        "<div align=center style='width:400px'><div><div id=g style='width:100px;height:10px'></div></div></div>",
    );
    assert_eq!(abs(&d, "g").0, 150.0, "inherited through a plain div");
    // Reset at a table, and in its cells.
    let d = doc(
        "<div align=center style='width:400px'><table id=t style='width:200px;border-spacing:0'><tr><td style='padding:0'><div id=k style='width:50px;height:10px'></div></td></tr></table></div>",
    );
    assert_eq!(
        (abs(&d, "t").0, abs(&d, "k").0),
        (100.0, 100.0),
        "reset at a table"
    );
    // align=right on a div.
    let d = doc(
        "<div align=right style='width:400px'><div id=rr style='width:100px;height:10px'></div></div>",
    );
    assert_eq!(abs(&d, "rr").0, 300.0, "align=right");
    // align=justify: text only.
    let d = doc(
        "<div align=justify style='width:400px'><div id=j style='width:100px;height:10px'></div></div>",
    );
    assert_eq!(abs(&d, "j").0, 0.0, "align=justify aligns text only");
    // Flex parent: not aligned (Chrome: flex items ignore it).
    let d = doc(
        "<div align=center style='width:400px'><div style='display:flex'><div id=fi style='width:100px;height:10px'></div></div></div>",
    );
    assert_eq!(abs(&d, "fi").0, 0.0, "a flex item is not aligned");
    // A host block (f) with an auto width.
    let d = doc(
        "<div style='width:400px'><div id=hb style='height:30px;background:red'><div style='display:table-cell;width:50px'>x</div></div></div>",
    );
    assert_eq!(
        abs(&d, "hb").2,
        400.0,
        "a block standing for a table keeps its width"
    );
    let d = doc(
        "<div style='width:400px'><div id=ht>text before<div style='display:table-cell;width:50px'>x</div></div></div>",
    );
    assert_eq!(
        abs(&d, "ht").2,
        400.0,
        "a block with text beside a cell stays a block"
    );
}

fn cellh(h: &str) -> String {
    format!(
        "<table style='border-spacing:0'><tr><td id=c style='padding:0;{h}'><div id=x style='height:50%;width:10px'><div style='height:30px'></div></div></td></tr></table><div id=after style='height:5px'></div>"
    )
}

#[test]
fn c_cell_height_changed_after_layout() {
    let mut d = doc(&cellh("height:200px"));
    eprintln!("fixed: x={:?}", abs(&d, "x"));
    let c = q(&d, "c");
    d.set_attribute(c, "style", "padding:0");
    d.resolve_layout(610.0, 800.0);
    let mut bad = same(&d, &cellh(""), 610.0, &["c", "x", "after"], "200px->auto");
    d.set_attribute(c, "style", "padding:0;height:200px");
    d.resolve_layout(620.0, 800.0);
    bad.extend(same(
        &d,
        &cellh("height:200px"),
        620.0,
        &["c", "x", "after"],
        "auto->200px",
    ));
    report(bad);
}

fn ib(text: &str) -> String {
    format!("<table style='width:600px;border-spacing:0'><tr>
        <td id=cell style='text-align:center;padding:40px 4px;font-size:0'>
        <table id=t1 style='display:inline-block;max-width:280px;width:100%;border-spacing:0'><tr>
          <td style='padding:0 12px 24px'><div style='width:256px;height:120px'></div></td></tr></table>
        <table id=t2 style='display:inline-block;width:100%;border-spacing:0'><tr>
          <td id=tc style='padding:0 12px;font-size:16px;line-height:20px;text-align:left'>{text}</td></tr></table>
        </td></tr></table>")
}
const TXT: &str = "Some white text that belongs inside the dark cell, long enough to wrap onto a second line at this width.";

#[test]
fn b_percent_inline_blocks_across_layouts() {
    let mut d = doc(&ib(TXT));
    let cell = q(&d, "cell");
    let mut bad = vec![];
    for (i, w) in [600.0, 400.0, 600.0, 650.0, 600.0].into_iter().enumerate() {
        d.set_attribute(cell, "data-x", &i.to_string());
        d.resolve_layout(w, 800.0);
        bad.extend(same(
            &d,
            &ib(TXT),
            w,
            &["cell", "t1", "t2"],
            &format!("layout {i} at {w}"),
        ));
    }
    let tc = q(&d, "tc");
    let t = d.get_children(tc)[0];
    d.set_text_content(t, "short");
    d.resolve_layout(610.0, 800.0);
    bad.extend(same(
        &d,
        &ib("short"),
        610.0,
        &["cell", "t1", "t2"],
        "text short",
    ));
    d.set_text_content(t, TXT);
    d.resolve_layout(620.0, 800.0);
    bad.extend(same(
        &d,
        &ib(TXT),
        620.0,
        &["cell", "t1", "t2"],
        "text long again",
    ));
    report(bad);
}

fn nested(w: &str) -> String {
    format!(
        "<div style='width:{w}'><div id=o style='display:inline-block;width:50%'><div id=m style='display:inline-block;width:80%'><span id=i style='display:inline-block;width:100%'>word word word word word word word word word word</span></div></div></div>"
    )
}

#[test]
#[ignore = "fails on main too: nested percentage inline-blocks shrink on each relayout; filed separately"]
fn b_nested_percent_inline_blocks_resize() {
    let mut d = doc(&nested("500px"));
    let mut bad = vec![];
    let ids = ["o", "m", "i"];
    for (i, (w, vw)) in [
        ("500px", 600.0),
        ("300px", 610.0),
        ("300px", 620.0),
        ("500px", 630.0),
        ("500px", 640.0),
    ]
    .into_iter()
    .enumerate()
    {
        let outer = d.get_children(d.body())[0];
        d.set_attribute(outer, "style", &format!("width:{w}"));
        d.resolve_layout(vw, 800.0);
        bad.extend(same(&d, &nested(w), vw, &ids, &format!("layout {i} {w}")));
    }
    // And the same inside a shrink-to-fit cell.
    let cellnest = |t: &str| {
        format!(
            "<table style='border-spacing:0'><tr><td id=cc style='padding:0'><div id=o style='display:inline-block;width:50%'><div id=m style='display:inline-block;width:80%'><span id=i style='display:inline-block;width:100%'>{t}</span></div></div></td></tr></table>"
        )
    };
    let txt = "word word word word word word word word word word";
    let mut d = doc(&cellnest(txt));
    let cc = q(&d, "cc");
    for (i, vw) in [610.0, 300.0, 620.0, 630.0].into_iter().enumerate() {
        d.set_attribute(cc, "data-x", &i.to_string());
        d.resolve_layout(vw, 800.0);
        bad.extend(same(
            &d,
            &cellnest(txt),
            vw,
            &["cc", "o", "m", "i"],
            &format!("cell layout {i} at {vw}"),
        ));
    }
    eprintln!(
        "cell nest: cc={:?} o={:?} i={:?}",
        abs(&d, "cc"),
        abs(&d, "o"),
        abs(&d, "i")
    );
    report(bad);
}

#[test]
fn a_percent_picture_cell_width_changes() {
    const IMG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAyAAAADICAIAAACf7RJNAAADBElEQVR42u3WMQ0AAAgEsZeDJhQjCxNsNKmCmy7VAwDAoUgAAGCwAAAMFgCAwQIAwGABABgsAACDBQCAwQIAMFgAAAYLAACDBQBgsAAADBYAgMECAMBgAQAYLAAAgwUAgMECADBYAAAGCwAAgwUAYLAAAAwWAAAGCwDAYAEAGCwAAIMFAIDBAgAwWAAABgsAAIMFAGCwAAAMFgAABgsAwGABABgsAAAMFgCAwQIAMFgAAAYLAACDBQBgsAAADBYAAAYLAMBgAQAYLAAADBYAgMECADBYAAAYLAAAgwUAYLAAAAwWAAAGCwDAYAEAGCwAAAwWAIDBAgAwWAAAGCwAAIMFAGCwAAAMFgAABgsAwGABABgsAAAMFgCAwQIAMFgAABgsAACDBQBgsAAAMFgAAAYLAMBgAQAYLAAADBYAgMECADBYAAAYLAAAgwUAYLAAADBYAAAGCwDAYAEAYLAAAAwWAIDBAgAwWAAAGCwAAIMFAGCwAAAwWAAABgsAwGABAGCwAAAMFgCAwQIAwGABABgsAACDBQBgsAAAMFgAAAYLAMBgAQBgsAAADBYAgMECAMBgAQAYLAAAgwUAYLAAADBYAAAGCwDAYAEAYLAAAAwWAIDBAgDAYAEAGCwAAIMFAIDBAgAwWAAABgsAwGABAGCwAAAMFgCAwQIAwGABABgsAACDBQCAwQIAMFgAAAYLAACDBQBgsAAADBYAgMECAMBgAQAYLAAAgwUAgMECADBYAAAGCwAAgwUAYLAAAAwWAAAGCwDAYAEAGCwAAIMFAIDBAgAwWAAABgsAAIMFAGCwAAAMFgAABgsAwGABABgsAACDpQIAgMECADBYAAAGCwAAgwUAYLAAAAwWAAAGCwDAYAEAGCwAAAwWAIDBAgAwWAAABgsAAIMFAGCwAAAMFgAABgsAwGABABgsAAAMFgCAwQIAMFgAABgsAACDBQBgsAAADBYAAAYLAMBgAQAYLAAADBYAgMECADBYAAAYLAAAgwUAYLAAADBYAAAGCwDAYAEAfLMLtdks6n5QTwAAAABJRU5ErkJggg==";
    let html = |w: u32| {
        format!("<table style='width:600px;border-spacing:0'><tr>
            <td id=a width={w} style='padding:0'><img id=i width='100%' style='display:block' src='{IMG}'></td>
            <td id=b width=200 style='padding:0'>text</td></tr></table>")
    };
    let mut d = doc(&html(300));
    let a = q(&d, "a");
    d.set_attribute(a, "width", "100");
    d.resolve_layout(610.0, 800.0);
    let mut bad = same(&d, &html(100), 610.0, &["a", "i", "b"], "300->100");
    d.set_attribute(a, "width", "300");
    d.resolve_layout(620.0, 800.0);
    bad.extend(same(&d, &html(300), 620.0, &["a", "i", "b"], "100->300"));
    report(bad);
}

#[test]
fn e_parent_display_changed_after_layout() {
    let html = |s: &str| {
        format!(
            "<div id=pa align=center style='width:400px;{s}'><div id=k style='width:100px;height:10px'></div></div>"
        )
    };
    let mut d = doc(&html(""));
    let pa = q(&d, "pa");
    d.set_attribute(pa, "style", "width:400px;display:flex");
    d.resolve_layout(610.0, 800.0);
    report(same(
        &d,
        &html("display:flex"),
        610.0,
        &["k"],
        "block->flex",
    ));
}

#[test]
fn f_mixed_host_vs_chrome() {
    // A block mixing a block and a row is not a table host (CSS wraps the
    // row alone in an anonymous table; not modelled): printed, not checked.
    let d = doc(&host(&format!("{P}{ROW}"), ""));
    eprintln!(
        "p+row: h={:?} p={:?} c1={:?} c2={:?} (Chrome h=(0,0,300,22) p=(0,0,300,12) c1=(0,12,50,10) c2=(50,12,70,10))",
        abs(&d, "h"),
        abs(&d, "p"),
        abs(&d, "c1"),
        abs(&d, "c2")
    );
    let d = doc(
        "<div style='width:400px'><div id=ht>text before<div id=tc style='display:table-cell;width:50px'>x</div></div></div>",
    );
    assert_eq!(abs(&d, "ht"), (0.0, 0.0, 400.0, 40.0));
    assert_eq!(abs(&d, "tc"), (0.0, 20.0, 50.0, 20.0));
}
