//! WASM WebSocket signaling via web_sys.

use std::sync::Mutex;
use std::sync::mpsc;

use wasm_bindgen::prelude::*;
use web_sys::{MessageEvent, WebSocket};

use crate::error::SignalingError;
use crate::{SignalingChannel, SignalingMessage};

/// A signaling channel that communicates over a browser WebSocket.
///
/// Messages are serialized as JSON text frames. Incoming messages are
/// queued into an `mpsc` channel so that `recv()` can block (on the
/// calling thread) until a message arrives.
pub struct WebSocketSignaling {
    ws: WebSocket,
    rx: Mutex<mpsc::Receiver<Result<SignalingMessage, SignalingError>>>,
    // Hold closures to prevent them from being dropped
    _on_message: Closure<dyn FnMut(MessageEvent)>,
    _on_close: Closure<dyn FnMut()>,
}

// Safety: WebSocket signaling on WASM is single-threaded.
// The Send bound is required by the trait but WASM has no real threads.
unsafe impl Send for WebSocketSignaling {}

impl WebSocketSignaling {
    /// Connect to a WebSocket signaling server at the given URL.
    pub fn connect(url: &str) -> Result<Self, SignalingError> {
        let ws =
            WebSocket::new(url).map_err(|e| SignalingError::ConnectionFailed(format!("{e:?}")))?;

        ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

        let (tx, rx) = mpsc::channel();

        let tx_msg = tx.clone();
        let on_message = Closure::wrap(Box::new(move |event: MessageEvent| {
            if let Some(text) = event.data().as_string() {
                let result = serde_json::from_str::<SignalingMessage>(&text)
                    .map_err(|e| SignalingError::Serialization(e.to_string()));
                let _ = tx_msg.send(result);
            }
        }) as Box<dyn FnMut(MessageEvent)>);

        let tx_close = tx;
        let on_close = Closure::wrap(Box::new(move || {
            let _ = tx_close.send(Err(SignalingError::Closed));
        }) as Box<dyn FnMut()>);

        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));

        Ok(Self {
            ws,
            rx: Mutex::new(rx),
            _on_message: on_message,
            _on_close: on_close,
        })
    }
}

impl SignalingChannel for WebSocketSignaling {
    fn send(&self, msg: SignalingMessage) -> Result<(), SignalingError> {
        let json = serde_json::to_string(&msg)
            .map_err(|e| SignalingError::Serialization(e.to_string()))?;

        self.ws
            .send_with_str(&json)
            .map_err(|e| SignalingError::Io(format!("{e:?}")))?;

        Ok(())
    }

    fn recv(&self) -> Result<SignalingMessage, SignalingError> {
        let rx = self.rx.lock().unwrap();
        rx.recv().map_err(|_| SignalingError::Closed)?
    }

    fn close(&self) {
        let _ = self.ws.close();
    }
}
