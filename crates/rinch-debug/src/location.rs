//! Where an app's debug socket lives: a directory only its user can enter.
//!
//! The directory is `<runtime>/rinch/debug`, where `<runtime>` is the first
//! of these that passes the checks below:
//!
//! 1. `$XDG_RUNTIME_DIR`, when it is absolute, a directory (not a link),
//!    owned by this user and writable by no one else;
//! 2. on Apple platforms, `$TMPDIR` under the same conditions (it is a
//!    per-user directory there);
//! 3. `/tmp/rinch-<uid>`, created with mode `0700` if missing; if it exists
//!    it must be a directory, not a link, owned by this user, with mode
//!    exactly `0700`.
//!
//! `rinch/` and `rinch/debug/` are created with mode `0700` and must, if
//! they exist, be ours with that mode. When the third choice fails its
//! checks there is no socket at all: the debug server never falls back to a
//! place another user could reach.
//!
//! Everything here takes its inputs as a [`Locations`] value, so tests
//! resolve inside a temporary directory and never touch the real runtime
//! directory.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// The inputs directory resolution reads: the environment, the user, and
/// where the last fallback goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locations {
    /// `$XDG_RUNTIME_DIR`.
    pub xdg_runtime_dir: Option<PathBuf>,
    /// `$TMPDIR`, used only when [`prefer_tmpdir`](Self::prefer_tmpdir).
    pub tmpdir: Option<PathBuf>,
    /// Whether `$TMPDIR` is a per-user directory to use before the shared
    /// fallback (Apple platforms).
    pub prefer_tmpdir: bool,
    /// The directory the last fallback, `rinch-<uid>`, is made in: `/tmp`.
    pub fallback_root: PathBuf,
    /// The effective user id the directories must belong to.
    pub uid: u32,
}

/// Why no socket directory could be used.
#[derive(Debug)]
pub enum LocationError {
    /// A directory that must be this user's own is not: it is a link, not a
    /// directory, owned by someone else, or open to others.
    Unsafe {
        /// The directory.
        path: PathBuf,
        /// What is wrong with it.
        reason: &'static str,
    },
    /// A directory could not be created or inspected.
    Io {
        /// The directory.
        path: PathBuf,
        /// The error.
        source: io::Error,
    },
}

impl std::fmt::Display for LocationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsafe { path, reason } => write!(f, "{} {reason}", path.display()),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}

impl std::error::Error for LocationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Unsafe { .. } => None,
        }
    }
}

impl Locations {
    /// The inputs of this process: its environment and effective user.
    #[must_use]
    pub fn from_process() -> Self {
        Self::from_vars(
            |name| std::env::var_os(name),
            rustix::process::geteuid().as_raw(),
        )
    }

    /// The inputs `var` (an environment lookup) and `uid` give.
    pub fn from_vars(var: impl Fn(&str) -> Option<OsString>, uid: u32) -> Self {
        Self {
            xdg_runtime_dir: var("XDG_RUNTIME_DIR").map(PathBuf::from),
            tmpdir: var("TMPDIR").map(PathBuf::from),
            prefer_tmpdir: cfg!(target_vendor = "apple"),
            fallback_root: PathBuf::from("/tmp"),
            uid,
        }
    }

    /// The runtime directory sockets go under, by the rules in the module
    /// documentation, creating the last fallback if it is needed.
    ///
    /// # Errors
    ///
    /// When neither `$XDG_RUNTIME_DIR` nor `$TMPDIR` passes the checks and
    /// the fallback cannot be made or fails them.
    pub fn runtime_dir(&self) -> Result<PathBuf, LocationError> {
        let candidates = [
            self.xdg_runtime_dir.as_ref(),
            self.tmpdir.as_ref().filter(|_| self.prefer_tmpdir),
        ];
        for candidate in candidates.into_iter().flatten() {
            if candidate.is_absolute() && check_private_enough(candidate, self.uid).is_ok() {
                return Ok(candidate.clone());
            }
        }
        let fallback = self.fallback_root.join(format!("rinch-{}", self.uid));
        ensure_private_dir(&fallback, self.uid)?;
        Ok(fallback)
    }

    /// `<runtime>/rinch/debug`, created owner-only if missing: where an
    /// app's server puts its socket.
    ///
    /// # Errors
    ///
    /// As [`runtime_dir`](Self::runtime_dir), and when `rinch/` or
    /// `rinch/debug/` cannot be made or is not this user's alone.
    pub fn create_socket_dir(&self) -> Result<PathBuf, LocationError> {
        let runtime = self.runtime_dir()?;
        let rinch = runtime.join("rinch");
        ensure_private_dir(&rinch, self.uid)?;
        let debug = rinch.join("debug");
        ensure_private_dir(&debug, self.uid)?;
        Ok(debug)
    }

    /// `<runtime>/rinch/debug` if it exists and is this user's alone, for a
    /// client listing sockets; a client creates nothing. `None` when there
    /// is no such directory, or it fails the checks (a client never
    /// connects into a directory someone else controls).
    #[must_use]
    pub fn existing_socket_dir(&self) -> Option<PathBuf> {
        let candidates = [
            self.xdg_runtime_dir.clone(),
            self.tmpdir.clone().filter(|_| self.prefer_tmpdir),
            Some(self.fallback_root.join(format!("rinch-{}", self.uid))),
        ];
        candidates
            .into_iter()
            .flatten()
            .filter(|runtime| runtime.is_absolute())
            .map(|runtime| runtime.join("rinch").join("debug"))
            .find(|dir| check_private(dir, self.uid).is_ok())
    }
}

/// Creates `dir` with mode `0700` if it is missing, then checks it is this
/// user's alone ([`check_private`]).
pub(crate) fn ensure_private_dir(dir: &Path, uid: u32) -> Result<(), LocationError> {
    match fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(source) => {
            return Err(LocationError::Io {
                path: dir.into(),
                source,
            });
        }
    }
    check_private(dir, uid)
}

/// That `dir` is a directory, not a link, owned by `uid`, with mode exactly
/// `0700`.
pub(crate) fn check_private(dir: &Path, uid: u32) -> Result<(), LocationError> {
    let metadata = owned_directory(dir, uid)?;
    if metadata.permissions().mode() & 0o777 != 0o700 {
        return Err(unsafe_dir(dir, "is not mode 0700"));
    }
    Ok(())
}

/// That `dir` is a directory, not a link, owned by `uid` and writable by no
/// one else: what a runtime directory such as `$XDG_RUNTIME_DIR` must be.
pub(crate) fn check_private_enough(dir: &Path, uid: u32) -> Result<(), LocationError> {
    let metadata = owned_directory(dir, uid)?;
    if metadata.permissions().mode() & 0o022 != 0 {
        return Err(unsafe_dir(dir, "is writable by others"));
    }
    Ok(())
}

fn owned_directory(dir: &Path, uid: u32) -> Result<fs::Metadata, LocationError> {
    let metadata = fs::symlink_metadata(dir).map_err(|source| LocationError::Io {
        path: dir.into(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(unsafe_dir(dir, "is a symbolic link"));
    }
    if !metadata.is_dir() {
        return Err(unsafe_dir(dir, "is not a directory"));
    }
    if metadata.uid() != uid {
        return Err(unsafe_dir(dir, "belongs to another user"));
    }
    Ok(metadata)
}

fn unsafe_dir(dir: &Path, reason: &'static str) -> LocationError {
    LocationError::Unsafe {
        path: dir.into(),
        reason,
    }
}

#[cfg(test)]
#[path = "location_tests.rs"]
mod tests;
