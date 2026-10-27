//! Errors from a peer, a channel, and a refused send.

use std::fmt;

/// Something a [`Peer`](crate::Peer) or [`Channel`](crate::Channel) failed to
/// do, or a problem it reports.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RtcError {
    /// The call is not valid in the peer's current state: `create_answer`
    /// with no remote offer set, a call on a closed peer.
    InvalidState(String),
    /// A session description the stack could not parse or apply.
    Sdp(String),
    /// A remote ICE candidate the stack could not parse or add (reported
    /// through [`Peer::on_error`](crate::Peer::on_error)).
    Candidate(String),
    /// An ICE candidate error: a STUN or TURN server that could not be
    /// reached or refused the credentials. Informational: the connection
    /// state says whether the peer still connects.
    Ice {
        /// The STUN/TURN URL the error came from.
        url: String,
        /// The STUN error code (e.g. `401`), or `0` when there is none.
        code: u16,
        /// The server's or the stack's description.
        text: String,
    },
    /// A data channel failed. Followed by the channel's `on_close`.
    Channel(String),
    /// The peer is closed.
    Closed,
    /// The stack failed underneath rinch (on native: the worker thread).
    Internal(String),
    /// A media track could not be sent or played (the `media` feature):
    /// remote audio the browser's autoplay policy refused, a codec that
    /// failed.
    Media(String),
}

impl fmt::Display for RtcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RtcError::InvalidState(m) => write!(f, "invalid state: {m}"),
            RtcError::Sdp(m) => write!(f, "session description: {m}"),
            RtcError::Candidate(m) => write!(f, "ICE candidate: {m}"),
            RtcError::Ice { url, code, text } => write!(f, "ICE server {url}: {code} {text}"),
            RtcError::Channel(m) => write!(f, "data channel: {m}"),
            RtcError::Closed => write!(f, "the peer is closed"),
            RtcError::Internal(m) => write!(f, "internal: {m}"),
            RtcError::Media(m) => write!(f, "media: {m}"),
        }
    }
}

impl std::error::Error for RtcError {}

/// Why [`Channel::send`](crate::Channel::send) refused a message. A refused
/// message is not queued, and the channel stays as it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    /// The channel is not open (still connecting, or closing or closed).
    NotOpen,
    /// The message is larger than the peer's advertised max-message-size.
    TooLarge {
        /// The message's length.
        len: usize,
        /// [`Channel::max_message_size`](crate::Channel::max_message_size).
        max: usize,
    },
    /// Queueing it would take the channel's buffered amount past its
    /// [`send_buffer_limit`](crate::ChannelOptions::send_buffer_limit). Wait
    /// for [`on_buffered_amount_low`](crate::Channel::on_buffered_amount_low),
    /// or on an unreliable channel drop it and send the next fresh one.
    QueueFull {
        /// The buffered amount when the send was refused.
        buffered: usize,
        /// The channel's limit.
        limit: usize,
    },
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendError::NotOpen => write!(f, "the channel is not open"),
            SendError::TooLarge { len, max } => {
                write!(
                    f,
                    "a {len}-byte message exceeds the peer's {max}-byte limit"
                )
            }
            SendError::QueueFull { buffered, limit } => {
                write!(f, "send queue full ({buffered} of {limit} bytes buffered)")
            }
        }
    }
}

impl std::error::Error for SendError {}
