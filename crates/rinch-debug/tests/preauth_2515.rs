//! Issue #2515: a peer that has not shown the token can neither hold the
//! interface nor make the app allocate. A silent peer, a handshake frame
//! declaring 64 MiB, and a wrong token each leave a good client's handshake
//! prompt, and no allocation near the declared size happens.
//!
//! One test in its own binary: it points `HOME` at a directory of its own and
//! installs a counting allocator.

use rinch_debug::DebugResult;
use rinch_debug::protocol::{read_frame, write_frame};
use std::alloc::{GlobalAlloc, Layout, System};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// The largest single allocation the process has made.
static LARGEST: AtomicUsize = AtomicUsize::new(0);

struct Counting;
// SAFETY: forwards to the system allocator, only recording sizes.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        LARGEST.fetch_max(l.size(), Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        LARGEST.fetch_max(n, Ordering::Relaxed);
        unsafe { System.realloc(p, l, n) }
    }
}
#[global_allocator]
static A: Counting = Counting;

fn good_handshake(port: u16, token: &str) -> Duration {
    let t = Instant::now();
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let hello = serde_json::json!({"protocol": "rinch-debug", "version": 1, "token": token});
    write_frame(&mut s, &serde_json::to_vec(&hello).unwrap()).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&read_frame(&mut s).unwrap()).unwrap();
    assert!(v.get("app_name").is_some(), "good client refused: {v}");
    t.elapsed()
}

/// True when the server closed (or errored) the connection within `within`.
fn server_hangs_up(s: &mut TcpStream, within: Duration) -> bool {
    s.set_read_timeout(Some(within)).unwrap();
    let mut b = [0u8; 256];
    loop {
        match s.read(&mut b) {
            Ok(0) => return true,
            Ok(_) => continue, // a refusal frame, then the close
            Err(e) => {
                return !matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                );
            }
        }
    }
}

#[test]
fn an_unauthenticated_peer_neither_blocks_nor_allocates() {
    let home = std::env::temp_dir().join(format!("rinch-2515-home-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    // SAFETY: this binary runs one test; nothing else reads the environment.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::remove_var("RINCH_DEBUG_PORT");
    }
    let (server, rx) = rinch_debug::attach("probe-2515", || {}).expect("server starts");
    std::thread::spawn(move || {
        for cmd in rx.0 {
            let _ = cmd.response_tx.send(DebugResult::Json {
                data: serde_json::json!("pong"),
            });
        }
    });
    let port = server.port();
    let file = home
        .join(".rinch/debug")
        .join(format!("{}.json", std::process::id()));
    let entry: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    let token = entry["token"].as_str().unwrap().to_string();
    let baseline = LARGEST.load(Ordering::Relaxed);
    assert!(
        baseline < 1 << 20,
        "the harness itself allocated {baseline}"
    );

    // 1. Silent peers (several, reconnecting): the good client is not delayed.
    let silent: Vec<TcpStream> = (0..8)
        .map(|_| TcpStream::connect(("127.0.0.1", port)).unwrap())
        .collect();
    let took = good_handshake(port, &token);
    assert!(
        took < Duration::from_millis(1000),
        "a good client waited {took:?} behind silent peers"
    );

    // ... and the server gives up on a silent peer on its own.
    let mut first = silent.into_iter().next().unwrap();
    assert!(
        server_hangs_up(&mut first, Duration::from_secs(6)),
        "a silent peer was never dropped"
    );

    // 2. A handshake frame declaring 64 MiB, and no data: refused at once,
    // with no allocation of that size, and the good client still gets in.
    let mut big = TcpStream::connect(("127.0.0.1", port)).unwrap();
    big.write_all(&(64u32 * 1024 * 1024).to_be_bytes()).unwrap();
    assert!(
        server_hangs_up(&mut big, Duration::from_secs(1)),
        "an oversized pre-auth frame was not refused promptly"
    );
    let took = good_handshake(port, &token);
    assert!(took < Duration::from_millis(1000), "waited {took:?}");

    // 3. A wrong token: closed, and the good client still gets in.
    let mut wrong = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let hello = serde_json::json!({"protocol": "rinch-debug", "version": 1, "token": "0000"});
    write_frame(&mut wrong, &serde_json::to_vec(&hello).unwrap()).unwrap();
    assert!(server_hangs_up(&mut wrong, Duration::from_secs(1)));
    good_handshake(port, &token);

    let largest = LARGEST.load(Ordering::Relaxed);
    assert!(
        largest < 1 << 20,
        "something allocated {largest} bytes before the token was checked"
    );

    drop(server);
    let _ = std::fs::remove_dir_all(&home);
}
