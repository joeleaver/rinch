//! The debug server: listeners on a background thread each, a thread per
//! connected client, and every command handed to the app's runtime.
//!
//! On Unix it listens on a socket in a directory only this user can enter
//! (see [`crate::location`]), checking each peer's user id before the
//! handshake (see [`crate::peer`]). It also still listens on the legacy TCP
//! port, with its `~/.rinch/debug/{pid}.json`, so clients that know only TCP
//! keep working while they move to the socket.

use std::io;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::protocol::*;
use crate::transport::Connection;
use crate::{CommandSender, discovery};

/// How long a listener sleeps between polls for a connection, and so how
/// long it takes to notice a shutdown.
const POLL: Duration = Duration::from_millis(100);

/// What a [`DebugServer`] listens on.
pub struct ServerOptions {
    /// The legacy TCP port on `127.0.0.1`: 0 for any free port.
    pub tcp_port: u16,
    /// Where the legacy discovery file for the TCP port is written
    /// (`~/.rinch/debug` in an app); `None` writes none.
    pub discovery_dir: Option<PathBuf>,
    /// Where the socket goes.
    #[cfg(unix)]
    pub socket: SocketPlace,
    /// How a socket peer's credentials are read.
    #[cfg(unix)]
    pub peer_credentials: crate::peer::PeerCredentials,
}

/// Where the debug socket goes.
#[cfg(unix)]
#[derive(Debug, Clone)]
pub enum SocketPlace {
    /// No socket.
    None,
    /// `<pid>.sock` in the directory these inputs resolve to.
    Directory(crate::location::Locations),
    /// Exactly this path (`RINCH_DEBUG_SOCKET`), for a test that starts an
    /// app and must know where to connect. Its directory must be this
    /// user's and writable by no one else.
    Exact(PathBuf),
}

impl ServerOptions {
    /// What an app listens on, from its environment:
    ///
    /// - TCP on `RINCH_DEBUG_PORT`, or any free port, with the discovery
    ///   file in `~/.rinch/debug`;
    /// - on Unix, the socket at `RINCH_DEBUG_SOCKET`, or `<pid>.sock` in the
    ///   runtime directory's `rinch/debug`.
    #[must_use]
    pub fn from_process() -> Self {
        let port = std::env::var("RINCH_DEBUG_PORT")
            .ok()
            .and_then(|port| port.parse().ok())
            .unwrap_or(0);
        Self {
            tcp_port: port,
            discovery_dir: Some(discovery::discovery_dir()),
            #[cfg(unix)]
            socket: match std::env::var_os("RINCH_DEBUG_SOCKET") {
                Some(path) => SocketPlace::Exact(path.into()),
                None => SocketPlace::Directory(crate::location::Locations::from_process()),
            },
            #[cfg(unix)]
            peer_credentials: crate::peer::kernel_credentials(),
        }
    }
}

/// A running debug server. Dropping it stops the listeners, ends every
/// client's connection, and removes the socket and the discovery file.
pub struct DebugServer {
    shared: Arc<Shared>,
    port: u16,
    socket_path: Option<PathBuf>,
    discovery_file: Option<PathBuf>,
    listeners: Vec<JoinHandle<()>>,
}

/// What every listener and client thread shares.
struct Shared {
    app_name: String,
    sender: CommandSender,
    shutdown: AtomicBool,
    /// Each connected client.
    clients: Mutex<Vec<ClientThread>>,
}

/// A connected client: a handle to end its stream with, and the thread
/// serving it.
struct ClientThread {
    stream: Box<dyn Connection>,
    thread: JoinHandle<()>,
}

impl DebugServer {
    /// Starts listening as `options` say, for the app `app_name`.
    ///
    /// A socket that cannot be made (its directory fails the checks) is
    /// logged and left out; the TCP listener still starts.
    ///
    /// # Errors
    ///
    /// When the TCP listener or a thread cannot be started.
    pub fn start(
        app_name: &str,
        sender: CommandSender,
        options: ServerOptions,
    ) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            app_name: app_name.to_owned(),
            sender,
            shutdown: AtomicBool::new(false),
            clients: Mutex::new(Vec::new()),
        });
        let mut server = Self {
            shared,
            port: 0,
            socket_path: None,
            discovery_file: None,
            listeners: Vec::new(),
        };
        #[cfg(unix)]
        server.start_socket(&options)?;
        server.start_tcp(options.tcp_port, options.discovery_dir)?;
        Ok(server)
    }

    /// The legacy TCP port.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The socket's path, if there is a socket.
    #[must_use]
    pub fn socket_path(&self) -> Option<&std::path::Path> {
        self.socket_path.as_deref()
    }

    fn start_tcp(&mut self, port: u16, discovery_dir: Option<PathBuf>) -> io::Result<()> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        if let Some(dir) = discovery_dir {
            self.discovery_file = Some(discovery::write_discovery_file(
                &dir,
                &self.shared.app_name,
                port,
            )?);
        }
        let shared = Arc::clone(&self.shared);
        let handle = thread::Builder::new()
            .name("rinch-debug-tcp".into())
            .spawn(move || {
                accept_loop(&shared, || {
                    let (stream, _) = listener.accept()?;
                    stream.set_nonblocking(false)?;
                    Ok(Some(stream))
                });
            })?;
        tracing::info!(
            "rinch-debug listening on 127.0.0.1:{port} (legacy TCP, open to every local user)"
        );
        self.port = port;
        self.listeners.push(handle);
        Ok(())
    }

    #[cfg(unix)]
    fn start_socket(&mut self, options: &ServerOptions) -> io::Result<()> {
        use std::os::unix::net::UnixListener;

        let path = match socket_path(&options.socket) {
            Ok(Some(path)) => path,
            Ok(None) => return Ok(()),
            Err(error) => {
                tracing::error!("rinch-debug: no socket, its directory is unusable: {error}");
                return Ok(());
            }
        };
        remove_stale_socket(&path);
        let listener = UnixListener::bind(&path)?;
        // The directory is owner-only from its creation, so no one else
        // can reach the socket even before its own mode is set.
        set_mode(&path, 0o600)?;
        listener.set_nonblocking(true)?;
        let shared = Arc::clone(&self.shared);
        let credentials = Arc::clone(&options.peer_credentials);
        let own_uid = rustix::process::geteuid().as_raw();
        let handle = thread::Builder::new()
            .name("rinch-debug-socket".into())
            .spawn(move || {
                accept_loop(&shared, || {
                    let (stream, _) = listener.accept()?;
                    stream.set_nonblocking(false)?;
                    match crate::peer::admit(own_uid, credentials(&stream)) {
                        Ok(_) => Ok(Some(stream)),
                        Err(refusal) => {
                            // Closed without a byte written.
                            tracing::warn!("rinch-debug refused a connection: {refusal}");
                            Ok(None)
                        }
                    }
                });
            })?;
        tracing::info!("rinch-debug listening on {}", path.display());
        self.socket_path = Some(path);
        self.listeners.push(handle);
        Ok(())
    }
}

/// The socket's path for `place`, its directory made and checked.
#[cfg(unix)]
fn socket_path(place: &SocketPlace) -> Result<Option<PathBuf>, crate::location::LocationError> {
    match place {
        SocketPlace::None => Ok(None),
        SocketPlace::Directory(locations) => {
            let dir = locations.create_socket_dir()?;
            Ok(Some(dir.join(format!("{}.sock", std::process::id()))))
        }
        SocketPlace::Exact(path) => {
            let parent = path.parent().unwrap_or(std::path::Path::new("/"));
            let uid = rustix::process::geteuid().as_raw();
            crate::location::check_private_enough(parent, uid)?;
            Ok(Some(path.clone()))
        }
    }
}

/// Removes a socket left at `path` by an earlier process, but nothing that
/// is not a socket.
#[cfg(unix)]
fn remove_stale_socket(path: &std::path::Path) {
    use std::os::unix::fs::FileTypeExt;
    if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_socket()) {
        std::fs::remove_file(path).ok();
    }
}

#[cfg(unix)]
fn set_mode(path: &std::path::Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

/// Accepts connections until shutdown, serving each on a thread of its
/// own. `accept` gives `Ok(None)` for a connection it refused.
fn accept_loop<S: Connection + 'static>(
    shared: &Arc<Shared>,
    mut accept: impl FnMut() -> io::Result<Option<S>>,
) {
    while !shared.shutdown.load(Ordering::Acquire) {
        match accept() {
            Ok(Some(stream)) => serve(shared, stream),
            Ok(None) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => thread::sleep(POLL),
            Err(error) => {
                if !shared.shutdown.load(Ordering::Acquire) {
                    tracing::error!("rinch-debug accept error: {error}");
                }
                break;
            }
        }
    }
}

/// Serves `stream` on a thread of its own, remembering it so a shutdown can
/// end it.
fn serve<S: Connection + 'static>(shared: &Arc<Shared>, stream: S) {
    let Ok(handle) = stream.try_clone_connection() else {
        return;
    };
    let client_shared = Arc::clone(shared);
    let spawned = thread::Builder::new()
        .name("rinch-debug-client".into())
        .spawn(move || handle_client(stream, &client_shared));
    let Ok(thread) = spawned else {
        tracing::error!("rinch-debug: cannot start a client thread");
        return;
    };
    let mut clients = shared
        .clients
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    clients.retain(|client| !client.thread.is_finished());
    clients.push(ClientThread {
        stream: handle,
        thread,
    });
}

fn handle_client(mut stream: impl Connection, shared: &Shared) {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();

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
        let err = serde_json::to_vec(&serde_json::json!({"error": "unsupported protocol/version"}))
            .unwrap_or_default();
        let _ = write_frame(&mut stream, &err);
        return;
    }
    let response = HandshakeResponse {
        protocol: "rinch-debug".into(),
        version: 1,
        app_name: shared.app_name.clone(),
        pid: std::process::id(),
    };
    let Ok(resp_data) = serde_json::to_vec(&response) else {
        return;
    };
    if write_frame(&mut stream, &resp_data).is_err() {
        return;
    }

    while !shared.shutdown.load(Ordering::Acquire) {
        let req_data = match read_frame(&mut stream) {
            Ok(data) => data,
            Err(e)
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(_) => break, // Client disconnected
        };
        let response = match serde_json::from_slice::<Request>(&req_data) {
            Ok(request) => Response {
                id: request.id,
                result: shared.sender.execute(request.command),
            },
            Err(e) => Response {
                id: 0,
                result: DebugResult::Error {
                    message: format!("Invalid request: {}", e),
                },
            },
        };
        let Ok(data) = serde_json::to_vec(&response) else {
            break;
        };
        if write_frame(&mut stream, &data).is_err() {
            break;
        }
    }
}

impl Drop for DebugServer {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Release);
        // The listeners poll the flag between accepts.
        for listener in self.listeners.drain(..) {
            let _ = listener.join();
        }
        // End every client's stream, which wakes its thread from a read.
        let clients = std::mem::take(
            &mut *self
                .shared
                .clients
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        for client in &clients {
            let _ = client.stream.shutdown();
        }
        for client in clients {
            let _ = client.thread.join();
        }
        if let Some(path) = &self.socket_path {
            let _ = std::fs::remove_file(path);
        }
        if let Some(path) = &self.discovery_file {
            let _ = std::fs::remove_file(path);
        }
    }
}
