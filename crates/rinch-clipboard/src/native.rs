//! Native clipboard implementation: one worker thread owning one `arboard::Clipboard`.
//!
//! # Why a worker thread (issue #149)
//!
//! A clipboard *read* is a request to whichever process owns the selection. On
//! X11 arboard waits up to 4 seconds for an answer, so a read issued from an
//! event handler froze the whole UI for that long — and the built-in editor
//! paste chained three of them, stacking the worst case.
//!
//! The fix is not to make the read faster (we don't own the other process) but
//! to take it off the UI thread and let a caller stop waiting. Both need the
//! clipboard to live somewhere a caller can *talk to* rather than *hold*:
//!
//! - Every operation is a [`Job`] — a closure posted to the worker, run there
//!   with exclusive access to the backend.
//! - The blocking API posts a job and waits on a reply channel. Dropping that
//!   wait (a timeout) does **not** cancel the job: it still runs to completion on
//!   the worker and its reply is discarded, so the requests queued behind it move
//!   as soon as it finishes. Under the old global `Mutex<Clipboard>` an
//!   abandoning caller could not exist at all — the lock was held for the whole
//!   stall.
//! - The async API posts a job that calls the user's callback on the worker.
//!
//! # Backend seam
//!
//! The worker talks to a [`Backend`], not to arboard directly, so the queueing,
//! timeout and probe logic is testable with no X11/Wayland display — which CI
//! does not have. [`ArboardBackend`] is the only production implementation.

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::{ClipboardError, ClipboardResult, ImageData, RichPaste};

impl From<arboard::Error> for ClipboardError {
    fn from(err: arboard::Error) -> Self {
        match err {
            arboard::Error::ContentNotAvailable => ClipboardError::ContentTypeMismatch,
            other => ClipboardError::AccessFailed(other.to_string()),
        }
    }
}

// ── The backend seam ─────────────────────────────────────────────────────────

/// The system clipboard, as the worker sees it.
///
/// `&mut self` throughout: the worker owns exactly one backend and runs one job
/// at a time, which is also what arboard wants (`Clipboard` is `Send`, not
/// `Sync`).
pub(crate) trait Backend: Send {
    fn get_text(&mut self) -> ClipboardResult<String>;
    fn set_text(&mut self, text: &str) -> ClipboardResult<()>;
    fn get_html(&mut self) -> ClipboardResult<String>;
    fn set_html(&mut self, html: &str, alt_text: Option<&str>) -> ClipboardResult<()>;
    fn get_image(&mut self) -> ClipboardResult<ImageData<'static>>;
    fn set_image(&mut self, image: ImageData<'_>) -> ClipboardResult<()>;
    fn clear(&mut self) -> ClipboardResult<()>;

    /// Whether this is the stand-in for a backend that could not be created, in
    /// which case the worker throws it away and retries the real one next job.
    fn is_stand_in(&self) -> bool {
        false
    }
}

struct ArboardBackend(arboard::Clipboard);

impl Backend for ArboardBackend {
    fn get_text(&mut self) -> ClipboardResult<String> {
        Ok(self.0.get_text()?)
    }
    fn set_text(&mut self, text: &str) -> ClipboardResult<()> {
        self.0.set_text(text)?;
        Ok(())
    }
    fn get_html(&mut self) -> ClipboardResult<String> {
        Ok(self.0.get().html()?)
    }
    fn set_html(&mut self, html: &str, alt_text: Option<&str>) -> ClipboardResult<()> {
        self.0.set_html(html, alt_text)?;
        Ok(())
    }
    fn get_image(&mut self) -> ClipboardResult<ImageData<'static>> {
        let image = self.0.get_image()?;
        Ok(ImageData {
            width: image.width,
            height: image.height,
            bytes: image.bytes,
        })
    }
    fn set_image(&mut self, image: ImageData<'_>) -> ClipboardResult<()> {
        self.0.set_image(arboard::ImageData {
            width: image.width,
            height: image.height,
            bytes: image.bytes,
        })?;
        Ok(())
    }
    fn clear(&mut self) -> ClipboardResult<()> {
        self.0.clear()?;
        Ok(())
    }
}

/// The stand-in used when `Clipboard::new()` failed (no display, no compositor).
///
/// Every job still gets an answer — the creation error — instead of the worker
/// dying and taking the queue with it. The worker retries the real backend on
/// the next job, so a clipboard that becomes available later starts working
/// without a restart.
struct UnavailableBackend(String);

impl UnavailableBackend {
    fn err<T>(&self) -> ClipboardResult<T> {
        Err(ClipboardError::AccessFailed(self.0.clone()))
    }
}

impl Backend for UnavailableBackend {
    fn get_text(&mut self) -> ClipboardResult<String> {
        self.err()
    }
    fn set_text(&mut self, _text: &str) -> ClipboardResult<()> {
        self.err()
    }
    fn get_html(&mut self) -> ClipboardResult<String> {
        self.err()
    }
    fn set_html(&mut self, _html: &str, _alt_text: Option<&str>) -> ClipboardResult<()> {
        self.err()
    }
    fn get_image(&mut self) -> ClipboardResult<ImageData<'static>> {
        self.err()
    }
    fn set_image(&mut self, _image: ImageData<'_>) -> ClipboardResult<()> {
        self.err()
    }
    fn clear(&mut self) -> ClipboardResult<()> {
        self.err()
    }
    fn is_stand_in(&self) -> bool {
        true
    }
}

// ── The worker ───────────────────────────────────────────────────────────────

/// One unit of clipboard work, run on the worker thread with the backend.
///
/// A closure rather than a request enum: every operation (blocking, timed,
/// async, and the combined rich probe) is "do this with the clipboard, then
/// deliver the answer yourself", and the worker never needs to know which.
pub(crate) type Job = Box<dyn FnOnce(&mut dyn Backend) + Send>;

/// A handle on a running clipboard worker.
pub(crate) struct Worker {
    tx: Sender<Job>,
}

impl Worker {
    /// Spawn a worker that builds its backend with `make`, lazily and retried
    /// per job until it succeeds.
    pub(crate) fn spawn(
        mut make: impl FnMut() -> ClipboardResult<Box<dyn Backend>> + Send + 'static,
    ) -> Worker {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("rinch-clipboard".into())
            .spawn(move || {
                ON_WORKER.with(|f| f.set(true));
                let mut backend: Option<Box<dyn Backend>> = None;
                // Ends when the last `Sender` is dropped, i.e. at process exit or
                // when a dead worker is replaced.
                while let Ok(job) = rx.recv() {
                    if backend.is_none() {
                        backend = Some(match make() {
                            Ok(b) => b,
                            Err(e) => Box::new(UnavailableBackend(e.to_string())),
                        });
                    }
                    // Retry the real backend next time if this one is the
                    // stand-in for a failed creation.
                    let mut b = backend.take().expect("backend created above");
                    job(b.as_mut());
                    if !b.is_stand_in() {
                        backend = Some(b);
                    }
                }
            })
            .expect("failed to spawn the clipboard worker thread");
        Worker { tx }
    }

    /// Post `job`. On failure the job is handed **back**, so the caller can
    /// re-post it to a replacement worker instead of losing it.
    fn submit(&self, job: Job) -> Result<(), Job> {
        self.tx.send(job).map_err(|e| e.0)
    }
}

thread_local! {
    /// Set on the clipboard worker thread. A *blocking* call made from a job
    /// would wait on the very thread running it, so it is reported instead.
    static ON_WORKER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The process-wide worker, spawned on first use.
///
/// `Mutex<Option<..>>` rather than a `OnceLock`: if the worker thread ever dies
/// (an arboard panic), the next call clears the slot and spawns a fresh one
/// instead of leaving the clipboard permanently dead.
static WORKER: Mutex<Option<Worker>> = Mutex::new(None);

/// Post `job` to the shared worker, spawning it if needed.
///
/// Returns whether it was accepted. `false` means the worker was dead *and* a
/// freshly spawned replacement would not take it either — nothing will run the
/// job, so the caller must answer its own request.
fn submit(job: Job) -> bool {
    fn spawn_arboard_worker() -> Worker {
        Worker::spawn(|| {
            Ok(Box::new(ArboardBackend(arboard::Clipboard::new()?)) as Box<dyn Backend>)
        })
    }

    let mut guard = WORKER.lock().unwrap_or_else(|e| e.into_inner());
    let worker = guard.get_or_insert_with(spawn_arboard_worker);
    match worker.submit(job) {
        Ok(()) => true,
        // The worker died (an arboard panic). Replace it and re-post the job we
        // got back, so one dead thread costs one clipboard operation, not all of
        // them for the rest of the process's life.
        Err(job) => guard.insert(spawn_arboard_worker()).submit(job).is_ok(),
    }
}

fn worker_gone<T>() -> ClipboardResult<T> {
    Err(ClipboardError::AccessFailed(
        "clipboard worker unavailable".into(),
    ))
}

/// Run `op` on the worker and wait for its answer, at most `timeout`.
///
/// On timeout the job is **not** cancelled — it runs to completion on the worker
/// and its reply is dropped — so it never blocks the requests behind it.
fn run_blocking<T: Send + 'static>(
    op: impl FnOnce(&mut dyn Backend) -> ClipboardResult<T> + Send + 'static,
    timeout: Option<Duration>,
) -> ClipboardResult<T> {
    if ON_WORKER.with(|f| f.get()) {
        return Err(ClipboardError::AccessFailed(
            "a blocking clipboard call was made from inside a clipboard callback, \
             which would wait on the worker running it; use the *_async variants"
                .into(),
        ));
    }
    let (tx, rx) = mpsc::channel();
    if !submit(Box::new(move |backend| {
        // The receiver is gone after a timeout; the send failing is the normal,
        // intended outcome there.
        let _ = tx.send(op(backend));
    })) {
        return worker_gone();
    }
    match timeout {
        None => rx.recv().unwrap_or_else(|_| worker_gone()),
        Some(d) => match rx.recv_timeout(d) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => Err(ClipboardError::TimedOut),
            Err(RecvTimeoutError::Disconnected) => worker_gone(),
        },
    }
}

/// Post `op` to the worker and hand its result to `on_done` **on the worker
/// thread**. Never blocks the caller.
fn run_async<T: Send + 'static>(
    op: impl FnOnce(&mut dyn Backend) -> ClipboardResult<T> + Send + 'static,
    on_done: impl FnOnce(ClipboardResult<T>) + Send + 'static,
) {
    // The callback lives in a slot both paths can claim, so it is invoked
    // exactly once whether the job ran or could not be posted at all. A dropped
    // callback would leave an async consumer waiting forever with nothing to
    // observe — the one outcome worse than an error.
    type Slot<T> = Arc<Mutex<Option<Box<dyn FnOnce(ClipboardResult<T>) + Send>>>>;
    let slot: Slot<T> = Arc::new(Mutex::new(Some(Box::new(on_done))));
    let claimed = slot.clone();
    let posted = submit(Box::new(move |backend| {
        let result = op(backend);
        let cb = claimed.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(cb) = cb {
            cb(result);
        }
    }));
    if !posted {
        let cb = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(cb) = cb {
            cb(worker_gone());
        }
    }
}

/// The `text/html` → bitmap → `text/plain` probe, as **one** worker job.
///
/// Chaining `paste_html`, then `has_image`/`paste_image`, then `paste_text` from
/// the caller costs one queue round-trip *and one worst-case stall* apiece; done
/// here they share a single job.
fn rich_probe(backend: &mut dyn Backend) -> ClipboardResult<RichPaste> {
    if let Ok(html) = backend.get_html() {
        if !html.trim().is_empty() {
            return Ok(RichPaste::Html(html));
        }
    }
    if let Ok(image) = backend.get_image() {
        return Ok(RichPaste::Image(image));
    }
    match backend.get_text() {
        Ok(text) if !text.is_empty() => Ok(RichPaste::Text(text)),
        Ok(_) => Err(ClipboardError::ContentTypeMismatch),
        Err(e) => Err(e),
    }
}

// ── Blocking API (unchanged signatures) ──────────────────────────────────────

/// Copy text to the clipboard.
pub fn copy_text(text: impl AsRef<str>) -> ClipboardResult<()> {
    let text = text.as_ref().to_string();
    run_blocking(move |b| b.set_text(&text), None)
}

/// Copy text to the clipboard **without waiting** for the write to happen.
///
/// The write is queued on the clipboard worker, so a copy issued from an event
/// handler never blocks the UI thread — not even behind an in-flight paste that
/// is stalled on a hung selection owner. Errors are dropped; use [`copy_text`]
/// when the result matters.
pub fn copy_text_async(text: impl AsRef<str>) {
    let text = text.as_ref().to_string();
    // A write has no result to deliver, so a failed post is simply a copy that
    // did not happen — exactly what `let _ = copy_text(..)` already meant.
    submit(Box::new(move |b| {
        let _ = b.set_text(&text);
    }));
}

/// Paste text from the clipboard.
///
/// Blocks until the clipboard owner answers — potentially seconds. Prefer
/// [`paste_text_timeout`] or [`paste_text_async`] on an interactive path.
pub fn paste_text() -> ClipboardResult<String> {
    run_blocking(|b| b.get_text(), None)
}

/// [`paste_text`], giving up after `timeout` with [`ClipboardError::TimedOut`].
pub fn paste_text_timeout(timeout: Duration) -> ClipboardResult<String> {
    run_blocking(|b| b.get_text(), Some(timeout))
}

/// [`paste_text`] without blocking: `on_done` is called with the result **on the
/// clipboard worker thread** (see the module docs for hopping back to the UI).
pub fn paste_text_async(on_done: impl FnOnce(ClipboardResult<String>) + Send + 'static) {
    run_async(|b| b.get_text(), on_done);
}

/// Check if the clipboard contains text.
pub fn has_text() -> bool {
    paste_text().is_ok()
}

/// Clear the clipboard contents.
pub fn clear() -> ClipboardResult<()> {
    run_blocking(|b| b.clear(), None)
}

/// Copy an image to the clipboard.
///
/// The image data should be in RGBA format.
pub fn copy_image(image: ImageData) -> ClipboardResult<()> {
    let image = image.into_owned();
    run_blocking(move |b| b.set_image(image), None)
}

/// Paste an image from the clipboard.
///
/// Returns the image data in RGBA format.
pub fn paste_image() -> ClipboardResult<ImageData<'static>> {
    run_blocking(|b| b.get_image(), None)
}

/// [`paste_image`], giving up after `timeout` with [`ClipboardError::TimedOut`].
pub fn paste_image_timeout(timeout: Duration) -> ClipboardResult<ImageData<'static>> {
    run_blocking(|b| b.get_image(), Some(timeout))
}

/// [`paste_image`] without blocking; `on_done` runs on the clipboard worker thread.
pub fn paste_image_async(
    on_done: impl FnOnce(ClipboardResult<ImageData<'static>>) + Send + 'static,
) {
    run_async(|b| b.get_image(), on_done);
}

/// Check if the clipboard contains an image.
pub fn has_image() -> bool {
    paste_image().is_ok()
}

/// Copy HTML to the clipboard with a plain-text fallback.
///
/// Places both `text/html` and `text/plain` MIME types on the clipboard.
/// Applications that understand HTML will get the rich content, while
/// others fall back to the plain text.
pub fn copy_html(html: impl AsRef<str>, alt_text: Option<&str>) -> ClipboardResult<()> {
    let html = html.as_ref().to_string();
    let alt = alt_text.map(str::to_string);
    run_blocking(move |b| b.set_html(&html, alt.as_deref()), None)
}

/// [`copy_html`] without waiting for the write — see [`copy_text_async`].
pub fn copy_html_async(html: impl AsRef<str>, alt_text: Option<&str>) {
    let html = html.as_ref().to_string();
    let alt = alt_text.map(str::to_string);
    // Errors are dropped, as in `copy_text_async`.
    submit(Box::new(move |b| {
        let _ = b.set_html(&html, alt.as_deref());
    }));
}

/// Paste HTML from the clipboard.
///
/// Returns the HTML content if the clipboard contains `text/html`.
/// Returns `Err(ContentTypeMismatch)` if no HTML is available.
pub fn paste_html() -> ClipboardResult<String> {
    run_blocking(|b| b.get_html(), None)
}

/// [`paste_html`], giving up after `timeout` with [`ClipboardError::TimedOut`].
pub fn paste_html_timeout(timeout: Duration) -> ClipboardResult<String> {
    run_blocking(|b| b.get_html(), Some(timeout))
}

/// [`paste_html`] without blocking; `on_done` runs on the clipboard worker thread.
pub fn paste_html_async(on_done: impl FnOnce(ClipboardResult<String>) + Send + 'static) {
    run_async(|b| b.get_html(), on_done);
}

/// Check if the clipboard contains HTML.
pub fn has_html() -> bool {
    paste_html().is_ok()
}

/// The richest content on the clipboard — `text/html`, else a bitmap, else
/// `text/plain` — resolved in **one** trip to the clipboard.
pub fn paste_rich() -> ClipboardResult<RichPaste> {
    run_blocking(rich_probe, None)
}

/// [`paste_rich`], giving up after `timeout` with [`ClipboardError::TimedOut`].
pub fn paste_rich_timeout(timeout: Duration) -> ClipboardResult<RichPaste> {
    run_blocking(rich_probe, Some(timeout))
}

/// [`paste_rich`] without blocking; `on_done` runs on the clipboard worker thread.
///
/// This is the shape the built-in editor paste uses: one dispatch, one worker
/// pass, one completion — never three stacked worst-case stalls.
pub fn paste_rich_async(on_done: impl FnOnce(ClipboardResult<RichPaste>) + Send + 'static) {
    run_async(rich_probe, on_done);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};

    /// A clipboard that answers from memory, counts the calls it received, and can
    /// be told to stall — standing in for a hung X11 selection owner without
    /// needing a display.
    #[derive(Default)]
    struct FakeClipboard {
        text: Option<String>,
        html: Option<String>,
        image: Option<ImageData<'static>>,
        /// How long every *read* takes.
        read_delay: Duration,
        /// One counter per read kind: `[text, html, image]`.
        reads: Arc<[AtomicUsize; 3]>,
    }

    impl FakeClipboard {
        fn stall(&self) {
            if !self.read_delay.is_zero() {
                std::thread::sleep(self.read_delay);
            }
        }
    }

    impl Backend for FakeClipboard {
        fn get_text(&mut self) -> ClipboardResult<String> {
            self.reads[0].fetch_add(1, Ordering::SeqCst);
            self.stall();
            self.text.clone().ok_or(ClipboardError::ContentTypeMismatch)
        }
        fn set_text(&mut self, text: &str) -> ClipboardResult<()> {
            self.text = Some(text.to_string());
            Ok(())
        }
        fn get_html(&mut self) -> ClipboardResult<String> {
            self.reads[1].fetch_add(1, Ordering::SeqCst);
            self.stall();
            self.html.clone().ok_or(ClipboardError::ContentTypeMismatch)
        }
        fn set_html(&mut self, html: &str, alt_text: Option<&str>) -> ClipboardResult<()> {
            self.html = Some(html.to_string());
            if let Some(alt) = alt_text {
                self.text = Some(alt.to_string());
            }
            Ok(())
        }
        fn get_image(&mut self) -> ClipboardResult<ImageData<'static>> {
            self.reads[2].fetch_add(1, Ordering::SeqCst);
            self.stall();
            self.image
                .clone()
                .ok_or(ClipboardError::ContentTypeMismatch)
        }
        fn set_image(&mut self, image: ImageData<'_>) -> ClipboardResult<()> {
            self.image = Some(image.into_owned());
            Ok(())
        }
        fn clear(&mut self) -> ClipboardResult<()> {
            self.text = None;
            self.html = None;
            self.image = None;
            Ok(())
        }
    }

    /// A worker over a `FakeClipboard`, plus the counters to inspect it. The
    /// production `submit()` singleton is deliberately not used: these tests are
    /// about the queue, not about arboard.
    struct Harness {
        worker: Worker,
        reads: Arc<[AtomicUsize; 3]>,
        jobs: Arc<AtomicUsize>,
    }

    impl Harness {
        fn new(fill: impl FnOnce(&mut FakeClipboard) + Send + 'static, delay: Duration) -> Harness {
            let reads: Arc<[AtomicUsize; 3]> = Arc::new(Default::default());
            let jobs = Arc::new(AtomicUsize::new(0));
            let r = reads.clone();
            let mut fill = Some(fill);
            let worker = Worker::spawn(move || {
                let mut fake = FakeClipboard {
                    read_delay: delay,
                    reads: r.clone(),
                    ..Default::default()
                };
                (fill.take().expect("backend built once"))(&mut fake);
                Ok(Box::new(fake) as Box<dyn Backend>)
            });
            Harness {
                worker,
                reads,
                jobs,
            }
        }

        /// Post `op` and wait at most `timeout`, mirroring `run_blocking` but
        /// against this harness's worker instead of the process-wide one.
        fn blocking<T: Send + 'static>(
            &self,
            op: impl FnOnce(&mut dyn Backend) -> ClipboardResult<T> + Send + 'static,
            timeout: Option<Duration>,
        ) -> ClipboardResult<T> {
            let (tx, rx) = mpsc::channel();
            let jobs = self.jobs.clone();
            assert!(self
                .worker
                .submit(Box::new(move |b| {
                    jobs.fetch_add(1, Ordering::SeqCst);
                    let _ = tx.send(op(b));
                }))
                .is_ok());
            match timeout {
                None => rx.recv().unwrap_or_else(|_| worker_gone()),
                Some(d) => match rx.recv_timeout(d) {
                    Ok(r) => r,
                    Err(RecvTimeoutError::Timeout) => Err(ClipboardError::TimedOut),
                    Err(RecvTimeoutError::Disconnected) => worker_gone(),
                },
            }
        }

        fn read_counts(&self) -> (usize, usize, usize) {
            (
                self.reads[0].load(Ordering::SeqCst),
                self.reads[1].load(Ordering::SeqCst),
                self.reads[2].load(Ordering::SeqCst),
            )
        }
    }

    /// The headline property of the worker design (#149): a caller that gives up
    /// waiting does not take the clipboard down with it. The abandoned read
    /// finishes on the worker and the *next* caller is served normally.
    #[test]
    fn the_worker_survives_a_caller_that_timed_out() {
        let h = Harness::new(
            |c| c.text = Some("hello".into()),
            Duration::from_millis(250),
        );

        // Give up long before the (stalled) read can answer.
        let abandoned = h.blocking(|b| b.get_text(), Some(Duration::from_millis(10)));
        assert!(
            matches!(abandoned, Err(ClipboardError::TimedOut)),
            "a bounded read must report TimedOut, got {abandoned:?}"
        );

        // The worker is still alive and still answering.
        let after = h.blocking(|b| b.get_text(), None);
        assert_eq!(after.unwrap(), "hello");
        assert_eq!(
            h.read_counts().0,
            2,
            "the abandoned read still ran to completion on the worker"
        );
    }

    /// A slow request must not make a later fast one wait for a *lock* — only for
    /// its turn. The old design held a global mutex across the whole 4s stall, so
    /// a second caller was blocked even if it only wanted a timeout of 10ms.
    #[test]
    fn a_slow_request_does_not_wedge_a_later_fast_one() {
        let h = Arc::new(Harness::new(
            |c| c.text = Some("hello".into()),
            Duration::from_millis(200),
        ));

        // Caller A: a full-length blocking read, on another thread.
        let started = Arc::new(Barrier::new(2));
        let a = {
            let (h, started) = (h.clone(), started.clone());
            std::thread::spawn(move || {
                started.wait();
                h.blocking(|b| b.get_text(), None)
            })
        };
        started.wait();

        // Caller B bounds its own wait and is back promptly, even though A's read
        // is still in flight. Under a held global mutex B could not even *post*.
        let t0 = std::time::Instant::now();
        let b = h.blocking(|b| b.get_text(), Some(Duration::from_millis(20)));
        assert!(matches!(b, Err(ClipboardError::TimedOut)));
        assert!(
            t0.elapsed() < Duration::from_millis(150),
            "the bounded caller waited {:?}, i.e. it was blocked on the stall \
             rather than on its own timeout",
            t0.elapsed()
        );

        assert_eq!(a.join().unwrap().unwrap(), "hello");
    }

    /// The combined probe is **one** worker pass. The point of `paste_rich` is
    /// that a rich paste cannot stack three independent worst-case stalls, so
    /// this pins the job count, not just the answer.
    #[test]
    fn the_rich_probe_is_one_worker_pass() {
        let h = Harness::new(|c| c.text = Some("plain".into()), Duration::ZERO);

        let got = h.blocking(rich_probe, None).unwrap();
        assert!(matches!(got, RichPaste::Text(ref t) if t == "plain"));

        assert_eq!(
            h.jobs.load(Ordering::SeqCst),
            1,
            "the html/image/text probe must be a single job on the worker"
        );
        assert_eq!(
            h.read_counts(),
            (1, 1, 1),
            "the probe reads each flavour exactly once, inside that one job"
        );
    }

    /// Priority order: html wins over an image, an image over text.
    #[test]
    fn the_rich_probe_prefers_html_then_image_then_text() {
        let h = Harness::new(
            |c| {
                c.html = Some("<p>rich</p>".into());
                c.image = Some(ImageData::new(1, 1, vec![0u8; 4]));
                c.text = Some("plain".into());
            },
            Duration::ZERO,
        );
        assert!(
            matches!(h.blocking(rich_probe, None).unwrap(), RichPaste::Html(h) if h == "<p>rich</p>")
        );

        let h = Harness::new(
            |c| {
                c.image = Some(ImageData::new(1, 1, vec![0u8; 4]));
                c.text = Some("plain".into());
            },
            Duration::ZERO,
        );
        assert!(matches!(
            h.blocking(rich_probe, None).unwrap(),
            RichPaste::Image(_)
        ));

        // Whitespace-only html is not rich content — fall through to the text.
        let h = Harness::new(
            |c| {
                c.html = Some("   \n ".into());
                c.text = Some("plain".into());
            },
            Duration::ZERO,
        );
        assert!(
            matches!(h.blocking(rich_probe, None).unwrap(), RichPaste::Text(t) if t == "plain")
        );
    }

    /// An empty clipboard reports a content-type mismatch rather than an empty
    /// success, so a caller can tell "nothing to paste" from "pasted nothing".
    #[test]
    fn the_rich_probe_reports_an_empty_clipboard() {
        let h = Harness::new(|_| {}, Duration::ZERO);
        assert!(matches!(
            h.blocking(rich_probe, None),
            Err(ClipboardError::ContentTypeMismatch)
        ));
    }

    /// The async shape delivers on the worker thread — the reason the public
    /// callbacks are `Send` and the reason rinch marshals before touching UI state.
    #[test]
    fn an_async_job_delivers_on_the_worker_thread() {
        let h = Harness::new(|c| c.text = Some("hello".into()), Duration::ZERO);
        let (tx, rx) = mpsc::channel();
        assert!(h
            .worker
            .submit(Box::new(move |b| {
                let on_worker = ON_WORKER.with(|f| f.get());
                let _ = tx.send((b.get_text(), on_worker, std::thread::current().id()));
            }))
            .is_ok());
        let (text, on_worker, tid) = rx.recv().unwrap();
        assert_eq!(text.unwrap(), "hello");
        assert!(
            on_worker,
            "the callback runs with the worker-thread marker set"
        );
        assert_ne!(tid, std::thread::current().id());
    }

    /// A blocking call made from inside a callback would wait on the worker that
    /// is running it. It reports instead of hanging — a hang is the one failure
    /// mode a clipboard must never have.
    #[test]
    fn a_blocking_call_from_a_callback_is_reported_not_deadlocked() {
        let h = Harness::new(|c| c.text = Some("hello".into()), Duration::ZERO);
        let (tx, rx) = mpsc::channel();
        assert!(h
            .worker
            .submit(Box::new(move |_| {
                let _ = tx.send(paste_text());
            }))
            .is_ok());
        let result = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("a re-entrant blocking call must return, not hang the worker");
        assert!(matches!(result, Err(ClipboardError::AccessFailed(_))));
    }

    /// A backend that cannot be created answers every job with its creation
    /// error, and is retried on the next job rather than killing the worker.
    #[test]
    fn a_backend_that_fails_to_build_is_retried_not_fatal() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let a = attempts.clone();
        let worker = Worker::spawn(move || {
            let n = a.fetch_add(1, Ordering::SeqCst);
            if n < 2 {
                Err(ClipboardError::AccessFailed("no display".into()))
            } else {
                Ok(Box::new(FakeClipboard {
                    text: Some("late".into()),
                    ..Default::default()
                }) as Box<dyn Backend>)
            }
        });

        let ask = |worker: &Worker| {
            let (tx, rx) = mpsc::channel();
            assert!(worker
                .submit(Box::new(move |b| {
                    let _ = tx.send(b.get_text());
                }))
                .is_ok());
            rx.recv().unwrap()
        };

        assert!(matches!(ask(&worker), Err(ClipboardError::AccessFailed(_))));
        assert!(matches!(ask(&worker), Err(ClipboardError::AccessFailed(_))));
        assert_eq!(
            ask(&worker).unwrap(),
            "late",
            "a clipboard that becomes available later starts working with no restart"
        );
    }

    /// Copies and reads round-trip through the worker, so the blocking API keeps
    /// behaving exactly as it did before the worker existed.
    #[test]
    fn writes_and_reads_round_trip_through_the_worker() {
        let h = Harness::new(|_| {}, Duration::ZERO);
        assert!(h.blocking(|b| b.set_text("written"), None).is_ok());
        assert_eq!(h.blocking(|b| b.get_text(), None).unwrap(), "written");
        assert!(h
            .blocking(|b| b.set_html("<b>x</b>", Some("x")), None)
            .is_ok());
        assert_eq!(h.blocking(|b| b.get_html(), None).unwrap(), "<b>x</b>");
        assert_eq!(
            h.blocking(|b| b.get_text(), None).unwrap(),
            "x",
            "set_html's alt text is the text/plain alternative"
        );
        assert!(h.blocking(|b| b.clear(), None).is_ok());
        assert!(matches!(
            h.blocking(|b| b.get_text(), None),
            Err(ClipboardError::ContentTypeMismatch)
        ));
    }
}
