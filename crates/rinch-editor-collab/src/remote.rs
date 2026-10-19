//! Remote → model: turn a converged CRDT change back into editor steps.
//!
//! When a peer's change arrives, yrs merges it (convergence is *its* job). This module
//! closes the loop back to the model: [`build_remote_transaction`] rebuilds the model
//! from the converged CRDT ([`CollabDoc::to_doc`](crate::CollabDoc::to_doc)) and emits a
//! minimal **block-level** `ReplaceStep` (common-prefix/suffix on blocks, so untouched
//! blocks keep their identity). Because the model is rebuilt from the *same* CRDT both
//! peers converge to, `model ≡ project(model)` is restored exactly — no position-math
//! risk. Every remote change is therefore a block replace and never a text splice, which
//! is what makes a block changing *kind* — a paragraph becoming a `horizontal_rule`, or
//! back — nothing special here; only the caret carry below has to notice.
//!
//! There is deliberately no engine type in this file. The convergence-critical path only
//! ever needs "give me the converged document as a `Node`", which is why swapping the
//! CRDT engine (#190) left it untouched.
//!
//! A surgical, cursor-preserving translation (a remote change described as
//! insert/delete/mark ops, so a caret could be re-anchored instead of re-derived) used
//! to sit alongside this and was **dropped with automerge**: it consumed
//! `automerge::Patch`, was documented as not convergence-critical, and never fed the
//! session. It can be rebuilt on yrs observer deltas (`TextRef::observe` → `TextEvent`)
//! if a future refinement wants it.

use rinch_editor_core::{
    AttrValue, EditorState, Fragment, Node, Pos, Selection, SetNodeAttrStep, Slice, Transaction,
};

use crate::error::Result;
use crate::projection::BlockId;

/// Transaction meta key marking a transaction as a remote (collab) application, so the
/// history plugin and any origin-sensitive logic can tell it apart from local typing.
pub const ORIGIN_REMOTE: &str = "collabOriginRemote";

/// The block-level difference between an old document and the converged one:
/// the top-level blocks the two share at their start and end, and the old
/// document's positions around the changed middle.
struct BlockDiff {
    prefix: usize,
    suffix: usize,
    /// Position before the first changed block of the old document.
    start: usize,
    /// Position after its last changed block.
    end: usize,
}

/// `None` when `old` and `target` are the same document.
fn block_diff(old: &Node, target: &Node) -> Option<BlockDiff> {
    block_diff_with(old, target, None)
}

/// [`block_diff`], where with `ids` a top-level block holding a leaf block atom that
/// is another CRDT block on the two sides is changed even when equal by value
/// ([`AtomIds`]): `None` only when the documents are equal **and** every atom is
/// the same block.
fn block_diff_with(old: &Node, target: &Node, ids: Option<&AtomIds>) -> Option<BlockDiff> {
    let on = old.child_count();
    let nn = target.child_count();

    // Common leading/trailing blocks by structural equality (different trees, so no
    // Rc fast-path here).
    let mut prefix = 0;
    while prefix < on && prefix < nn && old.child(prefix) == target.child(prefix) {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < on - prefix
        && suffix < nn - prefix
        && old.child(on - 1 - suffix) == target.child(nn - 1 - suffix)
    {
        suffix += 1;
    }
    if let Some(ids) = ids
        && ids.old != ids.new
    {
        // The equal blocks at either end hold the same number of atoms on both
        // sides; stop each run at the first block whose atoms are other blocks.
        let mut k = 0;
        for i in 0..prefix {
            let c = block_atoms(old.child(i));
            if ids.old[k..k + c] != ids.new[k..k + c] {
                prefix = i;
                break;
            }
            k += c;
        }
        let (ol, nl) = (ids.old.len(), ids.new.len());
        let mut k = 0;
        for s in 0..suffix {
            let c = block_atoms(old.child(on - 1 - s));
            if ids.old[ol - k - c..ol - k] != ids.new[nl - k - c..nl - k] {
                suffix = s;
                break;
            }
            k += c;
        }
    }
    if let Some(inline) = ids.and_then(|ids| ids.inline_fitting(old, target))
        && inline.old != inline.new
    {
        // The same for inline atoms, where both sides are known to differ.
        let differ = |a: &Option<String>, b: &Option<String>| a.is_some() && b.is_some() && a != b;
        let mut k = 0;
        for i in 0..prefix {
            let c = inline_atoms(old.child(i));
            if (k..k + c).any(|j| differ(&inline.old[j], &inline.new[j])) {
                prefix = i;
                break;
            }
            k += c;
        }
        let (ol, nl) = (inline.old.len(), inline.new.len());
        let mut k = 0;
        for s in 0..suffix {
            let c = inline_atoms(old.child(on - 1 - s));
            if (0..c).any(|j| differ(&inline.old[ol - k - c + j], &inline.new[nl - k - c + j])) {
                suffix = s;
                break;
            }
            k += c;
        }
    }
    if prefix == on && nn == on {
        return None; // identical
    }

    // Model position before the first changed block, and after the last changed block.
    let start: usize = (0..prefix).map(|j| old.child(j).node_size()).sum();
    let end: usize = (0..on - suffix).map(|j| old.child(j).node_size()).sum();
    Some(BlockDiff {
        prefix,
        suffix,
        start,
        end,
    })
}

/// The transaction that replaces the changed middle of `state`'s document with
/// `target`'s, selection untouched (mapped).
fn replace_changed_blocks(
    state: &EditorState,
    target: &Node,
    diff: &BlockDiff,
) -> Result<Transaction> {
    let nn = target.child_count();
    // The replacement blocks (the changed middle of the new doc).
    let mid: Vec<Node> = (diff.prefix..nn - diff.suffix)
        .map(|j| target.child(j).clone())
        .collect();
    let slice = Slice::new(Fragment::from_children(mid), 0, 0);
    let mut tr = state.tr();
    tr.replace(diff.start, diff.end, slice)?;
    Ok(tr)
}

/// One attribute a remote change sets (`Some`) or removes (`None`) on the node
/// that starts at a position of the old document.
type AttrChange = (usize, String, Option<AttrValue>);

/// The CRDT block of every **leaf block atom** (an `embed`, a `horizontal_rule`) of
/// the old document and of the converged one, in document order — what proves a
/// leaf block atom is the same atom on both sides, whether its attributes changed
/// or not. Read by `CollabDoc::to_doc_with_atom_ids`; each list is as long as its
/// document has leaf block atoms ([`AtomIds::fits`]).
pub(crate) struct AtomIds<'a> {
    pub(crate) old: &'a [BlockId],
    pub(crate) new: &'a [BlockId],
    /// The identity of every **inline atom** (an `image`, a `hard_break`) of the
    /// old document and the converged one, in document order, when known
    /// ([`InlineIds`]).
    pub(crate) inline: Option<InlineIds<'a>>,
}

/// The identity (`crate::atoms`) of every inline atom of the old document and of
/// the converged one, in document order: `None` where it is not known (an atom
/// the model made since the last read). A picture whose attributes a peer changed
/// is the same identity on both sides, and gets `SetNodeAttrStep`s, so a node
/// view drawn for it stays mounted; another identity at its place — a moved,
/// re-inserted or replaced picture — is replaced, and the view remounted. An
/// unknown identity never proves sameness for an attribute change: on doubt the
/// block is replaced.
#[derive(Clone, Copy)]
pub(crate) struct InlineIds<'a> {
    pub(crate) old: &'a [Option<String>],
    pub(crate) new: &'a [Option<String>],
}

impl AtomIds<'_> {
    /// Whether the lists describe `old` and `target`: one id per leaf block atom.
    pub(crate) fn fits(&self, old: &Node, target: &Node) -> bool {
        self.old.len() == block_atoms(old) && self.new.len() == block_atoms(target)
    }

    /// The inline identities, when they describe `old` and `target`.
    fn inline_fitting(&self, old: &Node, target: &Node) -> Option<InlineIds<'_>> {
        self.inline
            .filter(|i| i.old.len() == inline_atoms(old) && i.new.len() == inline_atoms(target))
    }
}

/// Whether `node` is an inline atom (an inline leaf that is not text).
fn is_inline_atom(node: &Node) -> bool {
    node.is_inline() && node.is_leaf() && !node.is_text()
}

/// How many inline atoms `node` is or holds, in document order.
pub(crate) fn inline_atoms(node: &Node) -> usize {
    if is_inline_atom(node) {
        1
    } else if node.is_leaf() {
        0
    } else {
        node.content().children().iter().map(inline_atoms).sum()
    }
}

/// The inline atoms `node` is or holds, in document order.
pub(crate) fn inline_atom_nodes(node: &Node, out: &mut Vec<Node>) {
    if is_inline_atom(node) {
        out.push(node.clone());
    } else if !node.is_leaf() {
        for child in node.content().children() {
            inline_atom_nodes(child, out);
        }
    }
}

/// Whether `node` is a leaf block atom (the projection's predicate).
fn is_block_atom(node: &Node) -> bool {
    crate::projection::is_leaf_block_atom(node.node_type())
}

/// How many leaf block atoms `node` is or holds.
pub(crate) fn block_atoms(node: &Node) -> usize {
    if is_block_atom(node) {
        1
    } else if node.is_textblock() || node.is_leaf() {
        0 // a textblock's inline content holds no block
    } else {
        node.content().children().iter().map(block_atoms).sum()
    }
}

/// The leaf block atoms `node` is or holds, in the order [`block_atoms`] counts
/// them (document order).
pub(crate) fn atom_nodes(node: &Node, out: &mut Vec<Node>) {
    if is_block_atom(node) {
        out.push(node.clone());
    } else if !(node.is_textblock() || node.is_leaf()) {
        for child in node.content().children() {
            atom_nodes(child, out);
        }
    }
}

/// How a pair of leaf block atoms at one place, of one type, is told apart.
#[derive(Clone, Copy)]
enum AtomMatch<'a> {
    /// By their CRDT blocks: the same block is the same atom (its attributes may
    /// differ); another block is another atom, equal values or not.
    Ids(&'a AtomIds<'a>),
    /// By place alone: an atom whose attributes differ is that atom changed —
    /// right for asking whether any position moved, wrong for identity.
    Positional,
}

/// One change [`changed_middle`] found, at a position of the old document.
enum Edit {
    /// Replace the node at `from..to` with `node`.
    Replace { from: usize, to: usize, node: Node },
    /// Set or remove attributes of the leaf block atom at a position.
    Attrs(Vec<AttrChange>),
}

/// The changes that turn `old`'s changed middle (`diff`) into `target`'s, child by
/// child: a changed textblock (or any node that changed shape) is replaced on its
/// own, a container of the same shape is walked into, a leaf block atom that is the
/// same atom ([`AtomMatch`]) gets its attributes set, and one that is another atom —
/// even an equal one — is replaced, so a view drawn for the old one is remounted.
/// `None` when the middle's top-level block count changed: then the whole middle is
/// one replace.
fn changed_middle(
    old: &Node,
    target: &Node,
    diff: &BlockDiff,
    atoms: AtomMatch,
) -> Option<Vec<Edit>> {
    let (on, nn) = (old.child_count(), target.child_count());
    if on != nn {
        return None;
    }
    let before: usize = (0..diff.prefix).map(|i| block_atoms(old.child(i))).sum();
    let inline = match atoms {
        AtomMatch::Ids(ids) => ids.inline_fitting(old, target),
        AtomMatch::Positional => None,
    };
    let inline_before: usize = (0..diff.prefix).map(|i| inline_atoms(old.child(i))).sum();
    // An identity two chars carry (two copies of one picture that split apart:
    // `crate::atoms`, *What is left*) names neither of them.
    let shared = inline
        .map(|i| {
            let mut seen = std::collections::HashSet::new();
            let mut shared = std::collections::HashSet::new();
            for list in [i.old, i.new] {
                seen.clear();
                for id in list.iter().flatten() {
                    if !seen.insert(id.as_str()) {
                        shared.insert(id.clone());
                    }
                }
            }
            shared
        })
        .unwrap_or_default();
    let mut walk = MiddleWalk {
        shared,
        edits: Vec::new(),
        atoms,
        ko: before,
        kn: before,
        inline,
        io: inline_before,
        in_: inline_before,
    };
    let mut pos = diff.start;
    for i in diff.prefix..on - diff.suffix {
        let (a, b) = (old.child(i), target.child(i));
        walk.pair(a, b, pos);
        pos += a.node_size();
    }
    Some(walk.edits)
}

/// The walk behind [`changed_middle`].
struct MiddleWalk<'a> {
    edits: Vec<Edit>,
    atoms: AtomMatch<'a>,
    /// The document-order index of the next leaf block atom, old and new.
    ko: usize,
    kn: usize,
    /// The inline atoms' identities, when known, and the index of the next one,
    /// old and new.
    inline: Option<InlineIds<'a>>,
    io: usize,
    in_: usize,
    /// Identities more than one inline atom carries, on either side.
    shared: std::collections::HashSet<String>,
}

impl MiddleWalk<'_> {
    /// Whether the leaf block atoms of `a` and `b` (counted `ca`, `cb`) are the same
    /// atoms, in order.
    fn same_atoms(&self, ca: usize, cb: usize) -> bool {
        match self.atoms {
            AtomMatch::Ids(ids) => {
                ca == cb && ids.old[self.ko..self.ko + ca] == ids.new[self.kn..self.kn + cb]
            }
            AtomMatch::Positional => ca == cb,
        }
    }

    /// Whether the inline atoms of `a` and `b` (counted `ia`, `ib`) are not shown
    /// to be other atoms: no pair of them is two known, different identities.
    fn same_inline(&self, ia: usize, ib: usize) -> bool {
        let Some(inline) = self.inline else {
            return true;
        };
        ia == ib
            && (0..ia).all(|k| {
                match (&inline.old[self.io + k], &inline.new[self.in_ + k]) {
                    (Some(x), Some(y)) => x == y,
                    _ => true,
                }
            })
    }

    /// The attribute changes that turn textblock `a` into `b` when the two differ
    /// only in the attributes of inline atoms each **known** to be the same atom
    /// on both sides (identity). `None` otherwise.
    fn inline_attr_changes(&self, a: &Node, b: &Node, pos: usize) -> Option<Vec<AttrChange>> {
        let inline = self.inline?;
        if !(a.is_textblock()
            && a.type_name() == b.type_name()
            && a.marks() == b.marks()
            && a.attrs() == b.attrs()
            && a.child_count() == b.child_count())
        {
            return None;
        }
        // One node twice in the block (a picture copied and moved in one editor:
        // the two copies are one `Rc`) is no atom the model can tell from the
        // other; the projection cannot either, so the next local edit of either
        // may write the other's char. Replaced, the block is read afresh.
        let atoms: Vec<&Node> = (0..a.child_count())
            .map(|i| a.child(i))
            .filter(|c| is_inline_atom(c))
            .collect();
        if atoms
            .iter()
            .enumerate()
            .any(|(i, x)| atoms[..i].iter().any(|y| y.same_ref(x)))
        {
            return None;
        }
        let mut changes = Vec::new();
        let mut child_pos = pos + 1;
        let mut k = 0;
        for i in 0..a.child_count() {
            let (x, y) = (a.child(i), b.child(i));
            if is_inline_atom(x) {
                if !is_inline_atom(y) || x.type_name() != y.type_name() || x.marks() != y.marks() {
                    return None;
                }
                let (Some(ox), Some(ny)) = (&inline.old[self.io + k], &inline.new[self.in_ + k])
                else {
                    if x == y {
                        k += 1;
                        child_pos += 1;
                        continue;
                    }
                    return None;
                };
                if ox != ny || self.shared.contains(ox) {
                    return None;
                }
                for (key, value) in y.attrs().iter() {
                    if x.attrs().get(key) != Some(value) {
                        changes.push((child_pos, key.to_string(), Some(value.clone())));
                    }
                }
                for (key, _) in x.attrs().iter() {
                    if y.attrs().get(key).is_none() {
                        changes.push((child_pos, key.to_string(), None));
                    }
                }
                k += 1;
            } else if x != y {
                return None;
            }
            child_pos += x.node_size();
        }
        (!changes.is_empty()).then_some(changes)
    }

    fn pair(&mut self, a: &Node, b: &Node, pos: usize) {
        let (ca, cb) = (block_atoms(a), block_atoms(b));
        let (ia, ib) = (inline_atoms(a), inline_atoms(b));
        if (a.same_ref(b) || a == b) && self.same_atoms(ca, cb) && self.same_inline(ia, ib) {
            self.ko += ca;
            self.kn += cb;
            self.io += ia;
            self.in_ += ib;
            return;
        }
        if let Some(changes) = self.inline_attr_changes(a, b, pos) {
            self.edits.push(Edit::Attrs(changes));
            self.io += ia;
            self.in_ += ib;
            return;
        }
        let same_shape = a.type_name() == b.type_name() && a.marks() == b.marks();
        if same_shape && is_block_atom(a) && is_block_atom(b) && self.same_atoms(1, 1) {
            let mut changes = Vec::new();
            for (key, value) in b.attrs().iter() {
                if a.attrs().get(key) != Some(value) {
                    changes.push((pos, key.to_string(), Some(value.clone())));
                }
            }
            for (key, _) in a.attrs().iter() {
                if b.attrs().get(key).is_none() {
                    changes.push((pos, key.to_string(), None));
                }
            }
            self.edits.push(Edit::Attrs(changes));
        } else if same_shape
            && !a.is_leaf()
            && !a.is_textblock()
            && !b.is_textblock()
            && a.attrs() == b.attrs()
            && a.child_count() == b.child_count()
        {
            let mut child_pos = pos + 1;
            for i in 0..a.child_count() {
                let ca = a.child(i);
                self.pair(ca, b.child(i), child_pos);
                child_pos += ca.node_size();
            }
            return; // the children advanced the counts
        } else {
            self.edits.push(Edit::Replace {
                from: pos,
                to: pos + a.node_size(),
                node: b.clone(),
            });
        }
        self.ko += ca;
        self.kn += cb;
        self.io += ia;
        self.in_ += ib;
    }
}

/// Build the transaction that brings `state`'s model up to the converged CRDT
/// document `target`, as a minimal block-level replace. Returns `None` if nothing
/// changed. The transaction is tagged remote + non-undoable.
pub fn build_remote_transaction(state: &EditorState, target: &Node) -> Result<Option<Transaction>> {
    build_remote_transaction_with(state, target, None)
}

/// [`build_remote_transaction`], told the CRDT block of every leaf block atom on
/// both sides ([`AtomIds`]). Then:
///
/// * an atom that is **another CRDT block** than the one at its place is a change
///   even when the two are equal by value (two equal embeds a peer swapped), and is
///   replaced, so a node view drawn for the old one is remounted — never handed
///   the other node (decision (d) of the node views);
/// * an atom that is the **same block** with other attributes gets
///   `SetNodeAttrStep`s, so it keeps its identity in the model and a view stays
///   mounted — beside other changes in the same update too (a reconnect's batch);
/// * every other changed block is replaced on its own.
///
/// Without `ids` it is the one block-level replace of the changed middle.
pub(crate) fn build_remote_transaction_with(
    state: &EditorState,
    target: &Node,
    ids: Option<&AtomIds>,
) -> Result<Option<Transaction>> {
    let old = &state.doc;
    let ids = ids.filter(|ids| ids.fits(old, target));
    let Some(diff) = block_diff_with(old, target, ids) else {
        return Ok(None);
    };
    let (prefix, suffix, start) = (diff.prefix, diff.suffix, diff.start);
    let edits = ids.and_then(|ids| changed_middle(old, target, &diff, AtomMatch::Ids(ids)));

    // Only atoms' attributes changed: say so, attribute by attribute, and leave
    // the selection where it is — no position moves.
    if let Some(edits) = &edits
        && edits.iter().all(|e| matches!(e, Edit::Attrs(_)))
    {
        let mut tr = state.tr();
        for edit in edits {
            if let Edit::Attrs(changes) = edit {
                set_attrs(&mut tr, changes)?;
            }
        }
        tr.set_add_to_history(false);
        tr.set_meta(ORIGIN_REMOTE, true);
        return Ok(Some(tr));
    }

    // Where the selection's ends belong afterwards, worked out against the old
    // document before it is replaced (see `carried_position`). The result of the
    // edits below is the same document, position for position, as one replace of
    // the changed middle, so the same answer holds for either.
    let old_selection = &state.selection;
    let carried = (
        carried_position(old, target, prefix, suffix, start, old_selection.anchor().0),
        carried_position(old, target, prefix, suffix, start, old_selection.head().0),
    );

    let mut tr = match edits {
        Some(edits) => {
            // Back to front, so every edit's old position is still its position.
            let mut tr = state.tr();
            for edit in edits.iter().rev() {
                match edit {
                    Edit::Replace { from, to, node } => {
                        tr.replace(
                            *from,
                            *to,
                            Slice::new(Fragment::from_node(node.clone()), 0, 0),
                        )?;
                    }
                    Edit::Attrs(changes) => set_attrs(&mut tr, changes)?,
                }
            }
            tr
        }
        None => replace_changed_blocks(state, target, &diff)?,
    };
    let new_doc = tr.doc().clone();
    match carried {
        // Both ends sat in changed textblocks that are still there: put them back
        // where they were in the text.
        (Some(anchor), Some(head)) if anchor == head => {
            tr.set_selection(Selection::near(&new_doc, Pos(head), 1));
        }
        (Some(anchor), Some(head)) => {
            tr.set_selection(Selection::text(Pos(anchor), Pos(head)));
        }
        // Otherwise re-anchor to a safe spot from where the mapping left the head
        // (`near` guarantees a valid text position even if the old block is gone).
        _ => {
            let head = tr.selection().head();
            tr.set_selection(Selection::near(&new_doc, head, 1));
        }
    }
    tr.set_add_to_history(false);
    tr.set_meta(ORIGIN_REMOTE, true);
    Ok(Some(tr))
}

/// One `SetNodeAttrStep` per change.
fn set_attrs(tr: &mut Transaction, changes: &[AttrChange]) -> Result<()> {
    for (pos, attr, value) in changes {
        tr.step(Box::new(SetNodeAttrStep {
            pos: *pos,
            attr: attr.clone(),
            value: value.clone(),
        }))
        .map_err(|e| crate::error::CollabError::Schema(e.to_string()))?;
    }
    Ok(())
}

/// Carries selections **other than the live one** across a remote change: a
/// selection captured for work that finishes later (an editor's
/// `SelectionAnchor`: an asynchronous paste, a picture being uploaded) must
/// still name the content it was taken at after a peer's edit arrives.
///
/// The live selection is carried by [`build_remote_transaction`] itself; this
/// is the same rule for any other selection of the same old document, with one
/// difference. The live selection must always land somewhere, so an end the
/// change cannot be followed through is re-anchored to the nearest valid
/// place. A captured selection is a promise about *content*, so there the
/// answer is `None`: the block it was in was deleted, split, joined or changed
/// kind by the peer, and inserting "nearby" would be a guess.
pub struct RemoteCarry {
    /// Only atoms' attributes changed ([`changed_middle`]): no position
    /// moved, and every selection stands as it is.
    attrs_only: bool,
    old: Node,
    new_doc: Node,
    diff: BlockDiff,
    mapping: rinch_editor_core::Mapping,
}

impl RemoteCarry {
    /// Prepare to carry selections of `state`'s document over to `target`, the
    /// converged document. `None` when the two are the same document (every
    /// selection stands as it is) or the replace cannot be built.
    pub fn new(state: &EditorState, target: &Node) -> Option<RemoteCarry> {
        let diff = block_diff(&state.doc, target)?;
        // Positional is enough here: whichever atom stands where, no position
        // moved.
        let attrs_only = changed_middle(&state.doc, target, &diff, AtomMatch::Positional)
            .is_some_and(|edits| edits.iter().all(|e| matches!(e, Edit::Attrs(_))));
        let tr = replace_changed_blocks(state, target, &diff).ok()?;
        Some(RemoteCarry {
            attrs_only,
            old: state.doc.clone(),
            new_doc: tr.doc().clone(),
            mapping: tr.mapping().clone(),
            diff,
        })
    }

    /// Where `pos` of the old document is now: unmoved or shifted when it lies
    /// outside the changed blocks, followed through the text when it lies in
    /// one that is still there, `None` when it lay in one that is not.
    fn position(&self, pos: usize) -> Option<usize> {
        let d = &self.diff;
        if pos > d.start && pos < d.end {
            return carried_position(&self.old, &self.new_doc, d.prefix, d.suffix, d.start, pos);
        }
        Some(self.mapping.map(pos, 1))
    }

    /// `selection`, taken in the old document, in the converged one; `None`
    /// when the content it named is gone.
    pub fn selection(&self, selection: &Selection) -> Option<Selection> {
        if self.attrs_only {
            return Some(selection.clone());
        }
        let d = &self.diff;
        let inside = |pos: usize| pos > d.start && pos < d.end;
        let (anchor, head) = (selection.anchor().0, selection.head().0);
        if !inside(anchor) && !inside(head) {
            // Wholly outside the change: the block-level step map is exact.
            return Some(selection.map(&self.new_doc, &self.mapping));
        }
        let (anchor, head) = (self.position(anchor)?, self.position(head)?);
        let in_text = |pos: usize| {
            self.new_doc
                .resolve(Pos(pos))
                .is_ok_and(|r| r.parent().is_textblock())
        };
        match selection {
            Selection::Text(_) if in_text(anchor) && in_text(head) => {
                Some(Selection::text(Pos(anchor), Pos(head)))
            }
            Selection::Text(_) => None,
            // A selected node (an image in a paragraph a peer typed in) is
            // still selected if a node is still there.
            Selection::Node(_) => Selection::node_at(&self.new_doc, Pos(anchor)),
            // A cell selection inside a table the peer changed: not followed.
            Selection::Cell(_) => None,
        }
    }
}

/// Where model position `pos` of the old document belongs in the new one, when
/// it sits inside one of the changed blocks; `None` when it does not, or when
/// the change is not one this can follow, and the step mapping decides.
///
/// A remote change arrives as a block-level replace, and a position inside a
/// replaced range maps to the *end* of the replacement. For a caret in the very
/// paragraph a peer is typing in, that is the start of the next paragraph: every
/// keystroke of theirs threw it out of the line it was on, so two people could
/// not write in one paragraph. The text says where the caret belongs. With the
/// same number of changed blocks before and after, each old block corresponds
/// to the new block at the same index; inside a textblock the caret keeps its
/// place relative to the text the two versions share (before the change:
/// unmoved; after it: shifted by the difference in length; inside it: just
/// after what replaced it).
fn carried_position(
    old: &Node,
    target: &Node,
    prefix: usize,
    suffix: usize,
    start: usize,
    pos: usize,
) -> Option<usize> {
    let old_changed = old.child_count() - prefix - suffix;
    let new_changed = target.child_count() - prefix - suffix;
    if old_changed == 0 || old_changed != new_changed {
        return None; // blocks were added, removed, split or joined
    }
    // Both walks start at `start`: the blocks before it are identical.
    let (mut old_at, mut new_at) = (start, start);
    for index in prefix..prefix + old_changed {
        let (old_block, new_block) = (old.child(index), target.child(index));
        // Content positions of a block at `at`: `at + 1 ..= at + 1 + content_size`.
        let inside = pos > old_at && pos < old_at + old_block.node_size();
        if inside {
            return carried_into(old_block, new_block, old_at, new_at, pos);
        }
        old_at += old_block.node_size();
        new_at += new_block.node_size();
    }
    None
}

/// Where `pos`, inside `old_block` (which starts at `old_at`), belongs inside
/// `new_block` (which starts at `new_at`), the block that replaced it at the same
/// index.
///
/// Two textblocks: by the text they share ([`carried_offset`]). Two containers of the
/// same type (a quote, a list, a list item, a table, a row, a cell) whose content a
/// peer changed: recurse into
/// their children ([`carried_in_children`]), so a caret in a paragraph inside a quote
/// keeps its place when a peer types in that paragraph — the remote change replaces
/// the whole top-level quote, and the step mapping alone would carry the caret to its
/// end. Anything else (a block that changed kind) is `None`.
fn carried_into(
    old_block: &Node,
    new_block: &Node,
    old_at: usize,
    new_at: usize,
    pos: usize,
) -> Option<usize> {
    if old_block.is_textblock() && new_block.is_textblock() {
        let (old_units, new_units) = (flat_units(old_block)?, flat_units(new_block)?);
        let offset = carried_offset(&old_units, &new_units, pos - old_at - 1);
        return Some(new_at + 1 + offset);
    }
    // A **block atom** (`horizontal_rule`) is where the two can differ in kind: a peer
    // replacing a paragraph with a scene break, or a scene break with a paragraph,
    // and there is no text offset to carry across. `None` hands the caret to the step
    // mapping, which re-anchors it from the block-level replace the caller already
    // emits — an atom is never spliced as text. (An atom on the *old* side cannot even
    // reach here: a leaf is one position wide, so nothing sits `inside` it.)
    if old_block.is_textblock()
        || new_block.is_textblock()
        || old_block.is_leaf()
        || new_block.is_leaf()
        || old_block.type_name() != new_block.type_name()
    {
        return None;
    }
    carried_in_children(old_block, new_block, old_at + 1, new_at + 1, pos)
}

/// `pos`, inside the content of `old` (which begins at `old_start`), carried into the
/// content of `new` (which begins at `new_start`). The children the two share at
/// their start and at their end are the same nodes, so a position in one of those
/// keeps its offset from that end; a position in a changed child is followed into the
/// child that replaced it when the changed runs have the same length, as at the top
/// level ([`carried_position`]). `None` when the position sits between children or the
/// change is not one this can follow.
fn carried_in_children(
    old: &Node,
    new: &Node,
    old_start: usize,
    new_start: usize,
    pos: usize,
) -> Option<usize> {
    let (on, nn) = (old.child_count(), new.child_count());
    let mut prefix = 0;
    while prefix < on && prefix < nn && old.child(prefix) == new.child(prefix) {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < on - prefix
        && suffix < nn - prefix
        && old.child(on - 1 - suffix) == new.child(nn - 1 - suffix)
    {
        suffix += 1;
    }
    let mut old_at = old_start;
    let mut new_at = new_start;
    // The children before the change are identical, so their positions are too.
    for index in 0..prefix {
        let size = old.child(index).node_size();
        if pos > old_at && pos < old_at + size {
            return Some(new_at + (pos - old_at));
        }
        old_at += size;
        new_at += size;
    }
    let old_end = old_start + old.content_size();
    let new_end = new_start + new.content_size();
    // The children after it are identical too, measured from the end.
    let suffix_start = old_end
        - (on - suffix..on)
            .map(|i| old.child(i).node_size())
            .sum::<usize>();
    if pos > suffix_start && pos < old_end {
        return Some(new_end - (old_end - pos));
    }
    let (old_changed, new_changed) = (on - prefix - suffix, nn - prefix - suffix);
    if old_changed == 0 || old_changed != new_changed {
        return None;
    }
    for index in prefix..prefix + old_changed {
        let (old_child, new_child) = (old.child(index), new.child(index));
        if pos > old_at && pos < old_at + old_child.node_size() {
            return carried_into(old_child, new_child, old_at, new_at, pos);
        }
        old_at += old_child.node_size();
        new_at += new_child.node_size();
    }
    None
}

/// A textblock's inline content, one entry per model position: a text node's
/// characters, and a placeholder for an inline leaf (which is one position
/// wide). `None` for inline content with positions inside it.
fn flat_units(block: &Node) -> Option<Vec<char>> {
    let mut units = Vec::with_capacity(block.content_size());
    for i in 0..block.child_count() {
        let child = block.child(i);
        match child.text() {
            Some(text) => units.extend(text.chars()),
            None if child.node_size() == 1 => units.push('\u{FFFC}'),
            None => return None,
        }
    }
    Some(units)
}

/// `offset` into `old` carried over to `new`, by the text the two share at
/// their start and at their end.
fn carried_offset(old: &[char], new: &[char], offset: usize) -> usize {
    let shortest = old.len().min(new.len());
    let mut shared_start = 0;
    while shared_start < shortest && old[shared_start] == new[shared_start] {
        shared_start += 1;
    }
    let mut shared_end = 0;
    while shared_end < shortest - shared_start
        && old[old.len() - 1 - shared_end] == new[new.len() - 1 - shared_end]
    {
        shared_end += 1;
    }
    if offset <= shared_start {
        offset
    } else if offset >= old.len() - shared_end {
        new.len() - (old.len() - offset)
    } else {
        new.len() - shared_end
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rinch_editor_core::{Pos, Schema};
    use std::rc::Rc;

    fn doc(schema: &Rc<Schema>, paragraphs: &[&str]) -> Node {
        let blocks: Vec<Node> = paragraphs
            .iter()
            .map(|text| {
                let content = if text.is_empty() {
                    Fragment::empty()
                } else {
                    Fragment::from_node(schema.text(text).unwrap())
                };
                schema.branch("paragraph", content).unwrap()
            })
            .collect();
        schema
            .branch("doc", Fragment::from_children(blocks))
            .unwrap()
    }

    fn state_with_cursor(schema: &Rc<Schema>, paragraphs: &[&str], cursor: usize) -> EditorState {
        let state = EditorState::create(schema.clone(), doc(schema, paragraphs), vec![]);
        let mut tr = state.tr();
        tr.set_selection(Selection::cursor(Pos(cursor)));
        state.apply(tr)
    }

    fn head_after(state: &EditorState, target: &Node) -> usize {
        let tr = build_remote_transaction(state, target)
            .unwrap()
            .expect("the document changed");
        state.apply(tr).selection.head().0
    }

    /// A peer backspaces at the end of the line this caret is at the end of: the
    /// caret stays at the end of that line. It used to be carried to the end of the
    /// replaced range and re-anchored from there, which is the start of the *next*
    /// paragraph whenever there is one.
    #[test]
    fn a_peer_deleting_before_the_caret_in_its_own_paragraph_keeps_it_there() {
        let schema = Rc::new(Schema::starter_kit());
        // "Once in a while" is 15 long: content 1..=16, the caret at its end.
        let state = state_with_cursor(&schema, &["Once in a while", "next"], 16);
        let target = doc(&schema, &["Once in a whil", "next"]);
        assert_eq!(head_after(&state, &target), 15);
    }

    /// A peer types earlier in the same paragraph: the caret keeps its place in the
    /// text, which is one further along.
    #[test]
    fn a_peer_typing_before_the_caret_in_its_own_paragraph_shifts_it() {
        let schema = Rc::new(Schema::starter_kit());
        let state = state_with_cursor(&schema, &["hello world", "next"], 7); // before "world"
        let target = doc(&schema, &["oh hello world", "next"]);
        assert_eq!(head_after(&state, &target), 10);
    }

    /// A peer types after the caret in the same paragraph: the caret does not move.
    #[test]
    fn a_peer_typing_after_the_caret_in_its_own_paragraph_leaves_it() {
        let schema = Rc::new(Schema::starter_kit());
        let state = state_with_cursor(&schema, &["hello world", "next"], 3);
        let target = doc(&schema, &["hello world!", "next"]);
        assert_eq!(head_after(&state, &target), 3);
    }

    /// The peer rewrites the very text the caret is inside: it lands just after
    /// what replaced that text.
    #[test]
    fn a_caret_inside_the_text_a_peer_replaced_lands_after_the_replacement() {
        let schema = Rc::new(Schema::starter_kit());
        let state = state_with_cursor(&schema, &["one two three", "next"], 7); // "one tw|o three"
        let target = doc(&schema, &["one 2 three", "next"]);
        assert_eq!(head_after(&state, &target), 6); // "one 2| three"
    }

    /// A range selection keeps both ends.
    #[test]
    fn a_selection_in_the_edited_paragraph_keeps_both_ends() {
        let schema = Rc::new(Schema::starter_kit());
        let state = EditorState::create(
            schema.clone(),
            doc(&schema, &["hello world", "next"]),
            vec![],
        );
        let mut tr = state.tr();
        tr.set_selection(Selection::text(Pos(7), Pos(12))); // "world"
        let state = state.apply(tr);
        let target = doc(&schema, &["oh hello world", "next"]);
        let tr = build_remote_transaction(&state, &target).unwrap().unwrap();
        let after = state.apply(tr);
        assert_eq!(
            (after.selection.anchor().0, after.selection.head().0),
            (10, 15)
        );
    }

    /// The peer splits the caret's paragraph: not a change this follows, and the
    /// caret still ends up somewhere valid.
    #[test]
    fn a_split_paragraph_still_leaves_a_valid_caret() {
        let schema = Rc::new(Schema::starter_kit());
        let state = state_with_cursor(&schema, &["hello world"], 3);
        let target = doc(&schema, &["hello", " world"]);
        let head = head_after(&state, &target);
        assert!(head >= 1 && head <= target.content_size());
    }

    /// A caret in a paragraph the peer did not touch is carried by the mapping, as
    /// before.
    #[test]
    fn a_caret_in_an_untouched_later_paragraph_shifts_with_the_document() {
        let schema = Rc::new(Schema::starter_kit());
        let state = state_with_cursor(&schema, &["hello", "next"], 9); // "n|ext"
        let target = doc(&schema, &["hello there", "next"]);
        assert_eq!(head_after(&state, &target), 15);
    }

    /// A peer removes a whole block. The old and new changed ranges then no longer
    /// correspond index for index, so the walk that follows them in step must not
    /// start: `Fragment::child` indexes a `Vec`, and reading past the end of the
    /// shorter range is a panic — which on wasm takes the page down. The
    /// block-count guard is the whole of what stands in front of that, and this is
    /// its only pin; `a_split_paragraph_still_leaves_a_valid_caret` covers the
    /// other direction, where there are *more* new blocks and no walk runs off
    /// anything.
    #[test]
    fn a_peer_deleting_a_whole_paragraph_is_left_to_the_mapping() {
        let schema = Rc::new(Schema::starter_kit());
        let state = state_with_cursor(&schema, &["keep", "doomed"], 3); // "ke|ep"
        let target = doc(&schema, &["keep"]);
        assert_eq!(head_after(&state, &target), 3);
    }

    /// Two blocks change in one delta and the caret is in the **second** of them.
    /// Its new home is measured against what the earlier changed blocks have
    /// *become*, not against what they were — every other fixture puts the caret in
    /// the first changed block, where the two are the same number and a walk that
    /// accumulated the old sizes would still answer correctly.
    #[test]
    fn a_caret_in_a_later_changed_block_is_measured_against_the_new_earlier_block() {
        let schema = Rc::new(Schema::starter_kit());
        // "alpha" is 5 long, so its block is 7 wide and "beta" content is 8..=12;
        // the caret is at the end of "beta".
        let state = state_with_cursor(&schema, &["alpha", "beta", "tail"], 12);
        let target = doc(&schema, &["alpha rewritten", "beta!", "tail"]);
        // "alpha rewritten" is 15 long, so its block is 17 wide, and the caret keeps
        // its place at the end of "beta": 17 + 1 + 4.
        assert_eq!(head_after(&state, &target), 22);
    }

    /// A peer types before the caret in a line that holds an **inline atom**, which
    /// the caret has to be measured *across*. `flat_units` gives the atom one unit —
    /// the same single model position it occupies, and the same U+FFFC the collab
    /// projection stands it up with — so the offsets on either side of it still line
    /// up. Counting it as zero (or as its rendered width) would slide the caret by one
    /// per picture on every keystroke a peer makes.
    #[test]
    fn a_caret_after_an_inline_atom_is_carried_across_it() {
        let schema = Rc::new(Schema::starter_kit());
        let line = |lead: &str| {
            let image = schema
                .create_node(
                    "image",
                    rinch_editor_core::Attrs::new()
                        .with("src", rinch_editor_core::AttrValue::from("cat.png")),
                    Fragment::empty(),
                )
                .unwrap();
            let para = schema
                .branch(
                    "paragraph",
                    Fragment::from_children(vec![
                        schema.text(lead).unwrap(),
                        image,
                        schema.text("tail").unwrap(),
                    ]),
                )
                .unwrap();
            schema.branch("doc", Fragment::from_node(para)).unwrap()
        };
        // doc(paragraph("ab", image, "tail")): content 1..=8, the caret after "ta".
        let state = EditorState::create(schema.clone(), line("ab"), vec![]);
        let mut tr = state.tr();
        tr.set_selection(Selection::cursor(Pos(6)));
        let state = state.apply(tr);
        // The peer types one char before the atom: everything after it shifts by one.
        assert_eq!(head_after(&state, &line("abX")), 7);
    }

    /// The caret sits exactly where the two versions stop agreeing. It belongs to
    /// the text before it, which both versions share, so it does not move. Reading
    /// that boundary as *inside* the change instead would carry it to the end of
    /// whatever replaced the rest of the line — a caret in the middle of a line
    /// jumping to its end on a peer's keystroke.
    #[test]
    fn a_caret_at_the_point_the_two_versions_diverge_stays_put() {
        let schema = Rc::new(Schema::starter_kit());
        let state = state_with_cursor(&schema, &["hello world"], 7); // "hello |world"
        let target = doc(&schema, &["hello there"]);
        assert_eq!(head_after(&state, &target), 7);
    }
}
