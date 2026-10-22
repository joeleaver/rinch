//! Opening a store the same way on every target (issue #1773).
//!
//! [`FsStore::open`](crate::FsStore) takes a directory and returns at once;
//! [`IdbStore::open`](crate::IdbStore) takes a database name and must be
//! awaited. An app built for both wrote a `cfg` split to open one. This names
//! a store by the app and a store name and picks the place on each target.

use crate::{StorageError, StorageResult};

/// The [`Store`](crate::Store) backend of the target being built:
/// [`FsStore`](crate::FsStore) natively, [`IdbStore`](crate::IdbStore) in the
/// browser. What [`open_app_store`] returns.
#[cfg(not(target_arch = "wasm32"))]
pub type PlatformStore = crate::FsStore;
/// The [`Store`](crate::Store) backend of the target being built:
/// [`FsStore`](crate::FsStore) natively, [`IdbStore`](crate::IdbStore) in the
/// browser. What [`open_app_store`] returns.
#[cfg(target_arch = "wasm32")]
pub type PlatformStore = crate::IdbStore;

/// Open (creating if needed) the store `store_name` of the app `app_id`, in the
/// place the platform keeps an app's data.
///
/// - **Native:** the directory `<data dir>/<app_id>/<store_name>`, where the
///   data directory is the user's (`$XDG_DATA_HOME` or `~/.local/share` on
///   Linux, `~/Library/Application Support` on macOS, `%APPDATA%` on Windows).
///   [`StorageError::Unavailable`] where the platform names none. On Android
///   open an [`FsStore`](crate::FsStore) under the app's files directory
///   instead.
/// - **Web:** the IndexedDB database `"<app_id>/<store_name>"` holding one
///   object store `store_name`. One database per store, so a second store of
///   the same app is a database of its own and is created like the first.
///
/// Both names must be non-empty and contain no `/`, `\` or NUL, and neither may
/// be `.` or `..` — checked the same way on both targets, so a name that works
/// on one works on the other. Pick an `app_id` unique to the app (a reverse-DNS
/// name is a good choice): it is shared with every other app of the user.
///
/// ```ignore
/// use rinch_storage::{open_app_store, Store};
/// # async fn go() -> rinch_storage::StorageResult<()> {
/// let store = open_app_store("com.example.notes", "drafts").await?;
/// store.put("current", b"hello").await?;
/// # Ok(()) }
/// ```
pub async fn open_app_store(app_id: &str, store_name: &str) -> StorageResult<PlatformStore> {
    validate_name("app_id", app_id)?;
    validate_name("store_name", store_name)?;

    #[cfg(not(target_arch = "wasm32"))]
    {
        let data = dirs::data_dir().ok_or_else(|| {
            StorageError::Unavailable("the platform names no user data directory".to_string())
        })?;
        crate::FsStore::open(app_store_dir(&data, app_id, store_name))
    }

    #[cfg(target_arch = "wasm32")]
    {
        crate::IdbStore::open(&format!("{app_id}/{store_name}"), store_name).await
    }
}

/// Where a native app store lives under the data directory `data`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn app_store_dir(
    data: &std::path::Path,
    app_id: &str,
    store_name: &str,
) -> std::path::PathBuf {
    data.join(app_id).join(store_name)
}

fn validate_name(what: &str, name: &str) -> StorageResult<()> {
    let bad = name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']);
    if bad {
        return Err(StorageError::InvalidKey(format!(
            "{what} {name:?} must be non-empty, not `.` or `..`, and contain no `/`, `\\` or NUL"
        )));
    }
    Ok(())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::Store;

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        // The native futures never pend: one poll finishes them.
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        let mut f = std::pin::pin!(f);
        match f.as_mut().poll(&mut cx) {
            std::task::Poll::Ready(v) => v,
            std::task::Poll::Pending => panic!("a native storage future pended"),
        }
    }

    #[test]
    fn bad_names_are_refused_before_anything_is_created() {
        // Checked through `validate_name`: an `open_app_store` call that
        // accepted one would create it in the user's real data directory.
        for bad in [
            "",
            ".",
            "..",
            "a/b",
            "a\\b",
            "a\0b",
            // Windows: `C:` joined onto the data dir replaces it, `a:x` names
            // an NTFS stream; the rest are refused by NTFS (#1773 review).
            "C:",
            "app:x",
            "a*b",
            "a?b",
            "a\"b",
            "a<b",
            "a>b",
            "a|b",
            "a\tb",
            "trailing.",
            "trailing ",
            "CON",
            "con",
            "nul.txt",
            "COM1",
            "lpt9",
        ] {
            assert!(
                matches!(
                    validate_name("app_id", bad),
                    Err(StorageError::InvalidKey(_))
                ),
                "{bad:?} is refused"
            );
        }
        // And `open_app_store` checks both names before touching the disk.
        for (app, store) in [("", "s"), ("app", "")] {
            let err = block_on(open_app_store(app, store)).err();
            assert!(matches!(err, Some(StorageError::InvalidKey(_))), "{err:?}");
        }
    }

    #[test]
    fn names_windows_reads_differently_are_still_accepted_where_harmless() {
        for good in ["com.example.app", "CONSOLE", "com1x", "a.b", "my app"] {
            assert!(
                validate_name("app_id", good).is_ok(),
                "{good:?} is accepted"
            );
        }
    }

    #[test]
    fn a_native_app_store_is_a_directory_per_app_and_store() {
        let tmp = tempfile::tempdir().unwrap();
        let a = crate::FsStore::open(app_store_dir(tmp.path(), "com.example.app", "one")).unwrap();
        let b = crate::FsStore::open(app_store_dir(tmp.path(), "com.example.app", "two")).unwrap();
        block_on(a.put("k", b"first")).unwrap();
        block_on(b.put("k", b"second")).unwrap();
        assert_eq!(block_on(a.get("k")).unwrap(), Some(b"first".to_vec()));
        assert_eq!(block_on(b.get("k")).unwrap(), Some(b"second".to_vec()));
        assert!(tmp.path().join("com.example.app").join("one").is_dir());
    }
}
