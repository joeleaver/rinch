//! NativePeerConnection — str0m-based WebRTC peer connection for desktop.

use std::net::UdpSocket;
use std::sync::{Arc, Mutex};

use str0m::change::SdpPendingOffer;
use str0m::media::{Direction, MediaKind};
use str0m::{Candidate, Rtc};

use super::transport::{self, NetCommand, NetEvent, NetThreadHandle};
use crate::types::{
    ConnectionState, IceCandidate, RtcConfig, RtcError, SdpType, SessionDescription,
};

type BoxCallback<T> = Box<dyn Fn(T) + Send + 'static>;

/// Thread-safe callback storage.
struct Callbacks {
    on_ice_candidate: Option<BoxCallback<IceCandidate>>,
    on_track: Option<BoxCallback<RemoteMediaTrack>>,
    on_state_change: Option<BoxCallback<ConnectionState>>,
    on_media_data: Option<BoxCallback<IncomingMedia>>,
}

/// Information about a remote media track.
#[derive(Debug, Clone)]
pub struct RemoteMediaTrack {
    pub mid: String,
    pub kind: MediaKind,
}

/// Incoming encoded media data from the remote peer.
#[derive(Debug, Clone)]
pub struct IncomingMedia {
    pub mid: String,
    pub kind: MediaKind,
    pub data: Vec<u8>,
}

/// A native WebRTC peer connection backed by str0m.
pub struct NativePeerConnection {
    /// The network thread handle. None before start.
    net: Option<NetThreadHandle>,
    /// The pending offer from create_offer(), needed for accept_answer.
    pending_offer: Option<SdpPendingOffer>,
    /// Local socket address.
    local_addr: Option<std::net::SocketAddr>,
    /// Callbacks.
    callbacks: Arc<Mutex<Callbacks>>,
    /// Connection state.
    state: Mutex<ConnectionState>,
    /// Configuration (retained for future renegotiation).
    #[allow(dead_code)]
    config: RtcConfig,
}

impl NativePeerConnection {
    /// Create a new native peer connection.
    pub fn new(config: RtcConfig) -> Result<Self, RtcError> {
        Ok(Self {
            net: None,
            pending_offer: None,
            local_addr: None,
            callbacks: Arc::new(Mutex::new(Callbacks {
                on_ice_candidate: None,
                on_track: None,
                on_state_change: None,
                on_media_data: None,
            })),
            state: Mutex::new(ConnectionState::New),
            config,
        })
    }

    /// Bind a UDP socket and create an Rtc instance, returning (rtc, socket, candidate).
    fn setup_transport(&mut self) -> Result<(Rtc, UdpSocket), RtcError> {
        let socket = UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| RtcError::Transport(format!("bind UDP: {e}")))?;
        let local_addr = socket
            .local_addr()
            .map_err(|e| RtcError::Transport(format!("local_addr: {e}")))?;
        self.local_addr = Some(local_addr);

        let mut rtc = Rtc::new();

        // Add our local UDP address as a host candidate.
        // Use 127.0.0.1 for the candidate address since 0.0.0.0 is not a valid ICE candidate.
        let candidate_addr = if local_addr.ip().is_unspecified() {
            std::net::SocketAddr::new(
                std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                local_addr.port(),
            )
        } else {
            local_addr
        };

        let candidate = Candidate::host(candidate_addr, "udp")
            .map_err(|e| RtcError::Ice(format!("create candidate: {e}")))?;
        rtc.add_local_candidate(candidate);

        Ok((rtc, socket))
    }

    /// Create an SDP offer. This starts the network thread.
    pub fn create_offer(
        &mut self,
        send_audio: bool,
        send_video: bool,
    ) -> Result<SessionDescription, RtcError> {
        let (mut rtc, socket) = self.setup_transport()?;

        // Add media lines.
        let mut api = rtc.sdp_api();
        if send_audio {
            api.add_media(MediaKind::Audio, Direction::SendRecv, None, None, None);
        }
        if send_video {
            api.add_media(MediaKind::Video, Direction::SendRecv, None, None, None);
        }

        let (offer, pending) = api
            .apply()
            .ok_or_else(|| RtcError::Sdp("no media lines to negotiate".into()))?;

        self.pending_offer = Some(pending);

        // Emit local ICE candidate.
        if let Some(addr) = self.local_addr {
            let candidate_addr = if addr.ip().is_unspecified() {
                std::net::SocketAddr::new(
                    std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                    addr.port(),
                )
            } else {
                addr
            };
            if let Ok(c) = Candidate::host(candidate_addr, "udp") {
                let cbs = self.callbacks.lock().unwrap();
                if let Some(ref cb) = cbs.on_ice_candidate {
                    cb(IceCandidate {
                        candidate: c.to_sdp_string(),
                        sdp_mid: Some("0".into()),
                        sdp_m_line_index: Some(0),
                    });
                }
            }
        }

        // Start network thread.
        let handle = transport::spawn_network_thread(rtc, socket);
        self.net = Some(handle);

        let sdp = offer.to_sdp_string();
        Ok(SessionDescription {
            sdp_type: SdpType::Offer,
            sdp,
        })
    }

    /// Set the remote SDP answer (after creating an offer).
    pub fn set_remote_answer(&mut self, answer: SessionDescription) -> Result<(), RtcError> {
        let pending = self
            .pending_offer
            .take()
            .ok_or_else(|| RtcError::Sdp("no pending offer".into()))?;

        let net = self
            .net
            .as_ref()
            .ok_or_else(|| RtcError::Internal("network thread not started".into()))?;

        net.cmd_tx
            .send(NetCommand::AcceptAnswer {
                answer_sdp: answer.sdp,
                pending,
            })
            .map_err(|_| RtcError::Internal("network thread died".into()))?;

        Ok(())
    }

    /// Accept a remote SDP offer and produce an answer. This starts the network thread.
    pub fn create_answer(
        &mut self,
        offer: SessionDescription,
    ) -> Result<SessionDescription, RtcError> {
        let (rtc, socket) = self.setup_transport()?;

        // Emit local ICE candidate.
        if let Some(addr) = self.local_addr {
            let candidate_addr = if addr.ip().is_unspecified() {
                std::net::SocketAddr::new(
                    std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                    addr.port(),
                )
            } else {
                addr
            };
            if let Ok(c) = Candidate::host(candidate_addr, "udp") {
                let cbs = self.callbacks.lock().unwrap();
                if let Some(ref cb) = cbs.on_ice_candidate {
                    cb(IceCandidate {
                        candidate: c.to_sdp_string(),
                        sdp_mid: Some("0".into()),
                        sdp_m_line_index: Some(0),
                    });
                }
            }
        }

        // Start network thread — it will accept the offer.
        let handle = transport::spawn_network_thread(rtc, socket);

        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        handle
            .cmd_tx
            .send(NetCommand::AcceptOffer {
                offer_sdp: offer.sdp,
                reply: reply_tx,
            })
            .map_err(|_| RtcError::Internal("network thread died".into()))?;

        let answer_sdp = reply_rx
            .recv()
            .map_err(|_| RtcError::Internal("network thread died".into()))?
            .map_err(RtcError::Sdp)?;

        self.net = Some(handle);

        Ok(SessionDescription {
            sdp_type: SdpType::Answer,
            sdp: answer_sdp,
        })
    }

    /// Add a remote ICE candidate.
    pub fn add_ice_candidate(&self, candidate: IceCandidate) -> Result<(), RtcError> {
        let net = self
            .net
            .as_ref()
            .ok_or_else(|| RtcError::Internal("network thread not started".into()))?;

        net.cmd_tx
            .send(NetCommand::AddRemoteCandidate(candidate.candidate))
            .map_err(|_| RtcError::Internal("network thread died".into()))?;

        Ok(())
    }

    /// Register a callback for local ICE candidates.
    pub fn on_ice_candidate(&self, cb: impl Fn(IceCandidate) + Send + 'static) {
        self.callbacks.lock().unwrap().on_ice_candidate = Some(Box::new(cb));
    }

    /// Register a callback for remote track notifications.
    pub fn on_track(&self, cb: impl Fn(RemoteMediaTrack) + Send + 'static) {
        self.callbacks.lock().unwrap().on_track = Some(Box::new(cb));
    }

    /// Register a callback for connection state changes.
    pub fn on_connection_state_change(&self, cb: impl Fn(ConnectionState) + Send + 'static) {
        self.callbacks.lock().unwrap().on_state_change = Some(Box::new(cb));
    }

    /// Register a callback for incoming media data.
    pub fn on_media_data(&self, cb: impl Fn(IncomingMedia) + Send + 'static) {
        self.callbacks.lock().unwrap().on_media_data = Some(Box::new(cb));
    }

    /// Poll for events from the network thread. Call this periodically from the main thread.
    pub fn poll(&self) {
        let net = match &self.net {
            Some(n) => n,
            None => return,
        };

        let rx = net.event_rx.lock().unwrap();
        while let Ok(event) = rx.try_recv() {
            let cbs = self.callbacks.lock().unwrap();
            match event {
                NetEvent::IceCandidate(candidate_str) => {
                    if let Some(ref cb) = cbs.on_ice_candidate {
                        cb(IceCandidate {
                            candidate: candidate_str,
                            sdp_mid: None,
                            sdp_m_line_index: None,
                        });
                    }
                }
                NetEvent::StateChange(state) => {
                    drop(cbs);
                    *self.state.lock().unwrap() = state;
                    let cbs = self.callbacks.lock().unwrap();
                    if let Some(ref cb) = cbs.on_state_change {
                        cb(state);
                    }
                }
                NetEvent::MediaAdded { mid, kind } => {
                    if let Some(ref cb) = cbs.on_track {
                        cb(RemoteMediaTrack {
                            mid: mid.to_string(),
                            kind,
                        });
                    }
                }
                NetEvent::MediaData { mid, data, kind } => {
                    if let Some(ref cb) = cbs.on_media_data {
                        cb(IncomingMedia {
                            mid: mid.to_string(),
                            kind,
                            data,
                        });
                    }
                }
            }
        }
    }

    /// Get the current connection state.
    pub fn connection_state(&self) -> ConnectionState {
        *self.state.lock().unwrap()
    }

    /// Get a sender for the network command channel.
    ///
    /// Used by encoder threads to send encoded media to the network thread.
    /// Returns None if the network thread hasn't started.
    pub(crate) fn cmd_sender(&self) -> Option<std::sync::mpsc::Sender<NetCommand>> {
        self.net.as_ref().map(|n| n.cmd_tx.clone())
    }

    /// Close the connection.
    pub fn close(&mut self) {
        if let Some(net) = self.net.take() {
            let _ = net.cmd_tx.send(NetCommand::Shutdown);
            // Drop will join the thread.
        }
        *self.state.lock().unwrap() = ConnectionState::Closed;
    }
}
