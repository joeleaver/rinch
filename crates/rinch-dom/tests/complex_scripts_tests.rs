//! parley's `complex-scripts` feature must stay on, and this is what it buys.
//!
//! parley moved its text analysis from swash to icu4x in #436 (0.8.0) and, in
//! 0.10.0, put the dictionary-based line and word segmenter behind an
//! **opt-in** Cargo feature that is not in `default`. The workspace turns it on
//! at `Cargo.toml`'s `parley = { version = "0.11.1", features =
//! ["complex-scripts"] }`. Nothing about that is enforced by the type system:
//! dropping the feature compiles, and every Latin fixture in the workspace
//! stays green.
//!
//! # What the feature actually changes — measured, both ways, on this host
//!
//! A 20-character Thai string, laid out at 16px and broken at a width a third
//! of its own unwrapped width:
//!
//! | | Thai lines | CJK lines | stderr |
//! |---|---|---|---|
//! | `complex-scripts` on | **4** | 9 | clean |
//! | `complex-scripts` off | **1** | 9 | `ICU4X data error: No segmentation model for language: th` (twice per layout) |
//!
//! So Thai does **not** degrade to character-level breaks, as the feature's own
//! description suggests: without the dictionary it stops breaking **at all**
//! and the paragraph overflows its container on one line. That is the
//! regression this file pins.
//!
//! **CJK is not a discriminator and must not be read as one.** Its line count
//! is identical with the feature off — CJK line breaking comes from the
//! Unicode line-break property classes, not from the dictionary, which is why
//! it is unaffected. The CJK case is kept only as a second, independent witness
//! that the line breaker runs at all; the Thai case is the pin.
//!
//! # The stderr check, and why it needs a child process
//!
//! The message is not `log`gable. icu_provider is built here without its
//! `logging` feature, and its fallback shim is
//! `pub use std::eprintln as warn;` (`icu_provider/src/lib.rs`), gated on
//! `debug_assertions` — so it is a raw write to stderr in a debug build and a
//! **no-op macro in release**. Installing a `log` logger captures nothing;
//! measured, zero records while the text appears on stderr.
//!
//! libtest intercepts `eprintln!` from the test thread, so the only way to see
//! it is to re-exec this binary with `--nocapture` and read the child's stderr.
//! The child branch is the first thing each such test does.

use parley::style::{FontFamily, StyleProperty};

/// 20 Thai characters with no spaces — Thai does not write them, which is the
/// whole reason a dictionary is needed to find its line-break opportunities.
const THAI: &str = "\u{e2a}\u{e27}\u{e31}\u{e2a}\u{e14}\u{e35}\u{e0a}\u{e32}\u{e27}\u{e42}\
                    \u{e25}\u{e01}\u{e17}\u{e35}\u{e48}\u{e19}\u{e48}\u{e32}\u{e23}\u{e31}";

/// Japanese, mixing kanji and kana.
const CJK: &str = "\u{4eca}\u{65e5}\u{306f}\u{3044}\u{3044}\u{5929}\u{6c17}\u{3067}\u{3059}\
                   \u{306d}\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}\u{4e16}\u{754c}";

/// A Latin control, whose breaking needs no dictionary on any build.
const LATIN: &str = "the quick brown fox jumps over the lazy dog";

const FONT_SIZE: f32 = 16.0;

/// Set on the re-exec so the child does the layout and the parent reads its
/// stderr. See the module header.
const CHILD_ENV: &str = "RINCH_COMPLEX_SCRIPTS_CHILD";

/// Lay `text` out at an unbounded width, then again at `unwrapped / divisor`,
/// and return `(unwrapped width, line count when broken)`.
///
/// The break width is derived from the string's own measured width rather than
/// declared as a literal, so the fixture carries no pin on the host's Thai or
/// CJK metrics — only on whether break opportunities exist inside the run.
fn lines_at_fraction(text: &str, divisor: f32) -> (f32, usize) {
    let mut font_cx = rinch_dom::fonts::new_font_context();
    let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();

    let mut build = |max: Option<f32>| {
        let mut b = layout_cx.ranged_builder(&mut font_cx, text, 1.0, true);
        b.push_default(StyleProperty::FontSize(FONT_SIZE));
        b.push_default(StyleProperty::FontFamily(FontFamily::Source(
            std::borrow::Cow::Borrowed("sans-serif"),
        )));
        let mut layout = b.build(text);
        layout.break_all_lines(max);
        (layout.width(), layout.len())
    };

    let (unwrapped, _) = build(None);
    let (_, lines) = build(Some(unwrapped / divisor));
    (unwrapped, lines)
}

/// The pin. Without `complex-scripts` this is **1**, measured — a Thai
/// paragraph runs off the side of its container instead of wrapping.
#[test]
fn thai_finds_line_break_opportunities_inside_an_unspaced_run() {
    let (unwrapped, lines) = lines_at_fraction(THAI, 3.0);

    // Positive control. If the host resolved no face at all for Thai the run
    // could measure zero and "does not wrap" would be true for a reason that
    // has nothing to do with the dictionary, so say which it is.
    assert!(
        unwrapped > 0.0,
        "the Thai run measured {unwrapped}px wide, so nothing was shaped and \
         this fixture is measuring nothing — not a `complex-scripts` failure"
    );

    assert!(
        lines > 1,
        "a {} px Thai run did not break at a third of its own width \
         ({lines} line). parley's `complex-scripts` feature is off, so icu4x \
         has no Thai segmentation dictionary and finds no break opportunity \
         inside the run at all. Restore \
         `features = [\"complex-scripts\"]` on the workspace `parley` dependency.",
        unwrapped
    );
}

/// A second witness that the line breaker runs, and the Latin control beside
/// it. Neither of these two discriminates the feature — both are identical
/// with it off (measured) — which is exactly why the Thai fixture above is the
/// pin and this one is not.
#[test]
fn cjk_and_latin_still_break_and_are_not_the_pin() {
    let (cjk_width, cjk_lines) = lines_at_fraction(CJK, 3.0);
    assert!(cjk_width > 0.0, "nothing was shaped for the CJK run");
    assert!(
        cjk_lines > 1,
        "a CJK run did not break at a third of its own width ({cjk_lines} line); \
         CJK breaks on Unicode line-break classes and needs no dictionary, so \
         this is a line-breaker failure rather than a `complex-scripts` one"
    );

    let (latin_width, latin_lines) = lines_at_fraction(LATIN, 3.0);
    assert!(latin_width > 0.0, "nothing was shaped for the Latin run");
    assert!(
        latin_lines > 1,
        "a spaced Latin sentence did not break at a third of its own width \
         ({latin_lines} line); the line breaker is not running"
    );
}

/// Laying Thai out must leave stderr clean.
///
/// Debug-only by construction: icu_provider's stderr shim is compiled out when
/// `debug_assertions` is off (module header), so the check has nothing to look
/// at in a release test build and says so rather than passing vacuously.
#[test]
fn laying_out_thai_writes_nothing_to_stderr() {
    if std::env::var_os(CHILD_ENV).is_some() {
        // The child half: do the layout and let icu4x complain if it wants to.
        let _ = lines_at_fraction(THAI, 3.0);
        let _ = lines_at_fraction(CJK, 3.0);
        return;
    }

    if !cfg!(debug_assertions) {
        // Not a silent pass: name the reason, so a release run cannot be read
        // as evidence.
        eprintln!(
            "skipping the stderr check: icu_provider's `eprintln!` shim is \
             compiled out without `debug_assertions`"
        );
        return;
    }

    let exe = std::env::current_exe().expect("the test binary must have a path");
    let out = std::process::Command::new(&exe)
        .args([
            "--exact",
            "laying_out_thai_writes_nothing_to_stderr",
            "--nocapture",
        ])
        .env(CHILD_ENV, "1")
        .output()
        .expect("re-exec the test binary");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    // Positive control, and the one this repo's memory insists on: a filter
    // that matches nothing prints `0 passed` and reads exactly like a clean
    // run. Prove the child ran the test before believing its silence.
    assert!(
        stdout.contains("1 passed"),
        "the child process did not run the test — its filter matched nothing, \
         so its empty stderr means nothing.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        out.status.success(),
        "the child process failed.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    assert!(
        !stderr.contains("ICU4X data error"),
        "laying out Thai wrote an ICU4X data error to stderr, twice per layout. \
         parley's `complex-scripts` feature is off.\nstderr:\n{stderr}"
    );
}
