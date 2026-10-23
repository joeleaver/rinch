//! Files the user picked or dropped, read on demand (issue #1771).
//!
//! A [`LocalFile`] is what a file picker or an OS file drop hands the app on
//! every platform: a name, a size, the path where the platform has one (the
//! desktop), and a way to read the bytes. Nothing is read until the app asks —
//! [`LocalFile::bytes`] to await them, [`LocalFile::read`] for a callback — so a
//! drop of a large file costs nothing until it is wanted, and
//! [`size`](LocalFile::size) can decide whether it is wanted at all.
//!
//! Where the bytes come from is a [`FileSource`]: a path read on a worker
//! thread (native), a browser `File` read with a `FileReader` (`rinch`'s web
//! half), or bytes already in memory ([`LocalFile::from_bytes`]).

use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::rc::Rc;

/// The future a [`FileSource`] answers a read with.
pub type ReadFuture = Pin<Box<dyn Future<Output = io::Result<Vec<u8>>>>>;

/// Where a [`LocalFile`]'s bytes come from.
///
/// Implemented by the platforms; an app needs it only to hand rinch a file of
/// its own kind. `read` is called on the UI thread and returns a future that
/// is polled there (it need not be `Send`); it should start reading when first
/// polled, not when `read` is called.
pub trait FileSource {
    /// Read the whole file.
    fn read(&self) -> ReadFuture;
}

/// A file the user picked or dropped: its name, size and (on the desktop)
/// path, read on demand.
///
/// Cheap to clone; clones share the source. Read it with
/// [`bytes`](Self::bytes) (`.await` it inside
/// [`spawn_local`](crate::task::spawn_local)) or [`read`](Self::read)
/// (a callback). Each read reads the file again.
#[derive(Clone)]
pub struct LocalFile {
    name: String,
    size: u64,
    path: Option<PathBuf>,
    source: Rc<dyn FileSource>,
}

impl std::fmt::Debug for LocalFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalFile")
            .field("name", &self.name)
            .field("size", &self.size)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl LocalFile {
    /// A file read from `source`. `size` is what the platform reports before
    /// any read; `path` is where it lives, when there is one.
    pub fn from_source(
        name: impl Into<String>,
        size: u64,
        path: Option<PathBuf>,
        source: Rc<dyn FileSource>,
    ) -> Self {
        Self {
            name: name.into(),
            size,
            path,
            source,
        }
    }

    /// A file whose bytes are already in memory. It has no path.
    pub fn from_bytes(name: impl Into<String>, bytes: Vec<u8>) -> Self {
        let size = bytes.len() as u64;
        Self::from_source(name, size, None, Rc::new(InMemory(Rc::from(bytes))))
    }

    /// The file at `path`, read on a worker thread when asked. Takes one
    /// `stat` (for the size); fails when that does. A directory, a FIFO or a
    /// device is accepted and fails when read (only a regular file is read),
    /// as a dropped folder does in a browser. A read is of the whole file.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_path(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let size = std::fs::metadata(&path)?.len();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        Ok(Self::from_source(
            name,
            size,
            Some(path.clone()),
            Rc::new(OnDisk(path)),
        ))
    }

    /// The file's name, without any directory (`"model.glb"`).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The file's size in bytes, as reported before reading.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Where the file lives: `Some` on the desktop, `None` in a browser (which
    /// does not tell a page) and for a file made in memory.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Read the whole file. The read starts when the future is first polled;
    /// dropping the future drops its result.
    ///
    /// ```ignore
    /// spawn_local(async move {
    ///     match file.bytes().await {
    ///         Ok(bytes) => load_model(file.name(), &bytes),
    ///         Err(e) => status.set(format!("{}: {e}", file.name())),
    ///     }
    /// });
    /// ```
    pub fn bytes(&self) -> impl Future<Output = io::Result<Vec<u8>>> + 'static {
        let source = self.source.clone();
        async move { source.read().await }
    }

    /// Read the whole file and hand the result to `done`, later, on the UI
    /// thread, in one batch. A [`spawn_local`](crate::task::spawn_local) of
    /// [`bytes`](Self::bytes): called during a render, `done` is dropped unrun
    /// if that component unmounts first.
    pub fn read(&self, done: impl FnOnce(io::Result<Vec<u8>>) + 'static) {
        let bytes = self.bytes();
        crate::task::spawn_local(async move { done(bytes.await) });
    }
}

/// Bytes held in memory.
struct InMemory(Rc<[u8]>);

impl FileSource for InMemory {
    fn read(&self) -> ReadFuture {
        let bytes = self.0.clone();
        Box::pin(async move { Ok(bytes.to_vec()) })
    }
}

/// A file on disk, read on a spawned thread so a large file or a slow mount
/// does not hold the UI thread.
#[cfg(not(target_arch = "wasm32"))]
struct OnDisk(PathBuf);

#[cfg(not(target_arch = "wasm32"))]
impl FileSource for OnDisk {
    fn read(&self) -> ReadFuture {
        use std::sync::{Arc, Mutex};
        use std::task::{Poll, Waker};

        type Shared = Arc<Mutex<(Option<io::Result<Vec<u8>>>, Option<Waker>)>>;
        let path = self.0.clone();
        let mut started: Option<Shared> = None;
        Box::pin(std::future::poll_fn(move |cx| {
            let shared = started.get_or_insert_with(|| {
                let shared: Shared = Arc::default();
                let filled = shared.clone();
                let path = path.clone();
                std::thread::spawn(move || {
                    // A regular file only: a FIFO or a device (dropped from a
                    // file manager, picked from /dev) would block or never end.
                    let result = match std::fs::metadata(&path) {
                        Ok(meta) if meta.is_file() => std::fs::read(&path),
                        Ok(_) => Err(io::Error::other(format!(
                            "{} is not a regular file",
                            path.display()
                        ))),
                        Err(e) => Err(e),
                    };
                    let waker = {
                        let mut slot = filled.lock().unwrap_or_else(|e| e.into_inner());
                        slot.0 = Some(result);
                        slot.1.take()
                    };
                    if let Some(waker) = waker {
                        waker.wake();
                    }
                });
                shared
            });
            let mut slot = shared.lock().unwrap_or_else(|e| e.into_inner());
            match slot.0.take() {
                Some(result) => Poll::Ready(result),
                None => {
                    slot.1 = Some(cx.waker().clone());
                    Poll::Pending
                }
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block_on<F: Future>(fut: F) -> F::Output {
        use std::sync::Arc;
        use std::task::{Context, Poll, Wake};
        struct Thread(std::thread::Thread);
        impl Wake for Thread {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = Arc::new(Thread(std::thread::current())).into();
        let mut cx = Context::from_waker(&waker);
        let mut fut = std::pin::pin!(fut);
        loop {
            if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
                return v;
            }
            std::thread::park_timeout(std::time::Duration::from_millis(50));
        }
    }

    /// Poll `fut` until it is ready or `limit` has passed.
    #[cfg(unix)]
    fn poll_for<F: Future>(fut: F, limit: std::time::Duration) -> Option<F::Output> {
        use std::sync::Arc;
        use std::task::{Context, Poll, Wake};
        struct Thread(std::thread::Thread);
        impl Wake for Thread {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = Arc::new(Thread(std::thread::current())).into();
        let mut cx = Context::from_waker(&waker);
        let mut fut = std::pin::pin!(fut);
        let start = std::time::Instant::now();
        while start.elapsed() < limit {
            if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
                return Some(v);
            }
            std::thread::park_timeout(std::time::Duration::from_millis(50));
        }
        None
    }

    #[test]
    fn a_file_in_memory_reads_its_bytes() {
        let file = LocalFile::from_bytes("a.bin", vec![1, 2, 3]);
        assert_eq!(file.name(), "a.bin");
        assert_eq!(file.size(), 3);
        assert_eq!(file.path(), None);
        assert_eq!(block_on(file.bytes()).unwrap(), vec![1, 2, 3]);
    }

    /// A FIFO is not read (the read would block until a writer came): the
    /// read fails at once. Unix only; skipped where `mkfifo` is missing.
    #[cfg(unix)]
    #[test]
    fn a_fifo_is_refused_not_read() {
        let path = std::env::temp_dir().join(format!("rinch-fifo-1771-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let Ok(status) = std::process::Command::new("mkfifo").arg(&path).status() else {
            return;
        };
        assert!(status.success());
        let file = LocalFile::from_path(&path).unwrap();
        // Polled here with a deadline: without the check the read would
        // block its worker thread for ever (no writer ever comes).
        let refused = poll_for(file.bytes(), std::time::Duration::from_secs(5)).map(|r| r.is_err());
        let _ = std::fs::remove_file(&path);
        assert_eq!(
            refused,
            Some(true),
            "a FIFO read must fail at once, not block"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_file_on_disk_is_stat_ed_not_read_until_asked() {
        let dir = std::env::temp_dir().join(format!("rinch-file-1771-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("model.glb");
        std::fs::write(&path, b"glTF0123456").unwrap();
        let file = LocalFile::from_path(&path).unwrap();
        assert_eq!(file.name(), "model.glb");
        assert_eq!(file.size(), 11);
        assert_eq!(file.path(), Some(path.as_path()));
        // Not read at construction: the read sees what is there when it runs.
        std::fs::write(&path, b"later").unwrap();
        assert_eq!(block_on(file.bytes()).unwrap(), b"later");
        std::fs::remove_file(&path).unwrap();
        assert!(block_on(file.bytes()).is_err());
        assert!(LocalFile::from_path(&path).is_err());
        // A folder is a file whose read fails.
        let folder = LocalFile::from_path(&dir).unwrap();
        assert!(block_on(folder.bytes()).is_err());
        std::fs::remove_dir(&dir).unwrap();
    }
}
