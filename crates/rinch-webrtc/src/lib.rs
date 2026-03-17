//! WebRTC peer connections for rinch.
//!
//! Provides a platform-abstracted `PeerConnection` that uses str0m on native
//! and web_sys RTCPeerConnection on WASM, plus shared signaling types.

pub mod call;
pub mod peer;
pub mod types;

#[cfg(not(target_arch = "wasm32"))]
pub mod native;

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub mod web;

pub use call::{SignalingIO, VideoCall};
pub use peer::{PeerConnection, RemoteTrack};

pub use types::{
    AudioCodec, ConnectionState, IceCandidate, IceServer, RtcConfig, RtcError, SdpType,
    SessionDescription, SignalingMessage, VideoCodec,
};
#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub use web::{WebPeerConnection, WebRemoteTrack};
