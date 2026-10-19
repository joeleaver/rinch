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
use rinch_core::get_click_context;
use rinch_core::for_each_dom_typed;
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
    /// keys and type-ahead, and by a click.
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
    /// Put the keyboard's cursor on a row, without moving the keyboard.
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
/// its middle half, or its bottom quarter.
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
    fn of(data: &[TreeNodeData]) -> Self {
        fn walk(shape: &mut Shape, nodes: &[TreeNodeData], parent: Option<usize>, level: usize) -> Vec<usize> {
            let mut ids = Vec::with_capacity(nodes.len());
            for node in nodes {
                let id = shape.rows.len();
                shape.rows.push(ShapeRow { value: node.value.clone(), parent, level, children: Vec::new() });
                shape.by_value.insert(node.value.clone(), id);
                ids.push(id);
                let children = walk(shape, &node.children, Some(id), level + 1);
                shape.rows[id].children = children;
            }
            ids
        }
        let mut shape = Shape::default();
        shape.roots = walk(&mut shape, data, None, 0);
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
    fn placement(&self, over: usize, zone: DropZone, pointer_level: Option<usize>, expanded: &HashSet<String>) -> Placement {
        let row = &self.rows[over];
        match zone {
            DropZone::Before => Placement {
                parent: row.parent,
                slot: self.prev_sibling(over).map_or(Slot::First, Slot::After),
                line_level: row.level,
            },
            DropZone::Inside => Placement { parent: Some(over), slot: Slot::Last, line_level: row.level },
            DropZone::After if self.is_open(over, expanded) => {
                Placement { parent: Some(over), slot: Slot::First, line_level: row.level + 1 }
            }
            DropZone::After => {
                let deepest = row.level;
                let shallowest = self.next_visible(over, expanded).map_or(0, |next| self.rows[next].level).min(deepest);
                let level = pointer_level.map_or(deepest, |level| level.clamp(shallowest, deepest));
                let anchor = self.ancestor_at(over, level);
                Placement { parent: self.rows[anchor].parent, slot: Slot::After(anchor), line_level: level }
            }
        }
    }
}

/// The zone of a row a pointer `fraction` of the way down it is over.
fn zone_at(fraction: f32) -> DropZone {
    if fraction < 0.25 {
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
}

impl DragState {
    fn shape(&self) -> Rc<Shape> {
        self.shape
            .borrow_mut()
            .get_or_insert_with(|| Rc::new(Shape::of(&self.data.borrow())))
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
    /// Indentation per level in pixels. Overrides `level_offset`, and is what
    /// lets a drag at the end of a group choose its depth by the pointer's
    /// horizontal position.
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
    /// The drop on offer and its verdict, each time it changes; `None` when
    /// the pointer leaves the rows or the drag ends.
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
    /// row that starts with them. The cursor row is selected as it moves.
    pub keyboard: bool,
    /// Every key the tree's rows get, before the tree's own; `true` consumes
    /// it. [`TreeKey::drop_for`] turns a key into a structural move.
    pub handle_key: Option<TreeKeyHandler>,
    /// The cursor came to rest on a row (after `cursor_delay_ms`): for an
    /// app that opens what the cursor is on, once, not on every row it
    /// passed.
    pub oncursor: Option<ValueCallback<String>>,
    pub cursor_delay_ms: u32,
    /// The text type-ahead matches for a row (its label when not given).
    pub label_of: Option<LabelOf>,
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
        }
    }
}

impl Component for Tree {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { ul { class: "rinch-tree" } };
        container.set_attribute("role", "tree");

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
            ondropchange: self.ondropchange.clone(),
            ondrop: self.ondrop.clone(),
            ondragstart: self.ondragstart.clone(),
            ondragend: self.ondragend.clone(),
            indent: self.indent,
            hover_expand_ms: self.hover_expand_ms,
            guides: self.guides,
            drag: Rc::new(DragState::default()),
            keyboard: self.keyboard && self.tree.is_some(),
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
            let draggable = config.draggable || config.keyboard;

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

    if config.draggable || config.keyboard {
        // A row drawn again replaces the old box, which keeps nothing of a
        // drag. (No borrow of the tree's state is held across a DOM call
        // anywhere here: one can run pending effects, which can draw rows,
        // which come back here.)
        let old = config.drag.rows.borrow_mut().insert(node_value.clone(), content.clone());
        if let Some(old) = old {
            clear_drawn(&old);
            old.remove_class(DRAGGING_CLASS);
            old.remove_class(CURSOR_CLASS);
        }
    }
    if config.draggable {
        wire_drag(__scope, &content, &node_value, tree_state, &config);
    }
    if config.keyboard && untracked(|| tree_state.cursor.get()).as_deref() == Some(node_value.as_str()) {
        content.add_class(CURSOR_CLASS);
        *config.keys.drawn.borrow_mut() = Some(node_value.clone());
        // Drawn again while it had the keyboard: it keeps it.
        if config.keys.focused.get() {
            let row = content.clone();
            let _ = set_timeout(0, move || row.focus());
        }
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
        label.set_attribute("title", &node.label);
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

        let cfg = config.clone();
        let handler_id = __scope.register_handler(move || {
            if cfg.keyboard {
                tree_state.cursor.set(Some(value.clone()));
                set_focused(&cfg, true);
            }
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
                &format!("left: calc(var(--tree-level-offset) * {level} + var(--tree-guide-offset))"),
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
        let carried = match &cfg.drag_values {
            Some(values) => values(&v),
            None => vec![v.clone()],
        };
        mark_dragging(&cfg, &carried, true);
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
        let here = cfg.drag.candidate.borrow().as_ref().is_some_and(|(drop, _, _)| drop.over == v);
        if here {
            offer(&cfg, None);
        }
    });
    content.set_attribute("data-ondragleave", &leave.0.to_string());

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
    let shape = config.drag.shape();
    let Some(over) = shape.index(value) else { return };
    let dragged_rows: Vec<usize> = dragged.iter().filter_map(|d| shape.index(d)).collect();

    let ctx = get_click_context();
    let height = ctx.element_height.max(1.0);
    let zone = zone_at((ctx.mouse_y - ctx.element_y) / height);
    // Nothing is offered under a dragged row, or inside one. A dragged row's
    // own edges are places, which is how the last row of a group is dragged
    // out of it: left, over itself.
    let on_dragged_row = dragged_rows.contains(&over);
    let under_dragged = dragged_rows.iter().any(|&d| d != over && shape.is_within(over, d));
    if under_dragged || (on_dragged_row && zone == DropZone::Inside) {
        offer(config, None);
        return;
    }
    let pointer_level = config
        .indent
        .filter(|px| *px > 0.0)
        .map(|px| ((ctx.mouse_x - ctx.element_x).max(0.0) / px) as usize);
    let expanded = untracked(|| tree_state.expanded.get());
    let placement = shape.placement(over, zone, pointer_level, &expanded);

    // Hover to expand: resting inside a collapsed row with children.
    let wants_hover = zone == DropZone::Inside
        && config.hover_expand_ms > 0
        && !shape.rows[over].children.is_empty()
        && !expanded.contains(value);
    let waiting_here = config.drag.hover.borrow().as_ref().is_some_and(|(v, _)| v == value);
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
    // A row can never go into its own subtree, whatever the app says.
    let cycle = placement
        .parent
        .is_some_and(|parent| dragged_rows.iter().any(|&d| shape.is_within(parent, d)));
    let verdict = if cycle {
        DropVerdict::Refused
    } else {
        config.judge_drop.as_ref().map_or(DropVerdict::Allowed, |judge| judge(&drop))
    };
    offer(config, Some((drop, placement, verdict)));
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
    if candidate.is_none() {
        drag.cancel_hover();
        if drag.candidate.borrow().is_none() {
            return;
        }
    }
    let previous = drag.drawn.borrow_mut().take();
    let previous = previous.and_then(|value| drag.rows.borrow().get(&value).cloned());
    if let Some(row) = previous {
        clear_drawn(&row);
    }
    if let Some((drop, placement, verdict)) = &candidate {
        let row = drag.rows.borrow().get(&drop.over).cloned();
        if let Some(row) = row {
            *drag.drawn.borrow_mut() = Some(drop.over.clone());
            draw(&row, drop.zone, placement.line_level, *verdict);
        }
    }
    let told = candidate.as_ref().map(|(drop, _, verdict)| (drop.clone(), *verdict));
    *drag.candidate.borrow_mut() = candidate;
    if let Some(cb) = &config.ondropchange {
        cb.invoke(told);
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
        assert_eq!(zone_at(0.0), DropZone::Before);
        assert_eq!(zone_at(0.24), DropZone::Before);
        assert_eq!(zone_at(0.5), DropZone::Inside);
        assert_eq!(zone_at(0.76), DropZone::After);
        assert_eq!(zone_at(1.0), DropZone::After);
    }

    #[test]
    fn before_lands_after_the_previous_sibling_or_first() {
        let shape = Shape::of(&data());
        let open = expanded(&["a", "a1", "c"]);
        assert_eq!(place(&shape, "a2", DropZone::Before, None, &open), (Some("a".into()), after("a1"), 1));
        assert_eq!(place(&shape, "a1", DropZone::Before, None, &open), (Some("a".into()), TreeSlot::First, 1));
        assert_eq!(place(&shape, "a", DropZone::Before, None, &open), (None, TreeSlot::First, 0));
    }

    #[test]
    fn inside_lands_last() {
        let shape = Shape::of(&data());
        let open = expanded(&[]);
        assert_eq!(place(&shape, "b", DropZone::Inside, None, &open), (Some("b".into()), TreeSlot::Last, 0));
        assert_eq!(place(&shape, "a", DropZone::Inside, None, &open), (Some("a".into()), TreeSlot::Last, 0));
    }

    #[test]
    fn after_an_open_row_with_children_lands_first_inside_it() {
        let shape = Shape::of(&data());
        let open = expanded(&["a"]);
        assert_eq!(place(&shape, "a", DropZone::After, None, &open), (Some("a".into()), TreeSlot::First, 1));
        // Closed, the same zone is after it among its siblings.
        let closed = expanded(&[]);
        assert_eq!(place(&shape, "a", DropZone::After, None, &closed), (None, after("a"), 0));
    }

    /// The drag that outdents: after the last row of a group, the pointer's
    /// indent column picks the depth, between the row's own level and the
    /// level of the row drawn below it.
    #[test]
    fn after_the_end_of_a_group_takes_its_depth_from_the_pointer() {
        let shape = Shape::of(&data());
        let open = expanded(&["a", "a1"]);
        // a1y is the last of a1, which is not the last of a; below it is a2
        // (level 1), so levels 1..=2 are on offer.
        assert_eq!(place(&shape, "a1y", DropZone::After, Some(9), &open), (Some("a1".into()), after("a1y"), 2));
        assert_eq!(place(&shape, "a1y", DropZone::After, Some(2), &open), (Some("a1".into()), after("a1y"), 2));
        assert_eq!(place(&shape, "a1y", DropZone::After, Some(1), &open), (Some("a".into()), after("a1"), 1));
        assert_eq!(place(&shape, "a1y", DropZone::After, Some(0), &open), (Some("a".into()), after("a1"), 1));
        // Without a pointer column, the row's own depth.
        assert_eq!(place(&shape, "a1y", DropZone::After, None, &open), (Some("a1".into()), after("a1y"), 2));
        // a2 ends a, and b (level 0) is below: level 0 means after a.
        assert_eq!(place(&shape, "a2", DropZone::After, Some(0), &open), (None, after("a"), 0));
        // The last row of the tree can go all the way out.
        let open = expanded(&["c"]);
        assert_eq!(place(&shape, "c1", DropZone::After, Some(0), &open), (None, after("c"), 0));
        // A row in the middle of its siblings offers only its own level.
        let open = expanded(&["a", "a1"]);
        assert_eq!(place(&shape, "a1x", DropZone::After, Some(0), &open), (Some("a1".into()), after("a1x"), 2));
    }

    #[test]
    fn next_visible_follows_what_is_expanded() {
        let shape = Shape::of(&data());
        let next = |v: &str, open: &[&str]| {
            shape.next_visible(shape.index(v).unwrap(), &expanded(open)).map(|r| shape.rows[r].value.clone())
        };
        assert_eq!(next("a", &[]), Some("b".into()));
        assert_eq!(next("a", &["a"]), Some("a1".into()));
        assert_eq!(next("a1y", &["a", "a1"]), Some("a2".into()));
        assert_eq!(next("a2", &["a"]), Some("b".into()));
        assert_eq!(next("c1", &["c"]), None);
    }

    #[test]
    fn within_is_the_row_or_under_it() {
        let shape = Shape::of(&data());
        let i = |v: &str| shape.index(v).unwrap();
        assert!(shape.is_within(i("a1x"), i("a")));
        assert!(shape.is_within(i("a"), i("a")));
        assert!(!shape.is_within(i("a"), i("a1")));
        assert!(!shape.is_within(i("b"), i("a")));
    }
}

// =============================================================================
// Keyboard
// =============================================================================

/// A structural move of the cursor row, for an app that binds keys to them
/// ([`TreeKey::drop_for`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// moves the cursor off a row it is about to remove.
    pub above: Option<String>,
    pub below: Option<String>,
    shape: Rc<Shape>,
    carried: Vec<String>,
}

impl TreeKey {
    /// The drop that would make `movement` of the cursor row (what a drag of
    /// it would carry, [`Tree::drag_values`]), for the app to judge and
    /// deliver as it does a dropped one. `None` when there is nowhere to go:
    /// no previous sibling to indent into, no parent to outdent from, no
    /// sibling above or below.
    pub fn drop_for(&self, movement: TreeMove) -> Option<TreeDrop> {
        let shape = &self.shape;
        let row = shape.index(&self.cursor)?;
        let siblings = shape.siblings(row);
        let at = siblings.iter().position(|&s| s == row)?;
        let value = |row: usize| shape.rows[row].value.clone();
        let (parent, slot, zone) = match movement {
            TreeMove::Indent => {
                let before = *siblings.get(at.checked_sub(1)?)?;
                (Some(value(before)), TreeSlot::Last, DropZone::Inside)
            }
            TreeMove::Outdent => {
                let parent = shape.rows[row].parent?;
                (shape.rows[parent].parent.map(value), TreeSlot::After(value(parent)), DropZone::After)
            }
            TreeMove::Up => {
                at.checked_sub(1)?;
                let slot = match at.checked_sub(2) {
                    Some(two_up) => TreeSlot::After(value(siblings[two_up])),
                    None => TreeSlot::First,
                };
                (shape.rows[row].parent.map(value), slot, DropZone::Before)
            }
            TreeMove::Down => {
                let below = *siblings.get(at + 1)?;
                (shape.rows[row].parent.map(value), TreeSlot::After(value(below)), DropZone::After)
            }
        };
        Some(TreeDrop { dragged: self.carried.clone(), parent, slot, over: self.cursor.clone(), zone })
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
    /// Whether one of the rows has the keyboard.
    focused: Cell<bool>,
    /// Letters typed for type-ahead, and the timer that forgets them.
    typed: RefCell<String>,
    typed_timer: RefCell<Option<TimeoutHandle>>,
    /// The cursor's rest timer, before `oncursor`.
    rest_timer: RefCell<Option<TimeoutHandle>>,
    /// The tree's own list, for its focused class.
    container: RefCell<Option<NodeHandle>>,
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
            shape.siblings(cursor).iter().copied().filter(|&s| !shape.rows[s].children.is_empty()).collect(),
        ),
        _ => return None,
    })
}

/// The next drawn row after `cursor` (wrapping) whose text starts with
/// `typed`, case-insensitively. Typing one letter again and again steps
/// through the rows that start with it.
fn type_ahead(shape: &Shape, expanded: &HashSet<String>, cursor: usize, typed: &str, text: &dyn Fn(&str) -> String) -> Option<usize> {
    let visible = shape.visible(expanded);
    let at = visible.iter().position(|&r| r == cursor).unwrap_or(0);
    let typed = typed.to_lowercase();
    let mut chars = typed.chars();
    let first = chars.next()?;
    let repeated = chars.all(|c| c == first);
    let matches = |row: usize, prefix: &str| text(&shape.rows[row].value).to_lowercase().starts_with(prefix);
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

/// Move the cursor to `value` from the keyboard: select it, give its row the
/// keyboard, show it, and tell the app once it rests.
fn move_cursor(config: &Rc<TreeConfig>, tree_state: UseTreeReturn, value: &str) {
    tree_state.cursor.set(Some(value.to_string()));
    tree_state.controller.select(value);
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
fn on_tree_key(config: &Rc<TreeConfig>, tree_state: UseTreeReturn, container: &NodeHandle, key: &rinch_core::KeyEventData) -> bool {
    // Only a row's own keys: a field inside a row (a rename box) keeps its.
    let Some(active) = container.active_element() else { return false };
    let on_a_row = active
        .get_attribute("class")
        .is_some_and(|c| c.split_whitespace().any(|one| one == "rinch-tree__node-content"));
    if !on_a_row {
        return false;
    }
    let Some(row_value) = active.parent_node().and_then(|li| li.get_attribute("data-value")) else { return false };
    set_focused(config, true);
    let cursor = untracked(|| tree_state.cursor.get()).filter(|c| config.drag.rows.borrow().contains_key(c)).unwrap_or(row_value);
    let shape = config.drag.shape();
    let Some(cursor_row) = shape.index(&cursor) else { return false };

    if let Some(handle_key) = &config.handle_key {
        let expanded = untracked(|| tree_state.expanded.get());
        let visible = shape.visible(&expanded);
        let at = visible.iter().position(|&r| r == cursor_row);
        let neighbor = |i: Option<usize>| i.and_then(|i| visible.get(i)).map(|&r| shape.rows[r].value.clone());
        let carried = match &config.drag_values {
            Some(values) => values(&cursor),
            None => vec![cursor.clone()],
        };
        let offered = TreeKey {
            key: key.key.clone(),
            ctrl: key.ctrl,
            shift: key.shift,
            alt: key.alt,
            meta: key.meta,
            parent: shape.rows[cursor_row].parent.map(|p| shape.rows[p].value.clone()),
            above: neighbor(at.and_then(|i| i.checked_sub(1))),
            below: neighbor(at.map(|i| i + 1)),
            cursor: cursor.clone(),
            shape: shape.clone(),
            carried,
        };
        if handle_key(&offered) {
            return true;
        }
    }
    if key.ctrl || key.alt || key.meta {
        return false;
    }

    let expanded = untracked(|| tree_state.expanded.get());
    if !key.shift {
        if let Some(step) = nav(&shape, &expanded, cursor_row, &key.key) {
            match step {
                Nav::To(row) => move_cursor(config, tree_state, &shape.rows[row].value),
                Nav::Expand(row) => expand(config, tree_state, &shape.rows[row].value),
                Nav::Collapse(row) => {
                    let value = shape.rows[row].value.clone();
                    tree_state.controller.collapse(&value);
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

    // Type-ahead: a printable character.
    let mut letters = key.key.chars();
    let (Some(letter), None) = (letters.next(), letters.next()) else { return false };
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
        move_cursor(config, tree_state, &shape.rows[row].value);
    }
    true
}

/// The tree's keyboard: its key handler while focus is inside it, the
/// cursor drawn on its row, and focus requests.
fn wire_keyboard(__scope: &mut RenderScope, container: &NodeHandle, tree_state: UseTreeReturn, config: &Rc<TreeConfig>) {
    container.add_class("rinch-tree--keyboard");
    *config.keys.container.borrow_mut() = Some(container.clone());

    let cfg = config.clone();
    let owner = container.clone();
    let leave_cfg = Rc::downgrade(config);
    let handle = rinch_core::push_key_handler(
        container,
        move |key| on_tree_key(&cfg, tree_state, &owner, key),
        move || {
            if let Some(cfg) = leave_cfg.upgrade() {
                set_focused(&cfg, false);
            }
        },
    );
    __scope.on_cleanup(move || drop(handle));

    let cfg = config.clone();
    __scope.create_effect(move || {
        let cursor = tree_state.cursor.get();
        untracked(|| draw_cursor(&cfg, cursor.as_deref()));
    });

    let cfg = config.clone();
    let first = Cell::new(true);
    __scope.create_effect(move || {
        let _ = tree_state.controller.focus_requests.get();
        if first.replace(false) {
            return;
        }
        untracked(|| {
            let cursor = tree_state.cursor.get().filter(|c| cfg.drag.rows.borrow().contains_key(c));
            let target = cursor.or_else(|| {
                let shape = cfg.drag.shape();
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
        Shape::of(&[
            TreeNodeData::new("a", "Apples").with_children(vec![
                TreeNodeData::new("a1", "Avocado"),
                TreeNodeData::new("a2", "Banana"),
            ]),
            TreeNodeData::new("b", "Beets"),
            TreeNodeData::new("c", "Carrots").with_child(TreeNodeData::new("c1", "Celery")),
        ])
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
        assert_eq!(to(&s, nav(&s, &open(), i("c"), "ArrowDown")), "c", "the last row stays");
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
        assert_eq!(nav(&s, &open(), i("c"), "ArrowRight"), Some(Nav::Expand(i("c"))));
        assert_eq!(to(&s, nav(&s, &open(), i("a"), "ArrowRight")), "a1");
        assert_eq!(nav(&s, &open(), i("b"), "ArrowRight"), None, "a leaf has nowhere to go");
        assert_eq!(nav(&s, &open(), i("a"), "ArrowLeft"), Some(Nav::Collapse(i("a"))));
        assert_eq!(to(&s, nav(&s, &open(), i("a2"), "ArrowLeft")), "a");
        assert_eq!(nav(&s, &open(), i("a1"), "Enter"), None);
        assert_eq!(nav(&s, &open(), i("b"), "*"), Some(Nav::ExpandAll(vec![i("a"), i("c")])));
    }

    #[test]
    fn type_ahead_finds_the_next_row_and_steps_on_one_letter() {
        let s = shape();
        let labels: HashMap<String, String> =
            [("a", "Apples"), ("a1", "Avocado"), ("a2", "Banana"), ("b", "Beets"), ("c", "Carrots"), ("c1", "Celery")]
                .into_iter()
                .map(|(v, l)| (v.to_string(), l.to_string()))
                .collect();
        let text = |v: &str| labels[v].clone();
        let i = |v: &str| s.index(v).unwrap();
        let name = |row: Option<usize>| row.map(|r| s.rows[r].value.clone());
        assert_eq!(name(type_ahead(&s, &open(), i("a"), "b", &text)), Some("a2".into()));
        assert_eq!(name(type_ahead(&s, &open(), i("a2"), "b", &text)), Some("b".into()));
        assert_eq!(name(type_ahead(&s, &open(), i("a2"), "be", &text)), Some("b".into()));
        assert_eq!(name(type_ahead(&s, &open(), i("a"), "aa", &text)), Some("a1".into()), "a repeated letter steps on");
        assert_eq!(name(type_ahead(&s, &open(), i("a"), "ce", &text)), None, "Celery is not drawn");
    }

    fn key(cursor: &str) -> TreeKey {
        let s = Rc::new(shape());
        let parent = s.rows[s.index(cursor).unwrap()].parent.map(|p| s.rows[p].value.clone());
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
        }
    }

    fn place(drop: Option<TreeDrop>) -> Option<(Option<String>, TreeSlot)> {
        drop.map(|d| (d.parent, d.slot))
    }

    #[test]
    fn structural_moves_are_drops() {
        let after = |v: &str| TreeSlot::After(v.into());
        assert_eq!(place(key("a2").drop_for(TreeMove::Indent)), Some((Some("a1".into()), TreeSlot::Last)));
        assert_eq!(place(key("a1").drop_for(TreeMove::Indent)), None, "no sibling above");
        assert_eq!(place(key("a2").drop_for(TreeMove::Outdent)), Some((None, after("a"))));
        assert_eq!(place(key("b").drop_for(TreeMove::Outdent)), None, "the top level");
        assert_eq!(place(key("a2").drop_for(TreeMove::Up)), Some((Some("a".into()), TreeSlot::First)));
        assert_eq!(place(key("c").drop_for(TreeMove::Up)), Some((None, after("a"))));
        assert_eq!(place(key("a").drop_for(TreeMove::Up)), None);
        assert_eq!(place(key("a1").drop_for(TreeMove::Down)), Some((Some("a".into()), after("a2"))));
        assert_eq!(place(key("c").drop_for(TreeMove::Down)), None);
    }
}
