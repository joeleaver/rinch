//! Who is on the other end of a debug socket, and whether to let them in.
//!
//! Every connection to the socket is checked before the handshake: the
//! peer's user id must be this process's effective user id. Root is refused
//! like any other user. The socket's directory already keeps other users
//! out (see [`crate::location`]); the check also covers a directory whose
//! permissions someone loosened.

use std::io;
use std::os::unix::net::UnixStream;
use std::sync::Arc;

/// The credentials of a connected peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Peer {
    /// Its effective user id.
    pub uid: u32,
    /// Its process id, where the platform reports it.
    pub pid: Option<u32>,
}

/// Reads a connected peer's credentials: [`peer_of`] in an app, a stand-in
/// in tests.
pub type PeerCredentials = Arc<dyn Fn(&UnixStream) -> io::Result<Peer> + Send + Sync>;

/// Why a peer was refused.
#[derive(Debug)]
pub enum Refusal {
    /// It runs as another user.
    OtherUser(Peer),
    /// Its credentials could not be read.
    Unknown(io::Error),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OtherUser(Peer {
                uid,
                pid: Some(pid),
            }) => write!(f, "uid {uid} (pid {pid}) is not this app's user"),
            Self::OtherUser(Peer { uid, pid: None }) => {
                write!(f, "uid {uid} is not this app's user")
            }
            Self::Unknown(error) => write!(f, "its credentials cannot be read: {error}"),
        }
    }
}

/// Whether to let the peer whose credentials are `credentials` in, when
/// this process runs as `own_uid`.
///
/// # Errors
///
/// [`Refusal`] for another user, root included, or unreadable credentials.
pub fn admit(own_uid: u32, credentials: io::Result<Peer>) -> Result<Peer, Refusal> {
    let peer = credentials.map_err(Refusal::Unknown)?;
    if peer.uid == own_uid {
        Ok(peer)
    } else {
        Err(Refusal::OtherUser(peer))
    }
}

/// The kernel's account of the peer: `SO_PEERCRED` on Linux and Android.
///
/// # Errors
///
/// When the socket option cannot be read.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn peer_of(stream: &UnixStream) -> io::Result<Peer> {
    let credentials = rustix::net::sockopt::socket_peercred(stream)?;
    Ok(Peer {
        uid: credentials.uid.as_raw(),
        pid: u32::try_from(credentials.pid.as_raw_nonzero().get()).ok(),
    })
}

/// The kernel's account of the peer: `getpeereid` on the BSDs and Apple
/// platforms, which give no process id.
///
/// # Errors
///
/// When `getpeereid` fails.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub fn peer_of(stream: &UnixStream) -> io::Result<Peer> {
    let (uid, _gid) = nix::unistd::getpeereid(stream).map_err(io::Error::from)?;
    Ok(Peer {
        uid: uid.as_raw(),
        pid: None,
    })
}

/// [`peer_of`], as the [`PeerCredentials`] an app uses.
#[must_use]
pub fn kernel_credentials() -> PeerCredentials {
    Arc::new(peer_of)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admits_only_the_same_user() {
        let own = Peer {
            uid: 1000,
            pid: Some(7),
        };
        assert_eq!(admit(1000, Ok(own)).unwrap(), own);
        for uid in [1001, 0] {
            let foreign = Peer { uid, pid: Some(8) };
            let refusal = admit(1000, Ok(foreign)).unwrap_err();
            assert!(matches!(refusal, Refusal::OtherUser(peer) if peer == foreign));
        }
        let unreadable = admit(1000, Err(io::Error::other("no SO_PEERCRED"))).unwrap_err();
        assert!(matches!(unreadable, Refusal::Unknown(_)));
    }

    #[test]
    fn reads_its_own_credentials_from_the_kernel() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let peer = peer_of(&ours).unwrap();
        assert_eq!(peer.uid, rustix::process::geteuid().as_raw());
        if cfg!(any(target_os = "linux", target_os = "android")) {
            assert_eq!(peer.pid, Some(std::process::id()));
        }
        drop(theirs);
    }
}
