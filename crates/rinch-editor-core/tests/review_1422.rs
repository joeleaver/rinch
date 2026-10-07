//! Review of #1422: a differential oracle for `Transform::delete` over every
//! pair of positions of random nested documents.
//!
//! For each range: main's behaviour is the plain `ReplaceStep` (computed here),
//! the PR's is `Transform::delete`. Where the plain step applies the two must
//! be the same document. Where it does not and the PR's delete applies:
//! the document is valid, the step undoes, every leaf (text char, image, break,
//! rule) outside the range survives in order and nothing else does, and the
//! step's map sends each surviving leaf to where it now is.
//!
//! And through the state (`deleteSelection`): the selection is a caret where
//! the range began, no fitted delete leaves the document as it was, and a text
//! selection inside one isolating scope is never refused.
//!
//! `RINCH_1422_SEEDS` (default 40; the review ran 120: 204,503 ranges),
//! `RINCH_1422_SHOW=1` prints samples.

use rinch_editor_core::serialize::slice_from_html;
use rinch_editor_core::transform::{ReplaceStep, Step, Transform};
use rinch_editor_core::{EditorState, Node, Pos, Schema, Selection, Slice};
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

fn inline(rng: &mut Rng, out: &mut String, n: &mut usize) {
    for _ in 0..rng.below(3) {
        *n += 1;
        let text = format!("{}{}", ["a", "bc", "d", "xy"][rng.below(4)], *n % 10);
        match rng.below(8) {
            0 => out.push_str(&format!("<strong>{text}</strong>")),
            1 => out.push_str(&format!("<a href=\"https://e.x/\">{text}</a>")),
            2 => out.push_str("<br>"),
            3 => out.push_str("<img src=\"https://e.x/i.png\" alt=\"i\">"),
            _ => out.push_str(&text),
        }
    }
}

fn blocks(rng: &mut Rng, depth: usize, out: &mut String, n: &mut usize, tasks: bool) {
    for _ in 0..1 + rng.below(2) {
        let pick = if depth >= 3 {
            rng.below(4)
        } else {
            rng.below(12)
        };
        match pick {
            0 | 1 => {
                out.push_str("<p>");
                inline(rng, out, n);
                out.push_str("</p>");
            }
            2 => {
                out.push_str("<h2>");
                inline(rng, out, n);
                out.push_str("</h2>");
            }
            3 => out.push_str(if rng.chance(50) {
                "<hr>"
            } else {
                "<pre>let x;\ny</pre>"
            }),
            4..=6 => {
                let tag = if rng.chance(50) { "ul" } else { "ol" };
                out.push_str(&format!("<{tag}>"));
                for _ in 0..1 + rng.below(2) {
                    out.push_str("<li>");
                    blocks(rng, depth + 1, out, n, tasks);
                    out.push_str("</li>");
                }
                out.push_str(&format!("</{tag}>"));
            }
            7 if tasks => {
                out.push_str("<ul data-type=\"taskList\">");
                for _ in 0..1 + rng.below(2) {
                    out.push_str("<li data-type=\"taskItem\" data-checked=\"true\">");
                    blocks(rng, depth + 1, out, n, tasks);
                    out.push_str("</li>");
                }
                out.push_str("</ul>");
            }
            7..=9 => {
                out.push_str("<blockquote>");
                blocks(rng, depth + 1, out, n, tasks);
                out.push_str("</blockquote>");
            }
            _ => {
                out.push_str("<table>");
                let cols = 1 + rng.below(2);
                for _ in 0..1 + rng.below(2) {
                    out.push_str("<tr>");
                    for _ in 0..cols {
                        out.push_str("<td>");
                        blocks(rng, depth + 2, out, n, tasks);
                        out.push_str("</td>");
                    }
                    out.push_str("</tr>");
                }
                out.push_str("</table>");
            }
        }
    }
}

fn random_doc(schema: &Schema, rng: &mut Rng, tasks: bool) -> Node {
    let mut html = String::new();
    let mut n = 0;
    blocks(rng, 0, &mut html, &mut n, tasks);
    let slice = slice_from_html(schema, &html).expect("generated html parses");
    schema.branch("doc", slice.content).expect("a valid doc")
}

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
                "{what}: mark"
            );
        }
        assert_valid(child, what);
    }
}

/// Every leaf of `node` with its position: a char of text, or a leaf node.
fn leaves(node: &Node, at: usize, out: &mut Vec<(usize, String)>) {
    let mut pos = at;
    for child in node.content().children() {
        if let Some(t) = child.text() {
            for (i, c) in t.chars().enumerate() {
                out.push((pos + i, c.to_string()));
            }
        } else if child.is_leaf() {
            out.push((pos, child.type_name().to_string()));
        } else {
            leaves(child, pos + 1, out);
        }
        pos += child.node_size();
    }
}

fn isolating_scope(doc: &Node, pos: usize) -> Option<usize> {
    let r = doc.resolve(Pos(pos)).unwrap();
    (1..=r.depth())
        .rev()
        .find(|&d| r.node(d).node_type().is_isolating())
        .map(|d| r.start(d))
}

#[derive(Default, Debug)]
struct Tally {
    ranges: usize,
    plain: usize,
    fitted: usize,
    fitted_around: usize,
    refused: usize,
    refused_same_scope: usize,
    refused_text_ends: usize,
    refused_text_ends_same_scope: usize,
    uncollapsed: usize,
    fitted_noop: usize,
    fitted_noop_text_ends: usize,
    uncollapsed_stuck: usize,
}

fn check_all_ranges(schema: &Rc<Schema>, doc: &Node, tally: &mut Tally, show: bool) {
    let size = doc.content_size();
    let mut before = Vec::new();
    leaves(doc, 0, &mut before);
    for from in 0..=size {
        let Ok(rf) = doc.resolve(Pos(from)) else {
            continue;
        };
        for to in from + 1..=size {
            let Ok(rt) = doc.resolve(Pos(to)) else {
                continue;
            };
            tally.ranges += 1;
            let what = format!("{from}..{to} of {doc:?}");
            let plain = ReplaceStep::new(from, to, Slice::empty()).apply(doc);
            let mut tf = Transform::new(schema, doc.clone());
            let result = tf.delete(from, to).map(|_| ());
            let text_ends = rf.parent().is_textblock() && rt.parent().is_textblock();
            match (&plain, &result) {
                (Ok(p), Ok(())) => {
                    tally.plain += 1;
                    assert_eq!(&tf.doc, p, "{what}: differs from the plain step");
                    continue;
                }
                (Ok(_), Err(e)) => panic!("{what}: plain applies, delete refused: {e}"),
                (Err(_), Err(_)) => {
                    tally.refused += 1;
                    let same = isolating_scope(doc, from) == isolating_scope(doc, to);
                    if same {
                        tally.refused_same_scope += 1;
                    }
                    if text_ends {
                        tally.refused_text_ends += 1;
                        // A range with no leaf in it holds only the tokens
                        // between two textblocks that cannot join: nothing
                        // to delete, so a refusal.
                        let holds_a_leaf = before.iter().any(|(p, _)| *p >= from && *p < to);
                        if same && holds_a_leaf {
                            tally.refused_text_ends_same_scope += 1;
                            if show {
                                eprintln!("REFUSED(text ends, one scope) {what}");
                            }
                        }
                    }
                    assert_eq!(&tf.doc, doc, "{what}: a refusal changed the doc");
                    assert!(tf.steps().is_empty());
                    continue;
                }
                (Err(_), Ok(())) => {}
            }
            tally.fitted += 1;
            // A delete that deletes nothing is exactly one whose document is the same size.
            assert_eq!(
                tf.doc.content_size() == doc.content_size(),
                &tf.doc == doc,
                "{what}: size and equality disagree"
            );
            if &tf.doc == doc {
                tally.fitted_noop += 1;
                if text_ends {
                    tally.fitted_noop_text_ends += 1;
                    if show && tally.fitted_noop_text_ends <= 3 {
                        eprintln!("NOOP {what}: {:?}", tf.steps()[0]);
                    }
                }
            }
            assert_eq!(
                isolating_scope(doc, from),
                isolating_scope(doc, to),
                "{what}: fitted across isolating nodes"
            );
            assert_valid(&tf.doc, &what);
            assert_eq!(tf.steps().len(), 1, "{what}");
            let step = &tf.steps()[0];
            if format!("{step:?}").starts_with("ReplaceAroundStep") {
                tally.fitted_around += 1;
            }
            let undone = step
                .invert(doc)
                .apply(&tf.doc)
                .unwrap_or_else(|e| panic!("{what}: inverse: {e}"));
            assert_eq!(&undone, doc, "{what}: undo");
            // Leaves: exactly those outside the range, in order.
            let mut after = Vec::new();
            leaves(&tf.doc, 0, &mut after);
            let expect: Vec<&(usize, String)> = before
                .iter()
                .filter(|(p, _)| *p + 1 <= from || *p >= to)
                .collect();
            let got: Vec<&String> = after.iter().map(|(_, s)| s).collect();
            let want: Vec<&String> = expect.iter().map(|(_, s)| s).collect();
            assert_eq!(got, want, "{what}: leaves -> {:?}", tf.doc);
            // The map sends each surviving leaf to where it is.
            let map = step.get_map();
            for ((old, s), (new, _)) in expect.iter().zip(after.iter()) {
                let assoc = if *old >= to { 1 } else { -1 };
                assert_eq!(
                    map.map(*old, assoc),
                    *new,
                    "{what}: leaf {s:?} at {old} is at {new} in {:?} (step {step:?})",
                    tf.doc
                );
            }
            assert_eq!(map.map(from, -1), from, "{what}");
            if show && text_ends && tally.fitted % 97 == 0 {
                eprintln!("FITTED {what}\n   -> {:?}\n   {step:?}", tf.doc);
            }
            // Through the state: the caret is at `from` and in a textblock
            // when the range began in one.
            if text_ends {
                let mut state = EditorState::create(
                    schema.clone(),
                    doc.clone(),
                    rinch_editor_core::default_plugins(),
                );
                state.selection = Selection::text(Pos(from), Pos(to));
                let next = state
                    .run("deleteSelection")
                    .unwrap_or_else(|| panic!("{what}: deleteSelection did nothing"));
                assert_eq!(next.doc, tf.doc, "{what}");
                if !(next.selection.is_empty() && next.selection.from() == Pos(from)) {
                    tally.uncollapsed += 1;
                    let again = next.run("deleteSelection");
                    match &again {
                        None => tally.uncollapsed_stuck += 1,
                        Some(a) if a.doc == next.doc => tally.uncollapsed_stuck += 1,
                        Some(_) => {}
                    }
                    if show && tally.uncollapsed <= 6 {
                        eprintln!(
                            "UNCOLLAPSED {what}\n  -> {:?}\n  sel {:?}; again -> {:?}",
                            next.doc,
                            next.selection,
                            again.map(|a| (format!("{:?}", a.doc), a.selection))
                        );
                    }
                } else {
                    let r = next.doc.resolve(Pos(from)).unwrap();
                    assert!(r.parent().is_textblock(), "{what}: caret not in text");
                }
                let back = next
                    .run("undo")
                    .unwrap_or_else(|| panic!("{what}: undo did nothing"));
                assert_eq!(&back.doc, doc, "{what}: history undo");
            }
        }
    }
}

#[test]
fn every_range_of_random_documents() {
    let seeds: u64 = std::env::var("RINCH_1422_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(40);
    let show = std::env::var("RINCH_1422_SHOW").is_ok();
    let schema = Rc::new(Schema::starter_kit());
    let mut tally = Tally::default();
    for seed in 1..=seeds {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let doc = random_doc(&schema, &mut rng, true);
        if doc.content_size() > 140 {
            continue;
        }
        check_all_ranges(&schema, &doc, &mut tally, show);
    }
    eprintln!("{tally:?}");
    assert!(
        tally.fitted > 1000 && tally.fitted_around > 100,
        "{tally:?}"
    );
    assert!(tally.refused_text_ends > 100, "{tally:?}");
    assert_eq!(tally.refused_text_ends_same_scope, 0, "{tally:?}");
    assert_eq!((tally.uncollapsed, tally.fitted_noop), (0, 0), "{tally:?}");
}

fn doc_of(schema: &Schema, html: &str) -> Node {
    schema
        .branch("doc", slice_from_html(schema, html).unwrap().content)
        .unwrap()
}

fn pos_of(doc: &Node, needle: &str) -> usize {
    let mut l = Vec::new();
    leaves(doc, 0, &mut l);
    let first: Vec<String> = needle.chars().map(|c| c.to_string()).collect();
    l.windows(first.len())
        .find(|w| w.iter().map(|(_, s)| s).eq(first.iter()))
        .unwrap_or_else(|| panic!("no {needle}"))[0]
        .0
}

/// What a command over the text selection `a|..|b` (from before `a` to after
/// `b`) leaves, or `None` when it does nothing.
fn run(
    html: &str,
    a: &str,
    b: &str,
    f: impl Fn(&EditorState) -> Option<EditorState>,
) -> Option<String> {
    let schema = Rc::new(Schema::starter_kit());
    let doc = doc_of(&schema, html);
    let (from, to) = (pos_of(&doc, a), pos_of(&doc, b) + b.chars().count());
    let mut state = EditorState::create(schema, doc, rinch_editor_core::default_plugins());
    state.selection = Selection::text(Pos(from), Pos(to));
    f(&state).map(|s| format!("{:?}", s.doc))
}

const NESTED: &str = "<ul><li><p>one</p><ul><li><p>two</p></li></ul></li></ul>";
const TABLE: &str =
    "<p>before</p><table><tr><td><p>cell</p></td><td><p>next</p></td></tr></table><p>after</p>";

fn delete(html: &str, a: &str, b: &str) -> Option<String> {
    run(html, a, b, |s| s.run("deleteSelection"))
}

/// The edits that delete the selection first take the fitted delete too.
#[test]
fn the_edits_that_delete_the_selection_first_are_fitted() {
    let one = "doc([bullet_list([list_item([paragraph([text(\"oo\")])])])])";
    assert_eq!(delete(NESTED, "ne", "tw").as_deref(), Some(one));
    assert_eq!(
        run(NESTED, "ne", "tw", |s| s.run("deleteWordBackward")).as_deref(),
        Some(one)
    );
    assert_eq!(
        run(NESTED, "ne", "tw", |s| s.run("splitBlock")).as_deref(),
        Some(
            "doc([bullet_list([list_item([paragraph([text(\"o\")]), \
             paragraph([text(\"o\")])])])])"
        )
    );
    assert_eq!(
        run(NESTED, "ne", "tw", |s| s.run("insertHorizontalRule")).as_deref(),
        Some(
            "doc([bullet_list([list_item([paragraph([text(\"o\")]), horizontal_rule, \
             paragraph([text(\"o\")])])])])"
        )
    );
}

/// A range that leaves or enters a table cell is refused: the fit never
/// crosses an isolating node, and the document is left alone.
#[test]
fn a_delete_into_or_out_of_a_table_cell_is_refused() {
    assert_eq!(delete(TABLE, "fore", "ce"), None);
    assert_eq!(delete(TABLE, "ll", "af"), None);
    let schema = Rc::new(Schema::starter_kit());
    let doc = doc_of(&schema, TABLE);
    let (from, to) = (pos_of(&doc, "fore"), pos_of(&doc, "ce") + 2);
    let mut tf = Transform::new(&schema, doc.clone());
    assert!(tf.delete(from, to).is_err());
    assert_eq!(tf.doc, doc);
    assert!(tf.steps().is_empty());
    // Inside one cell the fit is taken.
    let c = "<table><tr><td><p>cell</p><ul><li><p>item</p></li></ul></td></tr></table>";
    assert_eq!(
        delete(c, "ll", "it").as_deref(),
        Some("doc([table([table_row([table_cell([paragraph([text(\"ceem\")])])])])])")
    );
}

/// Into and out of a quote, and out of a list into code.
#[test]
fn a_delete_across_a_quote_or_into_code_joins_the_two_textblocks() {
    let q = "<p>before</p><blockquote><p>quoted</p><p>second</p></blockquote><p>after</p>";
    assert_eq!(
        delete(q, "fore", "quo").as_deref(),
        Some(
            "doc([paragraph([text(\"beted\")]), blockquote([paragraph([text(\"second\")])]), \
             paragraph([text(\"after\")])])"
        )
    );
    assert_eq!(
        delete(q, "cond", "af").as_deref(),
        Some(
            "doc([paragraph([text(\"before\")]), blockquote([paragraph([text(\"quoted\")]), \
             paragraph([text(\"seter\")])])])"
        )
    );
    let h = "<ul><li><p>item</p></li></ul><pre>code</pre>";
    assert_eq!(
        delete(h, "em", "co").as_deref(),
        Some("doc([bullet_list([list_item([paragraph([text(\"itde\")])])])])")
    );
}

/// When the textblock the range began in cannot take what is left of the one
/// it ends in (code, then marked text and an image), the tail stays a block of
/// its own, so the range's end maps past its start. The selection is still a
/// caret where the range began, and a second Delete has nothing to delete.
#[test]
fn a_delete_whose_tail_cannot_join_still_leaves_a_caret() {
    let k = "<pre>code</pre><ul><li><p>it<strong>em</strong><img src=\"x\"></p></li></ul>";
    let schema = Rc::new(Schema::starter_kit());
    let doc = doc_of(&schema, k);
    let (from, to) = (pos_of(&doc, "de"), pos_of(&doc, "it") + 1);
    let mut state = EditorState::create(schema, doc, rinch_editor_core::default_plugins());
    state.selection = Selection::text(Pos(from), Pos(to));
    let next = state.run("deleteSelection").expect("the delete is fitted");
    assert_eq!(
        format!("{:?}", next.doc),
        "doc([code_block([text(\"co\")]), bullet_list([list_item([paragraph([text(\"t\"), \
         text(\"em\", marks=[\"bold\"]), image])])])])"
    );
    assert!(next.selection.is_empty(), "{:?}", next.selection);
    assert_eq!(next.selection.from(), Pos(from));
    assert!(next.run("deleteSelection").is_none());
}

/// A range holding only the tokens between two textblocks that cannot join
/// deletes nothing: it is refused, not recorded as a step that changes nothing.
#[test]
fn a_range_of_only_the_tokens_between_two_unjoinable_textblocks_is_refused() {
    let k = "<pre>code</pre><ul><li><p><strong>em</strong></p></li></ul>";
    let schema = Rc::new(Schema::starter_kit());
    let doc = doc_of(&schema, k);
    let (from, to) = (pos_of(&doc, "e") + 1, pos_of(&doc, "em"));
    assert!(to > from + 1);
    let mut tf = Transform::new(&schema, doc.clone());
    assert!(tf.delete(from, to).is_err());
    assert_eq!(tf.doc, doc);
    assert!(tf.steps().is_empty());
    let mut state = EditorState::create(schema, doc, rinch_editor_core::default_plugins());
    state.selection = Selection::text(Pos(from), Pos(to));
    assert!(state.run("deleteSelection").is_none());
}

/// Still not fitted (#1460): an insert over such a selection goes through the
/// plain replace and does nothing. A fix flips these.
#[test]
fn an_insert_over_a_selection_across_depths_is_still_refused_issue_1460() {
    assert_eq!(run(NESTED, "ne", "tw", |s| s.run("insertHardBreak")), None);
    let typed = run(NESTED, "ne", "tw", |s| {
        let mut tr = s.tr();
        tr.insert_text("X").ok()?;
        Some(s.apply(tr))
    });
    assert_eq!(typed, None);
}
