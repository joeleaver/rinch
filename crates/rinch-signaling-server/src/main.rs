//! Minimal WebSocket signaling relay server for rinch WebRTC.
//!
//! Rooms hold up to 2 peers. Messages from one peer are forwarded to the
//! other peer in the same room. Room IDs are parsed from the URL path:
//! `ws://host:port/room/{room_id}`.
//!
//! Usage:
//!   rinch-signaling-server [port]
//!   RINCH_SIGNAL_PORT=9090 rinch-signaling-server

use std::collections::HashMap;
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use tungstenite::http::Uri;
use tungstenite::{Message, WebSocket, accept_hdr};

/// A connected peer in a room.
struct Peer {
    socket: WebSocket<TcpStream>,
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

    let state: SharedState = Arc::new(Mutex::new(ServerState::new()));

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

fn handle_connection(stream: TcpStream, state: SharedState) -> Result<(), String> {
    let peer_addr = stream
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "unknown".into());

    // We need to extract the URI from the HTTP upgrade request.
    // tungstenite's accept_hdr gives us the request during handshake.
    let mut room_id = String::new();
    let room_id_ref = &mut room_id;

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

    let ws = accept_hdr(stream, callback).map_err(|e| format!("handshake failed: {e}"))?;

    if room_id.is_empty() {
        tracing::warn!("connection from {peer_addr} without room ID, closing");
        return Err("no room ID in path".into());
    }

    tracing::info!("peer {peer_addr} joined room '{room_id}'");

    // Register peer
    let peer_id = {
        let mut s = state.lock().unwrap();
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
        s.peers.insert(id, Peer { socket: ws });
        id
    };

    // Read loop — forward messages to the other peer in the room
    loop {
        let msg = {
            let mut s = state.lock().unwrap();
            let peer = match s.peers.get_mut(&peer_id) {
                Some(p) => p,
                None => break,
            };
            peer.socket.read()
        };

        match msg {
            Ok(Message::Text(text)) => {
                // Forward to the other peer in the room
                let mut s = state.lock().unwrap();
                let room = match s.rooms.get(&room_id) {
                    Some(r) => r,
                    None => break,
                };

                let other_id = room.peers.iter().find(|&&id| id != peer_id).copied();

                if let Some(other_id) = other_id
                    && let Some(other_peer) = s.peers.get_mut(&other_id)
                    && let Err(e) = other_peer.socket.send(Message::Text(text))
                {
                    tracing::warn!("failed to forward message to peer {other_id}: {e}");
                }
            }
            Ok(Message::Close(_)) => {
                tracing::info!("peer {peer_id} ({peer_addr}) disconnected from room '{room_id}'");
                break;
            }
            Ok(_) => {
                // Ignore binary/ping/pong
                continue;
            }
            Err(e) => {
                tracing::debug!("peer {peer_id} read error: {e}");
                break;
            }
        }
    }

    // Clean up: remove peer from room and state
    {
        let mut s = state.lock().unwrap();
        s.peers.remove(&peer_id);

        // Find the other peer in the room (if any) before mutating
        let other_id = s.rooms.get_mut(&room_id).and_then(|room| {
            room.peers.retain(|&id| id != peer_id);
            room.peers.first().copied()
        });

        // Notify remaining peer that the other disconnected
        if let Some(other_id) = other_id
            && let Some(other_peer) = s.peers.get_mut(&other_id)
        {
            let bye =
                serde_json::to_string(&rinch_webrtc::SignalingMessage::Bye).unwrap_or_default();
            let _ = other_peer.socket.send(Message::Text(bye));
        }

        // Remove empty rooms
        if let Some(room) = s.rooms.get(&room_id)
            && room.peers.is_empty()
        {
            s.rooms.remove(&room_id);
        }
    }

    Ok(())
}
