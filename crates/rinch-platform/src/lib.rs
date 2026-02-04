//! Platform abstraction traits for rinch.
//!
//! This crate defines the contract between the rinch runtime and platform
//! backends. The rinch runtime is platform-agnostic -- it processes
//! `PlatformEvent`s and returns `AppAction`s. Each platform backend
//! (desktop, web, mobile) implements these traits to provide windowing,
//! rendering, event loops, and menus.
//!
//! # Traits
//!
//! - [`PlatformWindow`] -- window operations (size, scale, redraw, minimize/maximize)
//! - [`PlatformRenderer`] -- GPU rendering pipeline (init, resize, render scene)
//! - [`PlatformEventLoop`] / [`PlatformWaker`] -- event loop and cross-thread wake
//! - [`PlatformMenu`] -- native menu system (optional, desktop-only)
//!
//! # Types
//!
//! - [`PlatformEvent`] -- platform-agnostic input events
//! - [`KeyCode`], [`Modifiers`], [`MouseButton`] -- input primitives
//! - [`AppAction`] -- actions the runtime requests from the platform
//! - [`Instant`] -- platform-agnostic time (std or web_time)

pub mod event;
pub mod event_loop;
pub mod menu;
pub mod renderer;
pub mod time;
pub mod window;

pub use event::*;
pub use event_loop::*;
pub use menu::*;
pub use renderer::*;
pub use time::*;
pub use window::*;
