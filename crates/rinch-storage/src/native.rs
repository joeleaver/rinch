//! Native (non-wasm) [`Store`] backed by a directory of blob files.
//!
//! One [`FsStore`] owns a base directory; each key maps to a single file
//! directly under it. The key is reversibly encoded to a filesystem-safe name
//! (see [`encode_key`]): every byte outside `[A-Za-z0-9_-]` — crucially `/`, `:`
//! and `.` — becomes `%XX`. So a key never creates a subdirectory, never escapes
//! the base via `..`, and never collides with the internal temp files; and
//! [`list`](Store::list) recovers the original keys by decoding the file names.
//!
//! Writes are **atomic**: bytes go to a hidden temp file, then `rename` into
//! place (atomic on the same filesystem), so an interrupted write leaves the old
//! value intact rather than a truncated one. That atomic-replace is the
//! durability guarantee the offline-first snapshot store depends on.
//!
//! The filesystem is blocking, so each op does its work when the returned future
//! is first polled and resolves immediately. That is fine for the small, infrequent
//! blobs this serves (an Automerge snapshot per save); offloading to a worker
//! thread is a possible future optimization, not needed now.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{StorageError, StorageFuture, StorageResult, Store};

/// A [`Store`] persisting each key as a file under a base directory.
///
/// Cheap to [`clone`](Clone) (the base path is behind an `Arc`); a clone points
/// at the same directory.
#[derive(Clone)]
pub struct FsStore {
    base: Arc<PathBuf>,
}

impl FsStore {
    /// Open (creating if needed) a store rooted at `base`.
    ///
    /// The directory is created eagerly so later writes don't race on it. Two
    /// `FsStore`s pointing at the same `base` see the same data — that is the
    /// durability path the tests exercise (write, drop, reopen, read back).
    pub fn open(base: impl Into<PathBuf>) -> StorageResult<Self> {
        let base = base.into();
        std::fs::create_dir_all(&base)
            .map_err(|e| StorageError::Io(format!("create_dir_all {}: {e}", base.display())))?;
        Ok(Self {
            base: Arc::new(base),
        })
    }

    fn path_for(&self, key: &str) -> StorageResult<PathBuf> {
        if key.is_empty() {
            return Err(StorageError::InvalidKey("empty key".to_string()));
        }
        Ok(self.base.join(encode_key(key)))
    }
}

impl Store for FsStore {
    fn get(&self, key: &str) -> StorageFuture<Option<Vec<u8>>> {
        let path = self.path_for(key);
        Box::pin(async move {
            let path = path?;
            match std::fs::read(&path) {
                Ok(bytes) => Ok(Some(bytes)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(StorageError::Io(format!("read {}: {e}", path.display()))),
            }
        })
    }

    fn put(&self, key: &str, value: &[u8]) -> StorageFuture<()> {
        let path = self.path_for(key);
        let base = self.base.clone();
        let value = value.to_vec();
        Box::pin(async move {
            let path = path?;
            let tmp = base.join(temp_name());
            std::fs::write(&tmp, &value)
                .map_err(|e| StorageError::Io(format!("write {}: {e}", tmp.display())))?;
            // Atomic replace. On failure, don't leak the temp file.
            if let Err(e) = std::fs::rename(&tmp, &path) {
                let _ = std::fs::remove_file(&tmp);
                return Err(StorageError::Io(format!(
                    "rename {} -> {}: {e}",
                    tmp.display(),
                    path.display()
                )));
            }
            Ok(())
        })
    }

    fn delete(&self, key: &str) -> StorageFuture<()> {
        let path = self.path_for(key);
        Box::pin(async move {
            let path = path?;
            match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                // Absent key is not an error.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(StorageError::Io(format!("remove {}: {e}", path.display()))),
            }
        })
    }

    fn list(&self, prefix: &str) -> StorageFuture<Vec<String>> {
        let base = self.base.clone();
        let prefix = prefix.to_string();
        Box::pin(async move {
            let mut keys = Vec::new();
            let entries = match std::fs::read_dir(&*base) {
                Ok(rd) => rd,
                // A never-written store lists as empty rather than erroring.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(keys),
                Err(e) => {
                    return Err(StorageError::Io(format!(
                        "read_dir {}: {e}",
                        base.display()
                    )));
                }
            };
            for entry in entries {
                let entry =
                    entry.map_err(|e| StorageError::Io(format!("dir entry: {e}")))?;
                let name = entry.file_name();
                let Some(name) = name.to_str() else { continue };
                // Skip internal temp files. An encoded key never starts with '.'
                // (a literal '.' encodes to "%2E"), so this only drops our own.
                if name.starts_with('.') {
                    continue;
                }
                if let Some(key) = decode_key(name)
                    && key.starts_with(&prefix)
                {
                    keys.push(key);
                }
            }
            Ok(keys)
        })
    }
}

/// Reversibly encode a key to a single filesystem-safe file name.
///
/// Keeps `[A-Za-z0-9_-]` verbatim; percent-encodes every other byte as `%XX`.
/// That neutralizes `/`, `:`, `.`, whitespace and non-ASCII, so the key maps to
/// exactly one flat file with no traversal and no `.`/`..` names.
fn encode_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for &b in key.as_bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(hex_digit(b >> 4));
            out.push(hex_digit(b & 0x0f));
        }
    }
    out
}

/// Inverse of [`encode_key`]. Returns `None` for a malformed name (not one we
/// wrote), so a stray file in the directory is skipped rather than crashing.
fn decode_key(name: &str) -> Option<String> {
    let bytes = name.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hi = from_hex(bytes[i + 1])?;
            let lo = from_hex(bytes[i + 2])?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hex_digit(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        _ => (b'A' + (n - 10)) as char,
    }
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// A unique, hidden temp file name for the write-then-rename dance.
///
/// The `.` prefix keeps it out of [`list`](Store::list) results, and the pid +
/// process-lifetime counter make concurrent writers (and concurrent keys) pick
/// distinct temp files.
fn temp_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(".tmp-{pid}-{n}-{nanos}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Namespace;
    use std::future::Future;
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

    /// Minimal executor for the tests. The fs futures do their (blocking) work on
    /// the first poll and resolve immediately, so a single poll with a no-op waker
    /// is enough — no async runtime dependency needed.
    fn block_on<F: Future>(fut: F) -> F::Output {
        fn noop(_: *const ()) {}
        fn clone(_: *const ()) -> RawWaker {
            RawWaker::new(std::ptr::null(), &VTABLE)
        }
        static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
        let mut cx = Context::from_waker(&waker);
        let mut fut = Box::pin(fut);
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => v,
            Poll::Pending => panic!("fs future unexpectedly pended"),
        }
    }

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().expect("create tempdir")
    }

    #[test]
    fn put_get_delete_round_trip() {
        let dir = tempdir();
        let store = FsStore::open(dir.path()).unwrap();

        assert_eq!(block_on(store.get("missing")).unwrap(), None);

        block_on(store.put("a", b"hello")).unwrap();
        assert_eq!(block_on(store.get("a")).unwrap(), Some(b"hello".to_vec()));

        // Overwrite replaces.
        block_on(store.put("a", b"world!")).unwrap();
        assert_eq!(block_on(store.get("a")).unwrap(), Some(b"world!".to_vec()));

        // Delete is real, and idempotent.
        block_on(store.delete("a")).unwrap();
        assert_eq!(block_on(store.get("a")).unwrap(), None);
        block_on(store.delete("a")).unwrap();
    }

    #[test]
    fn list_filters_by_prefix_and_recovers_keys() {
        let dir = tempdir();
        let store = FsStore::open(dir.path()).unwrap();

        // Keys with the delimiters the fs encoding must neutralize.
        block_on(store.put("chapter:1/snapshot", b"s1")).unwrap();
        block_on(store.put("chapter:1/changes/0001", b"c1")).unwrap();
        block_on(store.put("chapter:2/snapshot", b"s2")).unwrap();

        let mut all = block_on(store.list("")).unwrap();
        all.sort();
        assert_eq!(
            all,
            vec![
                "chapter:1/changes/0001".to_string(),
                "chapter:1/snapshot".to_string(),
                "chapter:2/snapshot".to_string(),
            ]
        );

        let mut ch1 = block_on(store.list("chapter:1/")).unwrap();
        ch1.sort();
        assert_eq!(
            ch1,
            vec![
                "chapter:1/changes/0001".to_string(),
                "chapter:1/snapshot".to_string(),
            ]
        );

        assert!(block_on(store.list("nope")).unwrap().is_empty());
    }

    #[test]
    fn unicode_and_delimiter_keys_round_trip() {
        let dir = tempdir();
        let store = FsStore::open(dir.path()).unwrap();
        for key in ["a b", "emoji/📚", "colon:sep", "dot.name", "..", "%literal"] {
            block_on(store.put(key, key.as_bytes())).unwrap();
            assert_eq!(
                block_on(store.get(key)).unwrap(),
                Some(key.as_bytes().to_vec()),
                "round-trip failed for key {key:?}"
            );
        }
        // ".." must be a stored key, not the parent directory.
        assert_eq!(
            block_on(store.get("..")).unwrap(),
            Some(b"..".to_vec())
        );
    }

    #[test]
    fn empty_key_is_rejected() {
        let dir = tempdir();
        let store = FsStore::open(dir.path()).unwrap();
        assert!(matches!(
            block_on(store.put("", b"x")),
            Err(StorageError::InvalidKey(_))
        ));
    }

    /// The durability guarantee: bytes written by one `FsStore` survive a fresh
    /// `FsStore` pointed at the same directory (i.e. an app restart).
    #[test]
    fn bytes_survive_a_fresh_store_instance() {
        let dir = tempdir();
        let payload = vec![0u8, 1, 2, 250, 251, 252, 0, 255];
        {
            let store = FsStore::open(dir.path()).unwrap();
            block_on(store.put("chapter:x/snapshot", &payload)).unwrap();
        } // drop the store entirely

        let reopened = FsStore::open(dir.path()).unwrap();
        assert_eq!(
            block_on(reopened.get("chapter:x/snapshot")).unwrap(),
            Some(payload)
        );
    }

    /// The round-trip the persistence spike proved, now through `rinch-storage`:
    /// snapshot an Automerge doc, persist the bytes, drop the store, reopen, read
    /// the bytes back byte-identical, and reload them into a working Automerge doc.
    #[test]
    fn automerge_snapshot_round_trip() {
        use automerge::transaction::Transactable;
        use automerge::{AutoCommit, ReadDoc, ROOT};

        let dir = tempdir();

        // Author a doc and snapshot it to bytes (what the editor collab seam hands us).
        let snapshot = {
            let mut doc = AutoCommit::new();
            doc.put(ROOT, "title", "Chapter One").unwrap();
            doc.put(ROOT, "body", "The lantern guttered against the fog.")
                .unwrap();
            doc.save()
        };

        // Persist, then fully drop the store.
        {
            let store = FsStore::open(dir.path()).unwrap();
            block_on(store.put("chapter:one:snapshot", &snapshot)).unwrap();
        }

        // Reopen and read back.
        let store = FsStore::open(dir.path()).unwrap();
        let restored = block_on(store.get("chapter:one:snapshot"))
            .unwrap()
            .expect("snapshot present after reopen");

        // Contract 1: byte-identical in and out.
        assert_eq!(restored, snapshot, "persisted bytes must be identical");

        // Contract 2: the bytes still load into a working Automerge doc.
        let doc = AutoCommit::load(&restored).expect("reload automerge doc");
        let title = doc.get(ROOT, "title").unwrap().unwrap().0;
        assert_eq!(title.into_string().unwrap(), "Chapter One");
    }

    /// A `Namespace` partitions one physical store, and its `list` is prefix-relative.
    #[test]
    fn namespace_scopes_keys() {
        let dir = tempdir();
        let store = FsStore::open(dir.path()).unwrap();

        let ch1 = Namespace::new(store.clone(), "chapter:1/");
        let ch2 = Namespace::new(store.clone(), "chapter:2/");

        block_on(ch1.put("snapshot", b"one")).unwrap();
        block_on(ch2.put("snapshot", b"two")).unwrap();

        // Same relative key, no collision.
        assert_eq!(block_on(ch1.get("snapshot")).unwrap(), Some(b"one".to_vec()));
        assert_eq!(block_on(ch2.get("snapshot")).unwrap(), Some(b"two".to_vec()));

        // Namespaced list is relative (prefix stripped).
        assert_eq!(
            block_on(ch1.list("")).unwrap(),
            vec!["snapshot".to_string()]
        );

        // The physical store sees the fully-qualified keys.
        let mut physical = block_on(store.list("")).unwrap();
        physical.sort();
        assert_eq!(
            physical,
            vec!["chapter:1/snapshot".to_string(), "chapter:2/snapshot".to_string()]
        );
    }
}
