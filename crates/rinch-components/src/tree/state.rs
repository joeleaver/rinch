//! Tree state: the expanded and selected sets, the controller, and the
//! custom row-render payload.

use super::*;

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
        let anchor = Signal::new(None);

        let controller = TreeController {
            expanded,
            selected,
            cursor,
            focus_requests,
            anchor,
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
    pub(super) selected: Signal<HashSet<String>>,
    cursor: Signal<Option<String>>,
    pub(super) focus_requests: Signal<u64>,
    /// Where a run of selected rows starts ([`set_anchor`](Self::set_anchor)).
    anchor: Signal<Option<String>>,
    pub(super) multiple: bool,
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

    /// Make `value` the row the next Shift run starts at (Shift-click,
    /// Shift with a move key, Shift+Space in a `multiple` tree), or clear
    /// it with `None`. The tree moves it itself to the row last clicked,
    /// Ctrl-clicked, toggled with Space or entered with Enter, but not to a
    /// row the cursor merely moved to; an app whose selection follows the
    /// cursor (it calls [`set_selection`](Self::set_selection) from
    /// [`Tree::oncursor`]) sets it there too, so the next run starts at the
    /// row it selected.
    ///
    /// Nothing else changes: the selection, the cursor and the callbacks
    /// ([`Tree::onselectionchange`], [`Tree::onselect`]) are untouched. A
    /// value no row has is an anchor whose row was deleted, and `None` is no
    /// anchor: either way the next run starts at the row it is made from
    /// (the cursor row for a key, the clicked row for a click), which then
    /// becomes the anchor.
    pub fn set_anchor(&self, value: Option<&str>) {
        self.anchor.set(value.map(str::to_string));
    }

    /// The row the next Shift run starts at ([`set_anchor`](Self::set_anchor)).
    pub fn anchor(&self) -> Option<String> {
        self.anchor.get()
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
