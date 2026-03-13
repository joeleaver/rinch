//! Shared WebRTC types used by both native and WASM backends.

use serde::{Deserialize, Serialize};
use std::fmt;

/// SDP message type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SdpType {
    Offer,
    Answer,
    Pranswer,
}

/// An SDP session description (offer or answer).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDescription {
    pub sdp_type: SdpType,
    pub sdp: String,
}

/// An ICE candidate for peer connectivity checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IceCandidate {
    pub candidate: String,
    pub sdp_mid: Option<String>,
    pub sdp_m_line_index: Option<u16>,
}

/// Messages exchanged over a signaling channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SignalingMessage {
    Offer(SessionDescription),
    Answer(SessionDescription),
    IceCandidate(IceCandidate),
    Bye,
}

/// Video codec preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoCodec {
    VP8,
    VP9,
    H264,
    AV1,
}

/// Audio codec preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioCodec {
    Opus,
}

/// An ICE/TURN server configuration entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IceServer {
    pub urls: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

/// Configuration for creating a [`PeerConnection`](crate::PeerConnection).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RtcConfig {
    pub ice_servers: Vec<IceServer>,
    pub video_codec: VideoCodec,
    pub audio_codec: AudioCodec,
}

impl Default for RtcConfig {
    fn default() -> Self {
        Self {
            ice_servers: vec![IceServer {
                urls: vec!["stun:stun.l.google.com:19302".into()],
                username: None,
                credential: None,
            }],
            video_codec: VideoCodec::VP8,
            audio_codec: AudioCodec::Opus,
        }
    }
}

/// Peer connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionState {
    New,
    Connecting,
    Connected,
    Disconnected,
    Failed,
    Closed,
}

/// Errors from WebRTC operations.
#[derive(Debug, Clone)]
pub enum RtcError {
    Sdp(String),
    Ice(String),
    Codec(String),
    Signaling(String),
    Transport(String),
    Internal(String),
}

impl fmt::Display for RtcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RtcError::Sdp(msg) => write!(f, "SDP error: {msg}"),
            RtcError::Ice(msg) => write!(f, "ICE error: {msg}"),
            RtcError::Codec(msg) => write!(f, "Codec error: {msg}"),
            RtcError::Signaling(msg) => write!(f, "Signaling error: {msg}"),
            RtcError::Transport(msg) => write!(f, "Transport error: {msg}"),
            RtcError::Internal(msg) => write!(f, "Internal error: {msg}"),
        }
    }
}

impl std::error::Error for RtcError {}
