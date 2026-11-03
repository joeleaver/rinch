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
    builder.push_default(parley::style::StyleProperty::FontFamily(
        parley::style::FontFamily::Source(std::borrow::Cow::Owned(stack.to_string())),
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
        app.register_app_font(font.clone());
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

/// Registration is not only about the family *name*. A face that claims
/// `sans-serif` and `system-ui` (`AppFont::sans_serif`) has to head the slot
/// in **both** contexts, which is what makes a bundled face reachable from a
/// theme stack.
///
/// Pinning it here rather than only for `Inter` is what makes the generic
/// mapping, and the `family_ids` plumbing that feeds it, non-deletable: both
/// survive a test that asks only for the family by name. (It used
/// `register_font_data`, which claimed the slot too until #2102: data now
/// fills only an *empty* slot, and on this host the platform fills it.)
#[test]
fn both_contexts_resolve_the_generic_the_face_claimed() {
    let mut app = app_with(&[AppFont::sans_serif(FACE)]);

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
    builder.push_default(parley::style::StyleProperty::FontFamily(
        parley::style::FontFamily::Source(std::borrow::Cow::Borrowed("Inter")),
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
        .fallback_families(FallbackKey::new(Script::from_bytes(*b"Hani"), None))
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

/// `register_font_data` heads `sans-serif` **and** `system-ui` where they are
/// empty, and both halves are load-bearing.
///
/// The default theme's stack ends in `sans-serif`; `system-ui` is the same
/// request phrased as "whatever this platform uses for its own UI", and a
/// theme that names neither a family nor a generic ends up asking for one of
/// the two. On wasm — which is the front door this function exists for —
/// there is no platform face behind either slot, so a registration that
/// quietly stopped filling them would not fall back to something worse, it
/// would fall back to nothing. Measured on a context with no system fonts,
/// which is that platform's shape on any host (#2102: on a desktop the
/// platform fills both, and data no longer displaces it).
#[test]
fn register_font_data_answers_sans_serif_and_system_ui() {
    let mut font_cx = bare_context();
    register_data(&mut font_cx, FACE);
    // The slots themselves: on a bare context an empty `sans-serif` would
    // still *resolve* to the face, through the script fallback it also joins.
    assert_eq!(
        slot_names(&mut font_cx, GenericFamily::SansSerif),
        ["Inter"]
    );
    assert_eq!(slot_names(&mut font_cx, GenericFamily::SystemUi), ["Inter"]);
    assert_eq!(
        resolved_statics(&mut font_cx, "sans-serif"),
        vec![FACE.as_ptr() as usize],
        "the registered face must head an empty `sans-serif`, which is where \
         the default theme's stack ends up"
    );
    assert_eq!(
        resolved_statics(&mut font_cx, "system-ui"),
        vec![FACE.as_ptr() as usize],
        "and `system-ui` with it — on wasm there is no system UI face for it \
         to mean instead"
    );
}

// ── Data fonts never change what a generic means (#2102) ─────────────────────
//
// `register_font_data` (the embed's `RinchContextConfig::fonts` and
// `register_font`) used to *claim* `sans-serif` and `system-ui` for every
// face: a prepend, so the face registered **last** headed the slot, a script
// face as easily as a text face, and every `sans-serif` element drew in it.
// Data now fills only an empty slot, only one of its own class, and never
// displaces what is there. The fixtures are Inter with its family name and
// declared class rewritten (`tests/support/font_variants.rs`), so each is a
// distinct family whose class the test chooses.

#[path = "../../tests/support/font_variants.rs"]
mod font_variants;
use font_variants::{Declared, variant};

/// A variant leaked to `'static`: the shape an app had to use before #2101,
/// so the desktop pin below runs unchanged against the old API.
fn leaked(name: &str, class: Declared) -> &'static [u8] {
    Box::leak(variant(name, class).into_boxed_slice())
}

/// A context with no system fonts: every generic slot starts empty, the
/// shape of wasm and of an embed on a platform with none, on any host.
fn bare_context() -> parley::FontContext {
    use parley::fontique::{Collection, CollectionOptions};
    parley::FontContext {
        collection: Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        }),
        ..Default::default()
    }
}

/// What `register_font_data` does to one context.
fn register_data(font_cx: &mut parley::FontContext, data: impl Into<FontData>) {
    RinchApp::register_font_on_context(font_cx, &PendingFont::data(data));
}

/// The families in `generic`'s slot, by name.
fn slot_names(font_cx: &mut parley::FontContext, generic: GenericFamily) -> Vec<String> {
    let ids: Vec<_> = font_cx.collection.generic_families(generic).collect();
    ids.into_iter()
        .filter_map(|id| font_cx.collection.family_name(id).map(str::to_owned))
        .collect()
}

use crate::font::GenericFamily;

/// The classes the fixtures declare are the classes the classifier reads,
/// so the slot tests below are about the slot, not the fixture.
#[test]
fn the_classifier_reads_each_fixtures_declared_class() {
    use rinch_dom::fonts::{FaceClass, classify_face};
    for (class, expected) in [
        (Declared::SansSerif, FaceClass::SansSerif),
        (Declared::Serif, FaceClass::Serif),
        (Declared::Script, FaceClass::Display),
        (Declared::Monospace, FaceClass::Monospace),
        (Declared::Unclassified, FaceClass::SansSerif),
    ] {
        assert_eq!(
            classify_face(&variant("Probe", class), 0),
            expected,
            "{class:?}"
        );
    }
}

/// The issue's shape on an empty slot (wasm): a sans face, then a script
/// face, in either order. `sans-serif` is the sans face both ways — not the
/// last registered, and not the script face even when it came first.
#[test]
fn an_empty_sans_serif_is_the_first_sans_face_whatever_came_before_or_after() {
    let sans = leaked("Sansa", Declared::SansSerif);
    let script = leaked("Scrpt", Declared::Script);
    for order in [[sans, script], [script, sans]] {
        let mut font_cx = bare_context();
        for data in order {
            register_data(&mut font_cx, data);
        }
        assert_eq!(
            resolved_statics(&mut font_cx, "sans-serif"),
            vec![sans.as_ptr() as usize],
            "`sans-serif` must be the sans face, registered {}",
            if order[0] == sans { "first" } else { "second" }
        );
        assert_eq!(slot_names(&mut font_cx, GenericFamily::SystemUi), ["Sansa"]);
        // The script face is reachable by its own name all the same.
        assert_eq!(
            resolved_statics(&mut font_cx, "Scrpt"),
            vec![script.as_ptr() as usize]
        );
    }
}

/// Once a slot holds a face, later data never changes it — a second sans
/// face included. After each registration, not just at the end.
#[test]
fn later_data_never_displaces_a_filled_slot() {
    let first = leaked("Sansa", Declared::SansSerif);
    let mut font_cx = bare_context();
    register_data(&mut font_cx, first);
    for (name, class) in [
        ("Sansb", Declared::SansSerif),
        ("Serfa", Declared::Serif),
        ("Scrpt", Declared::Script),
        ("Unkno", Declared::Unclassified),
    ] {
        register_data(&mut font_cx, variant(name, class));
        assert_eq!(
            slot_names(&mut font_cx, GenericFamily::SansSerif),
            ["Sansa"],
            "after registering {name}"
        );
        assert_eq!(
            resolved_statics(&mut font_cx, "sans-serif"),
            vec![first.as_ptr() as usize],
            "after registering {name}"
        );
    }
}

/// Each class fills its own slots and no other; a script face fills none.
#[test]
fn each_class_fills_only_its_own_empty_slots() {
    let mut font_cx = bare_context();
    register_data(&mut font_cx, variant("Scrpt", Declared::Script));
    for generic in [
        GenericFamily::SansSerif,
        GenericFamily::Serif,
        GenericFamily::Monospace,
        GenericFamily::SystemUi,
    ] {
        assert!(
            slot_names(&mut font_cx, generic).is_empty(),
            "a script face must not answer {generic:?}"
        );
    }
    register_data(&mut font_cx, variant("Serfa", Declared::Serif));
    register_data(&mut font_cx, variant("Monoa", Declared::Monospace));
    register_data(&mut font_cx, variant("Unkno", Declared::Unclassified));
    let expected = [
        (GenericFamily::Serif, "Serfa"),
        (GenericFamily::UiSerif, "Serfa"),
        (GenericFamily::Monospace, "Monoa"),
        (GenericFamily::UiMonospace, "Monoa"),
        (GenericFamily::SansSerif, "Unkno"),
        (GenericFamily::SystemUi, "Unkno"),
        (GenericFamily::UiSansSerif, "Unkno"),
    ];
    for (generic, name) in expected {
        assert_eq!(slot_names(&mut font_cx, generic), [name], "{generic:?}");
    }
}

/// The explicit way in still works: a face *claimed* for `sans-serif` heads
/// the slot, ahead of the data face that filled it. That is how an app says
/// it wants its face to be the generic.
#[test]
fn a_claimed_face_still_heads_the_slot() {
    let mut font_cx = bare_context();
    register_data(&mut font_cx, variant("Sansa", Declared::SansSerif));
    let claimed = PendingFont {
        font: AppFont::from_data(variant("Scrpt", Declared::Script))
            .with_generics(AppFont::SANS_SERIF),
        fill_empty_generics: false,
    };
    RinchApp::register_font_on_context(&mut font_cx, &claimed);
    assert_eq!(
        slot_names(&mut font_cx, GenericFamily::SansSerif),
        ["Scrpt", "Sansa"]
    );
}

/// The issue as reported (desktop, platform fonts present): register a sans
/// face and then a script face as data, through the app, and `sans-serif`
/// still means what the platform maps it to — in the document's context and
/// the hit-test one, after each registration. Before #2102 the script face,
/// registered last, headed `sans-serif` and this failed at the second face.
#[test]
fn registering_data_never_changes_what_sans_serif_means_on_a_desktop() {
    let mut app = app_with(&[]);
    assert!(
        app.hit_test_font_cx
            .collection
            .generic_families(GenericFamily::SansSerif)
            .next()
            .is_some(),
        "precondition: this host maps `sans-serif` to a platform face"
    );
    let doc_lens = |app: &RinchApp| {
        let doc = app.doc().expect("mounted");
        let mut d = doc.borrow_mut();
        resolved_blob_lens(&mut d.font_cx, "sans-serif")
    };
    let before = doc_lens(&app);
    let before_hit = resolved_blob_lens(&mut app.hit_test_font_cx, "sans-serif");
    let faces = [
        leaked("Sansa", Declared::SansSerif),
        leaked("Scrpt", Declared::Script),
        leaked("Serfa", Declared::Serif),
    ];
    for face in faces {
        app.register_font_data(face);
        assert_eq!(doc_lens(&app), before, "the document's `sans-serif`");
        assert_eq!(
            resolved_blob_lens(&mut app.hit_test_font_cx, "sans-serif"),
            before_hit,
            "the hit-test context's `sans-serif`"
        );
        let statics = doc_statics(&app, "sans-serif");
        assert!(
            faces
                .iter()
                .all(|f| !statics.contains(&(f.as_ptr() as usize))),
            "no registered data face may answer `sans-serif` here: {statics:?}"
        );
    }
}

// ── Owned font bytes are not leaked (#2101) ──────────────────────────────────

/// Owned bytes handed to the app — as data and as a claimed face — are held
/// while the app lives and released when it is dropped, after a mount that
/// laid out and painted text in that face. Measured by the `Arc`'s count:
/// anything rinch kept (a font context, a pending list, a text or glyph
/// cache) would hold a reference past the drop.
#[test]
fn owned_font_bytes_are_released_when_the_app_is_dropped() {
    let bytes: std::sync::Arc<[u8]> = variant("Ownad", Declared::SansSerif).into();
    {
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("p");
            root.set_attribute("style", "font-family: Ownad; font-size: 16px");
            root.append_child(&scope.create_text("Owned bytes"));
            root
        });
        app.register_font_data(bytes.clone());
        app.register_app_font(AppFont::from_data(bytes.clone()).with_generics(AppFont::SERIF));
        app.mount_component(400.0, 200.0);
        app.resolve_and_repaint(400.0, 200.0);
        #[cfg(software_shell)]
        let _ = app.build_pixels(1.0, (400, 200), false);
        assert!(
            std::sync::Arc::strong_count(&bytes) > 1,
            "positive control: the app holds the bytes while it lives"
        );
        assert_eq!(
            doc_statics(&app, "Ownad"),
            vec![bytes.as_ptr() as usize],
            "positive control: text resolves to the owned bytes themselves"
        );
    }
    assert_eq!(
        std::sync::Arc::strong_count(&bytes),
        1,
        "every reference rinch took must be gone with the app"
    );
}

/// A text control sized before a face was registered is re-sized from that
/// face (#1177): its width is its primary font's average character width, and
/// the metrics cached on the control are keyed on the document's font
/// generation, which a late registration moves. As wide as in an app that had
/// the face before the mount.
#[test]
fn a_late_registration_resizes_a_text_control() {
    fn input_app(fonts: &[AppFont]) -> RinchApp {
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let input = scope.create_element("input");
            input.set_attribute(
                "style",
                "font: 16px/20px Inter, sans-serif; padding: 0; border: 0",
            );
            root.append_child(&input);
            root
        });
        for font in fonts {
            app.register_app_font(font.clone());
        }
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        app
    }
    fn input_width(app: &RinchApp) -> f32 {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        let (_, n) = d
            .tree
            .nodes
            .iter()
            .find(|(_, n)| n.tag() == Some("input"))
            .expect("the input");
        n.layout.width
    }
    let early = input_app(&[AppFont::new(OTHER_FACE)]);
    // Inter at 16px: 20 × 10.2422 + 53 − 10.2422 → 248 (Chrome 153: 248).
    assert_eq!(input_width(&early), 248.0);
    let mut late = input_app(&[]);
    let before = input_width(&late);
    late.register_app_font(AppFont::new(OTHER_FACE));
    late.resolve_and_repaint(800.0, 600.0);
    assert_ne!(before, 248.0, "the fallback face is not Inter");
    assert_eq!(input_width(&late), 248.0);
}
