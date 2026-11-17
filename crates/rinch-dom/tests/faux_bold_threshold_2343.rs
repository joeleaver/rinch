//! When a face the app registered is drawn faux bold (#2343).
//!
//! Blink synthesises bold by two rules (measured in Chrome 156, canvas ink
//! at 48px, with Liberation Sans Regular as a system font and as `@font-face`
//! rules declaring 300, 400, 500 and 600):
//!
//! - an `@font-face` face is emboldened when the request is 600 or more and
//!   the face is under 600 (`CSSSegmentedFontFace`): a 400 face from 600,
//!   not at 599; a 600 face never, even at 900;
//! - a system face when the request is more than 200 over the face
//!   (`FontCache::CreateFontPlatformData`): a 400 face from 601, not at 600.
//!
//! A font registered with the collection is the app's own, rinch's
//! counterpart of `@font-face`, and takes the first rule; fontique applied the
//! system rule to every face, so `font-weight: 600` over a registered
//! regular-only family stayed regular. The rule lives in the parley fork
//! (`FontInfo::synthesis`); this pins what reaches a laid-out run. The system
//! rule is pinned through fontconfig by `fontconfig_metric_aliases_2336.rs`.
//! Host-independent: a bare collection holding only the bundled Inter.

use parley::fontique::{Blob, Collection, CollectionOptions, FontInfoOverride, FontWeight};
use parley::layout::PositionedLayoutItem;
use parley::style::{FontFamily, StyleProperty};

const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// A bare font context with Inter Regular registered as `family`, its weight
/// declared as `weight` (as an `@font-face` `font-weight` would).
fn context(family: &str, weight: Option<f32>) -> parley::FontContext {
    let mut cx = parley::FontContext {
        collection: Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        }),
        ..Default::default()
    };
    cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(INTER)),
        Some(FontInfoOverride {
            family_name: Some(family),
            weight: weight.map(FontWeight::new),
            ..Default::default()
        }),
    );
    cx
}

/// Whether every glyph run of `Hamburg` in `family` at `weight` is
/// emboldened; panics if nothing was laid out in `family`.
fn emboldened(cx: &mut parley::FontContext, family: &str, weight: f32) -> bool {
    let mut layout_cx: parley::LayoutContext<()> = parley::LayoutContext::new();
    let mut b = layout_cx.ranged_builder(cx, "Hamburg", 1.0, true);
    b.push_default(StyleProperty::FontSize(48.0));
    b.push_default(StyleProperty::FontFamily(FontFamily::Source(
        format!("'{family}'").into(),
    )));
    b.push_default(StyleProperty::FontWeight(FontWeight::new(weight)));
    let mut layout = b.build("Hamburg");
    layout.break_all_lines(None);
    let runs: Vec<bool> = layout
        .lines()
        .flat_map(|line| line.items().collect::<Vec<_>>())
        .filter_map(|item| match item {
            PositionedLayoutItem::GlyphRun(g) => Some(g.run().synthesis().embolden()),
            _ => None,
        })
        .collect();
    // Positive control: the text was laid out (in the only face there is).
    assert!(!runs.is_empty(), "no glyph run for {family} at {weight}");
    assert!(runs.iter().all(|&e| e == runs[0]), "{runs:?}");
    runs[0]
}

#[test]
fn a_registered_regular_face_is_emboldened_from_600() {
    let mut cx = context("Regular2343", None);
    for weight in [100.0, 400.0, 500.0, 550.0, 599.0] {
        assert!(!emboldened(&mut cx, "Regular2343", weight), "at {weight}");
    }
    // 600 is the issue's case: regular before the fix (600 is not > 400 +
    // 200), faux bold in Chrome.
    for weight in [600.0, 601.0, 700.0, 900.0] {
        assert!(emboldened(&mut cx, "Regular2343", weight), "at {weight}");
    }
}

#[test]
fn a_registered_light_face_is_emboldened_from_600_not_500() {
    // Chrome: a 300 face is emboldened at 600, not at 500 (and the old
    // +200 rule agreed at 500 but emboldened at 501).
    let mut cx = context("Light2343", Some(300.0));
    for weight in [300.0, 500.0, 501.0, 599.0] {
        assert!(!emboldened(&mut cx, "Light2343", weight), "at {weight}");
    }
    assert!(emboldened(&mut cx, "Light2343", 600.0));
}

#[test]
fn a_registered_semibold_face_is_never_emboldened() {
    // Chrome does not embolden a 600 face even at 900; the old rule did
    // (900 > 600 + 200).
    let mut cx = context("Semibold2343", Some(600.0));
    for weight in [600.0, 700.0, 800.0, 801.0, 900.0] {
        assert!(!emboldened(&mut cx, "Semibold2343", weight), "at {weight}");
    }
}
