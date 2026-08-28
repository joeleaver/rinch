//! Desktop WebSocket signaling via tungstenite.
//!
//! One thread owns the socket and nothing else touches it. [`send`] queues a
//! message for that thread and returns; [`recv`] blocks on the queue of messages
//! the thread has read. That is the same shape as the WASM sibling in
//! `websocket_wasm.rs`, whose `recv` reads from an `mpsc` queue rather than the
//! socket, and as `rinch-ws`'s native backend.
//!
//! It replaced a single `Mutex<WebSocket>` that both directions shared, where
//! `recv` held the mutex across its blocking read and a concurrent `send` could
//! not proceed until the remote peer happened to say something (#175). That is
//! not hypothetical for this crate's one caller: `VideoCall::start` sends
//! trickle-ICE candidates from the WebRTC thread while its receiver thread is
//! parked in `recv`. Bounding the read was not enough — a reader that re-takes
//! an unfair mutex immediately can starve a waiting sender indefinitely, which
//! it duly did for seconds at a time.
//!
//! [`send`]: SignalingChannel::send
//! [`recv`]: SignalingChannel::recv

use std::io;
use std::sync::mpsc::{self, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Error as TungError, Message, WebSocket, connect};

use crate::error::SignalingError;
use crate::{SignalingChannel, SignalingMessage};

/// How long the owning thread blocks in a read before returning to write
/// whatever has been queued for it. Also the worst-case latency a queued send
/// waits before it goes out.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// A request to the socket-owning thread.
enum Cmd {
    Text(String),
    Close,
}

/// A signaling channel that communicates over a WebSocket connection.
///
/// Messages are serialized as JSON text frames. Uses tungstenite for
/// synchronous WebSocket I/O (no async runtime), on a thread of its own.
pub struct WebSocketSignaling {
    /// Outgoing messages, handed to the socket-owning thread.
    cmd_tx: Sender<Cmd>,
    /// Incoming messages from that thread, ending with the error that finished
    /// it. Shared so that clones of this channel see one connection, as they did
    /// when they shared a socket.
    incoming: Arc<Mutex<mpsc::Receiver<Result<SignalingMessage, SignalingError>>>>,
}

impl WebSocketSignaling {
    /// Connect to a WebSocket signaling server at the given URL.
    ///
    /// The URL should be in the form `ws://host:port/room/{room_id}`.
    pub fn connect(url: &str) -> Result<Self, SignalingError> {
        let (socket, _response) =
            connect(url).map_err(|e| SignalingError::ConnectionFailed(e.to_string()))?;

        tracing::debug!("connected to signaling server: {url}");

        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (msg_tx, msg_rx) = mpsc::channel();
        thread::Builder::new()
            .name("rinch-signaling-ws".into())
            .spawn(move || run(socket, &cmd_rx, &msg_tx))
            .map_err(|e| {
                SignalingError::ConnectionFailed(format!("spawn signaling thread: {e}"))
            })?;

        Ok(Self {
            cmd_tx,
            incoming: Arc::new(Mutex::new(msg_rx)),
        })
    }
}

/// Whether an I/O error is a benign "no data yet" from the read timeout.
/// A `SO_RCVTIMEO` expiry is `WouldBlock` on unix and `TimedOut` on Windows.
fn is_would_block(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

/// Own the socket: write what is queued, read what arrives, until either end
/// finishes. Returning drops `msg_tx`, which is what ends a blocked `recv`.
fn run(
    mut socket: WebSocket<MaybeTlsStream<std::net::TcpStream>>,
    cmd_rx: &mpsc::Receiver<Cmd>,
    msg_tx: &Sender<Result<SignalingMessage, SignalingError>>,
) {
    // Bound how long `read()` blocks so this thread can also service sends.
    // `MaybeTlsStream` is `#[non_exhaustive]` and this crate enables no TLS
    // backend, so `Plain` is the only variant it can produce on its own; a
    // TLS-capable build just gets a longer worst-case send latency.
    if let MaybeTlsStream::Plain(s) = socket.get_mut()
        && let Err(e) = s.set_read_timeout(Some(POLL_INTERVAL))
    {
        tracing::debug!("could not bound signaling read timeout: {e}");
    }

    loop {
        // 1. Write everything queued.
        loop {
            match cmd_rx.try_recv() {
                Ok(Cmd::Text(json)) => {
                    if let Err(e) = socket.send(Message::Text(json)) {
                        let _ = msg_tx.send(Err(SignalingError::Io(e.to_string())));
                        return;
                    }
                }
                Ok(Cmd::Close) => {
                    let _ = socket.close(None);
                    let _ = socket.flush();
                    return;
                }
                Err(TryRecvError::Empty) => break,
                // Every handle to this channel is gone; nobody can send or
                // receive any more.
                Err(TryRecvError::Disconnected) => {
                    let _ = socket.close(None);
                    let _ = socket.flush();
                    return;
                }
            }
        }

        // Flush anything tungstenite queued on its own, such as a pong reply.
        match socket.flush() {
            Ok(()) => {}
            Err(TungError::Io(e)) if is_would_block(&e) => {}
            Err(e) => {
                let _ = msg_tx.send(Err(SignalingError::Io(e.to_string())));
                return;
            }
        }

        // 2. Read one frame, or time out and go round again.
        let delivered = match socket.read() {
            Ok(Message::Text(text)) => msg_tx.send(
                serde_json::from_str(&text)
                    .map_err(|e| SignalingError::Serialization(e.to_string())),
            ),
            Ok(Message::Close(_)) => {
                let _ = msg_tx.send(Err(SignalingError::Closed));
                return;
            }
            // Skip ping/pong/binary frames.
            Ok(_) => Ok(()),
            Err(TungError::Io(e)) if is_would_block(&e) => Ok(()),
            Err(TungError::ConnectionClosed | TungError::AlreadyClosed) => {
                let _ = msg_tx.send(Err(SignalingError::Closed));
                return;
            }
            Err(e) => {
                let _ = msg_tx.send(Err(SignalingError::Io(e.to_string())));
                return;
            }
        };
        // Nobody is listening any more.
        if delivered.is_err() {
            return;
        }
    }
}

impl SignalingChannel for WebSocketSignaling {
    fn send(&self, msg: SignalingMessage) -> Result<(), SignalingError> {
        let json = serde_json::to_string(&msg)
            .map_err(|e| SignalingError::Serialization(e.to_string()))?;

        // Queued rather than written here, so a send never waits on a read. A
        // write that then fails is reported to `recv`, which is where the rest
        // of the connection's errors surface.
        self.cmd_tx
            .send(Cmd::Text(json))
            .map_err(|_| SignalingError::Closed)
    }

    fn recv(&self) -> Result<SignalingMessage, SignalingError> {
        // Held across the blocking receive, so two threads calling `recv` take
        // turns — as on WASM. It is not the send path's lock: `send` never
        // touches it.
        let incoming = self.incoming.lock().unwrap_or_else(|e| e.into_inner());
        incoming.recv().map_err(|_| SignalingError::Closed)?
    }

    fn close(&self) {
        let _ = self.cmd_tx.send(Cmd::Close);
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
            cmd_tx: self.cmd_tx.clone(),
            incoming: Arc::clone(&self.incoming),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use rinch_webrtc::SignalingIO as _;

    use super::*;

    /// Budget for anything that should happen in milliseconds. A wedged channel
    /// shows up as this elapsing, never as a hang.
    const TEST_TIMEOUT: Duration = Duration::from_secs(5);

    /// Long enough that a receiver thread is certainly parked in its read.
    const SETTLE: Duration = Duration::from_millis(300);

    /// A server that says nothing until it is spoken to, then echoes. Returns
    /// the ephemeral port it is listening on.
    fn start_echo_server() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("local_addr").port();
        thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut ws = tungstenite::accept(stream).expect("handshake");
            while let Ok(msg) = ws.read() {
                if let Message::Text(text) = msg
                    && ws.send(Message::Text(text)).is_err()
                {
                    return;
                }
            }
        });
        port
    }

    fn connect_to(port: u16) -> WebSocketSignaling {
        WebSocketSignaling::connect(&format!("ws://127.0.0.1:{port}/room/test")).expect("connect")
    }

    /// A send must not have to wait for the peer to speak first.
    ///
    /// The two threads here are the ones `VideoCall::start` creates: a receiver
    /// parked in `recv`, and a trickle-ICE callback sending from elsewhere.
    /// Before this fix they deadlocked — `recv` held the socket, and the only
    /// message this server will ever send is the echo of the send that could not
    /// get out.
    #[test]
    fn a_send_does_not_wait_for_a_concurrent_recv() {
        let port = start_echo_server();
        let signaling = connect_to(port);

        let (received_tx, received_rx) = mpsc::channel();
        let receiver = signaling.clone_box();
        thread::spawn(move || {
            let _ = received_tx.send(receiver.recv_msg());
        });

        // Let the receiver get into its read first; a send that won the race
        // would prove nothing.
        thread::sleep(SETTLE);

        let (sent_tx, sent_rx) = mpsc::channel();
        let sender = signaling.clone_box();
        thread::spawn(move || {
            let _ = sent_tx.send(sender.send_msg(SignalingMessage::Bye));
        });

        sent_rx
            .recv_timeout(TEST_TIMEOUT)
            .expect("send blocked behind a concurrent recv")
            .expect("send failed");

        let received = received_rx
            .recv_timeout(TEST_TIMEOUT)
            .expect("the echo never arrived")
            .expect("recv failed");
        assert!(matches!(received, SignalingMessage::Bye));
    }

    /// Many sends while a receiver is parked, all of which must get through.
    #[test]
    fn sends_keep_flowing_while_a_recv_is_parked() {
        let port = start_echo_server();
        let signaling = connect_to(port);

        let (received_tx, received_rx) = mpsc::channel();
        let receiver = signaling.clone_box();
        thread::spawn(move || {
            for _ in 0..8 {
                if received_tx.send(receiver.recv_msg()).is_err() {
                    return;
                }
            }
        });
        thread::sleep(SETTLE);

        for _ in 0..8 {
            signaling.send(SignalingMessage::Bye).expect("send");
        }
        for i in 0..8 {
            let msg = received_rx
                .recv_timeout(TEST_TIMEOUT)
                .unwrap_or_else(|_| panic!("echo {i} never arrived"))
                .expect("recv failed");
            assert!(matches!(msg, SignalingMessage::Bye));
        }
    }

    /// A peer that hangs up ends `recv`, rather than leaving it parked forever.
    #[test]
    fn recv_reports_a_dropped_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("local_addr").port();
        thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            // Hang up without a closing handshake.
            drop(tungstenite::accept(stream).expect("handshake"));
        });

        let signaling = connect_to(port);
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(signaling.recv());
        });

        let result = rx
            .recv_timeout(TEST_TIMEOUT)
            .expect("recv never returned after the peer hung up");
        assert!(result.is_err(), "a dropped connection must end recv");
    }

    /// Closing locally ends a parked `recv` too, and takes the thread with it.
    #[test]
    fn close_ends_a_parked_recv() {
        let port = start_echo_server();
        let signaling = connect_to(port);

        let (tx, rx) = mpsc::channel();
        let receiver = signaling.clone_box();
        thread::spawn(move || {
            let _ = tx.send(receiver.recv_msg());
        });
        thread::sleep(SETTLE);

        signaling.close();
        let result = rx
            .recv_timeout(TEST_TIMEOUT)
            .expect("recv never returned after close");
        assert!(result.is_err(), "close must end recv");
    }
}
