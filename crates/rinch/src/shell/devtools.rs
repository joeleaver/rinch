//! DevTools state for rinch applications.
//!
//! Provides a developer tools panel for inspecting the UI tree,
//! viewing element styles, and debugging hook state.

/// The currently active panel in the devtools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DevToolsPanel {
    /// DOM tree inspector.
    #[default]
    Elements,
    /// Computed styles viewer.
    Styles,
    /// Hook state inspector.
    Hooks,
}

/// State for the developer tools overlay.
#[derive(Debug, Clone, Default)]
pub struct DevToolsState {
    /// Whether the devtools panel is visible.
    pub visible: bool,
    /// The currently selected DOM node ID.
    pub selected_node: Option<usize>,
    /// Whether inspect mode is active (click to select elements).
    pub inspect_mode: bool,
    /// The currently active panel.
    pub active_panel: DevToolsPanel,
    /// The width of the devtools panel in pixels.
    pub panel_width: u32,
}

impl DevToolsState {
    /// Create a new DevToolsState with default settings.
    pub fn new() -> Self {
        Self {
            visible: false,
            selected_node: None,
            inspect_mode: false,
            active_panel: DevToolsPanel::Elements,
            panel_width: 300,
        }
    }

    /// Toggle the visibility of the devtools panel.
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
        if !self.visible {
            self.inspect_mode = false;
        }
    }

    /// Toggle inspect mode.
    pub fn toggle_inspect_mode(&mut self) {
        self.inspect_mode = !self.inspect_mode;
    }

    /// Set the selected node.
    pub fn select_node(&mut self, node_id: usize) {
        self.selected_node = Some(node_id);
    }

    /// Clear the selection.
    pub fn clear_selection(&mut self) {
        self.selected_node = None;
    }

    /// Set the active panel.
    pub fn set_panel(&mut self, panel: DevToolsPanel) {
        self.active_panel = panel;
    }
}
