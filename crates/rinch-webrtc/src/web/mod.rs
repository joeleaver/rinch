//! WASM WebRTC backend using web_sys bindings.

pub mod peer;

pub use peer::{WebPeerConnection, WebRemoteTrack};
