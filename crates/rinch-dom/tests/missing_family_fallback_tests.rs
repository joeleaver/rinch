//! A font stack whose every family is missing falls back to `sans-serif` for
//! every character, digits included (#1198).
//!
//! parley puts a cluster that has the Unicode `Emoji` property through a
//! different query from the rest of the text: the stack's own families and
//! then the `emoji` generic, **ahead of** the script fallback. The ASCII
//! digits, `#` and `*` have that property (they are keycap bases), so in a
//! stack that names only a family the host does not have — `font-family:
//! Helvetica` on Linux, or any typo — a digit's query is `[emoji]` and it is
//! drawn from the colour-emoji face, 1.245em wide, while the letters beside
//! it reach the script fallback. Measured on this host at 16px:
//! `0000000000` was 199px under `Helvetica` and 92px under `sans-serif`.
//!
//! A stack that ends in a generic is not affected: the generic's face covers
//! the digit and the emoji face is never asked. So rinch finishes every stack
//! that names no generic with `sans-serif`, which is the face the letters
//! already reached; Chrome 153 does the same with its default font, which on
//! Linux is serif — see `fonts::finish_font_stack` for why rinch keeps
//! `sans-serif`.
//!
//! The fixtures are host-independent: `sans-serif` is set to the bundled
//! Space Grotesk alone and the `emoji` generic to the bundled Inter alone,
//! whose digits are a different width, so the pre-fix routing is visible on a host with no
//! emoji font at all (CI).

use parley::fontique::{Blob, FontInfoOverride, GenericFamily};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

/// `sans-serif` in these documents.
const SANS: &[u8] = include_bytes!("../assets/fonts/SpaceGrotesk-VariableFont_wght.ttf");
/// The `emoji` generic in these documents. Inter is the stand-in because it
/// covers characters with emoji presentation that Space Grotesk lacks (U+2B1C,
/// U+2764), and its digits are narrower than Space Grotesk's.
const EMOJI_STAND_IN: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// A document whose `sans-serif` is Space Grotesk and whose `emoji` generic is
/// Inter. Returns the two faces' blob ids, to recognise their runs by.
fn document() -> (RinchDocument, u64, u64) {
    let mut doc = RinchDocument::new();
    let collection = &mut doc.font_cx.collection;
    let sans_blob = Blob::new(std::sync::Arc::new(SANS));
    let sans_id = sans_blob.id();
    let sans = collection.register_fonts(
        sans_blob,
        Some(FontInfoOverride {
            family_name: Some("ProbeSans1198"),
            ..Default::default()
        }),
    );
    let emoji_blob = Blob::new(std::sync::Arc::new(EMOJI_STAND_IN));
    let emoji_id = emoji_blob.id();
    let emoji = collection.register_fonts(
        emoji_blob,
        Some(FontInfoOverride {
            family_name: Some("ProbeEmoji1198"),
            ..Default::default()
        }),
    );
    // Set, not claimed: the platform's families behind a claim would make
    // what these slots cover depend on the host.
    collection.set_generic_families(GenericFamily::SansSerif, sans.iter().map(|(f, _)| *f));
    collection.set_generic_families(GenericFamily::Emoji, emoji.iter().map(|(f, _)| *f));
    (doc, sans_id, emoji_id)
}

/// An `inline-block` holding `text` in `family`, laid out: its width is the
/// text's advance.
fn block(family: &str, text: &str) -> (RinchDocument, NodeId, u64, u64) {
    let (mut doc, sans, emoji) = document();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        &format!("display: inline-block; font: 40px/48px {family}"),
    );
    let t = doc.create_text(text);
    doc.append_child(div, t);
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);
    (doc, div, sans, emoji)
}

fn width(family: &str, text: &str) -> f32 {
    let (doc, div, _, _) = block(family, text);
    doc.tree.get(div.0).unwrap().layout.width
}

/// The blob id of the face each glyph run of `text` is drawn in.
fn run_faces(family: &str, text: &str) -> (Vec<u64>, u64, u64) {
    let (doc, div, sans, emoji) = block(family, text);
    let faces = faces_of(&doc, div).into_iter().map(|f| f.0).collect();
    (faces, sans, emoji)
}

const MISSING: [&str; 4] = [
    "NoSuchFamily1198",
    "Helvetica1198",
    "'Times 1198'",
    "Courier1198",
];

/// The digits of a stack that names only a missing family are as wide as
/// `sans-serif`'s. Before the fix they were the `emoji` generic's (the Inter
/// stand-in here; Noto Color Emoji on a desktop that has it).
#[test]
fn digits_in_a_missing_only_stack_are_drawn_in_sans_serif() {
    let sans = width("sans-serif", "1111111111");
    assert!(sans > 0.0);
    for family in MISSING {
        assert_eq!(
            width(family, "1111111111"),
            sans,
            "digits under `{family}` against sans-serif"
        );
    }
}

/// The face, not only the width: every run of mixed text is `sans-serif`'s.
#[test]
fn every_run_of_mixed_text_in_a_missing_only_stack_is_the_sans_serif_face() {
    let (faces, sans, emoji) = run_faces("NoSuchFamily1198", "a1b2#3*c");
    assert!(!faces.is_empty());
    assert!(
        faces.iter().all(|f| *f == sans),
        "runs {faces:?}: sans-serif is {sans}, the emoji stand-in {emoji}"
    );
}

/// Letters too: they used to reach the platform's script fallback, which is
/// not the document's `sans-serif` when that has been set or claimed.
#[test]
fn letters_in_a_missing_only_stack_are_as_wide_as_sans_serif() {
    assert_eq!(
        width("NoSuchFamily1198", "abcdefghij"),
        width("sans-serif", "abcdefghij")
    );
}

/// A stack with a generic is untouched: `serif` stays `serif`, so a missing
/// family ahead of it does not pick up `sans-serif` instead.
#[test]
fn a_stack_that_names_a_generic_is_not_finished_with_sans_serif() {
    let (mut doc, _, _) = document();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "font-family: NoSuchFamily1198, serif");
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        doc.tree.get(div.0).unwrap().computed_style.font_family,
        "NoSuchFamily1198, serif"
    );
}

/// An emoji is drawn exactly as it is under `sans-serif`, and — on a host
/// with a colour-emoji face — from a colour face, not a text one.
///
/// Appending `sans-serif` puts the platform's whole `sans-serif` list ahead of
/// the `emoji` generic, as it already is for every stack that names a generic
/// (the theme's default among them), so an emoji-presentation character lands
/// wherever `sans-serif` sends it: on this host 😀 is Segoe UI Emoji, ⬜ and
/// ❤️ a symbol or text face (Chrome draws all three from its colour-emoji
/// face — #1198's follow-up). The first half pins that consistency; the
/// second is the guard against fixing #1198 by pointing the `emoji` slot at a
/// text face, and needs a colour face to discriminate, so it asserts nothing
/// on a host with none (CI) — it prints which way it went.
#[test]
fn an_emoji_is_drawn_as_under_sans_serif_and_in_colour_where_it_can_be() {
    for text in ["\u{1f600}", "\u{2b1c}", "\u{2764}\u{fe0f}"] {
        let missing = run_faces("NoSuchFamily1198", text).0;
        let generic = run_faces("sans-serif", text).0;
        assert_eq!(missing, generic, "{text:?}");
    }
    let (doc, div) = block_on_host("emoji", "\u{1f600}");
    let host_emoji_is_colour = faces_of(&doc, div).iter().all(|f| f.1);
    eprintln!("host emoji generic draws U+1F600 in colour: {host_emoji_is_colour}");
    if host_emoji_is_colour {
        let (doc, div) = block_on_host("NoSuchFamily1198", "\u{1f600}");
        let faces = faces_of(&doc, div);
        assert!(
            !faces.is_empty() && faces.iter().all(|f| f.1),
            "U+1F600 under a missing-only stack: {faces:?}"
        );
    }
}

/// [`block`] in a document with the host's own generics (no stand-ins).
fn block_on_host(family: &str, text: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        &format!("display: inline-block; font: 40px/48px {family}"),
    );
    let t = doc.create_text(text);
    doc.append_child(div, t);
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);
    (doc, div)
}

/// Each glyph run's face: its blob id, and whether it has a colour table.
fn faces_of(doc: &RinchDocument, div: NodeId) -> Vec<(u64, bool)> {
    use skrifa::raw::TableProvider;
    let layout = doc
        .tree
        .get(div.0)
        .and_then(|n| n.text_layout.as_ref())
        .expect("the inline-block is an IFC root");
    let mut faces = Vec::new();
    for line in layout.layout.lines() {
        for item in line.items() {
            if let parley::layout::PositionedLayoutItem::GlyphRun(run) = item {
                let font = run.run().font();
                let face = skrifa::FontRef::from_index(font.data.as_ref(), font.index).unwrap();
                let colour = face.colr().is_ok() || face.cbdt().is_ok() || face.sbix().is_ok();
                faces.push((font.data.id(), colour));
            }
        }
    }
    faces
}

/// End to end on the host: a text control's width (#1196, measured from the
/// face that draws `0`) under a missing-only stack is its width under
/// `sans-serif`. It was 400px for `font-family: Helvetica` on a host with
/// Noto Color Emoji before #1196 measured an `x` instead; with the stack
/// finished it holds whichever character is measured.
#[test]
fn a_text_control_in_a_missing_only_stack_is_as_wide_as_under_sans_serif() {
    let input = |family: &str| {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = doc.create_element("input");
        doc.set_attribute(
            c,
            "style",
            &format!("font: 16px/20px {family}; padding: 0; border: 0"),
        );
        doc.append_child(body, c);
        doc.resolve_layout(800.0, 600.0);
        doc.tree.get(c.0).unwrap().layout.width
    };
    let sans = input("sans-serif");
    assert!(sans > 0.0);
    for family in ["Helvetica", "NoSuchFamily1198"] {
        assert_eq!(input(family), sans, "input under `{family}`");
    }
}
