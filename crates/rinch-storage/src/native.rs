//! Native (non-wasm) [`Store`] backed by a directory of blob files.
//!
//! One [`FsStore`] owns a base directory; each key maps to a single file
//! directly under it. The key is reversibly encoded to a filesystem-safe name
//! (see [`encode_key`]): every byte outside `[A-Za-z0-9_-]` — crucially `/`, `:`
//! and `.` — becomes `%XX`. So a key never creates a subdirectory, never escapes
//! the base via `..`, and never collides with the internal temp files; and
//! [`list`](Store::list) recovers the original keys by decoding the file names.
//!
//! Writes are **atomic and durable**, which are two separate properties and need
//! two separate mechanisms:
//!
//! - *Atomic*: bytes go to a hidden temp file, then `rename` into place (atomic on
//!   the same filesystem), so an interrupted write leaves the old value intact
//!   rather than a truncated one.
//! - *Durable*: the temp file is `fsync`ed before the rename, and the base
//!   directory is `fsync`ed after it. Without the first, a crash can leave the
//!   renamed file present but empty or partially written — the rename metadata
//!   reaching disk before the data it points at. Without the second, the rename
//!   itself can be lost and the key reverts to its previous value.
//!
//! `rename` alone buys atomicity only. A store whose whole purpose is surviving
//! restarts has to pay for the `fsync`s too.
//!
//! The filesystem is blocking, so each op does its work when the returned future
//! is first polled and resolves immediately. That is fine for the small, infrequent
//! blobs this serves; offloading to a worker thread is a possible future
//! optimization, not needed now.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{StorageError, StorageFuture, StorageResult, Store};

/// The longest encoded key this backend accepts, in bytes.
///
/// Each key maps to one file name, and every mainstream filesystem caps a single
/// name at 255 bytes. Note this bounds the *encoded* length: a byte outside
/// `[A-Za-z0-9_-]` costs 3 bytes, so a worst-case key is ~85 characters.
///
/// This is a native-only limit — IndexedDB imposes none — so a key near the bound
/// is a portability hazard. Keys that must work on both targets should stay well
/// under it.
pub const MAX_ENCODED_KEY_LEN: usize = 255;

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
        // Rules shared with every other backend first, then the filename-length
        // bound that is specific to storing a key as a file.
        crate::validate_key(key)?;
        let name = encode_key(key);
        // A key becomes one file name, and file names are capped (255 bytes on
        // ext4/APFS/NTFS). Check it here so an over-long key is a clear, testable
        // `InvalidKey` instead of an `Io` error naming an internal temp path — the
        // caller otherwise cannot tell "key too long" from "disk full".
        if name.len() > MAX_ENCODED_KEY_LEN {
            return Err(StorageError::InvalidKey(format!(
                "key encodes to {} bytes, over the {MAX_ENCODED_KEY_LEN}-byte file-name limit \
                 (bytes outside [A-Za-z0-9_-] encode to 3 bytes each)",
                name.len()
            )));
        }
        Ok(self.base.join(name))
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
            // Write + flush the contents to disk *before* the rename publishes the
            // name. Otherwise a crash can order the rename ahead of the data and
            // leave a present-but-empty file — losing the old value and the new one.
            if let Err(e) = write_and_sync(&tmp, &value) {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
            // Atomic replace. On failure, don't leak the temp file.
            if let Err(e) = std::fs::rename(&tmp, &path) {
                let _ = std::fs::remove_file(&tmp);
                return Err(StorageError::Io(format!(
                    "rename {} -> {}: {e}",
                    tmp.display(),
                    path.display()
                )));
            }
            // Flush the directory entry so the rename itself survives a crash.
            sync_dir(&base)?;
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
                let entry = entry.map_err(|e| StorageError::Io(format!("dir entry: {e}")))?;
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

/// Write `value` to `path` and flush it all the way to the storage device.
///
/// `std::fs::write` only hands the bytes to the page cache; `sync_all` is what
/// makes them survive a crash.
fn write_and_sync(path: &std::path::Path, value: &[u8]) -> StorageResult<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)
        .map_err(|e| StorageError::Io(format!("create {}: {e}", path.display())))?;
    f.write_all(value)
        .map_err(|e| StorageError::Io(format!("write {}: {e}", path.display())))?;
    f.sync_all()
        .map_err(|e| StorageError::Io(format!("sync {}: {e}", path.display())))?;
    Ok(())
}

/// Flush a directory's entries so a `rename` into it is durable.
///
/// POSIX-only: a rename is a directory metadata change, and on Unix the way to
/// commit it is to `fsync` the directory itself.
#[cfg(unix)]
fn sync_dir(dir: &std::path::Path) -> StorageResult<()> {
    std::fs::File::open(dir)
        .and_then(|d| d.sync_all())
        .map_err(|e| StorageError::Io(format!("sync dir {}: {e}", dir.display())))
}

/// Non-Unix fallback.
///
/// Windows has no directory-`fsync` equivalent — a directory can't be opened as a
/// file, and `MoveFileEx`-style replacement is journaled by the filesystem rather
/// than flushed by the caller. The `sync_all` on the file itself (above) is the
/// part that carries over, and is done.
#[cfg(not(unix))]
fn sync_dir(_dir: &std::path::Path) -> StorageResult<()> {
    Ok(())
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
        assert_eq!(block_on(store.get("..")).unwrap(), Some(b"..".to_vec()));
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

    /// An over-long key is rejected as `InvalidKey`, not surfaced as an opaque `Io`
    /// error from the underlying `rename`.
    ///
    /// The bound is on the *encoded* name: a byte outside `[A-Za-z0-9_-]` costs 3
    /// bytes, so a key of `:` characters hits the limit at a third the length of an
    /// alphanumeric one. This is a native-only constraint (IndexedDB has none), so
    /// it is also the one place a key can behave differently across targets.
    #[test]
    fn over_long_keys_are_rejected_as_invalid_not_io() {
        let dir = tempdir();
        let store = FsStore::open(dir.path()).unwrap();

        // Just inside the bound, both encodings.
        block_on(store.put(&"a".repeat(MAX_ENCODED_KEY_LEN), b"v")).unwrap();
        block_on(store.put(&":".repeat(MAX_ENCODED_KEY_LEN / 3), b"v")).unwrap();

        // Just outside it: a clear InvalidKey rather than an Io from rename.
        for key in [
            "a".repeat(MAX_ENCODED_KEY_LEN + 1),
            ":".repeat(MAX_ENCODED_KEY_LEN / 3 + 1),
        ] {
            match block_on(store.put(&key, b"v")) {
                Err(StorageError::InvalidKey(_)) => {}
                other => panic!(
                    "expected InvalidKey for a {}-char key, got {other:?}",
                    key.len()
                ),
            }
            // get/delete/`path_for` agree, so the rejection is consistent per key.
            assert!(matches!(
                block_on(store.get(&key)),
                Err(StorageError::InvalidKey(_))
            ));
        }
    }

    /// Opaque binary blobs survive a write / drop / reopen cycle byte-identically.
    ///
    /// The payload is deliberately hostile for a *file-backed* store: every one of
    /// the 256 byte values (so embedded NULs, newlines, and invalid UTF-8), plus a
    /// leading UTF-8 BOM and a trailing byte — the shapes that get mangled by a
    /// backend that treats values as text, trims, or NUL-terminates.
    #[test]
    fn arbitrary_binary_blobs_round_trip_across_reopen() {
        let dir = tempdir();

        let mut payload = vec![0xEF, 0xBB, 0xBF]; // UTF-8 BOM
        payload.extend((0..=255u8).cycle().take(4096));
        payload.push(0x00); // trailing NUL

        {
            let store = FsStore::open(dir.path()).unwrap();
            block_on(store.put("blob", &payload)).unwrap();
        }

        // A fresh store over the same directory reads the identical bytes.
        let store = FsStore::open(dir.path()).unwrap();
        let restored = block_on(store.get("blob")).unwrap().expect("present");
        assert_eq!(restored, payload, "persisted bytes must be identical");
        assert_eq!(restored.len(), payload.len(), "no truncation at a NUL");

        // Empty values are a legitimate value, distinct from an absent key.
        block_on(store.put("empty", b"")).unwrap();
        assert_eq!(block_on(store.get("empty")).unwrap(), Some(Vec::new()));
        assert_eq!(block_on(store.get("never-written")).unwrap(), None);
    }

    /// Overwriting a key replaces it wholly — no leftover tail from a longer prior
    /// value, which is the failure mode of writing in place instead of atomically.
    #[test]
    fn overwrite_replaces_rather_than_patches() {
        let dir = tempdir();
        let store = FsStore::open(dir.path()).unwrap();

        block_on(store.put("k", b"a-long-original-value")).unwrap();
        block_on(store.put("k", b"short")).unwrap();
        assert_eq!(block_on(store.get("k")).unwrap(), Some(b"short".to_vec()));

        // And the temp files used to do it are not visible as keys.
        assert_eq!(block_on(store.list("")).unwrap(), vec!["k".to_string()]);
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
        assert_eq!(
            block_on(ch1.get("snapshot")).unwrap(),
            Some(b"one".to_vec())
        );
        assert_eq!(
            block_on(ch2.get("snapshot")).unwrap(),
            Some(b"two".to_vec())
        );

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
            vec![
                "chapter:1/snapshot".to_string(),
                "chapter:2/snapshot".to_string()
            ]
        );
    }

    /// Executable proof of the crate's answer to "there is no `batch()`": the
    /// manifest / pointer-flip recipe (module docs, "Atomicity: one key at a
    /// time") gives a multi-key invariant out of single-key atomicity alone.
    ///
    /// A "generation" is a set of blobs written under fresh keys; one small
    /// `manifest` key names the live generation, and a reader resolves everything
    /// *through* it. Because publishing is that one atomic `put`, the multi-blob
    /// state flips as a unit — and a crash before the flip leaves the previous
    /// generation whole, never a mixture. This is the guarantee a real consumer
    /// would rely on, so it is pinned here rather than left as prose.
    #[test]
    fn manifest_pointer_flip_gives_multi_key_atomicity() {
        // A reader NEVER touches a generation's keys directly — it follows the
        // manifest to the live generation and reads only under it. This is the
        // whole discipline the recipe asks of a consumer.
        fn read_live(store: &FsStore) -> Option<(Vec<u8>, Vec<u8>)> {
            let generation = block_on(store.get("manifest")).unwrap()?;
            let generation = String::from_utf8(generation).unwrap();
            let snapshot = block_on(store.get(&format!("{generation}/snapshot"))).unwrap()?;
            let log = block_on(store.get(&format!("{generation}/log"))).unwrap()?;
            Some((snapshot, log))
        }

        let dir = tempdir();
        let store = FsStore::open(dir.path()).unwrap();

        // Nothing is published until a manifest exists.
        assert_eq!(read_live(&store), None);

        // Generation 1: stage the blobs under fresh keys, THEN publish them with
        // the single atomic manifest put. Only this last write makes them live.
        block_on(store.put("gen1/snapshot", b"snap-1")).unwrap();
        block_on(store.put("gen1/log", b"log-1")).unwrap();
        block_on(store.put("manifest", b"gen1")).unwrap();
        assert_eq!(
            read_live(&store),
            Some((b"snap-1".to_vec(), b"log-1".to_vec()))
        );

        // Generation 2, INTERRUPTED: both new blobs reach disk, but the process
        // "crashes" before the manifest flip. A restart (a fresh store instance
        // over the same directory — FsStore keeps no in-memory state) must still
        // see the complete generation 1, not a snap-2/log-1 mix.
        block_on(store.put("gen2/snapshot", b"snap-2")).unwrap();
        block_on(store.put("gen2/log", b"log-2")).unwrap();
        // <-- crash here: `manifest` still names gen1.
        let reopened = FsStore::open(dir.path()).unwrap();
        assert_eq!(
            read_live(&reopened),
            Some((b"snap-1".to_vec(), b"log-1".to_vec())),
            "a crash before the manifest flip leaves the previous generation intact"
        );

        // Retry: the flip is one atomic put, so once it lands the entire new
        // generation is published at once.
        block_on(reopened.put("manifest", b"gen2")).unwrap();
        assert_eq!(
            read_live(&reopened),
            Some((b"snap-2".to_vec(), b"log-2".to_vec()))
        );

        // The superseded generation is now unreferenced garbage — no reader can
        // reach it — so it can be swept whenever convenient without affecting the
        // live state.
        for key in block_on(reopened.list("gen1/")).unwrap() {
            block_on(reopened.delete(&key)).unwrap();
        }
        assert_eq!(
            read_live(&reopened),
            Some((b"snap-2".to_vec(), b"log-2".to_vec()))
        );
    }
}
