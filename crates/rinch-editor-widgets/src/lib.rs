//! Toolbar and control widgets for the Rinch rich-text editor.
//!
//! Provides configurable toolbars (fixed and bubble menu),
//! individual formatting controls, and the main RichTextEditor widget configuration.
//!
//! This crate is pure configuration/data. DOM rendering belongs in the view layer.

pub mod toolbar;
pub mod controls;
pub mod editor_widget;
pub mod toolbar_render;
pub mod status_bar;

pub use toolbar::{ToolbarConfig, ToolbarControl, ToolbarGroup, ToolbarStyle};
pub use controls::ControlButton;
pub use editor_widget::RichTextEditorConfig;
pub use toolbar_render::render_toolbar;
pub use status_bar::render_status_bar;
