//! Decorations — view-side annotations that are **rendered but never part of the
//! document**.
//!
//! The placeholder shown in an empty editor (design A8), the IME composition
//! preedit (M6), a search highlight, a spellcheck squiggle, and a remote
//! collaborator's caret (M9) are all decorations: they are computed *from*
//! [`EditorState`] but live outside the `doc`, so they never flow through a
//! [`Step`](crate::transform::Step) and never corrupt the model. The view treats
//! the [`DecorationSet`] as a first-class input and diffs `prev` vs `next`
//! decorations **independently of the document diff** (design A4) — a transaction
//! that changes only decorations (an IME preedit keystroke, a spellchecker
//! reporting a new misspelling) still produces a visible update with zero
//! document churn.
//!
//! Plugins contribute decorations through
//! [`Plugin::decorations`](crate::plugin::Plugin::decorations); the state
//! aggregates them in
//! [`EditorState::decorations`](crate::state::EditorState::decorations).
//!
//! # Mapping through document changes
//!
//! A [`DecorationSet`] is **not** stored and remapped by the core the way a
//! [`Selection`](crate::Selection) is. `EditorState::decorations()` calls every
//! plugin's `decorations(&self)` afresh for the state being rendered, so the set
//! the view sees is always recomputed against the *current* document. There is
//! therefore no position mapping to get wrong inside the core — and, by the same
//! token, **mapping is the plugin's responsibility**: a plugin that caches
//! ranges across transactions (a spellchecker that does not want to re-scan the
//! whole document on every keystroke) must map its own cached positions through
//! `tr.mapping()` in [`Plugin::apply`](crate::plugin::Plugin::apply) — exactly as
//! it would for any other position it holds — and hand out the mapped ranges from
//! `decorations`. A plugin that derives its ranges from the document each time
//! (the placeholder plugin, a plugin that scans `state.doc`) needs to do nothing.
//!
//! **Scope:** [`Widget`] decorations (the placeholder) and [`Inline`] decorations
//! (a class applied to a document range — spellcheck squiggles, search
//! highlights). Node decorations — a class on a whole block — land in M7; adding
//! that variant now would be unconstructed and trip the crate's no-dead-code
//! gate.
//!
//! [`EditorState`]: crate::state::EditorState
//! [`Inline`]: Decoration::Inline

use crate::model::Attrs;
use crate::pos::Pos;
use std::rc::Rc;

/// One decoration: a [`Decoration::Widget`] anchored at a single position, or a
/// [`Decoration::Inline`] styling a range of the document. The node kind arrives
/// with a later milestone (see the module docs).
#[derive(Clone, Debug, PartialEq)]
pub enum Decoration {
    /// A widget anchored at a single document position, drawn *between* content
    /// and never selectable. Used for the empty-editor placeholder now, and for
    /// the IME preedit / collaborator carets in later milestones.
    Widget {
        /// The document position the widget is anchored at.
        pos: Pos,
        /// Draw bias when content or another widget shares this position: a
        /// negative `side` draws before, a positive `side` after. Mirrors
        /// ProseMirror's widget `side`.
        side: i32,
        /// The renderer-agnostic widget payload.
        widget: Widget,
    },
    /// Styling applied to the text in `[from, to)` without touching the document
    /// — the spellcheck squiggle and the search highlight.
    ///
    /// `attrs` is renderer-agnostic; the view reads **`class`** (a space-separated
    /// CSS class list, exactly as the HTML attribute) and puts it on the host
    /// element(s) it wraps the decorated text in, so the same decoration is styled
    /// by ordinary CSS on the web backend and by the same cascade on the native
    /// one. An empty range (`from == to`) draws nothing — use a
    /// [`Decoration::Widget`] for a zero-width marker.
    Inline {
        /// The first position of the decorated range (inclusive).
        from: Pos,
        /// The position just past the decorated range (exclusive).
        to: Pos,
        /// Renderer-agnostic attributes; `class` is the one the view honours.
        attrs: Attrs,
    },
}

impl Decoration {
    /// A [`Decoration::Widget`] at `pos`.
    pub fn widget(pos: Pos, side: i32, widget: Widget) -> Decoration {
        Decoration::Widget { pos, side, widget }
    }

    /// A [`Decoration::Inline`] over `[from, to)`.
    ///
    /// The range is normalised, so a caller that computed its ends in either
    /// order gets the same decoration.
    pub fn inline(from: Pos, to: Pos, attrs: Attrs) -> Decoration {
        let (from, to) = if from.0 <= to.0 {
            (from, to)
        } else {
            (to, from)
        };
        Decoration::Inline { from, to, attrs }
    }

    /// The (single) document position this decoration is anchored at. For a
    /// widget, the position it is drawn at; for an inline decoration, the start
    /// of its range.
    pub fn pos(&self) -> Pos {
        match self {
            Decoration::Widget { pos, .. } => *pos,
            Decoration::Inline { from, .. } => *from,
        }
    }

    /// The range this decoration covers: `[from, to)` for an inline decoration,
    /// the empty range at its anchor for a widget.
    pub fn range(&self) -> (Pos, Pos) {
        match self {
            Decoration::Widget { pos, .. } => (*pos, *pos),
            Decoration::Inline { from, to, .. } => (*from, *to),
        }
    }

    /// The CSS class list of a non-empty [`Decoration::Inline`]; `None` for a
    /// widget, for an inline decoration with no `class` attr, and for one whose
    /// range is empty (nothing to wrap). The view's whole read of an inline
    /// decoration goes through this, so it never matches the enum.
    pub fn inline_class(&self) -> Option<&str> {
        match self {
            Decoration::Inline { from, to, attrs } if from.0 < to.0 => {
                attrs.get_str("class").filter(|c| !c.trim().is_empty())
            }
            _ => None,
        }
    }

    /// This decoration's placeholder prompt text, if it is a
    /// [`Widget::Placeholder`]; `None` otherwise (other widget kinds, and inline
    /// decorations). Lets the view pull the placeholder out of a
    /// [`DecorationSet`] without matching the enum.
    pub fn as_placeholder(&self) -> Option<&str> {
        match self {
            Decoration::Widget {
                widget: Widget::Placeholder(text),
                ..
            } => Some(text),
            Decoration::Inline { .. } => None,
        }
    }
}

/// The renderer-agnostic content of a [`Decoration::Widget`]. The view maps each
/// variant onto host nodes; the core never references a renderer. Grows in later
/// milestones (M6 IME preedit, M9 collaborator caret).
#[derive(Clone, Debug, PartialEq)]
pub enum Widget {
    /// Dimmed prompt text shown when the editor is empty (design A8). Shared from
    /// the placeholder plugin's config, so cloning a `DecorationSet` is cheap.
    Placeholder(Rc<str>),
}

/// An ordered collection of [`Decoration`]s for one editor state — the view's
/// decoration input. Cheap to clone and compare (`PartialEq`) so the view can
/// diff `prev` against `next`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DecorationSet {
    decorations: Vec<Decoration>,
}

impl DecorationSet {
    /// The empty set.
    pub fn empty() -> DecorationSet {
        DecorationSet::default()
    }

    /// A set wrapping the given decorations (in draw order).
    pub fn new(decorations: Vec<Decoration>) -> DecorationSet {
        DecorationSet { decorations }
    }

    /// True if there are no decorations.
    pub fn is_empty(&self) -> bool {
        self.decorations.is_empty()
    }

    /// The number of decorations.
    pub fn len(&self) -> usize {
        self.decorations.len()
    }

    /// Iterate the decorations in draw order.
    pub fn iter(&self) -> std::slice::Iter<'_, Decoration> {
        self.decorations.iter()
    }

    /// Append one decoration.
    pub fn push(&mut self, decoration: Decoration) {
        self.decorations.push(decoration);
    }

    /// Merge `other`'s decorations into this set (used to aggregate plugins'
    /// contributions in [`EditorState::decorations`](crate::state::EditorState::decorations)).
    pub fn merge(&mut self, other: DecorationSet) {
        self.decorations.extend(other.decorations);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_set_has_no_decorations() {
        let set = DecorationSet::empty();
        assert!(set.is_empty());
        assert_eq!(set.len(), 0);
        assert_eq!(set.iter().count(), 0);
    }

    #[test]
    fn widget_decoration_records_pos_and_payload() {
        let d = Decoration::widget(Pos(1), 0, Widget::Placeholder(Rc::from("Type here…")));
        assert_eq!(d.pos(), Pos(1));
        match &d {
            Decoration::Widget { widget, side, .. } => {
                assert_eq!(*side, 0);
                assert_eq!(*widget, Widget::Placeholder(Rc::from("Type here…")));
            }
            other => panic!("expected a widget, got {other:?}"),
        }
    }

    #[test]
    fn push_and_merge_accumulate_in_order() {
        let mut set = DecorationSet::empty();
        set.push(Decoration::widget(
            Pos(1),
            0,
            Widget::Placeholder(Rc::from("a")),
        ));
        let mut other = DecorationSet::empty();
        other.push(Decoration::widget(
            Pos(2),
            -1,
            Widget::Placeholder(Rc::from("b")),
        ));
        set.merge(other);
        assert_eq!(set.len(), 2);
        let positions: Vec<Pos> = set.iter().map(|d| d.pos()).collect();
        assert_eq!(positions, vec![Pos(1), Pos(2)]);
    }

    #[test]
    fn sets_compare_structurally_for_diffing() {
        let a = DecorationSet::new(vec![Decoration::widget(
            Pos(1),
            0,
            Widget::Placeholder(Rc::from("x")),
        )]);
        let b = DecorationSet::new(vec![Decoration::widget(
            Pos(1),
            0,
            Widget::Placeholder(Rc::from("x")),
        )]);
        let c = DecorationSet::new(vec![Decoration::widget(
            Pos(2),
            0,
            Widget::Placeholder(Rc::from("x")),
        )]);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    /// `class` is the one attr the view reads, so build an inline decoration the
    /// way a consumer plugin would.
    fn classed(from: usize, to: usize, class: &str) -> Decoration {
        let mut attrs = Attrs::new();
        attrs = attrs.with("class", class);
        Decoration::inline(Pos(from), Pos(to), attrs)
    }

    #[test]
    fn inline_decoration_records_its_range_and_class() {
        let d = classed(3, 9, "pm-spell-error");
        assert_eq!(d.pos(), Pos(3));
        assert_eq!(d.range(), (Pos(3), Pos(9)));
        assert_eq!(d.inline_class(), Some("pm-spell-error"));
        // An inline decoration is not a placeholder, and a widget has no class.
        assert_eq!(d.as_placeholder(), None);
        let w = Decoration::widget(Pos(1), 0, Widget::Placeholder(Rc::from("x")));
        assert_eq!(w.inline_class(), None);
        assert_eq!(w.range(), (Pos(1), Pos(1)));
    }

    #[test]
    fn inline_range_is_normalised() {
        assert_eq!(classed(9, 3, "c").range(), (Pos(3), Pos(9)));
        assert_eq!(classed(9, 3, "c"), classed(3, 9, "c"));
    }

    #[test]
    fn an_inline_decoration_with_nothing_to_draw_has_no_class() {
        // Empty range: there is no text to wrap, so the view must skip it.
        assert_eq!(classed(4, 4, "pm-spell-error").inline_class(), None);
        // No `class` attr, and a blank one, are both nothing to apply.
        assert_eq!(
            Decoration::inline(Pos(0), Pos(2), Attrs::new()).inline_class(),
            None
        );
        assert_eq!(classed(0, 2, "   ").inline_class(), None);
    }

    #[test]
    fn inline_sets_compare_structurally_for_diffing() {
        // The view's whole decoration diff is this comparison (design A4): a
        // spellchecker that reports the same misspellings must produce an equal
        // set, and a moved range an unequal one.
        let a = DecorationSet::new(vec![classed(3, 9, "pm-spell-error")]);
        let b = DecorationSet::new(vec![classed(3, 9, "pm-spell-error")]);
        let moved = DecorationSet::new(vec![classed(4, 10, "pm-spell-error")]);
        let reclassed = DecorationSet::new(vec![classed(3, 9, "pm-grammar-error")]);
        assert_eq!(a, b);
        assert_ne!(a, moved);
        assert_ne!(a, reclassed);
    }

    #[test]
    fn widgets_and_inlines_merge_into_one_set_in_order() {
        let mut set = DecorationSet::new(vec![Decoration::widget(
            Pos(1),
            0,
            Widget::Placeholder(Rc::from("a")),
        )]);
        set.merge(DecorationSet::new(vec![
            classed(2, 5, "pm-spell-error"),
            classed(8, 11, "pm-spell-error"),
        ]));
        assert_eq!(set.len(), 3);
        // The placeholder is still findable past the inline decorations, and the
        // inline ones are findable past the widget.
        assert_eq!(set.iter().find_map(|d| d.as_placeholder()), Some("a"));
        let ranges: Vec<(Pos, Pos)> = set
            .iter()
            .filter(|d| d.inline_class().is_some())
            .map(|d| d.range())
            .collect();
        assert_eq!(ranges, vec![(Pos(2), Pos(5)), (Pos(8), Pos(11))]);
    }
}
