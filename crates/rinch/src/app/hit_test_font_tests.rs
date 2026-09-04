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
//! Faces are identified by **the length of the blob the glyphs came out of**,
//! which is the only identification a system font cannot satisfy by having the
//! right name.

use super::*;

/// The face under test. Not installed on any machine, so its blob length is a
/// unique fingerprint for "this registration arrived".
///
/// The repository's only font files live under `examples/ui-zoo-web/assets`;
/// PR #286's `font_tests.rs` reaches for the same two, and the `resolved_faces`
/// helper below is the same shape as its own. When that lands, the two files
/// are worth merging — they are kept apart here only so neither PR has to
/// resolve a conflict in a file both create.
const FACE: &[u8] = include_bytes!("../../../../examples/ui-zoo-web/assets/Inter-Bold.ttf");

/// A second file from the same family, used only to establish that blob length
/// distinguishes one *file* from another rather than one family from another —
/// a fixture where both candidates had the same length could not fail.
const OTHER_FACE: &[u8] =
    include_bytes!("../../../../examples/ui-zoo-web/assets/Inter-Regular.ttf");

/// The byte lengths of the font files that supplied the glyphs for `stack`, in
/// run order. Empty when nothing resolved.
///
/// Takes the `FontContext` rather than the app, because the whole point is to
/// ask the same question of two different contexts.
fn resolved_faces(font_cx: &mut parley::FontContext, stack: &str) -> Vec<usize> {
    let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    let text = "Mm";
    let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);
    builder.push_default(parley::style::StyleProperty::FontSize(16.0));
    builder.push_default(parley::style::StyleProperty::FontStack(
        parley::style::FontStack::Source(std::borrow::Cow::Owned(stack.to_string())),
    ));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    let mut faces = Vec::new();
    for line in layout.lines() {
        for item in line.items() {
            if let parley::layout::PositionedLayoutItem::GlyphRun(run) = item {
                faces.push(run.run().font().data.as_ref().len());
            }
        }
    }
    faces
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

/// The two files really are distinguishable by length, so the assertions below
/// can tell "the registered face answered" from "some other Inter answered".
#[test]
fn the_two_fixture_faces_have_different_blob_lengths() {
    assert_ne!(
        FACE.len(),
        OTHER_FACE.len(),
        "the fixture identifies a face by blob length; two same-length files \
         would make every assertion below unfalsifiable"
    );
}

/// The baseline: the document context — the one layout and paint use — resolves
/// the registered family to the registered bytes. If this fails, the test below
/// is measuring nothing.
#[test]
fn the_document_context_resolves_the_registered_face() {
    let app = mounted_app();
    let doc = app.doc().expect("mounted");
    let mut d = doc.borrow_mut();
    assert_eq!(
        resolved_faces(&mut d.font_cx, "Inter"),
        vec![FACE.len()],
        "`font-family: Inter` must come out of the bytes that were registered"
    );
}

/// The pin (#492). The hit-test context must answer the same family with the
/// same file. Dropping the second line of `register_font_data` leaves this
/// context with no Inter at all, so it falls back to whatever the machine has —
/// a different blob, or none — while every other test in the suite stays green.
#[test]
fn the_hit_test_context_resolves_the_same_face_as_the_document() {
    let mut app = mounted_app();

    let from_document = {
        let doc = app.doc().expect("mounted");
        let mut d = doc.borrow_mut();
        resolved_faces(&mut d.font_cx, "Inter")
    };
    let from_hit_test = resolved_faces(&mut app.hit_test_font_cx, "Inter");

    assert_eq!(
        from_hit_test,
        vec![FACE.len()],
        "the hit-test context must resolve `Inter` to the registered bytes; \
         it resolved {from_hit_test:?} instead. Text would render from one face \
         and be hit-tested against another, so taps land on the wrong character."
    );
    assert_eq!(
        from_hit_test, from_document,
        "every FontContext in the process must resolve a stack identically"
    );
}
