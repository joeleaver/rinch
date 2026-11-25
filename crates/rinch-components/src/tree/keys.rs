//! The keyboard's public key type, structural moves, and key-to-row navigation.

use super::*;

// =============================================================================
// Keyboard
// =============================================================================

/// A structural move of the cursor row, for an app that binds keys to them
/// ([`TreeKey::drop_for`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TreeMove {
    /// Into the previous sibling, as its last child.
    Indent,
    /// Out of its parent, right after it.
    Outdent,
    /// Before its previous sibling.
    Up,
    /// After its next sibling.
    Down,
}

/// A key pressed while one of the tree's rows has the keyboard, offered to
/// the app before the tree's own keys ([`Tree::handle_key`]).
pub struct TreeKey {
    /// The key, as `KeyEventData::key` names it (`"ArrowDown"`, `"F2"`, `"a"`).
    pub key: String,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
    /// The row the keyboard is on.
    pub cursor: String,
    /// The cursor row's parent; `None` at the top level.
    pub parent: Option<String>,
    /// The rows drawn right above and below the cursor row, for an app that
    /// moves the cursor off a row it is about to remove. When the cursor row
    /// is one of several selected rows (what [`TreeKey::carried`] moves),
    /// the rows drawn right above and below that block instead, the rows
    /// inside it skipped.
    pub above: Option<String>,
    pub below: Option<String>,
    pub(super) shape: Rc<Shape>,
    /// The rows the key moves, in tree order: the cursor row, or the selected
    /// rows when it is one of several. Taken from the tree, never from
    /// `drag_values` (which may name app ids rather than rows).
    pub(super) rows: Vec<usize>,
    pub(super) carried: Vec<String>,
    pub(super) judge_drop: Option<JudgeDrop>,
}

impl TreeKey {
    /// What the key acts on: the selection top to bottom when the cursor
    /// row is one of several selected rows (rows inside other selected rows
    /// left out), the cursor row alone otherwise; through `drag_values`
    /// when it is given (each value once).
    pub fn carried(&self) -> &[String] {
        &self.carried
    }

    /// The drop that would make `movement` of the cursor row, or of the
    /// selected rows as one block (tree order) when it is one of several,
    /// carrying [`TreeKey::carried`], for the app to deliver as it
    /// does a dropped one. Which rows move never depends on
    /// [`Tree::drag_values`]. `None` when there is nowhere to go (no previous
    /// sibling to indent into, no parent to outdent from, no sibling above
    /// or below), and whenever a drag would refuse the same drop: into a row
    /// that takes no children ([`Tree::accepts_children`]), into a carried
    /// row or its subtree, or refused by [`Tree::judge_drop`].
    pub fn drop_for(&self, movement: TreeMove) -> Option<TreeDrop> {
        let shape = &self.shape;
        // The rows moved (several when the cursor row is in a selection)
        // move as one block: they must share a parent.
        let rows = &self.rows;
        let (&first, &last) = (rows.first()?, rows.last()?);
        let parent_of = |r: usize| shape.rows[r].parent;
        if rows.iter().any(|&r| parent_of(r) != parent_of(first)) {
            return None;
        }
        let siblings = shape.siblings(first);
        let at_first = siblings.iter().position(|&s| s == first)?;
        let at_last = siblings.iter().position(|&s| s == last)?;
        let (parent, slot, zone) = match movement {
            TreeMove::Indent => {
                let before = *siblings.get(at_first.checked_sub(1)?)?;
                (Some(before), Slot::Last, DropZone::Inside)
            }
            TreeMove::Outdent => {
                let parent = parent_of(first)?;
                (
                    shape.rows[parent].parent,
                    Slot::After(parent),
                    DropZone::After,
                )
            }
            TreeMove::Up => {
                at_first.checked_sub(1)?;
                let slot = match at_first.checked_sub(2) {
                    Some(two_up) => Slot::After(siblings[two_up]),
                    None => Slot::First,
                };
                (parent_of(first), slot, DropZone::Before)
            }
            TreeMove::Down => {
                let below = *siblings.get(at_last + 1)?;
                (parent_of(first), Slot::After(below), DropZone::After)
            }
        };
        let value = |row: usize| shape.rows[row].value.clone();
        let drop = TreeDrop {
            dragged: self.carried.clone(),
            parent: parent.map(value),
            slot: match slot {
                Slot::First => TreeSlot::First,
                Slot::Last => TreeSlot::Last,
                Slot::After(row) => TreeSlot::After(value(row)),
            },
            over: self.cursor.clone(),
            zone,
        };
        let placement = Placement {
            parent,
            slot,
            line_level: 0,
        };
        let carried_rows: Vec<usize> = self.carried.iter().filter_map(|c| shape.index(c)).collect();
        match judge(
            shape,
            &placement,
            &carried_rows,
            &drop,
            self.judge_drop.as_ref(),
        ) {
            DropVerdict::Allowed => Some(drop),
            DropVerdict::Refused => None,
        }
    }
}

/// How the app reads keys first ([`Tree::handle_key`]); `true` consumes the key.
pub type TreeKeyHandler = Rc<dyn Fn(&TreeKey) -> bool>;
/// The text type-ahead matches for a row ([`Tree::label_of`]).
pub type LabelOf = Rc<dyn Fn(&str) -> String>;

/// The class the cursor row carries; drawn while the tree has the keyboard.
pub(super) const CURSOR_CLASS: &str = "rinch-tree__node-content--cursor";
/// The class the tree carries while one of its rows has the keyboard.
pub(super) const FOCUSED_CLASS: &str = "rinch-tree--focused";
/// How far PageUp and PageDown move the cursor, in rows.
const PAGE_ROWS: usize = 10;
/// How long typed letters are remembered for type-ahead.
pub(super) const TYPE_AHEAD_MS: u32 = 700;

/// One tree's keyboard state.
#[derive(Default)]
pub(super) struct KeyState {
    /// The row drawn as the cursor, by value.
    pub(super) drawn: RefCell<Option<String>>,
    /// Whether one of the rows has the keyboard.
    pub(super) focused: Cell<bool>,
    /// The cursor row was removed while it had the keyboard (it was drawn
    /// again): the focus that fell from it is still the tree's.
    pub(super) lost_row: Cell<bool>,
    /// Letters typed for type-ahead, and the timer that forgets them.
    pub(super) typed: RefCell<String>,
    pub(super) typed_timer: RefCell<Option<TimeoutHandle>>,
    /// The cursor's rest timer, before `oncursor`.
    pub(super) rest_timer: RefCell<Option<TimeoutHandle>>,
    /// The tree's own list, for its focused class.
    pub(super) container: RefCell<Option<NodeHandle>>,
    /// The one row that is a Tab stop (`tabindex="0"`), by value.
    pub(super) stop: RefCell<Option<String>>,
}

/// A document-unique id for a row's group of children (its `aria-owns`).
pub(super) fn next_group_id() -> String {
    thread_local! {
        static NEXT: Cell<u64> = const { Cell::new(0) };
    }
    let n = NEXT.with(|next| {
        let n = next.get();
        next.set(n + 1);
        n
    });
    format!("rinch-tree-group-{n}")
}

impl Shape {
    /// The rows drawn, top to bottom, given what is expanded.
    pub(super) fn visible(&self, expanded: &HashSet<String>) -> Vec<usize> {
        fn walk(shape: &Shape, rows: &[usize], expanded: &HashSet<String>, out: &mut Vec<usize>) {
            for &row in rows {
                out.push(row);
                if shape.is_open(row, expanded) {
                    walk(shape, &shape.rows[row].children, expanded, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(self, &self.roots, expanded, &mut out);
        out
    }
}

/// What a navigation key does to the cursor, given the rows drawn.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Nav {
    /// Put the cursor on this row.
    To(usize),
    Expand(usize),
    Collapse(usize),
    /// Expand these rows (`*`: the cursor row's siblings).
    ExpandAll(Vec<usize>),
}

/// The tree's own answer to `key` with the cursor on `cursor`; `None` for a
/// key it does not handle.
pub(super) fn nav(
    shape: &Shape,
    expanded: &HashSet<String>,
    cursor: usize,
    key: &str,
) -> Option<Nav> {
    let visible = shape.visible(expanded);
    let at = visible.iter().position(|&r| r == cursor)?;
    let last = visible.len().checked_sub(1)?;
    let has_children = !shape.rows[cursor].children.is_empty();
    Some(match key {
        "ArrowDown" => Nav::To(visible[(at + 1).min(last)]),
        "ArrowUp" => Nav::To(visible[at.saturating_sub(1)]),
        "Home" => Nav::To(visible[0]),
        "End" => Nav::To(visible[last]),
        "PageDown" => Nav::To(visible[(at + PAGE_ROWS).min(last)]),
        "PageUp" => Nav::To(visible[at.saturating_sub(PAGE_ROWS)]),
        "ArrowRight" if has_children && !shape.is_open(cursor, expanded) => Nav::Expand(cursor),
        "ArrowRight" if has_children => Nav::To(shape.rows[cursor].children[0]),
        "ArrowRight" => return None,
        "ArrowLeft" if shape.is_open(cursor, expanded) => Nav::Collapse(cursor),
        "ArrowLeft" => Nav::To(shape.rows[cursor].parent?),
        "*" => Nav::ExpandAll(
            shape
                .siblings(cursor)
                .iter()
                .copied()
                .filter(|&s| !shape.rows[s].children.is_empty())
                .collect(),
        ),
        _ => return None,
    })
}

/// The next drawn row after `cursor` (wrapping) whose text starts with
/// `typed`, case-insensitively. Typing one letter again and again steps
/// through the rows that start with it.
pub(super) fn type_ahead(
    shape: &Shape,
    expanded: &HashSet<String>,
    cursor: usize,
    typed: &str,
    text: &dyn Fn(&str) -> String,
) -> Option<usize> {
    let visible = shape.visible(expanded);
    let at = visible.iter().position(|&r| r == cursor).unwrap_or(0);
    let typed = typed.to_lowercase();
    let mut chars = typed.chars();
    let first = chars.next()?;
    let repeated = chars.all(|c| c == first);
    let matches = |row: usize, prefix: &str| {
        text(&shape.rows[row].value)
            .to_lowercase()
            .starts_with(prefix)
    };
    let visible = &visible;
    // A longer prefix may still match the cursor row itself; one letter
    // moves on from it.
    let from = |start: usize| (0..visible.len()).map(move |i| visible[(start + i) % visible.len()]);
    if typed.chars().count() > 1 {
        if let Some(row) = from(at).find(|&r| matches(r, &typed)) {
            return Some(row);
        }
        if !repeated {
            return None;
        }
    }
    let one = first.to_string();
    from(at + 1).find(|&r| matches(r, &one))
}
