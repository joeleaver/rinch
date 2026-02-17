//! Toolbar and control components for the Rinch rich-text editor.
//!
//! Provides configurable toolbars (fixed and bubble menu),
//! individual formatting controls, and the main RichTextEditor component configuration.
//!
//! This crate is pure configuration/data. DOM rendering belongs in the view layer.

pub mod controls;
pub mod editor_component;
pub mod status_bar;
pub mod toolbar;
pub mod toolbar_render;

pub use controls::ControlButton;
pub use editor_component::RichTextEditorConfig;
pub use status_bar::render_status_bar;
pub use toolbar::{ToolbarConfig, ToolbarControl, ToolbarGroup, ToolbarStyle};
pub use toolbar_render::render_toolbar;
