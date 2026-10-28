//! A room relay for WebRTC signalling, which `examples/video-call` talks to.
//!
//! The binary is `src/main.rs`; [`run`] serves a listener, so another
//! example's tests can start a relay of their own (video-call's do).
//!
//! A peer connects to `ws://host:port/room/{room}`. A room holds two peers,
//! and every text message from one is forwarded to the other unchanged; the
//! relay does not read them. It adds two notices of its own:
//!
//! - `{"type":"peer-joined"}` to the peer already waiting, when a second one
//!   arrives (video-call makes the offer then);
//! - `{"type":"bye"}` to the peer left behind, when the other goes.
//!
//! A third peer is refused. Run it with `cargo run -p rtc-room-relay [port]`
//! (default 8080, or `RTC_RELAY_PORT`).
//!
//! # Threading
//!
//! One thread per connection, and that thread owns its socket: synchronous
//! tungstenite cannot split one into read and write halves. The shared state
//! holds, per peer, a bounded queue that the owning thread drains between
//! reads (each read times out after [`POLL_INTERVAL`]). So delivering to a
//! peer is a non-blocking `try_send` under a briefly held lock, and a peer
//! that goes quiet holds up nobody: no thread blocks while holding the lock.
//! A peer whose queue fills is gone, not slow, and is dropped.

use std::collections::HashMap;
use std::io;
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::http::Uri;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{Error as WsError, Message, WebSocket, accept_hdr_with_config};

#[cfg(test)]
mod tests;

/// How long a read blocks before its thread services the peer's queue; also
/// the longest a forwarded message waits to be written.
const POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Messages queued for one peer before it is dropped as gone.
const OUTBOX_CAPACITY: usize = 64;
/// The largest message accepted. SDP and candidates are kilobytes; with
/// [`OUTBOX_CAPACITY`] this bounds what a peer that stops reading can cost.
const MAX_MESSAGE_SIZE: usize = 256 * 1024;

const PEER_JOINED: &str = r#"{"type":"peer-joined"}"#;
const BYE: &str = r#"{"type":"bye"}"#;

/// A peer as the shared state holds it: not its socket, which belongs to its
/// connection thread.
struct Peer {
    outbox: SyncSender<Message>,
    /// A second handle on the socket, only to shut it down: dropping `outbox`
    /// wakes a thread that is polling, not one blocked writing.
    shutdown: TcpStream,
    room: String,
}

struct Room {
    peers: Vec<usize>,
}

struct ServerState {
    rooms: HashMap<String, Room>,
    peers: HashMap<usize, Peer>,
    next_peer_id: usize,
}

impl ServerState {
    fn new() -> Self {
        Self {
            rooms: HashMap::new(),
            peers: HashMap::new(),
            next_peer_id: 0,
        }
    }
}

type SharedState = Arc<Mutex<ServerState>>;

/// Lock the state, through poisoning: one panicking connection must not take
/// every other one down. The maps stay readable either way.
fn lock_state(state: &SharedState) -> MutexGuard<'_, ServerState> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Serve rooms on `listener`, forever (the binary's `main`, and a test's
/// relay on an ephemeral port).
pub fn run(listener: TcpListener) {
    serve(listener, Arc::new(Mutex::new(ServerState::new())));
}

/// Accept connections forever, one thread each.
fn serve(listener: TcpListener, state: SharedState) {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = Arc::clone(&state);
                thread::spawn(move || {
                    if let Err(e) = handle_connection(stream, state) {
                        tracing::warn!("connection: {e}");
                    }
                });
            }
            Err(e) => tracing::warn!("accept: {e}"),
        }
    }
}

/// The room in an upgrade path `/room/{room}`.
fn extract_room_id(uri: &Uri) -> Option<String> {
    let room = uri.path().strip_prefix("/room/")?;
    (!room.is_empty()).then(|| room.to_string())
}

fn other_peer_in_room(s: &ServerState, room: &str, peer_id: usize) -> Option<usize> {
    s.rooms
        .get(room)?
        .peers
        .iter()
        .find(|&&id| id != peer_id)
        .copied()
}

/// Queue `msg` for `peer_id`, never blocking; a full or dead queue drops
/// the peer.
fn deliver(s: &mut ServerState, peer_id: usize, msg: Message) {
    let Some(peer) = s.peers.get(&peer_id) else {
        return;
    };
    match peer.outbox.try_send(msg) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => {
            tracing::warn!("peer {peer_id} is {OUTBOX_CAPACITY} messages behind; dropping it");
            drop_peer(s, peer_id);
        }
        Err(TrySendError::Disconnected(_)) => drop_peer(s, peer_id),
    }
}

/// Remove a peer, close its socket, and leave its room consistent (an empty
/// room goes). Idempotent.
fn drop_peer(s: &mut ServerState, peer_id: usize) {
    let Some(peer) = s.peers.remove(&peer_id) else {
        return;
    };
    let _ = peer.shutdown.shutdown(Shutdown::Both);
    if let Some(room) = s.rooms.get_mut(&peer.room) {
        room.peers.retain(|&id| id != peer_id);
        if room.peers.is_empty() {
            s.rooms.remove(&peer.room);
        }
    }
}

/// A read timeout: `WouldBlock` on unix, `TimedOut` on Windows.
fn is_would_block(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

fn handle_connection(stream: TcpStream, state: SharedState) -> Result<(), String> {
    let shutdown = stream.try_clone().map_err(|e| format!("clone: {e}"))?;
    let mut room_id = String::new();
    #[allow(clippy::result_large_err)]
    let read_room = |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
        if let Some(id) = extract_room_id(req.uri()) {
            room_id = id;
        }
        Ok(resp)
    };
    let config = WebSocketConfig {
        max_message_size: Some(MAX_MESSAGE_SIZE),
        max_frame_size: Some(MAX_MESSAGE_SIZE),
        ..Default::default()
    };
    let mut socket = accept_hdr_with_config(stream, read_room, Some(config))
        .map_err(|e| format!("handshake: {e}"))?;
    if room_id.is_empty() {
        return Err("no /room/<name> in the path".into());
    }
    socket
        .get_ref()
        .set_read_timeout(Some(POLL_INTERVAL))
        .map_err(|e| format!("read timeout: {e}"))?;

    let (outbox_tx, outbox) = mpsc::sync_channel(OUTBOX_CAPACITY);
    let peer_id = {
        let mut s = lock_state(&state);
        let id = s.next_peer_id;
        s.next_peer_id += 1;
        let room = s
            .rooms
            .entry(room_id.clone())
            .or_insert_with(|| Room { peers: Vec::new() });
        if room.peers.len() >= 2 {
            return Err(format!("room '{room_id}' is full"));
        }
        room.peers.push(id);
        s.peers.insert(
            id,
            Peer {
                outbox: outbox_tx,
                shutdown,
                room: room_id.clone(),
            },
        );
        if let Some(waiting) = other_peer_in_room(&s, &room_id, id) {
            deliver(&mut s, waiting, Message::Text(PEER_JOINED.into()));
        }
        id
    };
    tracing::info!("peer {peer_id} joined room '{room_id}'");

    run_peer(&mut socket, &outbox, peer_id, &room_id, &state);

    let mut s = lock_state(&state);
    let other = other_peer_in_room(&s, &room_id, peer_id);
    drop_peer(&mut s, peer_id);
    if let Some(other) = other {
        deliver(&mut s, other, Message::Text(BYE.into()));
    }
    tracing::info!("peer {peer_id} left room '{room_id}'");
    Ok(())
}

/// Pump one peer until its connection ends: write what is queued for it,
/// then read one frame (or time out), and again. Never with the lock held.
fn run_peer(
    socket: &mut WebSocket<TcpStream>,
    outbox: &Receiver<Message>,
    peer_id: usize,
    room_id: &str,
    state: &SharedState,
) {
    loop {
        loop {
            match outbox.try_recv() {
                Ok(msg) => {
                    if socket.send(msg).is_err() {
                        return;
                    }
                }
                Err(TryRecvError::Empty) => break,
                // The relay dropped this peer.
                Err(TryRecvError::Disconnected) => return,
            }
        }
        match socket.flush() {
            Ok(()) => {}
            Err(WsError::Io(e)) if is_would_block(&e) => {}
            Err(_) => return,
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                let mut s = lock_state(state);
                if let Some(other) = other_peer_in_room(&s, room_id, peer_id) {
                    deliver(&mut s, other, Message::Text(text));
                }
            }
            Ok(Message::Close(_)) => {
                let _ = socket.flush();
                return;
            }
            Ok(_) => {}
            Err(WsError::Io(e)) if is_would_block(&e) => {}
            Err(e) => {
                tracing::debug!("peer {peer_id}: {e}");
                return;
            }
        }
    }
}
