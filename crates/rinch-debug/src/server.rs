use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crate::protocol::*;
use crate::{CommandSender, discovery};

pub struct DebugServer {
    port: u16,
    #[allow(dead_code)]
    app_name: String,
    shutdown: Arc<AtomicBool>,
    listener_thread: Option<thread::JoinHandle<()>>,
}

/// How long a peer has to deliver its whole handshake frame (#2515).
pub(crate) const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(2);
/// Peers that may be mid-handshake at once; further ones are dropped.
const MAX_PENDING: usize = 16;

struct ClientCtx {
    token: Arc<str>,
    app_name: String,
    cmd_sender: Arc<CommandSender>,
    shutdown: Arc<AtomicBool>,
    /// Held by the authenticated client being served: one at a time.
    slot: Arc<Mutex<()>>,
    pending: Arc<AtomicUsize>,
}

/// Reads with a read timeout shrunk to what is left of a deadline, so a peer
/// that drips bytes cannot stretch the handshake.
struct Deadlined<'a> {
    stream: &'a TcpStream,
    deadline: Instant,
}

impl std::io::Read for Deadlined<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        self.stream.set_read_timeout(Some(left))?;
        let mut s = self.stream;
        std::io::Read::read(&mut s, buf)
    }
}

/// What a refused client is told (#2481).
const REFUSED: &str = "rinch-debug: missing or wrong token. A client reads it from the app's \
discovery file; a rinch-mcp-server from before #2481 sends none and needs re-installing";

impl DebugServer {
    pub fn start(
        app_name: &str,
        cmd_sender: CommandSender,
        port_override: Option<u16>,
    ) -> std::io::Result<Self> {
        let addr = format!("127.0.0.1:{}", port_override.unwrap_or(0));
        let listener = TcpListener::bind(&addr)?;
        let port = listener.local_addr()?.port();

        // Anyone on the machine can connect to a loopback port; only a client
        // that read this token from the discovery file, which only this user
        // can read, gets past the handshake (#2481).
        let token: Arc<str> = discovery::new_token()?.into();
        discovery::write_discovery_file(app_name, port, &token)?;
        tracing::warn!(
            "rinch-debug: the debug interface is on, at 127.0.0.1:{port}; a client needs the \
             token in {} (readable by this user only)",
            discovery::own_discovery_file()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        );

        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = shutdown.clone();
        let app_name_owned = app_name.to_string();

        let handle = thread::Builder::new()
            .name("rinch-debug-listener".into())
            .spawn(move || {
                // Use non-blocking accept with sleep to check shutdown flag
                listener.set_nonblocking(true).ok();
                let slot = Arc::new(Mutex::new(()));
                let pending = Arc::new(AtomicUsize::new(0));
                let cmd_sender = Arc::new(cmd_sender);

                tracing::info!("rinch-debug listening on 127.0.0.1:{}", port);

                loop {
                    if shutdown_clone.load(Ordering::Acquire) {
                        break;
                    }

                    match listener.accept() {
                        Ok((stream, addr)) => {
                            // One thread per connection, so a peer that
                            // sends nothing holds only its own thread (#2515).
                            if pending.fetch_add(1, Ordering::AcqRel) >= MAX_PENDING {
                                pending.fetch_sub(1, Ordering::AcqRel);
                                tracing::warn!("rinch-debug: too many unauthenticated peers");
                                continue;
                            }
                            tracing::info!("rinch-debug client connected from {}", addr);
                            let ctx = ClientCtx {
                                token: token.clone(),
                                app_name: app_name_owned.clone(),
                                cmd_sender: cmd_sender.clone(),
                                shutdown: shutdown_clone.clone(),
                                slot: slot.clone(),
                                pending: pending.clone(),
                            };
                            let spawned = thread::Builder::new()
                                .name("rinch-debug-client".into())
                                .spawn(move || {
                                    Self::handle_client(stream, &ctx);
                                    tracing::info!("rinch-debug client disconnected");
                                });
                            if spawned.is_err() {
                                // The thread never ran, so it will not
                                // release its pending slot.
                                pending.fetch_sub(1, Ordering::AcqRel);
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(std::time::Duration::from_millis(100));
                        }
                        Err(e) => {
                            if !shutdown_clone.load(Ordering::Acquire) {
                                tracing::error!("rinch-debug accept error: {}", e);
                            }
                            break;
                        }
                    }
                }
            })?;

        Ok(Self {
            port,
            app_name: app_name.to_string(),
            shutdown,
            listener_thread: Some(handle),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    fn handle_client(mut stream: TcpStream, ctx: &ClientCtx) {
        let handshake = Self::authenticate(&mut stream, ctx);
        ctx.pending.fetch_sub(1, Ordering::AcqRel);
        if !handshake {
            return;
        }
        // One authenticated client at a time; a second waits here, which
        // costs nothing (it holds no allocation and has been checked).
        let _guard = loop {
            if ctx.shutdown.load(Ordering::Acquire) {
                return;
            }
            match ctx.slot.try_lock() {
                Ok(g) => break g,
                Err(std::sync::TryLockError::Poisoned(p)) => break p.into_inner(),
                Err(_) => thread::sleep(Duration::from_millis(50)),
            }
        };
        Self::serve(stream, ctx);
    }

    /// The handshake, under a small frame cap and a deadline, before the
    /// token is checked (#2515). True when the client may be served; the
    /// handshake response has then been sent.
    fn authenticate(stream: &mut TcpStream, ctx: &ClientCtx) -> bool {
        let token = &*ctx.token;
        let app_name = &ctx.app_name;
        let handshake_data = match read_frame_max(
            &mut Deadlined {
                stream,
                deadline: Instant::now() + HANDSHAKE_DEADLINE,
            },
            MAX_HANDSHAKE_FRAME,
        ) {
            Ok(data) => data,
            Err(e) => {
                tracing::warn!("rinch-debug handshake read failed: {}", e);
                return false;
            }
        };
        let stream = &mut *stream;
        let handshake: HandshakeRequest = match serde_json::from_slice(&handshake_data) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!("rinch-debug invalid handshake: {}", e);
                return false;
            }
        };

        if handshake.protocol != "rinch-debug" || handshake.version != 1 {
            let err =
                serde_json::to_vec(&serde_json::json!({"error": "unsupported protocol/version"}))
                    .unwrap();
            let _ = write_frame(stream, &err);
            return false;
        }

        if !discovery::token_matches(handshake.token.as_deref(), token) {
            tracing::warn!(
                "rinch-debug: refused a client from {:?}: {}",
                stream.peer_addr().ok(),
                if handshake.token.is_some() {
                    "wrong token"
                } else {
                    "no token"
                }
            );
            let err = serde_json::to_vec(&serde_json::json!({ "error": REFUSED })).unwrap();
            let _ = write_frame(stream, &err);
            return false;
        }

        let response = HandshakeResponse {
            protocol: "rinch-debug".into(),
            version: 1,
            app_name: app_name.to_string(),
            pid: std::process::id(),
        };
        let resp_data = serde_json::to_vec(&response).unwrap();
        if write_frame(stream, &resp_data).is_err() {
            return false;
        }
        true
    }

    fn serve(mut stream: TcpStream, ctx: &ClientCtx) {
        let (shutdown, cmd_sender) = (&*ctx.shutdown, &*ctx.cmd_sender);
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
        // Request loop
        loop {
            if shutdown.load(Ordering::Acquire) {
                break;
            }

            let req_data = match read_frame(&mut stream) {
                Ok(data) => data,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    continue;
                }
                Err(_) => break, // Client disconnected
            };

            let request: Request = match serde_json::from_slice(&req_data) {
                Ok(r) => r,
                Err(e) => {
                    let error_resp = Response {
                        id: 0,
                        result: DebugResult::Error {
                            message: format!("Invalid request: {}", e),
                        },
                    };
                    let data = serde_json::to_vec(&error_resp).unwrap();
                    if write_frame(&mut stream, &data).is_err() {
                        break;
                    }
                    continue;
                }
            };

            // Execute command via runtime
            let result = cmd_sender.execute(request.command);

            let response = Response {
                id: request.id,
                result,
            };

            let resp_data = serde_json::to_vec(&response).unwrap();
            if write_frame(&mut stream, &resp_data).is_err() {
                break;
            }
        }
    }
}

impl Drop for DebugServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        discovery::remove_discovery_file();
        // Connect to self to unblock accept() (the listener uses non-blocking
        // accept with 100ms sleeps, but a self-connect wakes it immediately).
        let _ = std::net::TcpStream::connect(format!("127.0.0.1:{}", self.port));
        // Join the listener thread so it doesn't outlive the app.
        if let Some(handle) = self.listener_thread.take() {
            let _ = handle.join();
        }
    }
}
