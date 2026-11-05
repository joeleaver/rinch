//! The second layout review of #2213: histories against a fresh layout of
//! the final document, and `align`'s placement against Chrome.
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
fn add(d: &mut RinchDocument, parent: NodeId, html: &str) {
    let tmp = d.create_element("div");
    d.set_inner_html(tmp, html);
    for c in d.get_children(tmp) {
        d.append_child(parent, c);
    }
}
fn same(d: &RinchDocument, html: &str, w: f32, ids: &[&str], step: &str) -> Vec<String> {
    let fresh = doc_at(html, w);
    let mut bad = vec![];
    for id in ids {
        if d.query_selector(&format!("#{id}")).is_none() {
            continue;
        }
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
thread_local! {
    // Per test: tests run on threads of their own, in parallel.
    static VW: std::cell::Cell<f32> = const { std::cell::Cell::new(600.0) };
}
/// A layout at a viewport 7px wider than the last.
fn step(d: &mut RinchDocument) -> f32 {
    let w = VW.with(|v| {
        v.set(v.get() + 7.0);
        v.get()
    });
    d.resolve_layout(w, 800.0);
    w
}

const ROW: &str = "<div id=r style='display:table-row'><div id=c1 style='display:table-cell;width:50px;height:10px'></div><div id=c2 style='display:table-cell;width:70px;height:10px'></div></div>";
const P: &str = "<div id=p style='height:12px'></div>";
fn host(inner: &str) -> String {
    format!(
        "<div id=w style='width:300px'><div id=h style='background:red'>{inner}</div><div id=after style='height:5px'></div></div>"
    )
}
const IDS: &[&str] = &["w", "h", "r", "c1", "c2", "p", "after"];

#[test]
fn a1_become_stop_become() {
    let mut d = doc(&host(ROW));
    let h = q(&d, "h");
    let mut bad = vec![];
    for round in 0..2 {
        let r = q(&d, "r");
        d.remove_child(h, r);
        add(&mut d, h, P);
        let w = step(&mut d);
        bad.extend(same(&d, &host(P), w, IDS, &format!("{round}: stop")));
        let p = q(&d, "p");
        d.remove_child(h, p);
        add(&mut d, h, ROW);
        let w = step(&mut d);
        bad.extend(same(&d, &host(ROW), w, IDS, &format!("{round}: become")));
    }
    report(bad);
}

#[test]
fn a2_row_display_toggled_many_times() {
    let mut d = doc(&host(ROW));
    let r = q(&d, "r");
    let mut bad = vec![];
    for (i, disp) in [
        "block",
        "table-row",
        "none",
        "table-row",
        "flex",
        "table-row",
        "contents",
        "table-row",
    ]
    .iter()
    .enumerate()
    {
        d.set_attribute(r, "style", &format!("display:{disp}"));
        let w = step(&mut d);
        bad.extend(same(
            &d,
            &host(&ROW.replace("display:table-row", &format!("display:{disp}"))),
            w,
            IDS,
            &format!("{i}: {disp}"),
        ));
    }
    report(bad);
}

#[test]
fn a3_text_gained_then_lost() {
    // Text beside the rows (host stops qualifying), and text inside a cell.
    let mut d = doc(&host(ROW));
    let h = q(&d, "h");
    let mut bad = vec![];
    let t = d.create_text("loose text");
    d.append_child(h, t);
    let w = step(&mut d);
    bad.extend(same(
        &d,
        &host(&format!("{ROW}loose text")),
        w,
        IDS,
        "host text added",
    ));
    d.set_text_content(t, "   ");
    let w = step(&mut d);
    bad.extend(same(
        &d,
        &host(&format!("{ROW}   ")),
        w,
        IDS,
        "host text -> whitespace",
    ));
    d.set_text_content(t, "back");
    let w = step(&mut d);
    bad.extend(same(
        &d,
        &host(&format!("{ROW}back")),
        w,
        IDS,
        "host text back",
    ));
    d.remove_child(h, t);
    let w = step(&mut d);
    bad.extend(same(&d, &host(ROW), w, IDS, "host text removed"));
    let c1 = q(&d, "c1");
    let t2 = d.create_text("cell text");
    d.append_child(c1, t2);
    let w = step(&mut d);
    let withtext = ROW.replace(
        "height:10px'></div><div id=c2",
        "height:10px'>cell text</div><div id=c2",
    );
    bad.extend(same(&d, &host(&withtext), w, IDS, "cell text added"));
    d.remove_child(c1, t2);
    let w = step(&mut d);
    bad.extend(same(&d, &host(ROW), w, IDS, "cell text removed"));
    report(bad);
}

fn nest(inner_row: bool, outer_row: bool) -> String {
    let inner = if inner_row {
        "<div id=r2 style='display:table-row'><div id=d1 style='display:table-cell;width:30px;height:8px'></div><div id=d2 style='display:table-cell;width:40px;height:8px'></div></div>"
    } else {
        "<div id=r2 style='height:8px'></div>"
    };
    let rs = if outer_row {
        "display:table-row"
    } else {
        "display:block"
    };
    format!(
        "<div id=w style='width:300px'><div id=h1><div id=r1 style='{rs}'><div id=k style='display:table-cell;width:20px'>k</div><div id=x style='display:table-cell'><div id=h2>{inner}</div></div></div></div></div>"
    )
}

#[test]
fn a4_host_inside_a_host() {
    let ids = &["w", "h1", "r1", "k", "x", "h2", "r2", "d1", "d2"];
    let mut d = doc(&nest(true, true));
    let mut bad = same(&d, &nest(true, true), 600.0, ids, "start");
    let r2 = q(&d, "r2");
    d.set_attribute(r2, "style", "height:8px");
    let w = step(&mut d);
    // (children stay: compare against a fresh doc with the same children)
    let fresh_inner = nest(true, true).replace(
        "id=r2 style='display:table-row'",
        "id=r2 style='height:8px'",
    );
    bad.extend(same(&d, &fresh_inner, w, ids, "inner row -> block"));
    let r1 = q(&d, "r1");
    d.set_attribute(r1, "style", "display:block");
    let w = step(&mut d);
    let f2 = fresh_inner.replace(
        "id=r1 style='display:table-row'",
        "id=r1 style='display:block'",
    );
    bad.extend(same(&d, &f2, w, ids, "outer row -> block"));
    d.set_attribute(r2, "style", "display:table-row");
    d.set_attribute(r1, "style", "display:table-row");
    let w = step(&mut d);
    bad.extend(same(&d, &nest(true, true), w, ids, "both back, one frame"));
    report(bad);
}

#[test]
fn a5_host_parent_table_removed() {
    let html = |t: bool| {
        format!(
            "<div id=box style='width:400px'>{}</div><div id=after style='height:5px'></div>",
            if t {
                "<table id=T style='border-spacing:0'><tr><td id=td style='padding:0'><div id=h>ROW</div></td><td>z</td></tr></table>".replace("ROW", ROW)
            } else {
                String::new()
            }
        )
    };
    let mut d = doc(&html(true));
    let bx = q(&d, "box");
    let t = q(&d, "T");
    d.remove_child(bx, t);
    let w = step(&mut d);
    let mut bad = same(&d, &html(false), w, &["box", "after"], "table removed");
    d.append_child(bx, t);
    let w = step(&mut d);
    bad.extend(same(
        &d,
        &html(true),
        w,
        &["box", "T", "td", "h", "c1", "c2", "after"],
        "table re-added",
    ));
    // the <tr display:block> host that is the outer table's cell
    let h2 = "<table id=T2><tr id=rr style='display:block'><td id=a>one</td><td id=b>two</td></tr></table><div id=z style='height:4px'></div>";
    let mut d = doc(h2);
    let rr = q(&d, "rr");
    let a = q(&d, "a");
    d.remove_child(rr, a);
    let w = step(&mut d);
    bad.extend(same(
        &d,
        &h2.replace("<td id=a>one</td>", ""),
        w,
        &["T2", "rr", "b", "z"],
        "cell of host-row removed",
    ));
    report(bad);
}

#[test]
fn a6_two_hosts_one_frame() {
    let html = |a: &str, b: &str| {
        format!(
            "<div id=w style='width:300px'><div id=h1>{a}</div><div id=h2>{b}</div><div id=after style='height:3px'></div></div>"
        )
    };
    let row = |n: &str| {
        format!(
            "<div id=r{n} style='display:table-row'><div id=c{n} style='display:table-cell;width:40px;height:9px'></div><div style='display:table-cell;width:60px;height:9px'></div></div>"
        )
    };
    let p = |n: &str| format!("<div id=p{n} style='height:11px'></div>");
    let mut d = doc(&html(&row("a"), &p("b")));
    let (h1, h2) = (q(&d, "h1"), q(&d, "h2"));
    let ra = q(&d, "ra");
    d.remove_child(h1, ra);
    add(&mut d, h1, &p("a"));
    let pb = q(&d, "pb");
    d.remove_child(h2, pb);
    add(&mut d, h2, &row("b"));
    let w = step(&mut d);
    let ids = &["w", "h1", "h2", "pa", "rb", "cb", "after"];
    let mut bad = same(&d, &html(&p("a"), &row("b")), w, ids, "swap");
    // move the row from h2 to h1 in one frame
    let pa = q(&d, "pa");
    d.remove_child(h1, pa);
    let rb = q(&d, "rb");
    d.append_child(h1, rb);
    let w = step(&mut d);
    bad.extend(same(
        &d,
        &html(&row("b"), ""),
        w,
        &["w", "h1", "h2", "rb", "cb", "after"],
        "row moved between hosts",
    ));
    report(bad);
}

// ---- (b) the relative shift ----
fn ctr(child: &str) -> String {
    format!(
        "<div id=pa align=center style='width:400px'><div id=u style='height:10px;{child}'></div><div id=after style='height:5px'></div></div>"
    )
}

#[test]
fn b_shift_shapes_vs_chrome() {
    let cases: &[(&str, f32, &str)] = &[
        ("width:100px;margin-left:20px", 160.0, "static"),
        (
            "width:100px;margin-left:20px;position:relative;left:5px",
            165.0,
            "relative+left",
        ),
        (
            "width:100px;margin-left:20px;position:relative",
            160.0,
            "relative, no inset",
        ),
        (
            "width:100px;margin-left:20px;transform:translateX(10px)",
            160.0,
            "transform (layout x)",
        ),
        (
            "width:100px;margin-left:auto;margin-right:20px",
            280.0,
            "auto margin",
        ),
        ("width:390px;margin-left:20px", 20.0, "no free space"),
        ("width:500px;margin-left:20px", 20.0, "overflowing"),
        (
            "width:500px;margin:0 10px",
            10.0,
            "overflowing, equal margins",
        ),
        ("max-width:100px;margin-left:20px", 160.0, "max-width"),
        (
            "width:100px;margin-left:20%",
            190.0,
            "percent margin (20%=80): 80+(400-180)/2 (Chrome 156: 190)",
        ),
        (
            "width:100px;margin-right:20px",
            140.0,
            "unequal, the other side",
        ),
        (
            "width:500px;margin-right:20px",
            0.0,
            "overflowing, right margin",
        ),
    ];
    let mut bad = vec![];
    for (s, x, what) in cases {
        let d = doc(&ctr(s));
        let got = abs(&d, "u").0;
        eprintln!("{what}: x={got} (Chrome {x})");
        if (got - x).abs() > 0.5 {
            bad.push(format!("{what}: {got} vs Chrome {x}"));
        }
    }
    // Not under align: no shift.
    let d = doc(
        "<div style='width:400px'><div id=u style='width:100px;margin-left:20px;height:10px'></div></div>",
    );
    eprintln!("no align: x={}", abs(&d, "u").0);
    // RTL
    let d = doc(
        "<div align=center dir=rtl style='width:400px;direction:rtl'><div id=u style='width:100px;margin-left:20px;height:10px'></div></div>",
    );
    eprintln!("rtl: x={} (Chrome 160)", abs(&d, "u").0);
    report(bad);
}

#[test]
fn b_shift_incremental() {
    let mut d = doc(&ctr("width:100px;margin-left:20px"));
    let u = q(&d, "u");
    let pa = q(&d, "pa");
    let mut bad = vec![];
    let steps: &[(&str, &str)] = &[
        ("u", "height:10px;width:100px;margin-left:40px"),
        (
            "u",
            "height:10px;width:100px;margin-left:40px;position:relative;left:3px",
        ),
        ("u", "height:10px;width:100px;margin-left:40px"),
        ("u", "height:10px;width:100px;margin:0 10px"),
        ("u", "height:10px;width:100px;margin-left:20px"),
    ];
    for (i, (_, s)) in steps.iter().enumerate() {
        d.set_attribute(u, "style", s);
        let w = step(&mut d);
        bad.extend(same(
            &d,
            &ctr(&s.replace("height:10px;", "")),
            w,
            &["u", "after"],
            &format!("{i}: {s}"),
        ));
    }
    d.remove_attribute(pa, "align");
    let w = step(&mut d);
    bad.extend(same(
        &d,
        &ctr("width:100px;margin-left:20px").replace("align=center ", ""),
        w,
        &["u", "after"],
        "align removed",
    ));
    d.set_attribute(pa, "align", "center");
    let w = step(&mut d);
    bad.extend(same(
        &d,
        &ctr("width:100px;margin-left:20px"),
        w,
        &["u", "after"],
        "align re-added",
    ));
    report(bad);
}

#[test]
fn b_shift_scroll_extent() {
    // Chrome: a 400px scroller, child 100 wide at 160: scrollWidth 400.
    let d = doc(
        "<div id=s align=center style='width:400px;height:50px;overflow:auto'><div id=u style='width:100px;margin-left:300px;height:10px'></div></div>",
    );
    // margin box 400 fits exactly: Chrome x=300, scrollWidth 400.
    eprintln!(
        "x={} scroll_width={} (Chrome 300 / 400)",
        abs(&d, "u").0,
        d.scroll_width(q(&d, "s"))
    );
    let d = doc(
        "<div id=s align=center style='width:400px;height:50px;overflow:auto'><div id=u style='width:100px;margin-left:200px;height:10px'></div></div>",
    );
    // free 100: Chrome x = 200 + 50 = 250, scrollWidth 400.
    eprintln!(
        "x={} scroll_width={} (Chrome 250 / 400)",
        abs(&d, "u").0,
        d.scroll_width(q(&d, "s"))
    );
}

// ---- (c) cell heights ----
fn cellh(h: &str) -> String {
    format!(
        "<table style='border-spacing:0'><tr><td id=c style='padding:0;{h}'><div id=x style='height:50%;min-height:20%;width:10px'><div style='height:30px'></div></div><div id=ab style='position:absolute;top:0;left:0;width:50%;height:5px'></div></td></tr></table><div id=after style='height:5px'></div>"
    )
}

#[test]
fn c_cell_height_histories() {
    let mut d = doc(&cellh("height:200px"));
    let c = q(&d, "c");
    let mut bad = vec![];
    for h in [
        "height:50%",
        "",
        "height:200px",
        "height:120px",
        "height:auto",
        "height:50%",
        "height:200px",
    ] {
        d.set_attribute(c, "style", &format!("padding:0;{h}"));
        let w = step(&mut d);
        bad.extend(same(&d, &cellh(h), w, &["c", "x", "ab", "after"], h));
    }
    // a cell that stops being a cell, and back
    for disp in ["display:block;height:200px", "height:200px"] {
        d.set_attribute(c, "style", &format!("padding:0;{disp}"));
        let w = step(&mut d);
        bad.extend(same(&d, &cellh(disp), w, &["c", "x", "ab", "after"], disp));
    }
    report(bad);
}

#[test]
fn a7_mixed_block_loses_its_non_row_child() {
    let mut d = doc(&host(&format!("{P}{ROW}")));
    let h = q(&d, "h");
    let p = q(&d, "p");
    d.remove_child(h, p);
    let w = step(&mut d);
    let mut bad = same(&d, &host(ROW), w, IDS, "p removed");
    add(&mut d, h, P);
    let w = step(&mut d);
    bad.extend(same(
        &d,
        &host(&format!("{ROW}{P}")),
        w,
        IDS,
        "p added back",
    ));
    let p = q(&d, "p");
    d.set_attribute(p, "style", "display:none");
    let w = step(&mut d);
    bad.extend(same(
        &d,
        &host(&format!(
            "{ROW}{}",
            P.replace("height:12px", "display:none")
        )),
        w,
        IDS,
        "p hidden",
    ));
    report(bad);
}
