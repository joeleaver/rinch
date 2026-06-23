//! A mark instance: a piece of inline formatting applied to text.
//!
//! A [`Mark`] pairs a [`MarkType`] handle with its typed [`Attrs`] (e.g. a `link`
//! mark carries `href`, a `text_color` mark carries `color`). Marks are small,
//! cheap to clone, and `Hash`/`Eq` (so a `Node`'s mark list and `Attrs` can be
//! compared structurally).

use crate::model::{Attrs, MarkType};

/// An applied mark (formatting) on inline content.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Mark {
    /// The mark type (bold, italic, link, …).
    pub typ: MarkType,
    /// The mark's attributes (empty for simple marks like bold).
    pub attrs: Attrs,
}

impl Mark {
    /// Create a mark of `typ` with `attrs`.
    pub fn new(typ: MarkType, attrs: Attrs) -> Self {
        Mark { typ, attrs }
    }

    /// Create a simple, attribute-less mark.
    pub fn simple(typ: MarkType) -> Self {
        Mark {
            typ,
            attrs: Attrs::new(),
        }
    }

    /// The mark type's name (e.g. `"bold"`).
    pub fn type_name(&self) -> &str {
        self.typ.name()
    }

    /// True if `self` and `other` are the same mark type (regardless of attrs).
    pub fn same_type(&self, other: &Mark) -> bool {
        self.typ == other.typ
    }

    /// True if this mark is present in `set` (same type and attrs).
    pub fn is_in(&self, set: &[Mark]) -> bool {
        set.contains(self)
    }

    /// Add this mark to `set`, returning the new set. If an existing mark equals
    /// this one the set is returned unchanged; marks this one *excludes* are
    /// dropped; if an existing mark excludes this one the set is returned unchanged
    /// (the mark cannot be added). Port of ProseMirror's `Mark.addToSet` (without
    /// the rank-based ordering — the new mark is appended, which preserves set
    /// membership; ordering is not load-bearing for our value model).
    pub fn add_to_set(&self, set: &[Mark]) -> Vec<Mark> {
        let mut copy: Option<Vec<Mark>> = None;
        for (i, other) in set.iter().enumerate() {
            if self == other {
                return set.to_vec();
            }
            if self.typ.excludes(other.type_name()) {
                // this excludes other → drop other
                if copy.is_none() {
                    copy = Some(set[..i].to_vec());
                }
            } else if other.typ.excludes(self.type_name()) {
                // other excludes this → cannot add
                return set.to_vec();
            } else if let Some(c) = copy.as_mut() {
                c.push(other.clone());
            }
        }
        let mut copy = copy.unwrap_or_else(|| set.to_vec());
        copy.push(self.clone());
        copy
    }

    /// Remove this mark from `set`, returning the new set (unchanged if absent).
    /// Port of `Mark.removeFromSet`.
    pub fn remove_from_set(&self, set: &[Mark]) -> Vec<Mark> {
        match set.iter().position(|m| m == self) {
            Some(i) => {
                let mut v = set[..i].to_vec();
                v.extend_from_slice(&set[i + 1..]);
                v
            }
            None => set.to_vec(),
        }
    }
}
