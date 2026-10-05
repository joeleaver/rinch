//! Seeded fuzz for the fitted replace (`Transform::replace_range`, #1382):
//! a random slice cut from a random document, pasted over a random range of
//! another, must never panic and never leave an invalid document, and the step
//! it adds must undo to the document it was applied to.
//!
//! Content as the HTML reader gives it must also always fit at a caret, or a
//! selection, inside one textblock.
//!
//! `RINCH_FIT_FUZZ_SEEDS` raises the seed count (default 4000);
//! `RINCH_FIT_FUZZ_SHOW=<0..3>` prints the refusals of one kind of slice.

use rinch_editor_core::serialize::{clipboard_slice, slice_from_html};
use rinch_editor_core::transform::Transform;
use rinch_editor_core::{EditorState, Fragment, Node, Pos, Schema, Selection, Slice};
use std::rc::Rc;

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
    fn chance(&mut self, pct: usize) -> bool {
        self.below(100) < pct
    }
}

fn inline(rng: &mut Rng, out: &mut String) {
    for _ in 0..rng.below(4) {
        let text = ["a", "bc", "d e", "xyz"][rng.below(4)];
        match rng.below(8) {
            0 => out.push_str(&format!("<strong>{text}</strong>")),
            1 => out.push_str(&format!("<em>{text}</em>")),
            2 => out.push_str(&format!("<a href=\"https://e.x/\">{text}</a>")),
            3 => out.push_str("<br>"),
            4 => out.push_str("<img src=\"https://e.x/i.png\" alt=\"i\">"),
            _ => out.push_str(text),
        }
    }
}

fn blocks(rng: &mut Rng, depth: usize, out: &mut String) {
    for _ in 0..1 + rng.below(3) {
        let pick = if depth >= 3 {
            rng.below(4)
        } else {
            rng.below(12)
        };
        match pick {
            0 | 1 => {
                out.push_str("<p>");
                inline(rng, out);
                out.push_str("</p>");
            }
            2 => {
                let level = 1 + rng.below(3);
                out.push_str(&format!("<h{level}>"));
                inline(rng, out);
                out.push_str(&format!("</h{level}>"));
            }
            3 => out.push_str(if rng.chance(50) {
                "<hr>"
            } else {
                "<pre>let x;\ny</pre>"
            }),
            4 | 5 => {
                let tag = if rng.chance(50) { "ul" } else { "ol" };
                out.push_str(&format!("<{tag}>"));
                for _ in 0..1 + rng.below(3) {
                    out.push_str("<li>");
                    blocks(rng, depth + 1, out);
                    out.push_str("</li>");
                }
                out.push_str(&format!("</{tag}>"));
            }
            6 | 7 => {
                out.push_str("<ul data-type=\"taskList\">");
                for _ in 0..1 + rng.below(3) {
                    let checked = rng.chance(50);
                    out.push_str(&format!(
                        "<li data-type=\"taskItem\" data-checked=\"{checked}\">"
                    ));
                    blocks(rng, depth + 1, out);
                    out.push_str("</li>");
                }
                out.push_str("</ul>");
            }
            8 | 9 => {
                out.push_str("<blockquote>");
                blocks(rng, depth + 1, out);
                out.push_str("</blockquote>");
            }
            _ => {
                out.push_str("<table>");
                let cols = 1 + rng.below(3);
                for _ in 0..1 + rng.below(2) {
                    out.push_str("<tr>");
                    for _ in 0..cols {
                        out.push_str("<td>");
                        blocks(rng, depth + 2, out);
                        out.push_str("</td>");
                    }
                    out.push_str("</tr>");
                }
                out.push_str("</table>");
            }
        }
    }
}

fn random_doc(schema: &Schema, rng: &mut Rng) -> Node {
    let mut html = String::new();
    blocks(rng, 0, &mut html);
    let slice = slice_from_html(schema, &html).expect("generated html parses");
    normalized(&schema.branch("doc", slice.content).expect("a valid doc"))
}

/// `node` with adjacent text runs of the same marks joined, as every edit
/// leaves them (the HTML reader keeps `<b>a</b><b>b</b>` as two runs, and an
/// undo would then differ from the original by that alone).
fn normalized(node: &Node) -> Node {
    let mut content = Fragment::empty();
    for child in node.content().children() {
        content = content.append(&Fragment::from_node(normalized(child)));
    }
    if node.is_text() {
        node.clone()
    } else {
        node.copy_with_content(content)
    }
}

/// Every node's content satisfies its type's expression and carries only the
/// marks its parent allows.
fn assert_valid(node: &Node, what: &str) {
    let names: Vec<&str> = node
        .content()
        .children()
        .iter()
        .map(Node::type_name)
        .collect();
    assert!(
        node.node_type().content_match().matches(&names),
        "{what}: <{}> holds {names:?}",
        node.type_name()
    );
    for child in node.content().children() {
        for mark in child.marks() {
            assert!(
                node.node_type().spec().marks.allows(mark.type_name()),
                "{what}: <{}> holds a {} mark",
                node.type_name(),
                mark.type_name()
            );
        }
        assert_valid(child, what);
    }
}

fn random_range(rng: &mut Rng, doc: &Node) -> (usize, usize) {
    let size = doc.content_size();
    let a = rng.below(size + 1);
    let b = if rng.chance(60) {
        a
    } else {
        rng.below(size + 1)
    };
    (a.min(b), a.max(b))
}

/// The positions before two cells of one row of `doc` (possibly the same
/// cell), when it has a table: the corners of a cell selection.
fn two_cells(doc: &Node, rng: &mut Rng) -> Option<(usize, usize)> {
    let mut rows: Vec<Vec<usize>> = Vec::new();
    doc.nodes_between(0, doc.content_size(), &mut |node, pos, _| {
        if node.type_name() == "table_row" {
            let mut at = pos + 1;
            let mut cells = Vec::new();
            for cell in node.content().children() {
                cells.push(at);
                at += cell.node_size();
            }
            rows.push(cells);
        }
        true
    });
    rows.retain(|r| !r.is_empty());
    if rows.is_empty() {
        return None;
    }
    let row = &rows[rng.below(rows.len())];
    Some((row[rng.below(row.len())], row[rng.below(row.len())]))
}

fn random_slice(schema: &Schema, rng: &mut Rng) -> Option<(usize, Slice)> {
    let source = random_doc(schema, rng);
    let kind = rng.below(4);
    let slice = match kind {
        // What a copy puts on the clipboard, read back as a paste reads it.
        0 => {
            let (from, to) = random_range(rng, &source);
            let slice = clipboard_slice(&source, from, to).ok()?;
            let html = rinch_editor_core::serialize::slice_to_html(&slice);
            slice_from_html(schema, &html).ok()?
        }
        // The cut itself, open depths as they are.
        1 => {
            let (from, to) = random_range(rng, &source);
            source.slice(from, to).ok()?
        }
        // Whole blocks, closed.
        2 => Slice::from_fragment(source.content().clone()),
        // The cut with open depths it does not have.
        _ => {
            let (from, to) = random_range(rng, &source);
            let slice = source.slice(from, to).ok()?;
            let open = |rng: &mut Rng, max: usize| if max == 0 { 0 } else { rng.below(max + 1) };
            let (s, e) = (open(rng, slice.open_start), open(rng, slice.open_end));
            Slice::new(slice.content, s, e)
        }
    };
    Some((kind, slice))
}

#[test]
fn a_fitted_replace_never_leaves_an_invalid_document() {
    let seeds: u64 = std::env::var("RINCH_FIT_FUZZ_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4000);
    let schema = Rc::new(Schema::starter_kit());
    let (mut applied, mut refused, mut around) = (0usize, 0usize, 0usize);
    let mut by_kind = [(0usize, 0usize); 4];
    let mut cell_selections = 0usize;
    for seed in 1..=seeds {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let doc = random_doc(&schema, &mut rng);
        assert_valid(&doc, "the generated document");
        let Some((kind, slice)) = random_slice(&schema, &mut rng) else {
            continue;
        };
        let (from, to) = random_range(&mut rng, &doc);
        if doc.resolve(Pos(from)).is_err() || doc.resolve(Pos(to)).is_err() {
            continue;
        }
        let what = format!("seed {seed}: {slice:?} over {from}..{to} of {doc:?}");

        let mut tf = Transform::new(&schema, doc.clone());
        match tf.replace_range(from, to, slice.clone()) {
            Ok(end) => {
                applied += 1;
                by_kind[kind].0 += 1;
                assert_valid(&tf.doc, &what);
                assert!(end <= tf.doc.content_size(), "{what}: end {end}");
                assert!(tf.doc.resolve(Pos(end)).is_ok(), "{what}: end {end}");
                assert!(tf.steps().len() <= 1, "{what}: one step");
                // The step undoes to the document it was applied to.
                if let Some(step) = tf.steps().last() {
                    let undone = step
                        .invert(&doc)
                        .apply(&tf.doc)
                        .unwrap_or_else(|e| panic!("{what}: the inverse does not apply: {e}"));
                    assert_eq!(undone, doc, "{what}: undo");
                    if format!("{step:?}").starts_with("ReplaceAroundStep") {
                        around += 1;
                    }
                }
            }
            Err(e) => {
                // A paste — content as the HTML reader gives it — at a caret
                // in a textblock always has a place.
                // (over a selection inside one textblock included)
                let caret_in_text = match (doc.resolve(Pos(from)), doc.resolve(Pos(to))) {
                    (Ok(a), Ok(b)) => {
                        a.parent().is_textblock()
                            && a.depth() == b.depth()
                            && a.start(a.depth()) == b.start(b.depth())
                    }
                    _ => false,
                };
                assert!(
                    !(caret_in_text && matches!(kind, 0 | 2) && slice.size() > 0),
                    "{what}: refused: {e}"
                );
                refused += 1;
                by_kind[kind].1 += 1;
                if std::env::var("RINCH_FIT_FUZZ_SHOW").is_ok_and(|k| k == kind.to_string()) {
                    eprintln!("REFUSED {what}");
                }
                assert_eq!(tf.doc, doc, "{what}: a refusal changes nothing");
                assert!(tf.steps().is_empty(), "{what}");
            }
        }

        // Over a cell selection, when the document has two cells in one
        // row: the state layer refuses, whatever the slice.
        if let Some((a, b)) = two_cells(&doc, &mut rng) {
            cell_selections += 1;
            let state = EditorState::create(schema.clone(), doc.clone(), Vec::new());
            let mut tr = state.tr();
            tr.set_selection(Selection::cell(Pos(a), Pos(b)));
            assert!(
                tr.replace_selection(slice.clone()).is_err(),
                "{what}: a cell selection {a}..{b} was replaced"
            );
            assert_eq!(tr.doc(), &doc, "{what}: cell selection {a}..{b}");
        }

        // The same through the state layer, with the caret it leaves.
        let state = EditorState::create(schema.clone(), doc.clone(), Vec::new());
        let mut tr = state.tr();
        tr.set_selection(Selection::text(Pos(from), Pos(to)));
        if tr.replace_selection(slice).is_ok() {
            let next = state.apply(tr);
            assert_valid(&next.doc, &what);
            let sel = &next.selection;
            assert!(
                sel.to().0 <= next.doc.content_size(),
                "{what}: selection {sel:?}"
            );
        }
    }
    // The instrument fired: most replacements fit (the refused ones are
    // ranges across structure, cuts that are no content, and slices whose
    // open depths are not their own), and both step kinds were made.
    assert!(
        applied * 10 >= seeds as usize * 6,
        "{applied} applied, {refused} refused; by kind {by_kind:?}"
    );
    assert!(around > 0, "no paste moved the text after the range");
    assert!(
        cell_selections * 20 >= seeds as usize,
        "{cell_selections} cell selections"
    );
    eprintln!(
        "{applied} applied ({around} moved inline content), {refused} refused; by kind {by_kind:?}"
    );
}

/// The empty slice is a deletion.
#[test]
fn an_empty_slice_deletes_the_range() {
    let schema = Schema::starter_kit();
    let doc = schema
        .branch(
            "doc",
            slice_from_html(&schema, "<p>abc</p>").unwrap().content,
        )
        .unwrap();
    let mut tf = Transform::new(&schema, doc);
    assert_eq!(tf.replace_range(2, 3, Slice::empty()).unwrap(), 2);
    assert_eq!(
        tf.doc.content(),
        &slice_from_html(&schema, "<p>ac</p>").unwrap().content
    );
}
