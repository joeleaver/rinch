//! Drag-and-drop types, and the visible-row shape a drag is judged against.

use super::*;

// =============================================================================
// Drag and drop
// =============================================================================

/// The part of a row the pointer is over while dragging: its top quarter,
/// its middle half, or its bottom quarter. A row that takes no children
/// ([`Tree::accepts_children`]) has no middle: its top half is before it
/// and its bottom half after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropZone {
    /// Before the row, among its siblings.
    Before,
    /// Into the row, as its last child.
    Inside,
    /// After the row: among its siblings, as the first child of an expanded
    /// row with children, or (at the end of a group, by the pointer's
    /// horizontal position) after one of its ancestors.
    After,
}

/// Where among a parent's children a drop lands. An anchor, not an index:
/// what the app does with it is its own business.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeSlot {
    /// First among the parent's children.
    First,
    /// Last among the parent's children.
    Last,
    /// Right after this child of the parent.
    After(String),
}

/// What a drop just below an expanded row with children means
/// ([`Tree::after_open_row`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AfterOpenRow {
    /// First inside the row, where its first child is drawn: the same place
    /// as just above that child.
    #[default]
    FirstChild,
    /// The pointer's indent column picks, as it picks the depth at the end of
    /// a group: one indent in from the row's own or further is first inside
    /// it (the indicator starts one level in); at the row's own indent or to
    /// its left is after the row among its siblings, after its whole subtree
    /// (the indicator starts at the row's indent). The indicator moves as
    /// the pointer moves sideways.
    ByColumn,
}

/// A drop the tree offers while a drag is over it, and delivers when the
/// pointer is released. `parent` and `slot` are where the dragged rows
/// would land; `over` and `zone` are the row under the pointer and which
/// part of it, for an app that describes the drop in words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeDrop {
    /// The values being dragged, in the order the drag started with.
    pub dragged: Vec<String>,
    /// The new parent's value; `None` for the top level.
    pub parent: Option<String>,
    /// Where among the new parent's children.
    pub slot: TreeSlot,
    /// The row under the pointer.
    pub over: String,
    /// Which part of that row.
    pub zone: DropZone,
}

/// Whether a drop may happen, as the app judges it ([`Tree::judge_drop`]).
/// A refused drop is drawn as refused and is not delivered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropVerdict {
    Allowed,
    Refused,
}

/// What a drag from a row carries ([`Tree::drag_values`]).
pub type DragValues = Rc<dyn Fn(&str) -> Vec<String>>;
/// Whether a row takes children by a drop ([`Tree::accepts_children`]).
pub type AcceptsChildren = Rc<dyn Fn(&TreeNodeData) -> bool>;
/// How the app judges a drop ([`Tree::judge_drop`]).
pub type JudgeDrop = Rc<dyn Fn(&TreeDrop) -> DropVerdict>;

/// The tree's rows as structure, for resolving a drop: each row's parent,
/// level and children, by index. Built from the data the tree last drew.
#[derive(Default)]
pub(super) struct Shape {
    pub(super) rows: Vec<ShapeRow>,
    by_value: HashMap<String, usize>,
    pub(super) roots: Vec<usize>,
}

pub(super) struct ShapeRow {
    pub(super) value: String,
    pub(super) parent: Option<usize>,
    pub(super) level: usize,
    pub(super) children: Vec<usize>,
    /// Whether a drop may put rows into this one ([`Tree::accepts_children`]).
    pub(super) accepts: bool,
    /// Whether the row is disabled: the keyboard's cursor can rest on it, but
    /// never selects it.
    pub(super) disabled: bool,
}

/// Where a drop lands, by row index, and where its indicator is drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Placement {
    pub(super) parent: Option<usize>,
    pub(super) slot: Slot,
    /// The level whose indent the indicator starts at.
    pub(super) line_level: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Slot {
    First,
    Last,
    After(usize),
}

impl Shape {
    pub(super) fn of(data: &[TreeNodeData], accepts: Option<&AcceptsChildren>) -> Self {
        fn walk(
            shape: &mut Shape,
            nodes: &[TreeNodeData],
            parent: Option<usize>,
            level: usize,
            accepts: Option<&AcceptsChildren>,
        ) -> Vec<usize> {
            let mut ids = Vec::with_capacity(nodes.len());
            for node in nodes {
                let id = shape.rows.len();
                shape.rows.push(ShapeRow {
                    value: node.value.clone(),
                    parent,
                    level,
                    children: Vec::new(),
                    accepts: accepts.is_none_or(|accepts| accepts(node)),
                    disabled: node.disabled,
                });
                shape.by_value.insert(node.value.clone(), id);
                ids.push(id);
                let children = walk(shape, &node.children, Some(id), level + 1, accepts);
                shape.rows[id].children = children;
            }
            ids
        }
        let mut shape = Shape::default();
        shape.roots = walk(&mut shape, data, None, 0, accepts);
        shape
    }

    pub(super) fn index(&self, value: &str) -> Option<usize> {
        self.by_value.get(value).copied()
    }

    pub(super) fn siblings(&self, row: usize) -> &[usize] {
        match self.rows[row].parent {
            Some(parent) => &self.rows[parent].children,
            None => &self.roots,
        }
    }

    fn prev_sibling(&self, row: usize) -> Option<usize> {
        let siblings = self.siblings(row);
        let at = siblings.iter().position(|&s| s == row)?;
        at.checked_sub(1).map(|before| siblings[before])
    }

    /// Whether `row` is `ancestor` or under it.
    pub(super) fn is_within(&self, mut row: usize, ancestor: usize) -> bool {
        loop {
            if row == ancestor {
                return true;
            }
            match self.rows[row].parent {
                Some(parent) => row = parent,
                None => return false,
            }
        }
    }

    pub(super) fn is_open(&self, row: usize, expanded: &HashSet<String>) -> bool {
        !self.rows[row].children.is_empty() && expanded.contains(&self.rows[row].value)
    }

    /// The row drawn right below `row`, given what is expanded.
    pub(super) fn next_visible(&self, row: usize, expanded: &HashSet<String>) -> Option<usize> {
        if self.is_open(row, expanded) {
            return self.rows[row].children.first().copied();
        }
        let mut at = row;
        loop {
            let siblings = self.siblings(at);
            let position = siblings.iter().position(|&s| s == at)?;
            if let Some(&next) = siblings.get(position + 1) {
                return Some(next);
            }
            at = self.rows[at].parent?;
        }
    }

    /// Whether `row` is drawn: every row above it is expanded.
    pub(super) fn is_shown(&self, mut row: usize, expanded: &HashSet<String>) -> bool {
        while let Some(parent) = self.rows[row].parent {
            if !expanded.contains(&self.rows[parent].value) {
                return false;
            }
            row = parent;
        }
        true
    }

    /// `row`'s ancestor at `level` (`row` itself at its own level).
    fn ancestor_at(&self, mut row: usize, level: usize) -> usize {
        while self.rows[row].level > level {
            match self.rows[row].parent {
                Some(parent) => row = parent,
                None => break,
            }
        }
        row
    }

    /// Where a drop over `over`'s `zone` lands. `pointer_level` is the indent
    /// column under the pointer, when the tree knows its indent in pixels: it
    /// picks the depth of an "after" at the end of a group, from `over`'s own
    /// level up to the level of the row drawn below it. `after_open` says
    /// what "after" an open row with children is ([`Tree::after_open_row`]).
    pub(super) fn placement(
        &self,
        over: usize,
        zone: DropZone,
        pointer_level: Option<usize>,
        expanded: &HashSet<String>,
        after_open: AfterOpenRow,
    ) -> Placement {
        let row = &self.rows[over];
        match zone {
            DropZone::Before => Placement {
                parent: row.parent,
                slot: self.prev_sibling(over).map_or(Slot::First, Slot::After),
                line_level: row.level,
            },
            DropZone::Inside => Placement {
                parent: Some(over),
                slot: Slot::Last,
                line_level: row.level,
            },
            // After an open row with children is first inside it (where its
            // first child is drawn), unless it takes no children: then it is
            // after it, at its own level. With `ByColumn` the pointer's
            // column picks between the two: one indent in from the row's own
            // or further, first inside; at the row's own indent or left of
            // it, after it among its siblings (after its whole subtree; its
            // first child is drawn below it, deeper, so no shallower depth is
            // offered). No column known: first inside.
            DropZone::After if row.accepts && self.is_open(over, expanded) => {
                let inside = match after_open {
                    AfterOpenRow::FirstChild => true,
                    AfterOpenRow::ByColumn => pointer_level.is_none_or(|level| level > row.level),
                };
                if inside {
                    Placement {
                        parent: Some(over),
                        slot: Slot::First,
                        line_level: row.level + 1,
                    }
                } else {
                    Placement {
                        parent: row.parent,
                        slot: Slot::After(over),
                        line_level: row.level,
                    }
                }
            }
            DropZone::After => {
                let deepest = row.level;
                let shallowest = self
                    .next_visible(over, expanded)
                    .map_or(0, |next| self.rows[next].level)
                    .min(deepest);
                let level = pointer_level.map_or(deepest, |level| level.clamp(shallowest, deepest));
                let anchor = self.ancestor_at(over, level);
                Placement {
                    parent: self.rows[anchor].parent,
                    slot: Slot::After(anchor),
                    line_level: level,
                }
            }
        }
    }
}

/// The zone of a row a pointer `fraction` of the way down it is over;
/// `accepts` is whether the row takes children (no middle zone when not).
pub(super) fn zone_at(fraction: f32, accepts: bool) -> DropZone {
    if !accepts {
        if fraction < 0.5 {
            DropZone::Before
        } else {
            DropZone::After
        }
    } else if fraction < 0.25 {
        DropZone::Before
    } else if fraction > 0.75 {
        DropZone::After
    } else {
        DropZone::Inside
    }
}

/// One tree's drag state, shared by every row it draws.
#[derive(Default)]
pub(super) struct DragState {
    /// The data the tree last drew, and its shape once a drag needed it.
    pub(super) data: RefCell<Vec<TreeNodeData>>,
    pub(super) shape: RefCell<Option<Rc<Shape>>>,
    /// Each row's content box, by value: where the indicator goes.
    pub(super) rows: RefCell<HashMap<String, NodeHandle>>,
    /// What the drag in progress carries; empty when none started here.
    pub(super) dragged: RefCell<Vec<String>>,
    /// The drop on offer, its placement and the app's verdict.
    pub(super) candidate: RefCell<Option<(TreeDrop, Placement, DropVerdict)>>,
    /// The row the indicator is drawn on, by value.
    pub(super) drawn: RefCell<Option<String>>,
    /// The row a hover-to-expand is waiting on.
    pub(super) hover: RefCell<Option<(String, TimeoutHandle)>>,
    /// Whether the app's last `ondropchange` report was a drop.
    pub(super) told: Cell<bool>,
    /// The pointer left the row on offer: unless the next row it reaches in
    /// the same move offers a drop, the app is told `None` at the source's
    /// drag move ([`withdraw`]).
    pub(super) pending_none: Cell<bool>,
    /// Pixels per level measured from the rows, when the tree was given no
    /// [`Tree::indent`]; once per drag.
    pub(super) level_px: Cell<Option<f32>>,
}

impl DragState {
    pub(super) fn shape(&self, accepts: Option<&AcceptsChildren>) -> Rc<Shape> {
        self.shape
            .borrow_mut()
            .get_or_insert_with(|| Rc::new(Shape::of(&self.data.borrow(), accepts)))
            .clone()
    }

    pub(super) fn cancel_hover(&self) {
        if let Some((_, handle)) = self.hover.borrow_mut().take() {
            clear_timeout(handle);
        }
    }
}
