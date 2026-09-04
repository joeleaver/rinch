//! Fonts an application ships inside its own binary.
//!
//! A rinch app that wants a particular typeface has two ways to get one: the
//! platform's font list, or its own bytes. The platform list is not a promise
//! anyone can keep — a phone has whatever the OEM shipped, a Linux desktop has
//! whatever the user installed — so an app with a designed identity, or an app
//! that needs a *monospaced* face for content whose alignment carries meaning,
//! has to carry the file.
//!
//! [`AppFont`] is one such file plus the answer to the question the CSS side
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
//!   [`AppFont::generics`] does.
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
/// Re-exported from `fontique` rather than mirrored, so that the set is
/// whatever the text stack actually understands and cannot drift from it.
/// The CSS names are the ones [`GenericFamily::parse`] accepts: `serif`,
/// `sans-serif`, `monospace`, `cursive`, `fantasy`, `system-ui`, `ui-serif`,
/// `ui-sans-serif`, `ui-monospace`, `ui-rounded`, `emoji`, `math`,
/// `fangsong`.
pub use parley::fontique::GenericFamily;

/// A font file the application carries in its own binary, and the CSS names it
/// answers to.
///
/// The bytes are a TrueType (`.ttf`), OpenType (`.otf`) or collection
/// (`.ttc`/`.otc`) file, normally from [`include_bytes!`]. They are `'static`
/// because a registered face outlives every layout that used it.
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
/// ```
#[derive(Clone, Copy, Debug)]
pub struct AppFont {
    /// The font file.
    ///
    /// A collection file registers every face it holds; each keeps its own
    /// family name, and `generics` applies to all of them.
    pub data: &'static [u8],

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
    /// A face reachable by its own family name and nothing else.
    pub const fn new(data: &'static [u8]) -> Self {
        Self {
            data,
            generics: &[],
            script_fallback: false,
        }
    }

    /// A face that also answers `font-family: serif`.
    pub const fn serif(data: &'static [u8]) -> Self {
        Self {
            generics: &[GenericFamily::Serif, GenericFamily::UiSerif],
            ..Self::new(data)
        }
    }

    /// A face that also answers `font-family: sans-serif` — and `system-ui`,
    /// which is the same request phrased as "whatever this platform uses for
    /// UI", and which a theme's default stack usually ends in.
    pub const fn sans_serif(data: &'static [u8]) -> Self {
        Self {
            generics: &[
                GenericFamily::SansSerif,
                GenericFamily::SystemUi,
                GenericFamily::UiSansSerif,
            ],
            ..Self::new(data)
        }
    }

    /// A face that also answers `font-family: monospace`.
    pub const fn monospace(data: &'static [u8]) -> Self {
        Self {
            generics: &[GenericFamily::Monospace, GenericFamily::UiMonospace],
            ..Self::new(data)
        }
    }
}
