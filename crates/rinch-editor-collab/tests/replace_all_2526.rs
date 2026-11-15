//! Issue #2526: a transaction that edits every paragraph of a document (a
//! replace-all) is written through the mapped path (`record_local_mapped`, #1885) in
//! time that grows with the document, not with its square.
//!
//! The mapped path found each changed block's places by searching the whole document
//! for the block and walking every step of the transaction back for each of its
//! places, and the top-level diff compared every changed block with every other to
//! tell whether the change shared one: (paragraphs × steps) for a replace-all. The
//! first test pins the work it does (`testing::mapped_work`) at two sizes. The rest
//! hold the path to what it wrote before: two peers converge, a peer's concurrent
//! edits between the places land once, undo and redo of a replace-all round-trip,
//! remote changes interleave, and marks and inline atoms inside the replaced
//! paragraphs survive.

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use rinch_editor_collab::testing::{
    mapped_work, session_from_bytes_with_client_id, session_with_client_id,
};
use rinch_editor_collab::{CollabPlugin, CollabSession};
use rinch_editor_core::serialize::slice_from_html;
use rinch_editor_core::{
    AttrValue, Attrs, BatchEdit, EditorState, Fragment, Mapping, Mark, Node, Plugin, Schema, Slice,
    Transaction, default_plugins,
};

fn plugins() -> Vec<Rc<dyn Plugin>> {
    let mut p = default_plugins();
    p.push(Rc::new(CollabPlugin));
    p
}

fn para(s: &Schema, text: &str) -> Node {
    s.branch("paragraph", Fragment::from_node(s.text(text).unwrap()))
        .unwrap()
}

fn doc_of(s: &Schema, blocks: Vec<Node>) -> Node {
    s.branch("doc", Fragment::from_children(blocks)).unwrap()
}

/// The content start of every top-level block of `doc`.
fn starts(doc: &Node) -> Vec<usize> {
    let mut out = Vec::with_capacity(doc.child_count());
    let mut pos = 0;
    for i in 0..doc.child_count() {
        out.push(pos + 1);
        pos += doc.child(i).node_size();
    }
    out
}

/// How the replace-all's edits are made.
#[derive(Clone, Copy, Debug)]
enum Order {
    /// One step per place, first paragraph first (positions mapped through the
    /// transaction so far, as a find-and-replace does).
    Forward,
    /// One step per place, last paragraph first.
    Backward,
    /// One `BatchStep` holding every place.
    Batch,
}

/// A replace-all over `state`: in every top-level block, the `len` positions at each
/// offset of `at` (from the block's content start) become `with(block, place)`.
fn replace_all(
    state: &EditorState,
    at: &[usize],
    len: usize,
    order: Order,
    with: &mut dyn FnMut(usize, usize) -> String,
) -> Transaction {
    let s = state.schema();
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for (b, start) in starts(&state.doc).into_iter().enumerate() {
        for (k, off) in at.iter().enumerate() {
            edits.push((start + off, start + off + len, with(b, k)));
        }
    }
    let frag = |t: &str| Fragment::from_node(s.text(t).unwrap());
    let mut tr = state.tr();
    match order {
        Order::Forward => {
            for (from, to, t) in &edits {
                let (from, to) = (tr.mapping().map(*from, 1), tr.mapping().map(*to, -1));
                tr.replace_with(from, to, frag(t)).unwrap();
            }
        }
        Order::Backward => {
            for (from, to, t) in edits.iter().rev() {
                tr.replace_with(*from, *to, frag(t)).unwrap();
            }
        }
        Order::Batch => {
            let edits = edits
                .iter()
                .map(|(from, to, t)| BatchEdit::Replace {
                    from: *from,
                    to: *to,
                    slice: Slice::new(frag(t), 0, 0),
                })
                .collect();
            tr.batch(edits).unwrap();
        }
    }
    tr
}

// --- the cost --------------------------------------------------------------------

/// The work recording a replace-all over `n` 80-char paragraphs costs, at the places
/// `at` of each, through the mapped path.
fn replace_all_work(n: usize, at: &[usize], order: Order) -> u64 {
    let s = Rc::new(Schema::starter_kit());
    let blocks = (0..n)
        .map(|i| {
            let mut t = format!("p{i:05} ");
            while t.len() < 80 {
                t.push_str("lorem ipsum ");
            }
            para(&s, &t)
        })
        .collect();
    let state = EditorState::create(s.clone(), doc_of(&s, blocks), plugins());
    let mut session = CollabSession::new(&state).unwrap();
    let tr = replace_all(&state, at, 5, order, &mut |_, _| "XYZ".into());
    let mapping = tr.mapping().clone();
    let next = state.apply(tr);
    let w0 = mapped_work();
    session
        .record_local_mapped(&s, &state.doc, &next.doc, &mapping)
        .expect("the replace-all projects");
    let work = mapped_work() - w0;
    assert_eq!(
        next.doc,
        session.projected_doc(&s).unwrap(),
        "model ≡ project(model)"
    );
    work
}

/// Twice the paragraphs cost at most about twice the work: (paragraphs × steps) is four
/// times. One place per paragraph (the diff then writes each block whole), two (the
/// mapped path writes each place alone), steps first-to-last and last-to-first, and one
/// batch step holding them all.
#[test]
fn a_replace_all_costs_work_in_proportion_to_the_document() {
    for (at, order) in [
        (&[10][..], Order::Forward),
        (&[10, 50][..], Order::Forward),
        (&[10, 50][..], Order::Backward),
        (&[10, 50][..], Order::Batch),
    ] {
        let small = replace_all_work(200, at, order);
        let large = replace_all_work(400, at, order);
        // Positive control: the mapped path ran and counted.
        assert!(small > 0, "{at:?} {order:?}: no work counted");
        assert!(
            large * 10 <= small * 27,
            "{at:?} {order:?}: 200 paragraphs cost {small}, 400 cost {large} \
             ({:.2}×; linear is 2×, quadratic 4×)",
            large as f64 / small as f64
        );
    }
}

// --- two peers ---------------------------------------------------------------------

struct Peer {
    schema: Rc<Schema>,
    state: EditorState,
    session: CollabSession,
}

impl Peer {
    fn host(s: &Rc<Schema>, doc: Node) -> Peer {
        let state = EditorState::create(s.clone(), doc, plugins());
        let session = session_with_client_id(&state, 1).unwrap();
        Peer {
            schema: s.clone(),
            state,
            session,
        }
    }

    fn join(&self, client_id: u64) -> Peer {
        let session =
            session_from_bytes_with_client_id(&self.session.snapshot(), client_id).unwrap();
        let doc = session.projected_doc(&self.schema).unwrap();
        Peer {
            schema: self.schema.clone(),
            state: EditorState::create(self.schema.clone(), doc, plugins()),
            session,
        }
    }

    /// Record the move to `next` the way the editor does: with its mapping.
    fn record(&mut self, next: EditorState, mapping: &Mapping) {
        self.session
            .record_local_mapped(&self.schema, &self.state.doc, &next.doc, mapping)
            .expect("the edit projects");
        self.state = next;
        assert_eq!(
            self.state.doc,
            self.session.projected_doc(&self.schema).unwrap(),
            "after a local edit: model ≡ project(model)"
        );
    }

    fn commit(&mut self, tr: Transaction) {
        let mapping = tr.mapping().clone();
        let next = self.state.apply(tr);
        self.record(next, &mapping);
    }

    fn run(&mut self, name: &str) {
        let (next, mapping) = self.state.run_mapped(name).expect("the command applies");
        self.record(next, &mapping);
    }

    fn send(&mut self) -> Vec<u8> {
        self.session.save_incremental().unwrap()
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
}

fn exchange(a: &mut Peer, b: &mut Peer) {
    let (da, db) = (a.send(), b.send());
    a.receive(&db);
    b.receive(&da);
    assert_eq!(a.state.doc, b.state.doc, "converged");
}

/// The text under `n`, an inline atom as U+FFFC.
fn flat(n: &Node) -> String {
    if let Some(t) = n.text() {
        return t.to_string();
    }
    if n.is_leaf() {
        return '\u{FFFC}'.to_string();
    }
    (0..n.child_count()).map(|i| flat(n.child(i))).collect()
}

/// A source of chars never used before.
struct Fresh(u32);

impl Fresh {
    fn char(&mut self) -> char {
        self.0 += 1;
        char::from_u32(self.0).unwrap()
    }
    fn string(&mut self, n: usize) -> String {
        (0..n).map(|_| self.char()).collect()
    }
}

/// Whether every char of `doc` that nobody deleted is in it exactly once, and no
/// deleted one is: the check that every edit merged.
fn merged(doc: &Node, all: &BTreeSet<char>, deleted: &BTreeSet<char>) -> Result<(), String> {
    let mut count: BTreeMap<char, usize> = BTreeMap::new();
    for c in flat(doc).chars() {
        *count.entry(c).or_default() += 1;
    }
    for c in all {
        let want = usize::from(!deleted.contains(c));
        let got = count.get(c).copied().unwrap_or(0);
        if got != want {
            return Err(format!("{c:?} is there {got} times, expected {want}"));
        }
    }
    Ok(())
}

/// A host and a guest on `n` paragraphs of 30 unique chars, synced.
fn unique_world(n: usize, fresh: &mut Fresh) -> (Peer, Peer, BTreeSet<char>) {
    let s = Rc::new(Schema::starter_kit());
    let texts: Vec<String> = (0..n).map(|_| fresh.string(30)).collect();
    let all = texts.iter().flat_map(|t| t.chars()).collect();
    let doc = doc_of(&s, texts.iter().map(|t| para(&s, t)).collect());
    let mut a = Peer::host(&s, doc);
    let mut b = a.join(2);
    let _ = (a.send(), b.send());
    (a, b, all)
}

/// The chars `from..to` of top-level block `block` of `doc`.
fn chars_at(doc: &Node, block: usize, from: usize, to: usize) -> Vec<char> {
    flat(doc.child(block))
        .chars()
        .skip(from)
        .take(to - from)
        .collect()
}

/// A replace-all on A (two chars at offsets 5 and 20 of every paragraph become one
/// fresh char) while B, in every third paragraph, types a fresh char at 12 and deletes
/// the char at 15 — between A's two places. Every edit lands once.
#[test]
fn a_replace_all_merges_with_a_peers_concurrent_edits_between_its_places() {
    for order in [Order::Forward, Order::Backward, Order::Batch] {
        let mut fresh = Fresh(0x4E00);
        let (mut a, mut b, mut all) = unique_world(40, &mut fresh);
        let mut deleted = BTreeSet::new();
        // B's edits, last paragraph first so each position is the one drawn.
        let mut tr = b.state.tr();
        for (i, start) in starts(&b.state.doc).into_iter().enumerate().rev() {
            if i % 3 != 0 {
                continue;
            }
            deleted.extend(chars_at(&b.state.doc, i, 15, 16));
            tr.delete(start + 15, start + 16).unwrap();
            let c = fresh.char();
            all.insert(c);
            tr.replace_with(
                start + 12,
                start + 12,
                Fragment::from_node(b.schema.text(&c.to_string()).unwrap()),
            )
            .unwrap();
        }
        b.commit(tr);
        // A's replace-all.
        for (i, _) in starts(&a.state.doc).iter().enumerate() {
            for off in [5, 20] {
                deleted.extend(chars_at(&a.state.doc, i, off, off + 2));
            }
        }
        let mut typed = Vec::new();
        let tr = replace_all(&a.state, &[5, 20], 2, order, &mut |_, _| {
            let c = fresh.char();
            typed.push(c);
            c.to_string()
        });
        all.extend(typed);
        a.commit(tr);
        exchange(&mut a, &mut b);
        merged(&a.state.doc, &all, &deleted).unwrap_or_else(|e| panic!("{order:?}: {e}"));
    }
}

/// Undo of a replace-all, a peer's edit, redo: each records through the mapped path
/// (the undo's steps run last paragraph first), and every edit lands once.
#[test]
fn undo_and_redo_of_a_replace_all_round_trip_among_peers() {
    let mut fresh = Fresh(0x4E00);
    let (mut a, mut b, mut all) = unique_world(30, &mut fresh);
    let original = a.state.doc.clone();
    let mut typed = Vec::new();
    let tr = replace_all(&a.state, &[4, 18], 3, Order::Forward, &mut |_, _| {
        let s = fresh.string(2);
        typed.extend(s.chars());
        s
    });
    all.extend(typed.iter().copied());
    a.commit(tr);
    let replaced = a.state.doc.clone();
    exchange(&mut a, &mut b);

    a.run("undo");
    assert_eq!(a.state.doc, original, "the undo restores the document");
    // B types a fresh char between the places of every other paragraph meanwhile.
    let mut tr = b.state.tr();
    for (i, start) in starts(&b.state.doc).into_iter().enumerate().rev() {
        if i % 2 == 0 {
            let c = fresh.char();
            all.insert(c);
            tr.replace_with(
                start + 12,
                start + 12,
                Fragment::from_node(b.schema.text(&c.to_string()).unwrap()),
            )
            .unwrap();
        }
    }
    b.commit(tr);
    exchange(&mut a, &mut b);
    let deleted: BTreeSet<char> = typed.iter().copied().collect();
    merged(&a.state.doc, &all, &deleted).unwrap();

    a.run("redo");
    exchange(&mut a, &mut b);
    let mut deleted = BTreeSet::new();
    for (i, _) in starts(&original).iter().enumerate() {
        deleted.extend(chars_at(&original, i, 4, 7));
        deleted.extend(chars_at(&original, i, 18, 21));
    }
    merged(&a.state.doc, &all, &deleted).unwrap();
    // Without B's chars the redo is the replace-all again.
    let b_chars: BTreeSet<char> = flat(&a.state.doc)
        .chars()
        .filter(|c| !flat(&replaced).contains(*c))
        .collect();
    assert_eq!(b_chars.len(), 15, "B's 15 chars, and nothing else new");
}

/// Remote changes between local replace-alls: A replaces, takes in B's typing,
/// replaces again over the merged document, and undoes both while B keeps typing.
#[test]
fn remote_changes_interleave_with_replace_alls_and_their_undo() {
    let mut fresh = Fresh(0x4E00);
    let (mut a, mut b, mut all) = unique_world(30, &mut fresh);
    let mut deleted = BTreeSet::new();
    let b_types = |b: &mut Peer, fresh: &mut Fresh, all: &mut BTreeSet<char>, at: usize| {
        let mut tr = b.state.tr();
        for start in starts(&b.state.doc).into_iter().rev() {
            let c = fresh.char();
            all.insert(c);
            tr.replace_with(
                start + at,
                start + at,
                Fragment::from_node(b.schema.text(&c.to_string()).unwrap()),
            )
            .unwrap();
        }
        b.commit(tr);
    };
    let mut time = 0;
    let mut a_replaces = |a: &mut Peer, fresh: &mut Fresh, all: &mut BTreeSet<char>, at| {
        let doc = a.state.doc.clone();
        let mut typed = Vec::new();
        let mut tr = replace_all(&a.state, at, 1, Order::Forward, &mut |_, _| {
            let c = fresh.char();
            typed.push(c);
            c.to_string()
        });
        for (i, _) in starts(&doc).iter().enumerate() {
            for &off in at {
                deleted.extend(chars_at(&doc, i, off, off + 1));
            }
        }
        all.extend(typed.iter().copied());
        // Far enough apart that the history keeps them as two undo events.
        time += 10_000;
        tr.set_time(time);
        a.commit(tr);
        typed
    };
    let first = a_replaces(&mut a, &mut fresh, &mut all, &[2, 25]);
    b_types(&mut b, &mut fresh, &mut all, 10);
    let db = b.send();
    a.receive(&db);
    let second = a_replaces(&mut a, &mut fresh, &mut all, &[8, 16]);
    b_types(&mut b, &mut fresh, &mut all, 20);
    exchange(&mut a, &mut b);
    merged(&a.state.doc, &all, &deleted).unwrap();

    // Undo both replace-alls: what they typed goes, what they deleted comes back,
    // and B's typing stays.
    a.run("undo");
    a.run("undo");
    exchange(&mut a, &mut b);
    let deleted: BTreeSet<char> = first.into_iter().chain(second).collect();
    merged(&a.state.doc, &all, &deleted).unwrap();
}

// --- marks and inline atoms ------------------------------------------------------

/// The chars of top-level block `block` of `doc`, an inline atom as U+FFFC, and the
/// content start of the block.
fn block_text(doc: &Node, block: usize) -> (Vec<char>, usize) {
    (flat(doc.child(block)).chars().collect(), starts(doc)[block])
}

/// The position of the first `needle` in block `block` of `doc`.
fn find(doc: &Node, block: usize, needle: &str) -> usize {
    let (text, start) = block_text(doc, block);
    let n: Vec<char> = needle.chars().collect();
    let i = text
        .windows(n.len())
        .position(|w| w == n.as_slice())
        .unwrap_or_else(|| panic!("{needle:?} in block {block}"));
    start + i
}

/// Every inline atom of `doc` with its `alt`, in order.
fn images(doc: &Node) -> Vec<String> {
    let mut out = Vec::new();
    doc.nodes_between(0, doc.content_size(), &mut |n: &Node, _, _| {
        if n.type_name() == "image" {
            out.push(match n.attrs().get("alt") {
                Some(AttrValue::Str(s)) => s.to_string(),
                other => format!("{other:?}"),
            });
        }
        true
    });
    out
}

/// A replace-all whose places sit inside a bold run, beside an inline image and
/// inside a link, in one transaction that also italicizes a word of every paragraph,
/// while B types inside the bold run between A's places and changes every image's
/// `alt`. B's typing lands once, its `alt` survives (the image kept its identity), and
/// both peers read the same marks.
#[test]
fn marks_and_inline_atoms_inside_replaced_paragraphs_survive() {
    let s = Rc::new(Schema::starter_kit());
    let p = r#"<p>alpha <strong>bravo charlie delta</strong> echo <img src="a.png" alt="old"> foxtrot <a href="https://x.test">golf hotel</a> india</p>"#;
    let html: String = (0..20).map(|_| p).collect();
    let slice = slice_from_html(&s, &html).unwrap();
    let doc = s.branch("doc", slice.content.clone()).unwrap();
    let mut a = Peer::host(&s, doc);
    let mut b = a.join(2);
    let _ = (a.send(), b.send());
    let n = a.state.doc.child_count();
    assert_eq!(n, 20);

    // B: "Z" inside "charlie", and every image's alt.
    let mut tr = b.state.tr();
    for i in (0..n).rev() {
        // The image first: it comes after "charlie", so its position is still the drawn one.
        let img = find(&b.state.doc, i, "\u{FFFC}");
        tr.set_node_attr(img, "alt", AttrValue::from("peer"))
            .unwrap();
        let at = find(&b.state.doc, i, "charlie") + 4;
        tr.replace_with(at, at, Fragment::from_node(s.text("Z").unwrap()))
            .unwrap();
    }
    b.commit(tr);

    // A: "bravo" → "BRAVO", " foxtrot" → " FOXTROT" (right after the image), "hotel"
    // → "HOTEL" (in the link), each a step, and "india" italicized.
    let em = Mark::new(s.mark_type("italic").unwrap().clone(), Attrs::new());
    let mut tr = a.state.tr();
    for i in 0..n {
        for (needle, with) in [
            ("bravo", "BRAVO"),
            (" foxtrot", " FOXTROT"),
            ("hotel", "HOTEL"),
        ] {
            let from = tr.mapping().map(find(&a.state.doc, i, needle), 1);
            let to = from + needle.chars().count();
            tr.replace_with(from, to, Fragment::from_node(s.text(with).unwrap()))
                .unwrap();
        }
        let from = tr.mapping().map(find(&a.state.doc, i, "india"), 1);
        tr.add_mark(from, from + 5, em.clone()).unwrap();
    }
    a.commit(tr);
    exchange(&mut a, &mut b);

    let text = flat(&a.state.doc);
    assert_eq!(
        text.matches("charZlie").count(),
        n,
        "B's typing, once each: {text}"
    );
    assert_eq!(text.matches("BRAVO").count(), n);
    assert_eq!(text.matches(" FOXTROT").count(), n);
    assert_eq!(text.matches("HOTEL").count(), n);
    assert_eq!(
        images(&a.state.doc),
        vec!["peer".to_string(); n],
        "every image kept its identity and took B's alt"
    );
    // Every paragraph, marks and all: the plain replacements left the bold run and the
    // link, B's plain "Z" split the bold run, "india" is italic.
    let want = r#"<p>alpha BRAVO<strong> char</strong>Z<strong>lie delta</strong> echo <img alt="peer" src="a.png"> FOXTROT <a href="https://x.test">golf </a>HOTEL <em>india</em></p>"#;
    assert_eq!(
        rinch_editor_core::serialize::node_to_html(&a.state.doc),
        want.repeat(n)
    );
}

/// A caller handing `record_local_mapped` an inverted mapping (`Mapping::invert` of
/// the change that went the other way, which describes this one) gets the change
/// diffed the usual way: the CRDT reads as the model, and a peer converges on it.
#[test]
fn an_inverted_mapping_is_recorded_as_a_diff() {
    let s = Rc::new(Schema::starter_kit());
    let edited = |doc: Node| {
        let state = EditorState::create(s.clone(), doc, plugins());
        let tr = replace_all(&state, &[2, 7], 1, Order::Backward, &mut |_, _| "XY".into());
        let mapping = tr.mapping().clone();
        (state.apply(tr).doc, mapping)
    };
    let original = doc_of(&s, vec![para(&s, "abcdefghij"), para(&s, "klmnopqrst")]);
    let (replaced, mapping) = edited(original.clone());
    // The host starts on the replaced document and goes back to the original, with
    // the inverse of the mapping that went the other way.
    let mut a = Peer::host(&s, replaced);
    let mut b = a.join(2);
    let _ = (a.send(), b.send());
    let next = EditorState::create(s.clone(), original.clone(), plugins());
    let inverse = mapping.invert();
    assert!(inverse.maps().iter().all(|m| m.is_inverted()));
    a.record(next, &inverse);
    assert_eq!(a.state.doc, original);
    exchange(&mut a, &mut b);
}
