//! #1847, adopted from the review of #1931: random documents of tables with
//! several merged cells, nested tables and (some) a row loaded ragged; random
//! text selections (which land as carets, text ranges or cell selections);
//! the deletes, Enter, the inserts, typing and two pastes. No result may have
//! more ragged tables than the document had, a document with none may not
//! get one, every result is valid, and every edit undoes to the document it
//! was made on. `RINCH_1847_REPAIR_SEEDS` raises the seed count.

use std::rc::Rc;

use rinch_editor_core::schema::validation::validate_content;
use rinch_editor_core::tables::is_table;
use rinch_editor_core::*;

fn para(s: &Schema, text: &str) -> Node {
    let content = if text.is_empty() {
        Fragment::empty()
    } else {
        Fragment::from_node(s.text(text).unwrap())
    };
    s.branch("paragraph", content).unwrap()
}

fn branch(s: &Schema, name: &str, children: Vec<Node>) -> Node {
    s.create_node(name, Attrs::new(), Fragment::from_children(children))
        .unwrap()
}

fn cell(s: &Schema, colspan: i64, rowspan: i64, blocks: Vec<Node>) -> Node {
    s.create_node(
        "table_cell",
        Attrs::from_iter([
            ("colspan", AttrValue::Int(colspan)),
            ("rowspan", AttrValue::Int(rowspan)),
        ]),
        Fragment::from_children(blocks),
    )
    .unwrap()
}

/// A state holding `selection` the way an editor lands one: through a
/// transaction (`EditorHandle::set_selection`, a drag, a mapped remote edit),
/// which normalizes it.
fn state(s: &Rc<Schema>, doc: Node, selection: Selection) -> EditorState {
    let st = EditorState::create(s.clone(), doc, default_plugins());
    let mut tr = st.tr();
    tr.set_selection(selection);
    st.apply(tr)
}

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

const EXTRA: [&str; 3] = ["type", "paste-inline", "paste-blocks"];

fn run_extra(st: &EditorState, name: &str) -> Option<EditorState> {
    let s = st.schema();
    let start = if matches!(st.selection, Selection::Cell(_)) {
        let tr = commands::table_ops::clear_cells(st)?;
        st.apply(tr)
    } else {
        st.clone()
    };
    let mut tr = start.tr();
    match name {
        "type" => {
            if tr.insert_text("Q").is_err() {
                tr = start.tr();
                tr.delete_selection().ok()?;
                tr.insert_text("Q").ok()?;
            }
        }
        "paste-inline" => {
            let slice = serialize::slice_from_text(s, "pp").ok()?;
            tr.replace_selection(slice).ok()?;
        }
        _ => {
            let slice = serialize::slice_from_html(s, "<p>p1</p><h2>p2</h2>").ok()?;
            tr.replace_selection(slice).ok()?;
        }
    }
    Some(start.apply(tr))
}

/// A table with several merged cells (non-overlapping rectangles), maybe nested tables,
/// and with `ragged` one row loses its last cell.
fn table2(s: &Schema, rng: &mut Rng, depth: usize, ragged: bool) -> Node {
    let cols = 1 + rng.below(4);
    let rows = 1 + rng.below(4);
    let mut owner: Vec<Option<usize>> = vec![None; rows * cols];
    let mut merges = Vec::new();
    for _ in 0..rng.below(4) {
        let (top, left) = (rng.below(rows), rng.below(cols));
        let (h, w) = (1 + rng.below(rows - top), 1 + rng.below(cols - left));
        let free = (top..top + h).all(|r| (left..left + w).all(|c| owner[r * cols + c].is_none()));
        if free && h * w > 1 {
            let id = merges.len();
            for r in top..top + h {
                for c in left..left + w {
                    owner[r * cols + c] = Some(id);
                }
            }
            merges.push((top, left, h, w));
        }
    }
    let content = |rng: &mut Rng| -> Vec<Node> {
        if depth < 1 && rng.below(6) == 0 {
            vec![para(s, "n"), {
                let rg = rng.below(4) == 0;
                table2(s, rng, depth + 1, rg)
            }]
        } else {
            vec![para(s, ["", "a", "bc d"][rng.below(3)])]
        }
    };
    let mut out_rows = Vec::new();
    for r in 0..rows {
        let mut cells = Vec::new();
        for c in 0..cols {
            match owner[r * cols + c] {
                Some(id) => {
                    let (top, left, h, w) = merges[id];
                    if r == top && c == left {
                        let b = content(rng);
                        cells.push(cell(s, w as i64, h as i64, b));
                    }
                }
                None => {
                    let b = content(rng);
                    cells.push(cell(s, 1, 1, b));
                }
            }
        }
        out_rows.push(cells);
    }
    if ragged {
        let r = rng.below(rows);
        if out_rows[r].len() > 1 {
            out_rows[r].pop();
        }
    }
    let rows: Vec<Node> = out_rows
        .into_iter()
        .map(|c| branch(s, "table_row", c))
        .collect();
    branch(s, "table", rows)
}

fn doc2(s: &Schema, rng: &mut Rng) -> Node {
    let mut blocks = Vec::new();
    for _ in 0..2 + rng.below(3) {
        match rng.below(4) {
            0 => blocks.push(para(s, ["x", "", "yz"][rng.below(3)])),
            1 => blocks.push(branch(s, "blockquote", {
                let rg = rng.below(4) == 0;
                vec![table2(s, rng, 0, rg)]
            })),
            _ => {
                let rg = rng.below(4) == 0;
                blocks.push(table2(s, rng, 0, rg))
            }
        }
    }
    branch(s, "doc", blocks)
}

fn all_tables(doc: &Node) -> Vec<(usize, bool)> {
    let mut v = Vec::new();
    doc.nodes_between(0, doc.content().size(), &mut |n, pos, _| {
        if is_table(n) {
            v.push((pos, tiling_problem(n).is_some()));
        }
        true
    });
    v
}

#[test]
fn no_edit_over_random_selections_makes_a_table_ragged() {
    let s = Rc::new(Schema::starter_kit());
    let seeds: u64 = std::env::var("RINCH_1847_REPAIR_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let cmds = [
        "deleteSelection",
        "deleteCharBackward",
        "enter",
        "splitBlock",
        "insertHardBreak",
        "insertHorizontalRule",
        "insertTable",
    ];
    let (mut runs, mut applied, mut fails) = (0, 0, Vec::<String>::new());
    let mut more_ragged = 0;
    let mut kinds = std::collections::BTreeMap::<String, usize>::new();
    for seed in 1..=seeds {
        let mut rng = Rng(0xA076_1D64_78BD_642F ^ seed.wrapping_mul(0x2545_F491_4F6C_DD1D));
        let doc = doc2(&s, &mut rng);
        if !valid(&s, &doc) {
            continue;
        }
        let size = doc.content().size();
        let before_ragged = all_tables(&doc).iter().filter(|t| t.1).count();
        let mut tpos = Vec::new();
        for p in 0..=size {
            if doc.resolve(Pos(p)).unwrap().parent().is_textblock() {
                tpos.push(p);
            }
        }
        for _ in 0..60 {
            let (a, b) = (tpos[rng.below(tpos.len())], tpos[rng.below(tpos.len())]);
            if a == b {
                continue;
            }
            let st = state(&s, doc.clone(), Selection::text(Pos(a), Pos(b)));
            let k = match &st.selection {
                Selection::Cell(_) => "cell",
                Selection::Text(t) if t.anchor == t.head => "caret",
                Selection::Text(_) => "text",
                _ => "node",
            };
            *kinds.entry(k.to_string()).or_default() += 1;
            let names: Vec<&str> = cmds.iter().copied().chain(EXTRA).collect();
            for name in names {
                runs += 1;
                let next = if EXTRA.contains(&name) {
                    run_extra(&st, name)
                } else {
                    st.run(name)
                };
                let Some(next) = next else { continue };
                applied += 1;
                let after = all_tables(&next.doc);
                let after_ragged = after.iter().filter(|t| t.1).count();
                let tag = format!(
                    "seed {seed} {:?} (landed {:?}) {name}: {} -> {}",
                    Selection::text(Pos(a), Pos(b)),
                    st.selection,
                    serialize::node_to_html(&doc),
                    serialize::node_to_html(&next.doc)
                );
                if after_ragged > before_ragged {
                    more_ragged += 1;
                    fails.push(format!(
                        "MORE RAGGED ({before_ragged}->{after_ragged}) {tag}"
                    ));
                }
                if !valid(&s, &next.doc) {
                    fails.push(format!("INVALID {tag}"));
                }
                if before_ragged == 0 && after_ragged > 0 {
                    fails.push(format!("RAGGED-FROM-CLEAN {tag}"));
                }
                if next.doc != doc && !EXTRA.contains(&name) {
                    match next.run("undo") {
                        Some(u) if u.doc == doc => {}
                        _ => fails.push(format!("UNDO {tag}")),
                    }
                }
            }
        }
    }
    eprintln!(
        "runs {runs} applied {applied} more_ragged {more_ragged} kinds {kinds:?} fails {}",
        fails.len()
    );
    let mut by = std::collections::BTreeMap::<String, (usize, String)>::new();
    for f in &fails {
        let key: String = f.split_whitespace().next().unwrap().to_string()
            + " "
            + f.split(") ")
                .nth(1)
                .map(|x| x.split(':').next().unwrap_or(""))
                .unwrap_or("");
        let e = by.entry(key).or_insert((0, f.clone()));
        e.0 += 1;
        if f.len() < e.1.len() {
            e.1 = f.clone();
        }
    }
    for (k, (n, f)) in &by {
        eprintln!("== {k}: {n}\n{f}\n");
    }
    // Positive controls: each kind of selection was made, edits applied.
    for kind in ["caret", "cell", "text"] {
        assert!(
            kinds.get(kind).copied().unwrap_or(0) > 20,
            "few {kind}: {kinds:?}"
        );
    }
    assert!(applied > runs / 4, "only {applied} of {runs} runs applied");
    assert!(
        fails.is_empty(),
        "{} failures: {:?}",
        fails.len(),
        by.keys().collect::<Vec<_>>()
    );
}
