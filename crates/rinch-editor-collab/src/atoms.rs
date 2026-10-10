//! An inline atom's attributes, merged **per attribute**.
//!
//! An inline atom (`image`, `hard_break`) is one U+FFFC char in its block's `Text`
//! carrying the reserved formatting attribute `@atom` (see [`crate::projection`]). That
//! value is the atom's type and attrs **as they were when the char was written** (or
//! moved, below), and its attrs are never rewritten to change one: a change of one attr
//! is one entry of the root map [`ATOMS`] instead (the first one also adds `@id` to the
//! value, see *What a build from before reads*):
//!
//! ```text
//! root Map "atoms"
//!   "<identity>/<attr>" -> Any    // the attr's value; `Undefined` = the attr removed
//! ```
//!
//! An atom reads as its `@atom` value with every entry under its identity laid over
//! it. yrs merges a map per key, so two peers changing **different** attrs of one atom
//! at once (one its `alt`, the other an app's `data-*` attribute) write different keys and both
//! survive; two peers changing the **same** attr write one key, and the replicas
//! converge on one of the two values (last writer wins, yrs's map rule). Before this,
//! `@atom` was the only place the attrs lived, a change rewrote the whole value, and
//! yrs kept one of two concurrent rewrites: one peer's image, whole.
//!
//! The entries are flat keys under one root map, never a map per atom: two peers making
//! the *first* change to one atom at once would each create that atom's map, and yrs
//! keeps one of two maps written under one key, so the other peer's change would be lost
//! exactly as before.
//!
//! ## Identity
//!
//! An atom's **identity** is the yrs id of its char (`"<client>:<clock>"`), which every
//! replica reads the same and which needs nothing written: an atom written before this
//! module existed has one too, so its attrs merge per attribute from its first change on.
//!
//! An identity must stay on **its** atom: an app keys its own data to a picture through
//! an attr of it (a `data-*` attribute), and that shown on another picture is worse
//! than lost.
//! Two things decide which char is which atom, and both ask the model, not the text:
//!
//! * **The text diff** (`projection::text_splice_bounds`) matches an atom's placeholder
//!   only with the same atom: the editor keeps the `Rc` of a node an edit does not
//!   rebuild, so an atom `Node::same_ref` to one before the change is that atom. Every
//!   atom is the same U+FFFC, so a diff by chars alone took a picture inserted right
//!   before another for that other (review of #1503, F2). An atom changed in place (a new
//!   node) is the atom at its place when neither is in the change otherwise and both
//!   have one type and one `src`: a `src` change makes a new atom, so a picture pasted
//!   over a selected one (which the model cannot tell from a `src` change) never takes
//!   the old one's identity. Chosen by Joe (2026-10-09): a wrong attribution is worse
//!   than a lost one.
//! * **A move** writes a new char, since yrs has no move: Enter before an image in its
//!   line deletes the image's char and inserts one in the new block, and Backspace
//!   joining the line back does the same. [`carry_over`] hands the new char the old
//!   one's identity, in the `@atom` value's reserved key [`ATOM_ID`], when the model says
//!   it is the same atom (`same_ref`). A peer's concurrent change of the atom's attrs is
//!   an entry under that identity, so it applies to the moved atom as well. When the
//!   model does not say so plainly, nothing is carried and that change is lost.
//!
//! **Leaked values.** yrs extends a formatted range over a peer's concurrent insert at
//! its edge, so an `@atom` value written over a char can show on a char another peer
//! inserts beside it at the same moment. The value names the char it was written for
//! ([`ATOM_FOR`]) beside any [`ATOM_ID`], and an identity is honoured only on that
//! char; and no value written over an existing char holds changed attrs (those are
//! entries). So a leaked copy reads as the attrs the atom was written with, under the
//! char's own identity: never another atom's changes (review 2 of #1503, R2-1).

//! ## What a build from before reads
//!
//! An atom only ever inserted (and typed around, deleted, marked) is written exactly as
//! before: one `@atom` value, no `@id`, nothing in [`ATOMS`]. A per-attribute change
//! also marks the atom's char with the reserved formatting attribute [`ATOM_STAMP`],
//! and a move writes `@id`: a build from before refuses either (`unknown mark type
//! @entries`, `unknown reserved key @id`), which poisons its session until it rejoins
//! on an upgraded build (#196). So it never shows attrs it would get wrong. Once
//! refused, a document stays refused for such a build (an `@entries` mark can extend
//! over text beside the picture and is not cleared).

//! ## Growth
//!
//! An entry is never removed: one per attr ever changed per atom (an overwritten value
//! is collected by yrs). An atom deleted keeps its entries in the map; each costs a few
//! dozen bytes (measured: the first app attribute of a picture, entry and `@entries` mark,
//! +62 bytes; 200 further `alt` changes of it +38 bytes in all; deleting it 0) and one
//! read per operation that reads the map ([`overlay_scope`]).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use yrs::types::Attrs as YAttrs;
use yrs::types::text::YChange;
use yrs::{
    Any, ArrayRef, Assoc, IndexedSequence, Map, Out, ReadTxn, Text, TextRef, TransactionMut,
    WriteTxn,
};

use rinch_editor_core::{AttrValue, Attrs, Node};

use crate::error::{CollabError, Result};
use crate::projection::{
    ATOM_MARK, ATOM_PLACEHOLDER, ATOM_TYPE, CONTENT, RESERVED_PREFIX, attr_to_any,
    decode_attr_value, decode_mark_value, encode_mark_value,
};

/// The root map holding every atom attribute written after its atom's char.
pub(crate) const ATOMS: &str = "atoms";

/// A reserved formatting attribute (value `true`) over the char of an atom that has
/// [`ATOMS`] entries. This build reads nothing from it ([`scan_text`] skips it); it is
/// there so a build from before per-attribute merging, which reads every formatting
/// attribute as a mark, refuses the document loudly (`unknown mark type`) rather than
/// show the attrs the char was written with and miss the entries (review of #1503,
/// F5). A separate attribute, not a key in the `@atom` value: yrs can extend a
/// formatted range over a peer's concurrent insert at its edge, and a value naming an
/// identity that leaked onto a neighbouring picture would show that picture as this
/// one (measured by the differential, seed 208). A leaked `@entries` is harmless.
pub(crate) const ATOM_STAMP: &str = "@entries";

/// The reserved key in an `@atom` value naming the identity the char carries over from
/// the atom it replaced (a move), or a fresh one (an atom edited apart from a copy).
pub(crate) const ATOM_ID: &str = "@id";

/// The reserved key beside [`ATOM_ID`] naming the char (`"<client>:<clock>"`) the value
/// was written for. yrs extends a formatted range over a peer's concurrent insert at
/// its edge, so a value can leak onto a neighbouring char; an [`ATOM_ID`] is honoured
/// only on the char this names, and a leaked copy is inert (review 2 of #1503, R2-1:
/// an app attribute showed on a picture inserted beside a moved one).
pub(crate) const ATOM_FOR: &str = "@for";

/// One char of a block's text carrying an `@atom` value, as written.
#[derive(Debug, Clone)]
pub(crate) struct AtomChar {
    /// Char offset in the block's text.
    pub index: usize,
    /// UTF-16 offset in the block's text.
    pub u16: u32,
    /// Whether the char is the placeholder (an `@atom` over any other char is a stray
    /// the next local edit clears; it is never an atom).
    pub placeholder: bool,
    /// The `@atom` value without [`ATOM_ID`] and [`ATOM_FOR`]: the atom's type and
    /// attrs when the char was written.
    pub value: Attrs,
    /// The value's [`ATOM_ID`].
    pub carried: Option<String>,
    /// The value's [`ATOM_FOR`]: the char the [`ATOM_ID`] was written for.
    pub carried_for: Option<String>,
}

impl AtomChar {
    /// The atom's identity: its carried id when that was written for this char, or its
    /// char's yrs id. `None` only for a char yrs cannot locate, which a live char always
    /// is.
    pub fn identity<T: ReadTxn>(&self, txn: &T, text: &TextRef) -> Option<String> {
        let own = char_id(txn, text, self.u16)?;
        match (&self.carried, &self.carried_for) {
            (Some(id), Some(target)) if *target == own => Some(id.clone()),
            _ => Some(own),
        }
    }

    /// Whether this char's identity is a carried one (written for it), not its own id.
    fn carries<T: ReadTxn>(&self, txn: &T, text: &TextRef) -> bool {
        self.carried.is_some()
            && self.carried_for.is_some()
            && self.carried_for == char_id(txn, text, self.u16)
    }
}

/// The yrs id of the char at UTF-16 offset `u16`, as `"<client>:<clock>"`.
fn char_id<T: ReadTxn>(txn: &T, text: &TextRef, u16: u32) -> Option<String> {
    let sticky = text.sticky_index(txn, u16, Assoc::After)?;
    let id = sticky.id()?;
    Some(format!("{}:{}", id.client, id.clock))
}

/// A block's text read once: the string, the spans of every attribute but `@atom`, and
/// every char carrying `@atom`.
pub(crate) struct ScannedText {
    pub text: String,
    pub spans: Vec<(String, Attrs, usize, usize)>,
    pub atoms: Vec<AtomChar>,
}

/// Read a `Text` as [`ScannedText`]. Fails loud on an embedded value and on a
/// formatting value [`decode_mark_value`] refuses.
pub(crate) fn scan_text<T: ReadTxn>(txn: &T, text: &TextRef) -> Result<ScannedText> {
    TEXT_SCANS.with(|c| c.set(c.get() + 1));
    let mut s = String::new();
    let mut chars = 0usize;
    let mut u16 = 0u32;
    let mut spans = Vec::new();
    let mut atoms: Vec<AtomChar> = Vec::new();
    for chunk in text.diff(txn, YChange::identity) {
        let Out::Any(Any::String(part)) = &chunk.insert else {
            return Err(CollabError::unsupported(
                "an embedded value inside a block's text is not supported; an inline \
                 atom is projected as a placeholder char with an `@atom` attribute, \
                 never as a yrs embed",
            ));
        };
        let start = chars;
        let n = part.chars().count();
        let mut atom_value: Option<&Any> = None;
        if let Some(attrs) = &chunk.attributes {
            for (name, value) in attrs.iter() {
                // A cleared format can linger as an explicit null; it is not a mark.
                if matches!(value, Any::Null | Any::Undefined) || name.as_ref() == ATOM_STAMP {
                    continue;
                }
                if name.as_ref() == ATOM_MARK {
                    atom_value = Some(value);
                } else {
                    spans.push((
                        name.to_string(),
                        decode_mark_value(value)?,
                        start,
                        start + n,
                    ));
                }
            }
        }
        if let Some(value) = atom_value {
            let decoded = decode_mark_value(value)?;
            let carried = match decoded.get(ATOM_ID) {
                None => None,
                Some(AttrValue::Str(id)) => Some(id.to_string()),
                Some(other) => {
                    return Err(CollabError::schema(format!(
                        "an inline atom's `{ATOM_ID}` must be a string, got {other:?}"
                    )));
                }
            };
            let carried_for = match decoded.get(ATOM_FOR) {
                Some(AttrValue::Str(id)) => Some(id.to_string()),
                _ => None,
            };
            let stripped = decoded.without(ATOM_ID).without(ATOM_FOR);
            let mut at = u16;
            for (k, c) in part.chars().enumerate() {
                atoms.push(AtomChar {
                    index: start + k,
                    u16: at,
                    placeholder: c == ATOM_PLACEHOLDER,
                    value: stripped.clone(),
                    carried: carried.clone(),
                    carried_for: carried_for.clone(),
                });
                at += c.len_utf16() as u32;
            }
        }
        s.push_str(part);
        chars += n;
        u16 += part.chars().map(|c| c.len_utf16() as u32).sum::<u32>();
    }
    Ok(ScannedText {
        text: s,
        spans,
        atoms,
    })
}

/// Every entry of the [`ATOMS`] map, grouped by identity.
pub(crate) type Overlay = HashMap<String, Vec<(String, Any)>>;

thread_local! {
    /// While an [`overlay_scope`] is held: the [`ATOMS`] map read once for the whole
    /// scope (`Some(None)` until something first needs it). `None` outside a scope.
    static OVERLAY: RefCell<Option<Option<Arc<Overlay>>>> = const { RefCell::new(None) };
    /// How many [`ATOMS`] entries this thread has read ([`read_overlay`]).
    static OVERLAY_ENTRY_READS: Cell<u64> = const { Cell::new(0) };
    /// How many block texts this thread has scanned ([`scan_text`]), plus the blocks
    /// [`find_atoms`] visited.
    static TEXT_SCANS: Cell<u64> = const { Cell::new(0) };
}

/// How many block texts this thread has scanned, and blocks [`find_atoms`] visited, so
/// far (`testing::text_scans`).
#[cfg_attr(not(feature = "test-util"), allow(dead_code))]
pub(crate) fn text_scans() -> u64 {
    TEXT_SCANS.with(Cell::get)
}

/// How many [`ATOMS`] entries this thread has read so far (`testing::overlay_entry_reads`).
#[cfg_attr(not(feature = "test-util"), allow(dead_code))]
pub(crate) fn overlay_entry_reads() -> u64 {
    OVERLAY_ENTRY_READS.with(Cell::get)
}

/// Every entry of the [`ATOMS`] map, grouped by identity. Empty when the map does not
/// exist or holds nothing. Reads the whole map: callers go through [`with_overlay`],
/// which reads it once per [`overlay_scope`].
fn read_overlay<T: ReadTxn>(txn: &T) -> Overlay {
    let mut out: Overlay = HashMap::new();
    let Some(map) = txn.get_map(ATOMS) else {
        return out;
    };
    let mut n = 0u64;
    for (key, value) in map.iter(txn) {
        n += 1;
        let (Some((identity, attr)), Out::Any(any)) = (key.split_once('/'), value) else {
            continue;
        };
        out.entry(identity.to_string())
            .or_default()
            .push((attr.to_string(), any));
    }
    OVERLAY_ENTRY_READS.with(|c| c.set(c.get() + n));
    out
}

/// A `CollabDoc`'s read of the [`ATOMS`] map, kept between operations and dropped
/// whenever the map changes (an observer on it, local and remote writes alike).
pub(crate) type OverlayCache = Arc<Mutex<Option<Arc<Overlay>>>>;

/// While the returned guard lives, the [`ATOMS`] map is read at most once on this
/// thread, and the per-attribute writes made meanwhile ([`write_atom_change`]) are laid
/// into that read. Held by the operations that read many blocks (`CollabDoc::to_doc`,
/// `CollabDoc::project_change`): without it every block holding an atom read the whole
/// map, which made a remote integrate quadratic in edited atoms (review of #1503, F4).
/// The scope starts from `cache` (the doc's read, when the map has not changed since)
/// and leaves its read there, so a keystroke reads no entry at all unless the map
/// changed. Nested scopes share the outermost one.
///
/// A scope must not outlive a change it did not see: one is held only for the length
/// of one call on one `CollabDoc`, during which nothing else writes to it.
pub(crate) fn overlay_scope(cache: &OverlayCache) -> OverlayScope {
    let outer = OVERLAY.with(|c| {
        let mut c = c.borrow_mut();
        if c.is_some() {
            false
        } else {
            *c = Some(lock_cache(cache).clone());
            true
        }
    });
    OverlayScope {
        cache: outer.then(|| cache.clone()),
    }
}

fn lock_cache(cache: &OverlayCache) -> std::sync::MutexGuard<'_, Option<Arc<Overlay>>> {
    cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The guard [`overlay_scope`] returns.
pub(crate) struct OverlayScope {
    /// The doc's cache, for the outermost scope.
    cache: Option<OverlayCache>,
}

impl Drop for OverlayScope {
    fn drop(&mut self) {
        if let Some(cache) = &self.cache {
            let read = OVERLAY.with(|c| c.borrow_mut().take()).flatten();
            if read.is_some() {
                *lock_cache(cache) = read;
            }
        }
    }
}

/// Drop `cache`: the [`ATOMS`] map changed (called by its observer).
pub(crate) fn invalidate(cache: &OverlayCache) {
    *lock_cache(cache) = None;
}

/// Run `f` over the [`ATOMS`] map: the scope's read when an [`overlay_scope`] is held
/// (made now if this is the first need), a fresh read otherwise. `f` must not reach
/// [`with_overlay`] again.
fn with_overlay<T: ReadTxn, R>(txn: &T, f: impl FnOnce(&Overlay) -> R) -> R {
    let scoped = OVERLAY.with(|c| c.borrow().is_some());
    if !scoped {
        return f(&read_overlay(txn));
    }
    let loaded = OVERLAY.with(|c| matches!(&*c.borrow(), Some(Some(_))));
    if !loaded {
        let read = Arc::new(read_overlay(txn));
        OVERLAY.with(|c| *c.borrow_mut() = Some(Some(read)));
    }
    let overlay = OVERLAY.with(|c| match &*c.borrow() {
        Some(Some(overlay)) => overlay.clone(),
        _ => unreachable!("loaded above"),
    });
    f(&overlay)
}

/// Lay one entry just written into the scope's read, if the scope has read the map.
fn note_entry(identity: &str, attr: &str, value: &Any) {
    OVERLAY.with(|c| {
        if let Some(Some(overlay)) = &mut *c.borrow_mut() {
            let entries = Arc::make_mut(overlay)
                .entry(identity.to_string())
                .or_default();
            match entries.iter_mut().find(|(k, _)| k == attr) {
                Some(slot) => slot.1 = value.clone(),
                None => entries.push((attr.to_string(), value.clone())),
            }
        }
    });
}

/// An atom's value with the entries of its identity laid over it. An entry for a
/// reserved key is ignored (the type is never changed this way: a retype writes a
/// fresh identity); `Undefined` removes the attr.
pub(crate) fn merged(value: &Attrs, entries: Option<&Vec<(String, Any)>>) -> Result<Attrs> {
    let Some(entries) = entries.filter(|e| !e.is_empty()) else {
        return Ok(value.clone());
    };
    // One map for every entry (not `Attrs::with` per entry, which copies the
    // map each time: quadratic in the attribute count).
    let mut out: std::collections::BTreeMap<Box<str>, AttrValue> = value
        .iter()
        .map(|(k, v)| (Box::from(k), v.clone()))
        .collect();
    for (k, v) in entries {
        if k.starts_with(RESERVED_PREFIX) {
            continue;
        }
        match v {
            Any::Undefined => {
                out.remove(k.as_str());
            }
            other => {
                out.insert(k.as_str().into(), decode_attr_value(k, other)?);
            }
        }
    }
    Ok(Attrs::from_iter(out))
}

/// The merged value of every char of `scanned` carrying `@atom`, in order, each with
/// its char offset. Identities are looked up only when the [`ATOMS`] map holds an entry.
pub(crate) fn merged_atoms<T: ReadTxn>(
    txn: &T,
    text: &TextRef,
    scanned: &ScannedText,
) -> Result<Vec<(usize, Attrs)>> {
    if scanned.atoms.is_empty() {
        return Ok(Vec::new());
    }
    with_overlay(txn, |overlay| {
        let mut out = Vec::with_capacity(scanned.atoms.len());
        for a in &scanned.atoms {
            // A stray `@atom` over an ordinary char is no atom and has no identity.
            let value = if a.placeholder && !overlay.is_empty() {
                let entries = a.identity(txn, text).and_then(|id| overlay.get(&id));
                merged(&a.value, entries)?
            } else {
                a.value.clone()
            };
            out.push((a.index, value));
        }
        Ok(out)
    })
}

/// A fresh identity, unique across replicas: the writer's client id and the clock its
/// next write takes. Prefixed so it can never equal a char's id.
fn fresh_identity(txn: &TransactionMut) -> String {
    let client = txn.doc().client_id();
    let clock = txn.state_vector().get(&client);
    format!("n{client}:{clock}")
}

/// Write `value` as the `@atom` of the char at UTF-16 offset `u16`, under `identity`
/// (or none) and [`ATOM_FOR`] that char.
fn write_value(
    txn: &mut TransactionMut,
    text: &TextRef,
    u16: u32,
    value: &Attrs,
    identity: Option<&str>,
) {
    let value = match (identity, char_id(txn, text, u16)) {
        (Some(id), Some(own)) => value
            .with(ATOM_ID, AttrValue::from(id))
            .with(ATOM_FOR, AttrValue::from(own)),
        _ => value.clone(),
    };
    text.format(
        txn,
        u16,
        1,
        YAttrs::from([(Arc::from(ATOM_MARK), encode_mark_value(&value))]),
    );
}

/// Bring the atom at char `atom.index` from `have` (its merged value) to `want`, both
/// carrying [`ATOM_TYPE`]: an **attr change** of an atom the text diff kept in place
/// (`projection::reconcile_text`): one [`ATOMS`] entry per attr that changed, under
/// its identity. An atom whose identity another live char also carries (two peers
/// moved it at once) is edited apart: its value is rewritten with a fresh identity and
/// its attrs **as written** (never the changed ones: a value can leak onto a char a
/// peer inserts beside it at once), and the change is entries under the fresh one.
///
/// Writing entries also marks the char with [`ATOM_STAMP`], so a build from before
/// per-attribute merging refuses the document loudly rather than silently miss them
/// (review of #1503, F5). Two peers marking one char at once write equal values.
pub(crate) fn write_atom_change(
    txn: &mut TransactionMut,
    text: &TextRef,
    atom: &AtomChar,
    have: &Attrs,
    want: &Attrs,
) {
    let same_type = have.get(ATOM_TYPE) == want.get(ATOM_TYPE);
    if !same_type || !atom.placeholder {
        // Never reached through the text diff, which matches an atom only with one of
        // its own type; written whole for safety.
        let fresh = fresh_identity(txn);
        write_value(txn, text, atom.u16, want, Some(&fresh));
        return;
    }
    let identity = atom.identity(txn, text);
    let carries = atom.carries(txn, text);
    // A carried identity can be held by two chars: two peers who moved one image at
    // once (both pressed Enter before it) each wrote a copy carrying it. Both read the
    // same, on every replica; a change made to one of them here must not reach the
    // other, so the one changed takes an identity of its own. An identity that is the
    // char's own id is held by no other live char: a copy that carries it was written
    // by a move that deleted this char.
    let (identity, base) = match identity.filter(|id| !carries || claims(txn, id) < 2) {
        Some(identity) => (identity, have.clone()),
        None => {
            let fresh = fresh_identity(txn);
            write_value(txn, text, atom.u16, &atom.value, Some(&fresh));
            (fresh, atom.value.clone())
        }
    };
    let have = &base;
    text.format(
        txn,
        atom.u16,
        1,
        YAttrs::from([(Arc::from(ATOM_STAMP), Any::Bool(true))]),
    );
    let map = txn.get_or_insert_map(ATOMS);
    let put = |txn: &mut TransactionMut, k: &str, any: Any| {
        note_entry(&identity, k, &any);
        map.insert(txn, format!("{identity}/{k}"), any);
    };
    for (k, v) in want.iter() {
        if k.starts_with(RESERVED_PREFIX) || have.get(k) == Some(v) {
            continue;
        }
        put(txn, k, attr_to_any(v).unwrap_or(Any::Null));
    }
    for (k, _) in have.iter() {
        if !k.starts_with(RESERVED_PREFIX) && want.get(k).is_none() {
            put(txn, k, Any::Undefined);
        }
    }
}

/// How many live atom chars in the whole document hold the carried identity `id`. Reads
/// every block's text, so it is asked only when a moved atom's attrs change.
fn claims<T: ReadTxn>(txn: &T, id: &str) -> usize {
    fn count<T: ReadTxn>(txn: &T, value: Out, id: &str) -> usize {
        use yrs::Array;
        match value {
            Out::YText(text) => scan_text(txn, &text).map_or(0, |s| {
                s.atoms
                    .iter()
                    .filter(|a| {
                        a.placeholder && a.carries(txn, &text) && a.carried.as_deref() == Some(id)
                    })
                    .count()
            }),
            Out::YMap(map) => map.iter(txn).map(|(_, v)| count(txn, v, id)).sum(),
            Out::YArray(array) => array.iter(txn).map(|v| count(txn, v, id)).sum(),
            _ => 0,
        }
    }
    use yrs::Array;
    let Some(content) = txn.get_array(CONTENT) else {
        return 0;
    };
    content.iter(txn).map(|v| count(txn, v, id)).sum()
}

// --- carrying an atom's identity through a move -----------------------------------

/// One atom found under a node: where it is and what it reads as.
pub(crate) struct FoundAtom {
    text: TextRef,
    u16: u32,
    identity: String,
    /// Its value as written, without [`ATOM_ID`] and [`ATOM_FOR`].
    value: Attrs,
    /// Its merged value: what the model holds.
    pub merged: Attrs,
}

/// Every inline atom in the nodes at raw indices `range` of `list`, in document order,
/// descending through containers but **not into tables** (a table's cells are keyed by
/// row and column, so the CRDT's order is not the model's; an atom moved into or out of
/// a table is not carried, see [`carry_over`]).
///
/// Best effort, never an error: it runs on both sides of a local change's write (the
/// write's own pre-pass is what refuses a corrupt document). `None` for a text it
/// cannot read, which makes [`carry_over`] carry nothing.
pub(crate) fn find_atoms<T: ReadTxn>(
    txn: &T,
    list: &ArrayRef,
    range: std::ops::Range<u32>,
) -> Option<Vec<FoundAtom>> {
    use yrs::Array;
    with_overlay(txn, |overlay| {
        let mut out = Vec::new();
        // By index: iterating the list would visit every block of the document.
        for i in range {
            TEXT_SCANS.with(|c| c.set(c.get() + 1));
            walk(txn, list.get(txn, i)?, overlay, &mut out)?;
        }
        Some(out)
    })
}

fn walk<T: ReadTxn>(
    txn: &T,
    value: Out,
    overlay: &Overlay,
    out: &mut Vec<FoundAtom>,
) -> Option<()> {
    use yrs::Array;
    match value {
        Out::YText(text) => {
            let scanned = scan_text(txn, &text).ok()?;
            for a in scanned.atoms.iter().filter(|a| a.placeholder) {
                let identity = a.identity(txn, &text)?;
                let merged = merged(&a.value, overlay.get(&identity)).ok()?;
                out.push(FoundAtom {
                    text: text.clone(),
                    u16: a.u16,
                    identity,
                    value: a.value.clone(),
                    merged,
                });
            }
        }
        Out::YMap(map) => {
            if crate::table::is_table_map(txn, &map) {
                return Some(());
            }
            // A node map: its `text` or its `content` (the other keys hold no atom).
            for key in ["text", "content"] {
                if let Some(v) = map.get(txn, key) {
                    walk(txn, v, overlay, out)?;
                }
            }
        }
        Out::YArray(array) => {
            for v in array.iter(txn) {
                walk(txn, v, overlay, out)?;
            }
        }
        _ => {}
    }
    Some(())
}

/// One inline atom of the model: the node (for its identity, `Node::same_ref`) and
/// its projected value (its attrs and [`ATOM_TYPE`]).
pub(crate) struct ModelAtom {
    pub node: Node,
    pub value: Attrs,
}

/// After a local change: an atom the change **moved** keeps its identity. The model
/// says which: the editor keeps the `Rc` of every node an edit does not rebuild, so an
/// atom of `after` that is `same_ref` to one of `before` is that atom, wherever it
/// now is (Enter or Backspace before it, a drag). When the projection wrote it as a
/// new char (yrs has no move), the new char takes the old char's identity, so a peer's
/// concurrent change of its attrs reaches it where it now is.
///
/// `before` / `after` are the CRDT's atoms in the change's span (from [`find_atoms`]),
/// `model_before` / `model_after` the model's in the same blocks, in the same order.
/// The two are paired by position and must agree value for value; when they do not
/// (an atom in a table, a CRDT the model is not in step with), nothing is carried: an
/// identity left behind is a peer's concurrent change lost, an identity carried to the
/// wrong atom would be that change shown on another picture.
///
/// Only an atom that is `same_ref` to exactly one atom of `before`, whose char this
/// change removed, and that is the only one of `after` `same_ref` to it, is carried
/// (an atom copied within the editor and pasted keeps its `Rc` twice: the copy that is
/// new gets an identity of its own). An atom with no attrs of its own (a `hard_break`)
/// has nothing a peer could change, so it carries nothing: its value stays the one-key
/// `{"@type": …}`, whose encoding is the same on every run (#841).
///
/// The new char's value is the moved atom's value **as written** under its identity
/// (and [`ATOM_FOR`] the new char): the entries of that identity lay its changes over
/// it, and a copy of the value that leaks onto a char a peer inserts beside it at once
/// carries neither the changes nor, being written for another char, the identity.
pub(crate) fn carry_over(
    txn: &mut TransactionMut,
    before: &[FoundAtom],
    after: &[FoundAtom],
    model_before: &[ModelAtom],
    model_after: &[ModelAtom],
) {
    let agrees = |crdt: &[FoundAtom], model: &[ModelAtom]| {
        crdt.len() == model.len() && crdt.iter().zip(model).all(|(c, m)| c.merged == m.value)
    };
    if !agrees(before, model_before) || !agrees(after, model_after) {
        return;
    }
    let before_ids: HashSet<&str> = before.iter().map(|a| a.identity.as_str()).collect();
    let after_ids: HashSet<&str> = after.iter().map(|a| a.identity.as_str()).collect();
    let has_attrs = |a: &FoundAtom| {
        a.merged
            .iter()
            .any(|(k, _)| !k.starts_with(RESERVED_PREFIX))
    };
    let mut writes = Vec::new();
    for (a, m) in after.iter().zip(model_after) {
        if before_ids.contains(a.identity.as_str()) {
            continue; // a char the change kept
        }
        if model_after
            .iter()
            .filter(|o| o.node.same_ref(&m.node))
            .count()
            != 1
        {
            continue;
        }
        let mut sources = before
            .iter()
            .zip(model_before)
            .filter(|(_, o)| o.node.same_ref(&m.node));
        let (Some((gone, _)), None) = (sources.next(), sources.next()) else {
            continue;
        };
        if after_ids.contains(gone.identity.as_str()) || !has_attrs(gone) {
            continue;
        }
        writes.push((
            a.text.clone(),
            a.u16,
            gone.value.clone(),
            gone.identity.clone(),
        ));
    }
    for (text, u16, value, identity) in writes {
        write_value(txn, &text, u16, &value, Some(&identity));
    }
}
