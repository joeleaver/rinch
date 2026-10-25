//! Undo among a real collaboration peer's changes (#1249; scenarios from the review
//! of PR #1889). Two `CollabSession`s exchange real yrs updates; I am peer `a`.
//!
//! Each scenario asserts what the undo should do. The ones the history still gets
//! wrong — where the documents before and after a peer's change do not say which
//! of two identical runs is whose (#1895) — assert what it does today, through
//! [`known_wrong`], so a fix flips them.

use std::rc::Rc;

use rinch_editor_collab::testing::{session_from_bytes_with_client_id, session_with_client_id};
use rinch_editor_collab::{CollabPlugin, CollabSession};
use rinch_editor_core::serialize::{node_to_html, slice_from_html};
use rinch_editor_core::{
    EditorState, Fragment, Node, Plugin, Pos, Schema, Selection, Slice, default_plugins,
};

fn plugins() -> Vec<Rc<dyn Plugin>> {
    let mut p = default_plugins();
    p.push(Rc::new(CollabPlugin));
    p
}

struct Peer {
    schema: Rc<Schema>,
    state: EditorState,
    session: CollabSession,
}

impl Peer {
    fn commit(&mut self, next: EditorState) {
        self.session
            .record_local_edit(&self.schema, &self.state.doc, &next.doc, &[])
            .expect("projects");
        self.state = next;
    }
    fn receive(&mut self, delta: &[u8]) {
        if let Some(next) = self
            .session
            .integrate_incremental(&self.state, delta)
            .unwrap()
        {
            self.state = next;
        }
    }
    fn html(&self) -> String {
        node_to_html(&self.state.doc)
    }
    fn type_at(&mut self, at: usize, s: &str, time: u64) {
        let mut tr = self.state.tr();
        tr.set_time(time);
        tr.set_selection(Selection::cursor(Pos(at)));
        tr.insert_text(s).unwrap();
        let n = self.state.apply(tr);
        self.commit(n);
    }
    fn delete(&mut self, a: usize, b: usize, time: u64) {
        let mut tr = self.state.tr();
        tr.set_time(time);
        tr.delete(a, b).unwrap();
        let n = self.state.apply(tr);
        self.commit(n);
    }
    fn paste_text(&mut self, at: usize, s: &str, time: u64) {
        let mut tr = self.state.tr();
        tr.set_time(time);
        let f = Fragment::from_node(self.schema.text(s).unwrap());
        tr.replace(at, at, Slice::new(f, 0, 0)).unwrap();
        let n = self.state.apply(tr);
        self.commit(n);
    }
    fn cmd(&mut self, sel: Selection, name: &str) {
        let mut s = self.state.clone();
        s.selection = sel;
        let n = s.run(name).unwrap_or_else(|| panic!("{name} applies"));
        self.commit(n);
    }
    fn undo(&mut self) -> bool {
        match self.state.run("undo") {
            Some(n) => {
                self.commit(n);
                true
            }
            None => false,
        }
    }
    fn redo(&mut self) -> bool {
        match self.state.run("redo") {
            Some(n) => {
                self.commit(n);
                true
            }
            None => false,
        }
    }
}

fn world(html: &str, seed: u64) -> (Peer, Peer) {
    let s = Rc::new(Schema::starter_kit());
    let slice = slice_from_html(&s, html).unwrap();
    let doc = s.branch("doc", slice.content.clone()).unwrap();
    let state = EditorState::create(s.clone(), doc, plugins());
    let host = session_with_client_id(&state, (seed << 8) | 1).unwrap();
    let guest = session_from_bytes_with_client_id(&host.snapshot(), (seed << 8) | 2).unwrap();
    let gdoc = guest.projected_doc(&s).unwrap();
    (
        Peer {
            schema: s.clone(),
            state,
            session: host,
        },
        Peer {
            schema: s.clone(),
            state: EditorState::create(s.clone(), gdoc, plugins()),
            session: guest,
        },
    )
}

fn sync(a: &mut Peer, b: &mut Peer) {
    let da = a.session.save_incremental().unwrap();
    let db = b.session.save_incremental().unwrap();
    a.receive(&db);
    b.receive(&da);
    assert_eq!(a.state.doc, b.state.doc, "converged");
}

fn report(name: &str, got: &str, want: &str) {
    assert_eq!(got, want, "{name}");
}

/// A scenario the history gets wrong (#1895): `got` is what it does today; `want`
/// is right. Fails when the behaviour changes, so a fix updates the pin.
fn known_wrong(name: &str, got: &str, today: &str, want: &str) {
    assert_ne!(today, want);
    assert_eq!(got, today, "{name} changed (#1895): is it now {want}?");
}

/// A: I delete `x` from `axb`. B pastes `ab` at the start of the paragraph (one
/// update). I undo: my `x` belongs between *my* a and b, i.e. after B's `ab`.
#[test]
fn s01_duplicate_text_peer_paste_before_my_delete() {
    let (mut a, mut b) = world("<p>axb</p>", 1);
    a.delete(2, 3, 1000);
    sync(&mut a, &mut b);
    b.paste_text(1, "ab", 2000);
    sync(&mut a, &mut b);
    assert_eq!(a.html(), "<p>abab</p>");
    assert!(a.undo());
    sync(&mut a, &mut b);
    known_wrong("s01 dup paste", &a.html(), "<p>axbab</p>", "<p>abaxb</p>");
}

/// The same with distinct letters (control: no ambiguity).
#[test]
fn s02_control_distinct_text() {
    let (mut a, mut b) = world("<p>axb</p>", 2);
    a.delete(2, 3, 1000);
    sync(&mut a, &mut b);
    b.paste_text(1, "PQ", 2000);
    sync(&mut a, &mut b);
    assert!(a.undo());
    sync(&mut a, &mut b);
    report("s02 control", &a.html(), "<p>PQaxb</p>");
}

/// Peer changes two paragraphs in one update (delayed delivery); my edit is in an
/// untouched paragraph between them.
#[test]
fn s03_two_paragraph_update_around_my_edit() {
    let (mut a, mut b) = world("<p>one</p><p>two</p><p>three</p>", 3);
    a.type_at(7, "X", 1000); // in "two"
    sync(&mut a, &mut b);
    assert_eq!(a.html(), "<p>one</p><p>tXwo</p><p>three</p>");
    b.type_at(2, "P", 2000); // "oPne"
    let end = b.state.doc.content_size() - 1;
    b.type_at(end, "Q", 3000); // "threeQ"
    sync(&mut a, &mut b); // one delta carries both
    assert!(a.undo(), "undo applies");
    sync(&mut a, &mut b);
    report(
        "s03 two-paragraph update",
        &a.html(),
        "<p>oPne</p><p>two</p><p>threeQ</p>",
    );
}

/// Peer deleted the range I'd undo: undo skips it cleanly.
#[test]
fn s04_peer_deleted_my_range() {
    let (mut a, mut b) = world("<p>hello world</p>", 4);
    a.type_at(7, "XYZ", 1000); // hello XYZworld
    sync(&mut a, &mut b);
    b.delete(5, 13, 2000); // delete " XYZwor"
    sync(&mut a, &mut b);
    let before = a.html();
    let applied = a.undo();
    sync(&mut a, &mut b);
    assert!(applied, "the undo is used up");
    assert_eq!(a.html(), before, "nothing of mine is left to undo");
}

/// Redo after remote changes.
#[test]
fn s05_redo_after_remote() {
    let (mut a, mut b) = world("<p>abc</p>", 5);
    a.type_at(3, "X", 1000); // abXc
    sync(&mut a, &mut b);
    assert!(a.undo()); // abc
    sync(&mut a, &mut b);
    b.type_at(1, "PP", 2000); // PPabc
    sync(&mut a, &mut b);
    assert!(a.redo());
    sync(&mut a, &mut b);
    report("s05 redo after remote", &a.html(), "<p>PPabXc</p>");
}

/// Undo of a bold on text a peer split.
#[test]
fn s06_undo_bold_peer_split() {
    let (mut a, mut b) = world("<p>abcdef</p>", 6);
    a.cmd(Selection::text(Pos(2), Pos(6)), "toggleBold"); // b..e bold
    sync(&mut a, &mut b);
    b.cmd(Selection::cursor(Pos(4)), "splitBlock"); // ab<b>c | d</b>ef
    sync(&mut a, &mut b);
    assert!(a.undo());
    sync(&mut a, &mut b);
    report("s06 bold/split", &a.html(), "<p>abc</p><p>def</p>");
}

/// Undo of my typing in a cell after a peer adds a row and a column.
#[test]
fn s07_table_restructured() {
    let html = "<table><tr><td><p>a1</p></td><td><p>b1</p></td></tr><tr><td><p>a2</p></td><td><p>b2</p></td></tr></table><p>end</p>";
    let (mut a, mut b) = world(html, 7);
    // find "b2" position
    let doc = a.state.doc.clone();
    let pos_b2 = find(&doc, "b2").unwrap();
    a.type_at(pos_b2 + 1, "X", 1000); // bX2
    sync(&mut a, &mut b);
    let p_a1 = find(&b.state.doc, "a1").unwrap();
    b.cmd(Selection::cursor(Pos(p_a1)), "addRowBefore");
    let p_a1 = find(&b.state.doc, "a1").unwrap();
    b.cmd(Selection::cursor(Pos(p_a1)), "addColumnBefore");
    sync(&mut a, &mut b);
    let before = a.html();
    assert!(a.undo());
    sync(&mut a, &mut b);
    report("s07 table", &a.html(), &before.replace("bX2", "b2"));
}

/// Peer merges the cell I typed in with its neighbour.
#[test]
fn s08_table_cells_merged() {
    let html = "<table><tr><td><p>a1</p></td><td><p>b1</p></td></tr><tr><td><p>a2</p></td><td><p>b2</p></td></tr></table><p>end</p>";
    let (mut a, mut b) = world(html, 8);
    let pos = find(&a.state.doc, "b1").unwrap();
    a.type_at(pos + 1, "X", 1000);
    sync(&mut a, &mut b);
    let p1 = find(&b.state.doc, "a1").unwrap();
    let p2 = find(&b.state.doc, "bX1").unwrap();
    // cell selection a1..b1
    let c1 = cell_before(&b.state.doc, p1);
    let c2 = cell_before(&b.state.doc, p2);
    b.cmd(Selection::cell(Pos(c1), Pos(c2)), "mergeCells");
    sync(&mut a, &mut b);
    let before = a.html();
    let applied = a.undo();
    sync(&mut a, &mut b);
    assert!(applied);
    assert!(
        !a.html().contains("X"),
        "my X is undone: {before} → {}",
        a.html()
    );
}

/// Both peers undo their own edits.
#[test]
fn s09_both_undo() {
    let (mut a, mut b) = world("<p>abc</p><p>def</p>", 9);
    a.type_at(2, "A", 1000); // aAbc
    b.type_at(7, "B", 1000); // dBef
    sync(&mut a, &mut b);
    a.type_at(1, "AA", 3000);
    sync(&mut a, &mut b);
    b.type_at(9, "BB", 3000);
    sync(&mut a, &mut b);
    assert!(a.undo());
    assert!(b.undo());
    sync(&mut a, &mut b);
    report("s09 both undo #1", &a.html(), "<p>aAbc</p><p>dBef</p>");
    assert!(a.undo());
    assert!(b.undo());
    sync(&mut a, &mut b);
    report("s09 both undo #2", &a.html(), "<p>abc</p><p>def</p>");
    assert!(a.redo());
    assert!(b.redo());
    sync(&mut a, &mut b);
    report("s09 both redo", &a.html(), "<p>aAbc</p><p>dBef</p>");
}

/// Peer moves a paragraph (delete + insert elsewhere in one update) past mine.
#[test]
fn s10_peer_moves_block_across_my_edit() {
    let (mut a, mut b) = world("<p>one</p><p>two</p><p>three</p><p>four</p>", 10);
    a.type_at(6, "X", 1000); // tXwo
    sync(&mut a, &mut b);
    // B: cut "one" paragraph and paste it at the end, one transaction
    let mut tr = b.state.tr();
    let one = b.state.doc.child(0).clone();
    let size = b.state.doc.content_size();
    tr.replace(
        size,
        size,
        Slice::new(Fragment::from_node(one.clone()), 0, 0),
    )
    .unwrap();
    tr.delete(0, one.node_size()).unwrap();
    let n = b.state.apply(tr);
    b.commit(n);
    sync(&mut a, &mut b);
    assert!(a.undo());
    sync(&mut a, &mut b);
    report(
        "s10 block move",
        &a.html(),
        "<p>two</p><p>three</p><p>four</p><p>one</p>",
    );
}

/// Peer's block wrap (my paragraph wrapped in a blockquote by the peer).
#[test]
fn s11_peer_wraps_my_paragraph() {
    let (mut a, mut b) = world("<p>one</p><p>two</p>", 11);
    a.type_at(7, "X", 1000); // twXo
    sync(&mut a, &mut b);
    b.cmd(Selection::cursor(Pos(7)), "wrapInBlockquote");
    sync(&mut a, &mut b);
    assert!(a.undo());
    sync(&mut a, &mut b);
    report(
        "s11 wrap",
        &a.html(),
        "<p>one</p><blockquote><p>two</p></blockquote>",
    );
}

/// Peer pastes duplicate paragraph content into a blockquote that holds my paragraph.
#[test]
fn s12_duplicate_block_in_container() {
    let (mut a, mut b) = world("<blockquote><p>ab</p></blockquote>", 12);
    a.delete(2, 3, 1000); // <p>b</p>? no: delete "b" → "a"
    sync(&mut a, &mut b);
    // B inserts an identical paragraph before mine inside the blockquote
    let p = b
        .schema
        .branch(
            "paragraph",
            Fragment::from_node(b.schema.text("a").unwrap()),
        )
        .unwrap();
    let mut tr = b.state.tr();
    tr.replace(1, 1, Slice::new(Fragment::from_node(p), 0, 0))
        .unwrap();
    let n = b.state.apply(tr);
    b.commit(n);
    sync(&mut a, &mut b);
    assert!(a.undo());
    sync(&mut a, &mut b);
    report(
        "s12 dup block in container",
        &a.html(),
        "<blockquote><p>a</p><p>ab</p></blockquote>",
    );
}

fn find(doc: &Node, needle: &str) -> Option<usize> {
    (0..doc.content_size()).find(|&p| {
        let end = p + needle.chars().count();
        end <= doc.content_size()
            && doc.resolve(Pos(p)).is_ok_and(|r| r.parent().is_textblock())
            && doc.slice(p, end).ok().is_some_and(|s| {
                s.content.child_count() > 0
                    && s.content.iter().all(|n| n.is_text())
                    && s.content
                        .iter()
                        .map(|n| n.text().unwrap())
                        .collect::<String>()
                        == needle
            })
    })
}

/// Position before the cell holding text position `p`.
fn cell_before(doc: &Node, p: usize) -> usize {
    let r = doc.resolve(Pos(p)).unwrap();
    for d in (0..=r.depth()).rev() {
        if r.node(d).type_name() == "table_cell" {
            return r.before(d).unwrap();
        }
    }
    panic!("not in a cell")
}

/// Peer bolds the word I just typed (no position moves).
#[test]
fn s13_peer_bolds_my_typing() {
    let (mut a, mut b) = world("<p>hello world</p>", 13);
    a.type_at(6, " big", 1000); // hello big world
    sync(&mut a, &mut b);
    b.cmd(Selection::text(Pos(1), Pos(16)), "toggleBold");
    sync(&mut a, &mut b);
    let applied = a.undo();
    sync(&mut a, &mut b);
    assert!(applied);
    report(
        "s13 peer bold",
        &a.html(),
        "<p><strong>hello world</strong></p>",
    );
}

/// Peer turns my paragraph into a heading.
#[test]
fn s14_peer_heading_on_my_paragraph() {
    let (mut a, mut b) = world("<p>hello</p><p>other</p>", 14);
    a.type_at(6, "!", 1000); // hello!
    sync(&mut a, &mut b);
    // use the command catalogue: toggle heading via setParagraph is not a heading;
    // build it directly.
    let mut tr = b.state.tr();
    tr.set_block_type(
        1,
        1,
        b.schema.node_type("heading").unwrap().clone(),
        rinch_editor_core::Attrs::new().with("level", rinch_editor_core::AttrValue::Int(2)),
    )
    .unwrap();
    let n = b.state.apply(tr);
    b.commit(n);
    sync(&mut a, &mut b);
    let applied = a.undo();
    sync(&mut a, &mut b);
    assert!(applied);
    report("s14 peer heading", &a.html(), "<h2>hello</h2><p>other</p>");
}

/// Peer centres my paragraph.
#[test]
fn s15_peer_aligns_my_paragraph() {
    let (mut a, mut b) = world("<p>hello</p><p>other</p>", 15);
    a.type_at(6, "!", 1000);
    sync(&mut a, &mut b);
    b.cmd(Selection::cursor(Pos(2)), "setTextAlignCenter");
    sync(&mut a, &mut b);
    let before = a.html();
    let applied = a.undo();
    sync(&mut a, &mut b);
    assert!(applied);
    report(
        "s15 peer align",
        &a.html(),
        &before.replace("hello!", "hello"),
    );
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn text_positions(doc: &Node) -> Vec<usize> {
    (0..=doc.content_size())
        .filter(|&p| doc.resolve(Pos(p)).is_ok_and(|r| r.parent().is_textblock()))
        .collect()
}

fn chars(n: &Node) -> String {
    if let Some(t) = n.text() {
        return t.to_string();
    }
    (0..n.child_count()).map(|i| chars(n.child(i))).collect()
}

fn upper(n: &Node) -> String {
    chars(n)
        .chars()
        .filter(|c| c.is_ascii_uppercase())
        .collect()
}

/// Random: B types UPPERCASE / deletes / splits / joins / bolds / centres; A types
/// lowercase, splits, undoes and redoes. Invariant: A's undo/redo never changes B's
/// letters; after A undoes everything, no lowercase letter is left.
#[test]
fn s99_random_real_collab() {
    let seeds: u64 = 200;
    let peer_ops: usize = 4;
    let mut failing = Vec::new();
    let mut undos = 0;
    for seed in 1..=seeds {
        let mut rng = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
        let (mut a, mut b) = world(
            "<p>AB</p><p>CA</p><ul><li><p>BC</p></li></ul><p>AA</p>",
            1000 + seed,
        );
        let mut t = 1000u64;
        let mut fail: Option<String> = None;
        let mut log: Vec<String> = Vec::new();
        for _ in 0..30 {
            t += if rng.below(10) < 6 { 10 } else { 1000 };
            match rng.below(10) {
                0..=2 => {
                    // B edit(s), then sync
                    let per: usize = 2;
                    for _ in 0..1 + rng.below(per) {
                        let tp = text_positions(&b.state.doc);
                        let at = tp[rng.below(tp.len())];
                        match rng.below(peer_ops) {
                            0 | 1 => {
                                let s: String = (0..1 + rng.below(2))
                                    .map(|_| (b'A' + rng.below(3) as u8) as char)
                                    .collect();
                                log.push(format!("B type {at} {s}"));
                                b.type_at(at, &s, t);
                            }
                            2 => {
                                let r = b.state.doc.resolve(Pos(at)).unwrap();
                                let end = at - r.parent_offset() + r.parent().content_size();
                                if end > at {
                                    let e = at + 1 + rng.below((end - at).min(3));
                                    log.push(format!("B delete {at}..{e}"));
                                    b.delete(at, e, t);
                                }
                            }
                            3 => {
                                let mut s = b.state.clone();
                                s.selection = Selection::cursor(Pos(at));
                                let name = if rng.below(2) == 0 {
                                    "splitBlock"
                                } else {
                                    "deleteCharBackward"
                                };
                                if let Some(n) = s.run(name) {
                                    log.push(format!("B {name} {at}"));
                                    b.commit(n);
                                }
                            }
                            4 => {
                                let e = tp[rng.below(tp.len())];
                                let mut s = b.state.clone();
                                s.selection = Selection::text(Pos(at.min(e)), Pos(at.max(e)));
                                if let Some(n) = s.run("toggleBold") {
                                    log.push(format!("B bold {at}..{e}"));
                                    b.commit(n);
                                }
                            }
                            _ => {
                                let mut s = b.state.clone();
                                s.selection = Selection::cursor(Pos(at));
                                if let Some(n) = s.run("setTextAlignCenter") {
                                    log.push(format!("B center {at}"));
                                    b.commit(n);
                                }
                            }
                        }
                    }
                    sync(&mut a, &mut b);
                }
                3 | 4 => {
                    let cmd = if rng.below(10) < 6 { "undo" } else { "redo" };
                    let before = upper(&a.state.doc);
                    let html_before = a.html();
                    let ok = if cmd == "undo" { a.undo() } else { a.redo() };
                    if ok {
                        undos += 1;
                        log.push(format!("A {cmd}"));
                        if upper(&a.state.doc) != before && fail.is_none() {
                            fail = Some(format!(
                                "{cmd} changed B's letters\n  before {html_before}\n  after  {}",
                                a.html()
                            ));
                        }
                    }
                    sync(&mut a, &mut b);
                }
                _ => {
                    let tp = text_positions(&a.state.doc);
                    let at = tp[rng.below(tp.len())];
                    if rng.below(4) == 0 {
                        let mut s = a.state.clone();
                        s.selection = Selection::cursor(Pos(at));
                        if let Some(n) = s.run("splitBlock") {
                            log.push(format!("A split {at}"));
                            a.commit(n);
                        }
                    } else {
                        let alen: usize = 2;
                        let s: String = (0..1 + rng.below(alen))
                            .map(|_| (b'a' + rng.below(3) as u8) as char)
                            .collect();
                        log.push(format!("A type {at} {s}"));
                        a.type_at(at, &s, t);
                    }
                    if rng.below(2) == 0 {
                        sync(&mut a, &mut b);
                    }
                }
            }
        }
        sync(&mut a, &mut b);
        let before_all = a.html();
        while a.undo() {}
        sync(&mut a, &mut b);
        if fail.is_none() && chars(&a.state.doc).chars().any(|c| c.is_ascii_lowercase()) {
            fail = Some(format!(
                "lowercase left after undo-all\n  before {before_all}\n  after  {}",
                a.html()
            ));
        }
        if let Some(f) = fail {
            eprintln!("seed {seed}: {f}\n  log: {}\n", log.join("; "));
            failing.push(seed);
        }
    }
    assert!(undos > 500, "the trials undo and redo ({undos})");
    // Known wrong (#1895): seed 48 leaves my text after undo-all. 107 seeds failed
    // with the first cut of #1889, 198 on main.
    assert_eq!(failing, [48]);
}

/// Concurrent: I type `b` inside `BCA` while B (who has not seen it) deletes `BCA`.
#[test]
fn s16_concurrent_delete_around_my_char() {
    let (mut a, mut b) = world("<p>BCA</p><p>z</p>", 16);
    a.type_at(3, "b", 1000); // BCbA, not synced
    b.delete(1, 4, 1000); // B deletes BCA
    sync(&mut a, &mut b);
    assert_eq!(a.html(), "<p>b</p><p>z</p>");
    let applied = a.undo();
    sync(&mut a, &mut b);
    assert!(applied);
    report("s16 concurrent delete around", &a.html(), "<p></p><p>z</p>");
}

/// B types at both ends of my paragraph in one update; my char is in the middle.
#[test]
fn s17_peer_edits_both_ends_one_update() {
    let (mut a, mut b) = world("<p>hello world</p>", 17);
    a.type_at(6, "X", 1000); // helloX world
    sync(&mut a, &mut b);
    b.type_at(1, "P", 2000);
    let end = b.state.doc.content_size() - 1;
    b.type_at(end, "Q", 3000);
    sync(&mut a, &mut b);
    let applied = a.undo();
    sync(&mut a, &mut b);
    assert!(applied);
    report("s17 both ends", &a.html(), "<p>Phello worldQ</p>");
}

/// Both press Enter at the end of `x`; B types `A` in their new line; I undo my Enter.
#[test]
fn s18_both_enter_empty_paragraph_ambiguity() {
    let (mut a, mut b) = world("<p>x</p>", 18);
    a.cmd(Selection::cursor(Pos(2)), "splitBlock"); // <p>x</p><p></p>
    sync(&mut a, &mut b);
    b.cmd(Selection::cursor(Pos(2)), "splitBlock"); // B's empty line between
    sync(&mut a, &mut b);
    b.type_at(4, "A", 2000); // B types in its line (index 1)
    sync(&mut a, &mut b);
    assert_eq!(a.html(), "<p>x</p><p>A</p><p></p>");
    let applied = a.undo();
    sync(&mut a, &mut b);
    assert!(applied);
    known_wrong(
        "s18 enter ambiguity",
        &a.html(),
        "<p>xA</p><p></p>",
        "<p>x</p><p>A</p>",
    );
}

/// I type `a` in an empty paragraph of a blockquote; B inserts a paragraph `a` before
/// mine in the same blockquote (the whole blockquote is replaced). I undo.
#[test]
fn s19_wrong_block_in_container() {
    let (mut a, mut b) = world("<blockquote><p></p></blockquote><p>z</p>", 19);
    a.type_at(2, "a", 1000);
    sync(&mut a, &mut b);
    let p = b
        .schema
        .branch(
            "paragraph",
            Fragment::from_node(b.schema.text("a").unwrap()),
        )
        .unwrap();
    let mut tr = b.state.tr();
    tr.replace(1, 1, Slice::new(Fragment::from_node(p), 0, 0))
        .unwrap();
    let n = b.state.apply(tr);
    b.commit(n);
    sync(&mut a, &mut b);
    assert_eq!(
        a.html(),
        "<blockquote><p>a</p><p>a</p></blockquote><p>z</p>"
    );
    let applied = a.undo();
    sync(&mut a, &mut b);
    assert!(applied);
    known_wrong(
        "s19 wrong block",
        &a.html(),
        "<blockquote><p></p><p>a</p></blockquote><p>z</p>",
        "<blockquote><p>a</p><p></p></blockquote><p>z</p>",
    );
}

/// Second review of #1889: a peer's changes right beside mine, one keystroke per
/// update, which the history folds into one map; folding them used to lose the
/// position between them.
///
/// A peer fixes my typo: backspaces over my `e` and types `Q`, each keystroke its
/// own update. My undo of `e` has nothing left to do.
#[test]
fn z1_peer_replaces_my_char_keystroke_by_keystroke() {
    let (mut a, mut b) = world("<p>xy</p>", 31);
    a.type_at(2, "e", 1000);
    sync(&mut a, &mut b);
    b.delete(2, 3, 2000);
    sync(&mut a, &mut b);
    b.type_at(2, "Q", 3000);
    sync(&mut a, &mut b);
    assert_eq!(a.html(), "<p>xQy</p>");
    assert!(a.undo());
    sync(&mut a, &mut b);
    assert_eq!(a.html(), "<p>xQy</p>");
}

/// The peer types `Q` before my `e`, then deletes my `e` (Delete key).
#[test]
fn z2_peer_types_before_my_char_then_deletes_it() {
    let (mut a, mut b) = world("<p>xy</p>", 32);
    a.type_at(2, "e", 1000);
    sync(&mut a, &mut b);
    b.type_at(2, "Q", 2000);
    sync(&mut a, &mut b);
    b.delete(3, 4, 3000);
    sync(&mut a, &mut b);
    assert_eq!(a.html(), "<p>xQy</p>");
    assert!(a.undo());
    sync(&mut a, &mut b);
    assert_eq!(a.html(), "<p>xQy</p>");
}

/// I delete `m`; the peer then deletes the chars on both sides of where it was, one
/// keystroke per update. My undo re-inserts `m`.
#[test]
fn z3_peer_deletes_both_neighbours_of_my_deletion() {
    let (mut a, mut b) = world("<p>abcmde</p>", 33);
    a.delete(4, 5, 1000);
    sync(&mut a, &mut b);
    b.delete(4, 5, 2000);
    sync(&mut a, &mut b);
    b.delete(3, 4, 3000);
    sync(&mut a, &mut b);
    assert_eq!(a.html(), "<p>abe</p>");
    assert!(a.undo());
    sync(&mut a, &mut b);
    assert_eq!(a.html(), "<p>abme</p>");
}

/// A word of mine; the peer types inside it and then deletes my char right after
/// their insertion, one keystroke per update; I undo my word.
#[test]
fn z4_peer_types_in_my_word_then_deletes_my_next_char() {
    let (mut a, mut b) = world("<p>z</p>", 34);
    a.type_at(1, "abcdef", 1000); // abcdefz
    sync(&mut a, &mut b);
    b.type_at(4, "Q", 2000); // abcQdefz
    sync(&mut a, &mut b);
    b.delete(5, 6, 3000); // abcQefz
    sync(&mut a, &mut b);
    assert!(a.undo());
    sync(&mut a, &mut b);
    assert_eq!(a.html(), "<p>Qz</p>");
}
