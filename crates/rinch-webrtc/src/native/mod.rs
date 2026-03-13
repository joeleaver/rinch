//! Native WebRTC backend using str0m.

#[cfg(feature = "native")]
pub mod audio_codec;
#[cfg(feature = "native")]
mod peer;
#[cfg(feature = "native")]
pub(crate) mod transport;

#[cfg(feature = "native")]
pub use peer::{IncomingMedia, NativePeerConnection, RemoteMediaTrack};
