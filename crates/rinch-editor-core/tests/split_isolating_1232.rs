//! #1232: a split never cuts an isolating node (a table cell).
//!
//! With a text cursor at a gap between blocks inside a cell (between a
//! horizontal rule and a paragraph), `splitBlock` and `enter` split the *cell*
//! and left a row with one cell more than the table has columns: a ragged
//! table, which stalls a collaborating editor's outbound. `liftEmptyBlock`
//! (Enter in an empty paragraph that is not its cell's last block) and
//! `insertHorizontalRule` (at such a gap) did the same.
//!
//! ProseMirror's `canSplit` refuses any split whose parent, or any node the
//! split cuts, is `isolating`; `createParagraphNear` (the second command of its
//! Enter chain) then puts an empty paragraph at the gap.
//!
//! The seeded half runs every built-in command at every position (a caret at
//! each position, a node selection of each leaf atom, random ranges inside one
//! cell) of random
//! documents holding tables (and lists, task lists, quotes, code blocks,
//! rules, embeds and pictures), and asserts no result is a ragged table or an
//! invalid document, and that each edit undoes to the document it was made on
//! and redoes to itself. `RINCH_SPLIT_FUZZ_SEEDS` raises the seed count.

use std::rc::Rc;

use rinch_editor_core::schema::validation::validate_content;
use rinch_editor_core::tables::{self, is_table, row_holes};
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

fn state(s: &Rc<Schema>, doc: Node, selection: Selection) -> EditorState {
    let mut st = EditorState::create(s.clone(), doc, default_plugins());
    st.selection = selection;
    st
}

/// Every table's rows cover the same number of grid columns.
fn ragged_tables(doc: &Node) -> Vec<String> {
    let mut bad = Vec::new();
    doc.nodes_between(0, doc.content().size(), &mut |n, pos, _| {
        if is_table(n) && row_holes(n).iter().any(|h| *h > 0) {
            bad.push(format!("table at {pos}: {}", serialize::node_to_html(n)));
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

/// `doc(table(row(cell(hr, p()), cell(p("b")))), p("zw"))`: the cell opens at 2,
/// the rule is 3..4, the gap the issue names is 4, the empty paragraph 4..6.
fn issue_doc(s: &Schema) -> Node {
    let c1 = branch(
        s,
        "table_cell",
        vec![leaf(s, "horizontal_rule"), para(s, "")],
    );
    let c2 = branch(s, "table_cell", vec![para(s, "b")]);
    let row = branch(s, "table_row", vec![c1, c2]);
    let table = branch(s, "table", vec![row]);
    branch(s, "doc", vec![table, para(s, "zw")])
}

#[test]
fn split_block_at_a_gap_inside_a_cell_does_not_split_the_cell() {
    let s = Rc::new(Schema::starter_kit());
    let st = state(&s, issue_doc(&s), Selection::cursor(Pos(4)));
    // ProseMirror's `splitBlock` refuses: the split's parent is the cell.
    assert!(
        st.run("splitBlock").is_none(),
        "splitBlock must refuse at a cell gap"
    );
    assert!(!st.can_run("splitBlock"));
}

#[test]
fn enter_at_a_gap_inside_a_cell_puts_a_paragraph_there() {
    let s = Rc::new(Schema::starter_kit());
    let st = state(&s, issue_doc(&s), Selection::cursor(Pos(4)));
    let next = st.run("enter").expect("enter makes a paragraph at the gap");
    assert_eq!(ragged_tables(&next.doc), Vec::<String>::new());
    assert_eq!(
        serialize::node_to_html(&next.doc),
        "<table><tr><td><hr><p></p><p></p></td><td><p>b</p></td></tr></table><p>zw</p>"
    );
    // The caret is in the new paragraph (createParagraphNear).
    assert_eq!(next.selection, Selection::cursor(Pos(5)));
    let r = next.doc.resolve(Pos(5)).unwrap();
    assert!(r.parent().is_textblock());
}

#[test]
fn enter_in_an_empty_paragraph_before_another_block_of_its_cell_keeps_the_cell() {
    // `liftEmptyBlock` split the grandparent before an empty block that is not
    // its parent's last child — here the cell.
    let s = Rc::new(Schema::starter_kit());
    // Not its cell's first block either: a split of the cell at offset 0 is
    // refused by the content check (an empty cell), which hid the bug.
    let c1 = branch(
        &s,
        "table_cell",
        vec![para(&s, "x"), para(&s, ""), para(&s, "y")],
    );
    let c2 = branch(&s, "table_cell", vec![para(&s, "b")]);
    let table = branch(&s, "table", vec![branch(&s, "table_row", vec![c1, c2])]);
    let doc = branch(&s, "doc", vec![table, para(&s, "zw")]);
    // cell opens at 2, p("x") is 3..6, the empty paragraph 6..8, its inside 7.
    let st = state(&s, doc, Selection::cursor(Pos(7)));
    let next = st.run("enter").expect("enter still splits the paragraph");
    assert_eq!(ragged_tables(&next.doc), Vec::<String>::new());
    assert_eq!(
        serialize::node_to_html(&next.doc),
        "<table><tr><td><p>x</p><p></p><p></p><p>y</p></td><td><p>b</p></td></tr></table><p>zw</p>"
    );
}

#[test]
fn a_rule_inserted_at_a_gap_inside_a_cell_keeps_the_cell() {
    let s = Rc::new(Schema::starter_kit());
    let st = state(&s, issue_doc(&s), Selection::cursor(Pos(4)));
    let next = st
        .run("insertHorizontalRule")
        .expect("the rule goes in at the gap");
    assert_eq!(ragged_tables(&next.doc), Vec::<String>::new());
    assert_eq!(
        serialize::node_to_html(&next.doc),
        "<table><tr><td><hr><hr><p></p></td><td><p>b</p></td></tr></table><p>zw</p>"
    );
}

#[test]
fn enter_in_a_paragraph_inside_a_cell_still_splits_the_paragraph() {
    // Positive control: the refusal is for the cell, not for every split in it.
    let s = Rc::new(Schema::starter_kit());
    let c1 = branch(&s, "table_cell", vec![para(&s, "ab")]);
    let table = branch(&s, "table", vec![branch(&s, "table_row", vec![c1])]);
    let doc = branch(&s, "doc", vec![table]);
    // cell at 2, paragraph at 3, "a|b" at 5.
    let st = state(&s, doc, Selection::cursor(Pos(5)));
    for cmd in ["splitBlock", "enter"] {
        let next = st.run(cmd).expect("splits the paragraph");
        assert_eq!(
            serialize::node_to_html(&next.doc),
            "<table><tr><td><p>a</p><p>b</p></td></tr></table>",
            "{cmd}"
        );
    }
}

#[test]
fn backspace_at_the_start_of_a_paragraph_after_a_table_adds_no_row() {
    // Found by the seeded check below: the join's find-wrapping branch wrapped
    // the paragraph in a new one-cell row of the table before it.
    let s = Rc::new(Schema::starter_kit());
    let c1 = branch(&s, "table_cell", vec![para(&s, "a")]);
    let c2 = branch(&s, "table_cell", vec![para(&s, "b")]);
    let table = branch(&s, "table", vec![branch(&s, "table_row", vec![c1, c2])]);
    let doc = branch(&s, "doc", vec![table, para(&s, "zw")]);
    // table 0..14, "zw" starts at 15.
    let st = state(&s, doc, Selection::cursor(Pos(15)));
    for cmd in ["deleteCharBackward", "deleteWordBackward"] {
        if let Some(next) = st.run(cmd) {
            assert_eq!(ragged_tables(&next.doc), Vec::<String>::new(), "{cmd}");
        }
    }
}

fn two_cell_table(s: &Schema, a: &str, b: &str) -> Node {
    let c1 = branch(s, "table_cell", vec![para(s, a)]);
    let c2 = branch(s, "table_cell", vec![para(s, b)]);
    branch(s, "table", vec![branch(s, "table_row", vec![c1, c2])])
}

#[test]
fn backspace_in_an_empty_paragraph_after_a_table_deletes_it_and_enters_the_table() {
    // ProseMirror's `joinBackward` empty-block branch: the paragraph goes, the
    // caret lands at the end of the last cell's last textblock.
    let s = Rc::new(Schema::starter_kit());
    let doc = branch(&s, "doc", vec![two_cell_table(&s, "a", "bc"), para(&s, "")]);
    // table 0..15 ("bc" is 10..12 inside the second cell's paragraph), p 15..17.
    let st = state(&s, doc.clone(), Selection::cursor(Pos(16)));
    let next = st.run("deleteCharBackward").expect("backspace applies");
    assert_eq!(
        serialize::node_to_html(&next.doc),
        "<table><tr><td><p>a</p></td><td><p>bc</p></td></tr></table>"
    );
    // The second cell's paragraph is 8..12, its text 9..11.
    assert_eq!(next.selection, Selection::cursor(Pos(11)));
    let undone = next.run("undo").expect("undo applies");
    assert_eq!(undone.doc, doc);
}

#[test]
fn backspace_at_the_start_of_a_paragraph_after_a_table_selects_the_table() {
    // `selectNodeBackward`; a second Backspace deletes it, undo puts it back.
    let s = Rc::new(Schema::starter_kit());
    let doc = branch(
        &s,
        "doc",
        vec![two_cell_table(&s, "a", "bc"), para(&s, "zw")],
    );
    let st = state(&s, doc.clone(), Selection::cursor(Pos(16)));
    let selected = st
        .run("deleteCharBackward")
        .expect("backspace selects the table");
    assert_eq!(selected.doc, doc);
    assert_eq!(
        selected.selection,
        Selection::Node(NodeSelection {
            anchor: Pos(0),
            head: Pos(15)
        })
    );
    let deleted = selected
        .run("deleteCharBackward")
        .expect("backspace deletes it");
    assert_eq!(serialize::node_to_html(&deleted.doc), "<p>zw</p>");
    let undone = deleted.run("undo").expect("undo applies");
    assert_eq!(undone.doc, doc);
}

#[test]
fn backspace_after_a_table_inside_a_list_item_or_a_quote_lifts_nothing() {
    // `li(p w, table, p x)`: the paragraph is not lifted out of the item (it
    // was, after the first round of #1232); the table is selected.
    let s = Rc::new(Schema::starter_kit());
    for (wrap, inner) in [("list_item", true), ("blockquote", false)] {
        let item = branch(
            &s,
            wrap,
            vec![para(&s, "w"), two_cell_table(&s, "a", "bc"), para(&s, "x")],
        );
        let outer = if inner {
            branch(&s, "bullet_list", vec![item])
        } else {
            item
        };
        let doc = branch(&s, "doc", vec![outer]);
        // The table starts at 2 + 3 inside the list (ul, li) or 1 + 3 in a quote.
        let table_pos = if inner { 5 } else { 4 };
        let caret = table_pos + 15 + 1;
        let st = state(&s, doc.clone(), Selection::cursor(Pos(caret)));
        let next = st.run("deleteCharBackward").expect("backspace applies");
        assert_eq!(next.doc, doc, "{wrap}: nothing lifted");
        assert_eq!(
            next.selection,
            Selection::Node(NodeSelection {
                anchor: Pos(table_pos),
                head: Pos(table_pos + 15)
            }),
            "{wrap}"
        );
    }
}

#[test]
fn delete_at_the_end_of_a_tables_last_row_does_not_join_the_next_table() {
    // Tables of two and of one column, side by side at the document level;
    // the caret at the end of the first table's last row (13, a position the
    // seeded check reaches), whose cut is the gap between the tables. A join
    // there made a ragged table.
    let s = Rc::new(Schema::starter_kit());
    let one = branch(
        &s,
        "table",
        vec![branch(
            &s,
            "table_row",
            vec![branch(&s, "table_cell", vec![para(&s, "c")])],
        )],
    );
    let doc = branch(
        &s,
        "doc",
        vec![two_cell_table(&s, "a", "bc"), one, para(&s, "")],
    );
    let st = state(&s, doc, Selection::cursor(Pos(13)));
    for cmd in [
        "deleteCharForward",
        "deleteWordForward",
        "deleteCharBackward",
    ] {
        if let Some(next) = st.run(cmd) {
            assert_eq!(ragged_tables(&next.doc), Vec::<String>::new(), "{cmd}");
        }
    }
}

// ── seeded: no command, at no position, makes a ragged table ────────────────

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

fn image(s: &Schema) -> Node {
    s.create_node(
        "image",
        Attrs::from_iter([("src", AttrValue::Str("i.png".into()))]),
        Fragment::empty(),
    )
    .unwrap()
}

fn random_block(s: &Schema, rng: &mut Rng, depth: usize) -> Node {
    let text = ["", "", "a", "bc d"][rng.below(4)];
    match rng.below(if depth >= 2 { 7 } else { 13 }) {
        0 => para(s, text),
        1 => {
            // A paragraph holding a picture.
            let mut kids = vec![image(s)];
            if !text.is_empty() {
                kids.insert(0, s.text(text).unwrap());
            }
            s.branch("paragraph", Fragment::from_children(kids))
                .unwrap()
        }
        4 => {
            let content = if text.is_empty() {
                Fragment::empty()
            } else {
                Fragment::from_node(s.text("let x;\ny").unwrap())
            };
            s.create_node("code_block", Attrs::new(), content).unwrap()
        }
        5 => leaf(s, "embed"),
        6 => para(s, text),
        7 => {
            let items = (0..1 + rng.below(2))
                .map(|_| {
                    let checked = AttrValue::Bool(rng.below(2) == 0);
                    s.create_node(
                        "task_item",
                        Attrs::from_iter([("checked", checked)]),
                        Fragment::from_children(random_blocks(s, rng, depth + 1)),
                    )
                    .unwrap()
                })
                .collect();
            branch(s, "task_list", items)
        }
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
        8 | 9 => {
            let items = (0..1 + rng.below(2))
                .map(|_| branch(s, "list_item", random_blocks(s, rng, depth + 1)))
                .collect();
            branch(s, "bullet_list", items)
        }
        10 => branch(s, "blockquote", random_blocks(s, rng, depth + 1)),
        _ => random_table(s, rng, depth + 1),
    }
}

/// One or more blocks, the first a paragraph when it would otherwise be a
/// node a list item may not start with.
fn random_blocks(s: &Schema, rng: &mut Rng, depth: usize) -> Vec<Node> {
    let mut out: Vec<Node> = (0..1 + rng.below(3))
        .map(|_| random_block(s, rng, depth))
        .collect();
    if !out[0].is_textblock() {
        out.insert(0, para(s, ""));
    }
    out
}

fn random_table(s: &Schema, rng: &mut Rng, depth: usize) -> Node {
    let cols = 1 + rng.below(3);
    let rows = (0..1 + rng.below(2))
        .map(|_| {
            let cells = (0..cols)
                .map(|_| {
                    let mut blocks: Vec<Node> = (0..1 + rng.below(3))
                        .map(|_| random_block(s, rng, depth + 1))
                        .collect();
                    // A cell may start with a rule: the issue's shape.
                    if rng.below(2) == 0 {
                        blocks.insert(0, leaf(s, "horizontal_rule"));
                    }
                    branch(s, "table_cell", blocks)
                })
                .collect();
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
    // A table with something after it, at the top level or inside a list item
    // or a quote (Backspace "after a table").
    let after = [para(s, "zw"), para(s, ""), leaf(s, "embed")][rng.below(3)].clone();
    let table = random_table(s, rng, 0);
    match rng.below(3) {
        0 => {
            blocks.push(table);
            if rng.below(3) > 0 {
                blocks.push(after);
            }
        }
        1 => blocks.push(branch(
            s,
            "bullet_list",
            vec![branch(s, "list_item", vec![para(s, "w"), table, after])],
        )),
        _ => blocks.push(branch(s, "blockquote", vec![table, after])),
    }
    if rng.below(2) == 0 {
        blocks.push(random_table(s, rng, 0));
    }
    branch(s, "doc", blocks)
}

#[test]
fn no_command_at_any_position_makes_a_ragged_table() {
    let s = Rc::new(Schema::starter_kit());
    let names: Vec<&'static str> = BaseCommandsPlugin
        .commands()
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    let seeds: u64 = std::env::var("RINCH_SPLIT_FUZZ_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let mut runs = 0usize;
    let mut applied = 0usize;
    let mut undone = 0usize;
    let mut failures = Vec::new();
    for seed in 1..=seeds {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ seed.wrapping_mul(0x2545_F491_4F6C_DD1D));
        let doc = random_doc(&s, &mut rng);
        assert!(
            valid(&s, &doc),
            "seed {seed}: generator made an invalid doc"
        );
        assert!(
            ragged_tables(&doc).is_empty(),
            "seed {seed}: generator made a ragged table"
        );
        let size = doc.content().size();
        // What a person can make: a caret anywhere, a node selection of a leaf
        // atom, and a range inside one cell (a drag across cells is a cell
        // selection). A range across cells and a node selection of a cell or row
        // are not here: #1847.
        let mut selections: Vec<Selection> =
            (0..=size).map(|p| Selection::cursor(Pos(p))).collect();
        selections.extend((0..=size).filter_map(|p| Selection::node_at(&doc, Pos(p))));
        // A range's ends are in textblocks of one innermost cell.
        let cell_of = |p: usize| {
            let r = doc.resolve(Pos(p)).ok()?;
            if !r.parent().is_textblock() {
                return None;
            }
            tables::cell_around(&r)
        };
        let mut ranges = 0;
        while ranges < 40 {
            let a = rng.below(size + 1);
            let b = rng.below(size + 1);
            if a != b && cell_of(a).is_some() && cell_of(a) == cell_of(b) {
                selections.push(Selection::text(Pos(a), Pos(b)));
            }
            ranges += 1;
        }
        for sel in selections {
            let st = state(&s, doc.clone(), sel.clone());
            for name in &names {
                runs += 1;
                let Some(next) = st.run(name) else { continue };
                applied += 1;
                let mut fail = |what: &str, got: &Node| {
                    failures.push((
                        name.to_string(),
                        format!(
                            "seed {seed} {sel:?} {name} {what}: {} -> {}",
                            serialize::node_to_html(&doc),
                            serialize::node_to_html(got)
                        ),
                    ))
                };
                if !ragged_tables(&next.doc).is_empty() {
                    fail("", &next.doc);
                    continue;
                }
                if !valid(&s, &next.doc) {
                    // #1852 (setCodeBlock keeping a picture a code block cannot
                    // hold) is fixed by #1846's `clear_incompatible`, so every
                    // command must leave a valid document.
                    fail("(invalid)", &next.doc);
                    continue;
                }
                if next.doc == doc {
                    continue;
                }
                // The edit undoes to the document it was made on and redoes to
                // itself, with no ragged table on the way.
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
    // Positive control: the commands did run and change things.
    assert!(applied > runs / 20, "only {applied} of {runs} runs applied");
    assert!(
        undone > applied / 4,
        "only {undone} of {applied} edits undone"
    );
    eprintln!("{runs} runs, {applied} applied, {undone} undone and redone");
    let mut by_command = std::collections::BTreeMap::<String, usize>::new();
    for (name, _) in &failures {
        *by_command.entry(name.clone()).or_default() += 1;
    }
    failures.sort_by_key(|(_, f)| f.len());
    assert!(
        failures.is_empty(),
        "{} of {runs} runs left a ragged table or an invalid doc ({by_command:?}); shortest:\n{}",
        failures.len(),
        by_command
            .keys()
            .filter_map(|c| failures
                .iter()
                .find(|(n, _)| n == c)
                .map(|(_, f)| f.clone()))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
