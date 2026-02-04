//! Platform-agnostic time types.
//!
//! On native platforms, this wraps `std::time::Instant`.
//! On WASM, this wraps `web_time::Instant` (since `std::time::Instant`
//! panics on wasm32-unknown-unknown).

#[cfg(not(target_arch = "wasm32"))]
pub use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
pub use std::time::Instant;

#[cfg(target_arch = "wasm32")]
pub use web_time::Duration;
#[cfg(target_arch = "wasm32")]
pub use web_time::Instant;
