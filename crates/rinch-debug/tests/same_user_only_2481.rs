//! Issue #2481: the debug interface is reachable only by the user running the
//! app. The discovery file holds a per-session token, readable only by that
//! user (0600, in a 0700 directory), and the server refuses a handshake that
//! does not carry it.
//!
//! One test in its own binary: it points `HOME` at a directory of its own, so
//! no other test may read the environment while it runs.

use rinch_debug::DebugResult;
use rinch_debug::protocol::{read_frame, write_frame};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// Connect and handshake with `token` (`None`: no token field at all, as an
/// older client sends). Returns the stream when the server accepted, or the
/// server's answer (or the read error) when it refused.
fn handshake(port: u16, token: Option<&str>) -> Result<TcpStream, String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut hello = serde_json::json!({"protocol": "rinch-debug", "version": 1});
    if let Some(t) = token {
        hello["token"] = serde_json::json!(t);
    }
    write_frame(&mut stream, &serde_json::to_vec(&hello).unwrap()).unwrap();
    match read_frame(&mut stream) {
        Ok(data) => {
            let v: serde_json::Value = serde_json::from_slice(&data).unwrap();
            if v.get("app_name").is_some() {
                Ok(stream)
            } else {
                Err(v.to_string())
            }
        }
        Err(e) => Err(format!("closed: {e}")),
    }
}

/// Handshake with `token`, expecting a refusal, then send a command on the
/// same stream anyway, as a client that ignores the refusal would. Returns
/// the refusal; the caller checks that the command did not run.
fn refused_then_command(port: u16, token: Option<&str>) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut hello = serde_json::json!({"protocol": "rinch-debug", "version": 1});
    if let Some(t) = token {
        hello["token"] = serde_json::json!(t);
    }
    write_frame(&mut stream, &serde_json::to_vec(&hello).unwrap()).unwrap();
    let answer = read_frame(&mut stream).expect("the server answers a refused client");
    let v: serde_json::Value = serde_json::from_slice(&answer).unwrap();
    assert!(v.get("app_name").is_none(), "accepted: {v}");
    let refusal = v["error"].as_str().expect("an error").to_string();
    // The server closes the connection after the refusal; the write may or
    // may not see that yet, so its result is not the test.
    let req = serde_json::json!({"id": 9, "method": "wait_frame"});
    let _ = write_frame(&mut stream, &serde_json::to_vec(&req).unwrap());
    if let Ok(frame) = read_frame(&mut stream) {
        panic!(
            "a refused client got a frame: {}",
            String::from_utf8_lossy(&frame)
        );
    }
    refusal
}

/// Send one `wait_frame` and return the response's `type`.
fn wait_frame(stream: &mut TcpStream) -> String {
    let req = serde_json::json!({"id": 7, "method": "wait_frame"});
    write_frame(stream, &serde_json::to_vec(&req).unwrap()).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&read_frame(stream).unwrap()).unwrap();
    v["type"].as_str().unwrap_or_default().to_string()
}

#[test]
fn only_a_client_holding_the_token_gets_in() {
    let home = std::env::temp_dir().join(format!("rinch-2481-home-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    // SAFETY: this binary runs one test; nothing else reads the environment.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::remove_var("RINCH_DEBUG_PORT");
    }

    let (server, rx) = rinch_debug::attach("probe-2481", || {}).expect("server starts");
    let executed = Arc::new(AtomicUsize::new(0));
    let count = executed.clone();
    std::thread::spawn(move || {
        for cmd in rx.0 {
            count.fetch_add(1, Ordering::SeqCst);
            let _ = cmd.response_tx.send(DebugResult::Json {
                data: serde_json::json!("pong"),
            });
        }
    });

    let dir = home.join(".rinch").join("debug");
    let file = dir.join(format!("{}.json", std::process::id()));
    let entry: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).expect("discovery file")).unwrap();
    assert_eq!(entry["port"].as_u64(), Some(server.port() as u64));

    // Only this user can read the directory and the file.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700, "the discovery directory");
        assert_eq!(mode(&file), 0o600, "the discovery file");
    }

    // No token, as a client from before #2481 sends: refused, nothing runs.
    let refused = handshake(server.port(), None);
    assert!(
        refused.is_err(),
        "a handshake without the token was accepted"
    );
    // A wrong token: refused too.
    let wrong = handshake(server.port(), Some("0000"));
    assert!(
        wrong.is_err(),
        "a handshake with a wrong token was accepted"
    );
    assert_eq!(
        executed.load(Ordering::SeqCst),
        0,
        "a refused client ran a command"
    );

    // A client that sends a command after being refused, on the same
    // connection: nothing runs (the server stops at the refusal).
    for token in [None, Some("0000")] {
        let refusal = refused_then_command(server.port(), token);
        assert!(refusal.contains("token"), "{refusal}");
    }
    assert_eq!(
        executed.load(Ordering::SeqCst),
        0,
        "a command sent after the refusal ran"
    );

    // The token from the file: in, and commands run.
    let token = entry["token"]
        .as_str()
        .expect("the discovery file holds a token");
    assert!(token.len() >= 32, "a token of {} chars", token.len());
    let mut stream = handshake(server.port(), Some(token)).expect("the right token is accepted");
    assert_eq!(wait_frame(&mut stream), "json");
    assert_eq!(executed.load(Ordering::SeqCst), 1);
    drop(stream);

    drop(server);
    assert!(!file.exists(), "the discovery file outlived the server");
    let _ = std::fs::remove_dir_all(&home);
}
