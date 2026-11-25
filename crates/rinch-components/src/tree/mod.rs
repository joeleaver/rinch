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

mod component;
mod drag;
mod drop;
mod keyboard;
mod keys;
mod render;
mod selection;
mod state;

#[cfg(test)]
mod drop_tests;
#[cfg(test)]
mod key_tests;
#[cfg(test)]
mod multi_tests;
#[cfg(test)]
mod review_1601_drop_for_tests;

pub use component::Tree;
pub use drop::{
    AcceptsChildren, AfterOpenRow, DragValues, DropVerdict, DropZone, JudgeDrop, TreeDrop, TreeSlot,
};
pub use keys::{LabelOf, TreeKey, TreeKeyHandler, TreeMove};
pub use state::{
    RenderTreeNode, RenderTreeNodePayload, TreeController, UseTreeOptions, UseTreeReturn,
    get_tree_expanded_state,
};

use component::*;
use drag::*;
use drop::*;
use keyboard::*;
use keys::*;
use render::*;
use selection::*;

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
