//! The public types: axes, places, moves, verdicts and the drag state.

use super::*;

/// The axis a list's items run along.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortAxis {
    /// Top to bottom (a column of cards, a stack of lanes).
    #[default]
    Vertical,
    /// Left to right (a row of columns, a tab strip).
    Horizontal,
}

/// Where in a list a drop lands, named by the items around it — not by an
/// index alone, so a reorder elsewhere in the list between the offer and the
/// app's write does not move it.
///
/// `after` is the item the carried items would follow and `before` the one
/// they would precede, both among the list's items with the carried ones left
/// out. First in the list: no `after`; last: no `before`; an empty list:
/// neither.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SortPlace {
    /// The key of the item right before the drop, `None` at the start.
    pub after: Option<String>,
    /// The key of the item right after the drop, `None` at the end.
    pub before: Option<String>,
    /// Where the first carried item would sit in the list once the carried
    /// items are taken out of it and put back here.
    pub index: usize,
}

impl SortPlace {
    /// First in the list (an empty list's only place included).
    pub fn is_first(&self) -> bool {
        self.after.is_none()
    }

    /// Last in the list (an empty list's only place included).
    pub fn is_last(&self) -> bool {
        self.before.is_none()
    }
}

/// A move on offer or delivered: which items, from which list to which, and
/// where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortMove {
    /// The group both lists belong to.
    pub group: String,
    /// The keys carried, in order: the dragged item's, or what the source
    /// list's `drag_keys` named for it — as given, not checked against any
    /// list.
    pub keys: Vec<String>,
    /// The `id` of the list the drag started in: the pressed item's list,
    /// even when `drag_keys` carries items of other lists too.
    pub from_list: String,
    /// The `id` of the list the drop lands in.
    pub to_list: String,
    /// Where in `to_list`.
    pub place: SortPlace,
}

/// A list's verdict on a move ([`SortableList::judge`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SortVerdict {
    /// The drop may happen.
    Accept,
    /// It may not; the reason is shown at the caret while the drag is over
    /// the place (an empty reason shows the refused caret alone).
    Refuse(String),
}

impl SortVerdict {
    /// Whether the verdict accepts.
    pub fn is_accept(&self) -> bool {
        matches!(self, SortVerdict::Accept)
    }
}

/// How a list judges a move into it.
pub type SortJudge = Rc<dyn Fn(&SortMove) -> SortVerdict>;
/// What a drag from an item carries: given the item's key, the keys moved
/// together, in order.
pub type SortKeys = Rc<dyn Fn(&str) -> Vec<String>>;
/// Builds a pointer drag's preview ([`SortPreview::Custom`]): given a scope to
/// build in and the dragged item's key, the element that follows the pointer.
/// The scope is disposed, and the element discarded, when the drag ends.
pub type SortPreviewFn = Rc<dyn Fn(&mut RenderScope, &str) -> NodeHandle>;

/// What follows the pointer during a pointer drag from a list
/// ([`SortableList::preview`]; the list the drag *starts* in decides). A
/// keyboard lift has no pointer and shows none.
#[derive(Clone, Default)]
pub enum SortPreview {
    /// Nothing: the dragged item stays where it is, styled by
    /// [`SortableList::source`].
    None,
    /// The runtime's built-in drag ghost: on desktop a picture of the item
    /// taken when the drag starts, kept at the grab offset under the pointer.
    /// It is a picture, not an element: no CSS reaches it. The web runtime has
    /// no built-in ghost, so nothing shows there.
    Snapshot,
    /// A copy of the item's elements, in a `.rinch-sortable__ghost` element on
    /// `<body>` that follows the pointer, keeping the point the item was
    /// grabbed at under it. Styled by classes and `--rinch-sortable-ghost-*`
    /// properties. Being on `<body>`, the copy takes no style that depends on
    /// the item's ancestors. The default, as a browser's native drag
    /// image: translucent (`--rinch-sortable-ghost-opacity`, 0.75), no tilt,
    /// no shadow.
    #[default]
    Clone,
    /// The app's own element, built by the function, in the same
    /// `.rinch-sortable__ghost` wrapper as [`SortPreview::Clone`].
    Custom(SortPreviewFn),
}

impl std::fmt::Debug for SortPreview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SortPreview::None => "None",
            SortPreview::Snapshot => "Snapshot",
            SortPreview::Clone => "Clone",
            SortPreview::Custom(_) => "Custom(..)",
        })
    }
}

/// How a list shows the place a drag would land in it
/// ([`SortableList::indicator`]; the list the place is in decides).
///
/// A refused place never opens a gap: it shows the refused caret and the
/// reason in every mode but [`SortIndicator::None`], which still shows the
/// reason.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortIndicator {
    /// An insertion line at the place (the default).
    #[default]
    Caret,
    /// The items from the place on slide apart along the list's axis to open
    /// a gap the size of what is carried, and back when the place moves on.
    /// The slide is a CSS transition (`--rinch-sortable-reflow-duration`,
    /// `--rinch-sortable-reflow-easing`); layout does not move.
    ///
    /// With [`SortableList::slot`] the gap also holds a box the app can
    /// style, where the drop lands.
    Gap,
    /// The gap and the caret.
    Both,
    /// Nothing but a refusal's reason: the app draws its own from
    /// [`sortable_drag`].
    None,
}

impl SortIndicator {
    pub(super) fn caret(self) -> bool {
        matches!(self, SortIndicator::Caret | SortIndicator::Both)
    }

    pub(super) fn gap(self) -> bool {
        matches!(self, SortIndicator::Gap | SortIndicator::Both)
    }
}

/// How the dragged items look in their list during a pointer drag
/// ([`SortableList::source`]; the list the drag starts in decides). A
/// keyboard lift always rings the lifted items instead.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortSource {
    /// Faded (`--rinch-sortable-dragging-opacity`).
    Dim,
    /// Hidden, and the items after them in their list slide up over their
    /// space (the same transition as [`SortIndicator::Gap`]); over its own
    /// place a list with a gap indicator opens the gap there again.
    Collapse,
    /// Hidden, their space kept.
    Hide,
    /// As they are: the default, as a browser's native drag leaves its
    /// source.
    #[default]
    Unchanged,
}

/// Where a drag is: a list and an index in it, counted with the carried items
/// left out (as [`SortPlace::index`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortTarget {
    /// The list's `id`.
    pub list: String,
    /// The place in it.
    pub index: usize,
}

/// The sortable drag in progress, for an app that draws its own presentation
/// ([`sortable_drag`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortDragState {
    /// The group dragged.
    pub group: String,
    /// The keys carried, in order.
    pub keys: Vec<String>,
    /// The key of the item the drag was started from.
    pub source_key: String,
    /// The `id` of the list the drag started in.
    pub from_list: String,
    /// A keyboard lift rather than a pointer drag.
    pub keyboard: bool,
    /// The place under the pointer (or the lift's place), whether or not it
    /// would change anything — the dragged item's own place included.
    pub over: Option<SortTarget>,
    /// The place on offer: where a drop now would land. `None` over nothing,
    /// and over a place that would leave the lists as they are.
    pub target: Option<SortTarget>,
    /// The verdict on [`Self::target`]; a refusal carries its reason.
    pub verdict: Option<SortVerdict>,
}

/// Where the pointer is in a pointer drag ([`sortable_drag_pointer`]), in
/// viewport-relative logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SortPointer {
    /// The pointer.
    pub x: f32,
    /// The pointer.
    pub y: f32,
    /// Where in the dragged item it was grabbed, from its top-left corner: a
    /// preview drawn at `(x - grab_x, y - grab_y)` keeps that point under the
    /// pointer.
    pub grab_x: f32,
    /// See [`Self::grab_x`].
    pub grab_y: f32,
    /// The dragged item's size when the drag started.
    pub width: f32,
    /// See [`Self::width`].
    pub height: f32,
}
