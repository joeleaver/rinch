//! Tests for the relay.
//!
//! The property under test is the one the design exists for: a peer that has
//! gone quiet must not hold anything up. Every test that could otherwise wait
//! forever is bounded by [`TEST_TIMEOUT`], and every inspection of the shared
//! state goes through `try_lock`, so a relay that has wedged itself fails these
//! tests instead of hanging them.
//!
//! Ports are always ephemeral (`127.0.0.1:0`, read back from the listener), so
//! the tests can run concurrently with each other and with anything else.

use std::io::Read as _;
use std::time::Instant;

use tungstenite::stream::MaybeTlsStream;

use super::*;

/// How long a client waits in one `read` before looking at the clock again.
const CLIENT_POLL: Duration = Duration::from_millis(50);

/// Overall budget for anything a working relay does in milliseconds. Generous
/// against [`POLL_INTERVAL`] so a loaded CI runner does not turn a pass into a
/// failure, and short enough that a wedged relay reports quickly.
const TEST_TIMEOUT: Duration = Duration::from_secs(5);

type Client = WebSocket<MaybeTlsStream<TcpStream>>;

/// Start a relay on an ephemeral port. Returns the port and the shared state, so
/// a test can observe registration rather than guessing at it with a sleep.
fn start_relay() -> (u16, SharedState) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let port = listener.local_addr().expect("local_addr").port();
    let state: SharedState = Arc::new(Mutex::new(ServerState::new()));
    let served = Arc::clone(&state);
    thread::spawn(move || serve(listener, served));
    (port, state)
}

fn connect_peer(port: u16, room: &str) -> Client {
    let url = format!("ws://127.0.0.1:{port}/room/{room}");
    let (socket, _resp) = tungstenite::connect(url.as_str()).expect("websocket connect");
    if let MaybeTlsStream::Plain(s) = socket.get_ref() {
        s.set_read_timeout(Some(CLIENT_POLL))
            .expect("client read timeout");
    }
    socket
}

/// Poll `cond` until it holds, or fail. Never blocks on the state mutex.
fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + TEST_TIMEOUT;
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(5));
    }
}

/// How many peers the relay currently has in `room`, or `None` if the state
/// mutex is held by somebody else right now.
///
/// `try_lock` rather than `lock` is the point: before this fix an idle peer's
/// connection thread held this mutex across its blocking read, so a caller that
/// waited for the lock would wait forever. Here that shows up as a timeout.
fn room_size(state: &SharedState, room: &str) -> Option<usize> {
    let s = state.try_lock().ok()?;
    Some(s.rooms.get(room).map_or(0, |r| r.peers.len()))
}

fn wait_for_room(state: &SharedState, room: &str, peers: usize) {
    wait_until(&format!("room '{room}' to hold {peers} peer(s)"), || {
        room_size(state, room) == Some(peers)
    });
}

/// Read until a text message arrives, or fail after [`TEST_TIMEOUT`].
fn expect_text(socket: &mut Client, what: &str) -> String {
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        match socket.read() {
            Ok(Message::Text(text)) => return text,
            Ok(_) => {}
            Err(TungError::Io(e)) if is_would_block(&e) => {}
            Err(e) => panic!("{what}: read failed: {e}"),
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
    }
}

/// A connected pair of loopback sockets, for tests that need a real `TcpStream`
/// without a relay behind it.
fn loopback_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let near = TcpStream::connect(addr).expect("connect");
    let (far, _) = listener.accept().expect("accept");
    (near, far)
}

/// The defect in #175, stated directly: peer A connects and says nothing, and
/// everything else must carry on regardless.
///
/// Before the fix, A's connection thread held the one global mutex parked in
/// `read()`, so B's registration never completed and B's message was forwarded
/// to nobody — with both clients still reporting a healthy connection.
#[test]
fn an_idle_peer_does_not_block_a_join_or_a_message() {
    let (port, state) = start_relay();

    let mut idle = connect_peer(port, "call");
    wait_for_room(&state, "call", 1);

    // B joins the same room while A sits there saying nothing.
    let mut caller = connect_peer(port, "call");
    wait_for_room(&state, "call", 2);

    caller
        .send(Message::Text("offer".into()))
        .expect("send to relay");
    assert_eq!(expect_text(&mut idle, "the forwarded offer"), "offer");

    // And the reply gets back the other way.
    idle.send(Message::Text("answer".into()))
        .expect("send to relay");
    assert_eq!(expect_text(&mut caller, "the forwarded answer"), "answer");
}

/// The same property across rooms: one silent peer must not be able to stop
/// unrelated peers from using the relay at all.
#[test]
fn an_idle_peer_does_not_block_another_room() {
    let (port, state) = start_relay();

    let _idle = connect_peer(port, "quiet");
    wait_for_room(&state, "quiet", 1);

    let mut left = connect_peer(port, "busy");
    let mut right = connect_peer(port, "busy");
    wait_for_room(&state, "busy", 2);

    left.send(Message::Text("ping".into())).expect("send");
    assert_eq!(expect_text(&mut right, "ping"), "ping");
    right.send(Message::Text("pong".into())).expect("send");
    assert_eq!(expect_text(&mut left, "pong"), "pong");
}

/// A peer leaving is noticed, announced, and leaves the room usable.
#[test]
fn a_departing_peer_is_announced_and_frees_its_slot() {
    let (port, state) = start_relay();

    let mut leaver = connect_peer(port, "call");
    wait_for_room(&state, "call", 1);
    let mut stayer = connect_peer(port, "call");
    wait_for_room(&state, "call", 2);

    leaver.close(None).expect("close");
    let _ = leaver.flush();
    drop(leaver);

    let bye = serde_json::to_string(&rinch_webrtc::SignalingMessage::Bye).expect("serialize Bye");
    assert_eq!(expect_text(&mut stayer, "the Bye notification"), bye);
    wait_for_room(&state, "call", 1);

    // The freed slot is really free, and the new peer really is wired up.
    let mut replacement = connect_peer(port, "call");
    wait_for_room(&state, "call", 2);
    replacement
        .send(Message::Text("hello".into()))
        .expect("send");
    assert_eq!(
        expect_text(&mut stayer, "the replacement's message"),
        "hello"
    );
}

/// The last peer out takes the room with it, so state does not grow forever.
#[test]
fn an_emptied_room_is_removed() {
    let (port, state) = start_relay();

    let mut only = connect_peer(port, "call");
    wait_for_room(&state, "call", 1);

    only.close(None).expect("close");
    let _ = only.flush();
    drop(only);

    wait_until("the empty room to be removed", || {
        state
            .try_lock()
            .ok()
            .is_some_and(|s| !s.rooms.contains_key("call") && s.peers.is_empty())
    });
}

/// A third peer is refused, and the refusal does not disturb the pair already in
/// the room.
#[test]
fn a_third_peer_is_rejected_without_disturbing_the_pair() {
    let (port, state) = start_relay();

    let mut first = connect_peer(port, "call");
    wait_for_room(&state, "call", 1);
    let mut second = connect_peer(port, "call");
    wait_for_room(&state, "call", 2);

    // The handshake succeeds (it runs before registration); the relay then drops
    // the connection, which the client sees as the stream ending.
    let mut third = connect_peer(port, "call");
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        match third.read() {
            Ok(msg) => panic!("a rejected peer received {msg:?}"),
            Err(TungError::Io(e)) if is_would_block(&e) => {}
            Err(_) => break,
        }
        assert!(Instant::now() < deadline, "rejected peer was never closed");
    }
    assert_eq!(room_size(&state, "call"), Some(2));

    first
        .send(Message::Text("still here".into()))
        .expect("send");
    assert_eq!(expect_text(&mut second, "the pair's traffic"), "still here");
}

/// Backpressure: a peer that never drains its queue is dropped, rather than
/// being allowed to block the peer sending to it or to grow without bound.
///
/// Driven against the state directly because the interesting boundary is the
/// queue, and filling it through a real socket would first have to fill the
/// kernel's send and receive buffers — megabytes of traffic and a timing race.
#[test]
fn a_peer_that_never_drains_is_dropped_not_waited_on() {
    let mut state = ServerState::new();
    let (near, far) = loopback_pair();

    let (outbox_tx, outbox_rx) = mpsc::sync_channel(OUTBOX_CAPACITY);
    state.rooms.insert(
        "call".into(),
        Room {
            peers: vec![0, 1], // 1 stands in for the peer doing the sending
        },
    );
    state.peers.insert(
        0,
        Peer {
            outbox: outbox_tx,
            shutdown: far,
            room: "call".into(),
        },
    );
    state.next_peer_id = 2;

    // Fill the queue exactly. Nothing drains it — that is the peer being tested.
    for i in 0..OUTBOX_CAPACITY {
        deliver(&mut state, 0, Message::Text(format!("msg {i}")));
    }
    assert!(
        state.peers.contains_key(&0),
        "a peer within its queue depth must be kept"
    );

    // One more than it can hold. This must return, not block.
    deliver(&mut state, 0, Message::Text("one too many".into()));
    assert!(
        !state.peers.contains_key(&0),
        "a peer past its queue depth must be dropped"
    );
    assert_eq!(
        state.rooms["call"].peers,
        vec![1],
        "dropping a peer must leave its room consistent"
    );

    // Dropped for real: its socket was shut down, which is what wakes a thread
    // blocked writing to a peer that stopped reading.
    near.set_read_timeout(Some(TEST_TIMEOUT))
        .expect("read timeout");
    let mut buf = [0u8; 1];
    assert_eq!(
        (&near).read(&mut buf).expect("read from dropped peer"),
        0,
        "the dropped peer's socket must be closed"
    );

    // Nothing beyond the bound was ever queued.
    drop(state);
    assert_eq!(outbox_rx.iter().count(), OUTBOX_CAPACITY);
}

/// One connection panicking while holding the state mutex must not take every
/// other connection down with it.
///
/// The panic below prints a backtrace notice on stderr during the run; that is
/// this test working, not a failure.
#[test]
fn a_poisoned_state_mutex_does_not_wedge_the_relay() {
    let state: SharedState = Arc::new(Mutex::new(ServerState::new()));

    let poisoner = Arc::clone(&state);
    let _ = thread::spawn(move || {
        let _guard = poisoner.lock().expect("lock");
        panic!("a connection thread panicked while holding the state");
    })
    .join();
    assert!(state.is_poisoned());

    // Would have panicked under `lock().unwrap()`, on every connection thread.
    let mut s = lock_state(&state);
    s.next_peer_id += 1;
    assert_eq!(s.next_peer_id, 1);
}

/// Room IDs come from the upgrade path, and a connection without one is refused.
#[test]
fn room_ids_come_from_the_path() {
    assert_eq!(
        extract_room_id(&"/room/abc".parse::<Uri>().expect("uri")),
        Some("abc".into())
    );
    assert_eq!(
        extract_room_id(&"/room/".parse::<Uri>().expect("uri")),
        None
    );
    assert_eq!(
        extract_room_id(&"/other".parse::<Uri>().expect("uri")),
        None
    );
}
