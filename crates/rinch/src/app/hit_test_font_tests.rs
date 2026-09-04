//! A registered font must reach the **hit-test** font context, not just the
//! document's (issue #492).
//!
//! `RinchApp` carries more than one `parley::FontContext`: the document's,
//! which layout and paint measure with, and `hit_test_font_cx`, which turns a
//! pointer position into a character index. They must resolve a `font-family`
//! stack **identically** — otherwise text renders from one face and is measured
//! for hit testing against another, and a tap lands on the wrong character by
//! however much the two faces' advances differ, accumulating along the line.
//!
//! That failure is invisible: nothing looks wrong, the caret is just one or two
//! characters off. Before this test, deleting the hit-test half of
//! `register_font_data` passed the entire suite, and the only thing asserting
//! the invariant was a comment.
//!
//! # Identifying a face, and why by pointer
//!
//! The obvious identifier is the length of the blob the glyphs came out of. It
//! is **not sufficient**, and this is measured rather than argued: with an
//! identical copy of `Inter-Bold.ttf` installed in `~/.local/share/fonts`, the
//! hit-test context with its registration deleted falls back to the *system*
//! Inter-Bold — same file, therefore same length — and a length assertion
//! passes against the very mutant it exists to catch. Inter is a widely
//! installed UI font and this repository vendors an official release of it, so
//! a byte-identical system copy is likely on a developer machine, not exotic.
//!
//! `register_font_on_context` does `Blob::new(Arc::new(data))` over the
//! `&'static [u8]` from `include_bytes!`, so the resolved blob's data pointer
//! **is** that static. A system font is file-backed and can never match it.
//! Pointer identity is therefore the discriminating check, and
//! [`resolved_statics`] is what the registered-face assertions use.
//!
//! The converse also had to be measured: two `FontContext`s map the same
//! *system* font at **different addresses** (probed: `monospace` resolves to
//! blob length 343140 in both contexts, at two different pointers). So a
//! stack that resolves to a system face can only be compared across contexts
//! by length — that is [`resolved_blob_lens`], and mixing the two up would
//! produce a test that fails for a reason that is not a defect.

use super::*;

/// The face under test.
///
/// The repository's only font files live under `examples/ui-zoo-web/assets`;
/// PR #286's `font_tests.rs` reaches for the same two, and the resolution
/// helpers below are the same shape as its own. When that lands, the two files
/// are worth merging — they are kept apart here only so neither PR has to
/// resolve a conflict in a file both create. That merge is also the right
/// moment to move these assets under `crates/rinch/`: `cargo package -p rinch`
/// does not carry `examples/`, so a packaged or sparsely-checked-out `rinch`
/// cannot compile its own test target. Loud (a compile error naming the path),
/// not silent, and `cargo package` already fails earlier on git dependencies.
const FACE: &[u8] = include_bytes!("../../../../examples/ui-zoo-web/assets/Inter-Bold.ttf");

/// A second file from the same family, used only to establish that these
/// identifiers distinguish one *file* from another — a fixture whose two
/// candidates were indistinguishable could not fail.
const OTHER_FACE: &[u8] =
    include_bytes!("../../../../examples/ui-zoo-web/assets/Inter-Regular.ttf");

/// Lay out `text` in `stack` against `font_cx` and hand each glyph run's source
/// blob to `identify`.
fn resolve_with<T>(
    font_cx: &mut parley::FontContext,
    stack: &str,
    identify: impl Fn(&[u8]) -> T,
) -> Vec<T> {
    let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    let text = "Mm";
    let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);
    builder.push_default(parley::style::StyleProperty::FontSize(16.0));
    builder.push_default(parley::style::StyleProperty::FontStack(
        parley::style::FontStack::Source(std::borrow::Cow::Owned(stack.to_string())),
    ));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    let mut out = Vec::new();
    for line in layout.lines() {
        for item in line.items() {
            if let parley::layout::PositionedLayoutItem::GlyphRun(run) = item {
                out.push(identify(run.run().font().data.as_ref()));
            }
        }
    }
    out
}

/// The addresses the glyphs came from. Equal to `FACE.as_ptr()` only when the
/// glyphs really came out of the registered static — never when a same-sized
/// system font answered instead. Use this for anything about the *registered*
/// face.
fn resolved_statics(font_cx: &mut parley::FontContext, stack: &str) -> Vec<usize> {
    resolve_with(font_cx, stack, |blob| blob.as_ptr() as usize)
}

/// The blob lengths the glyphs came from. Use this — and only this — to compare
/// two contexts on a stack that resolves to a *system* face, which the two map
/// at different addresses.
fn resolved_blob_lens(font_cx: &mut parley::FontContext, stack: &str) -> Vec<usize> {
    resolve_with(font_cx, stack, |blob| blob.len())
}

fn mounted_app() -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 100%; height: 100%");
        root
    });
    app.register_font_data(FACE);
    app.mount_component(800.0, 600.0);
    app
}

/// The two files really are distinguishable, so the assertions below can tell
/// "the registered face answered" from "some other Inter answered".
#[test]
fn the_two_fixture_faces_are_distinguishable() {
    assert_ne!(FACE.as_ptr(), OTHER_FACE.as_ptr());
    assert_ne!(FACE.len(), OTHER_FACE.len());
}

/// The baseline: the document context — the one layout and paint use — resolves
/// the registered family to the registered bytes. If this fails, the pin below
/// is measuring nothing.
#[test]
fn the_document_context_resolves_the_registered_face() {
    let app = mounted_app();
    let doc = app.doc().expect("mounted");
    let mut d = doc.borrow_mut();
    assert_eq!(
        resolved_statics(&mut d.font_cx, "Inter"),
        vec![FACE.as_ptr() as usize],
        "`font-family: Inter` must come out of the bytes that were registered"
    );
}

/// The pin (#492). The hit-test context must answer the same family with the
/// same file. Dropping the second line of `register_font_data` leaves this
/// context with no registered Inter, so it falls back — to nothing, to a
/// different face, or (the case a length check misses) to a byte-identical
/// system Inter at a different address.
#[test]
fn the_hit_test_context_resolves_the_registered_face() {
    let mut app = mounted_app();

    let from_document = {
        let doc = app.doc().expect("mounted");
        let mut d = doc.borrow_mut();
        resolved_statics(&mut d.font_cx, "Inter")
    };
    let from_hit_test = resolved_statics(&mut app.hit_test_font_cx, "Inter");

    assert_eq!(
        from_hit_test,
        vec![FACE.as_ptr() as usize],
        "the hit-test context must resolve `Inter` to the registered static; \
         text would render from one face and be hit-tested against another, so \
         taps land on the wrong character"
    );
    assert_eq!(from_hit_test, from_document);
}

/// Registration is not only about the family *name*. `register_font_on_context`
/// also claims the `sans-serif` and `system-ui` generic slots, which is what
/// makes a bundled face reachable from a theme stack — and on WASM, where no
/// system fonts exist and the default stack ends in `sans-serif`, it is the
/// only thing standing between the app and no text at all.
///
/// Pinning it here rather than only for `Inter` is what makes the generic
/// mapping, and the `family_ids` plumbing that feeds it, non-deletable: both
/// survive a test that asks only for the family by name.
#[test]
fn both_contexts_resolve_the_generic_the_face_claimed() {
    let mut app = mounted_app();

    let from_document = {
        let doc = app.doc().expect("mounted");
        let mut d = doc.borrow_mut();
        resolved_statics(&mut d.font_cx, "sans-serif")
    };
    let from_hit_test = resolved_statics(&mut app.hit_test_font_cx, "sans-serif");

    assert_eq!(
        from_document,
        vec![FACE.as_ptr() as usize],
        "a registered face claims `sans-serif`; without that mapping a themed \
         stack — and every WASM app — resolves to nothing"
    );
    assert_eq!(
        from_hit_test, from_document,
        "both contexts must agree on the generic, not just on the family name"
    );
}

/// The two contexts must also agree on a generic the app did **not** claim,
/// which is where they resolve independently through the platform.
///
/// Compared by blob length, not address: a system face is mapped separately by
/// each context. This is the `rinch_dom::fonts::new_font_context()` invariant
/// (#322) — but note it can only *catch* a divergence, not prove the repair is
/// present, because on Linux fontique fills `monospace` natively and both
/// constructors agree there. The platform where the repair matters is Android,
/// which this host test cannot reach.
#[test]
fn both_contexts_agree_on_an_unclaimed_generic() {
    let mut app = mounted_app();

    let from_document = {
        let doc = app.doc().expect("mounted");
        let mut d = doc.borrow_mut();
        resolved_blob_lens(&mut d.font_cx, "monospace")
    };
    let from_hit_test = resolved_blob_lens(&mut app.hit_test_font_cx, "monospace");

    assert!(
        !from_document.is_empty(),
        "monospace must resolve to something"
    );
    assert_eq!(
        from_hit_test, from_document,
        "every FontContext in the process must resolve a stack identically, or \
         a tap lands on the wrong character"
    );
}
