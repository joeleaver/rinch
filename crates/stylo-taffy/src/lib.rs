//! Conversion functions from Stylo types to Taffy types
//!
//! This crate provides interop between [`stylo`] and [`taffy`], converting Stylo's CSS computed
//! values into Taffy layout styles. Originally from the blitz project, now vendored into rinch.

mod wrapper;
pub use wrapper::TaffyStyloStyle;

pub mod convert;
#[doc(inline)]
pub use convert::to_taffy_style;

pub use style::Atom;
