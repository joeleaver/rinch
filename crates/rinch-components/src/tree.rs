//! Tree component.
//!
//! Hierarchical tree view with expand/collapse and selection.
//!
//! # Example
//!
//! ```ignore
//! let tree_data = vec![
//!     TreeNodeData::new("root", "Root Item")
//!         .with_children(vec![
//!             TreeNodeData::new("child1", "Child 1"),
//!             TreeNodeData::new("child2", "Child 2")
//!                 .with_children(vec![
//!                     TreeNodeData::new("grandchild", "Grandchild"),
//!                 ]),
//!         ]),
//! ];
//!
//! let tree_state = UseTreeReturn::new(UseTreeOptions::default());
//!
//! rsx! {
//!     Tree {
//!         data: tree_data,
//!         tree: Some(tree_state),
//!     }
//! }
//! ```

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ValueCallback;
use rinch_core::for_each_dom_typed;
use rinch_core::get_click_context;
use rinch_core::reactive::{Signal, untracked};
use rinch_core::timer::{TimeoutHandle, clear_timeout, set_timeout};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

/// The class selection adds to a tree row's content box.
const SELECTED_CLASS: &str = "rinch-tree__node-content--selected";
/// The class the expanded state adds to a tree row's chevron.
const CHEVRON_EXPANDED_CLASS: &str = "rinch-tree__chevron--expanded";
/// The class a drag adds to the rows it carries.
const DRAGGING_CLASS: &str = "rinch-tree__node-content--dragging";

// =============================================================================
// TreeNodeData
// =============================================================================

/// Represents a node in the tree data structure.
#[derive(Clone, Debug)]
pub struct TreeNodeData {
    /// Unique identifier for this node.
    pub value: String,
    /// Display label for this node.
    pub label: String,
    /// Child nodes.
    pub children: Vec<TreeNodeData>,
    /// Whether the node is disabled.
    pub disabled: bool,
    /// Optional icon to display.
    pub icon: Option<TablerIcon>,
    /// Optional payload data.
    pub payload: Option<Rc<dyn Any>>,
}

impl PartialEq for TreeNodeData {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
            && self.label == other.label
            && self.children == other.children
            && self.disabled == other.disabled
            && self.icon == other.icon
            && match (&self.payload, &other.payload) {
                (Some(a), Some(b)) => Rc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }
}

impl TreeNodeData {
    /// Create a new tree node with the given value and label.
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            children: Vec::new(),
            disabled: false,
            icon: None,
            payload: None,
        }
    }

    /// Add multiple children to this node.
    pub fn with_children(mut self, children: Vec<TreeNodeData>) -> Self {
        self.children = children;
        self
    }

    /// Add a single child to this node.
    pub fn with_child(mut self, child: TreeNodeData) -> Self {
        self.children.push(child);
        self
    }

    /// Set whether this node is disabled.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Set the icon for this node.
    pub fn with_icon(mut self, icon: TablerIcon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Attach custom payload data to this node.
    pub fn with_payload<T: Any + 'static>(mut self, payload: T) -> Self {
        self.payload = Some(Rc::new(payload));
        self
    }

    /// Check if this node has children.
    pub fn has_children(&self) -> bool {
        !self.children.is_empty()
    }

    /// Try to downcast the payload to a specific type.
    pub fn downcast_payload<T: Any + 'static>(&self) -> Option<&T> {
        self.payload.as_ref()?.downcast_ref::<T>()
    }
}

// =============================================================================
// Tree State Management
// =============================================================================

/// Options for initializing tree state.
#[derive(Clone, Default)]
pub struct UseTreeOptions {
    /// Initially expanded node values.
    pub initial_expanded: HashSet<String>,
    /// Initially selected node values.
    pub initial_selected: HashSet<String>,
    /// Whether multiple nodes can be selected.
    pub multiple: bool,
}

/// Tree state container. Create with `UseTreeReturn::new()`.
#[derive(Clone, Copy)]
pub struct UseTreeReturn {
    /// Signal containing expanded node values.
    pub expanded: Signal<HashSet<String>>,
    /// Signal containing selected node values.
    pub selected: Signal<HashSet<String>>,
    /// The row the keyboard is on ([`Tree::keyboard`]): moved by the arrow
    /// keys and type-ahead, and by a click. A cursor whose row the data
    /// drops moves to the row drawn below it (above it at the end).
    pub cursor: Signal<Option<String>>,
    /// Controller for programmatic tree manipulation.
    pub controller: TreeController,
}

impl UseTreeReturn {
    /// Create tree state.
    pub fn new(options: UseTreeOptions) -> Self {
        let expanded = Signal::new(options.initial_expanded);
        let selected = Signal::new(options.initial_selected);
        let cursor = Signal::new(None);
        let focus_requests = Signal::new(0u64);

        let controller = TreeController {
            expanded,
            selected,
            cursor,
            focus_requests,
            multiple: options.multiple,
        };

        Self {
            expanded,
            selected,
            cursor,
            controller,
        }
    }
}

/// Controller for programmatic tree state manipulation.
#[derive(Clone, Copy)]
pub struct TreeController {
    expanded: Signal<HashSet<String>>,
    selected: Signal<HashSet<String>>,
    cursor: Signal<Option<String>>,
    focus_requests: Signal<u64>,
    multiple: bool,
}

impl TreeController {
    /// Put the keyboard's cursor on a row. It does not give the tree the
    /// keyboard ([`focus`](Self::focus) does); when one of the tree's rows
    /// has it already, it moves to the cursor row.
    pub fn set_cursor(&self, value: &str) {
        self.cursor.set(Some(value.to_string()));
    }

    /// The row the keyboard's cursor is on.
    pub fn cursor(&self) -> Option<String> {
        self.cursor.get()
    }

    /// Give the tree the keyboard: its cursor row, or its first row when it
    /// has none ([`Tree::keyboard`]).
    pub fn focus(&self) {
        self.focus_requests.update(|n| *n += 1);
    }

    /// Expand a node by its value.
    pub fn expand(&self, value: &str) {
        self.expanded.update(|set| {
            set.insert(value.to_string());
        });
    }

    /// Collapse a node by its value.
    pub fn collapse(&self, value: &str) {
        self.expanded.update(|set| {
            set.remove(value);
        });
    }

    /// Toggle the expanded state of a node.
    pub fn toggle(&self, value: &str) {
        self.expanded.update(|set| {
            if set.contains(value) {
                set.remove(value);
            } else {
                set.insert(value.to_string());
            }
        });
    }

    /// Expand all nodes in the tree.
    pub fn expand_all(&self, data: &[TreeNodeData]) {
        self.expanded.update(|set| {
            fn collect(nodes: &[TreeNodeData], set: &mut HashSet<String>) {
                for node in nodes {
                    if node.has_children() {
                        set.insert(node.value.clone());
                        collect(&node.children, set);
                    }
                }
            }
            collect(data, set);
        });
    }

    /// Collapse all nodes.
    pub fn collapse_all(&self) {
        self.expanded.set(HashSet::new());
    }

    /// Check if a node is expanded.
    pub fn is_expanded(&self, value: &str) -> bool {
        self.expanded.get().contains(value)
    }

    /// Select a node by its value.
    pub fn select(&self, value: &str) {
        self.selected.update(|set| {
            if !self.multiple {
                set.clear();
            }
            set.insert(value.to_string());
        });
    }

    /// Deselect a node by its value.
    pub fn deselect(&self, value: &str) {
        self.selected.update(|set| {
            set.remove(value);
        });
    }

    /// Clear all selections.
    pub fn clear_selected(&self) {
        self.selected.set(HashSet::new());
    }

    /// Make exactly these rows the selection. [`Tree::onselectionchange`]
    /// is not called: it reports what clicks and keys do.
    pub fn set_selection(&self, values: impl IntoIterator<Item = String>) {
        self.selected.set(values.into_iter().collect());
    }

    /// Check if a node is selected.
    pub fn is_selected(&self, value: &str) -> bool {
        self.selected.get().contains(value)
    }
}

/// Generate initial expanded state from tree data.
///
/// Pass `&["*"]` to expand all nodes, or specific node values to expand.
///
/// # Example
///
/// ```ignore
/// // Expand all nodes
/// let expanded = get_tree_expanded_state(&data, &["*"]);
///
/// // Expand specific nodes
/// let expanded = get_tree_expanded_state(&data, &["root", "child1"]);
/// ```
pub fn get_tree_expanded_state(data: &[TreeNodeData], values: &[&str]) -> HashSet<String> {
    let expand_all = values.contains(&"*");
    let mut result = HashSet::new();

    fn collect(
        nodes: &[TreeNodeData],
        values: &[&str],
        expand_all: bool,
        result: &mut HashSet<String>,
    ) {
        for node in nodes {
            if node.has_children() {
                if expand_all || values.contains(&node.value.as_str()) {
                    result.insert(node.value.clone());
                }
                collect(&node.children, values, expand_all, result);
            }
        }
    }

    collect(data, values, expand_all, &mut result);
    result
}

// =============================================================================
// Custom Render Types
// =============================================================================

/// Payload passed to custom node render functions.
pub struct RenderTreeNodePayload<'a> {
    /// The node being rendered.
    pub node: &'a TreeNodeData,
    /// Signal for expanded state — create Effects to react to changes.
    pub expanded_signal: Signal<HashSet<String>>,
    /// Signal for selected state — create Effects to react to changes.
    pub selected_signal: Signal<HashSet<String>>,
    /// The value of this node (for checking against the signals).
    pub node_value: &'a str,
    /// Whether the node has children.
    pub has_children: bool,
    /// Nesting level (0 = root).
    pub level: usize,
}

impl<'a> RenderTreeNodePayload<'a> {
    /// Check if this node is currently expanded.
    pub fn is_expanded(&self) -> bool {
        self.expanded_signal.get().contains(self.node_value)
    }

    /// Check if this node is currently selected.
    pub fn is_selected(&self) -> bool {
        self.selected_signal.get().contains(self.node_value)
    }
}

/// Custom render function type for tree nodes.
pub type RenderTreeNode = Rc<dyn Fn(&RenderTreeNodePayload, &mut RenderScope) -> NodeHandle>;

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
struct Shape {
    rows: Vec<ShapeRow>,
    by_value: HashMap<String, usize>,
    roots: Vec<usize>,
}

struct ShapeRow {
    value: String,
    parent: Option<usize>,
    level: usize,
    children: Vec<usize>,
    /// Whether a drop may put rows into this one ([`Tree::accepts_children`]).
    accepts: bool,
    /// Whether the row is disabled: the keyboard's cursor can rest on it, but
    /// never selects it.
    disabled: bool,
}

/// Where a drop lands, by row index, and where its indicator is drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Placement {
    parent: Option<usize>,
    slot: Slot,
    /// The level whose indent the indicator starts at.
    line_level: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Slot {
    First,
    Last,
    After(usize),
}

impl Shape {
    fn of(data: &[TreeNodeData], accepts: Option<&AcceptsChildren>) -> Self {
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

    fn index(&self, value: &str) -> Option<usize> {
        self.by_value.get(value).copied()
    }

    fn siblings(&self, row: usize) -> &[usize] {
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
    fn is_within(&self, mut row: usize, ancestor: usize) -> bool {
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

    fn is_open(&self, row: usize, expanded: &HashSet<String>) -> bool {
        !self.rows[row].children.is_empty() && expanded.contains(&self.rows[row].value)
    }

    /// The row drawn right below `row`, given what is expanded.
    fn next_visible(&self, row: usize, expanded: &HashSet<String>) -> Option<usize> {
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
    fn is_shown(&self, mut row: usize, expanded: &HashSet<String>) -> bool {
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
    /// level up to the level of the row drawn below it.
    fn placement(
        &self,
        over: usize,
        zone: DropZone,
        pointer_level: Option<usize>,
        expanded: &HashSet<String>,
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
            // After an open row with children is first inside it, unless it
            // takes no children: then it is after it, at its own level.
            DropZone::After if row.accepts && self.is_open(over, expanded) => Placement {
                parent: Some(over),
                slot: Slot::First,
                line_level: row.level + 1,
            },
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
fn zone_at(fraction: f32, accepts: bool) -> DropZone {
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
struct DragState {
    /// The data the tree last drew, and its shape once a drag needed it.
    data: RefCell<Vec<TreeNodeData>>,
    shape: RefCell<Option<Rc<Shape>>>,
    /// Each row's content box, by value: where the indicator goes.
    rows: RefCell<HashMap<String, NodeHandle>>,
    /// What the drag in progress carries; empty when none started here.
    dragged: RefCell<Vec<String>>,
    /// The drop on offer, its placement and the app's verdict.
    candidate: RefCell<Option<(TreeDrop, Placement, DropVerdict)>>,
    /// The row the indicator is drawn on, by value.
    drawn: RefCell<Option<String>>,
    /// The row a hover-to-expand is waiting on.
    hover: RefCell<Option<(String, TimeoutHandle)>>,
    /// Whether the app's last `ondropchange` report was a drop.
    told: Cell<bool>,
    /// The pointer left the row on offer: unless the next row it reaches in
    /// the same move offers a drop, the app is told `None` at the source's
    /// drag move ([`withdraw`]).
    pending_none: Cell<bool>,
    /// Pixels per level measured from the rows, when the tree was given no
    /// [`Tree::indent`]; once per drag.
    level_px: Cell<Option<f32>>,
}

impl DragState {
    fn shape(&self, accepts: Option<&AcceptsChildren>) -> Rc<Shape> {
        self.shape
            .borrow_mut()
            .get_or_insert_with(|| Rc::new(Shape::of(&self.data.borrow(), accepts)))
            .clone()
    }

    fn cancel_hover(&self) {
        if let Some((_, handle)) = self.hover.borrow_mut().take() {
            clear_timeout(handle);
        }
    }
}

// =============================================================================
// Internal TreeConfig (shared across recursive renders)
// =============================================================================

struct TreeConfig {
    expand_on_click: bool,
    select_on_click: bool,
    render_node: Option<RenderTreeNode>,
    onselect: Option<ValueCallback<String>>,
    onexpand: Option<ValueCallback<String>>,
    oncollapse: Option<ValueCallback<String>>,
    draggable: bool,
    drag_values: Option<DragValues>,
    judge_drop: Option<JudgeDrop>,
    accepts_children: Option<AcceptsChildren>,
    ondropchange: Option<ValueCallback<Option<(TreeDrop, DropVerdict)>>>,
    ondrop: Option<ValueCallback<TreeDrop>>,
    ondragstart: Option<ValueCallback<Vec<String>>>,
    ondragend: Option<ValueCallback<()>>,
    /// Pixels per level, when the tree was given them ([`Tree::indent`]).
    indent: Option<f32>,
    hover_expand_ms: u32,
    guides: bool,
    drag: Rc<DragState>,
    keyboard: bool,
    onselectionchange: Option<ValueCallback<Vec<String>>>,
    handle_key: Option<TreeKeyHandler>,
    oncursor: Option<ValueCallback<String>>,
    cursor_delay_ms: u32,
    label_of: Option<LabelOf>,
    keys: KeyState,
}

// =============================================================================
// Tree Component
// =============================================================================

/// Tree view component for displaying hierarchical data.
///
/// # Example
///
/// ```ignore
/// let data = vec![
///     TreeNodeData::new("docs", "Documents")
///         .with_icon(TablerIcon::Folder)
///         .with_children(vec![
///             TreeNodeData::new("readme", "README.md").with_icon(TablerIcon::File),
///             TreeNodeData::new("license", "LICENSE").with_icon(TablerIcon::File),
///         ]),
/// ];
///
/// let tree = UseTreeReturn::new(UseTreeOptions::default());
///
/// rsx! {
///     Tree {
///         data: data,
///         tree: Some(tree),
///         expand_on_click: true,
///         select_on_click: true,
///         onselect: ValueCallback::new(|value| println!("Selected: {}", value)),
///     }
/// }
/// ```
pub struct Tree {
    /// Tree data to display.
    pub data: Vec<TreeNodeData>,
    /// Tree state (from `UseTreeReturn::new()`).
    pub tree: Option<UseTreeReturn>,
    /// Indentation per level (spacing scale: xs, sm, md, lg, xl or custom CSS).
    pub level_offset: String,
    /// Whether clicking a node expands/collapses it.
    pub expand_on_click: bool,
    /// Whether clicking a node selects it.
    pub select_on_click: bool,
    /// Custom render function for node content.
    pub render_node: Option<RenderTreeNode>,
    /// Callback when a node is selected.
    pub onselect: Option<ValueCallback<String>>,
    /// Callback when a node is expanded.
    pub onexpand: Option<ValueCallback<String>>,
    /// Callback when a node is collapsed.
    pub oncollapse: Option<ValueCallback<String>>,
    /// Reactive data source. When provided, root nodes update when this changes.
    pub data_source: Option<Rc<dyn Fn() -> Vec<TreeNodeData>>>,
    /// Indentation per level in pixels. Overrides `level_offset`. A drag at
    /// the end of a group chooses its depth by the pointer's horizontal
    /// position in units of this; without it, the tree measures the indent
    /// of its rendered rows (from `level_offset`) once per drag.
    pub indent: Option<f32>,
    /// Whether rows can be dragged to rearrange the tree (needs `tree`).
    /// While a drag is over the tree it shows where the rows would land: a
    /// line before or after a row, at the depth they would take, or an
    /// outline around a row they would go into.
    pub draggable: bool,
    /// What a drag from a row carries (its own value when not given), for
    /// an app that drags its whole selection.
    pub drag_values: Option<DragValues>,
    /// The app's judgement of each drop on offer. A refused drop is drawn in
    /// the error color and is not delivered. Drops into a dragged row's own
    /// subtree are always refused. Allowed when not given.
    pub judge_drop: Option<JudgeDrop>,
    /// Which rows can take children by a drop. A row that does not has no
    /// "inside" zone: its top half is before it, its bottom half after it
    /// (and it is never expanded by hovering). Every row takes children when
    /// not given.
    pub accepts_children: Option<AcceptsChildren>,
    /// The drop on offer and its verdict, each time it changes. `None` when
    /// no drop is on offer: over a row that offers none, once the pointer
    /// has left the rows, and when the drag ends. Moving from one row
    /// straight onto the next reports only the next row's drop.
    pub ondropchange: Option<ValueCallback<Option<(TreeDrop, DropVerdict)>>>,
    /// An allowed drop, when the pointer is released.
    pub ondrop: Option<ValueCallback<TreeDrop>>,
    /// A drag from a row started, carrying these values: for an app whose
    /// other drop targets take rows dragged out of the tree.
    pub ondragstart: Option<ValueCallback<Vec<String>>>,
    /// The drag ended, dropped anywhere or cancelled; after any drop.
    pub ondragend: Option<ValueCallback<()>>,
    /// How long a drag rests inside a collapsed row before it expands, in
    /// milliseconds; 0 never expands.
    pub hover_expand_ms: u32,
    /// Draw a faint vertical line down each expanded row's children.
    pub guides: bool,
    /// Keyboard navigation (needs `tree`): while one of the rows has the
    /// keyboard, the arrow keys move a cursor (Right expands or steps in,
    /// Left collapses or steps out), Home, End, PageUp and PageDown jump,
    /// `*` expands the cursor row's siblings, and typed letters find the next
    /// row that starts with them. The tree is one Tab stop: the cursor row
    /// (the first row before there is one) is `tabindex="0"`, every other
    /// row `-1`. In a single-select tree the selection follows the cursor as
    /// a click would select (only with `select_on_click`, never a disabled
    /// row, and `onselect` is told). In a `multiple` tree a plain arrow
    /// selects nothing; Space toggles the cursor row; Shift with a move key
    /// (and Shift+Space) selects the run from the anchor (the row last
    /// clicked, Ctrl-clicked or toggled) to the cursor row, replacing the
    /// selection; Ctrl+A selects the cursor row's siblings; Enter adds the
    /// cursor row as a click would, keeping the rest. All of these only with
    /// `select_on_click`, and none selects a disabled row (see
    /// [`Tree::onselectionchange`]).
    pub keyboard: bool,
    /// Every key the tree's rows get, before the tree's own; `true` consumes
    /// it. [`TreeKey::drop_for`] turns a key into a structural move.
    pub handle_key: Option<TreeKeyHandler>,
    /// The cursor came to rest on a row (after `cursor_delay_ms`): for an
    /// app that opens what the cursor is on, once, not on every row it
    /// passed.
    pub oncursor: Option<ValueCallback<String>>,
    /// How long the cursor rests on a row before `oncursor`, in
    /// milliseconds (120 by default); 0 reports every move at once.
    pub cursor_delay_ms: u32,
    /// The text type-ahead matches for a row (its label when not given).
    pub label_of: Option<LabelOf>,
    /// The selection, top to bottom, each time a click or a key changes it,
    /// in a `multiple` tree only (`UseTreeOptions::multiple`; a single-select
    /// tree tells `onselect`). There a plain click selects the row alone;
    /// Ctrl- (Cmd-) click adds or removes a row; Shift-click, Shift with a
    /// move key and Shift+Space select the run of drawn rows from the anchor
    /// (the row last clicked, Ctrl-clicked or toggled with Space; a hidden
    /// one counts as its shown ancestor) to that row, replacing the
    /// selection; Ctrl+A selects the cursor row's siblings; a collapse by a
    /// click or a key unselects the rows it hides. Disabled rows are never
    /// selected, and nothing is reported when the selection did not change.
    /// A selected value whose row left the data is not reported, though it
    /// stays in [`UseTreeReturn::selected`]: the tree does not prune the
    /// selection when the data changes.
    ///
    /// Differences from the WAI-ARIA tree pattern: Ctrl+A selects the cursor
    /// row's siblings, not every row; Shift with an arrow selects the run
    /// from the anchor (the desktop file-manager model) where the pattern's
    /// recommended model toggles the next row and keeps the rest; and
    /// Ctrl+Shift-click toggles the row (Ctrl wins) rather than adding the
    /// run.
    pub onselectionchange: Option<ValueCallback<Vec<String>>>,
}

impl std::fmt::Debug for Tree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tree")
            .field("data", &format!("{} nodes", self.data.len()))
            .field("level_offset", &self.level_offset)
            .field("expand_on_click", &self.expand_on_click)
            .field("select_on_click", &self.select_on_click)
            .finish()
    }
}

impl Default for Tree {
    fn default() -> Self {
        Self {
            data: Vec::new(),
            tree: None,
            level_offset: "md".into(),
            expand_on_click: true,
            select_on_click: false,
            render_node: None,
            onselect: None,
            onexpand: None,
            oncollapse: None,
            data_source: None,
            indent: None,
            draggable: false,
            drag_values: None,
            judge_drop: None,
            accepts_children: None,
            ondropchange: None,
            ondrop: None,
            ondragstart: None,
            ondragend: None,
            hover_expand_ms: 600,
            guides: false,
            keyboard: false,
            handle_key: None,
            oncursor: None,
            cursor_delay_ms: 120,
            label_of: None,
            onselectionchange: None,
        }
    }
}

impl Component for Tree {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { ul { class: "rinch-tree" } };
        container.set_attribute("role", "tree");
        if self.tree.is_some_and(|tree| tree.controller.multiple) {
            container.set_attribute("aria-multiselectable", "true");
        }
        if self.draggable && self.tree.is_some() {
            container.add_class("rinch-tree--draggable");
        }

        // Apply level offset CSS variable
        let offset = if self.level_offset.is_empty() {
            "md"
        } else {
            &self.level_offset
        };
        let offset_value = match offset {
            "xs" => "var(--rinch-spacing-xs)",
            "sm" => "var(--rinch-spacing-sm)",
            "md" => "var(--rinch-spacing-md)",
            "lg" => "var(--rinch-spacing-lg)",
            "xl" => "var(--rinch-spacing-xl)",
            custom => custom,
        };
        let offset_value = match self.indent {
            Some(px) => format!("{px}px"),
            None => offset_value.to_string(),
        };
        container.set_attribute("style", &format!("--tree-level-offset: {}", offset_value));

        // Build shared config
        let config = Rc::new(TreeConfig {
            expand_on_click: self.expand_on_click,
            select_on_click: self.select_on_click,
            render_node: self.render_node.clone(),
            onselect: self.onselect.clone(),
            onexpand: self.onexpand.clone(),
            oncollapse: self.oncollapse.clone(),
            draggable: self.draggable && self.tree.is_some(),
            drag_values: self.drag_values.clone(),
            judge_drop: self.judge_drop.clone(),
            accepts_children: self.accepts_children.clone(),
            ondropchange: self.ondropchange.clone(),
            ondrop: self.ondrop.clone(),
            ondragstart: self.ondragstart.clone(),
            ondragend: self.ondragend.clone(),
            indent: self.indent,
            hover_expand_ms: self.hover_expand_ms,
            guides: self.guides,
            drag: Rc::new(DragState::default()),
            keyboard: self.keyboard && self.tree.is_some(),
            onselectionchange: self.onselectionchange.clone(),
            handle_key: self.handle_key.clone(),
            oncursor: self.oncursor.clone(),
            cursor_delay_ms: self.cursor_delay_ms,
            label_of: self.label_of.clone(),
            keys: KeyState::default(),
        });

        if let Some(ref tree_state) = self.tree {
            let tree_state = *tree_state;
            let cfg = config.clone();
            if config.keyboard {
                wire_keyboard(__scope, &container, tree_state, &config);
            }

            // Use for_each_dom_typed for reactive root-level rendering
            let collection: Rc<dyn Fn() -> Vec<TreeNodeData>> =
                if let Some(ref data_source) = self.data_source {
                    data_source.clone()
                } else {
                    let data = self.data.clone();
                    Rc::new(move || data.clone())
                };
            // A drag resolves its drop against the data the rows were drawn
            // from, so the tree keeps the last of it.
            let drag = config.drag.clone();
            let draggable = config.draggable || config.keyboard || tree_state.controller.multiple;
            let keys_cfg = config.keyboard.then(|| Rc::downgrade(&config));

            for_each_dom_typed(
                __scope,
                &container,
                move || {
                    let data = collection();
                    if draggable {
                        let old = std::mem::replace(&mut *drag.data.borrow_mut(), data.clone());
                        drag.shape.borrow_mut().take();
                        if let Some(cfg) = keys_cfg.as_ref().and_then(|cfg| cfg.upgrade()) {
                            untracked(|| data_changed(&cfg, tree_state, &old));
                        }
                    }
                    data
                },
                |node: &TreeNodeData| node.value.clone(),
                move |node: TreeNodeData, __scope: &mut RenderScope| {
                    render_tree_node(__scope, &node, 0, tree_state, cfg.clone())
                },
            );
        } else {
            // No tree state — render statically
            for node in &self.data {
                let node_handle = render_tree_node_static(__scope, node, 0, &config);
                container.append_child(&node_handle);
            }
        }

        container
    }
}

// =============================================================================
// Render Functions
// =============================================================================

/// Render a single tree node with full reactive state.
fn render_tree_node(
    __scope: &mut RenderScope,
    node: &TreeNodeData,
    level: usize,
    tree_state: UseTreeReturn,
    config: Rc<TreeConfig>,
) -> NodeHandle {
    let expanded_signal = tree_state.expanded;
    let selected_signal = tree_state.selected;
    let has_children = node.has_children();
    let node_value = node.value.clone();

    // Create <li> wrapper
    let li = rinch_macros::rsx! { li { class: "rinch-tree__node" } };
    // The row's content box is the treeitem (it is what takes the keyboard,
    // so a screen reader announces the item and its state); the list item
    // around it is only structure.
    li.set_attribute("role", "none");
    li.set_attribute("data-value", &node.value);
    let content = rinch_macros::rsx! { div {} };
    content.set_attribute("role", "treeitem");
    content.set_attribute("aria-level", &(level + 1).to_string());
    if node.disabled {
        content.set_attribute("aria-disabled", "true");
    }

    // Reactive aria-expanded
    if has_children {
        let li_clone = content.clone();
        let nv = node_value.clone();
        __scope.create_effect(move || {
            let is_expanded = expanded_signal.get().contains(&nv);
            li_clone.set_attribute("aria-expanded", if is_expanded { "true" } else { "false" });
        });
    }

    // Reactive aria-selected
    {
        let li_clone = content.clone();
        let nv = node_value.clone();
        __scope.create_effect(move || {
            let is_selected = selected_signal.get().contains(&nv);
            if is_selected {
                li_clone.set_attribute("aria-selected", "true");
            } else {
                li_clone.set_attribute("aria-selected", "false");
            }
        });
    }

    // Create content wrapper.
    //
    // The base class is written **here**, not by the effect below. It used to
    // be the effect's job — the effect rewrote the whole `class` attribute on
    // every run, so its first run was also what established the node's identity
    // — and that is exactly the coupling issue #717 is about. The static
    // sibling path (`render_tree_node_static`) has always written it at
    // creation; this path now matches it.
    content.set_attribute(
        "class",
        if node.disabled {
            "rinch-tree__node-content rinch-tree__node-content--disabled"
        } else {
            "rinch-tree__node-content"
        },
    );
    // A keyboard tree is one Tab stop: the row its cursor is on, or its first
    // row (roving tabindex, `set_stop`). Every row of any other tree is one.
    let stop =
        !config.keyboard || config.keys.stop.borrow().as_deref() == Some(node_value.as_str());
    content.set_attribute("tabindex", if stop { "0" } else { "-1" });
    content.set_attribute(
        "style",
        &format!("padding-left: calc(var(--tree-level-offset) * {})", level),
    );

    if config.draggable || config.keyboard || tree_state.controller.multiple {
        // A row drawn again replaces the old box, which keeps nothing of a
        // drag. (No borrow of the tree's state is held across a DOM call
        // anywhere here: one can run pending effects, which can draw rows,
        // which come back here.)
        let old = config
            .drag
            .rows
            .borrow_mut()
            .insert(node_value.clone(), content.clone());
        if let Some(old) = old {
            clear_drawn(&old);
            old.remove_class(DRAGGING_CLASS);
            old.remove_class(CURSOR_CLASS);
        }
        // A row gone for good (not drawn again) leaves the map.
        let drag = Rc::downgrade(&config.drag);
        let value = node_value.clone();
        let mine = content.node_id();
        __scope.on_cleanup(move || {
            if let Some(drag) = drag.upgrade() {
                let mut rows = drag.rows.borrow_mut();
                if rows.get(&value).is_some_and(|row| row.node_id() == mine) {
                    rows.remove(&value);
                }
            }
        });
    }
    if config.draggable {
        wire_drag(__scope, &content, &node_value, tree_state, &config);
    }
    if config.keyboard {
        if untracked(|| tree_state.cursor.get()).as_deref() == Some(node_value.as_str()) {
            content.add_class(CURSOR_CLASS);
            *config.keys.drawn.borrow_mut() = Some(node_value.clone());
            // The tree is waiting for this row to give it back the keyboard.
            if config.keys.lost_row.get() {
                let cfg = Rc::downgrade(&config);
                let _ = set_timeout(0, move || {
                    if let Some(cfg) = cfg.upgrade() {
                        restore_keyboard(&cfg, tree_state);
                    }
                });
            }
        }
        // The cursor row removed while it has the keyboard (drawn again: its
        // data changed, or it moved): the focus falling from it stays the
        // tree's, and the row as drawn now takes it back (`wire_keyboard`).
        let cfg = Rc::downgrade(&config);
        let value = node_value.clone();
        __scope.on_cleanup(move || {
            if let Some(cfg) = cfg.upgrade() {
                let is_cursor =
                    untracked(|| tree_state.cursor.get()).as_deref() == Some(value.as_str());
                if cfg.keys.focused.get() && is_cursor {
                    cfg.keys.lost_row.set(true);
                    // A browser says nothing when the focused element goes,
                    // so the tree gives the keyboard back itself.
                    let weak = Rc::downgrade(&cfg);
                    let _ = set_timeout(0, move || {
                        if let Some(cfg) = weak.upgrade() {
                            restore_keyboard(&cfg, tree_state);
                        }
                    });
                }
            }
        });
    }

    // Reactive class for content (selection only — `disabled` is not reactive)
    {
        let content_clone = content.clone();
        let nv = node_value.clone();
        // Adding and removing the one class, never rewriting the attribute
        // (issue #717): `disabled` and the base class are written above and are
        // not this effect's business, so selection owns exactly one class — and
        // a rewrite would drop whatever a parent patched onto the row, which is
        // how every #474 category C prop travels.
        __scope.create_effect(move || {
            if selected_signal.get().contains(&nv) {
                content_clone.add_class(SELECTED_CLASS);
            } else {
                content_clone.remove_class(SELECTED_CLASS);
            }
        });
    }

    // Chevron (for expandable nodes)
    if has_children {
        // The base class is written here, not by the effect below — the same
        // repair the content box above took (issue #717), and what the static
        // sibling `render_tree_node_static` has always done.
        let chevron = rinch_macros::rsx! { span { class: "rinch-tree__chevron" } };
        let icon = crate::icons::chevron_right_dom(__scope);
        chevron.append_child(&icon);

        // Reactive chevron class: adds and removes the one modifier, never
        // rewriting the attribute (issue #717).
        let chevron_clone = chevron.clone();
        let nv = node_value.clone();
        __scope.create_effect(move || {
            if expanded_signal.get().contains(&nv) {
                chevron_clone.add_class(CHEVRON_EXPANDED_CLASS);
            } else {
                chevron_clone.remove_class(CHEVRON_EXPANDED_CLASS);
            }
        });

        // Dedicated click handler on chevron — always toggles expand/collapse
        // so that expand_on_click can be false while the chevron still works.
        {
            let controller = tree_state.controller;
            let value = node_value.clone();
            let onexpand = config.onexpand.clone();
            let oncollapse = config.oncollapse.clone();
            let cfg = config.clone();
            let handler_id = __scope.register_handler(move || {
                let was_expanded = expanded_signal.get().contains(&value);
                controller.toggle(&value);
                if !was_expanded {
                    if let Some(ref cb) = onexpand {
                        cb.invoke(value.clone());
                    }
                } else {
                    unselect_hidden(&cfg, tree_state, &value);
                    if let Some(ref cb) = oncollapse {
                        cb.invoke(value.clone());
                    }
                }
            });
            chevron.set_attribute("data-rid", &handler_id.0.to_string());
        }

        content.append_child(&chevron);
    } else {
        let spacer = rinch_macros::rsx! { span { class: "rinch-tree__spacer" } };
        content.append_child(&spacer);
    }

    // Render node icon if present
    if let Some(ref icon) = node.icon {
        let icon_wrapper = rinch_macros::rsx! { span { class: "rinch-tree__icon" } };
        let icon_el = render_tabler_icon(__scope, *icon, TablerIconStyle::Outline);
        icon_wrapper.append_child(&icon_el);
        content.append_child(&icon_wrapper);
    }

    // Label or custom render
    if let Some(ref render_fn) = config.render_node {
        let payload = RenderTreeNodePayload {
            node,
            expanded_signal,
            selected_signal,
            node_value: &node.value,
            has_children,
            level,
        };
        let custom = render_fn(&payload, __scope);
        content.append_child(&custom);
    } else {
        let label =
            rinch_macros::rsx! { span { class: "rinch-tree__label", {node.label.clone()} } };
        content.append_child(&label);
    }

    // Click handler
    if !node.disabled {
        let value = node.value.clone();
        let cfg = config.clone();
        let handler_id = __scope.register_handler(move || {
            if cfg.keyboard {
                tree_state.cursor.set(Some(value.clone()));
                set_focused(&cfg, true);
            }
            let modifiers = get_click_context().modifiers;
            let pick = if modifiers.ctrl || modifiers.meta {
                Pick::Toggle
            } else if modifiers.shift {
                Pick::Run
            } else {
                Pick::Replace
            };
            activate_row(&cfg, tree_state, &value, has_children, pick);
        });
        content.set_attribute("data-rid", &handler_id.0.to_string());
    }

    li.append_child(&content);

    // Children — always render, toggle visibility with display:none
    if has_children {
        let children_ul = rinch_macros::rsx! { ul { class: "rinch-tree__subtree" } };
        children_ul.set_attribute("role", "group");
        // The group is the treeitem's, though it sits beside it in the DOM.
        let group_id = next_group_id();
        children_ul.set_attribute("id", &group_id);
        content.set_attribute("aria-owns", &group_id);

        // With guides the children sit in a box with their guide beside
        // them, and the box is what shows and hides.
        let shown = if config.guides {
            let wrap = rinch_macros::rsx! { div { class: "rinch-tree__subtree-wrap" } };
            let guide = rinch_macros::rsx! { div { class: "rinch-tree__guide" } };
            guide.set_attribute(
                "style",
                &format!(
                    "left: calc(var(--tree-level-offset) * {level} + var(--tree-guide-offset))"
                ),
            );
            wrap.append_child(&guide);
            wrap.append_child(&children_ul);
            wrap
        } else {
            children_ul.clone()
        };

        // Reactive display toggle — preserves all child DOM and Effects
        let children_ul_vis = shown.clone();
        let nv = node_value.clone();
        __scope.create_effect(move || {
            if expanded_signal.get().contains(&nv) {
                children_ul_vis.set_style("display", "");
            } else {
                children_ul_vis.set_style("display", "none");
            }
        });

        // Render children using for_each_dom_typed for keyed reconciliation
        let children_data = node.children.clone();
        let child_level = level + 1;
        let cfg = config.clone();

        for_each_dom_typed(
            __scope,
            &children_ul,
            move || children_data.clone(),
            |node: &TreeNodeData| node.value.clone(),
            move |child_node: TreeNodeData, __scope: &mut RenderScope| {
                render_tree_node(__scope, &child_node, child_level, tree_state, cfg.clone())
            },
        );

        li.append_child(&shown);
    }

    li
}

/// Make `content` (the row of `value`) a drag source and a drop target.
fn wire_drag(
    __scope: &mut RenderScope,
    content: &NodeHandle,
    value: &str,
    tree_state: UseTreeReturn,
    config: &Rc<TreeConfig>,
) {
    content.set_attribute("draggable", "true");
    // A row redrawn while it is being dragged (its children arrived, say)
    // is still being dragged, and still shows the drop on offer.
    let carried = config.drag.dragged.borrow().iter().any(|d| d == value);
    if carried {
        content.add_class(DRAGGING_CLASS);
    }
    let drawn_here = config
        .drag
        .candidate
        .borrow()
        .as_ref()
        .filter(|(drop, _, _)| drop.over == value)
        .map(|(drop, placement, verdict)| (drop.zone, placement.line_level, *verdict));
    if let Some((zone, level, verdict)) = drawn_here {
        draw(content, zone, level, verdict);
    }

    let cfg = config.clone();
    let v = value.to_string();
    let start = __scope.register_handler(move || {
        let carried = carried_from(&cfg, tree_state, &v);
        // The tree shows the drag itself (the carried rows fade, the row
        // under the pointer draws the drop); the built-in snapshot of the
        // source row would sit over the very row the drop is drawn on.
        rinch_core::suppress_drag_ghost();
        mark_dragging(&cfg, &carried, true);
        cfg.drag.level_px.set(None);
        *cfg.drag.dragged.borrow_mut() = carried.clone();
        if let Some(cb) = &cfg.ondragstart {
            cb.invoke(carried);
        }
    });
    content.set_attribute("data-ondragstart", &start.0.to_string());

    let cfg = config.clone();
    let end = __scope.register_handler(move || end_drag(&cfg));
    content.set_attribute("data-ondragend", &end.0.to_string());

    let cfg = config.clone();
    let v = value.to_string();
    let over = __scope.register_handler(move || drag_over(&cfg, &v, tree_state));
    content.set_attribute("data-ondragover", &over.0.to_string());

    let cfg = config.clone();
    let v = value.to_string();
    let leave = __scope.register_handler(move || {
        let here = cfg
            .drag
            .candidate
            .borrow()
            .as_ref()
            .is_some_and(|(drop, _, _)| drop.over == v);
        if here {
            withdraw(&cfg);
        }
    });
    content.set_attribute("data-ondragleave", &leave.0.to_string());

    // Sent to the source row after every move, once the targets have heard
    // it: the moment to tell the app a withdrawn drop was not replaced.
    let cfg = config.clone();
    let moved = __scope.register_handler(move || report_withdrawn(&cfg));
    content.set_attribute("data-ondragmove", &moved.0.to_string());

    let cfg = config.clone();
    let drop = __scope.register_handler(move || {
        let candidate = cfg.drag.candidate.borrow().clone();
        offer(&cfg, None);
        if let Some((drop, _, DropVerdict::Allowed)) = candidate {
            if let Some(ondrop) = &cfg.ondrop {
                ondrop.invoke(drop);
            }
        }
        // The drop usually moves the dragged rows, and the source row a
        // drag's end is sent to may be gone by then: the drag ends here.
        end_drag(&cfg);
    });
    content.set_attribute("data-ondrop", &drop.0.to_string());
}

/// The drag is over, dropped or not: undim its rows, take down the drop on
/// offer, tell the app. Once: a drop ends it, and the source row's own end
/// (when the row is still there) finds nothing left to do.
fn end_drag(config: &Rc<TreeConfig>) {
    let carried = std::mem::take(&mut *config.drag.dragged.borrow_mut());
    if carried.is_empty() {
        return;
    }
    mark_dragging(config, &carried, false);
    offer(config, None);
    if let Some(cb) = &config.ondragend {
        cb.invoke(());
    }
}

/// Dim (or stop dimming) the rows a drag carries.
fn mark_dragging(config: &Rc<TreeConfig>, values: &[String], on: bool) {
    let rows: Vec<NodeHandle> = {
        let rows = config.drag.rows.borrow();
        values.iter().filter_map(|v| rows.get(v).cloned()).collect()
    };
    for row in &rows {
        if on {
            row.add_class(DRAGGING_CLASS);
        } else {
            row.remove_class(DRAGGING_CLASS);
        }
    }
}

/// A drag moved over the row of `value`: work out the drop on offer, show it,
/// and arm a hover-to-expand.
fn drag_over(config: &Rc<TreeConfig>, value: &str, tree_state: UseTreeReturn) {
    let dragged = config.drag.dragged.borrow().clone();
    if dragged.is_empty() {
        // Not a drag that started in this tree.
        return;
    }
    let shape = config.drag.shape(config.accepts_children.as_ref());
    let Some(over) = shape.index(value) else {
        return;
    };
    let dragged_rows: Vec<usize> = dragged.iter().filter_map(|d| shape.index(d)).collect();

    let ctx = get_click_context();
    let height = ctx.element_height.max(1.0);
    let zone = zone_at(
        (ctx.mouse_y - ctx.element_y) / height,
        shape.rows[over].accepts,
    );
    // Nothing is offered under a dragged row, or inside one. A dragged row's
    // own edges are places, which is how the last row of a group is dragged
    // out of it: left, over itself.
    let on_dragged_row = dragged_rows.contains(&over);
    let under_dragged = dragged_rows
        .iter()
        .any(|&d| d != over && shape.is_within(over, d));
    if under_dragged || (on_dragged_row && zone == DropZone::Inside) {
        offer(config, None);
        return;
    }
    let pointer_level = level_px(config, &shape, over)
        .filter(|px| *px > 0.0)
        .map(|px| ((ctx.mouse_x - ctx.element_x).max(0.0) / px) as usize);
    let expanded = untracked(|| tree_state.expanded.get());
    // The dragged rows are placed as if collapsed: their children travel
    // with them, so the row drawn below an expanded dragged row is the one
    // after its subtree, and its own bottom edge can take it out of its
    // group.
    let mut placing = expanded.clone();
    for d in &dragged {
        placing.remove(d);
    }
    let placement = shape.placement(over, zone, pointer_level, &placing);

    // Hover to expand: resting inside a collapsed row with children.
    let wants_hover = zone == DropZone::Inside
        && config.hover_expand_ms > 0
        && !shape.rows[over].children.is_empty()
        && !expanded.contains(value);
    let waiting_here = config
        .drag
        .hover
        .borrow()
        .as_ref()
        .is_some_and(|(v, _)| v == value);
    if !wants_hover {
        config.drag.cancel_hover();
    } else if !waiting_here {
        config.drag.cancel_hover();
        let v = value.to_string();
        let onexpand = config.onexpand.clone();
        let cfg = Rc::downgrade(config);
        let handle = set_timeout(config.hover_expand_ms, move || {
            let Some(cfg) = cfg.upgrade() else { return };
            cfg.drag.hover.borrow_mut().take();
            if cfg.drag.dragged.borrow().is_empty() {
                return;
            }
            tree_state.controller.expand(&v);
            if let Some(cb) = &onexpand {
                cb.invoke(v.clone());
            }
        });
        *config.drag.hover.borrow_mut() = Some((value.to_string(), handle));
    }

    let unchanged = config
        .drag
        .candidate
        .borrow()
        .as_ref()
        .is_some_and(|(drop, p, _)| drop.over == value && drop.zone == zone && *p == placement);
    if unchanged {
        return;
    }
    let row_value = |row: usize| shape.rows[row].value.clone();
    let drop = TreeDrop {
        dragged,
        parent: placement.parent.map(row_value),
        slot: match placement.slot {
            Slot::First => TreeSlot::First,
            Slot::Last => TreeSlot::Last,
            Slot::After(row) => TreeSlot::After(row_value(row)),
        },
        over: value.to_string(),
        zone,
    };
    let verdict = judge(
        &shape,
        &placement,
        &dragged_rows,
        &drop,
        config.judge_drop.as_ref(),
    );
    offer(config, Some((drop, placement, verdict)));
}

/// The verdict on a drop, for a drag and a keyboard move alike
/// ([`TreeKey::drop_for`]): refused when it would put a row into its own
/// subtree, or into a row that takes no children ([`Tree::accepts_children`]),
/// whatever the app says; otherwise the app's ([`Tree::judge_drop`]). The
/// places a drag offers never reach the first two cases (rows under a dragged
/// row offer nothing, a dragged row is placed as if collapsed, and a row that
/// takes no children has no inside zone); the keyboard's can.
fn judge(
    shape: &Shape,
    placement: &Placement,
    dragged_rows: &[usize],
    drop: &TreeDrop,
    judge_drop: Option<&JudgeDrop>,
) -> DropVerdict {
    let cycle = placement
        .parent
        .is_some_and(|parent| dragged_rows.iter().any(|&d| shape.is_within(parent, d)));
    let refused_inside = drop.zone == DropZone::Inside
        && placement
            .parent
            .is_some_and(|parent| !shape.rows[parent].accepts);
    if cycle || refused_inside {
        DropVerdict::Refused
    } else {
        judge_drop.map_or(DropVerdict::Allowed, |judge| judge(drop))
    }
}

/// Pixels per level, for picking a depth by the pointer's column: the
/// tree's [`Tree::indent`], else measured from the rendered rows once per
/// drag. A row's first child (its chevron or spacer) starts its level's
/// indent into the row, so the difference between `over`'s and a top-level
/// row's, over `over`'s level, is one level's width. `None` over a top-level
/// row before anything was measured (a depth is then never asked for).
fn level_px(config: &Rc<TreeConfig>, shape: &Shape, over: usize) -> Option<f32> {
    if config.indent.is_some() {
        return config.indent;
    }
    if let Some(px) = config.drag.level_px.get() {
        return Some(px);
    }
    let level = shape.rows[over].level;
    let root = *shape.roots.first()?;
    if level == 0 {
        return None;
    }
    let inset = |row: usize| -> Option<f32> {
        let handle = config
            .drag
            .rows
            .borrow()
            .get(&shape.rows[row].value)
            .cloned()?;
        let first = handle.children().into_iter().next()?;
        first.get_layout_bounds().map(|(x, _, _, _)| x)
    };
    let px = (inset(over)? - inset(root)?) / level as f32;
    config.drag.level_px.set(Some(px));
    Some(px)
}

/// The indicator's classes on a row: a line (`before`, `after`) or an
/// outline (`inside`), drawn by the stylesheet's `::after` at the indent of
/// `--tree-drop-level`. On the row itself, so a row redrawn mid-drag (its
/// children arrived) is simply drawn again, never stranded.
const DROP_CLASSES: [&str; 4] = [
    "rinch-tree__node-content--drop-before",
    "rinch-tree__node-content--drop-inside",
    "rinch-tree__node-content--drop-after",
    "rinch-tree__node-content--drop-refused",
];

fn draw(row: &NodeHandle, zone: DropZone, line_level: usize, verdict: DropVerdict) {
    clear_drawn(row);
    row.add_class(match zone {
        DropZone::Before => DROP_CLASSES[0],
        DropZone::Inside => DROP_CLASSES[1],
        DropZone::After => DROP_CLASSES[2],
    });
    if verdict == DropVerdict::Refused {
        row.add_class(DROP_CLASSES[3]);
    }
    row.set_style("--tree-drop-level", &line_level.to_string());
}

fn clear_drawn(row: &NodeHandle) {
    for class in DROP_CLASSES {
        row.remove_class(class);
    }
}

/// Make `candidate` the drop on offer: draw it, tell the app.
fn offer(config: &Rc<TreeConfig>, candidate: Option<(TreeDrop, Placement, DropVerdict)>) {
    let drag = &config.drag;
    drag.pending_none.set(false);
    if candidate.is_none() {
        drag.cancel_hover();
        if drag.candidate.borrow().is_none() && !drag.told.get() {
            return;
        }
    }
    take_down(drag);
    if let Some((drop, placement, verdict)) = &candidate {
        let row = drag.rows.borrow().get(&drop.over).cloned();
        if let Some(row) = row {
            *drag.drawn.borrow_mut() = Some(drop.over.clone());
            draw(&row, drop.zone, placement.line_level, *verdict);
        }
    }
    let told = candidate
        .as_ref()
        .map(|(drop, _, verdict)| (drop.clone(), *verdict));
    *drag.candidate.borrow_mut() = candidate;
    drag.told.set(told.is_some());
    if let Some(cb) = &config.ondropchange {
        cb.invoke(told);
    }
}

/// Clear the indicator from the row it is drawn on.
fn take_down(drag: &DragState) {
    let previous = drag.drawn.borrow_mut().take();
    let previous = previous.and_then(|value| drag.rows.borrow().get(&value).cloned());
    if let Some(row) = previous {
        clear_drawn(&row);
    }
}

/// The pointer left the row on offer. Its drop is no longer on offer, but
/// the app is not told yet: both backends send the old row's leave before
/// the new row's over, so a move from one row onto the next would report
/// `None` between the two. The source row's drag move, which follows every
/// move's targets, reports it if nothing replaced it ([`report_withdrawn`]).
fn withdraw(config: &Rc<TreeConfig>) {
    let drag = &config.drag;
    drag.cancel_hover();
    take_down(drag);
    drag.candidate.borrow_mut().take();
    drag.pending_none.set(true);
}

/// Tell the app a withdrawn drop was not replaced by another.
fn report_withdrawn(config: &Rc<TreeConfig>) {
    let drag = &config.drag;
    if !drag.pending_none.replace(false) || !drag.told.replace(false) {
        return;
    }
    if let Some(cb) = &config.ondropchange {
        cb.invoke(None);
    }
}

/// Render a tree node without reactive state (fallback when no tree state provided).
#[allow(clippy::only_used_in_recursion)]
fn render_tree_node_static(
    __scope: &mut RenderScope,
    node: &TreeNodeData,
    level: usize,
    config: &Rc<TreeConfig>,
) -> NodeHandle {
    let has_children = node.has_children();

    let li = rinch_macros::rsx! { li { class: "rinch-tree__node" } };
    li.set_attribute("role", "none");
    li.set_attribute("data-value", &node.value);

    let content = rinch_macros::rsx! { div {} };
    content.set_attribute("role", "treeitem");
    content.set_attribute("aria-level", &(level + 1).to_string());
    if node.disabled {
        content.set_attribute("aria-disabled", "true");
    }
    let base_class = if node.disabled {
        "rinch-tree__node-content rinch-tree__node-content--disabled"
    } else {
        "rinch-tree__node-content"
    };
    content.set_attribute("class", base_class);
    content.set_attribute(
        "style",
        &format!("padding-left: calc(var(--tree-level-offset) * {})", level),
    );
    content.set_attribute("tabindex", "0");

    if has_children {
        let chevron = rinch_macros::rsx! { span { class: "rinch-tree__chevron" } };
        let icon = crate::icons::chevron_right_dom(__scope);
        chevron.append_child(&icon);
        content.append_child(&chevron);
    } else {
        let spacer = rinch_macros::rsx! { span { class: "rinch-tree__spacer" } };
        content.append_child(&spacer);
    }

    if let Some(ref icon) = node.icon {
        let icon_wrapper = rinch_macros::rsx! { span { class: "rinch-tree__icon" } };
        let icon_el = render_tabler_icon(__scope, *icon, TablerIconStyle::Outline);
        icon_wrapper.append_child(&icon_el);
        content.append_child(&icon_wrapper);
    }

    let label = rinch_macros::rsx! { span { class: "rinch-tree__label", {node.label.clone()} } };
    content.append_child(&label);

    li.append_child(&content);

    if has_children {
        let children_ul = rinch_macros::rsx! { ul { class: "rinch-tree__subtree" } };
        children_ul.set_attribute("role", "group");
        let group_id = next_group_id();
        children_ul.set_attribute("id", &group_id);
        content.set_attribute("aria-owns", &group_id);

        for child in &node.children {
            let child_handle = render_tree_node_static(__scope, child, level + 1, config);
            children_ul.append_child(&child_handle);
        }

        li.append_child(&children_ul);
    }

    li
}

#[cfg(test)]
mod drop_tests {
    use super::*;

    /// a
    /// ├ a1
    /// │ ├ a1x
    /// │ └ a1y
    /// └ a2
    /// b
    /// c
    /// └ c1
    fn data() -> Vec<TreeNodeData> {
        vec![
            TreeNodeData::new("a", "a").with_children(vec![
                TreeNodeData::new("a1", "a1").with_children(vec![
                    TreeNodeData::new("a1x", "a1x"),
                    TreeNodeData::new("a1y", "a1y"),
                ]),
                TreeNodeData::new("a2", "a2"),
            ]),
            TreeNodeData::new("b", "b"),
            TreeNodeData::new("c", "c").with_child(TreeNodeData::new("c1", "c1")),
        ]
    }

    fn expanded(values: &[&str]) -> HashSet<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    /// The placement as values: (parent, slot, line level).
    fn place(
        shape: &Shape,
        over: &str,
        zone: DropZone,
        pointer: Option<usize>,
        open: &HashSet<String>,
    ) -> (Option<String>, TreeSlot, usize) {
        let p = shape.placement(shape.index(over).unwrap(), zone, pointer, open);
        let name = |row: usize| shape.rows[row].value.clone();
        let slot = match p.slot {
            Slot::First => TreeSlot::First,
            Slot::Last => TreeSlot::Last,
            Slot::After(row) => TreeSlot::After(name(row)),
        };
        (p.parent.map(name), slot, p.line_level)
    }

    fn after(v: &str) -> TreeSlot {
        TreeSlot::After(v.to_string())
    }

    #[test]
    fn zones_split_a_row_into_quarter_half_quarter() {
        assert_eq!(zone_at(0.0, true), DropZone::Before);
        assert_eq!(zone_at(0.24, true), DropZone::Before);
        assert_eq!(zone_at(0.5, true), DropZone::Inside);
        assert_eq!(zone_at(0.76, true), DropZone::After);
        assert_eq!(zone_at(1.0, true), DropZone::After);
        // Review of #1589: off the fixed points, so a half/half split fails.
        assert_eq!(zone_at(0.3, true), DropZone::Inside);
        assert_eq!(zone_at(0.7, true), DropZone::Inside);
    }

    #[test]
    fn before_lands_after_the_previous_sibling_or_first() {
        let shape = Shape::of(&data(), None);
        let open = expanded(&["a", "a1", "c"]);
        assert_eq!(
            place(&shape, "a2", DropZone::Before, None, &open),
            (Some("a".into()), after("a1"), 1)
        );
        assert_eq!(
            place(&shape, "a1", DropZone::Before, None, &open),
            (Some("a".into()), TreeSlot::First, 1)
        );
        assert_eq!(
            place(&shape, "a", DropZone::Before, None, &open),
            (None, TreeSlot::First, 0)
        );
    }

    #[test]
    fn inside_lands_last() {
        let shape = Shape::of(&data(), None);
        let open = expanded(&[]);
        assert_eq!(
            place(&shape, "b", DropZone::Inside, None, &open),
            (Some("b".into()), TreeSlot::Last, 0)
        );
        assert_eq!(
            place(&shape, "a", DropZone::Inside, None, &open),
            (Some("a".into()), TreeSlot::Last, 0)
        );
    }

    #[test]
    fn after_an_open_row_with_children_lands_first_inside_it() {
        let shape = Shape::of(&data(), None);
        let open = expanded(&["a"]);
        assert_eq!(
            place(&shape, "a", DropZone::After, None, &open),
            (Some("a".into()), TreeSlot::First, 1)
        );
        // Closed, the same zone is after it among its siblings.
        let closed = expanded(&[]);
        assert_eq!(
            place(&shape, "a", DropZone::After, None, &closed),
            (None, after("a"), 0)
        );
    }

    /// The drag that outdents: after the last row of a group, the pointer's
    /// indent column picks the depth, between the row's own level and the
    /// level of the row drawn below it.
    #[test]
    fn after_the_end_of_a_group_takes_its_depth_from_the_pointer() {
        let shape = Shape::of(&data(), None);
        let open = expanded(&["a", "a1"]);
        // a1y is the last of a1, which is not the last of a; below it is a2
        // (level 1), so levels 1..=2 are on offer.
        assert_eq!(
            place(&shape, "a1y", DropZone::After, Some(9), &open),
            (Some("a1".into()), after("a1y"), 2)
        );
        assert_eq!(
            place(&shape, "a1y", DropZone::After, Some(2), &open),
            (Some("a1".into()), after("a1y"), 2)
        );
        assert_eq!(
            place(&shape, "a1y", DropZone::After, Some(1), &open),
            (Some("a".into()), after("a1"), 1)
        );
        assert_eq!(
            place(&shape, "a1y", DropZone::After, Some(0), &open),
            (Some("a".into()), after("a1"), 1)
        );
        // Without a pointer column, the row's own depth.
        assert_eq!(
            place(&shape, "a1y", DropZone::After, None, &open),
            (Some("a1".into()), after("a1y"), 2)
        );
        // a2 ends a, and b (level 0) is below: level 0 means after a.
        assert_eq!(
            place(&shape, "a2", DropZone::After, Some(0), &open),
            (None, after("a"), 0)
        );
        // The last row of the tree can go all the way out.
        let open = expanded(&["c"]);
        assert_eq!(
            place(&shape, "c1", DropZone::After, Some(0), &open),
            (None, after("c"), 0)
        );
        // A row in the middle of its siblings offers only its own level.
        let open = expanded(&["a", "a1"]);
        assert_eq!(
            place(&shape, "a1x", DropZone::After, Some(0), &open),
            (Some("a1".into()), after("a1x"), 2)
        );
    }

    #[test]
    fn next_visible_follows_what_is_expanded() {
        let shape = Shape::of(&data(), None);
        let next = |v: &str, open: &[&str]| {
            shape
                .next_visible(shape.index(v).unwrap(), &expanded(open))
                .map(|r| shape.rows[r].value.clone())
        };
        assert_eq!(next("a", &[]), Some("b".into()));
        assert_eq!(next("a", &["a"]), Some("a1".into()));
        assert_eq!(next("a1y", &["a", "a1"]), Some("a2".into()));
        assert_eq!(next("a2", &["a"]), Some("b".into()));
        assert_eq!(next("c1", &["c"]), None);
    }

    #[test]
    fn within_is_the_row_or_under_it() {
        let shape = Shape::of(&data(), None);
        let i = |v: &str| shape.index(v).unwrap();
        assert!(shape.is_within(i("a1x"), i("a")));
        assert!(shape.is_within(i("a"), i("a")));
        assert!(!shape.is_within(i("a"), i("a1")));
        assert!(!shape.is_within(i("b"), i("a")));
    }

    /// A row that takes no children has no middle: half before, half after.
    #[test]
    fn a_row_that_takes_no_children_splits_in_half() {
        assert_eq!(zone_at(0.3, false), DropZone::Before);
        assert_eq!(zone_at(0.49, false), DropZone::Before);
        assert_eq!(zone_at(0.5, false), DropZone::After);
        assert_eq!(zone_at(0.7, false), DropZone::After);
    }

    /// The predicate reaches the shape, and after an open row that takes no
    /// children is after it, not first inside it.
    #[test]
    fn after_an_open_row_that_takes_no_children_is_after_it() {
        let only_a: AcceptsChildren = Rc::new(|node: &TreeNodeData| node.value == "a");
        let shape = Shape::of(&data(), Some(&only_a));
        assert!(shape.rows[shape.index("a").unwrap()].accepts);
        assert!(!shape.rows[shape.index("c").unwrap()].accepts);
        let open = expanded(&["a", "c"]);
        assert_eq!(
            place(&shape, "a", DropZone::After, None, &open),
            (Some("a".into()), TreeSlot::First, 1)
        );
        assert_eq!(
            place(&shape, "c", DropZone::After, None, &open),
            (None, after("c"), 0)
        );
    }

    /// A drop into a dragged row's own subtree is refused whatever the app
    /// says. No drag reaches it any more (the guard); it is pinned here.
    #[test]
    fn a_drop_into_a_dragged_rows_subtree_is_refused() {
        let shape = Shape::of(&data(), None);
        let i = |v: &str| shape.index(v).unwrap();
        let drop = TreeDrop {
            dragged: vec!["a".into()],
            parent: Some("a1".into()),
            slot: TreeSlot::Last,
            over: "a1".into(),
            zone: DropZone::Inside,
        };
        let allow: JudgeDrop = Rc::new(|_: &TreeDrop| DropVerdict::Allowed);
        let into = Placement {
            parent: Some(i("a1")),
            slot: Slot::Last,
            line_level: 1,
        };
        assert_eq!(
            judge(&shape, &into, &[i("a")], &drop, Some(&allow)),
            DropVerdict::Refused
        );
        assert_eq!(
            judge(&shape, &into, &[i("a")], &drop, None),
            DropVerdict::Refused
        );
        // Positive control: the same drop of a row from elsewhere is the app's.
        assert_eq!(
            judge(&shape, &into, &[i("b")], &drop, Some(&allow)),
            DropVerdict::Allowed
        );
        let refuse: JudgeDrop = Rc::new(|_: &TreeDrop| DropVerdict::Refused);
        assert_eq!(
            judge(&shape, &into, &[i("b")], &drop, Some(&refuse)),
            DropVerdict::Refused
        );
    }
}

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
    shape: Rc<Shape>,
    carried: Vec<String>,
    judge_drop: Option<JudgeDrop>,
}

impl TreeKey {
    /// What the key acts on: the selection top to bottom when the cursor
    /// row is one of several selected rows (rows inside other selected rows
    /// left out), the cursor row alone otherwise; through `drag_values`.
    pub fn carried(&self) -> &[String] {
        &self.carried
    }

    /// The drop that would make `movement` of the cursor row (what a drag of
    /// it would carry, [`Tree::drag_values`]), for the app to deliver as it
    /// does a dropped one. `None` when there is nowhere to go (no previous
    /// sibling to indent into, no parent to outdent from, no sibling above
    /// or below), and whenever a drag would refuse the same drop: into a row
    /// that takes no children ([`Tree::accepts_children`]), into a carried
    /// row or its subtree, or refused by [`Tree::judge_drop`].
    pub fn drop_for(&self, movement: TreeMove) -> Option<TreeDrop> {
        let shape = &self.shape;
        // The rows carried (several when the cursor row is in a selection)
        // move as one block: they must share a parent.
        let rows: Vec<usize> = self.carried.iter().filter_map(|v| shape.index(v)).collect();
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
const CURSOR_CLASS: &str = "rinch-tree__node-content--cursor";
/// The class the tree carries while one of its rows has the keyboard.
const FOCUSED_CLASS: &str = "rinch-tree--focused";
/// How far PageUp and PageDown move the cursor, in rows.
const PAGE_ROWS: usize = 10;
/// How long typed letters are remembered for type-ahead.
const TYPE_AHEAD_MS: u32 = 700;

/// One tree's keyboard state.
#[derive(Default)]
struct KeyState {
    /// The row drawn as the cursor, by value.
    drawn: RefCell<Option<String>>,
    /// Where a run of selected rows starts: the row last clicked,
    /// Ctrl-clicked or toggled with Space (the most recently selected row),
    /// not one the cursor merely moved to.
    anchor: RefCell<Option<String>>,
    /// Whether one of the rows has the keyboard.
    focused: Cell<bool>,
    /// The cursor row was removed while it had the keyboard (it was drawn
    /// again): the focus that fell from it is still the tree's.
    lost_row: Cell<bool>,
    /// Letters typed for type-ahead, and the timer that forgets them.
    typed: RefCell<String>,
    typed_timer: RefCell<Option<TimeoutHandle>>,
    /// The cursor's rest timer, before `oncursor`.
    rest_timer: RefCell<Option<TimeoutHandle>>,
    /// The tree's own list, for its focused class.
    container: RefCell<Option<NodeHandle>>,
    /// The one row that is a Tab stop (`tabindex="0"`), by value.
    stop: RefCell<Option<String>>,
}

/// A document-unique id for a row's group of children (its `aria-owns`).
fn next_group_id() -> String {
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
    fn visible(&self, expanded: &HashSet<String>) -> Vec<usize> {
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
enum Nav {
    /// Put the cursor on this row.
    To(usize),
    Expand(usize),
    Collapse(usize),
    /// Expand these rows (`*`: the cursor row's siblings).
    ExpandAll(Vec<usize>),
}

/// The tree's own answer to `key` with the cursor on `cursor`; `None` for a
/// key it does not handle.
fn nav(shape: &Shape, expanded: &HashSet<String>, cursor: usize, key: &str) -> Option<Nav> {
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
fn type_ahead(
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

/// Draw the cursor on the row of `value` (and off the one it was on).
fn draw_cursor(config: &Rc<TreeConfig>, value: Option<&str>) {
    if config.keys.drawn.borrow().as_deref() == value {
        // A row drawn again draws its own cursor (`render_tree_node`).
        return;
    }
    let previous = config.keys.drawn.borrow_mut().take();
    let previous = previous.and_then(|v| config.drag.rows.borrow().get(&v).cloned());
    if let Some(row) = previous {
        row.remove_class(CURSOR_CLASS);
    }
    if let Some(value) = value {
        let row = config.drag.rows.borrow().get(value).cloned();
        if let Some(row) = row {
            *config.keys.drawn.borrow_mut() = Some(value.to_string());
            row.add_class(CURSOR_CLASS);
        }
    }
}

/// Make the row of `value` the tree's one Tab stop (roving tabindex): it
/// gets `tabindex="0"`, the one before it `-1`. A row not drawn yet takes it
/// when it is (`render_tree_node`).
fn set_stop(config: &Rc<TreeConfig>, value: Option<String>) {
    let old = config.keys.stop.replace(value.clone());
    if old == value {
        return;
    }
    let (old, new) = {
        let rows = config.drag.rows.borrow();
        (
            old.and_then(|v| rows.get(&v).cloned()),
            value.and_then(|v| rows.get(&v).cloned()),
        )
    };
    if let Some(old) = old {
        old.set_attribute("tabindex", "-1");
    }
    if let Some(new) = new {
        new.set_attribute("tabindex", "0");
    }
}

/// The Tab stop is the cursor row while it is shown, else the first row.
fn refresh_stop(config: &Rc<TreeConfig>, tree_state: UseTreeReturn) {
    let cursor = untracked(|| tree_state.cursor.get());
    let expanded = untracked(|| tree_state.expanded.get());
    let shape = config.drag.shape(config.accepts_children.as_ref());
    let stop = cursor
        .and_then(|c| shape.index(&c))
        .filter(|&row| shape.is_shown(row, &expanded))
        .or_else(|| shape.roots.first().copied())
        .map(|row| shape.rows[row].value.clone());
    set_stop(config, stop);
}

/// The tree's row the keyboard is on, if it is on one of them, by value.
fn focused_row(config: &Rc<TreeConfig>) -> Option<String> {
    let container = config.keys.container.borrow().clone()?;
    let active = container.active_element()?;
    let on_a_row = active.get_attribute("class").is_some_and(|c| {
        c.split_whitespace()
            .any(|one| one == "rinch-tree__node-content")
    });
    if !on_a_row || !focus_inside(config) {
        return None;
    }
    active
        .parent_node()
        .and_then(|li| li.get_attribute("data-value"))
}

/// The cursor moved while a row has the keyboard (the app put it somewhere,
/// [`TreeController::set_cursor`]): the keyboard goes with it.
fn follow_focus(config: &Rc<TreeConfig>, cursor: Option<&str>) {
    let Some(cursor) = cursor else { return };
    if !config.keys.focused.get() {
        return;
    }
    let Some(focused) = focused_row(config) else {
        return;
    };
    if focused == cursor {
        return;
    }
    let row = config.drag.rows.borrow().get(cursor).cloned();
    if let Some(row) = row.filter(|row| row.is_rendered()) {
        row.focus();
        row.scroll_into_view();
    }
}

/// The tree's data changed (`old` is what it was). A cursor whose row is
/// gone moves to the row drawn below it (above it at the end) that is still
/// there, taking the keyboard with it if it had it; and the Tab stop follows.
fn data_changed(config: &Rc<TreeConfig>, tree_state: UseTreeReturn, old: &[TreeNodeData]) {
    let cursor = tree_state.cursor.get();
    let shape = config.drag.shape(config.accepts_children.as_ref());
    if let Some(cursor) = cursor.filter(|c| shape.index(c).is_none()) {
        let expanded = tree_state.expanded.get();
        let before = Shape::of(old, None);
        let next = before.index(&cursor).and_then(|row| {
            let visible = before.visible(&expanded);
            let at = visible.iter().position(|&r| r == row)?;
            visible[at + 1..]
                .iter()
                .chain(visible[..at].iter().rev())
                .map(|&r| &before.rows[r].value)
                .find(|v| shape.index(v).is_some())
                .cloned()
        });
        if config.keys.focused.get() && next.is_none() {
            set_focused(config, false);
        }
        if config.keys.focused.get() && next.is_some() {
            config.keys.lost_row.set(true);
            let weak = Rc::downgrade(config);
            let _ = set_timeout(0, move || {
                if let Some(cfg) = weak.upgrade() {
                    restore_keyboard(&cfg, tree_state);
                }
            });
        }
        tree_state.cursor.set(next);
    }
    refresh_stop(config, tree_state);
}

/// Whether the keyboard is on one of the tree's elements (a row, or a field
/// the app put in one).
fn focus_inside(config: &Rc<TreeConfig>) -> bool {
    let container = config.keys.container.borrow().clone();
    let Some(container) = container else {
        return false;
    };
    let mut at = container.active_element();
    while let Some(node) = at {
        if node.node_id() == container.node_id() {
            return true;
        }
        at = node.parent_node();
    }
    false
}

/// Whether something outside the tree has the keyboard now (a field the
/// user went to): the tree must not take it back.
fn focus_elsewhere(config: &Rc<TreeConfig>) -> bool {
    let container = config.keys.container.borrow().clone();
    let Some(active) = container.and_then(|c| c.active_element()) else {
        return false;
    };
    // The removed row the focus fell from is in no document.
    active.is_rendered() && !focus_inside(config)
}

/// Give the keyboard back to the cursor row as drawn now, after the box that
/// had it was removed: once, right after. Not when the keyboard is somewhere
/// in the tree already (a rename box the app opened) or has gone somewhere
/// else, and not to a row that is not drawn.
fn restore_keyboard(config: &Rc<TreeConfig>, tree_state: UseTreeReturn) {
    if !config.keys.lost_row.replace(false) {
        return;
    }
    if focus_inside(config) {
        return;
    }
    if focus_elsewhere(config) {
        set_focused(config, false);
        return;
    }
    let cursor = untracked(|| tree_state.cursor.get());
    let row = cursor.and_then(|c| config.drag.rows.borrow().get(&c).cloned());
    match row.filter(|row| row.is_rendered()) {
        Some(row) => row.focus(),
        None => set_focused(config, false),
    }
}

fn set_focused(config: &Rc<TreeConfig>, focused: bool) {
    config.keys.focused.set(focused);
    let container = config.keys.container.borrow().clone();
    if let Some(container) = container {
        if focused {
            container.add_class(FOCUSED_CLASS);
        } else {
            container.remove_class(FOCUSED_CLASS);
        }
    }
}

/// Select the row of `value` from the keyboard, as a click on it would:
/// only in a tree that selects on click, never a disabled row, and the app
/// is told ([`Tree::onselect`]). In a `multiple` tree the row is toggled.
fn select_from_keyboard(
    config: &Rc<TreeConfig>,
    tree_state: UseTreeReturn,
    shape: &Shape,
    value: &str,
) -> bool {
    let selectable = config.select_on_click
        && shape
            .index(value)
            .is_some_and(|row| !shape.rows[row].disabled);
    if !selectable {
        return false;
    }
    let controller = tree_state.controller;
    if controller.multiple {
        // Space toggles the row and starts the next Shift run from it; a row
        // it selects is told to `onselect`, as a click on it is.
        *config.keys.anchor.borrow_mut() = Some(value.to_string());
        if untracked(|| controller.is_selected(value)) {
            controller.deselect(value);
            selection_changed(config, tree_state);
            return true;
        }
        controller.select(value);
        selection_changed(config, tree_state);
    } else {
        controller.select(value);
    }
    if let Some(cb) = &config.onselect {
        cb.invoke(value.to_string());
    }
    true
}

/// Move the cursor to `value` from the keyboard: give its row the keyboard,
/// show it, and tell the app once it rests. A single-select tree's
/// selection follows the cursor (as a click would select); a `multiple`
/// tree's does not: Space selects there. (Nor does a plain move change where
/// the next Shift run starts: that is the row last clicked or toggled.)
fn move_cursor(config: &Rc<TreeConfig>, tree_state: UseTreeReturn, shape: &Shape, value: &str) {
    tree_state.cursor.set(Some(value.to_string()));
    if !tree_state.controller.multiple {
        select_from_keyboard(config, tree_state, shape, value);
    }
    show_cursor(config, value);
}

/// The cursor has moved to `value`: its row takes the keyboard and is
/// shown, and [`Tree::oncursor`] hears of it once it rests.
fn show_cursor(config: &Rc<TreeConfig>, value: &str) {
    let row = config.drag.rows.borrow().get(value).cloned();
    if let Some(row) = row {
        row.focus();
        row.scroll_into_view();
    }
    if let Some(handle) = config.keys.rest_timer.borrow_mut().take() {
        clear_timeout(handle);
    }
    if let Some(oncursor) = config.oncursor.clone() {
        let v = value.to_string();
        let fire = move || oncursor.invoke(v.clone());
        if config.cursor_delay_ms == 0 {
            fire();
        } else {
            let cfg = Rc::downgrade(config);
            let handle = set_timeout(config.cursor_delay_ms, move || {
                if let Some(cfg) = cfg.upgrade() {
                    cfg.keys.rest_timer.borrow_mut().take();
                }
                fire();
            });
            *config.keys.rest_timer.borrow_mut() = Some(handle);
        }
    }
}

fn expand(config: &Rc<TreeConfig>, tree_state: UseTreeReturn, value: &str) {
    tree_state.controller.expand(value);
    if let Some(cb) = &config.onexpand {
        cb.invoke(value.to_string());
    }
}

/// A key pressed while focus is inside the tree.
fn on_tree_key(
    config: &Rc<TreeConfig>,
    tree_state: UseTreeReturn,
    container: &NodeHandle,
    key: &rinch_core::KeyEventData,
) -> bool {
    // Only a row's own keys: a field inside a row (a rename box) keeps its.
    let Some(active) = container.active_element() else {
        return false;
    };
    let on_a_row = active.get_attribute("class").is_some_and(|c| {
        c.split_whitespace()
            .any(|one| one == "rinch-tree__node-content")
    });
    if !on_a_row {
        return false;
    }
    let Some(row_value) = active
        .parent_node()
        .and_then(|li| li.get_attribute("data-value"))
    else {
        return false;
    };
    set_focused(config, true);
    // The keys act from the row that has the keyboard: it is the cursor row
    // (a click on a row's own control, or a scripted focus, can put the
    // keyboard on another).
    let cursor = row_value;
    if untracked(|| tree_state.cursor.get()).as_deref() != Some(cursor.as_str()) {
        tree_state.cursor.set(Some(cursor.clone()));
    }
    let shape = config.drag.shape(config.accepts_children.as_ref());
    let Some(cursor_row) = shape.index(&cursor) else {
        return false;
    };

    if let Some(handle_key) = &config.handle_key {
        let expanded = untracked(|| tree_state.expanded.get());
        let block = shape.carried(
            &expanded,
            &untracked(|| tree_state.selected.get()),
            cursor_row,
            tree_state.controller.multiple,
        );
        // The cursor row alone: the rows drawn right above and below it
        // (#1601). A block of selected rows: the rows around the block.
        let (above, below) = if block == [cursor_row] {
            let visible = shape.visible(&expanded);
            let at = visible.iter().position(|&r| r == cursor_row);
            (
                at.and_then(|i| i.checked_sub(1)).map(|i| visible[i]),
                at.and_then(|i| visible.get(i + 1).copied()),
            )
        } else {
            shape.around(&expanded, &block)
        };
        let neighbor = |row: Option<usize>| row.map(|r| shape.rows[r].value.clone());
        let carried = carried_from(config, tree_state, &cursor);
        let offered = TreeKey {
            key: key.key.clone(),
            ctrl: key.ctrl,
            shift: key.shift,
            alt: key.alt,
            meta: key.meta,
            parent: shape.rows[cursor_row]
                .parent
                .map(|p| shape.rows[p].value.clone()),
            above: neighbor(above),
            below: neighbor(below),
            cursor: cursor.clone(),
            shape: shape.clone(),
            carried,
            judge_drop: config.judge_drop.clone(),
        };
        if handle_key(&offered) {
            return true;
        }
    }
    let multiple = tree_state.controller.multiple;
    // Selecting several rows from the keyboard, in a tree that selects
    // (#1601: one that does not select on click selects nothing from it).
    let selects_several = multiple && config.select_on_click;
    // Ctrl+A: the cursor row and its siblings (not disabled ones).
    if selects_several && (key.ctrl || key.meta) && !key.alt && key.key.eq_ignore_ascii_case("a") {
        let siblings: HashSet<String> = shape
            .siblings(cursor_row)
            .iter()
            .filter(|&&r| !shape.rows[r].disabled)
            .map(|&r| shape.rows[r].value.clone())
            .collect();
        change_selection(config, tree_state, siblings);
        return true;
    }
    if key.ctrl || key.alt || key.meta {
        return false;
    }

    let expanded = untracked(|| tree_state.expanded.get());
    // Shift with a move: a run from the anchor to where the cursor goes
    // (only the cursor moves when the tree does not select).
    if key.shift && multiple {
        if let Some(Nav::To(row)) = nav(&shape, &expanded, cursor_row, &key.key) {
            let to = shape.rows[row].value.clone();
            if config.select_on_click {
                select_run(config, tree_state, &cursor, &to);
            }
            tree_state.cursor.set(Some(to.clone()));
            show_cursor(config, &to);
            return true;
        }
    }
    // Shift+Space: the run from the anchor (the row last selected) to the
    // cursor row.
    if key.key == " " && key.shift && selects_several && config.keys.typed.borrow().is_empty() {
        select_run(config, tree_state, &cursor, &cursor);
        return true;
    }
    // Enter in a `multiple` tree: what a click does, except that the row
    // joins the selection instead of replacing it (the runtime's activation
    // would be a plain click, which replaces it).
    if key.key == "Enter" && multiple && !shape.rows[cursor_row].disabled {
        let has_children = !shape.rows[cursor_row].children.is_empty();
        activate_row(config, tree_state, &cursor, has_children, Pick::Add);
        return true;
    }
    // `*` is a shifted key on most layouts (Shift+8 on a US keyboard).
    if !key.shift || key.key == "*" {
        if let Some(step) = nav(&shape, &expanded, cursor_row, &key.key) {
            match step {
                Nav::To(row) => move_cursor(config, tree_state, &shape, &shape.rows[row].value),
                Nav::Expand(row) => expand(config, tree_state, &shape.rows[row].value),
                Nav::Collapse(row) => {
                    let value = shape.rows[row].value.clone();
                    tree_state.controller.collapse(&value);
                    unselect_hidden(config, tree_state, &value);
                    if let Some(cb) = &config.oncollapse {
                        cb.invoke(value);
                    }
                }
                Nav::ExpandAll(rows) => {
                    for row in rows {
                        expand(config, tree_state, &shape.rows[row].value);
                    }
                }
            }
            return true;
        }
    }

    // Space selects the cursor row (toggles it in a `multiple` tree), unless
    // it is part of a type-ahead word.
    if key.key == " " && !key.shift && config.keys.typed.borrow().is_empty() {
        return select_from_keyboard(config, tree_state, &shape, &cursor);
    }

    // Type-ahead: a printable character.
    let mut letters = key.key.chars();
    let (Some(letter), None) = (letters.next(), letters.next()) else {
        return false;
    };
    if letter.is_control() || (letter == ' ' && config.keys.typed.borrow().is_empty()) {
        return false;
    }
    config.keys.typed.borrow_mut().push(letter);
    if let Some(handle) = config.keys.typed_timer.borrow_mut().take() {
        clear_timeout(handle);
    }
    let cfg = Rc::downgrade(config);
    let handle = set_timeout(TYPE_AHEAD_MS, move || {
        if let Some(cfg) = cfg.upgrade() {
            cfg.keys.typed.borrow_mut().clear();
            cfg.keys.typed_timer.borrow_mut().take();
        }
    });
    *config.keys.typed_timer.borrow_mut() = Some(handle);
    let typed = config.keys.typed.borrow().clone();
    // The app's text for a row, or its label.
    let mut labels = HashMap::new();
    if config.label_of.is_none() {
        fn collect(nodes: &[TreeNodeData], labels: &mut HashMap<String, String>) {
            for node in nodes {
                labels.insert(node.value.clone(), node.label.clone());
                collect(&node.children, labels);
            }
        }
        collect(&config.drag.data.borrow(), &mut labels);
    }
    let text_of = |value: &str| match &config.label_of {
        Some(label_of) => label_of(value),
        None => labels.get(value).cloned().unwrap_or_default(),
    };
    if let Some(row) = type_ahead(&shape, &expanded, cursor_row, &typed, &text_of) {
        move_cursor(config, tree_state, &shape, &shape.rows[row].value);
    }
    true
}

/// The tree's keyboard: its key handler while focus is inside it, the
/// cursor drawn on its row, and focus requests.
fn wire_keyboard(
    __scope: &mut RenderScope,
    container: &NodeHandle,
    tree_state: UseTreeReturn,
    config: &Rc<TreeConfig>,
) {
    container.add_class("rinch-tree--keyboard");
    *config.keys.container.borrow_mut() = Some(container.clone());

    let cfg = config.clone();
    let owner = container.clone();
    let leave_cfg = Rc::downgrade(config);
    let handle = rinch_core::push_key_handler(
        container,
        move |key| on_tree_key(&cfg, tree_state, &owner, key),
        move || {
            // Focus that fell from a cursor row the tree removed (it was drawn
            // again) is still the tree's: the row as drawn now takes it back.
            // Any other focus that left the tree has left: to an editor, a
            // field, or nothing at all.
            let Some(cfg) = leave_cfg.upgrade() else {
                return;
            };
            let weak = Rc::downgrade(&cfg);
            let _ = set_timeout(0, move || {
                let Some(cfg) = weak.upgrade() else { return };
                // The keyboard is inside the tree after all: the tree still
                // has it.
                if focus_inside(&cfg) {
                    return;
                }
                // A removed cursor row's keyboard is being given back
                // (`restore_keyboard`) unless it has gone somewhere else.
                if cfg.keys.lost_row.get() {
                    if !focus_elsewhere(&cfg) {
                        return;
                    }
                    cfg.keys.lost_row.set(false);
                }
                set_focused(&cfg, false);
            });
        },
    );
    __scope.on_cleanup(move || drop(handle));

    let cfg = config.clone();
    __scope.create_effect(move || {
        let cursor = tree_state.cursor.get();
        // A collapse can hide the cursor row, which moves the Tab stop.
        let _ = tree_state.expanded.get();
        untracked(|| {
            draw_cursor(&cfg, cursor.as_deref());
            refresh_stop(&cfg, tree_state);
            follow_focus(&cfg, cursor.as_deref());
        });
    });

    let cfg = config.clone();
    let first = Cell::new(true);
    __scope.create_effect(move || {
        let _ = tree_state.controller.focus_requests.get();
        if first.replace(false) {
            return;
        }
        untracked(|| {
            let cursor = tree_state
                .cursor
                .get()
                .filter(|c| cfg.drag.rows.borrow().contains_key(c));
            let target = cursor.or_else(|| {
                let shape = cfg.drag.shape(cfg.accepts_children.as_ref());
                shape.roots.first().map(|&r| shape.rows[r].value.clone())
            });
            if let Some(value) = target {
                tree_state.cursor.set(Some(value.clone()));
                let row = cfg.drag.rows.borrow().get(&value).cloned();
                if let Some(row) = row {
                    set_focused(&cfg, true);
                    row.focus();
                    row.scroll_into_view();
                }
            }
        });
    });
}

#[cfg(test)]
mod key_tests {
    use super::*;

    /// a (open: a1, a2), b, c (closed: c1)
    fn shape() -> Shape {
        Shape::of(
            &[
                TreeNodeData::new("a", "Apples").with_children(vec![
                    TreeNodeData::new("a1", "Avocado"),
                    TreeNodeData::new("a2", "Banana"),
                ]),
                TreeNodeData::new("b", "Beets"),
                TreeNodeData::new("c", "Carrots").with_child(TreeNodeData::new("c1", "Celery")),
            ],
            None,
        )
    }

    fn open() -> HashSet<String> {
        ["a".to_string()].into_iter().collect()
    }

    fn to(shape: &Shape, step: Option<Nav>) -> String {
        match step {
            Some(Nav::To(row)) => shape.rows[row].value.clone(),
            other => panic!("expected a move, got {other:?}"),
        }
    }

    #[test]
    fn arrows_walk_the_rows_drawn() {
        let s = shape();
        let i = |v: &str| s.index(v).unwrap();
        assert_eq!(to(&s, nav(&s, &open(), i("a"), "ArrowDown")), "a1");
        assert_eq!(to(&s, nav(&s, &open(), i("a2"), "ArrowDown")), "b");
        assert_eq!(
            to(&s, nav(&s, &open(), i("c"), "ArrowDown")),
            "c",
            "the last row stays"
        );
        assert_eq!(to(&s, nav(&s, &open(), i("b"), "ArrowUp")), "a2");
        assert_eq!(to(&s, nav(&s, &open(), i("a"), "ArrowUp")), "a");
        assert_eq!(to(&s, nav(&s, &open(), i("a1"), "End")), "c");
        assert_eq!(to(&s, nav(&s, &open(), i("c"), "Home")), "a");
        assert_eq!(to(&s, nav(&s, &open(), i("a"), "PageDown")), "c");
    }

    #[test]
    fn right_and_left_open_close_and_step() {
        let s = shape();
        let i = |v: &str| s.index(v).unwrap();
        assert_eq!(
            nav(&s, &open(), i("c"), "ArrowRight"),
            Some(Nav::Expand(i("c")))
        );
        assert_eq!(to(&s, nav(&s, &open(), i("a"), "ArrowRight")), "a1");
        assert_eq!(
            nav(&s, &open(), i("b"), "ArrowRight"),
            None,
            "a leaf has nowhere to go"
        );
        assert_eq!(
            nav(&s, &open(), i("a"), "ArrowLeft"),
            Some(Nav::Collapse(i("a")))
        );
        assert_eq!(to(&s, nav(&s, &open(), i("a2"), "ArrowLeft")), "a");
        assert_eq!(nav(&s, &open(), i("a1"), "Enter"), None);
        assert_eq!(
            nav(&s, &open(), i("b"), "*"),
            Some(Nav::ExpandAll(vec![i("a"), i("c")]))
        );
    }

    #[test]
    fn type_ahead_finds_the_next_row_and_steps_on_one_letter() {
        let s = shape();
        let labels: HashMap<String, String> = [
            ("a", "Apples"),
            ("a1", "Avocado"),
            ("a2", "Banana"),
            ("b", "Beets"),
            ("c", "Carrots"),
            ("c1", "Celery"),
        ]
        .into_iter()
        .map(|(v, l)| (v.to_string(), l.to_string()))
        .collect();
        let text = |v: &str| labels[v].clone();
        let i = |v: &str| s.index(v).unwrap();
        let name = |row: Option<usize>| row.map(|r| s.rows[r].value.clone());
        assert_eq!(
            name(type_ahead(&s, &open(), i("a"), "b", &text)),
            Some("a2".into())
        );
        assert_eq!(
            name(type_ahead(&s, &open(), i("a2"), "b", &text)),
            Some("b".into())
        );
        assert_eq!(
            name(type_ahead(&s, &open(), i("a2"), "be", &text)),
            Some("b".into())
        );
        assert_eq!(
            name(type_ahead(&s, &open(), i("a"), "aa", &text)),
            Some("a1".into()),
            "a repeated letter steps on"
        );
        assert_eq!(
            name(type_ahead(&s, &open(), i("a"), "ce", &text)),
            None,
            "Celery is not drawn"
        );
    }

    fn key(cursor: &str) -> TreeKey {
        let s = Rc::new(shape());
        let parent = s.rows[s.index(cursor).unwrap()]
            .parent
            .map(|p| s.rows[p].value.clone());
        TreeKey {
            key: String::new(),
            ctrl: false,
            shift: false,
            alt: false,
            meta: false,
            cursor: cursor.into(),
            parent,
            above: None,
            below: None,
            shape: s,
            carried: vec![cursor.into()],
            judge_drop: None,
        }
    }

    fn place(drop: Option<TreeDrop>) -> Option<(Option<String>, TreeSlot)> {
        drop.map(|d| (d.parent, d.slot))
    }

    #[test]
    fn structural_moves_are_drops() {
        let after = |v: &str| TreeSlot::After(v.into());
        assert_eq!(
            place(key("a2").drop_for(TreeMove::Indent)),
            Some((Some("a1".into()), TreeSlot::Last))
        );
        assert_eq!(
            place(key("a1").drop_for(TreeMove::Indent)),
            None,
            "no sibling above"
        );
        assert_eq!(
            place(key("a2").drop_for(TreeMove::Outdent)),
            Some((None, after("a")))
        );
        assert_eq!(
            place(key("b").drop_for(TreeMove::Outdent)),
            None,
            "the top level"
        );
        assert_eq!(
            place(key("a2").drop_for(TreeMove::Up)),
            Some((Some("a".into()), TreeSlot::First))
        );
        assert_eq!(
            place(key("c").drop_for(TreeMove::Up)),
            Some((None, after("a")))
        );
        assert_eq!(place(key("a").drop_for(TreeMove::Up)), None);
        assert_eq!(
            place(key("a1").drop_for(TreeMove::Down)),
            Some((Some("a".into()), after("a2")))
        );
        assert_eq!(place(key("c").drop_for(TreeMove::Down)), None);
    }
}

// =============================================================================
// Selecting several rows
// =============================================================================

impl Shape {
    /// The rows drawn from `a` to `b` (either way round), top to bottom. An
    /// end hidden inside a collapsed row counts as the nearest row drawn
    /// above it, its shown ancestor.
    fn run(&self, expanded: &HashSet<String>, a: usize, b: usize) -> Vec<usize> {
        let visible = self.visible(expanded);
        let at = |mut row: usize| loop {
            if let Some(i) = visible.iter().position(|&r| r == row) {
                return Some(i);
            }
            row = self.rows[row].parent?;
        };
        let (Some(i), Some(j)) = (at(a), at(b)) else {
            return vec![b];
        };
        let (from, to) = if i <= j { (i, j) } else { (j, i) };
        visible[from..=to].to_vec()
    }

    /// What a drag or a move of `row` carries: the selection, top to bottom,
    /// when `row` is one of several selected rows (each selected row inside
    /// another selected one travels inside it, so it is left out); `row`
    /// alone otherwise.
    fn carried(
        &self,
        expanded: &HashSet<String>,
        selected: &HashSet<String>,
        row: usize,
        multiple: bool,
    ) -> Vec<usize> {
        if !multiple || selected.len() < 2 || !selected.contains(&self.rows[row].value) {
            return vec![row];
        }
        let chosen: Vec<usize> = self
            .visible(expanded)
            .into_iter()
            .filter(|&r| selected.contains(&self.rows[r].value))
            .collect();
        chosen
            .iter()
            .copied()
            .filter(|&r| {
                !chosen
                    .iter()
                    .any(|&other| other != r && self.is_within(r, other))
            })
            .collect()
    }

    /// The rows drawn right above and right below the block `rows` (and
    /// everything under them).
    fn around(&self, expanded: &HashSet<String>, rows: &[usize]) -> (Option<usize>, Option<usize>) {
        let visible = self.visible(expanded);
        let inside = |r: usize| rows.iter().any(|&b| self.is_within(r, b));
        let first = visible.iter().position(|&r| inside(r));
        let above = first.and_then(|i| i.checked_sub(1)).map(|i| visible[i]);
        let last = visible.iter().rposition(|&r| inside(r));
        let below = last.and_then(|i| visible[i + 1..].iter().copied().find(|&r| !inside(r)));
        (above, below)
    }
}

/// What a drag or a key move from the row of `value` carries, through the
/// app's `drag_values`.
fn carried_from(config: &Rc<TreeConfig>, tree_state: UseTreeReturn, value: &str) -> Vec<String> {
    let shape = config.drag.shape(config.accepts_children.as_ref());
    let rows = match shape.index(value) {
        Some(row) => {
            let expanded = untracked(|| tree_state.expanded.get());
            let selected = untracked(|| tree_state.selected.get());
            shape.carried(&expanded, &selected, row, tree_state.controller.multiple)
        }
        None => Vec::new(),
    };
    let values: Vec<String> = if rows.is_empty() {
        vec![value.to_string()]
    } else {
        rows.iter().map(|&r| shape.rows[r].value.clone()).collect()
    };
    match &config.drag_values {
        Some(each) => values.iter().flat_map(|v| each(v)).collect(),
        None => values,
    }
}

/// How a click (or Enter) on a row changes the selection.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pick {
    /// A plain click: the row alone.
    Replace,
    /// Ctrl- (or Cmd-) click in a `multiple` tree: the row in or out.
    Toggle,
    /// Shift-click in a `multiple` tree: the run from the anchor.
    Run,
    /// Enter in a `multiple` tree: the row joins the selection, which is
    /// otherwise kept (what a click did there before #1598).
    Add,
}

/// A click on the row of `value`, or Enter on it (`Pick::Add`).
fn activate_row(
    config: &Rc<TreeConfig>,
    tree_state: UseTreeReturn,
    value: &str,
    has_children: bool,
    pick: Pick,
) {
    let controller = tree_state.controller;
    let multiple = controller.multiple;
    let select_on_click = config.select_on_click;
    // Ctrl- and Shift-clicks change the selection and nothing else.
    if multiple && select_on_click {
        match pick {
            Pick::Toggle => {
                let mut next = untracked(|| controller.selected.get());
                if !next.remove(value) {
                    next.insert(value.to_string());
                }
                *config.keys.anchor.borrow_mut() = Some(value.to_string());
                change_selection(config, tree_state, next);
                return;
            }
            Pick::Run => {
                select_run(config, tree_state, value, value);
                return;
            }
            Pick::Replace | Pick::Add => {}
        }
    }
    if config.expand_on_click && has_children {
        let was_expanded = untracked(|| tree_state.expanded.get()).contains(value);
        controller.toggle(value);
        if !was_expanded {
            if let Some(cb) = &config.onexpand {
                cb.invoke(value.to_string());
            }
        } else {
            unselect_hidden(config, tree_state, value);
            if let Some(cb) = &config.oncollapse {
                cb.invoke(value.to_string());
            }
        }
    }
    if select_on_click {
        if multiple {
            let next = if pick == Pick::Add {
                let mut next = untracked(|| controller.selected.get());
                next.insert(value.to_string());
                next
            } else {
                HashSet::from([value.to_string()])
            };
            *config.keys.anchor.borrow_mut() = Some(value.to_string());
            change_selection(config, tree_state, next);
        } else {
            controller.select(value);
        }
        if let Some(cb) = &config.onselect {
            cb.invoke(value.to_string());
        }
    }
}

/// Make `next` the selection, and tell the app if that changed it.
fn change_selection(config: &Rc<TreeConfig>, tree_state: UseTreeReturn, next: HashSet<String>) {
    if untracked(|| tree_state.selected.get()) == next {
        return;
    }
    tree_state.controller.set_selection(next);
    selection_changed(config, tree_state);
}

/// Select the run of drawn rows from the anchor to `to`, leaving out
/// disabled rows. With no anchor, or one no longer in the data, the run
/// starts at `fallback` (which becomes the anchor).
fn select_run(config: &Rc<TreeConfig>, tree_state: UseTreeReturn, fallback: &str, to: &str) {
    let shape = config.drag.shape(config.accepts_children.as_ref());
    let Some(b) = shape.index(to) else {
        return;
    };
    let anchor = config.keys.anchor.borrow().clone();
    let a = match anchor.as_deref().and_then(|v| shape.index(v)) {
        Some(a) => a,
        None => {
            let a = shape.index(fallback).unwrap_or(b);
            *config.keys.anchor.borrow_mut() = Some(shape.rows[a].value.clone());
            a
        }
    };
    let expanded = untracked(|| tree_state.expanded.get());
    let run: HashSet<String> = shape
        .run(&expanded, a, b)
        .into_iter()
        .filter(|&r| !shape.rows[r].disabled)
        .map(|r| shape.rows[r].value.clone())
        .collect();
    change_selection(config, tree_state, run);
}

/// A collapsed row's descendants leave the selection: nothing hidden stays
/// selected.
fn unselect_hidden(config: &Rc<TreeConfig>, tree_state: UseTreeReturn, value: &str) {
    if !tree_state.controller.multiple {
        return;
    }
    let shape = config.drag.shape(config.accepts_children.as_ref());
    let Some(row) = shape.index(value) else {
        return;
    };
    let selected = untracked(|| tree_state.selected.get());
    let kept: HashSet<String> = selected
        .iter()
        .filter(|v| {
            shape
                .index(v)
                .is_none_or(|r| r == row || !shape.is_within(r, row))
        })
        .cloned()
        .collect();
    change_selection(config, tree_state, kept);
}

/// Tell the app the selection, top to bottom. A selected value with no row
/// in the data (the app deleted it) is not reported, though the selection
/// keeps it until it next changes.
fn selection_changed(config: &Rc<TreeConfig>, tree_state: UseTreeReturn) {
    let Some(cb) = &config.onselectionchange else {
        return;
    };
    let shape = config.drag.shape(config.accepts_children.as_ref());
    let selected = untracked(|| tree_state.selected.get());
    let mut ordered: Vec<(usize, String)> = selected
        .into_iter()
        .filter_map(|v| Some((shape.index(&v)?, v)))
        .collect();
    ordered.sort();
    cb.invoke(ordered.into_iter().map(|(_, v)| v).collect());
}

#[cfg(test)]
mod multi_tests {
    use super::*;

    /// a (open: a1, a2 (open: a2x)), b, c
    fn shape() -> Shape {
        Shape::of(
            &[
                TreeNodeData::new("a", "a").with_children(vec![
                    TreeNodeData::new("a1", "a1"),
                    TreeNodeData::new("a2", "a2").with_child(TreeNodeData::new("a2x", "a2x")),
                ]),
                TreeNodeData::new("b", "b"),
                TreeNodeData::new("c", "c"),
            ],
            None,
        )
    }

    fn set(values: &[&str]) -> HashSet<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    fn names(s: &Shape, rows: &[usize]) -> Vec<String> {
        rows.iter().map(|&r| s.rows[r].value.clone()).collect()
    }

    #[test]
    fn a_run_is_the_rows_drawn_between_either_way() {
        let s = shape();
        let open = set(&["a", "a2"]);
        let i = |v: &str| s.index(v).unwrap();
        assert_eq!(
            names(&s, &s.run(&open, i("a1"), i("b"))),
            ["a1", "a2", "a2x", "b"]
        );
        assert_eq!(
            names(&s, &s.run(&open, i("b"), i("a1"))),
            ["a1", "a2", "a2x", "b"]
        );
        let closed = set(&["a"]);
        assert_eq!(
            names(&s, &s.run(&closed, i("a1"), i("b"))),
            ["a1", "a2", "b"]
        );
    }

    #[test]
    fn a_selection_carries_top_down_without_rows_inside_selected_rows() {
        let s = shape();
        let open = set(&["a", "a2"]);
        let i = |v: &str| s.index(v).unwrap();
        // Out of order, with a child of a selected row.
        let carried = s.carried(&open, &set(&["c", "a2x", "a2", "a1"]), i("c"), true);
        assert_eq!(names(&s, &carried), ["a1", "a2", "c"]);
        // A row outside the selection carries itself.
        assert_eq!(
            names(&s, &s.carried(&open, &set(&["a1", "c"]), i("b"), true)),
            ["b"]
        );
        // Without multiple selection, always the row.
        assert_eq!(
            names(&s, &s.carried(&open, &set(&["a1", "c"]), i("c"), false)),
            ["c"]
        );
    }

    #[test]
    fn around_a_block_skips_what_is_inside_it() {
        let s = shape();
        let open = set(&["a", "a2"]);
        let i = |v: &str| s.index(v).unwrap();
        let name = |r: Option<usize>| r.map(|r| s.rows[r].value.clone());
        // a2 is open: the row below it is b, not its child.
        let (above, below) = s.around(&open, &[i("a2")]);
        assert_eq!(
            (name(above), name(below)),
            (Some("a1".into()), Some("b".into()))
        );
        let (above, below) = s.around(&open, &[i("a1"), i("b")]);
        assert_eq!(
            (name(above), name(below)),
            (Some("a".into()), Some("c".into()))
        );
        let (above, below) = s.around(&open, &[i("c")]);
        assert_eq!((name(above), name(below)), (Some("b".into()), None));
    }

    #[test]
    fn a_block_moves_as_one() {
        let s = Rc::new(shape());
        let key = |carried: &[&str]| TreeKey {
            key: String::new(),
            ctrl: false,
            shift: false,
            alt: false,
            meta: false,
            cursor: carried[carried.len() - 1].into(),
            parent: None,
            above: None,
            below: None,
            shape: s.clone(),
            carried: carried.iter().map(|v| v.to_string()).collect(),
            judge_drop: None,
        };
        let place = |d: Option<TreeDrop>| d.map(|d| (d.parent, d.slot));
        // b and c: up goes before a; indent goes into a (the row above the
        // first); down has nowhere to go.
        assert_eq!(
            place(key(&["b", "c"]).drop_for(TreeMove::Up)),
            Some((None, TreeSlot::First))
        );
        assert_eq!(
            place(key(&["b", "c"]).drop_for(TreeMove::Indent)),
            Some((Some("a".into()), TreeSlot::Last))
        );
        assert_eq!(place(key(&["b", "c"]).drop_for(TreeMove::Down)), None);
        // Rows of different parents do not move as one, even where the first
        // of them could move on its own (a2 has a1 above it).
        assert_eq!(place(key(&["a1", "b"]).drop_for(TreeMove::Up)), None);
        assert_eq!(place(key(&["a2", "b"]).drop_for(TreeMove::Up)), None);
        assert_eq!(place(key(&["a2", "b"]).drop_for(TreeMove::Indent)), None);
        assert_eq!(
            place(key(&["a2"]).drop_for(TreeMove::Up)),
            Some((Some("a".into()), TreeSlot::First)),
            "control: a2 alone moves up"
        );
    }
}

#[cfg(test)]
mod review_1601_drop_for_tests {
    use super::*;

    /// a (a1, a2), b, c (c1)
    fn data() -> Vec<TreeNodeData> {
        vec![
            TreeNodeData::new("a", "A").with_children(vec![
                TreeNodeData::new("a1", "A1"),
                TreeNodeData::new("a2", "A2"),
            ]),
            TreeNodeData::new("b", "B"),
            TreeNodeData::new("c", "C").with_child(TreeNodeData::new("c1", "C1")),
        ]
    }

    fn key(shape: Shape, cursor: &str, carried: &[&str]) -> TreeKey {
        let s = Rc::new(shape);
        TreeKey {
            key: String::new(),
            ctrl: false,
            shift: false,
            alt: false,
            meta: false,
            cursor: cursor.into(),
            parent: None,
            above: None,
            below: None,
            shape: s,
            carried: carried.iter().map(|c| c.to_string()).collect(),
            judge_drop: None,
        }
    }

    /// RU1: a drag never offers Inside on a row `accepts_children` refuses;
    /// the keyboard's Indent does.
    #[test]
    fn ru1_indent_into_a_row_that_takes_no_children_is_none() {
        let leaves_take_none: AcceptsChildren = Rc::new(|n: &TreeNodeData| n.has_children());
        let shape = Shape::of(&data(), Some(&leaves_take_none));
        // c's previous sibling is b, a leaf that takes no children.
        let drop = key(shape, "c", &["c"]).drop_for(TreeMove::Indent);
        assert!(
            drop.is_none(),
            "Indent into b, which accepts no children: {:?}",
            drop.map(|d| (d.parent, d.slot))
        );
    }

    /// A keyboard move the app's judge refuses is no move, as a dragged one
    /// is not delivered; one it allows is the same drop as without a judge.
    #[test]
    fn a_move_the_app_refuses_is_none() {
        let mut k = key(Shape::of(&data(), None), "c", &["c"]);
        k.judge_drop = Some(Rc::new(|d: &TreeDrop| {
            if d.parent.as_deref() == Some("b") {
                DropVerdict::Refused
            } else {
                DropVerdict::Allowed
            }
        }));
        assert!(k.drop_for(TreeMove::Indent).is_none(), "into b is refused");
        assert_eq!(
            k.drop_for(TreeMove::Up).map(|d| d.slot),
            Some(TreeSlot::After("a".into())),
            "control: another move is allowed"
        );
    }

    /// RU2: with drag_values carrying more than the cursor row (a
    /// multi-selection), Indent can put a carried row into itself. A drag
    /// refuses that (`judge`'s cycle check); the keyboard hands it over.
    #[test]
    fn ru2_indent_into_a_carried_row_is_none() {
        let shape = Shape::of(&data(), None);
        let drop = key(shape, "a2", &["a1", "a2"]).drop_for(TreeMove::Indent);
        assert!(
            drop.is_none(),
            "a1 would move into itself: {:?}",
            drop.map(|d| (d.dragged, d.parent, d.slot))
        );
    }
}
