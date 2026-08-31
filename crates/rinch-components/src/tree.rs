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
use std::collections::HashSet;
use std::rc::Rc;

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ValueCallback;
use rinch_core::for_each_dom_typed;
use rinch_core::reactive::Signal;
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

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
// Internal TreeConfig (shared across recursive renders)
// =============================================================================

struct TreeConfig {
    expand_on_click: bool,
    select_on_click: bool,
    render_node: Option<RenderTreeNode>,
    onselect: Option<ValueCallback<String>>,
    onexpand: Option<ValueCallback<String>>,
    oncollapse: Option<ValueCallback<String>>,
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
        container.set_attribute("style", &format!("--tree-level-offset: {}", offset_value));

        // Build shared config
        let config = Rc::new(TreeConfig {
            expand_on_click: self.expand_on_click,
            select_on_click: self.select_on_click,
            render_node: self.render_node.clone(),
            onselect: self.onselect.clone(),
            onexpand: self.onexpand.clone(),
            oncollapse: self.oncollapse.clone(),
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

            for_each_dom_typed(
                __scope,
                &container,
                move || collection(),
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

    // Create content wrapper
    let content = rinch_macros::rsx! { div {} };
    content.set_attribute("tabindex", "0");
    content.set_attribute(
        "style",
        &format!("padding-left: calc(var(--tree-level-offset) * {})", level),
    );

    // Reactive class for content (selected + disabled)
    {
        let content_clone = content.clone();
        let nv = node_value.clone();
        let is_disabled = node.disabled;
        __scope.create_effect(move || {
            let is_selected = selected_signal.get().contains(&nv);
            let class = if is_selected {
                if is_disabled {
                    "rinch-tree__node-content rinch-tree__node-content--disabled rinch-tree__node-content--selected"
                } else {
                    "rinch-tree__node-content rinch-tree__node-content--selected"
                }
            } else if is_disabled {
                "rinch-tree__node-content rinch-tree__node-content--disabled"
            } else {
                "rinch-tree__node-content"
            };
            content_clone.set_attribute("class", class);
        });
    }

    // Chevron (for expandable nodes)
    if has_children {
        let chevron = rinch_macros::rsx! { span {} };
        let icon = crate::icons::chevron_right_dom(__scope);
        chevron.append_child(&icon);

        // Reactive chevron class
        let chevron_clone = chevron.clone();
        let nv = node_value.clone();
        __scope.create_effect(move || {
            let is_expanded = expanded_signal.get().contains(&nv);
            chevron_clone.set_attribute(
                "class",
                if is_expanded {
                    "rinch-tree__chevron rinch-tree__chevron--expanded"
                } else {
                    "rinch-tree__chevron"
                },
            );
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

        // Reactive display toggle — preserves all child DOM and Effects
        let children_ul_vis = children_ul.clone();
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

        li.append_child(&children_ul);
    }

    li
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
