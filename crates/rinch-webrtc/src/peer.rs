//! PeerConnection — platform-abstracted WebRTC peer connection.

use crate::types::{ConnectionState, IceCandidate, RtcConfig, RtcError, SessionDescription};
use rinch_av::{TrackHandle, TrackKind};

/// A track received from the remote peer.
pub struct RemoteTrack {
    pub kind: TrackKind,
    pub handle: TrackHandle,
}

/// A WebRTC peer connection.
///
/// Wraps platform-specific internals (str0m on native, web_sys on WASM).
pub struct PeerConnection {
    #[cfg(all(not(target_arch = "wasm32"), feature = "native"))]
    inner: crate::native::NativePeerConnection,
    #[cfg(all(not(target_arch = "wasm32"), not(feature = "native")))]
    _config: RtcConfig,
    #[cfg(all(target_arch = "wasm32", feature = "web"))]
    web: crate::web::WebPeerConnection,
    #[cfg(all(target_arch = "wasm32", not(feature = "web")))]
    _config: RtcConfig,
}

impl PeerConnection {
    /// Create a new peer connection with the given configuration.
    pub fn new(config: RtcConfig) -> Result<Self, RtcError> {
        #[cfg(all(not(target_arch = "wasm32"), feature = "native"))]
        {
            Ok(Self {
                inner: crate::native::NativePeerConnection::new(config)?,
            })
        }
        #[cfg(all(not(target_arch = "wasm32"), not(feature = "native")))]
        {
            Ok(Self { _config: config })
        }
        #[cfg(all(target_arch = "wasm32", feature = "web"))]
        {
            let web = crate::web::WebPeerConnection::new(&config)?;
            Ok(Self { web })
        }
        #[cfg(all(target_arch = "wasm32", not(feature = "web")))]
        {
            Ok(Self { _config: config })
        }
    }

    /// Add a local track to be sent to the remote peer.
    pub fn add_track(&self, _track: &TrackHandle) -> Result<(), RtcError> {
        // Native: will be implemented in Task #4
        // WASM: tracks are added via add_stream on WebPeerConnection (using browser MediaStream)
        Err(RtcError::Internal("not implemented".into()))
    }

    /// Create an SDP offer.
    ///
    /// On WASM, this is async internally. Use `create_offer_async` for the
    /// async variant on WASM.
    pub fn create_offer(
        &mut self,
        send_audio: bool,
        send_video: bool,
    ) -> Result<SessionDescription, RtcError> {
        #[cfg(all(not(target_arch = "wasm32"), feature = "native"))]
        {
            self.inner.create_offer(send_audio, send_video)
        }
        #[cfg(all(not(target_arch = "wasm32"), not(feature = "native")))]
        {
            let _ = (send_audio, send_video);
            Err(RtcError::Internal(
                "native WebRTC not enabled — add features = [\"native\"]".into(),
            ))
        }
        #[cfg(all(target_arch = "wasm32", feature = "web"))]
        {
            let _ = (send_audio, send_video);
            Err(RtcError::Internal(
                "use create_offer_async() on WASM".into(),
            ))
        }
        #[cfg(all(target_arch = "wasm32", not(feature = "web")))]
        {
            let _ = (send_audio, send_video);
            Err(RtcError::Internal(
                "WASM WebRTC not enabled — add features = [\"web\"]".into(),
            ))
        }
    }

    /// Create an SDP answer from a remote offer.
    ///
    /// On WASM, this is async internally. Use `create_answer_async` for the
    /// async variant on WASM.
    pub fn create_answer_from_offer(
        &mut self,
        offer: SessionDescription,
    ) -> Result<SessionDescription, RtcError> {
        #[cfg(all(not(target_arch = "wasm32"), feature = "native"))]
        {
            self.inner.create_answer(offer)
        }
        #[cfg(all(not(target_arch = "wasm32"), not(feature = "native")))]
        {
            let _ = offer;
            Err(RtcError::Internal("not implemented".into()))
        }
        #[cfg(all(target_arch = "wasm32", feature = "web"))]
        {
            let _ = offer;
            Err(RtcError::Internal(
                "use create_answer_async() on WASM".into(),
            ))
        }
        #[cfg(all(target_arch = "wasm32", not(feature = "web")))]
        {
            let _ = offer;
            Err(RtcError::Internal("not implemented".into()))
        }
    }

    /// Set the local SDP description.
    /// On native, this is handled internally by create_offer/create_answer.
    pub fn set_local_description(&self, _desc: SessionDescription) -> Result<(), RtcError> {
        #[cfg(all(not(target_arch = "wasm32"), feature = "native"))]
        {
            // Handled internally by create_offer/create_answer on native.
            Ok(())
        }
        #[cfg(all(not(target_arch = "wasm32"), not(feature = "native")))]
        {
            Err(RtcError::Internal("not implemented".into()))
        }
        #[cfg(all(target_arch = "wasm32", feature = "web"))]
        {
            Err(RtcError::Internal(
                "use set_local_description_async() on WASM".into(),
            ))
        }
        #[cfg(all(target_arch = "wasm32", not(feature = "web")))]
        {
            Err(RtcError::Internal("not implemented".into()))
        }
    }

    /// Set the remote SDP description (answer).
    pub fn set_remote_description(&mut self, desc: SessionDescription) -> Result<(), RtcError> {
        #[cfg(all(not(target_arch = "wasm32"), feature = "native"))]
        {
            self.inner.set_remote_answer(desc)
        }
        #[cfg(all(not(target_arch = "wasm32"), not(feature = "native")))]
        {
            let _ = desc;
            Err(RtcError::Internal("not implemented".into()))
        }
        #[cfg(all(target_arch = "wasm32", feature = "web"))]
        {
            let _ = desc;
            Err(RtcError::Internal(
                "use set_remote_description_async() on WASM".into(),
            ))
        }
        #[cfg(all(target_arch = "wasm32", not(feature = "web")))]
        {
            let _ = desc;
            Err(RtcError::Internal("not implemented".into()))
        }
    }

    /// Add a remote ICE candidate.
    pub fn add_ice_candidate(&self, candidate: IceCandidate) -> Result<(), RtcError> {
        #[cfg(all(not(target_arch = "wasm32"), feature = "native"))]
        {
            self.inner.add_ice_candidate(candidate)
        }
        #[cfg(all(not(target_arch = "wasm32"), not(feature = "native")))]
        {
            let _ = candidate;
            Err(RtcError::Internal("not implemented".into()))
        }
        #[cfg(all(target_arch = "wasm32", feature = "web"))]
        {
            let _ = candidate;
            Err(RtcError::Internal(
                "use add_ice_candidate_async() on WASM".into(),
            ))
        }
        #[cfg(all(target_arch = "wasm32", not(feature = "web")))]
        {
            let _ = candidate;
            Err(RtcError::Internal("not implemented".into()))
        }
    }

    /// Register a callback for when a local ICE candidate is gathered.
    pub fn on_ice_candidate(&self, cb: impl Fn(IceCandidate) + Send + 'static) {
        #[cfg(all(not(target_arch = "wasm32"), feature = "native"))]
        {
            self.inner.on_ice_candidate(cb);
        }
        #[cfg(all(not(target_arch = "wasm32"), not(feature = "native")))]
        {
            let _ = cb;
        }
        #[cfg(all(target_arch = "wasm32", feature = "web"))]
        {
            self.web.on_ice_candidate(cb);
        }
        #[cfg(all(target_arch = "wasm32", not(feature = "web")))]
        {
            let _ = cb;
        }
    }

    /// Register a callback for when a remote track is received.
    ///
    /// Note: On WASM, the callback receives a `WebRemoteTrack` (browser MediaStreamTrack).
    /// The `RemoteTrack` wrapper with `TrackHandle` is used on native. Use
    /// `on_track_web()` for direct web_sys track access on WASM.
    pub fn on_track(&self, _cb: impl Fn(RemoteTrack) + Send + 'static) {
        // not implemented — native tracks need codec integration (Tasks #5/#6)
    }

    /// Register a callback for connection state changes.
    pub fn on_connection_state_change(&self, cb: impl Fn(ConnectionState) + Send + 'static) {
        #[cfg(all(not(target_arch = "wasm32"), feature = "native"))]
        {
            self.inner.on_connection_state_change(cb);
        }
        #[cfg(all(not(target_arch = "wasm32"), not(feature = "native")))]
        {
            let _ = cb;
        }
        #[cfg(all(target_arch = "wasm32", feature = "web"))]
        {
            self.web.on_connection_state_change(cb);
        }
        #[cfg(all(target_arch = "wasm32", not(feature = "web")))]
        {
            let _ = cb;
        }
    }

    /// Poll for events from the network thread. Call periodically on native.
    pub fn poll(&self) {
        #[cfg(all(not(target_arch = "wasm32"), feature = "native"))]
        {
            self.inner.poll();
        }
    }

    /// Get a command sender for the network thread (native only).
    #[cfg(all(not(target_arch = "wasm32"), feature = "native"))]
    pub(crate) fn inner_cmd_sender(
        &self,
    ) -> Option<std::sync::mpsc::Sender<crate::native::transport::NetCommand>> {
        self.inner.cmd_sender()
    }

    /// Close the peer connection.
    pub fn close(&mut self) {
        #[cfg(all(not(target_arch = "wasm32"), feature = "native"))]
        {
            self.inner.close();
        }
        #[cfg(all(target_arch = "wasm32", feature = "web"))]
        {
            self.web.close();
        }
    }
}

// --- WASM-specific async API ---

#[cfg(all(target_arch = "wasm32", feature = "web"))]
impl PeerConnection {
    /// Get a reference to the underlying `WebPeerConnection`.
    pub fn web(&self) -> &crate::web::WebPeerConnection {
        &self.web
    }

    /// Create an SDP offer (async, WASM only).
    pub async fn create_offer_async(&self) -> Result<SessionDescription, RtcError> {
        self.web.create_offer().await
    }

    /// Create an SDP answer (async, WASM only).
    pub async fn create_answer_async(&self) -> Result<SessionDescription, RtcError> {
        self.web.create_answer().await
    }

    /// Set the local SDP description (async, WASM only).
    pub async fn set_local_description_async(
        &self,
        desc: &SessionDescription,
    ) -> Result<(), RtcError> {
        self.web.set_local_description(desc).await
    }

    /// Set the remote SDP description (async, WASM only).
    pub async fn set_remote_description_async(
        &self,
        desc: &SessionDescription,
    ) -> Result<(), RtcError> {
        self.web.set_remote_description(desc).await
    }

    /// Add a remote ICE candidate (async, WASM only).
    pub async fn add_ice_candidate_async(&self, candidate: &IceCandidate) -> Result<(), RtcError> {
        self.web.add_ice_candidate(candidate).await
    }

    /// Register a callback for remote tracks (WASM, gives raw browser tracks).
    pub fn on_track_web(&self, cb: impl Fn(crate::web::WebRemoteTrack) + 'static) {
        self.web.on_track(cb);
    }

    /// Add a browser MediaStream's tracks to the connection.
    pub fn add_web_stream(&self, stream: &web_sys::MediaStream) {
        self.web.add_stream(stream);
    }
}
