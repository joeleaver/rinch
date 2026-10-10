//! An inline atom's attributes, merged **per attribute**.
//!
//! An inline atom (`image`, `hard_break`) is one U+FFFC char in its block's `Text`
//! carrying the reserved formatting attribute `@atom` (see [`crate::projection`]). That
//! value is the atom's type and attrs **as they were when the char was written**, and it
//! is never rewritten to change an attr. A change of one attr is one entry of the root
//! map [`ATOMS`] instead:
//!
//! ```text
//! root Map "atoms"
//!   "<identity>/<attr>" -> Any    // the attr's value; `Undefined` = the attr removed
//! ```
//!
//! An atom reads as its `@atom` value with every entry under its identity laid over
//! it. yrs merges a map per key, so two peers changing **different** attrs of one atom
//! at once (one its `alt`, the other an app's `board` id) write different keys and both
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
//! A char's id cannot follow the atom when the atom **moves**, and the projection moves
//! one more often than it looks: Enter before an image in its line deletes the image's
//! char and inserts a new one in the new block (yrs has no move), and Backspace joining
//! the line back does the same. A local transaction that removes an atom and inserts
//! one with the same type and attrs therefore hands the new char the old identity, in
//! the `@atom` value's reserved key [`ATOM_ID`] ([`carry_over`]). A peer's concurrent
//! change of the atom's attrs is an entry under that identity, so it applies to the
//! moved atom as well: neither the change nor the move is lost.
//!
//! An `@atom` value is also rewritten, with a **fresh** identity, when a char changes
//! from one atom type to another, or when its value was inherited (below): the entries
//! of the old identity no longer describe it.
//!
//! **Inherited values.** yrs extends a formatted range over an insert at its end, so a
//! char inserted right after an atom carries that atom's `@atom` value, `@id` and all.
//! Only the **first** char of a run of one `@atom` value holding an `@id` is that
//! identity; a char that continues the run is identified by its own char id
//! ([`AtomChar::inherited`]).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use yrs::types::Attrs as YAttrs;
use yrs::types::text::YChange;
use yrs::{
    Any, ArrayRef, Assoc, IndexedSequence, Map, Out, ReadTxn, Text, TextRef, TransactionMut,
    WriteTxn,
};

use rinch_editor_core::{AttrValue, Attrs};

use crate::error::{CollabError, Result};
use crate::projection::{
    ATOM_MARK, ATOM_PLACEHOLDER, ATOM_TYPE, CONTENT, RESERVED_PREFIX, attr_to_any,
    decode_attr_value, decode_mark_value, encode_mark_value,
};

/// The root map holding every atom attribute written after its atom's char.
pub(crate) const ATOMS: &str = "atoms";

/// The reserved key in an `@atom` value naming the identity the char carries over from
/// the atom it replaced (a move), or a fresh one (a retype).
pub(crate) const ATOM_ID: &str = "@id";

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
    /// The `@atom` value without [`ATOM_ID`]: the atom's type and attrs when the char
    /// was written.
    pub value: Attrs,
    /// The value's [`ATOM_ID`].
    pub carried: Option<String>,
    /// The value has an [`ATOM_ID`] and the char before carries the same value: this
    /// char inherited it by being inserted at the end of that atom's range.
    pub inherited: bool,
}

impl AtomChar {
    /// The atom's identity: its carried id, or its char's yrs id. `None` only for a
    /// char yrs cannot locate, which a live char always is.
    pub fn identity<T: ReadTxn>(&self, txn: &T, text: &TextRef) -> Option<String> {
        match &self.carried {
            Some(id) if !self.inherited => Some(id.clone()),
            _ => char_id(txn, text, self.u16),
        }
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
    let mut s = String::new();
    let mut chars = 0usize;
    let mut u16 = 0u32;
    let mut spans = Vec::new();
    let mut atoms: Vec<AtomChar> = Vec::new();
    // The `@atom` value of the char before the current one, as written.
    let mut prev: Option<Any> = None;
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
                if matches!(value, Any::Null | Any::Undefined) {
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
            let stripped = decoded.without(ATOM_ID);
            let mut at = u16;
            for (k, c) in part.chars().enumerate() {
                let continues = if k == 0 {
                    prev.as_ref() == Some(value)
                } else {
                    true
                };
                atoms.push(AtomChar {
                    index: start + k,
                    u16: at,
                    placeholder: c == ATOM_PLACEHOLDER,
                    value: stripped.clone(),
                    carried: carried.clone(),
                    inherited: carried.is_some() && continues,
                });
                at += c.len_utf16() as u32;
            }
            prev = Some(value.clone());
        } else if n > 0 {
            prev = None;
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

/// Every entry of the [`ATOMS`] map, grouped by identity. Empty when the map does not
/// exist or holds nothing.
pub(crate) fn read_overlay<T: ReadTxn>(txn: &T) -> HashMap<String, Vec<(String, Any)>> {
    let mut out: HashMap<String, Vec<(String, Any)>> = HashMap::new();
    let Some(map) = txn.get_map(ATOMS) else {
        return out;
    };
    for (key, value) in map.iter(txn) {
        let (Some((identity, attr)), Out::Any(any)) = (key.split_once('/'), value) else {
            continue;
        };
        out.entry(identity.to_string())
            .or_default()
            .push((attr.to_string(), any));
    }
    out
}

/// An atom's value with the entries of its identity laid over it. An entry for a
/// reserved key is ignored (the type is never changed this way: a retype writes a
/// fresh identity); `Undefined` removes the attr.
pub(crate) fn merged(value: &Attrs, entries: Option<&Vec<(String, Any)>>) -> Result<Attrs> {
    let mut out = value.clone();
    for (k, v) in entries.into_iter().flatten() {
        if k.starts_with(RESERVED_PREFIX) {
            continue;
        }
        out = match v {
            Any::Undefined => out.without(k),
            other => out.with(k.as_str(), decode_attr_value(k, other)?),
        };
    }
    Ok(out)
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
    let overlay = read_overlay(txn);
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
}

/// A fresh identity, unique across replicas: the writer's client id and the clock its
/// next write takes. Prefixed so it can never equal a char's id.
fn fresh_identity(txn: &TransactionMut) -> String {
    let client = txn.doc().client_id();
    let clock = txn.state_vector().get(&client);
    format!("n{client}:{clock}")
}

/// Write `value` as the `@atom` of the char at UTF-16 offset `u16`, under `identity`
/// (or none).
fn write_value(
    txn: &mut TransactionMut,
    text: &TextRef,
    u16: u32,
    value: &Attrs,
    identity: Option<&str>,
) {
    let value = match identity {
        Some(id) => value.with(ATOM_ID, AttrValue::from(id)),
        None => value.clone(),
    };
    text.format(
        txn,
        u16,
        1,
        YAttrs::from([(Arc::from(ATOM_MARK), encode_mark_value(&value))]),
    );
}

/// Bring the atom at char `atom.index` from `have` (its merged value) to `want`, both
/// carrying [`ATOM_TYPE`]. The same type on a char that is its own atom: one [`ATOMS`]
/// entry per attr that changed, and nothing else. Anything else (a retype, a value the
/// char inherited): its `@atom` is rewritten whole under a fresh identity.
pub(crate) fn write_atom_change(
    txn: &mut TransactionMut,
    text: &TextRef,
    atom: &AtomChar,
    have: &Attrs,
    want: &Attrs,
) {
    let same_type = have.get(ATOM_TYPE) == want.get(ATOM_TYPE);
    let identity = if same_type && atom.placeholder && !atom.inherited {
        atom.identity(txn, text)
    } else {
        None
    };
    // A carried identity can be held by two chars: two peers who moved one image at
    // once (both pressed Enter before it) each wrote a copy carrying it. Both read the
    // same, on every replica; a change made to one of them here must not reach the
    // other, so the one changed takes an identity of its own.
    let identity = identity.filter(|id| atom.carried.is_none() || claims(txn, id) < 2);
    let Some(identity) = identity else {
        let fresh = fresh_identity(txn);
        write_value(txn, text, atom.u16, want, Some(&fresh));
        return;
    };
    let map = txn.get_or_insert_map(ATOMS);
    for (k, v) in want.iter() {
        if k.starts_with(RESERVED_PREFIX) || have.get(k) == Some(v) {
            continue;
        }
        let any = attr_to_any(v).unwrap_or(Any::Null);
        map.insert(txn, format!("{identity}/{k}"), any);
    }
    for (k, _) in have.iter() {
        if !k.starts_with(RESERVED_PREFIX) && want.get(k).is_none() {
            map.insert(txn, format!("{identity}/{k}"), Any::Undefined);
        }
    }
}

/// How many live atom chars in the whole document hold the carried identity `id`. Reads
/// every block's text, so it is asked only when a carried atom's attrs change.
fn claims<T: ReadTxn>(txn: &T, id: &str) -> usize {
    fn count<T: ReadTxn>(txn: &T, value: Out, id: &str) -> usize {
        use yrs::Array;
        match value {
            Out::YText(text) => scan_text(txn, &text).map_or(0, |s| {
                s.atoms
                    .iter()
                    .filter(|a| a.placeholder && !a.inherited && a.carried.as_deref() == Some(id))
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
    /// Its value as written, without [`ATOM_ID`].
    value: Attrs,
    /// Its merged value: what the model holds.
    merged: Attrs,
}

/// Every inline atom in the nodes at raw indices `range` of `list`, in document order,
/// descending through containers and tables.
///
/// Best effort, never an error: it runs on both sides of a local change's write (the
/// write's own pre-pass is what refuses a corrupt document), and a text it cannot read
/// only means an atom in it keeps the identity of its char.
pub(crate) fn find_atoms<T: ReadTxn>(
    txn: &T,
    list: &ArrayRef,
    range: std::ops::Range<u32>,
) -> Vec<FoundAtom> {
    use yrs::Array;
    let overlay = read_overlay(txn);
    let mut out = Vec::new();
    for (i, child) in list.iter(txn).enumerate() {
        if range.contains(&(i as u32)) {
            walk(txn, child, &overlay, &mut out);
        }
    }
    out
}

fn walk<T: ReadTxn>(
    txn: &T,
    value: Out,
    overlay: &HashMap<String, Vec<(String, Any)>>,
    out: &mut Vec<FoundAtom>,
) {
    use yrs::Array;
    match value {
        Out::YText(text) => {
            let Ok(scanned) = scan_text(txn, &text) else {
                return;
            };
            for a in scanned.atoms.iter().filter(|a| a.placeholder) {
                let Some(identity) = a.identity(txn, &text) else {
                    continue;
                };
                let Ok(merged) = merged(&a.value, overlay.get(&identity)) else {
                    continue;
                };
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
            // A yrs map's key order is not document order; sorted, at least it is the
            // same on every run. Only a table's cells (keyed by row and column) come back
            // out of document order, and matching a moved atom does not depend on it.
            let mut entries: Vec<(String, Out)> =
                map.iter(txn).map(|(k, v)| (k.to_string(), v)).collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (_, v) in entries {
                walk(txn, v, overlay, out);
            }
        }
        Out::YArray(array) => {
            for v in array.iter(txn) {
                walk(txn, v, overlay, out);
            }
        }
        _ => {}
    }
}

/// After a local change: every atom in `after` that is not one of `before` (a char this
/// change inserted) and reads exactly as one of `before` that is gone (a char this
/// change removed) takes that atom's identity. Each removed atom is taken at most once,
/// in document order.
///
/// An atom with no attrs of its own (a `hard_break`) has nothing a peer could change,
/// so it carries nothing: its value stays the one-key `{"@type": …}`, whose encoding
/// is the same on every run (#841).
pub(crate) fn carry_over(txn: &mut TransactionMut, before: &[FoundAtom], after: &[FoundAtom]) {
    let before_ids: HashSet<&str> = before.iter().map(|a| a.identity.as_str()).collect();
    let after_ids: HashSet<&str> = after.iter().map(|a| a.identity.as_str()).collect();
    let has_attrs = |a: &FoundAtom| {
        a.merged
            .iter()
            .any(|(k, _)| !k.starts_with(RESERVED_PREFIX))
    };
    let mut removed: Vec<Option<&FoundAtom>> = before
        .iter()
        .filter(|a| has_attrs(a) && !after_ids.contains(a.identity.as_str()))
        .map(Some)
        .collect();
    if removed.is_empty() {
        return;
    }
    for a in after
        .iter()
        .filter(|a| !before_ids.contains(a.identity.as_str()))
    {
        let Some(slot) = removed
            .iter_mut()
            .find(|r| r.is_some_and(|r| r.merged == a.merged))
        else {
            continue;
        };
        let gone = slot.take().expect("matched a live slot");
        write_value(txn, &a.text, a.u16, &a.value, Some(&gone.identity));
    }
}
