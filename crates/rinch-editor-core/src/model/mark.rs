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
}
