//! App-bundled fonts: what a registered face is reachable **as**, and which
//! font contexts can reach it.
//!
//! Two issues meet here because they share a fixture and a question. #286 asks
//! *what names answer to a bundled face* — its own family name always, a CSS
//! generic only if it asked for that slot. #492 asks *which of rinch's several
//! font contexts* can answer at all. Both are settled by the same measurement:
//! lay out text against a `FontContext` and identify the file the glyphs came
//! out of.
//!
//! These are not about typography. The repository's two font files are both
//! Inter, which is not monospaced, and that does not matter — what is under
//! test is which *file* the text stack picked, and a face is put into the
//! `monospace` slot by being declared for it, never by its design.
//!
//! What is **not** covered: two faces from two different families, each
//! claiming its own generic, which is the shape a real app has. Both files here
//! are the same family, so registering them is one family with two weights and
//! cannot distinguish two slots.
//!
//! # The hit-test half (#492)
//!
//! A registered font must reach the **hit-test** font context, not just the
//! document's.
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
/// These live under `crates/rinch/assets/` rather than in an example, because
/// `cargo package -p rinch` does not carry `examples/`: a packaged or sparsely
/// checked-out `rinch` could not compile its own test target. Loud (a compile
/// error naming the path), not silent.
const FACE: &[u8] = include_bytes!("../../assets/fonts/Inter-Bold.ttf");

/// A second file from the same family. It does double duty: it establishes
/// that these identifiers distinguish one *file* from another — a fixture whose
/// two candidates were indistinguishable could not fail — and it stands in for
/// a platform face in
/// [`a_declared_face_beats_a_generic_the_repair_already_filled`].
const OTHER_FACE: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

/// Lay out `text` in `stack` against `font_cx` and hand each glyph run's source
/// blob to `identify`.
fn resolve_with<T>(
    font_cx: &mut parley::FontContext,
    stack: &str,
    text: &str,
    identify: impl Fn(&[u8]) -> T,
) -> Vec<T> {
    let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
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
    resolve_with(font_cx, stack, LATIN, |blob| blob.as_ptr() as usize)
}

/// The blob lengths the glyphs came from. Use this — and only this — to compare
/// two contexts on a stack that resolves to a *system* face, which the two map
/// at different addresses.
fn resolved_blob_lens(font_cx: &mut parley::FontContext, stack: &str) -> Vec<usize> {
    resolve_with(font_cx, stack, LATIN, |blob| blob.len())
}

/// The text every resolution above is measured with. Latin, because that is
/// what the fixture faces have glyphs for — see [`HAN`] for the other case.
const LATIN: &str = "Mm";

/// Text in a script neither fixture face covers, so the only thing that can
/// answer it is *fallback*. Inter is a Latin/Greek/Cyrillic face with no CJK
/// coverage at all.
const HAN: &str = "\u{6f22}\u{5b57}";

/// A mounted app carrying `fonts`, so there is a document with the same font
/// context the real layout path measures against.
fn app_with(fonts: &[AppFont]) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 100%; height: 100%");
        root
    });
    for font in fonts {
        app.register_app_font(*font);
    }
    app.mount_component(800.0, 600.0);
    app
}

/// [`app_with`] plus the wasm-shaped `register_font_data` registration, which
/// is the subject of the #492 pins below.
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

/// The addresses `stack` resolves to in the **document's** context — the one
/// layout and paint measure with.
fn doc_statics(app: &RinchApp, stack: &str) -> Vec<usize> {
    doc_statics_for(app, stack, LATIN)
}

/// [`doc_statics`] for text the fixture faces do not cover, which is the only
/// way to observe *fallback* rather than family/generic matching.
fn doc_statics_for(app: &RinchApp, stack: &str, text: &str) -> Vec<usize> {
    let doc = app.doc().expect("mounted");
    let mut d = doc.borrow_mut();
    resolve_with(&mut d.font_cx, stack, text, |blob| blob.as_ptr() as usize)
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

// ── What a bundled face is reachable as (#286) ───────────────────────────────
//
// Every assertion below identifies the answering face by the **address** of the
// blob its glyphs came out of, for the reason the module doc gives: a length is
// satisfiable by a byte-identical system copy of the same file, and Inter is a
// widely installed UI font. `AppFont::new(FACE)` claims nothing, so a length
// check on `font-family: Inter` would pass with the registration deleted.

/// Registering a face is enough to reach it **by its own family name** — the
/// name in the file's `name` table, with nothing declared anywhere.
#[test]
fn a_registered_face_answers_to_its_own_family_name() {
    let app = app_with(&[AppFont::new(FACE)]);
    assert_eq!(
        doc_statics(&app, "Inter"),
        vec![FACE.as_ptr() as usize],
        "`font-family: Inter` must come out of the bytes that were registered"
    );
}

/// The motivating case. A generic is a slot the platform fills, not a name any
/// font file carries, so a bundled face is in it only by declaring it — and
/// once declared, it answers.
#[test]
fn a_face_declared_monospace_answers_font_family_monospace() {
    let app = app_with(&[AppFont::monospace(FACE)]);
    assert_eq!(
        doc_statics(&app, "monospace"),
        vec![FACE.as_ptr() as usize],
        "`font-family: monospace` must resolve to the face that asked for that slot"
    );
}

/// And the half that makes the declaration mean something: registering a face
/// does **not** quietly make it the answer to every generic. A registered face
/// with no generics is reachable only by name, so a bundled serif does not
/// become the app's `monospace` by being present.
#[test]
fn registration_alone_does_not_claim_a_generic() {
    let app = app_with(&[AppFont::new(FACE)]);
    assert!(
        !doc_statics(&app, "monospace").contains(&(FACE.as_ptr() as usize)),
        "a face that declared no generic must not answer `monospace`; \
         it is reachable by name and that is all"
    );
}

/// A stack is tried in order, so the same face registered for `monospace` is
/// still not what `serif` gets. One declaration is one slot.
#[test]
fn a_declared_generic_claims_that_slot_and_no_other() {
    let app = app_with(&[AppFont::monospace(FACE)]);
    assert_eq!(doc_statics(&app, "monospace"), vec![FACE.as_ptr() as usize]);
    assert!(
        !doc_statics(&app, "serif").contains(&(FACE.as_ptr() as usize)),
        "declaring `monospace` must not also claim `serif`"
    );
}

/// A bundled face is put **ahead of** the platform's own entry for a slot: an
/// app that ships a face for `monospace` means that one. Asserted against
/// whatever the host has, by requiring the app's bytes to be first.
#[test]
fn a_bundled_face_outranks_the_platform_in_its_slot() {
    let app = app_with(&[AppFont::monospace(FACE)]);
    assert_eq!(
        doc_statics(&app, "monospace").first().copied(),
        Some(FACE.as_ptr() as usize),
        "the app's face must be picked before the system's monospace, not after"
    );
}

/// A face registered **after** the mount reaches the document that is already
/// on screen, not only the next one. It is still a frame late — which is what
/// [`App::fonts`](crate::App::fonts) exists to avoid — but it is not lost.
#[test]
fn a_late_registration_reaches_the_mounted_document() {
    let mut app = app_with(&[]);
    assert!(
        !doc_statics(&app, "monospace").contains(&(FACE.as_ptr() as usize)),
        "nothing registered yet"
    );
    app.register_app_font(AppFont::monospace(FACE));
    assert_eq!(
        doc_statics(&app, "monospace"),
        vec![FACE.as_ptr() as usize],
        "registering after the mount must reach the live font context"
    );
}

/// A declared face beats the #322 generic repair, not only the system map.
///
/// `rinch_dom::fonts::new_font_context` repairs a generic the platform left
/// empty by **appending a platform family into the collection's own list** —
/// at construction, before any app font can possibly register. On Android that
/// puts a platform face at the head of the `monospace` slot, so an app face
/// that merely *appended* itself would sit behind it and lose: the explicit
/// `AppFont::monospace(...)` declaration would silently resolve to
/// `Droid Sans Mono`. The sibling test
/// [`a_bundled_face_outranks_the_platform_in_its_slot`] cannot catch that —
/// fontconfig's generics live in the *system* map, behind the collection's own
/// list, which is exactly the one value where append and claim agree.
///
/// So this test performs the repair's move itself: it appends a stand-in
/// platform family (a different file, under a name no font carries) into the
/// mounted document's own `monospace` slot, then registers the app's face for
/// that slot, and requires the glyphs to come out of the app's bytes.
#[test]
fn a_declared_face_beats_a_generic_the_repair_already_filled() {
    use parley::fontique::{Blob, FontInfoOverride, GenericFamily};
    use std::sync::Arc;

    let mut app = app_with(&[]);
    {
        // What `repair_generic_families` does on Android: append a platform
        // family into an empty generic slot of the collection's own map.
        let doc = app.doc().expect("mounted");
        let mut d = doc.borrow_mut();
        let blob = Blob::new(Arc::new(OTHER_FACE));
        let families = d.font_cx.collection.register_fonts(
            blob,
            Some(FontInfoOverride {
                family_name: Some("Repair Standin Mono"),
                ..Default::default()
            }),
        );
        let ids: Vec<_> = families.iter().map(|(id, _)| *id).collect();
        assert!(!ids.is_empty(), "the stand-in must have registered");
        d.font_cx
            .collection
            .append_generic_families(GenericFamily::Monospace, ids.into_iter());
    }

    // The stand-in really is what `monospace` resolves to before the claim —
    // otherwise the assertion below would pass against a slot the app face
    // never had to win.
    assert_eq!(
        doc_statics(&app, "monospace").first().copied(),
        Some(OTHER_FACE.as_ptr() as usize),
        "the appended stand-in must hold the slot, or this fixture proves nothing"
    );

    app.register_app_font(AppFont::monospace(FACE));
    assert_eq!(
        doc_statics(&app, "monospace"),
        vec![FACE.as_ptr() as usize],
        "the face the app declared for `monospace` must beat the platform \
         face the repair filled the slot with"
    );
}

/// The families `text`'s glyphs came from, each as `(blob address, glyph ids)`.
///
/// The glyph ids are what distinguishes "this face answered" from "this face
/// answered *usefully*": a font with no coverage for a character still shapes
/// it, as glyph 0 — `.notdef`, the empty box.
fn han_run_via_bundled_face(app: &RinchApp) -> Vec<(usize, Vec<u32>)> {
    let doc = app.doc().expect("mounted");
    let mut d = doc.borrow_mut();
    let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    let mut builder = layout_cx.ranged_builder(&mut d.font_cx, HAN, 1.0, true);
    builder.push_default(parley::style::StyleProperty::FontSize(16.0));
    builder.push_default(parley::style::StyleProperty::FontStack(
        parley::style::FontStack::Source(std::borrow::Cow::Borrowed("Inter")),
    ));
    let mut layout = builder.build(HAN);
    layout.break_all_lines(None);
    let mut out = Vec::new();
    for line in layout.lines() {
        for item in line.items() {
            if let parley::layout::PositionedLayoutItem::GlyphRun(run) = item {
                out.push((
                    run.run().font().data.as_ref().as_ptr() as usize,
                    run.glyphs().map(|g| g.id).collect(),
                ));
            }
        }
    }
    out
}

/// Whether the registered `Inter` family is in the collection's own fallback
/// list for Han — the list fontique consults *before* the platform's.
fn bundled_face_is_a_han_fallback(app: &RinchApp) -> bool {
    use parley::fontique::{FallbackKey, Script};
    let doc = app.doc().expect("mounted");
    let mut d = doc.borrow_mut();
    let Some(inter) = d.font_cx.collection.family_id("Inter") else {
        panic!("the fixture face must be registered under its own family name");
    };
    d.font_cx
        .collection
        .fallback_families(FallbackKey::new(Script(*b"Hani"), None))
        .any(|id| id == inter)
}

/// `script_fallback` is off by default, and off has to *mean* something.
///
/// fontique resolves a script's fallback from the collection's own list first
/// and the platform's second — as an `else`, not a chain — so an entry in the
/// app's list **replaces** the platform's fallback for that script rather than
/// extending it. A bundled Latin face that joined every script would therefore
/// become what CJK, Arabic and emoji fall back to. Hence the default.
///
/// Asserted against the fallback list rather than a rendered run, because the
/// list is the same on every host: what a script resolves to when the app has
/// *not* claimed it depends on which fonts the machine has, and CI's set is
/// not ours.
#[test]
fn a_face_that_did_not_ask_for_fallback_leaves_the_platforms_script_alone() {
    let app = app_with(&[AppFont::monospace(FACE)]);
    assert!(
        !bundled_face_is_a_han_fallback(&app),
        "a face that did not ask for script fallback must stay out of Han's \
         fallback list; it has no CJK glyphs, and being there displaces the \
         platform face that does"
    );
}

/// The converse, and the measured cost of asking for it.
///
/// `register_font_data` — the wasm/embed front door — *does* set
/// `script_fallback`, because on a platform with no system fonts a last-resort
/// face is the only thing that renders a character the author did not name a
/// font for. On a platform that *has* fonts it is the harm above, and this
/// pins it concretely: with the claim in place, Han asked of a stack the
/// bundled face heads comes out of the bundled face's own bytes as glyph 0 —
/// `.notdef`, an empty box — because nothing else is left in the list to
/// answer it.
///
/// Host-independent in both halves: the primary family and the only fallback
/// are the same registered face, so no system font can change the outcome.
#[test]
fn register_font_data_joins_the_fallback_chain_notdef_and_all() {
    let app = mounted_app();
    assert!(
        bundled_face_is_a_han_fallback(&app),
        "register_font_data must claim every script's fallback; on wasm it is \
         the only thing standing between the app and no text at all"
    );

    let run = han_run_via_bundled_face(&app);
    let (blob, glyphs) = run.first().expect("Han must shape to something");
    assert_eq!(
        *blob,
        FACE.as_ptr() as usize,
        "with the claim in place, the bundled face is what answers Han"
    );
    assert!(
        glyphs.iter().all(|g| *g == 0),
        "and it answers with .notdef — this is the cost the default avoids, \
         not a hypothetical: got glyph ids {glyphs:?}"
    );
}
