//! Browser-driven tests for the IndexedDB backend.
//!
//! `IdbStore` cannot be exercised by a native `cargo test` — it needs a real
//! IndexedDB, which means a real browser. Run them with a chromedriver matching
//! the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-storage --target wasm32-unknown-unknown
//! ```
//!
//! Each test opens its own database so they stay independent regardless of
//! ordering or of state left behind by a previous run.
#![cfg(target_arch = "wasm32")]

use rinch_storage::{IdbStore, Namespace, StorageError, Store};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// A fresh database name per test, so tests never share state.
fn db(name: &str) -> String {
    format!("rinch-storage-test-{name}")
}

#[wasm_bindgen_test]
async fn put_get_delete_round_trip() {
    let store = IdbStore::open(&db("round-trip"), "blobs").await.unwrap();

    // Absent key reads as None rather than erroring.
    assert_eq!(store.get("missing").await.unwrap(), None);

    store.put("a", b"hello").await.unwrap();
    assert_eq!(store.get("a").await.unwrap(), Some(b"hello".to_vec()));

    // Overwrite replaces wholly — no tail left from the longer previous value.
    store.put("a", b"hi").await.unwrap();
    assert_eq!(store.get("a").await.unwrap(), Some(b"hi".to_vec()));

    store.delete("a").await.unwrap();
    assert_eq!(store.get("a").await.unwrap(), None);
    // Deleting an absent key is a no-op, not an error.
    store.delete("a").await.unwrap();
}

#[wasm_bindgen_test]
async fn arbitrary_binary_values_survive() {
    let store = IdbStore::open(&db("binary"), "blobs").await.unwrap();

    // The same hostile payload the native backend is tested with: every byte
    // value, a leading BOM, a trailing NUL.
    let mut payload = vec![0xEF, 0xBB, 0xBF];
    payload.extend((0..=255u8).cycle().take(4096));
    payload.push(0x00);

    store.put("blob", &payload).await.unwrap();
    let restored = store.get("blob").await.unwrap().expect("present");
    assert_eq!(restored, payload, "bytes must survive the Uint8Array hop");
    assert_eq!(restored.len(), payload.len(), "no truncation at a NUL");

    // Empty is a real value, distinct from absent.
    store.put("empty", b"").await.unwrap();
    assert_eq!(store.get("empty").await.unwrap(), Some(Vec::new()));
}

/// `list` must return exactly the keys under a prefix.
///
/// This is the test that covers the `IDBKeyRange` prefix scan: the range has to
/// select the same set the caller-side `starts_with` filter would, including for
/// keys that differ only after the prefix and for non-ASCII continuations.
#[wasm_bindgen_test]
async fn list_returns_exactly_the_prefix_matches() {
    let store = IdbStore::open(&db("list"), "blobs").await.unwrap();

    for k in [
        "e:1/state",
        "e:1/log/0001",
        "e:1/log/0002",
        "e:2/state",
        "e:10/state", // shares the "e:1" prefix textually — must be included
        "other",
        "e:1\u{00E9}",  // non-ASCII immediately after the prefix
        "e:1\u{1F600}", // astral (surrogate pair) after the prefix
    ] {
        store.put(k, b"v").await.unwrap();
    }

    let mut under_e1 = store.list("e:1").await.unwrap();
    under_e1.sort();
    let mut want = vec![
        "e:1\u{00E9}".to_string(),
        "e:1\u{1F600}".to_string(),
        "e:1/log/0001".to_string(),
        "e:1/log/0002".to_string(),
        "e:1/state".to_string(),
        "e:10/state".to_string(),
    ];
    want.sort();
    assert_eq!(under_e1, want, "prefix scan must not drop or add keys");

    // A more selective prefix.
    let mut logs = store.list("e:1/log/").await.unwrap();
    logs.sort();
    assert_eq!(logs, vec!["e:1/log/0001", "e:1/log/0002"]);

    // The empty prefix lists everything (the no-range path).
    assert_eq!(store.list("").await.unwrap().len(), 8);

    // A prefix matching nothing is empty, not an error.
    assert!(store.list("nope:").await.unwrap().is_empty());
}

/// The `Namespace` wrapper behaves the same over IndexedDB as over the
/// filesystem: it scopes writes and lists prefix-relative.
#[wasm_bindgen_test]
async fn namespace_scopes_keys() {
    let store = IdbStore::open(&db("namespace"), "blobs").await.unwrap();

    let ns = Namespace::new(store.clone(), "e:abc/");
    ns.put("state", b"s").await.unwrap();
    ns.put("log/1", b"l").await.unwrap();
    store.put("outside", b"o").await.unwrap();

    assert_eq!(ns.get("state").await.unwrap(), Some(b"s".to_vec()));
    // Not visible through the namespace...
    assert_eq!(ns.get("outside").await.unwrap(), None);
    // ...but written under the physical prefixed key.
    assert_eq!(store.get("e:abc/state").await.unwrap(), Some(b"s".to_vec()));

    let mut listed = ns.list("").await.unwrap();
    listed.sort();
    assert_eq!(listed, vec!["log/1", "state"], "list is prefix-relative");
}

/// An empty key is rejected the same way the native backend rejects it, so the
/// error contract does not depend on the target.
#[wasm_bindgen_test]
async fn empty_key_is_rejected() {
    let store = IdbStore::open(&db("empty-key"), "blobs").await.unwrap();
    match store.put("", b"v").await {
        Err(StorageError::InvalidKey(_)) => {}
        other => panic!("expected InvalidKey, got {other:?}"),
    }
}
