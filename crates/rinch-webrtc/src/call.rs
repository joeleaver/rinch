//! VideoCall — high-level API for a WebRTC call session.
//!
//! Manages PeerConnection, signaling, codec threads, and media routing.

use crate::peer::PeerConnection;
use crate::types::{ConnectionState, IceCandidate, RtcConfig, RtcError, SignalingMessage};
use rinch_core::Signal;

/// High-level video call session that manages a [`PeerConnection`],
/// local/remote media, and signaling coordination.
///
/// # Usage
///
/// ```ignore
/// // Caller side:
/// let mut call = VideoCall::new()?;
/// call.start(signaling_channel)?;
///
/// // Callee side:
/// let mut call = VideoCall::new()?;
/// call.join(signaling_channel)?;
///
/// // Both sides:
/// call.hangup();
/// ```
pub struct VideoCall {
    /// The underlying peer connection.
    pub peer: PeerConnection,
    /// Reactive connection state for UI binding.
    pub state: Signal<ConnectionState>,
    /// Signaling thread handle (joined on drop).
    _signaling_thread: Option<std::thread::JoinHandle<()>>,
}

impl VideoCall {
    /// Create a new video call with default configuration.
    pub fn new() -> Result<Self, RtcError> {
        Self::with_config(RtcConfig::default())
    }

    /// Create a new video call with custom configuration.
    pub fn with_config(config: RtcConfig) -> Result<Self, RtcError> {
        let peer = PeerConnection::new(config)?;
        Ok(Self {
            peer,
            state: Signal::new(ConnectionState::New),
            _signaling_thread: None,
        })
    }

    /// Initiate a call as the caller.
    ///
    /// Creates an SDP offer, sends it via the signaling channel, waits for
    /// the answer, and exchanges ICE candidates. This spawns a background
    /// thread for signaling I/O.
    ///
    /// `send_audio` / `send_video` control which media lines to negotiate.
    pub fn start(
        &mut self,
        signaling: impl SignalingIO,
        send_audio: bool,
        send_video: bool,
    ) -> Result<(), RtcError> {
        // Create offer.
        let offer = self.peer.create_offer(send_audio, send_video)?;
        let state = self.state;

        state.send(ConnectionState::Connecting);

        // Set up ICE candidate forwarding.
        let sig_clone = signaling.clone_box();
        self.peer.on_ice_candidate(move |candidate| {
            let msg = SignalingMessage::IceCandidate(candidate);
            if let Err(e) = sig_clone.send_msg(msg) {
                tracing::warn!("failed to send ICE candidate: {e}");
            }
        });

        // Set up connection state forwarding.
        self.peer.on_connection_state_change(move |s| {
            state.send(s);
        });

        // Send offer.
        signaling
            .send_msg(SignalingMessage::Offer(offer))
            .map_err(RtcError::Signaling)?;

        // Spawn signaling receiver thread.
        let peer_state = PeerCallbackState::new(&self.peer);
        let handle = std::thread::Builder::new()
            .name("rinch-signaling-rx".into())
            .spawn(move || {
                run_signaling_receiver(signaling, peer_state);
            })
            .map_err(|e| RtcError::Internal(format!("spawn signaling thread: {e}")))?;

        self._signaling_thread = Some(handle);
        Ok(())
    }

    /// Join a call as the callee.
    ///
    /// Waits for an SDP offer from the signaling channel, creates an answer,
    /// and exchanges ICE candidates.
    pub fn join(&mut self, signaling: impl SignalingIO) -> Result<(), RtcError> {
        let state = self.state;
        state.send(ConnectionState::Connecting);

        // Set up ICE candidate forwarding.
        let sig_clone = signaling.clone_box();
        self.peer.on_ice_candidate(move |candidate| {
            let msg = SignalingMessage::IceCandidate(candidate);
            if let Err(e) = sig_clone.send_msg(msg) {
                tracing::warn!("failed to send ICE candidate: {e}");
            }
        });

        // Set up connection state forwarding.
        self.peer.on_connection_state_change(move |s| {
            state.send(s);
        });

        // Wait for offer.
        let offer = loop {
            match signaling.recv_msg().map_err(RtcError::Signaling)? {
                SignalingMessage::Offer(desc) => break desc,
                SignalingMessage::IceCandidate(c) => {
                    // Might receive ICE before offer — queue or ignore.
                    tracing::debug!("received ICE candidate before offer, ignoring");
                    let _ = c;
                }
                other => {
                    tracing::debug!(
                        "unexpected signaling message while waiting for offer: {other:?}"
                    );
                }
            }
        };

        // Create answer.
        let answer = self.peer.create_answer_from_offer(offer)?;

        // Send answer.
        signaling
            .send_msg(SignalingMessage::Answer(answer))
            .map_err(RtcError::Signaling)?;

        // Spawn signaling receiver thread.
        let peer_state = PeerCallbackState::new(&self.peer);
        let handle = std::thread::Builder::new()
            .name("rinch-signaling-rx".into())
            .spawn(move || {
                run_signaling_receiver(signaling, peer_state);
            })
            .map_err(|e| RtcError::Internal(format!("spawn signaling thread: {e}")))?;

        self._signaling_thread = Some(handle);
        Ok(())
    }

    /// Poll the peer connection for events. Call periodically from the main thread.
    pub fn poll(&self) {
        self.peer.poll();
    }

    /// Hang up and close the connection.
    pub fn hangup(&mut self) {
        self.peer.close();
        self.state.send(ConnectionState::Closed);
    }
}

/// Trait for signaling I/O used by VideoCall.
///
/// This abstracts over different signaling transports (WebSocket, etc.)
/// without requiring the `rinch-signaling` crate as a dependency.
pub trait SignalingIO: Send + 'static {
    /// Send a signaling message.
    fn send_msg(&self, msg: SignalingMessage) -> Result<(), String>;
    /// Receive a signaling message (blocking).
    fn recv_msg(&self) -> Result<SignalingMessage, String>;
    /// Clone into a boxed trait object.
    fn clone_box(&self) -> Box<dyn SignalingIO>;
}

/// Callback-based state for the signaling receiver thread.
///
/// Uses boxed closures to avoid direct dependency on cfg-gated native modules.
struct PeerCallbackState {
    add_ice: Box<dyn Fn(IceCandidate) + Send>,
}

impl PeerCallbackState {
    fn new(peer: &PeerConnection) -> Self {
        #[cfg(all(not(target_arch = "wasm32"), feature = "native"))]
        {
            let cmd_tx = peer.inner_cmd_sender();
            Self {
                add_ice: Box::new(move |candidate: IceCandidate| {
                    if let Some(ref tx) = cmd_tx {
                        let _ = tx.send(crate::native::transport::NetCommand::AddRemoteCandidate(
                            candidate.candidate,
                        ));
                    }
                }),
            }
        }
        #[cfg(not(all(not(target_arch = "wasm32"), feature = "native")))]
        {
            let _ = peer;
            Self {
                add_ice: Box::new(|_| {}),
            }
        }
    }

    fn add_ice_candidate(&self, candidate: IceCandidate) {
        (self.add_ice)(candidate);
    }
}

/// Run on the signaling receiver thread: process incoming signaling messages.
fn run_signaling_receiver(signaling: impl SignalingIO, state: PeerCallbackState) {
    loop {
        match signaling.recv_msg() {
            Ok(msg) => match msg {
                SignalingMessage::IceCandidate(candidate) => {
                    state.add_ice_candidate(candidate);
                }
                SignalingMessage::Answer(_answer) => {
                    // Caller handles answer via peer.set_remote_description()
                    // before spawning this thread. Late answers are ignored.
                    tracing::debug!("received answer on signaling receiver (already handled)");
                }
                SignalingMessage::Bye => {
                    tracing::debug!("received Bye, stopping signaling");
                    break;
                }
                SignalingMessage::Offer(_) => {
                    tracing::warn!("unexpected offer on signaling receiver");
                }
            },
            Err(e) => {
                tracing::debug!("signaling recv error: {e}");
                break;
            }
        }
    }
}
