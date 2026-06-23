//! Decorations — view-side annotations that are **rendered but never part of the
//! document**.
//!
//! The placeholder shown in an empty editor (design A8), the IME composition
//! preedit (M6), a search highlight, and a remote collaborator's caret (M9) are
//! all decorations: they are computed *from* [`EditorState`] but live outside the
//! `doc`, so they never flow through a [`Step`](crate::transform::Step) and never
//! corrupt the model. The view treats the [`DecorationSet`] as a first-class
//! input and diffs `prev` vs `next` decorations **independently of the document
//! diff** (design A4) — a transaction that changes only decorations (an IME
//! preedit keystroke) still produces a visible update with zero document churn.
//!
//! Plugins contribute decorations through
//! [`Plugin::decorations`](crate::plugin::Plugin::decorations); the state
//! aggregates them in
//! [`EditorState::decorations`](crate::state::EditorState::decorations).
//!
//! **Scope (M5):** only [`Widget`] decorations exist, because the placeholder is
//! the only thing M5 renders this way. Inline decorations (preedit underline,
//! search highlight — `Inline { from, to, attrs }`) land in M6 and node
//! decorations in M7; adding those variants now would be unconstructed and trip
//! the crate's no-dead-code gate.
//!
//! [`EditorState`]: crate::state::EditorState

use crate::pos::Pos;
use std::rc::Rc;

/// One decoration. Currently always a [`Decoration::Widget`]; the inline and node
/// kinds arrive with later milestones (see the module docs).
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
}

impl Decoration {
    /// A [`Decoration::Widget`] at `pos`.
    pub fn widget(pos: Pos, side: i32, widget: Widget) -> Decoration {
        Decoration::Widget { pos, side, widget }
    }

    /// The (single) document position this decoration is anchored at. For a
    /// widget, the position it is drawn at.
    pub fn pos(&self) -> Pos {
        match self {
            Decoration::Widget { pos, .. } => *pos,
        }
    }

    /// This decoration's placeholder prompt text, if it is a
    /// [`Widget::Placeholder`]; `None` otherwise (other widget kinds and the
    /// inline/node decoration kinds added in later milestones). Lets the view pull
    /// the placeholder out of a [`DecorationSet`] without matching the enum.
    pub fn as_placeholder(&self) -> Option<&str> {
        match self {
            Decoration::Widget {
                widget: Widget::Placeholder(text),
                ..
            } => Some(text),
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
}
