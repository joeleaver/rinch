//! Issue #1819: on Unix an `FsStore` keeps the user's data owner-only — every
//! file it writes is 0600 and every directory it creates 0700 — whatever the
//! process umask, so a session token kept in a store cannot be read by another
//! local user.
//!
//! The umask is process-wide, so this is one test in a binary of its own: it
//! sets a permissive umask (0) first, under which a file written by
//! `File::create` is 0666 and a directory made by `create_dir_all` 0777, so
//! nothing here can pass by agreeing with a restrictive default.
#![cfg(all(unix, not(target_os = "android")))]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use rinch_storage::{FsStore, Store};

fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    // Every `FsStore` future resolves on its first poll.
    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);
    let mut fut = std::pin::pin!(fut);
    match fut.as_mut().poll(&mut cx) {
        std::task::Poll::Ready(v) => v,
        std::task::Poll::Pending => panic!("an FsStore future did not resolve at once"),
    }
}

fn mode(p: &Path) -> u32 {
    std::fs::metadata(p).unwrap().permissions().mode() & 0o777
}

/// Every entry in `dir`, with its mode.
fn modes_in(dir: &Path) -> Vec<(String, u32)> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            (
                e.file_name().to_string_lossy().into_owned(),
                mode(&e.path()),
            )
        })
        .collect();
    v.sort();
    v
}

#[test]
fn a_store_writes_owner_only_files_and_directories_under_a_permissive_umask() {
    // SAFETY: `umask` has no preconditions.
    unsafe { libc::umask(0) };
    // Positive control: the umask took, so a plain create is world-writable.
    let tmp = tempfile::tempdir().unwrap();
    let probe = tmp.path().join("probe");
    std::fs::write(&probe, b"x").unwrap();
    assert_eq!(mode(&probe), 0o666, "umask 0 is in effect");

    // Directories the store creates, the missing parents included.
    let base = tmp.path().join("app").join("store");
    let store = FsStore::open(&base).unwrap();
    assert_eq!(mode(&tmp.path().join("app")), 0o700, "a created parent");
    assert_eq!(mode(&base), 0o700, "the store's directory");

    // A new key, an overwrite, and a key whose file was already there 0666
    // (written before this change, or by hand): each is 0600 after a put.
    block_on(store.put("session-token", b"secret")).unwrap();
    block_on(store.put("panel/height", b"240")).unwrap();
    block_on(store.put("panel/height", b"260")).unwrap();
    std::fs::write(base.join("recent"), b"old").unwrap();
    assert_eq!(mode(&base.join("recent")), 0o666);
    block_on(store.put("recent", b"new")).unwrap();
    let entries = modes_in(&base);
    assert_eq!(
        entries,
        vec![
            ("panel%2Fheight".to_string(), 0o600),
            ("recent".to_string(), 0o600),
            ("session-token".to_string(), 0o600),
        ],
        "every file is 0600 and no temp file is left"
    );
    assert_eq!(
        block_on(store.get("session-token")).unwrap(),
        Some(b"secret".to_vec())
    );

    // A directory that already exists is the caller's: `FsStore::open` leaves
    // its mode alone, and still writes its files owner-only.
    let shared = tmp.path().join("shared");
    std::fs::create_dir(&shared).unwrap();
    assert_eq!(mode(&shared), 0o777);
    let s2 = FsStore::open(&shared).unwrap();
    assert_eq!(mode(&shared), 0o777, "an existing directory keeps its mode");
    block_on(s2.put("k", b"v")).unwrap();
    assert_eq!(mode(&shared.join("k")), 0o600);

    // `open_app_store`'s directory is rinch's own: a fresh one is 0700 and so
    // is a missing app directory, and one left world-readable from before is
    // made 0700, so its old files are out of other users' reach too. Linux
    // only: `dirs` reads `$XDG_DATA_HOME` there.
    #[cfg(target_os = "linux")]
    {
        let data = tmp.path().join("data");
        std::fs::create_dir(&data).unwrap();
        // SAFETY: this binary runs one test, on this thread, and nothing else
        // reads the environment while it is written.
        unsafe { std::env::set_var("XDG_DATA_HOME", &data) };
        let fresh = block_on(rinch_storage::open_app_store("com.example.a", "s")).unwrap();
        block_on(fresh.put("t", b"x")).unwrap();
        assert_eq!(mode(&data.join("com.example.a")), 0o700);
        assert_eq!(mode(&data.join("com.example.a").join("s")), 0o700);
        assert_eq!(mode(&data.join("com.example.a").join("s").join("t")), 0o600);

        let old = data.join("com.example.b").join("s");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("t"), b"x").unwrap();
        assert_eq!(mode(&old), 0o777);
        let _store = block_on(rinch_storage::open_app_store("com.example.b", "s")).unwrap();
        assert_eq!(
            mode(&old),
            0o700,
            "an existing store directory is tightened"
        );
        assert_eq!(
            mode(&data.join("com.example.b")),
            0o777,
            "the app's own directory is left as it is"
        );
    }
}
