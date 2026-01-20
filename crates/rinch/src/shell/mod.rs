//! Shell module - window management and event loop.

pub mod devtools;
pub mod devtools_overlay;
#[cfg(feature = "hot-reload")]
pub mod hot_reload;
pub mod runtime;
pub mod window_manager;

pub use devtools::{DevToolsPanel, DevToolsState};
pub use devtools_overlay::render_overlay;
#[cfg(feature = "hot-reload")]
pub use hot_reload::{HotReloadConfig, HotReloader};
pub use runtime::{run, RinchEvent, Runtime};
#[cfg(feature = "hot-reload")]
pub use runtime::run_with_hot_reload;
pub use window_manager::{ManagedWindow, WindowManager};
