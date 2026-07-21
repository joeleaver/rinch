//! Storage error type.

use thiserror::Error;

/// Errors from a [`Store`](crate::Store) operation.
///
/// A **missing key is not an error** — [`Store::get`](crate::Store::get) returns
/// `Ok(None)` for it, and [`Store::delete`](crate::Store::delete) of an absent
/// key is `Ok(())` (idempotent). Only a genuine failure to complete the
/// operation surfaces here, so the normal path never allocates an error.
#[derive(Debug, Error)]
pub enum StorageError {
    /// A native filesystem operation failed (read/write/rename/`read_dir`).
    #[error("storage io error: {0}")]
    Io(String),

    /// A web-backend (IndexedDB) operation failed. Carries the JS error text.
    #[error("storage backend error: {0}")]
    Backend(String),

    /// The key could not be used: empty, or otherwise unrepresentable on the
    /// backend. Keys are opaque UTF-8 strings; the native backend encodes any
    /// byte to a filesystem-safe form, so this is reserved for the empty key.
    #[error("invalid storage key: {0}")]
    InvalidKey(String),

    /// The backend is not available in this environment — e.g. no `window` /
    /// no IndexedDB (a worker without it, or a privacy mode that blocks it).
    #[error("storage unavailable: {0}")]
    Unavailable(String),
}
