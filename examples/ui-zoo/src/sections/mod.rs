//! UI Zoo sections - each section showcases a category of components.

#[cfg(feature = "desktop")]
mod av;
mod buttons;
mod context_menus;
mod css_features;
mod data_display;
mod editor;
mod feedback;
#[cfg(feature = "desktop")]
mod file_drop;
mod icons;
mod inputs;
mod layout;
mod navigation;
mod overlays;
mod overview;
#[cfg(feature = "desktop")]
mod render_surface;
mod tree;
mod typography;
#[cfg(feature = "desktop")]
mod video;
#[cfg(feature = "desktop")]
mod webrtc;

#[cfg(feature = "desktop")]
pub use av::{av_section, init_av_state};
pub use buttons::{buttons_section, init_buttons_state};
pub use context_menus::context_menus_section;
pub use css_features::css_features_section;
pub use data_display::data_display_section;
pub use editor::editor_section;
pub use feedback::{feedback_section, init_feedback_state};
#[cfg(feature = "desktop")]
pub use file_drop::file_drop_section;
pub use icons::{icons_section, init_icons_state};
pub use inputs::{init_inputs_state, inputs_section};
pub use layout::layout_section;
pub use navigation::{init_navigation_state, navigation_section};
pub use overlays::{OverlaysSectionState, init_overlays_state, overlays_section};
pub use overview::overview_section;
#[cfg(feature = "desktop")]
pub use render_surface::render_surface_section;
pub use tree::{init_tree_state, tree_section};
pub use typography::typography_section;
#[cfg(feature = "desktop")]
pub use video::video_section;
#[cfg(feature = "desktop")]
pub use webrtc::webrtc_section;
