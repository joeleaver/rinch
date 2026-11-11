//! The byte streams the protocol runs over, and how an app reaches its
//! listeners: a Unix socket owner-only on Unix, and the legacy TCP port on
//! `127.0.0.1` (until it is removed on the platforms that have the socket).

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::time::Duration;

/// How a client reaches an app's debug server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Transport {
    /// A Unix socket in a directory only this user can enter, its peers
    /// checked: reachable by this user alone.
    Socket,
    /// A TCP port on `127.0.0.1`, found through `~/.rinch/debug/*.json`:
    /// what rinch apps built before the socket transport offer. Open to
    /// every local user and any sandboxed app with network access, and its
    /// peers are not checked.
    LegacyTcp,
}

impl Transport {
    /// The name a listing shows: `socket` or `legacy-tcp`.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Socket => "socket",
            Self::LegacyTcp => "legacy-tcp",
        }
    }

    /// Whether only this user can reach an app over it.
    #[must_use]
    pub fn is_restricted(self) -> bool {
        matches!(self, Self::Socket)
    }
}

impl std::fmt::Display for Transport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// A connected stream of either transport.
pub trait Connection: Read + Write + Send {
    /// How long a read waits before it fails with `WouldBlock` or
    /// `TimedOut`.
    ///
    /// # Errors
    ///
    /// As the stream's own `set_read_timeout`.
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()>;

    /// Ends both directions, waking a thread blocked reading it.
    ///
    /// # Errors
    ///
    /// As the stream's own `shutdown`.
    fn shutdown(&self) -> io::Result<()>;

    /// A second handle to the same stream.
    ///
    /// # Errors
    ///
    /// As the stream's own `try_clone`.
    fn try_clone_connection(&self) -> io::Result<Box<dyn Connection>>;
}

impl Connection for TcpStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        TcpStream::set_read_timeout(self, timeout)
    }

    fn shutdown(&self) -> io::Result<()> {
        TcpStream::shutdown(self, Shutdown::Both)
    }

    fn try_clone_connection(&self) -> io::Result<Box<dyn Connection>> {
        Ok(Box::new(self.try_clone()?))
    }
}

#[cfg(unix)]
impl Connection for std::os::unix::net::UnixStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        std::os::unix::net::UnixStream::set_read_timeout(self, timeout)
    }

    fn shutdown(&self) -> io::Result<()> {
        std::os::unix::net::UnixStream::shutdown(self, Shutdown::Both)
    }

    fn try_clone_connection(&self) -> io::Result<Box<dyn Connection>> {
        Ok(Box::new(self.try_clone()?))
    }
}
