//! WASM WebRTC backend — wraps browser RTCPeerConnection via web_sys.

use crate::types::{
    ConnectionState, IceCandidate, RtcConfig, RtcError, SdpType, SessionDescription,
};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::{
    MediaStream, MediaStreamTrack, RtcIceCandidate, RtcIceCandidateInit, RtcPeerConnection,
    RtcPeerConnectionIceEvent, RtcSdpType, RtcSessionDescriptionInit, RtcTrackEvent,
};

/// Convert a JsValue error into an RtcError.
fn js_err(val: JsValue) -> RtcError {
    let msg = val
        .as_string()
        .or_else(|| js_sys::JSON::stringify(&val).ok().map(|s| s.into()))
        .unwrap_or_else(|| "unknown JS error".into());
    RtcError::Internal(msg)
}

/// Convert our SdpType to the web_sys enum.
fn to_rtc_sdp_type(t: SdpType) -> RtcSdpType {
    match t {
        SdpType::Offer => RtcSdpType::Offer,
        SdpType::Answer => RtcSdpType::Answer,
        SdpType::Pranswer => RtcSdpType::Pranswer,
    }
}

/// Stored closures that must be kept alive to prevent GC from collecting them.
struct WebClosures {
    _on_ice_candidate: Option<Closure<dyn FnMut(RtcPeerConnectionIceEvent)>>,
    _on_track: Option<Closure<dyn FnMut(RtcTrackEvent)>>,
    _on_connection_state_change: Option<Closure<dyn FnMut()>>,
}

/// A remote track received from the browser's RTCPeerConnection.
pub struct WebRemoteTrack {
    pub track: MediaStreamTrack,
    pub streams: Vec<MediaStream>,
}

/// WASM WebRTC peer connection wrapping browser's RTCPeerConnection.
pub struct WebPeerConnection {
    inner: RtcPeerConnection,
    closures: Rc<RefCell<WebClosures>>,
}

impl WebPeerConnection {
    /// Create a new WebPeerConnection backed by the browser's RTCPeerConnection.
    pub fn new(config: &RtcConfig) -> Result<Self, RtcError> {
        // Build RtcConfiguration
        let rtc_config = web_sys::RtcConfiguration::new();

        // Set ICE servers
        let ice_servers = js_sys::Array::new();
        for server in &config.ice_servers {
            let ice_server = web_sys::RtcIceServer::new();
            let urls = js_sys::Array::new();
            for url in &server.urls {
                urls.push(&JsValue::from_str(url));
            }
            ice_server.set_urls(&urls.into());
            if let Some(ref username) = server.username {
                ice_server.set_username(username);
            }
            if let Some(ref credential) = server.credential {
                ice_server.set_credential(credential);
            }
            ice_servers.push(&ice_server);
        }
        rtc_config.set_ice_servers(&ice_servers.into());

        let inner = RtcPeerConnection::new_with_configuration(&rtc_config).map_err(js_err)?;

        Ok(Self {
            inner,
            closures: Rc::new(RefCell::new(WebClosures {
                _on_ice_candidate: None,
                _on_track: None,
                _on_connection_state_change: None,
            })),
        })
    }

    /// Get a reference to the underlying RTCPeerConnection.
    pub fn raw(&self) -> &RtcPeerConnection {
        &self.inner
    }

    /// Create an SDP offer.
    pub async fn create_offer(&self) -> Result<SessionDescription, RtcError> {
        let promise = self.inner.create_offer();
        let js_desc = wasm_bindgen_futures::JsFuture::from(promise)
            .await
            .map_err(js_err)?;

        let sdp = js_sys::Reflect::get(&js_desc, &"sdp".into())
            .map_err(js_err)?
            .as_string()
            .ok_or_else(|| RtcError::Sdp("missing sdp field".into()))?;

        Ok(SessionDescription {
            sdp_type: SdpType::Offer,
            sdp,
        })
    }

    /// Create an SDP answer.
    pub async fn create_answer(&self) -> Result<SessionDescription, RtcError> {
        let promise = self.inner.create_answer();
        let js_desc = wasm_bindgen_futures::JsFuture::from(promise)
            .await
            .map_err(js_err)?;

        let sdp = js_sys::Reflect::get(&js_desc, &"sdp".into())
            .map_err(js_err)?
            .as_string()
            .ok_or_else(|| RtcError::Sdp("missing sdp field".into()))?;

        Ok(SessionDescription {
            sdp_type: SdpType::Answer,
            sdp,
        })
    }

    /// Set the local SDP description.
    pub async fn set_local_description(&self, desc: &SessionDescription) -> Result<(), RtcError> {
        let mut init = RtcSessionDescriptionInit::new(to_rtc_sdp_type(desc.sdp_type));
        init.sdp(&desc.sdp);
        let promise = self.inner.set_local_description(&init);
        wasm_bindgen_futures::JsFuture::from(promise)
            .await
            .map_err(js_err)?;
        Ok(())
    }

    /// Set the remote SDP description.
    pub async fn set_remote_description(&self, desc: &SessionDescription) -> Result<(), RtcError> {
        let mut init = RtcSessionDescriptionInit::new(to_rtc_sdp_type(desc.sdp_type));
        init.sdp(&desc.sdp);
        let promise = self.inner.set_remote_description(&init);
        wasm_bindgen_futures::JsFuture::from(promise)
            .await
            .map_err(js_err)?;
        Ok(())
    }

    /// Add a remote ICE candidate.
    pub async fn add_ice_candidate(&self, candidate: &IceCandidate) -> Result<(), RtcError> {
        let mut init = RtcIceCandidateInit::new(&candidate.candidate);
        if let Some(ref mid) = candidate.sdp_mid {
            init.sdp_mid(Some(mid));
        }
        if let Some(idx) = candidate.sdp_m_line_index {
            init.sdp_m_line_index(Some(idx));
        }
        let rtc_candidate = RtcIceCandidate::new(&init).map_err(js_err)?;
        let promise = self
            .inner
            .add_ice_candidate_with_opt_rtc_ice_candidate(Some(&rtc_candidate));
        wasm_bindgen_futures::JsFuture::from(promise)
            .await
            .map_err(js_err)?;
        Ok(())
    }

    /// Add a local MediaStream's tracks to the peer connection.
    pub fn add_stream(&self, stream: &MediaStream) {
        let tracks = stream.get_tracks();
        for i in 0..tracks.length() {
            if let Ok(track) = tracks.get(i).dyn_into::<MediaStreamTrack>() {
                self.inner.add_track_0(&track, stream);
            }
        }
    }

    /// Register a callback for when a local ICE candidate is gathered.
    pub fn on_ice_candidate(&self, cb: impl Fn(IceCandidate) + 'static) {
        let closure = Closure::<dyn FnMut(RtcPeerConnectionIceEvent)>::new(
            move |event: RtcPeerConnectionIceEvent| {
                if let Some(candidate) = event.candidate() {
                    let ice = IceCandidate {
                        candidate: candidate.candidate(),
                        sdp_mid: candidate.sdp_mid(),
                        sdp_m_line_index: candidate.sdp_m_line_index(),
                    };
                    cb(ice);
                }
            },
        );
        self.inner
            .set_onicecandidate(Some(closure.as_ref().unchecked_ref()));
        self.closures.borrow_mut()._on_ice_candidate = Some(closure);
    }

    /// Register a callback for when a remote track is received.
    pub fn on_track(&self, cb: impl Fn(WebRemoteTrack) + 'static) {
        let closure = Closure::<dyn FnMut(RtcTrackEvent)>::new(move |event: RtcTrackEvent| {
            let track = event.track();
            let js_streams = event.streams();
            let mut streams = Vec::new();
            for i in 0..js_streams.length() {
                if let Ok(s) = js_streams.get(i).dyn_into::<MediaStream>() {
                    streams.push(s);
                }
            }
            cb(WebRemoteTrack { track, streams });
        });
        self.inner
            .set_ontrack(Some(closure.as_ref().unchecked_ref()));
        self.closures.borrow_mut()._on_track = Some(closure);
    }

    /// Register a callback for connection state changes.
    pub fn on_connection_state_change(&self, cb: impl Fn(ConnectionState) + 'static) {
        let pc = self.inner.clone();
        let closure = Closure::<dyn FnMut()>::new(move || {
            let state = map_connection_state(&pc);
            cb(state);
        });
        self.inner
            .set_onconnectionstatechange(Some(closure.as_ref().unchecked_ref()));
        self.closures.borrow_mut()._on_connection_state_change = Some(closure);
    }

    /// Get the current connection state.
    pub fn connection_state(&self) -> ConnectionState {
        map_connection_state(&self.inner)
    }

    /// Close the peer connection.
    pub fn close(&self) {
        self.inner.close();
    }
}

/// Map the browser's RTCPeerConnectionState to our ConnectionState.
fn map_connection_state(pc: &RtcPeerConnection) -> ConnectionState {
    // connection_state() returns RtcPeerConnectionState enum
    match pc.connection_state() {
        web_sys::RtcPeerConnectionState::New => ConnectionState::New,
        web_sys::RtcPeerConnectionState::Connecting => ConnectionState::Connecting,
        web_sys::RtcPeerConnectionState::Connected => ConnectionState::Connected,
        web_sys::RtcPeerConnectionState::Disconnected => ConnectionState::Disconnected,
        web_sys::RtcPeerConnectionState::Failed => ConnectionState::Failed,
        web_sys::RtcPeerConnectionState::Closed => ConnectionState::Closed,
        _ => ConnectionState::New,
    }
}

impl Drop for WebPeerConnection {
    fn drop(&mut self) {
        self.inner.set_onicecandidate(None);
        self.inner.set_ontrack(None);
        self.inner.set_onconnectionstatechange(None);
        self.inner.close();
    }
}
