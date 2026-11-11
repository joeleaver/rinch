//! Finding running apps and talking to them: the one client implementation
//! every tool uses (the MCP server, the visual tests, the end-to-end tests).
//!
//! A listing ([`Listing::apps`]) has the apps that serve the socket
//! transport first. Then, for a period, it also has the apps that offer
//! only the legacy TCP port: apps built on rinch revisions from before the
//! socket, found through `~/.rinch/debug/*.json`. Each [`App`] says which
//! [`Transport`] it is reached by, so a tool can show the user which apps
//! are still open to every local user. An app that offers both is listed
//! once, by its socket.

use std::collections::HashSet;
use std::io;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::discovery;
use crate::protocol::{
    DebugCommandKind, DebugResult, HandshakeRequest, HandshakeResponse, Request, Response,
    read_frame, write_frame,
};
use crate::transport::{Connection, Transport};

/// How long a handshake may take: the app's listener answers it itself, so
/// a live app answers at once even while its main thread is busy.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a command may take before the client gives up on it.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

/// A running app a client can connect to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    /// Its process id.
    pub pid: u32,
    /// Its name, as it gave it.
    pub app_name: String,
    /// How it is reached.
    pub transport: Transport,
    /// Where it is reached.
    pub endpoint: Endpoint,
}

/// Where an app's debug server is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// A Unix socket.
    Socket(PathBuf),
    /// A TCP port on `127.0.0.1`.
    Tcp(u16),
}

/// Where a listing looks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    /// The socket directory; `None` where there is none to trust (see
    /// [`crate::location::Locations::existing_socket_dir`]).
    pub socket_dir: Option<PathBuf>,
    /// The legacy discovery directory, `~/.rinch/debug`; `None` to leave
    /// legacy apps out.
    pub legacy_dir: Option<PathBuf>,
}

impl Listing {
    /// Where this process looks: `RINCH_DEBUG_DIR` if set (an explicit
    /// reach into another session's socket directory, such as an isolated
    /// test session's), else the runtime directory's `rinch/debug`; and
    /// `~/.rinch/debug` for legacy apps.
    #[must_use]
    pub fn from_process() -> Self {
        let socket_dir = match std::env::var_os("RINCH_DEBUG_DIR") {
            Some(dir) => Some(PathBuf::from(dir)),
            #[cfg(unix)]
            None => crate::location::Locations::from_process().existing_socket_dir(),
            #[cfg(not(unix))]
            None => None,
        };
        Self {
            socket_dir,
            legacy_dir: Some(discovery::discovery_dir()),
        }
    }

    /// Every app found, socket apps first, each by pid.
    ///
    /// A socket that refuses connections belongs to an app that died
    /// without cleaning up, and is removed. A legacy entry counts only
    /// while its process is alive, and is left as it is either way.
    #[must_use]
    pub fn apps(&self) -> Vec<App> {
        let mut apps = self
            .socket_dir
            .as_deref()
            .map(socket_apps)
            .unwrap_or_default();
        apps.sort_by_key(|app| app.pid);
        let by_socket: HashSet<u32> = apps.iter().map(|app| app.pid).collect();
        let mut legacy: Vec<App> = self
            .legacy_dir
            .as_deref()
            .map(discovery::read_discovery_entries)
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| !by_socket.contains(&entry.pid) && alive(entry.pid))
            .map(|entry| App {
                pid: entry.pid,
                app_name: entry.app_name,
                transport: Transport::LegacyTcp,
                endpoint: Endpoint::Tcp(entry.port),
            })
            .collect();
        legacy.sort_by_key(|app| app.pid);
        apps.extend(legacy);
        apps
    }
}

#[cfg(unix)]
fn socket_apps(dir: &Path) -> Vec<App> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension().is_none_or(|extension| extension != "sock") {
                return None;
            }
            match Client::connect_socket(&path) {
                Ok(client) => Some(App {
                    pid: client.pid,
                    app_name: client.app_name,
                    transport: Transport::Socket,
                    endpoint: Endpoint::Socket(path),
                }),
                Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
                    remove_dead_socket(&path);
                    None
                }
                Err(_) => None,
            }
        })
        .collect()
}

#[cfg(not(unix))]
fn socket_apps(_dir: &Path) -> Vec<App> {
    Vec::new()
}

/// Removes a socket no process listens on any more, if it is a socket.
#[cfg(unix)]
fn remove_dead_socket(path: &Path) {
    use std::os::unix::fs::FileTypeExt;
    if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_socket()) {
        std::fs::remove_file(path).ok();
    }
}

/// Whether the process `pid` exists (any process: a signal-0 probe).
#[cfg(unix)]
fn alive(pid: u32) -> bool {
    let Some(pid) = i32::try_from(pid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    else {
        return false;
    };
    match rustix::process::test_kill_process(pid) {
        Ok(()) => true,
        // It exists, under another user.
        Err(error) => error == rustix::io::Errno::PERM,
    }
}

/// Assumed alive where there is no cheap probe: connecting fails if not.
#[cfg(not(unix))]
fn alive(_pid: u32) -> bool {
    true
}

/// A connection to one app's debug server, past the handshake.
pub struct Client {
    stream: Box<dyn Connection>,
    next_id: u64,
    /// The app's name, from the handshake.
    pub app_name: String,
    /// The app's process id, from the handshake.
    pub pid: u32,
    /// How the app was reached.
    pub transport: Transport,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("app_name", &self.app_name)
            .field("pid", &self.pid)
            .field("transport", &self.transport)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// Connects to `app` where its listing found it.
    ///
    /// # Errors
    ///
    /// When the app cannot be reached or does not complete the handshake.
    pub fn connect(app: &App) -> io::Result<Self> {
        match &app.endpoint {
            #[cfg(unix)]
            Endpoint::Socket(path) => Self::connect_socket(path),
            #[cfg(not(unix))]
            Endpoint::Socket(_) => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "no socket transport on this platform",
            )),
            Endpoint::Tcp(port) => Self::connect_tcp(*port),
        }
    }

    /// Connects to the app serving the socket at `path`.
    ///
    /// # Errors
    ///
    /// When nothing listens there (`ConnectionRefused` for a dead app's
    /// socket), or the handshake fails.
    #[cfg(unix)]
    pub fn connect_socket(path: &Path) -> io::Result<Self> {
        let stream = std::os::unix::net::UnixStream::connect(path)?;
        Self::handshake(Box::new(stream), Transport::Socket)
    }

    /// Connects to the app on the legacy TCP `port` of `127.0.0.1`.
    ///
    /// # Errors
    ///
    /// When nothing listens there, or the handshake fails.
    pub fn connect_tcp(port: u16) -> io::Result<Self> {
        let stream = TcpStream::connect(("127.0.0.1", port))?;
        Self::handshake(Box::new(stream), Transport::LegacyTcp)
    }

    fn handshake(mut stream: Box<dyn Connection>, transport: Transport) -> io::Result<Self> {
        stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
        let request = HandshakeRequest {
            protocol: "rinch-debug".into(),
            version: 1,
        };
        write_frame(
            &mut stream,
            &serde_json::to_vec(&request).map_err(io::Error::other)?,
        )?;
        let reply = read_frame(&mut stream)?;
        let response: HandshakeResponse = serde_json::from_slice(&reply)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        stream.set_read_timeout(Some(COMMAND_TIMEOUT))?;
        Ok(Self {
            stream,
            next_id: 1,
            app_name: response.app_name,
            pid: response.pid,
            transport,
        })
    }

    /// Runs `command` in the app and returns its result.
    ///
    /// # Errors
    ///
    /// When the connection fails, or the answer is not this command's.
    pub fn execute(&mut self, command: DebugCommandKind) -> io::Result<DebugResult> {
        let id = self.next_id;
        self.next_id += 1;
        let request = Request { id, command };
        write_frame(
            &mut self.stream,
            &serde_json::to_vec(&request).map_err(io::Error::other)?,
        )?;
        let reply = read_frame(&mut self.stream)?;
        let response: Response = serde_json::from_slice(&reply)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if response.id != id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("answer to request {} while waiting for {id}", response.id),
            ));
        }
        Ok(response.result)
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
