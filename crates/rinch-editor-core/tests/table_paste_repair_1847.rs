//! #1847, adopted from the second review of #1931: a table pasted with
//! ragged rows, spans that collide or run past the last row, is repaired in
//! the paste's own transaction until it tiles (the repair stopped after three
//! rounds, and one random table in 3,000 stayed ragged), and the padding is a
//! cell per missing column, as prosemirror-tables pads, unless that would
//! take more than the HTML reader's bound (a pasted `colspan=1000`).
//! `RINCH_1847_PASTE_SEEDS` sets the seed count.

use std::rc::Rc;

use rinch_editor_core::schema::validation::validate_content;
use rinch_editor_core::serialize::{node_to_html, slice_from_html};
use rinch_editor_core::tables::is_table;
use rinch_editor_core::*;

fn span(cell: &Node, name: &str) -> usize {
    cell.attrs().get_int(name).unwrap_or(1).max(1) as usize
}

/// What is wrong with `table`'s grid, by prosemirror-tables' `computeMap`
/// rules (written out here rather than through `TableMap`, which is what the
/// commands under test use): a slot two cells claim (`collision`), a rowspan
/// past the last row, or a slot no cell covers (a ragged row).
fn tiling_problem(table: &Node) -> Option<String> {
    let height = table.child_count();
    // `findWidth`: the widest row, rowspans from above included.
    let mut carried = vec![0usize; height + 1];
    let mut width = 0;
    for r in 0..height {
        let row = table.child(r);
        let mut w = carried[r];
        for i in 0..row.child_count() {
            let c = row.child(i);
            let (cs, rs) = (span(c, "colspan"), span(c, "rowspan"));
            w += cs;
            for below in &mut carried[r + 1..(r + rs).min(height).max(r + 1)] {
                *below += cs;
            }
        }
        width = width.max(w);
    }
    let mut grid = vec![false; width * height];
    for r in 0..height {
        let row = table.child(r);
        let mut col = 0;
        for i in 0..row.child_count() {
            let c = row.child(i);
            while col < width && grid[r * width + col] {
                col += 1;
            }
            let (cs, rs) = (span(c, "colspan"), span(c, "rowspan"));
            if r + rs > height {
                return Some(format!("row {r} cell {i}: rowspan {rs} past the last row"));
            }
            for h in 0..rs {
                for w in 0..cs {
                    let slot = (r + h) * width + col + w;
                    if col + w >= width || grid[slot] {
                        return Some(format!("row {r} cell {i}: collision"));
                    }
                    grid[slot] = true;
                }
            }
            col += cs;
        }
    }
    if let Some(slot) = grid.iter().position(|s| !s) {
        return Some(format!("slot {slot} of a {width}-wide grid uncovered"));
    }
    None
}

fn valid(s: &Schema, n: &Node) -> bool {
    if n.is_text() {
        return true;
    }
    let names: Vec<&str> = n
        .content()
        .children()
        .iter()
        .map(|c| c.type_name())
        .collect();
    validate_content(s, n.type_name(), &names).is_ok()
        && n.content().children().iter().all(|c| valid(s, c))
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn sk() -> Rc<Schema> {
    Rc::new(Schema::starter_kit())
}
fn doc_html(s: &Schema, html: &str) -> Node {
    let sl = slice_from_html(s, html).unwrap();
    s.branch("doc", sl.content.clone()).unwrap()
}
fn state(s: &Rc<Schema>, doc: Node, sel: Selection) -> EditorState {
    let st = EditorState::create(s.clone(), doc, default_plugins());
    let mut tr = st.tr();
    tr.set_selection(sel);
    st.apply(tr)
}
fn tables(doc: &Node) -> Vec<(usize, Option<String>, String)> {
    let mut v = Vec::new();
    doc.nodes_between(0, doc.content().size(), &mut |n, pos, _| {
        if is_table(n) {
            v.push((pos, tiling_problem(n), node_to_html(n)));
        }
        true
    });
    v
}
fn at(doc: &Node, text: &str) -> usize {
    let mut f = None;
    doc.nodes_between(0, doc.content().size(), &mut |n, pos, _| {
        if f.is_none() && n.is_text() && n.text().is_some_and(|t| t.contains(text)) {
            f = Some(pos + n.text().unwrap().find(text).unwrap());
        }
        f.is_none()
    });
    f.unwrap_or_else(|| panic!("no {text}"))
}
fn paste_at(s: &Rc<Schema>, doc_src: &str, pos: usize, html: &str) -> EditorState {
    let st = state(s, doc_html(s, doc_src), Selection::cursor(Pos(pos)));
    let mut tr = st.tr();
    tr.replace_selection(slice_from_html(s, html).unwrap())
        .unwrap();
    st.apply(tr)
}

/// Random ragged tables pasted at a caret: every result must tile.
#[test]
fn every_pasted_ragged_table_is_padded_until_it_tiles() {
    let s = sk();
    let seeds: u64 = std::env::var("RINCH_1847_PASTE_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3000);
    let (mut bad, mut n) = (Vec::new(), 0);
    for seed in 1..=seeds {
        let mut rng = Rng(0xD1B5_4A32_D192_ED03 ^ seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let rows = 1 + rng.below(4);
        let mut html = String::from("<table>");
        for _ in 0..rows {
            html.push_str("<tr>");
            for _ in 0..rng.below(5) {
                let cs = 1 + rng.below(3);
                let rs = 1 + rng.below(3);
                html.push_str(&format!(
                    "<td colspan=\"{cs}\" rowspan=\"{rs}\"><p>c</p></td>"
                ));
            }
            html.push_str("</tr>");
        }
        html.push_str("</table>");
        let after = paste_at(&s, "<p>xy</p>", 2, &html);
        n += 1;
        for (_, prob, h) in tables(&after.doc) {
            if let Some(p) = prob {
                bad.push(format!("{p}\n  IN  {html}\n  OUT {h}"));
            }
        }
        if !valid(&s, &after.doc) {
            bad.push(format!("INVALID {html}"));
        }
    }
    eprintln!("pasted {n}, bad {}", bad.len());
    bad.sort_by_key(|b| b.len());
    for b in bad.iter().take(4) {
        eprintln!("{b}\n");
    }
    assert!(bad.is_empty());
}

#[test]
fn a_short_row_gets_a_cell_per_missing_column() {
    let s = sk();
    // A header row one cell short of three columns, and a gap split by a
    // rowspan from above: single cells, as prosemirror-tables pads.
    for (html, want) in [
        (
            "<table><tr><th><p>h1</p></th></tr><tr><td><p>a</p></td><td><p>b</p></td><td><p>c</p></td></tr></table>",
            "<table><tr><th><p>h1</p></th><th><p></p></th><th><p></p></th></tr><tr><td><p>a</p></td><td><p>b</p></td><td><p>c</p></td></tr></table>",
        ),
        (
            "<table><tr><td><p>a</p></td><td><p>b</p></td><td rowspan=\"2\"><p>c</p></td><td><p>d</p></td></tr><tr><td><p>e</p></td></tr></table>",
            "<table><tr><td><p>a</p></td><td><p>b</p></td><td rowspan=\"2\"><p>c</p></td><td><p>d</p></td></tr><tr><td><p>e</p></td><td><p></p></td><td><p></p></td></tr></table>",
        ),
    ] {
        let after = paste_at(&s, "<p>xy</p>", 2, html);
        let t = tables(&after.doc);
        assert_eq!(t.len(), 1, "{html}");
        assert_eq!(t[0].1, None, "{html}");
        assert_eq!(t[0].2, want);
    }
}

#[test]
fn an_edit_across_a_three_column_table_pads_with_plain_cells() {
    // A delete from a cell of one table into the next leaves a short row;
    // its padding is plain cells, never a merged cell nobody made.
    let s = sk();
    let src = "<table><tr><td><p>a1</p></td><td><p>b1</p></td><td><p>c1</p></td></tr>\
               <tr><td><p>d1</p></td><td><p>e1</p></td><td><p>f1</p></td></tr></table>\
               <table><tr><td><p>g1</p></td><td><p>h1</p></td><td><p>i1</p></td></tr></table><p>zw</p>";
    let doc = doc_html(&s, src);
    let st = state(
        &s,
        doc.clone(),
        Selection::text(Pos(at(&doc, "d1") + 1), Pos(at(&doc, "i1") + 1)),
    );
    let next = st.run("deleteSelection").expect("deletes");
    let html = node_to_html(&next.doc);
    assert_eq!(
        html,
        "<table><tr><td><p>a1</p></td><td><p>b1</p></td><td><p>c1</p></td></tr>\
         <tr><td><p>d1</p></td><td><p></p></td><td><p></p></td></tr></table><p>zw</p>"
    );
    for (_, problem, h) in tables(&next.doc) {
        assert_eq!(problem, None, "{h}");
    }
}

#[test]
fn a_wide_span_over_bare_rows_is_padded_by_spanning_cells() {
    // 65 one-cell rows under a 1000-column cell: a cell per column would be
    // 64,935 cells; one spanning cell per short row is 65.
    let s = sk();
    let html = format!(
        "<table><tr><td colspan=\"1000\"><p>a</p></td></tr>{}</table>",
        "<tr><td><p>b</p></td></tr>".repeat(65)
    );
    let after = paste_at(&s, "<p>xy</p>", 2, &html);
    let out = node_to_html(&after.doc);
    assert_eq!(out.matches("<td").count(), 1 + 65 + 65, "{}", &out[..200]);
    assert!(tables(&after.doc).iter().all(|t| t.1.is_none()));
}
