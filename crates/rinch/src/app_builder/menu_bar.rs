//! The menu bar drawn in the window's document (Linux, a borderless
//! Windows window, and every build without `native-menus`, #2435) and the
//! first-frame width estimate of the inline titlebar spacer (#529).

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::WindowProps;

/// A crude per-character "narrow units" estimate for the inline titlebar
/// spacer's **first frame** (issue #529). Not a real text-advance measurement
/// — no font context exists yet at the point this runs, before the window or
/// its document are created. It is only the first frame's answer:
/// `BorderlessWindow` replaces it with the menu row's measured box as soon as
/// the row has been laid out, so what it gets wrong lasts one frame. It is a
/// bounded estimate: each `char` counts as 1 narrow unit or 2 for the common
/// double-width East Asian ranges (CJK ideographs, kana, hangul, fullwidth
/// forms, most emoji), rather than `str::len()`'s UTF-8 **byte** count, which
/// over-reserves any non-ASCII label by up to 4x (a 4-byte emoji) and already
/// over-reserves plain Cyrillic/Greek/accented text 2x (every such codepoint
/// is 2 UTF-8 bytes for one visual character).
#[cfg(all(
    feature = "windowing",
    any(
        target_os = "linux",
        target_os = "windows",
        not(feature = "native-menus")
    )
))]
fn char_width_units(c: char) -> u32 {
    let cp = c as u32;
    let is_wide = matches!(cp,
        0x1100..=0x115F     // Hangul Jamo
        | 0x2E80..=0x303E   // CJK Radicals, Kangxi, CJK Symbols and Punctuation
        | 0x3041..=0x33FF   // Hiragana .. CJK Compatibility
        | 0x3400..=0x4DBF   // CJK Unified Ideographs Extension A
        | 0x4E00..=0x9FFF   // CJK Unified Ideographs
        | 0xA000..=0xA4CF   // Yi Syllables and Radicals
        | 0xAC00..=0xD7A3   // Hangul Syllables
        | 0xF900..=0xFAFF   // CJK Compatibility Ideographs
        | 0xFF00..=0xFF60   // Fullwidth Forms
        | 0xFFE0..=0xFFE6   // Fullwidth Signs
        | 0x1F300..=0x1FAFF // Misc Symbols/Pictographs, Emoticons, Transport, Supplemental Symbols
        | 0x20000..=0x3FFFD // CJK Unified Ideographs Extension B and beyond
    );
    if is_wide { 2 } else { 1 }
}

/// The inline titlebar spacer width for a set of menu-bar labels (issue
/// #529), extracted so it can be pinned at all — it used to sit inside a
/// closure inside [`run_desktop_in_app_bar`], which only ever runs by taking over
/// the event loop and so could never be unit-tested.
///
/// `10px` padding-left + hamburger(`~36px`) + `2px` gap per item + each
/// label (`~8px` per narrow-unit + `16px` padding) + `10px` padding-right.
#[cfg(all(
    feature = "windowing",
    any(
        target_os = "linux",
        target_os = "windows",
        not(feature = "native-menus")
    )
))]
pub(crate) fn inline_spacer_width(labels: &[&str]) -> u32 {
    let labels_width: u32 = labels
        .iter()
        .map(|l| {
            let width_units: u32 = l.chars().map(char_width_units).sum();
            width_units * 8 + 16 + 2
        })
        .sum();
    10 + 36 + labels_width + 10
}

/// The startup path that draws the menu bar in the document: every Linux
/// window (there is no native bar there), a borderless Windows window
/// (a Win32 bar needs the system frame the window asked not to have, so
/// `SetMenu` on it shows nothing and the app had no menus at all), and every
/// window of a build without `native-menus` (#2435).
///
/// Split out of [`App::run`](super::App::run) only because the body is long; it is not a
/// second entry point.
#[cfg(all(
    feature = "windowing",
    any(
        target_os = "linux",
        target_os = "windows",
        not(feature = "native-menus")
    )
))]
pub(super) fn run_desktop_in_app_bar<F>(
    component: F,
    props: WindowProps,
    menus: Option<Vec<(String, crate::menu::Menu)>>,
    fonts: &[crate::font::AppFont],
) -> Result<(), crate::RunError>
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    let Some(menu_data) = menus else {
        return crate::shell::rinch_runtime::try_run_rinch_with_window_props_and_menu(
            component, props, None, fonts,
        );
    };
    let (wrapped, native_menu) = with_menu_bar_in_document(component, &props, menu_data);
    crate::shell::rinch_runtime::try_run_rinch_with_window_props_and_menu(
        wrapped,
        props,
        native_menu,
        fonts,
    )
}

/// A window's root component, boxed: what [`with_menu_bar_in_document`]
/// hands the shell.
#[cfg(all(
    feature = "windowing",
    any(
        target_os = "linux",
        target_os = "windows",
        not(feature = "native-menus")
    )
))]
pub(crate) type RootComponent = Box<dyn FnOnce(&mut RenderScope) -> NodeHandle>;

/// A window's root component wrapped in the menu bar drawn in its document,
/// and the native bar to hand the shell, which is `None` without
/// `native-menus`.
///
/// Both builds register the same chords and callbacks, by two routes. With
/// `native-menus` the native bar is built even here, where it is never shown
/// (Linux has no native bar, and a borderless Windows window's is never
/// attached): building it is what registers the items in the thread-local
/// registry, before the event loop starts. Without them there is no muda to
/// build with, and the wrapped component arms the chords itself through
/// [`register_menu_shortcuts`](crate::menu::register_menu_shortcuts), the
/// route the web takes, holding the token until the window's root scope is
/// disposed.
///
/// Apart from the shell's event loop, this is the whole of
/// [`run_desktop_in_app_bar`], so it is what the tests run.
#[cfg(all(
    feature = "windowing",
    any(
        target_os = "linux",
        target_os = "windows",
        not(feature = "native-menus")
    )
))]
pub(crate) fn with_menu_bar_in_document<F>(
    component: F,
    props: &WindowProps,
    menu_data: Vec<(String, crate::menu::Menu)>,
) -> (RootComponent, Option<crate::menu::NativeMenuBar>)
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    #[cfg(feature = "native-menus")]
    let native_menu = {
        let refs: Vec<(&str, crate::menu::Menu)> = menu_data
            .iter()
            .map(|(label, menu)| (label.as_str(), menu.clone()))
            .collect();
        Some(crate::menu::build_native_menu_bar(refs))
    };
    #[cfg(not(feature = "native-menus"))]
    let native_menu = None;

    if !props.borderless {
        let wrapped = move |scope: &mut RenderScope| {
            #[cfg(not(feature = "native-menus"))]
            arm_menu_bar_chords(scope, &menu_data);
            let content = component(scope);
            let menu_refs: Vec<(&str, &crate::menu::Menu)> = menu_data
                .iter()
                .map(|(label, menu)| (label.as_str(), menu))
                .collect();
            crate::menu::app_menu_bar::render_with_menu_bar(scope, &menu_refs, content, 0)
        };
        return (Box::new(wrapped), native_menu);
    }

    // Borderless: set MenuBarContext — BorderlessWindow renders the bar
    // internally, as an absolutely positioned layer carrying its own `z-index`
    // and appended last. BorderlessWindow handles that ordering.
    const TITLEBAR_HEIGHT: u32 = 36;
    let menu_in_titlebar = props.menu_in_titlebar;
    let wrapped = move |scope: &mut RenderScope| {
        #[cfg(not(feature = "native-menus"))]
        arm_menu_bar_chords(scope, &menu_data);
        let menu_data_rc = std::rc::Rc::new(menu_data);

        if menu_in_titlebar {
            // Inline titlebar layout: split renderers share a Signal
            use rinch_core::reactive::Signal;
            let active_menu: Signal<i32> = Signal::new(-1);

            let items_renderer: rinch_core::MenuBarRenderer = {
                let md = menu_data_rc.clone();
                std::rc::Rc::new(move |scope| {
                    let refs: Vec<(&str, &crate::menu::Menu)> =
                        md.iter().map(|(l, m)| (l.as_str(), m)).collect();
                    crate::menu::app_menu_bar::render_menu_items_inline(scope, &refs, active_menu)
                })
            };

            let overlay_renderer: rinch_core::MenuBarRenderer = {
                std::rc::Rc::new(move |scope| {
                    crate::menu::app_menu_bar::render_inline_overlay(scope, active_menu)
                })
            };

            // First-frame estimate of the inline row's width, for the
            // titlebar spacer; `BorderlessWindow` replaces it with the row's
            // measured box after the first layout (see `inline_spacer_width`).
            let labels: Vec<&str> = menu_data_rc.iter().map(|(l, _)| l.as_str()).collect();
            let spacer_w = inline_spacer_width(&labels);

            rinch_core::create_context(rinch_core::MenuBarContext {
                renderer: items_renderer.clone(),
                bar_height: 0,
                layout: rinch_core::MenuBarLayout::InlineTitlebar,
                items_renderer: Some(items_renderer),
                overlay_renderer: Some(overlay_renderer),
                spacer_width: spacer_w,
            });
        } else {
            // Below-titlebar layout: single standalone renderer
            let renderer: rinch_core::MenuBarRenderer = {
                let md = menu_data_rc.clone();
                std::rc::Rc::new(move |scope| {
                    let refs: Vec<(&str, &crate::menu::Menu)> =
                        md.iter().map(|(l, m)| (l.as_str(), m)).collect();
                    crate::menu::app_menu_bar::render_menu_bar_standalone(
                        scope,
                        &refs,
                        TITLEBAR_HEIGHT,
                    )
                })
            };
            rinch_core::create_context(rinch_core::MenuBarContext {
                renderer,
                bar_height: crate::menu::app_menu_bar::MENU_BAR_HEIGHT,
                layout: rinch_core::MenuBarLayout::BelowTitlebar,
                items_renderer: None,
                overlay_renderer: None,
                spacer_width: 0,
            });
        }

        component(scope)
    };
    (Box::new(wrapped), native_menu)
}

/// Arm the menu bar's chords and callbacks for as long as the window's root
/// scope lives: what building the native bar does in a build with
/// `native-menus`, done the web's way (#2435).
#[cfg(all(feature = "windowing", not(feature = "native-menus")))]
fn arm_menu_bar_chords(scope: &mut RenderScope, menus: &[(String, crate::menu::Menu)]) {
    let refs: Vec<(&str, &crate::menu::Menu)> = menus
        .iter()
        .map(|(label, menu)| (label.as_str(), menu))
        .collect();
    let chords = crate::menu::register_menu_shortcuts(&refs);
    // Held until disposal, or the registration releases itself on the spot
    // (see `MenuBarChords`).
    scope.on_cleanup(move || drop(chords));
}

#[cfg(test)]
mod tests {
    use super::*;

    // #529: the inline titlebar spacer used to size itself from
    // `label.len()` — UTF-8 **bytes** — rather than the label's visual
    // width, so any non-ASCII label over-reserved space proportional to how
    // many bytes its characters happened to encode to, not how wide they
    // draw. These fixtures use `inline_spacer_width` as it stands today
    // (char-count + East-Asian-wide doubling) and are proven below, by hand,
    // to fail against both the original bytes-based formula and against a
    // plain `chars().count()` with no wide-character doubling — the two
    // most plausible mutants of this fix.

    /// An ASCII-only label is exactly where `len()` (bytes) and
    /// `chars().count()` (chars) agree — the issue's own warning that such a
    /// fixture "cannot see this bug". Kept as a sanity check that the new
    /// formula still matches the documented base case, not as the bug's pin.
    #[cfg(any(
        target_os = "linux",
        target_os = "windows",
        not(feature = "native-menus")
    ))]
    #[test]
    fn an_ascii_label_reserves_the_documented_width() {
        // "File": 4 narrow units * 8 + 16 + 2 = 50, plus the 10+36+10 frame.
        assert_eq!(inline_spacer_width(&["File"]), 10 + 36 + 50 + 10);
    }

    /// "Файл" (Cyrillic for "File") is 4 **characters** but 8 UTF-8 bytes —
    /// every Cyrillic codepoint is 2 bytes. The byte-based formula reserved
    /// (8*8+16+2)=82px for it; the character-based one reserves (4*8+16+2)=50,
    /// matching the ASCII case above exactly, as it should for 4 narrow
    /// glyphs. Mutant this kills: reverting `l.chars().map(char_width_units).sum()`
    /// back to `l.len() as u32` reserves 82, not 50 — confirmed by hand
    /// (temporarily restoring the byte-based line) before writing this
    /// comment, then reverted.
    #[cfg(any(
        target_os = "linux",
        target_os = "windows",
        not(feature = "native-menus")
    ))]
    #[test]
    fn a_cyrillic_label_is_sized_by_characters_not_bytes() {
        assert_eq!(inline_spacer_width(&["Файл"]), 10 + 36 + 50 + 10);
    }

    /// "Édition" (French for "Edit") is 7 characters, one of them (`É`) a
    /// 2-byte accented codepoint, so the byte-based formula counted 8 bytes
    /// and reserved (8*8+16+2)=82; the character-based one counts 7 narrow
    /// units and reserves (7*8+16+2)=74. Mutant this kills: the same
    /// byte-count reversion as above, which would assert 82 here instead.
    #[cfg(any(
        target_os = "linux",
        target_os = "windows",
        not(feature = "native-menus")
    ))]
    #[test]
    fn an_accented_label_is_sized_by_characters_not_bytes() {
        assert_eq!(inline_spacer_width(&["Édition"]), 10 + 36 + 74 + 10);
    }

    /// "ファイル" (Japanese katakana for "File") is 4 characters, each a
    /// double-width glyph, encoded as 12 UTF-8 bytes. The byte-based formula
    /// reserved (12*8+16+2)=114; `chars().count()` alone (no wide-character
    /// doubling) would under-reserve at (4*8+16+2)=50 — about as wrong in
    /// the other direction, per the issue's own warning. The wide-aware
    /// formula counts 4 chars * 2 width-units = 8 units, reserving
    /// (8*8+16+2)=82. Mutant this kills: deleting the `is_wide` doubling in
    /// `char_width_units` (always returning 1) would assert 50 here instead
    /// of 82 — confirmed by hand (temporarily hard-coding `1`) before
    /// writing this comment, then reverted.
    #[cfg(any(
        target_os = "linux",
        target_os = "windows",
        not(feature = "native-menus")
    ))]
    #[test]
    fn a_cjk_label_reserves_double_width_per_character() {
        assert_eq!(inline_spacer_width(&["ファイル"]), 10 + 36 + 82 + 10);
    }

    /// Several labels compose additively, confirming the per-label formula
    /// (not just a single-label fixed point) and that one wide label does
    /// not affect a sibling narrow one's own contribution.
    #[cfg(any(
        target_os = "linux",
        target_os = "windows",
        not(feature = "native-menus")
    ))]
    #[test]
    fn multiple_labels_sum_their_own_widths() {
        // "File" -> 50, "ファイル" -> 82.
        assert_eq!(
            inline_spacer_width(&["File", "ファイル"]),
            10 + 36 + 50 + 82 + 10
        );
    }
}
