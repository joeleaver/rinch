//! Native (non-wasm) WebSocket backend backed by synchronous `tungstenite`.
//!
//! [`connect`] spawns a `std::thread` that owns the socket and runs a single
//! read/write loop. Outgoing frames are queued from the main thread over an
//! `mpsc` channel; incoming frames and lifecycle events are hopped back to the
//! main thread via [`rinch_core::run_on_main_thread`] and delivered to the
//! callback registry by [`crate::dispatch`]. Only the connection id and the
//! `Send` [`WsEvent`] cross the boundary, so the (`!Send`) callbacks never leave
//! the main thread.
//!
//! `tungstenite` is synchronous and gives no way to split a socket into
//! independent read/write halves, so the one owning thread interleaves both: it
//! reads with a short timeout, and between reads drains the outgoing queue. The
//! timeout (`POLL_INTERVAL`) bounds how long a queued send waits before it goes
//! out — a small latency/wakeup trade-off, not a correctness issue.
//!
//! `wss://` is supported via `tungstenite`'s `rustls-tls-webpki-roots` feature,
//! reusing the `rustls` stack already present in the tree (via `rinch-http`'s
//! `ureq`).

use std::io;
use std::net::TcpStream;
use std::sync::mpsc::{self, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tungstenite::client::IntoClientRequest;
use tungstenite::handshake::client::Request;
use tungstenite::http::{HeaderName, HeaderValue};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Error as TungError, Message, WebSocket};

use rinch_core::run_on_main_thread;

use crate::{WsClose, WsError, WsEvent, WsMessage, WsRequest};

/// How long a read blocks before returning to drain the outgoing queue. Also the
/// worst-case latency a queued `send` waits before being written.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Close code reported when the connection dropped without a close frame.
const CLOSE_ABNORMAL: u16 = 1006;

/// Close code reported when the closing handshake completed normally.
const CLOSE_NORMAL: u16 = 1000;

/// Reported when the peer's close frame carried no status code. RFC 6455 reserves
/// 1005 ("no status received") for exactly this; 1006 means no close frame arrived
/// at all, which is a different thing. Browsers surface 1005 here, so the web
/// backend already reports it — native must match or the same server produces
/// different close codes on desktop and web.
const CLOSE_NO_STATUS: u16 = 1005;

/// How long to keep reading after a local `close()` so the peer can complete the
/// closing handshake. Without a grace period the first read timeout ends the
/// connection, and every graceful close on a link slower than [`POLL_INTERVAL`]
/// would be reported as abnormal.
const CLOSE_GRACE: Duration = Duration::from_secs(1);

/// A command from the main thread to the socket-owning worker thread.
enum Cmd {
    Text(String),
    Binary(Vec<u8>),
    Close,
}

/// The main-thread half of a native connection: a sender into the worker thread.
pub(crate) struct Backend {
    cmd_tx: Sender<Cmd>,
    /// The subprotocol the server chose, written by the worker before it
    /// emits the open (so `on_open` can read it); `""` until then.
    protocol: Arc<Mutex<String>>,
}

impl Backend {
    pub(crate) fn send_text(&self, text: String) {
        let _ = self.cmd_tx.send(Cmd::Text(text));
    }

    pub(crate) fn send_bytes(&self, bytes: Vec<u8>) {
        let _ = self.cmd_tx.send(Cmd::Binary(bytes));
    }

    pub(crate) fn close(&self) {
        let _ = self.cmd_tx.send(Cmd::Close);
    }

    pub(crate) fn protocol(&self) -> String {
        self.protocol.lock().map(|p| p.clone()).unwrap_or_default()
    }
}

pub(crate) fn connect(request: &WsRequest, id: u64) -> Result<Backend, WsError> {
    // Built here, on the calling thread, so an invalid header is refused
    // synchronously, as an invalid subprotocol is.
    let request = client_request(request)?;
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let protocol = Arc::new(Mutex::new(String::new()));
    let chosen = protocol.clone();
    // The blocking connect + read loop must not run on the main thread.
    std::thread::spawn(move || run(request, id, cmd_rx, chosen));
    Ok(Backend { cmd_tx, protocol })
}

/// Headers the opening handshake writes itself; [`WsRequest::header`] refuses
/// them rather than replace or duplicate them.
const HANDSHAKE_HEADERS: &[&str] = &[
    "host",
    "upgrade",
    "connection",
    "sec-websocket-key",
    "sec-websocket-version",
    "sec-websocket-protocol",
];

/// The tungstenite handshake request for `request`: its URL, its headers, and
/// the offered subprotocols in one `Sec-WebSocket-Protocol` header.
fn client_request(request: &WsRequest) -> Result<Request, WsError> {
    let mut req = request
        .url
        .as_str()
        .into_client_request()
        .map_err(|e| WsError::InvalidUrl(e.to_string()))?;
    let headers = req.headers_mut();
    for (name, value) in &request.headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|e| WsError::InvalidUrl(format!("header name {name:?}: {e}")))?;
        // `HeaderName` is lowercase, so this compares case-insensitively.
        if HANDSHAKE_HEADERS.contains(&name.as_str()) {
            return Err(WsError::InvalidUrl(format!(
                "header {name} is set by the WebSocket handshake{}",
                if name.as_str() == "sec-websocket-protocol" {
                    "; offer subprotocols with WsRequest::protocol"
                } else {
                    ""
                }
            )));
        }
        let value = HeaderValue::from_str(value)
            .map_err(|e| WsError::InvalidUrl(format!("header {name} value {value:?}: {e}")))?;
        headers.append(name, value);
    }
    if !request.protocols.is_empty() {
        // Joined with a bare ",": tungstenite reads the offered list back by
        // splitting on "," without trimming, so ", " would make it refuse the
        // server's (correct) answer for every protocol but the first.
        let joined = request.protocols.join(",");
        let value = HeaderValue::from_str(&joined)
            .map_err(|e| WsError::InvalidUrl(format!("subprotocols {joined:?}: {e}")))?;
        headers.insert("Sec-WebSocket-Protocol", value);
    }
    Ok(req)
}

/// Emit an event to the main-thread registry for connection `id`.
fn emit(id: u64, event: WsEvent) {
    run_on_main_thread(move || crate::dispatch(id, event));
}

fn run(request: Request, id: u64, cmd_rx: mpsc::Receiver<Cmd>, chosen: Arc<Mutex<String>>) {
    // tungstenite has already refused a chosen subprotocol that was not offered
    // — and, unlike a browser, offered subprotocols the server chose none of.
    // No redirects (`max_redirects` 0): a browser never follows one for a
    // WebSocket, and tungstenite would copy every header — an
    // `Authorization` included — to the `Location` target, `wss` to `ws`.
    let (mut socket, resp) = match tungstenite::client::connect_with_config(request, None, 0) {
        Ok(ok) => ok,
        Err(e) => {
            // Error, then an abnormal close, as a browser reports a failed
            // connect (`error`, then `close` 1006): an app that reconnects from
            // `on_close` must hear about a refused attempt too.
            emit(id, WsEvent::Error(WsError::Connect(e.to_string())));
            emit(
                id,
                WsEvent::Close(WsClose {
                    code: CLOSE_ABNORMAL,
                    reason: String::new(),
                }),
            );
            return;
        }
    };
    if let Some(p) = resp
        .headers()
        .get("Sec-WebSocket-Protocol")
        .and_then(|v| v.to_str().ok())
        && let Ok(mut slot) = chosen.lock()
    {
        *slot = p.to_string();
    }

    // Bound how long `read()` blocks so the same thread can also service sends.
    set_read_timeout(&mut socket, Some(POLL_INTERVAL));

    emit(id, WsEvent::Open);

    let mut close_emitted = false;
    // Set once a close has been initiated locally; we then read only to drive the
    // closing handshake to completion. `closing_since` starts the grace period in
    // which the peer may still send its half of the handshake.
    let mut closing = false;
    let mut closing_since: Option<Instant> = None;
    // A close we asked for that the peer completed is a *normal* closure; only a
    // drop without a completed handshake is abnormal.
    let local_close_code = |closing: bool| {
        if closing {
            CLOSE_NORMAL
        } else {
            CLOSE_ABNORMAL
        }
    };

    loop {
        // 1. Drain outgoing commands.
        loop {
            match cmd_rx.try_recv() {
                Ok(Cmd::Text(t)) => {
                    if let Err(e) = socket.send(Message::Text(t)) {
                        emit(id, WsEvent::Error(WsError::Send(e.to_string())));
                        emit_close(id, CLOSE_ABNORMAL, String::new(), &mut close_emitted);
                        return;
                    }
                }
                Ok(Cmd::Binary(b)) => {
                    if let Err(e) = socket.send(Message::Binary(b)) {
                        emit(id, WsEvent::Error(WsError::Send(e.to_string())));
                        emit_close(id, CLOSE_ABNORMAL, String::new(), &mut close_emitted);
                        return;
                    }
                }
                Ok(Cmd::Close) => {
                    let _ = socket.close(None);
                    closing = true;
                    closing_since.get_or_insert_with(Instant::now);
                }
                Err(TryRecvError::Empty) => break,
                // All senders dropped (handle dropped): close and finish.
                Err(TryRecvError::Disconnected) => {
                    let _ = socket.close(None);
                    closing = true;
                    closing_since.get_or_insert_with(Instant::now);
                    break;
                }
            }
        }

        // Flush any queued frames (pong replies, the close frame). A `WouldBlock`
        // just means the write would block right now — retry next iteration.
        match socket.flush() {
            Ok(()) => {}
            Err(TungError::Io(e)) if is_would_block(&e) => {}
            Err(TungError::ConnectionClosed | TungError::AlreadyClosed) => {
                emit_close(
                    id,
                    local_close_code(closing),
                    String::new(),
                    &mut close_emitted,
                );
                return;
            }
            Err(e) => {
                emit(id, WsEvent::Error(WsError::Send(e.to_string())));
                emit_close(id, CLOSE_ABNORMAL, String::new(), &mut close_emitted);
                return;
            }
        }

        // 2. Read one frame (or time out).
        match socket.read() {
            Ok(Message::Text(t)) => emit(id, WsEvent::Message(WsMessage::Text(t))),
            Ok(Message::Binary(b)) => emit(id, WsEvent::Message(WsMessage::Binary(b))),
            // Control/continuation frames are handled internally by tungstenite.
            Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {}
            Ok(Message::Close(frame)) => {
                let (code, reason) = match frame {
                    Some(f) => (u16::from(f.code), f.reason.into_owned()),
                    None => (CLOSE_NO_STATUS, String::new()),
                };
                emit_close(id, code, reason, &mut close_emitted);
                return;
            }
            Err(TungError::Io(e)) if is_would_block(&e) => {
                // Timed out with nothing to read. While closing, keep reading for a
                // bounded grace period so a peer slower than one poll interval can
                // still complete the handshake; only then give up as abnormal.
                if closing && closing_since.is_some_and(|t| t.elapsed() >= CLOSE_GRACE) {
                    emit_close(id, CLOSE_ABNORMAL, String::new(), &mut close_emitted);
                    return;
                }
            }
            Err(TungError::ConnectionClosed | TungError::AlreadyClosed) => {
                emit_close(
                    id,
                    local_close_code(closing),
                    String::new(),
                    &mut close_emitted,
                );
                return;
            }
            Err(e) => {
                emit(id, WsEvent::Error(WsError::Protocol(e.to_string())));
                emit_close(id, CLOSE_ABNORMAL, String::new(), &mut close_emitted);
                return;
            }
        }
    }
}

/// Emit a single `Close` event, at most once per connection.
fn emit_close(id: u64, code: u16, reason: String, emitted: &mut bool) {
    if *emitted {
        return;
    }
    *emitted = true;
    emit(id, WsEvent::Close(WsClose { code, reason }));
}

/// Whether an I/O error is a benign "no data yet" from the read timeout.
fn is_would_block(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

/// Set the read timeout on the underlying `TcpStream`, reaching through TLS.
fn set_read_timeout(socket: &mut WebSocket<MaybeTlsStream<TcpStream>>, dur: Option<Duration>) {
    match socket.get_mut() {
        MaybeTlsStream::Plain(s) => {
            let _ = s.set_read_timeout(dur);
        }
        MaybeTlsStream::Rustls(s) => {
            let _ = s.sock.set_read_timeout(dur);
        }
        // `MaybeTlsStream` is `#[non_exhaustive]`; other TLS backends are not
        // compiled in for this crate.
        _ => {}
    }
}
