//! #1847: a text selection whose ends are in different cells (or at row or
//! table level), or a node selection of a cell, row or table, let a command
//! leave a ragged table — a row with another column count than its table, or
//! spans that no longer tile the grid.
//!
//! The seeded half runs every built-in command, typing and paste over such
//! selections (and over cell selections, carets and in-cell ranges as
//! controls) in random documents holding tables, some with merged cells, and
//! asserts every table of the result tiles its grid, the document is valid,
//! and the edit undoes to the document it was made on and redoes to itself.
//! `RINCH_1847_SEEDS` raises the seed count.

use std::rc::Rc;

use rinch_editor_core::schema::validation::validate_content;
use rinch_editor_core::tables::{self, is_cell, is_row, is_table};
use rinch_editor_core::*;

fn para(s: &Schema, text: &str) -> Node {
    let content = if text.is_empty() {
        Fragment::empty()
    } else {
        Fragment::from_node(s.text(text).unwrap())
    };
    s.branch("paragraph", content).unwrap()
}

fn leaf(s: &Schema, name: &str) -> Node {
    s.create_node(name, Attrs::new(), Fragment::empty())
        .unwrap()
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

fn ragged_tables(doc: &Node) -> Vec<String> {
    let mut bad = Vec::new();
    doc.nodes_between(0, doc.content().size(), &mut |n, pos, _| {
        if is_table(n)
            && let Some(p) = tiling_problem(n)
        {
            bad.push(format!("table at {pos}: {p}"));
        }
        true
    });
    bad
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

// ── the issue's two shapes ──────────────────────────────────────────────────

/// `doc(p("x"), table(row(td(p("ab")), td(p("cd"))), row(td(p("ef")),
/// td(p("gh")))), p("zw"))`.
fn grid_doc(s: &Schema) -> Node {
    let row = |a: &str, b: &str| {
        branch(
            s,
            "table_row",
            vec![
                cell(s, 1, 1, vec![para(s, a)]),
                cell(s, 1, 1, vec![para(s, b)]),
            ],
        )
    };
    let table = branch(s, "table", vec![row("ab", "cd"), row("ef", "gh")]);
    branch(s, "doc", vec![para(s, "x"), table, para(s, "zw")])
}

// Positions in `grid_doc`: p("x") 0..3, the table 3..33, row 1 at 4, its
// cells at 5 and 11 (texts 7..9 and 13..15), row 2 at 18, its cells at 19 and
// 25 (texts 21..23 and 27..29); p("zw") 33..37, its text 34..36.

#[test]
fn a_selection_of_table_parts_is_held_as_a_cell_selection() {
    let s = Rc::new(Schema::starter_kit());
    let node = |a: usize, h: usize| {
        Selection::Node(NodeSelection {
            anchor: Pos(a),
            head: Pos(h),
        })
    };
    let held = |sel: Selection| state(&s, grid_doc(&s), sel).selection;
    // A cell, a row: cell selections.
    assert_eq!(held(node(11, 17)), Selection::cell(Pos(11), Pos(11)));
    assert_eq!(held(node(18, 32)), Selection::cell(Pos(19), Pos(25)));
    // The whole table stays node-selected (Backspace after a table, #1232).
    assert_eq!(held(node(3, 33)), node(3, 33));
    // Text across two cells: the rectangle between them.
    assert_eq!(
        held(Selection::text(Pos(8), Pos(28))),
        Selection::cell(Pos(5), Pos(25))
    );
    assert_eq!(
        held(Selection::text(Pos(22), Pos(14))),
        Selection::cell(Pos(19), Pos(11))
    );
    // An end at table level moves into the text towards the other end.
    assert_eq!(
        held(Selection::text(Pos(4), Pos(14))),
        Selection::cell(Pos(5), Pos(11))
    );
    // Only the boundary between two cells: a caret at its start.
    assert_eq!(
        held(Selection::text(Pos(9), Pos(13))),
        Selection::cursor(Pos(9))
    );
    // Controls: text inside one cell, and a range out of the table, stay.
    assert_eq!(
        held(Selection::text(Pos(7), Pos(9))),
        Selection::text(Pos(7), Pos(9))
    );
    assert_eq!(
        held(Selection::text(Pos(8), Pos(35))),
        Selection::text(Pos(8), Pos(35))
    );
}

#[test]
fn enter_and_split_block_refuse_a_cell_selection() {
    let s = Rc::new(Schema::starter_kit());
    let st = state(&s, grid_doc(&s), Selection::cell(Pos(5), Pos(25)));
    for cmd in ["enter", "splitBlock"] {
        assert!(st.run(cmd).is_none(), "{cmd}");
        assert!(!st.can_run(cmd), "{cmd}");
    }
    // Control: Backspace over it still clears the cells.
    let cleared = st.run("deleteCharBackward").expect("clears the cells");
    assert_eq!(
        serialize::node_to_html(&cleared.doc),
        "<p>x</p><table><tr><td><p></p></td><td><p></p></td></tr>\
         <tr><td><p></p></td><td><p></p></td></tr></table><p>zw</p>"
    );
}

/// The position `offset` chars into the first text node reading `text`.
fn at(doc: &Node, text: &str, offset: usize) -> usize {
    let mut found = None;
    doc.nodes_between(0, doc.content().size(), &mut |n, pos, _| {
        if found.is_none() && n.text() == Some(text) {
            found = Some(pos + offset);
        }
        found.is_none()
    });
    found.unwrap_or_else(|| panic!("no text {text:?}"))
}

/// `table(row(td(p a), td(p b)), row(td(p cd), td(p q)))` and
/// `table(row(td(p ef), td(p gh)), row(td(p ij), td(p kl)))` side by side,
/// then `p(zw)`; with
/// `ragged`, the first table's second row lacks its `q` cell, as a table
/// loaded from HTML may.
fn two_tables(s: &Schema, ragged: bool) -> Node {
    let row = |cells: Vec<Node>| branch(s, "table_row", cells);
    let td = |t: &str| cell(s, 1, 1, vec![para(s, t)]);
    let mut second = vec![td("cd")];
    if !ragged {
        second.push(td("q"));
    }
    let first = branch(s, "table", vec![row(vec![td("a"), td("b")]), row(second)]);
    let other = branch(
        s,
        "table",
        vec![row(vec![td("ef"), td("gh")]), row(vec![td("ij"), td("kl")])],
    );
    branch(s, "doc", vec![first, other, para(s, "zw")])
}

#[test]
fn a_drag_from_a_cell_into_the_next_table_leaves_whole_tables() {
    // The ends are in cells of two tables: no cell selection stands for it.
    // The fitted delete joins "c" with "h", one cell where the row had two,
    // and the second table's last row into the first table; the repair pads
    // the short row.
    let s = Rc::new(Schema::starter_kit());
    let doc = two_tables(&s, false);
    let sel = Selection::text(Pos(at(&doc, "cd", 1)), Pos(at(&doc, "gh", 1)));
    let st = state(&s, doc.clone(), sel.clone());
    assert_eq!(st.selection, sel);
    let next = st.run("deleteSelection").expect("deletes");
    assert_eq!(ragged_tables(&next.doc), Vec::<String>::new());
    assert_eq!(
        serialize::node_to_html(&next.doc),
        "<table><tr><td><p>a</p></td><td><p>b</p></td></tr>\
         <tr><td><p>ch</p></td><td><p></p></td></tr>\
         <tr><td><p>ij</p></td><td><p>kl</p></td></tr></table><p>zw</p>"
    );
    // One undo puts it all back, the repair included.
    assert_eq!(next.run("undo").expect("undo").doc, doc);
    for name in EXTRA {
        let next = run_extra(&st, name).expect(name);
        assert_eq!(ragged_tables(&next.doc), Vec::<String>::new(), "{name}");
    }
}

#[test]
fn a_drag_from_a_cell_out_of_the_table_is_refused_and_keeps_the_table() {
    // The desktop drag gate makes a cell selection only between two cells of
    // one table; from "a|b" to "z|w" it stays a text selection with one end
    // outside every table. The fitted replace refuses that range ("inconsistent
    // open depths"), as it did before #1847, so no edit over it changes the
    // document, and none leaves the table ragged.
    let s = Rc::new(Schema::starter_kit());
    let doc = grid_doc(&s);
    for sel in [
        Selection::text(Pos(8), Pos(35)),
        Selection::text(Pos(35), Pos(8)),
    ] {
        let st = state(&s, doc.clone(), sel.clone());
        assert_eq!(st.selection, sel);
        for cmd in [
            "deleteSelection",
            "deleteCharBackward",
            "enter",
            "insertHardBreak",
        ] {
            assert!(st.run(cmd).is_none(), "{cmd}");
        }
        for name in EXTRA {
            if let Some(next) = run_extra(&st, name) {
                assert_eq!(next.doc, doc, "{name}");
            }
        }
    }
}

#[test]
fn a_table_that_was_ragged_before_the_edit_is_left_as_it_was() {
    // The same drag with the first table loaded ragged: it is not this
    // edit's doing, so nothing is repaired: exactly the raw delete.
    let s = Rc::new(Schema::starter_kit());
    let doc = two_tables(&s, true);
    assert_eq!(ragged_tables(&doc).len(), 1);
    let (from, to) = (at(&doc, "cd", 1), at(&doc, "gh", 1));
    let st = state(&s, doc.clone(), Selection::text(Pos(from), Pos(to)));
    let next = st.run("deleteSelection").expect("deletes");
    let mut raw = EditorState::create(s.clone(), doc, default_plugins()).tr();
    raw.delete(from, to).unwrap();
    assert_eq!(next.doc, *raw.doc());
    assert_eq!(
        serialize::node_to_html(&next.doc),
        "<table><tr><td><p>a</p></td><td><p>b</p></td></tr>\
         <tr><td><p>ch</p></td></tr>\
         <tr><td><p>ij</p></td><td><p>kl</p></td></tr></table><p>zw</p>"
    );
}

// ── seeded ──────────────────────────────────────────────────────────────────

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

fn random_block(s: &Schema, rng: &mut Rng, depth: usize) -> Node {
    let text = ["", "a", "bc d"][rng.below(3)];
    match rng.below(if depth >= 2 { 6 } else { 9 }) {
        0 | 1 => para(s, text),
        2 => leaf(s, "horizontal_rule"),
        3 => {
            let attrs = Attrs::from_iter([("level", AttrValue::Int(2))]);
            let content = if text.is_empty() {
                Fragment::empty()
            } else {
                Fragment::from_node(s.text(text).unwrap())
            };
            s.create_node("heading", attrs, content).unwrap()
        }
        4 => {
            let content = if text.is_empty() {
                Fragment::empty()
            } else {
                Fragment::from_node(s.text("let x;").unwrap())
            };
            s.create_node("code_block", Attrs::new(), content).unwrap()
        }
        5 => para(s, text),
        6 => {
            let items = (0..1 + rng.below(2))
                .map(|_| branch(s, "list_item", vec![para(s, text)]))
                .collect();
            branch(s, "bullet_list", items)
        }
        7 => branch(s, "blockquote", vec![para(s, text)]),
        _ => random_table(s, rng, depth + 1),
    }
}

fn cell_blocks(s: &Schema, rng: &mut Rng, depth: usize) -> Vec<Node> {
    (0..1 + rng.below(2))
        .map(|_| random_block(s, rng, depth + 1))
        .collect()
}

/// A rectangular table, sometimes with a merged cell (a colspan or a rowspan)
/// that tiles.
fn random_table(s: &Schema, rng: &mut Rng, depth: usize) -> Node {
    let cols = 1 + rng.below(3);
    let rows = 1 + rng.below(3);
    // The merged cell, if any: its top-left slot and its extent.
    let merge = if rng.below(2) == 0 && (cols > 1 || rows > 1) {
        let (top, left) = (rng.below(rows), rng.below(cols));
        let (h, w) = (1 + rng.below(rows - top), 1 + rng.below(cols - left));
        Some((top, left, h, w))
    } else {
        None
    };
    let rows = (0..rows)
        .map(|r| {
            let mut cells = Vec::new();
            for c in 0..cols {
                match merge {
                    Some((top, left, h, w)) if (top..top + h).contains(&r) => {
                        if r == top && c == left {
                            let blocks = cell_blocks(s, rng, depth);
                            cells.push(cell(s, w as i64, h as i64, blocks));
                        } else if !(left..left + w).contains(&c) {
                            cells.push(cell(s, 1, 1, cell_blocks(s, rng, depth)));
                        }
                    }
                    _ => cells.push(cell(s, 1, 1, cell_blocks(s, rng, depth))),
                }
            }
            branch(s, "table_row", cells)
        })
        .collect();
    branch(s, "table", rows)
}

fn random_doc(s: &Schema, rng: &mut Rng) -> Node {
    let mut blocks = Vec::new();
    if rng.below(2) == 0 {
        blocks.push(para(s, "x"));
    }
    let table = random_table(s, rng, 0);
    match rng.below(3) {
        0 => blocks.push(table),
        1 => blocks.push(branch(
            s,
            "bullet_list",
            vec![branch(s, "list_item", vec![para(s, "w"), table])],
        )),
        _ => blocks.push(branch(s, "blockquote", vec![table])),
    }
    if rng.below(3) > 0 {
        blocks.push(para(s, ["zw", ""][rng.below(2)]));
    }
    if rng.below(3) == 0 {
        blocks.push(random_table(s, rng, 0));
    }
    branch(s, "doc", blocks)
}

/// Every table part's position and kind (cell, row, table).
fn table_parts(doc: &Node) -> Vec<(usize, Node)> {
    let mut out = Vec::new();
    doc.nodes_between(0, doc.content().size(), &mut |n, pos, _| {
        if is_table(n) || is_row(n) || is_cell(n) {
            out.push((pos, n.clone()));
        }
        true
    });
    out
}

/// The innermost cell around `p` (`None` outside every table), and whether
/// `p` is in a textblock or at row/table level.
fn place(doc: &Node, p: usize) -> (Option<usize>, bool, bool) {
    let r = doc.resolve(Pos(p)).unwrap();
    let parent = r.parent();
    (
        tables::cell_around(&r),
        parent.is_textblock(),
        is_table(parent) || is_row(parent),
    )
}

/// The edits under test, beside the named commands: typing and two pastes,
/// each built the way `rinch-editor-view`'s `EditorHandle` builds it.
const EXTRA: [&str; 3] = ["type", "paste-inline", "paste-blocks"];

fn run_extra(st: &EditorState, name: &str) -> Option<EditorState> {
    let s = st.schema();
    let start = if matches!(st.selection, Selection::Cell(_)) {
        // `insert_text` / `replace_selection_slice`: clear the cells first.
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

#[test]
fn no_edit_over_a_cross_cell_or_table_part_selection_makes_a_ragged_table() {
    let s = Rc::new(Schema::starter_kit());
    let mut names: Vec<&'static str> = BaseCommandsPlugin
        .commands()
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    names.extend(EXTRA);
    let seeds: u64 = std::env::var("RINCH_1847_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    let (mut runs, mut applied, mut undone) = (0usize, 0usize, 0usize);
    let mut kinds = std::collections::BTreeMap::<&str, usize>::new();
    let mut changed = std::collections::BTreeMap::<String, usize>::new();
    let mut failures: Vec<(String, String)> = Vec::new();
    for seed in 1..=seeds {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ seed.wrapping_mul(0x2545_F491_4F6C_DD1D));
        let doc = random_doc(&s, &mut rng);
        assert!(
            valid(&s, &doc),
            "seed {seed}: generator made an invalid doc"
        );
        assert_eq!(
            ragged_tables(&doc),
            Vec::<String>::new(),
            "seed {seed}: generator made a ragged table: {}",
            serialize::node_to_html(&doc)
        );
        let size = doc.content().size();
        let mut selections: Vec<(&str, Selection)> = Vec::new();
        // Node selections of every cell, row and table.
        for (pos, node) in table_parts(&doc) {
            let sel = Selection::Node(NodeSelection {
                anchor: Pos(pos),
                head: Pos(pos + node.node_size()),
            });
            let kind = if is_cell(&node) {
                "node(cell)"
            } else if is_row(&node) {
                "node(row)"
            } else {
                "node(table)"
            };
            selections.push((kind, sel));
        }
        // Text selections across cells (an end may be outside every table),
        // with an end at row/table level, and — controls — inside one cell.
        let mut tries = 0;
        let (mut across, mut level, mut inside) = (0, 0, 0);
        while tries < 4000 && (across < 30 || level < 8 || inside < 6) {
            tries += 1;
            let (a, b) = (rng.below(size + 1), rng.below(size + 1));
            if a == b {
                continue;
            }
            let (ca, ta, la) = place(&doc, a);
            let (cb, tb, lb) = place(&doc, b);
            if (la || lb) && (ta || la) && (tb || lb) && level < 8 {
                level += 1;
                selections.push(("text(row/table level)", Selection::text(Pos(a), Pos(b))));
            } else if ta && tb && ca != cb && (ca.is_some() || cb.is_some()) && across < 30 {
                across += 1;
                selections.push(("text(across cells)", Selection::text(Pos(a), Pos(b))));
            } else if ta && tb && ca.is_some() && ca == cb && inside < 6 {
                inside += 1;
                selections.push(("text(one cell)", Selection::text(Pos(a), Pos(b))));
            }
        }
        // Cell selections between random cells of one table (a control).
        let cells: Vec<usize> = table_parts(&doc)
            .into_iter()
            .filter(|(_, n)| is_cell(n))
            .map(|(p, _)| p)
            .collect();
        for _ in 0..4 {
            let (a, b) = (cells[rng.below(cells.len())], cells[rng.below(cells.len())]);
            if tables::same_table(&doc, Pos(a + 1), Pos(b + 1)) {
                selections.push(("cell", Selection::cell(Pos(a), Pos(b))));
            }
        }
        for (kind, sel) in selections {
            *kinds.entry(kind).or_default() += 1;
            let st = state(&s, doc.clone(), sel.clone());
            for name in &names {
                runs += 1;
                let next = if EXTRA.contains(name) {
                    run_extra(&st, name)
                } else {
                    st.run(name)
                };
                let Some(next) = next else { continue };
                applied += 1;
                if next.doc != doc {
                    *changed.entry(format!("{kind} {name}")).or_default() += 1;
                }
                let mut fail = |what: &str, got: &Node| {
                    failures.push((
                        format!("{kind} {name}"),
                        format!(
                            "seed {seed} {sel:?} {name} {what}: {} -> {}",
                            serialize::node_to_html(&doc),
                            serialize::node_to_html(got)
                        ),
                    ))
                };
                let bad = ragged_tables(&next.doc);
                if !bad.is_empty() {
                    fail(&format!("({})", bad.join("; ")), &next.doc);
                    continue;
                }
                if !valid(&s, &next.doc) {
                    fail("(invalid)", &next.doc);
                    continue;
                }
                if next.doc == doc || EXTRA.contains(name) {
                    continue;
                }
                undone += 1;
                match next.run("undo") {
                    Some(u) if u.doc == doc => match u.run("redo") {
                        Some(r) if r.doc == next.doc => {}
                        Some(r) => fail("then undo, redo", &r.doc),
                        None => fail("then undo, redo refused", &u.doc),
                    },
                    Some(u) => fail("then undo", &u.doc),
                    None => fail("then undo refused", &next.doc),
                }
            }
        }
    }
    eprintln!("{runs} runs, {applied} applied, {undone} undone and redone; selections {kinds:?}");
    if std::env::var("RINCH_1847_STATS").is_ok() {
        eprintln!("edits that changed the doc: {changed:#?}");
    }
    // Positive controls: every kind of selection was made, and the edits ran.
    for kind in [
        "node(cell)",
        "node(row)",
        "node(table)",
        "text(across cells)",
        "text(row/table level)",
        "text(one cell)",
        "cell",
    ] {
        assert!(
            kinds.get(kind).copied().unwrap_or(0) > 10,
            "few {kind}: {kinds:?}"
        );
    }
    assert!(applied > runs / 10, "only {applied} of {runs} runs applied");
    assert!(
        undone > applied / 4,
        "only {undone} of {applied} edits undone"
    );
    let mut by_kind = std::collections::BTreeMap::<String, usize>::new();
    for (k, _) in &failures {
        *by_kind.entry(k.clone()).or_default() += 1;
    }
    failures.sort_by_key(|(_, f)| f.len());
    assert!(
        failures.is_empty(),
        "{} of {runs} runs left a ragged table or an invalid doc:\n{by_kind:#?}\nshortest per kind:\n{}",
        failures.len(),
        by_kind
            .keys()
            .filter_map(|k| failures
                .iter()
                .find(|(n, _)| n == k)
                .map(|(_, f)| f.clone()))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn a_cell_selection_copies_its_cells_with_spans_cut_to_the_rectangle() {
    // `a b / c(rowspan 2) d / e`: the rectangle from `a` to `d` takes the
    // top row of `c` only.
    let s = Rc::new(Schema::starter_kit());
    let row = |cells: Vec<Node>| branch(&s, "table_row", cells);
    let td = |rs: i64, t: &str| cell(&s, 1, rs, vec![para(&s, t)]);
    let table = branch(
        &s,
        "table",
        vec![
            row(vec![td(1, "a"), td(1, "b")]),
            row(vec![td(2, "c"), td(1, "d")]),
            row(vec![td(1, "e")]),
        ],
    );
    let doc = branch(&s, "doc", vec![table]);
    let (a, d) = (at(&doc, "a", 0) - 2, at(&doc, "d", 0) - 2);
    let st = state(&s, doc, Selection::cell(Pos(a), Pos(d)));
    let slice = commands::table_ops::cell_selection_slice(&st).expect("a cell selection");
    assert_eq!(
        serialize::slice_to_html(&slice),
        "<table><tr><td><p>a</p></td><td><p>b</p></td></tr>\
         <tr><td><p>c</p></td><td><p>d</p></td></tr></table>"
    );
}

// ── review of #1931 ─────────────────────────────────────────────────────────

#[test]
fn extending_a_cell_selection_keeps_its_anchor_cell() {
    // A shift+click builds `text(selection.anchor(), clicked)`; a cell
    // selection's anchor is the position before its cell. Anchored on "cd"
    // (11), a click in "ab" (8) is the rectangle from "cd" to "ab", not a
    // range in "ab".
    let s = Rc::new(Schema::starter_kit());
    for (from, click, want) in [
        (
            Selection::cell(Pos(11), Pos(11)),
            8,
            Selection::cell(Pos(11), Pos(5)),
        ),
        (
            Selection::cell(Pos(11), Pos(25)),
            8,
            Selection::cell(Pos(11), Pos(5)),
        ),
        (
            Selection::cell(Pos(19), Pos(19)),
            14,
            Selection::cell(Pos(19), Pos(11)),
        ),
        (
            Selection::cell(Pos(5), Pos(5)),
            28,
            Selection::cell(Pos(5), Pos(25)),
        ),
    ] {
        let st = state(&s, grid_doc(&s), from.clone());
        let mut tr = st.tr();
        tr.set_selection(Selection::text(st.selection.anchor(), Pos(click)));
        assert_eq!(st.apply(tr).selection, want, "{from:?} shift+click {click}");
    }
}

#[test]
fn select_all_in_a_table_only_document_stays_a_text_selection() {
    // ProseMirror's select-all is an `AllSelection`, never a cell selection.
    let s = Rc::new(Schema::starter_kit());
    let row = |a: &str, b: &str| {
        branch(
            &s,
            "table_row",
            vec![
                cell(&s, 1, 1, vec![para(&s, a)]),
                cell(&s, 1, 1, vec![para(&s, b)]),
            ],
        )
    };
    let doc = branch(
        &s,
        "doc",
        vec![branch(&s, "table", vec![row("ab", "cd"), row("ef", "gh")])],
    );
    let st = state(&s, doc.clone(), Selection::cursor(Pos(4)));
    let all = st.run("selectAll").expect("selectAll");
    assert!(
        matches!(all.selection, Selection::Text(_)),
        "{:?}",
        all.selection
    );
    // Control: the same extent short of the last character is a cell selection.
    let to = all.selection.to().0;
    let near = state(&s, doc, Selection::text(all.selection.from(), Pos(to - 1)));
    assert!(matches!(near.selection, Selection::Cell(_)));
    // Backspace deletes the range, as at main: the text from the first cell's
    // to the last cell's, which leaves one empty cell (not the cleared 2x2 a
    // cell selection would leave).
    let gone = all.run("deleteCharBackward").expect("deletes");
    assert_eq!(
        serialize::node_to_html(&gone.doc),
        "<table><tr><td><p></p></td></tr></table>"
    );
}

#[test]
fn a_ragged_table_pasted_at_a_caret_is_padded_as_over_a_range() {
    let s = Rc::new(Schema::starter_kit());
    let doc = branch(&s, "doc", vec![para(&s, "x"), para(&s, "yz")]);
    let html =
        "<table><tr><td><p>a</p></td><td><p>b</p></td></tr><tr><td><p>c</p></td></tr></table>";
    for sel in [Selection::cursor(Pos(4)), Selection::text(Pos(4), Pos(6))] {
        let st = state(&s, doc.clone(), sel.clone());
        let mut tr = st.tr();
        tr.replace_selection(serialize::slice_from_html(&s, html).unwrap())
            .unwrap();
        let next = st.apply(tr);
        assert_eq!(ragged_tables(&next.doc), Vec::<String>::new(), "{sel:?}");
        assert!(
            serialize::node_to_html(&next.doc)
                .contains("<tr><td><p>c</p></td><td><p></p></td></tr>"),
            "{sel:?}: {}",
            serialize::node_to_html(&next.doc)
        );
    }
}

#[test]
fn inserts_over_a_cell_selection_clear_the_cells_and_go_in_the_first() {
    // prosemirror-tables' `CellSelection.replace`: what typing and paste do.
    let s = Rc::new(Schema::starter_kit());
    let st = state(&s, grid_doc(&s), Selection::cell(Pos(5), Pos(25)));
    let empty = "<td><p></p></td>";
    for (cmd, first) in [
        ("insertHardBreak", "<td><p><br></p></td>".to_string()),
        ("insertHorizontalRule", "<td><hr><p></p></td>".to_string()),
        (
            "insertTable",
            format!(
                "<td><table>{}</table><p></p></td>",
                format!("<tr>{}</tr>", empty.repeat(3)).repeat(3)
            ),
        ),
    ] {
        let next = st.run(cmd).unwrap_or_else(|| panic!("{cmd} applies"));
        assert_eq!(
            serialize::node_to_html(&next.doc),
            format!(
                "<p>x</p><table><tr>{first}{empty}</tr><tr>{empty}{empty}</tr></table><p>zw</p>"
            ),
            "{cmd}"
        );
        assert_eq!(next.run("undo").expect("undo").doc, grid_doc(&s), "{cmd}");
    }
}

#[test]
fn marks_block_types_and_alignment_reach_every_selected_cell() {
    // A cell selection's `from()..to()` stops before its last cell; the
    // commands walk `Selection::ranges`, one per cell (PM `selection.ranges`).
    let s = Rc::new(Schema::starter_kit());
    // The right column only: "cd" and "gh".
    let st = state(&s, grid_doc(&s), Selection::cell(Pos(11), Pos(25)));
    let html = |cmd: &str| serialize::node_to_html(&st.run(cmd).expect(cmd).doc);
    assert_eq!(
        html("toggleBold"),
        "<p>x</p><table><tr><td><p>ab</p></td><td><p><strong>cd</strong></p></td></tr>\
         <tr><td><p>ef</p></td><td><p><strong>gh</strong></p></td></tr></table><p>zw</p>"
    );
    assert_eq!(
        html("setHeading2"),
        "<p>x</p><table><tr><td><p>ab</p></td><td><h2>cd</h2></td></tr>\
         <tr><td><p>ef</p></td><td><h2>gh</h2></td></tr></table><p>zw</p>"
    );
    let centred = st.run("setTextAlignCenter").expect("align");
    let aligned: Vec<Option<String>> = ["ab", "cd", "ef", "gh"]
        .iter()
        .map(|t| {
            let p = at(&centred.doc, t, 0) - 1;
            centred
                .doc
                .node_at(p)
                .unwrap()
                .attrs()
                .get_str("text_align")
                .map(str::to_string)
        })
        .collect();
    assert_eq!(
        aligned,
        [None, Some("center".into()), None, Some("center".into())]
    );
    // Toggling again removes the mark from both cells; the toolbar state
    // reads every cell.
    let bold = st.run("toggleBold").unwrap();
    let mt = s.mark_type("bold").unwrap();
    assert!(commands::is_mark_active(&bold, mt));
    assert_eq!(bold.run("toggleBold").unwrap().doc, grid_doc(&s));
    // Ranges: one per cell, in document order.
    assert_eq!(
        st.selection.ranges(&st.doc),
        vec![(Pos(12), Pos(16)), (Pos(26), Pos(30))]
    );
}
