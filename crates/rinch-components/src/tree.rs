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
    /// Controller for programmatic tree manipulation.
    pub controller: TreeController,
}

impl UseTreeReturn {
    /// Create tree state.
    pub fn new(options: UseTreeOptions) -> Self {
        let expanded = Signal::new(options.initial_expanded);
        let selected = Signal::new(options.initial_selected);

        let controller = TreeController {
            expanded,
            selected,
            multiple: options.multiple,
        };

        Self {
            expanded,
            selected,
            controller,
        }
    }
}

/// Controller for programmatic tree state manipulation.
#[derive(Clone, Copy)]
pub struct TreeController {
    expanded: Signal<HashSet<String>>,
    selected: Signal<HashSet<String>>,
    multiple: bool,
}

impl TreeController {
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
        }
    }
}

impl Component for Tree {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { ul { class: "rinch-tree" } };
        container.set_attribute("role", "tree");
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
        });

        if let Some(ref tree_state) = self.tree {
            let tree_state = *tree_state;
            let cfg = config.clone();

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
            let draggable = config.draggable;

            for_each_dom_typed(
                __scope,
                &container,
                move || {
                    let data = collection();
                    if draggable {
                        *drag.data.borrow_mut() = data.clone();
                        drag.shape.borrow_mut().take();
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
    li.set_attribute("role", "treeitem");
    li.set_attribute("data-value", &node.value);

    // Reactive aria-expanded
    if has_children {
        let li_clone = li.clone();
        let nv = node_value.clone();
        __scope.create_effect(move || {
            let is_expanded = expanded_signal.get().contains(&nv);
            li_clone.set_attribute("aria-expanded", if is_expanded { "true" } else { "false" });
        });
    }

    // Reactive aria-selected
    {
        let li_clone = li.clone();
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
    let content = rinch_macros::rsx! { div {} };
    content.set_attribute(
        "class",
        if node.disabled {
            "rinch-tree__node-content rinch-tree__node-content--disabled"
        } else {
            "rinch-tree__node-content"
        },
    );
    content.set_attribute("tabindex", "0");
    content.set_attribute(
        "style",
        &format!("padding-left: calc(var(--tree-level-offset) * {})", level),
    );

    if config.draggable {
        wire_drag(__scope, &content, &node_value, tree_state, &config);
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
            let handler_id = __scope.register_handler(move || {
                let was_expanded = expanded_signal.get().contains(&value);
                controller.toggle(&value);
                if !was_expanded {
                    if let Some(ref cb) = onexpand {
                        cb.invoke(value.clone());
                    }
                } else if let Some(ref cb) = oncollapse {
                    cb.invoke(value.clone());
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
        let controller = tree_state.controller;
        let value = node.value.clone();
        let expand_on_click = config.expand_on_click && has_children;
        let select_on_click = config.select_on_click;
        let onselect = config.onselect.clone();
        let onexpand = config.onexpand.clone();
        let oncollapse = config.oncollapse.clone();

        let handler_id = __scope.register_handler(move || {
            if expand_on_click {
                let was_expanded = expanded_signal.get().contains(&value);
                controller.toggle(&value);
                if !was_expanded {
                    if let Some(ref cb) = onexpand {
                        cb.invoke(value.clone());
                    }
                } else if let Some(ref cb) = oncollapse {
                    cb.invoke(value.clone());
                }
            }
            if select_on_click {
                controller.select(&value);
                if let Some(ref cb) = onselect {
                    cb.invoke(value.clone());
                }
            }
        });
        content.set_attribute("data-rid", &handler_id.0.to_string());
    }

    li.append_child(&content);

    // Children — always render, toggle visibility with display:none
    if has_children {
        let children_ul = rinch_macros::rsx! { ul { class: "rinch-tree__subtree" } };
        children_ul.set_attribute("role", "group");

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
    // A row drawn again replaces the old box, which keeps nothing of a drag.
    // (No borrow of the drag state is held across a DOM call anywhere here:
    // one can run pending effects, which can draw rows, which come back here.)
    let old = config
        .drag
        .rows
        .borrow_mut()
        .insert(value.to_string(), content.clone());
    if let Some(old) = old {
        clear_drawn(&old);
        old.remove_class(DRAGGING_CLASS);
    }
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
        let carried = match &cfg.drag_values {
            Some(values) => values(&v),
            None => vec![v.clone()],
        };
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

/// The verdict on a drop: refused when it would put a row into its own
/// subtree, whatever the app says; otherwise the app's ([`Tree::judge_drop`]).
/// The places a drag offers never reach the first case (rows under a dragged
/// row offer nothing, and a dragged row is placed as if collapsed); it stays
/// as a guard.
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
    if cycle {
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
    li.set_attribute("role", "treeitem");
    li.set_attribute("data-value", &node.value);

    let content = rinch_macros::rsx! { div {} };
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
