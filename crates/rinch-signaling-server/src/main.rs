//! Minimal WebSocket signaling relay server for rinch WebRTC.
//!
//! Rooms hold up to 2 peers. Messages from one peer are forwarded to the
//! other peer in the same room. Room IDs are parsed from the URL path:
//! `ws://host:port/room/{room_id}`.
//!
//! # Threading
//!
//! One thread per connection, and that thread owns its peer's [`WebSocket`]
//! outright: the socket is *not* stored in the shared state, so a thread parked
//! waiting for a silent peer to say something holds no lock and blocks nobody.
//! What the shared state holds for each peer is a bounded outbound queue, so
//! delivering to a peer is a non-blocking [`SyncSender::try_send`] under a
//! briefly held lock. The owning thread drains that queue between reads bounded
//! by [`POLL_INTERVAL`] — the shape `rinch-ws`'s native backend uses, because
//! synchronous `tungstenite` gives no way to split a socket into independent
//! read and write halves.
//!
//! Lock order, for the two locks that exist: the state mutex is taken first and
//! the channel's internal lock second (inside `try_send`). Nothing takes them
//! the other way round — a connection thread's `try_recv` runs with no state
//! lock held — and neither call ever blocks while holding either.
//!
//! Usage:
//!   rinch-signaling-server [port]
//!   RINCH_SIGNAL_PORT=9090 rinch-signaling-server

use std::collections::HashMap;
use std::io;
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use tungstenite::http::Uri;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{Error as TungError, Message, WebSocket, accept_hdr_with_config};

#[cfg(test)]
mod tests;

/// How long a read blocks before returning so the same thread can drain its
/// peer's outbound queue. Also the worst-case latency a forwarded message waits
/// before it is written.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// How many messages may sit queued for one peer before the relay gives up on
/// it. Signaling traffic is a handful of small JSON messages per call, so a peer
/// this far behind is not slow, it is gone — and the queue is bounded precisely
/// so that a peer which never drains costs a bounded amount of memory.
const OUTBOX_CAPACITY: usize = 64;

/// Largest message the relay will accept. Signaling payloads are SDP blobs and
/// ICE candidates — kilobytes. Together with [`OUTBOX_CAPACITY`] this is what
/// makes "bounded" a number: a peer that stops reading can cost at most
/// `OUTBOX_CAPACITY * MAX_MESSAGE_SIZE`. tungstenite's default of 64 MiB would
/// leave it at 4 GiB.
const MAX_MESSAGE_SIZE: usize = 256 * 1024;

/// A connected peer in a room.
///
/// Deliberately not the socket: the socket belongs to the peer's own connection
/// thread, which is the only thread that reads or writes it.
struct Peer {
    /// Bounded queue of messages to write, drained by that connection thread.
    outbox: SyncSender<Message>,
    /// A second handle to the same TCP socket, used only to shut it down. A peer
    /// the relay drops must actually go away, and dropping `outbox` alone only
    /// wakes a thread that is polling — not one blocked writing to a peer whose
    /// receive window is full.
    shutdown: TcpStream,
    /// Which room this peer is in, so that dropping it leaves `rooms` consistent
    /// without the dropper having to know where it lives.
    room: String,
}

/// A room holds up to 2 peers.
struct Room {
    peers: Vec<usize>,
}

/// Shared server state.
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

/// Lock the shared state, tolerating poisoning.
///
/// Every connection thread takes this lock, so `unwrap()` here would let one
/// panicking connection take down every other connection and all future ones.
/// The state behind it is two plain maps; a thread that panicked mid-update
/// leaves them readable, and the relay's own consistency repair (a peer whose
/// socket is gone is reaped on its next delivery) applies either way.
fn lock_state(state: &SharedState) -> MutexGuard<'_, ServerState> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

fn main() {
    tracing_subscriber::fmt::init();

    let port = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("RINCH_SIGNAL_PORT").ok())
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(8080);

    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr).unwrap_or_else(|e| {
        eprintln!("failed to bind {addr}: {e}");
        std::process::exit(1);
    });

    tracing::info!("signaling server listening on {addr}");

    serve(listener, Arc::new(Mutex::new(ServerState::new())));
}

/// Accept connections forever, one thread per connection.
fn serve(listener: TcpListener, state: SharedState) {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = Arc::clone(&state);
                thread::spawn(move || {
                    if let Err(e) = handle_connection(stream, state) {
                        tracing::warn!("connection error: {e}");
                    }
                });
            }
            Err(e) => {
                tracing::warn!("accept error: {e}");
            }
        }
    }
}

/// Extract room ID from the HTTP upgrade request path.
/// Expected format: /room/{room_id}
fn extract_room_id(uri: &Uri) -> Option<String> {
    let path = uri.path();
    let stripped = path.strip_prefix("/room/")?;
    if stripped.is_empty() {
        return None;
    }
    Some(stripped.to_string())
}

/// The other peer in `room_id`, if the room currently holds one.
fn other_peer_in_room(s: &ServerState, room_id: &str, peer_id: usize) -> Option<usize> {
    s.rooms
        .get(room_id)?
        .peers
        .iter()
        .find(|&&id| id != peer_id)
        .copied()
}

/// Queue `msg` for `peer_id`, never blocking.
///
/// A full queue means that peer is [`OUTBOX_CAPACITY`] messages behind, so it is
/// dropped rather than allowed to hold up the sender: blocking here would put a
/// dead peer back in the path of every other peer's traffic, which is the whole
/// defect this design exists to avoid.
fn deliver(s: &mut ServerState, peer_id: usize, msg: Message) {
    let Some(peer) = s.peers.get(&peer_id) else {
        return;
    };
    let queued = peer.outbox.try_send(msg);
    match queued {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => {
            tracing::warn!("peer {peer_id} is {OUTBOX_CAPACITY} messages behind, dropping it");
            drop_peer(s, peer_id);
        }
        Err(TrySendError::Disconnected(_)) => {
            // Its connection thread has already finished; reap the stale entry.
            drop_peer(s, peer_id);
        }
    }
}

/// Remove a peer from the shared state and close its socket.
///
/// Dropping the [`Peer`] drops its outbound queue, which its own connection
/// thread observes as a disconnect; the socket shutdown is what wakes that
/// thread if it is blocked in `read` or `send` rather than between them.
/// Idempotent — a peer's own cleanup and an eviction can both run.
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

/// Whether an I/O error is a benign "no data yet" from the read timeout.
/// A `SO_RCVTIMEO` expiry is `WouldBlock` on unix and `TimedOut` on Windows.
fn is_would_block(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

fn handle_connection(stream: TcpStream, state: SharedState) -> Result<(), String> {
    let peer_addr = stream
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "unknown".into());

    // Taken before the stream is consumed by the handshake: the relay needs a way
    // to close this socket without owning the `WebSocket` that wraps it.
    let shutdown = stream
        .try_clone()
        .map_err(|e| format!("failed to clone socket: {e}"))?;

    // We need to extract the URI from the HTTP upgrade request.
    // tungstenite's accept_hdr gives us the request during handshake.
    let mut room_id = String::new();
    let room_id_ref = &mut room_id;

    #[allow(clippy::result_large_err)]
    let callback = |req: &tungstenite::handshake::server::Request,
                    resp: tungstenite::handshake::server::Response|
     -> Result<
        tungstenite::handshake::server::Response,
        tungstenite::handshake::server::ErrorResponse,
    > {
        if let Some(id) = extract_room_id(req.uri()) {
            *room_id_ref = id;
        }
        Ok(resp)
    };

    // The handshake itself stays blocking — there is nothing to interleave with
    // it yet, and no lock is held during it.
    let config = WebSocketConfig {
        max_message_size: Some(MAX_MESSAGE_SIZE),
        max_frame_size: Some(MAX_MESSAGE_SIZE),
        ..Default::default()
    };
    let mut socket = accept_hdr_with_config(stream, callback, Some(config))
        .map_err(|e| format!("handshake failed: {e}"))?;

    if room_id.is_empty() {
        tracing::warn!("connection from {peer_addr} without room ID, closing");
        return Err("no room ID in path".into());
    }

    // Bound how long `read()` blocks so the same thread can also service sends.
    if let Err(e) = socket.get_ref().set_read_timeout(Some(POLL_INTERVAL)) {
        return Err(format!("failed to set read timeout: {e}"));
    }

    tracing::info!("peer {peer_addr} joined room '{room_id}'");

    // Register peer. The socket stays here on the stack; only the write end of
    // its outbound queue goes into the shared state.
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
            tracing::warn!("room '{room_id}' is full, rejecting {peer_addr}");
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
        id
    };

    run_peer(&mut socket, &outbox, peer_id, &room_id, &state, &peer_addr);

    // Clean up: remove peer from room and state, and tell whoever is left.
    {
        let mut s = lock_state(&state);
        let other_id = other_peer_in_room(&s, &room_id, peer_id);
        drop_peer(&mut s, peer_id);

        if let Some(other_id) = other_id {
            let bye =
                serde_json::to_string(&rinch_webrtc::SignalingMessage::Bye).unwrap_or_default();
            deliver(&mut s, other_id, Message::Text(bye));
        }
    }

    Ok(())
}

/// Pump one peer: drain its outbound queue, then read one frame, repeat.
///
/// Returns when the connection is finished, for any reason. Both halves of the
/// socket are touched only from here, on the thread that owns it, and never with
/// the state lock held.
fn run_peer(
    socket: &mut WebSocket<TcpStream>,
    outbox: &Receiver<Message>,
    peer_id: usize,
    room_id: &str,
    state: &SharedState,
    peer_addr: &str,
) {
    loop {
        // 1. Write everything queued for this peer.
        loop {
            match outbox.try_recv() {
                Ok(msg) => {
                    if let Err(e) = socket.send(msg) {
                        tracing::debug!("peer {peer_id} write error: {e}");
                        return;
                    }
                }
                Err(TryRecvError::Empty) => break,
                // The sender lives in the shared state, so this means the relay
                // has dropped this peer (it fell too far behind, or the room went
                // away). Nothing left to do but finish.
                Err(TryRecvError::Disconnected) => return,
            }
        }

        // Flush anything tungstenite queued on its own, such as a pong reply.
        match socket.flush() {
            Ok(()) => {}
            Err(TungError::Io(e)) if is_would_block(&e) => {}
            Err(e) => {
                tracing::debug!("peer {peer_id} flush error: {e}");
                return;
            }
        }

        // 2. Read one frame, or time out and go round again.
        match socket.read() {
            Ok(Message::Text(text)) => {
                // Forward to the other peer in the room.
                let mut s = lock_state(state);
                match other_peer_in_room(&s, room_id, peer_id) {
                    Some(other_id) => deliver(&mut s, other_id, Message::Text(text)),
                    None => tracing::debug!(
                        "room '{room_id}' has no other peer, dropping message from {peer_id}"
                    ),
                }
            }
            Ok(Message::Close(_)) => {
                tracing::info!("peer {peer_id} ({peer_addr}) disconnected from room '{room_id}'");
                // tungstenite has queued the closing reply; push it out.
                let _ = socket.flush();
                return;
            }
            Ok(_) => {
                // Ignore binary/ping/pong.
            }
            // The read timeout expiring just means the peer has nothing to say
            // yet — go back and service its outbound queue.
            Err(TungError::Io(e)) if is_would_block(&e) => {}
            Err(e) => {
                tracing::debug!("peer {peer_id} read error: {e}");
                return;
            }
        }
    }
}
