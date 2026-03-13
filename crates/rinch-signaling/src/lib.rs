//! WebRTC signaling channel trait and WebSocket implementation for rinch.
//!
//! Provides [`SignalingChannel`] for exchanging SDP offers/answers and ICE
//! candidates between peers. The default implementation uses WebSockets
//! (tungstenite on desktop, web_sys on WASM).

mod error;

#[cfg(not(target_arch = "wasm32"))]
mod websocket;

#[cfg(target_arch = "wasm32")]
mod websocket_wasm;

pub use error::SignalingError;
pub use rinch_webrtc::SignalingMessage;

#[cfg(not(target_arch = "wasm32"))]
pub use websocket::WebSocketSignaling;

#[cfg(target_arch = "wasm32")]
pub use websocket_wasm::WebSocketSignaling;

/// A channel for exchanging signaling messages with a remote peer.
///
/// Implementations must be `Send + 'static` so they can be used from
/// background threads on desktop platforms.
pub trait SignalingChannel: Send + 'static {
    /// Send a signaling message to the remote peer.
    fn send(&self, msg: SignalingMessage) -> Result<(), SignalingError>;

    /// Block until a signaling message is received from the remote peer.
    fn recv(&self) -> Result<SignalingMessage, SignalingError>;

    /// Close the signaling channel.
    fn close(&self);
}
