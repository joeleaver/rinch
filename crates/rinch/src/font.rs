//! Fonts an application ships inside its own binary.
//!
//! A rinch app that wants a particular typeface has two ways to get one: the
//! platform's font list, or its own bytes. The platform list is not a promise
//! anyone can keep — a phone has whatever the OEM shipped, a Linux desktop has
//! whatever the user installed — so an app with a designed identity, or an app
//! that needs a *monospaced* face for content whose alignment carries meaning,
//! has to carry the file.
//!
//! `AppFont` is one such file plus the answer to the question the CSS side
//! actually asks: **what names does this face respond to?**
//!
//! # Two ways a face gets picked
//!
//! ```ignore
//! font-family: Newsreader, Georgia, serif;
//! ```
//!
//! Every name in that list is looked up one of two ways:
//!
//! * **By family name** — `Newsreader`, `Georgia`. This is the name in the
//!   font file's own `name` table, and registering the file is enough: nothing
//!   else has to be declared for `font-family: Newsreader` to find it.
//! * **By generic** — `serif`, `sans-serif`, `monospace`, `system-ui`. These
//!   are not names of anything; they are slots the platform fills. A bundled
//!   face is not in a slot until it is put in one, which is what
//!   `AppFont::generics` does.
//!
//! So a face registered with no generics is reachable, but only by an author
//! who spells its name. If the motivating stack ends in a generic — and
//! essentially every real stack does — the generic is the entry that has to
//! resolve, and it will not resolve to a bundled face by accident.
//!
//! # Ordering
//!
//! Fonts have to be registered **before the first layout pass**, or the first
//! frame is measured against a fallback and then reflows. That is why they are
//! part of the startup configuration — [`App::fonts`](crate::App::fonts) —
//! rather than a function to call at some point before starting the app: with
//! the fonts in the builder that starts the app, there is no ordering left to
//! get wrong. It is also why `fonts` is one method on the same builder as
//! everything else, so bundling a typeface costs nothing in what else an app
//! can configure (issue #493).

/// The CSS generic families a bundled face can answer to.
///
/// Not mirrored: this is the very type `fontique` (and so parley) uses —
/// `fontique::GenericFamily` is a re-export of `parlance::GenericFamily`,
/// which this names directly so the module builds without the desktop text
/// stack (on `wasm32` too, #1769). The set is whatever the text stack
/// understands and cannot drift from it.
/// The CSS names are the ones [`GenericFamily::parse`] accepts: `serif`,
/// `sans-serif`, `monospace`, `cursive`, `fantasy`, `system-ui`, `ui-serif`,
/// `ui-sans-serif`, `ui-monospace`, `ui-rounded`, `emoji`, `math`,
/// `fangsong`.
pub use parlance::GenericFamily;

/// The browser's font settings: the default font and what `serif` and
/// `sans-serif` mean. Chrome's unless an app sets others with
/// [`App::font_defaults`](crate::App::font_defaults) (#2255).
pub use rinch_core::font_defaults::FontDefaults;

/// Set this thread's [`FontDefaults`], for an embedder that builds its
/// context itself (`App::font_defaults` does this for an app). Call it on
/// the thread that lays out, before the first context is created.
#[cfg(any(feature = "windowing", feature = "android", feature = "embed"))]
pub fn set_font_defaults(defaults: FontDefaults) {
    rinch_dom::fonts::set_font_defaults(defaults);
}

/// Scan the installed fonts again, so text drawn from here on sees fonts
/// installed (or removed) since the app started (#2486).
///
/// The process scans the system fonts once and every font context shares
/// that scan (#2467), so without a refresh a font the user installs while a
/// long-running app is open (a panel, a launcher) stays missing until it
/// restarts. Call this when the app knows the fonts changed; on Linux the
/// font watch ([`watch_system_fonts`]) calls it for you.
///
/// It returns at once: the scan (as costly as the start-up one, about 250 M
/// instructions for 1,328 fonts) runs on a thread of its own, and then every
/// `RinchApp` rebuilds its font contexts on its next frame, registers its own
/// fonts on them again, and restyles, re-measures and repaints its document.
/// Text keeps the old fonts until then. On the web (the browser's fonts) and
/// on wasm (no system fonts) it does nothing.
#[cfg(any(feature = "windowing", feature = "android", feature = "embed"))]
pub fn refresh_system_fonts() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let refresh = || {
            rinch_dom::fonts::refresh_system_fonts();
            // Wake the event loop: an empty main-thread callback is a
            // `ReRender`, and a frame adopts the new scan.
            rinch_core::dispatch_main_callback(Box::new(|| {}));
        };
        if std::thread::Builder::new()
            .name("rinch-font-refresh".into())
            .spawn(refresh)
            .is_err()
        {
            refresh();
        }
    }
}

/// Watch for fonts installed while the app runs, or `None` to stop (#2486,
/// #2537). What `interval` means depends on the platform:
///
/// - **Linux** (the `windowing` feature): rinch asks the kernel (inotify) to say
///   when anything the font scan read changes: its fontconfig configuration
///   files and included configuration directories, and its font directories
///   (a configured one that does not exist yet included, through its parent).
///   `interval` is the **debounce**: after a change rinch waits that long for
///   more (installing a family is many files) and then rescans once
///   ([`refresh_system_fonts`]). The wait is counted from the first change,
///   not the last, so while changes keep coming there is at most one rescan
///   per interval. Between changes the watch thread sleeps in the kernel, so
///   an idle app has no wake-ups for it, whatever the interval.
/// - **macOS, Windows, Android**: nothing watches; the setting is kept and
///   unused. Call [`refresh_system_fonts`] when the app knows the fonts
///   changed.
///
/// [`App::run`](crate::App::run) starts the watch with
/// [`DEFAULT_SYSTEM_FONT_WATCH`] unless this was called first; an embedded
/// `RinchContext` has none until its host calls this. The watch takes its
/// list of paths from the scan rinch already made, so the start-up pays
/// nothing for it beyond the inotify watches themselves. It never makes that
/// scan: started before the first font context exists, it waits for that
/// context's scan (#2774), and watches nothing until one exists.
#[cfg(any(feature = "windowing", feature = "android", feature = "embed"))]
pub fn watch_system_fonts(interval: Option<std::time::Duration>) {
    crate::system_font_watch::set(interval);
}

/// What [`App::run`](crate::App::run) gives [`watch_system_fonts`] unless the
/// app chose: on Linux, the wait after a change before the rescan.
pub const DEFAULT_SYSTEM_FONT_WATCH: std::time::Duration = std::time::Duration::from_secs(5);

/// Register a face with the running app, from anywhere on the UI thread: a
/// component, an effect, a timer, the callback of a download.
///
/// [`App::fonts`](crate::App::fonts) is for the faces a build carries, known
/// before the first frame. This is for the ones that are not: a typeface the
/// user picks from a catalogue and the app fetches, a document that names
/// its own fonts. The app object that owns the font contexts
/// ([`RinchApp::register_app_font`](crate::app::RinchApp::register_app_font))
/// is the shell's, out of reach of component code; this queues the face for
/// it.
///
/// On the desktop it takes effect on the event loop's next turn: every open
/// window's document registers the face, and text already on screen in a
/// fallback is shaped and painted again in it, a frame behind. A window
/// opened afterwards is made with it, as with the build's own fonts. Called
/// before the loop starts, the face is registered with the first window,
/// before its first layout.
///
/// A face is reachable by the family name in its file, and answers the
/// generics it claims ([`AppFont::with_generics`]); see [`AppFont`]. Each
/// call registers the bytes once more: an app that may ask for one face
/// twice keeps its own record of what it registered. The bytes are normally
/// owned ([`AppFont::from_data`]) and live as long as the app does.
///
/// Where it applies:
///
/// - **Desktop** (`windowing`): as above. Off the UI thread it is ignored,
///   with a warning.
/// - **Web**: does nothing. The browser loads fonts from the page's CSS
///   (`@font-face`, a stylesheet `<link>`), as [`App::fonts`](crate::App::fonts)
///   leaves it to.
/// - **Android** and an embedded `RinchContext`: does nothing yet; the embed
///   registers through `RinchContext::register_app_font`.
pub fn register_font(font: AppFont) {
    #[cfg(feature = "windowing")]
    {
        if !rinch_core::is_main_thread() {
            tracing::warn!("rinch::font::register_font was called off the UI thread; ignored");
            return;
        }
        RUNTIME_FONTS.with(|q| q.borrow_mut().push(font));
        // Wake a running shell to register it. Before a runtime exists the
        // event is dropped (`send_native_event`), and the runtime takes the
        // face from the queue when it is made.
        crate::shell::rinch_runtime::send_native_event(
            crate::shell::rinch_runtime::RinchNativeEvent::RegisterFonts,
        );
    }
    #[cfg(not(feature = "windowing"))]
    {
        let _ = font;
        tracing::debug!("rinch::font::register_font: no desktop runtime here; ignored");
    }
}

#[cfg(feature = "windowing")]
thread_local! {
    /// Faces [`register_font`] queued for the desktop runtime, in order.
    static RUNTIME_FONTS: std::cell::RefCell<Vec<AppFont>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Take the faces [`register_font`] queued, oldest first, leaving none.
#[cfg(feature = "windowing")]
pub(crate) fn take_registered_fonts() -> Vec<AppFont> {
    RUNTIME_FONTS.with(|q| std::mem::take(&mut *q.borrow_mut()))
}

/// The bytes of a font file: borrowed for the life of the program, or owned
/// and shared (#2101).
///
/// `&'static [u8]` — normally [`include_bytes!`] — is what a font built into
/// the binary is. Font data that arrives while the app runs (a file the user
/// picks, a game's own fonts) is owned instead: a `Vec<u8>`, a `Box<[u8]>`,
/// an `Arc<[u8]>`, or a `Blob<u8>` from parley's stack. Each converts with
/// `.into()`, and rinch keeps the bytes alive for as long as a font context
/// that registered them does — no longer. Nothing is leaked: dropping the
/// app (or the embed's [`RinchContext`](crate::embed::RinchContext)) that
/// registered an owned face drops rinch's references to it.
///
/// Cloning is cheap (a pointer, or a reference count).
#[derive(Clone)]
pub struct FontData(FontDataRepr);

#[derive(Clone)]
enum FontDataRepr {
    Static(&'static [u8]),
    Shared(std::sync::Arc<dyn AsRef<[u8]> + Send + Sync>),
    /// A `Blob` kept whole, so its id — what parley's and Vello's caches key
    /// a font on — is the app's own.
    #[cfg(any(feature = "windowing", feature = "android", feature = "embed"))]
    Blob(peniko::Blob<u8>),
}

impl FontData {
    /// Bytes that live for the whole program (`include_bytes!`).
    pub const fn from_static(data: &'static [u8]) -> Self {
        Self(FontDataRepr::Static(data))
    }

    /// Bytes owned by a shared handle; rinch holds a clone of `data` for as
    /// long as a font context uses the face.
    pub fn shared(data: std::sync::Arc<dyn AsRef<[u8]> + Send + Sync>) -> Self {
        Self(FontDataRepr::Shared(data))
    }

    /// The font file's bytes.
    pub fn as_bytes(&self) -> &[u8] {
        match &self.0 {
            FontDataRepr::Static(data) => data,
            FontDataRepr::Shared(data) => (**data).as_ref(),
            #[cfg(any(feature = "windowing", feature = "android", feature = "embed"))]
            FontDataRepr::Blob(blob) => blob.data(),
        }
    }

    /// Whether the bytes are `'static` rather than owned.
    pub fn is_static(&self) -> bool {
        matches!(self.0, FontDataRepr::Static(_))
    }

    /// The `Blob` a font context registers: a `Blob` given as one keeps its
    /// id; otherwise a new one over the static slice, or over a clone of the
    /// owned handle (so every context shares the one allocation).
    #[cfg(any(feature = "windowing", feature = "android", feature = "embed"))]
    pub(crate) fn to_blob(&self) -> peniko::Blob<u8> {
        match &self.0 {
            FontDataRepr::Static(data) => peniko::Blob::new(std::sync::Arc::new(*data)),
            FontDataRepr::Shared(data) => peniko::Blob::new(data.clone()),
            FontDataRepr::Blob(blob) => blob.clone(),
        }
    }
}

impl std::fmt::Debug for FontData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontData")
            .field("len", &self.as_bytes().len())
            .field("static", &self.is_static())
            .finish()
    }
}

impl From<&'static [u8]> for FontData {
    fn from(data: &'static [u8]) -> Self {
        Self::from_static(data)
    }
}

impl<const N: usize> From<&'static [u8; N]> for FontData {
    fn from(data: &'static [u8; N]) -> Self {
        Self::from_static(data)
    }
}

impl From<Vec<u8>> for FontData {
    fn from(data: Vec<u8>) -> Self {
        Self::shared(std::sync::Arc::new(data))
    }
}

impl From<Box<[u8]>> for FontData {
    fn from(data: Box<[u8]>) -> Self {
        Self::shared(std::sync::Arc::new(data))
    }
}

/// One `Arc` around the caller's, made here once: an `Arc<[u8]>` is unsized,
/// so it cannot itself become the `Arc<dyn AsRef<[u8]>>` a `Blob` holds.
/// Every font context then clones that one handle.
impl From<std::sync::Arc<[u8]>> for FontData {
    fn from(data: std::sync::Arc<[u8]>) -> Self {
        Self::shared(std::sync::Arc::new(data))
    }
}

impl From<std::sync::Arc<Vec<u8>>> for FontData {
    fn from(data: std::sync::Arc<Vec<u8>>) -> Self {
        Self::shared(data)
    }
}

#[cfg(any(feature = "windowing", feature = "android", feature = "embed"))]
impl From<peniko::Blob<u8>> for FontData {
    fn from(data: peniko::Blob<u8>) -> Self {
        Self(FontDataRepr::Blob(data))
    }
}

/// A font file the application carries, and the CSS names it answers to.
///
/// The bytes are a TrueType (`.ttf`), OpenType (`.otf`) or collection
/// (`.ttc`/`.otc`) file: normally from [`include_bytes!`] through
/// [`AppFont::new`] and its siblings, or owned bytes loaded at run time
/// through [`AppFont::from_data`] (see [`FontData`]).
///
/// ```ignore
/// use rinch::prelude::*;
///
/// const FACES: &[AppFont] = &[
///     // Reachable as `font-family: Newsreader`, and as `serif`.
///     AppFont::serif(include_bytes!("../assets/fonts/Newsreader.ttf")),
///     // Reachable as `font-family: Karla` only — the app names it directly.
///     AppFont::new(include_bytes!("../assets/fonts/Karla.ttf")),
///     // What `font-family: monospace` resolves to, ahead of the platform.
///     AppFont::monospace(include_bytes!("../assets/fonts/DejaVuSansMono.ttf")),
/// ];
///
/// App::new(app).title("My App").fonts(FACES).run();
///
/// // A face read from disk while the app runs, answering `sans-serif`:
/// let bytes: Vec<u8> = std::fs::read("UserFont.ttf")?;
/// let face = AppFont::from_data(bytes).with_generics(AppFont::SANS_SERIF);
/// ```
#[derive(Clone, Debug)]
pub struct AppFont {
    /// The font file.
    ///
    /// A collection file registers every face it holds; each keeps its own
    /// family name, and `generics` applies to all of them.
    pub data: FontData,

    /// The CSS generic families this face becomes the answer to.
    ///
    /// A bundled face is put **ahead of** whatever the platform had in that
    /// slot: an app that ships a face for `monospace` means that one, not the
    /// system's. Empty is the honest default — a face is reachable by its own
    /// family name whatever this says, and claiming a generic a face is not
    /// (a serif answering `sans-serif`) is worse than claiming none.
    pub generics: &'static [GenericFamily],

    /// Whether this face joins the last-resort fallback chain for every
    /// script.
    ///
    /// Fallback is what runs when the matched face has no glyph for a
    /// character. **Leave this off unless the platform has no fonts at all.**
    /// On a platform that does have them, adding an entry to a script's
    /// fallback list replaces that script's platform fallback rather than
    /// extending it (fontique consults the app's list first and stops if it
    /// finds one), so one bundled Latin face would silently become the
    /// fallback for CJK, Arabic and emoji alike.
    ///
    /// On wasm, where there is no system font source, the opposite holds: a
    /// fallback face is the only thing that renders a character an author did
    /// not name a font for. That is the case
    /// [`RinchApp::register_font_data`](crate::app::RinchApp::register_font_data)
    /// was written for, and it still sets this.
    pub script_fallback: bool,
}

impl AppFont {
    /// The generics [`AppFont::serif`] claims.
    pub const SERIF: &'static [GenericFamily] = &[GenericFamily::Serif, GenericFamily::UiSerif];

    /// The generics [`AppFont::sans_serif`] claims.
    pub const SANS_SERIF: &'static [GenericFamily] = &[
        GenericFamily::SansSerif,
        GenericFamily::SystemUi,
        GenericFamily::UiSansSerif,
    ];

    /// The generics [`AppFont::monospace`] claims.
    pub const MONOSPACE: &'static [GenericFamily] =
        &[GenericFamily::Monospace, GenericFamily::UiMonospace];

    /// A face reachable by its own family name and nothing else.
    pub const fn new(data: &'static [u8]) -> Self {
        Self {
            data: FontData::from_static(data),
            generics: &[],
            script_fallback: false,
        }
    }

    /// [`AppFont::new`] for bytes of any kind [`FontData`] takes, owned ones
    /// included: a face loaded while the app runs.
    pub fn from_data(data: impl Into<FontData>) -> Self {
        Self {
            data: data.into(),
            generics: &[],
            script_fallback: false,
        }
    }

    /// This face, answering `generics` as well as its own family name
    /// ([`AppFont::SERIF`], [`AppFont::SANS_SERIF`], [`AppFont::MONOSPACE`],
    /// or a list of your own).
    pub fn with_generics(self, generics: &'static [GenericFamily]) -> Self {
        Self { generics, ..self }
    }

    /// A face that also answers `font-family: serif`.
    pub const fn serif(data: &'static [u8]) -> Self {
        Self {
            data: FontData::from_static(data),
            generics: Self::SERIF,
            script_fallback: false,
        }
    }

    /// A face that also answers `font-family: sans-serif` — and `system-ui`,
    /// which is the same request phrased as "whatever this platform uses for
    /// UI", and which a theme's default stack usually ends in.
    pub const fn sans_serif(data: &'static [u8]) -> Self {
        Self {
            data: FontData::from_static(data),
            generics: Self::SANS_SERIF,
            script_fallback: false,
        }
    }

    /// A face that also answers `font-family: monospace`.
    pub const fn monospace(data: &'static [u8]) -> Self {
        Self {
            data: FontData::from_static(data),
            generics: Self::MONOSPACE,
            script_fallback: false,
        }
    }
}

/// When the system's generic font families are looked up on a thread of
/// their own ([`App::font_warmup`](crate::App::font_warmup), issue #2552).
///
/// Text with a glyph the first fonts of its stack lack (a menu's `✓`)
/// otherwise looks them up one by one the first time, on the UI thread: a
/// panel's first menu took 80–560 ms to show. The look-up costs 250–670 ms
/// of one core once, and about 1 MB of memory kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontWarmup {
    /// Once the app has presented its first frame, so the look-up competes
    /// with nothing at start-up. An app that presents no frame (a windowless
    /// one, a main window that starts hidden) does not look them up until it
    /// does.
    #[default]
    AfterFirstFrame,
    /// As the app starts, whether or not it has a window: for an app whose
    /// first text comes late and must show at once, such as a launcher that
    /// starts hidden and is summoned by a key, or a notification daemon with
    /// no window until its first notification. It makes the font scan on its
    /// thread if no document has yet, so the first document may wait for it.
    AtStart,
    /// Never: the first text that needs a family looks it up.
    Off,
}
