use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use crate::protocol::*;
use crate::{CommandSender, discovery};

pub struct DebugServer {
    port: u16,
    #[allow(dead_code)]
    app_name: String,
    shutdown: Arc<AtomicBool>,
    listener_thread: Option<thread::JoinHandle<()>>,
}

impl DebugServer {
    pub fn start(
        app_name: &str,
        cmd_sender: CommandSender,
        port_override: Option<u16>,
    ) -> std::io::Result<Self> {
        let addr = format!("127.0.0.1:{}", port_override.unwrap_or(0));
        let listener = TcpListener::bind(&addr)?;
        let port = listener.local_addr()?.port();

        // Write discovery file
        discovery::write_discovery_file(app_name, port)?;

        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = shutdown.clone();
        let app_name_owned = app_name.to_string();

        let handle = thread::Builder::new()
            .name("rinch-debug-listener".into())
            .spawn(move || {
                // Use non-blocking accept with sleep to check shutdown flag
                listener.set_nonblocking(true).ok();

                tracing::info!("rinch-debug listening on 127.0.0.1:{}", port);

                loop {
                    if shutdown_clone.load(Ordering::Acquire) {
                        break;
                    }

                    match listener.accept() {
                        Ok((stream, addr)) => {
                            tracing::info!("rinch-debug client connected from {}", addr);
                            Self::handle_client(
                                stream,
                                &app_name_owned,
                                &cmd_sender,
                                &shutdown_clone,
                            );
                            tracing::info!("rinch-debug client disconnected");
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

    fn handle_client(
        mut stream: TcpStream,
        app_name: &str,
        cmd_sender: &CommandSender,
        shutdown: &AtomicBool,
    ) {
        // Set read timeout for checking shutdown
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .ok();

        // Handshake
        let handshake_data = match read_frame(&mut stream) {
            Ok(data) => data,
            Err(e) => {
                tracing::warn!("rinch-debug handshake read failed: {}", e);
                return;
            }
        };

        let handshake: HandshakeRequest = match serde_json::from_slice(&handshake_data) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!("rinch-debug invalid handshake: {}", e);
                return;
            }
        };

        if handshake.protocol != "rinch-debug" || handshake.version != 1 {
            let err =
                serde_json::to_vec(&serde_json::json!({"error": "unsupported protocol/version"}))
                    .unwrap();
            let _ = write_frame(&mut stream, &err);
            return;
        }

        let response = HandshakeResponse {
            protocol: "rinch-debug".into(),
            version: 1,
            app_name: app_name.to_string(),
            pid: std::process::id(),
        };
        let resp_data = serde_json::to_vec(&response).unwrap();
        if write_frame(&mut stream, &resp_data).is_err() {
            return;
        }

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
