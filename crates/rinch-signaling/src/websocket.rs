//! Desktop WebSocket signaling via tungstenite.

use std::sync::{Arc, Mutex};

use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket, connect};

use crate::error::SignalingError;
use crate::{SignalingChannel, SignalingMessage};

/// A signaling channel that communicates over a WebSocket connection.
///
/// Messages are serialized as newline-delimited JSON text frames.
/// Uses tungstenite for synchronous WebSocket I/O (no async runtime).
pub struct WebSocketSignaling {
    socket: Arc<Mutex<WebSocket<MaybeTlsStream<std::net::TcpStream>>>>,
}

impl WebSocketSignaling {
    /// Connect to a WebSocket signaling server at the given URL.
    ///
    /// The URL should be in the form `ws://host:port/room/{room_id}`.
    pub fn connect(url: &str) -> Result<Self, SignalingError> {
        let (socket, _response) =
            connect(url).map_err(|e| SignalingError::ConnectionFailed(e.to_string()))?;

        tracing::debug!("connected to signaling server: {url}");

        Ok(Self {
            socket: Arc::new(Mutex::new(socket)),
        })
    }
}

impl SignalingChannel for WebSocketSignaling {
    fn send(&self, msg: SignalingMessage) -> Result<(), SignalingError> {
        let json = serde_json::to_string(&msg)
            .map_err(|e| SignalingError::Serialization(e.to_string()))?;

        let mut socket = self.socket.lock().unwrap();
        socket
            .send(Message::Text(json))
            .map_err(|e| SignalingError::Io(e.to_string()))?;

        Ok(())
    }

    fn recv(&self) -> Result<SignalingMessage, SignalingError> {
        let mut socket = self.socket.lock().unwrap();

        loop {
            let msg = socket
                .read()
                .map_err(|e| SignalingError::Io(e.to_string()))?;

            match msg {
                Message::Text(text) => {
                    let signaling_msg: SignalingMessage = serde_json::from_str(&text)
                        .map_err(|e| SignalingError::Serialization(e.to_string()))?;
                    return Ok(signaling_msg);
                }
                Message::Close(_) => {
                    return Err(SignalingError::Closed);
                }
                // Skip ping/pong/binary frames
                _ => continue,
            }
        }
    }

    fn close(&self) {
        let mut socket = self.socket.lock().unwrap();
        let _ = socket.close(None);
    }
}

impl rinch_webrtc::SignalingIO for WebSocketSignaling {
    fn send_msg(&self, msg: SignalingMessage) -> Result<(), String> {
        SignalingChannel::send(self, msg).map_err(|e| e.to_string())
    }

    fn recv_msg(&self) -> Result<SignalingMessage, String> {
        SignalingChannel::recv(self).map_err(|e| e.to_string())
    }

    fn clone_box(&self) -> Box<dyn rinch_webrtc::SignalingIO> {
        Box::new(WebSocketSignaling {
            socket: self.socket.clone(),
        })
    }
}
