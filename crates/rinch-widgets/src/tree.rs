//! Tree widget.
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
//! let tree_state = use_tree(UseTreeOptions::default());
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

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ValueCallback;
use rinch_core::hooks::use_signal;
use rinch_core::reactive::Signal;
use rinch_core::{Widget, show_dom};
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

/// Return value from `use_tree` hook.
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
    /// Create tree state without using hooks.
    /// Use this when you need to create tree state outside of component rendering,
    /// such as in context initialization.
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

/// Hook for managing tree state.
///
/// # Example
///
/// ```ignore
/// let tree = use_tree(UseTreeOptions {
///     initial_expanded: HashSet::from(["root".to_string()]),
///     ..Default::default()
/// });
///
/// // Later, programmatically expand a node:
/// tree.controller.expand("child1");
/// ```
pub fn use_tree(options: UseTreeOptions) -> UseTreeReturn {
    let expanded = use_signal(|| options.initial_expanded.clone());
    let selected = use_signal(|| options.initial_selected.clone());

    let controller = TreeController {
        expanded,
        selected,
        multiple: options.multiple,
    };

    UseTreeReturn {
        expanded,
        selected,
        controller,
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
    /// Whether the node is expanded.
    pub expanded: bool,
    /// Whether the node is selected.
    pub selected: bool,
    /// Whether the node has children.
    pub has_children: bool,
    /// Nesting level (0 = root).
    pub level: usize,
}

/// Custom render function type for tree nodes.
pub type RenderTreeNode = Rc<dyn Fn(&RenderTreeNodePayload, &mut RenderScope) -> NodeHandle>;

// =============================================================================
// Tree Widget
// =============================================================================

/// Tree view widget for displaying hierarchical data.
///
/// # Example
///
/// ```ignore
/// let data = vec![
///     TreeNodeData::new("docs", "Documents")
///         .with_icon(Icon::Folder)
///         .with_children(vec![
///             TreeNodeData::new("readme", "README.md").with_icon(Icon::File),
///             TreeNodeData::new("license", "LICENSE").with_icon(Icon::File),
///         ]),
/// ];
///
/// let tree = use_tree(UseTreeOptions::default());
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
    /// Tree state (from `use_tree` hook).
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
        }
    }
}

impl Widget for Tree {
    fn render(&self, scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let container = scope.create_element("ul");
        container.set_attribute("class", "rinch-tree");
        container.set_attribute("role", "tree");

        // Apply level offset CSS variable
        let offset = if self.level_offset.is_empty() { "md" } else { &self.level_offset };
        let offset_value = match offset {
            "xs" => "var(--rinch-spacing-xs)",
            "sm" => "var(--rinch-spacing-sm)",
            "md" => "var(--rinch-spacing-md)",
            "lg" => "var(--rinch-spacing-lg)",
            "xl" => "var(--rinch-spacing-xl)",
            custom => custom,
        };
        container.set_attribute("style", &format!("--tree-level-offset: {}", offset_value));

        // Render root nodes
        for node in &self.data {
            let node_handle = self.render_node_recursive(scope, node, 0);
            container.append_child(&node_handle);
        }

        container
    }
}

impl Tree {
    fn render_node_recursive(
        &self,
        scope: &mut RenderScope,
        node: &TreeNodeData,
        level: usize,
    ) -> NodeHandle {
        let tree_state = self.tree.as_ref();
        let expanded = tree_state
            .map(|t| t.controller.is_expanded(&node.value))
            .unwrap_or(false);
        let selected = tree_state
            .map(|t| t.controller.is_selected(&node.value))
            .unwrap_or(false);
        let has_children = node.has_children();

        // Create <li> wrapper
        let li = scope.create_element("li");
        li.set_attribute("class", "rinch-tree__node");
        li.set_attribute("role", "treeitem");
        li.set_attribute("data-value", &node.value);
        if has_children {
            li.set_attribute("aria-expanded", if expanded { "true" } else { "false" });
        }
        if selected {
            li.set_attribute("aria-selected", "true");
        }

        // Create content wrapper
        let content = scope.create_element("div");
        let base_class = if node.disabled {
            "rinch-tree__node-content rinch-tree__node-content--disabled"
        } else {
            "rinch-tree__node-content"
        };
        let mut classes = vec![base_class];
        if selected {
            classes.push("rinch-tree__node-content--selected");
        }
        content.set_attribute("class", &classes.join(" "));
        content.set_attribute(
            "style",
            &format!("padding-left: calc(var(--tree-level-offset) * {})", level),
        );
        content.set_attribute("tabindex", "0");

        // Reactive Effect for selected class updates
        if let Some(ts) = tree_state {
            let content_clone = content.clone();
            let selected_signal = ts.selected;
            let node_value = node.value.clone();
            let is_disabled = node.disabled;

            scope.create_effect(move || {
                let is_selected = selected_signal.get().contains(&node_value);
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
            let chevron = scope.create_element("span");
            chevron.set_attribute(
                "class",
                if expanded {
                    "rinch-tree__chevron rinch-tree__chevron--expanded"
                } else {
                    "rinch-tree__chevron"
                },
            );
            let icon = crate::icons::chevron_right_dom(scope);
            chevron.append_child(&icon);
            content.append_child(&chevron);

            // Reactive Effect for chevron class updates
            if let Some(ts) = tree_state {
                let chevron_clone = chevron.clone();
                let expanded_signal = ts.expanded;
                let node_value = node.value.clone();

                scope.create_effect(move || {
                    let is_expanded = expanded_signal.get().contains(&node_value);
                    chevron_clone.set_attribute(
                        "class",
                        if is_expanded {
                            "rinch-tree__chevron rinch-tree__chevron--expanded"
                        } else {
                            "rinch-tree__chevron"
                        },
                    );
                });
            }

            // Reactive Effect for aria-expanded
            if let Some(ts) = tree_state {
                let li_clone = li.clone();
                let expanded_signal = ts.expanded;
                let node_value = node.value.clone();

                scope.create_effect(move || {
                    let is_expanded = expanded_signal.get().contains(&node_value);
                    li_clone
                        .set_attribute("aria-expanded", if is_expanded { "true" } else { "false" });
                });
            }
        } else {
            let spacer = scope.create_element("span");
            spacer.set_attribute("class", "rinch-tree__spacer");
            content.append_child(&spacer);
        }

        // Render node icon if present
        if let Some(ref icon) = node.icon {
            let icon_wrapper = scope.create_element("span");
            icon_wrapper.set_attribute("class", "rinch-tree__icon");
            let icon_el = render_tabler_icon(scope, *icon, TablerIconStyle::Outline);
            icon_wrapper.append_child(&icon_el);
            content.append_child(&icon_wrapper);
        }

        // Label or custom render
        if let Some(ref render_fn) = self.render_node {
            let payload = RenderTreeNodePayload {
                node,
                expanded,
                selected,
                has_children,
                level,
            };
            let custom = render_fn(&payload, scope);
            content.append_child(&custom);
        } else {
            let label = scope.create_element("span");
            label.set_attribute("class", "rinch-tree__label");
            let text = scope.create_text(&node.label);
            label.append_child(&text);
            content.append_child(&label);
        }

        // Click handler
        if !node.disabled
            && let Some(ref tree_state) = self.tree
        {
            let controller = tree_state.controller;
            let expanded_signal = tree_state.expanded;
            let value = node.value.clone();
            let expand_on_click = self.expand_on_click && has_children;
            let select_on_click = self.select_on_click;
            let onselect = self.onselect.clone();
            let onexpand = self.onexpand.clone();
            let oncollapse = self.oncollapse.clone();

            let handler_id = scope.register_handler(move || {
                if expand_on_click {
                    // Check current state before toggling
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

        // Children - use show_dom for reactive visibility
        if has_children {
            if let Some(ts) = tree_state {
                let expanded_signal = ts.expanded;
                let node_value = node.value.clone();

                // Clone all the data we need to render children inside the closure
                let children_data: Vec<TreeNodeData> = node.children.clone();
                let child_level = level + 1;
                let level_offset = self.level_offset.clone();
                let expand_on_click = self.expand_on_click;
                let select_on_click = self.select_on_click;
                let render_node = self.render_node.clone();
                let onselect = self.onselect.clone();
                let onexpand = self.onexpand.clone();
                let oncollapse = self.oncollapse.clone();
                let tree_state_clone = self.tree;

                // show_dom inserts marker + content directly into li
                show_dom(
                    scope,
                    &li,
                    move || expanded_signal.get().contains(&node_value),
                    move |child_scope| {
                        let children_ul = child_scope.create_element("ul");
                        children_ul.set_attribute("class", "rinch-tree__subtree");
                        children_ul.set_attribute("role", "group");

                        // Render children recursively
                        for child_node in &children_data {
                            let child_tree = Tree {
                                data: vec![child_node.clone()],
                                tree: tree_state_clone,
                                level_offset: level_offset.clone(),
                                expand_on_click,
                                select_on_click,
                                render_node: render_node.clone(),
                                onselect: onselect.clone(),
                                onexpand: onexpand.clone(),
                                oncollapse: oncollapse.clone(),
                            };
                            let child_handle = child_tree.render_node_recursive(
                                child_scope,
                                child_node,
                                child_level,
                            );
                            children_ul.append_child(&child_handle);
                        }

                        children_ul
                    },
                    None::<fn(&mut RenderScope) -> NodeHandle>,
                );
            } else {
                // No tree state - render children statically if expanded
                if expanded {
                    let children_ul = scope.create_element("ul");
                    children_ul.set_attribute("class", "rinch-tree__subtree");
                    children_ul.set_attribute("role", "group");

                    for child in &node.children {
                        let child_node = self.render_node_recursive(scope, child, level + 1);
                        children_ul.append_child(&child_node);
                    }

                    li.append_child(&children_ul);
                }
            }
        }

        li
    }
}
