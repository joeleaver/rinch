//! DevTools shared reactive store and data types.
//!
//! The `DevToolsStore` holds signals shared between the main app and
//! the DevTools window. Both run on the same thread, so the signals
//! are plain thread-local `Signal`s.
//!
//! The DevTools panel reads the main app's document directly (pull model)
//! rather than receiving serialized copies (push model). A version counter
//! signal tells the panel when to re-read.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::reactive::Signal;
use rinch_dom::RinchDocument;

/// The currently active tab in DevTools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DevToolsTab {
    #[default]
    Elements,
    Styles,
    Performance,
}

/// Wrapper for the main app's document, provided via `create_context`.
#[derive(Clone)]
pub struct MainDocRef(pub Rc<RefCell<RinchDocument>>);

/// A node in the flattened DOM tree for display in the Elements panel.
#[derive(Debug, Clone, PartialEq)]
pub struct DomTreeNode {
    pub id: usize,
    pub tag: String,
    pub id_attr: Option<String>,
    pub classes: Vec<String>,
    pub text_preview: Option<String>,
    pub layout: (f32, f32, f32, f32), // absolute x, y, w, h
    pub children: Vec<DomTreeNode>,
    pub depth: usize,
    pub is_text: bool,
}

/// A computed CSS property for display in the Styles panel.
#[derive(Debug, Clone, PartialEq)]
pub struct StyleProperty {
    pub name: String,
    pub value: String,
    pub category: StyleCategory,
}

/// Category grouping for style properties.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleCategory {
    Layout,
    BoxModel,
    Typography,
    Colors,
    Position,
    Flex,
    Visual,
}

impl std::fmt::Display for StyleCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StyleCategory::Layout => write!(f, "Layout"),
            StyleCategory::BoxModel => write!(f, "Box Model"),
            StyleCategory::Typography => write!(f, "Typography"),
            StyleCategory::Colors => write!(f, "Colors"),
            StyleCategory::Position => write!(f, "Position"),
            StyleCategory::Flex => write!(f, "Flex"),
            StyleCategory::Visual => write!(f, "Visual"),
        }
    }
}

/// Shared reactive store for DevTools.
///
/// Created once at startup. Both the main app and the DevTools window
/// read/write these signals.
#[derive(Clone, Copy)]
pub struct DevToolsStore {
    /// Whether the DevTools window is currently open.
    pub visible: Signal<bool>,
    /// The active tab.
    pub active_tab: Signal<DevToolsTab>,
    /// The selected node ID (from clicking in tree or inspect mode).
    pub selected_node_id: Signal<Option<usize>>,
    /// The hovered node ID (for inspect highlight in main window).
    pub hovered_node_id: Signal<Option<usize>>,
    /// Whether inspect mode is active.
    pub inspect_mode: Signal<bool>,
    /// Current FPS.
    pub fps: Signal<f64>,
    /// Frame time in milliseconds.
    pub frame_time_ms: Signal<f64>,
    /// Version counter — bumped when the main app's DOM changes.
    /// DevTools effects watch this to know when to re-read the document.
    pub doc_version: Signal<u64>,
}

impl Default for DevToolsStore {
    fn default() -> Self {
        Self::new()
    }
}

impl DevToolsStore {
    /// Create a new DevToolsStore with default values.
    pub fn new() -> Self {
        Self {
            visible: Signal::new(false),
            active_tab: Signal::new(DevToolsTab::Elements),
            selected_node_id: Signal::new(None),
            hovered_node_id: Signal::new(None),
            inspect_mode: Signal::new(false),
            fps: Signal::new(0.0),
            frame_time_ms: Signal::new(0.0),
            doc_version: Signal::new(0),
        }
    }

    /// Bump the version counter to notify DevTools that the DOM changed.
    pub fn bump_version(&self) {
        self.doc_version.update(|v| *v += 1);
    }
}
