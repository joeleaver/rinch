//! Files from the user, on the desktop and the web (issue #1771): a file
//! picker and OS file drops that hand the app [`LocalFile`]s — a name, a size,
//! the path where there is one, and the bytes on demand.
//!
//! ```ignore
//! use rinch::prelude::*;
//! use rinch::file::FilePicker;
//!
//! // A picker, opened from a click (the web requires a user gesture).
//! button {
//!     onclick: move || {
//!         FilePicker::new()
//!             .add_filter("glTF", &["glb", "gltf"])
//!             .open(move |files| {
//!                 for file in files {
//!                     file.read(move |bytes| { /* … */ });
//!                 }
//!             });
//!     },
//!     "Import…"
//! }
//!
//! // A drop zone: the same `LocalFile`s.
//! div { ondropfiles: move |files: Vec<LocalFile>| { /* … */ }, "Drop a model here" }
//! ```
//!
//! The picker is on the web always and on native builds with the
//! `file-dialogs` feature (it is `rinch::dialogs`' `rfd` dialog, modal like
//! them). `ondropfiles` works wherever an OS drop reaches rinch: the desktop
//! window and, on the web, a page under `rinch-web`.

pub use rinch_core::file::{FileSource, LocalFile, ReadFuture};

#[cfg(target_arch = "wasm32")]
pub use web::from_web_file;

/// A file picker: one or more files, read on demand.
///
/// Start it from an event handler with [`pick`](Self::pick) (await the files)
/// or [`open`](Self::open) (a callback). Cancelling the picker gives no files.
#[cfg(any(target_arch = "wasm32", feature = "file-dialogs"))]
#[derive(Debug, Clone, Default)]
pub struct FilePicker {
    filters: Vec<(String, Vec<String>)>,
    multiple: bool,
    title: Option<String>,
}

/// [`FilePicker::new`].
#[cfg(any(target_arch = "wasm32", feature = "file-dialogs"))]
pub fn pick_files() -> FilePicker {
    FilePicker::new()
}

#[cfg(any(target_arch = "wasm32", feature = "file-dialogs"))]
impl FilePicker {
    /// A picker for one file of any type.
    pub fn new() -> Self {
        Self::default()
    }

    /// Offer files with these extensions (no dot: `&["glb", "gltf"]`). On the
    /// web every filter's extensions together are the input's `accept`, which
    /// a browser treats as a suggestion; check what you get.
    pub fn add_filter(mut self, name: impl Into<String>, extensions: &[&str]) -> Self {
        self.filters.push((
            name.into(),
            extensions.iter().map(|e| e.to_string()).collect(),
        ));
        self
    }

    /// Let the user pick several files (default: one).
    pub fn multiple(mut self, multiple: bool) -> Self {
        self.multiple = multiple;
        self
    }

    /// The dialog's title, on the desktop. A browser's picker has none.
    pub fn set_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Open the picker **now** and resolve to the files picked, empty when it
    /// was cancelled.
    ///
    /// The picker opens when `pick` is called, not when the future is first
    /// polled, so call it in the event handler (a browser opens a picker only
    /// during a user gesture, and ignores the request otherwise — rinch then
    /// logs a warning and resolves to no files). Await it inside
    /// [`spawn_local`](crate::task::spawn_local):
    ///
    /// ```ignore
    /// onclick: move || {
    ///     let picked = FilePicker::new().pick();
    ///     spawn_local(async move {
    ///         for file in picked.await {
    ///             let bytes = file.bytes().await;
    ///             // …
    ///         }
    ///     });
    /// }
    /// ```
    ///
    /// On the desktop the dialog is modal: `pick` returns once it closes.
    pub fn pick(self) -> impl std::future::Future<Output = Vec<LocalFile>> + 'static {
        self.start()
    }

    /// Open the picker now and call `on_picked` with the files picked (none
    /// when it was cancelled), later, on the UI thread, in one batch. Called
    /// during a render, `on_picked` is dropped unrun if that component
    /// unmounts first (it is [`pick`](Self::pick) in a
    /// [`spawn_local`](crate::task::spawn_local)).
    pub fn open(self, on_picked: impl FnOnce(Vec<LocalFile>) + 'static) {
        let picked = self.pick();
        rinch_core::task::spawn_local(async move { on_picked(picked.await) });
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "file-dialogs"))]
    fn start(self) -> std::future::Ready<Vec<LocalFile>> {
        let mut dialog = rfd::FileDialog::new();
        if let Some(title) = &self.title {
            dialog = dialog.set_title(title);
        }
        for (name, extensions) in &self.filters {
            dialog = dialog.add_filter(name, extensions);
        }
        let paths = if self.multiple {
            dialog.pick_files().unwrap_or_default()
        } else {
            dialog.pick_file().into_iter().collect()
        };
        std::future::ready(
            paths
                .into_iter()
                .filter_map(|p| LocalFile::from_path(p).ok())
                .collect(),
        )
    }

    #[cfg(target_arch = "wasm32")]
    fn start(self) -> impl std::future::Future<Output = Vec<LocalFile>> + 'static {
        let accept = self
            .filters
            .iter()
            .flat_map(|(_, exts)| exts.iter())
            .map(|e| format!(".{}", e.trim_start_matches('.')))
            .collect::<Vec<_>>()
            .join(",");
        web::pick(accept, self.multiple)
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    //! The browser half: a hidden `<input type=file>` for the picker, a
    //! `FileReader` for a read.

    use super::LocalFile;
    use rinch_core::file::{FileSource, ReadFuture};
    use std::cell::{Cell, RefCell};
    use std::io;
    use std::rc::Rc;
    use std::task::{Poll, Waker};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;

    /// A browser `File` as a [`LocalFile`]: its name and size, no path, read
    /// with a `FileReader` when asked. `rinch-web` makes the files of a drop
    /// with it; an app holding a `File` of its own can too.
    pub fn from_web_file(file: web_sys::File) -> LocalFile {
        let name = file.name();
        let size = file.size().max(0.0) as u64;
        LocalFile::from_source(name, size, None, Rc::new(WebFile(file)))
    }

    struct WebFile(web_sys::File);

    type Slot<T> = Rc<RefCell<(Option<T>, Option<Waker>)>>;

    fn fill<T>(slot: &Slot<T>, value: T) {
        let waker = {
            let mut s = slot.borrow_mut();
            s.0 = Some(value);
            s.1.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    fn wait<T: 'static>(slot: Slot<T>) -> impl std::future::Future<Output = T> + 'static {
        std::future::poll_fn(move |cx| {
            let mut s = slot.borrow_mut();
            match s.0.take() {
                Some(value) => Poll::Ready(value),
                None => {
                    s.1 = Some(cx.waker().clone());
                    Poll::Pending
                }
            }
        })
    }

    impl FileSource for WebFile {
        fn read(&self) -> ReadFuture {
            let file = self.0.clone();
            Box::pin(async move {
                let reader = web_sys::FileReader::new()
                    .map_err(|e| io::Error::other(format!("{e:?}")))?;
                let slot: Slot<io::Result<Vec<u8>>> = Rc::default();
                let filled = slot.clone();
                // `loadend` fires once, after a success or a failure; the
                // closure frees itself when it has run.
                let loaded = {
                    let reader = reader.clone();
                    Closure::once_into_js(move || {
                        let result = match reader.result() {
                            Ok(result) => match result.dyn_into::<js_sys::ArrayBuffer>() {
                                Ok(buffer) => Ok(js_sys::Uint8Array::new(&buffer).to_vec()),
                                Err(_) => Err(read_error(&reader)),
                            },
                            Err(_) => Err(read_error(&reader)),
                        };
                        fill(&filled, result);
                    })
                };
                reader.set_onloadend(Some(loaded.unchecked_ref()));
                reader
                    .read_as_array_buffer(&file)
                    .map_err(|e| io::Error::other(format!("{e:?}")))?;
                wait(slot).await
            })
        }
    }

    fn read_error(reader: &web_sys::FileReader) -> io::Error {
        match reader.error() {
            Some(e) => io::Error::other(format!("{}: {}", e.name(), e.message())),
            None => io::Error::other("the file could not be read"),
        }
    }

    thread_local! {
        static REQUIRE_ACTIVATION: Cell<bool> = const { Cell::new(true) };
        static WARNED: Cell<bool> = const { Cell::new(false) };
    }

    /// Tests only: open the picker input with `click()`, which needs no user
    /// gesture to put the input in the page (a test has none), so a test can
    /// answer it itself.
    #[doc(hidden)]
    pub fn __set_require_user_activation(require: bool) {
        REQUIRE_ACTIVATION.with(|r| r.set(require));
    }

    /// The attribute the picker's hidden input carries.
    pub const PICKER_ATTR: &str = "data-rinch-file-picker";

    /// Show `input`'s picker. `showPicker()` where the browser has it: it
    /// refuses outright (`NotAllowedError`) without a user gesture, where
    /// `click()` is silently ignored and no `change` or `cancel` ever comes.
    /// `false` when the picker was refused.
    fn show(input: &web_sys::HtmlInputElement) -> bool {
        if !REQUIRE_ACTIVATION.with(Cell::get) {
            input.click();
            return true;
        }
        let show_picker = js_sys::Reflect::get(input, &JsValue::from_str("showPicker"))
            .ok()
            .and_then(|f| f.dyn_into::<js_sys::Function>().ok());
        let Some(show_picker) = show_picker else {
            input.click();
            return true;
        };
        match show_picker.call0(input) {
            Ok(_) => true,
            Err(e) => {
                let name = js_sys::Reflect::get(&e, &JsValue::from_str("name"))
                    .ok()
                    .and_then(|n| n.as_string())
                    .unwrap_or_default();
                if name != "NotAllowedError" {
                    // Any other refusal (an engine that wants the input
                    // rendered differently, say): the old way still works.
                    input.click();
                    return true;
                }
                if !WARNED.with(|w| w.replace(true)) {
                    tracing::warn!(
                        "FilePicker opened outside a user gesture; the browser shows no picker. \
                         Call pick()/open() from an event handler."
                    );
                }
                false
            }
        }
    }

    pub(super) fn pick(
        accept: String,
        multiple: bool,
    ) -> impl std::future::Future<Output = Vec<LocalFile>> + 'static {
        let slot: Slot<Vec<LocalFile>> = Rc::default();
        if !open_input(&accept, multiple, &slot) {
            fill(&slot, Vec::new());
        }
        wait(slot)
    }

    /// Put a hidden `<input type=file>` in the page and show its picker.
    /// `false` when there is nothing to wait for (no page, or no user gesture).
    fn open_input(accept: &str, multiple: bool, slot: &Slot<Vec<LocalFile>>) -> bool {
        let Some(window) = web_sys::window() else {
            return false;
        };
        let Some(document) = window.document() else {
            return false;
        };
        let Some(body) = document.body() else {
            return false;
        };
        let Ok(input) = document
            .create_element("input")
            .map(|e| e.unchecked_into::<web_sys::HtmlInputElement>())
        else {
            return false;
        };
        input.set_type("file");
        input.set_multiple(multiple);
        if !accept.is_empty() {
            input.set_accept(accept);
        }
        let _ = input.set_attribute(PICKER_ATTR, "");
        // Rendered but out of sight: `showPicker()` refuses an input that is
        // not being rendered (`display: none`).
        let _ = input.set_attribute(
            "style",
            "position: fixed; left: -10000px; top: 0; width: 1px; height: 1px; opacity: 0",
        );
        // One closure for `change` (files picked) and `cancel` (none); it
        // removes the input and drops itself when either arrives.
        let handler: Rc<RefCell<Option<Closure<dyn FnMut(web_sys::Event)>>>> = Rc::default();
        let closure = {
            let input = input.clone();
            let handler = handler.clone();
            let slot = slot.clone();
            Closure::wrap(Box::new(move |event: web_sys::Event| {
                let files = if event.type_() == "change" {
                    input
                        .files()
                        .map(|list| {
                            (0..list.length())
                                .filter_map(|i| list.get(i))
                                .map(from_web_file)
                                .collect()
                        })
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                input.remove();
                fill(&slot, files);
                handler.borrow_mut().take();
            }) as Box<dyn FnMut(web_sys::Event)>)
        };
        for name in ["change", "cancel"] {
            let _ = input.add_event_listener_with_callback(name, closure.as_ref().unchecked_ref());
        }
        *handler.borrow_mut() = Some(closure);
        if body.append_child(&input).is_err() {
            handler.borrow_mut().take();
            return false;
        }
        if !show(&input) {
            input.remove();
            handler.borrow_mut().take();
            return false;
        }
        true
    }
}

#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
pub use web::{PICKER_ATTR as __PICKER_ATTR, __set_require_user_activation};
