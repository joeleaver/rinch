//! A font stack that resolves to no family ends in the primary `sans-serif`
//! face, so its digits are drawn where its letters are (#1198).
//!
//! parley puts a cluster that has the Unicode `Emoji` property through a
//! different query from the rest of the text: the stack's own families and
//! then the `emoji` generic, **ahead of** the script fallback. The ASCII
//! digits, `#` and `*` have that property (they are keycap bases), so in a
//! stack that names only a family the host does not have — `font-family:
//! Helvetica` on Linux, or any typo — a digit's query was `[emoji]` and it was
//! drawn from the colour-emoji face, 1.245em wide, while the letters beside
//! it reached the script fallback. Measured on this host at 16px:
//! `0000000000` was 199px under `Helvetica` and 92px under `sans-serif`.
//!
//! `fonts::parley_font_family` appends the slot's first **face** by name, not
//! the `sans-serif` generic, whose platform list holds text faces that cover
//! emoji; see its doc for the accepted consequence on a DejaVu-primary host.
//!
//! The stand-in fixtures are host-independent: `sans-serif` is headed by the
//! bundled Space Grotesk and the `emoji` generic by the bundled Inter, whose
//! digits are a different width, so the pre-fix routing is visible on a host
//! with no emoji font at all (CI). The fixtures built on `block_on_host` use the host's
//! own fonts and say what they could check.

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

/// An emoji the appended face lacks still reaches the `emoji` generic, and an
/// emoji in a stack that resolves is untouched.
///
/// rinch appends the primary `sans-serif` **face**, not the generic: the
/// generic expands to the platform's whole list, whose text faces (DejaVu
/// Sans, FreeSans) would win an emoji ahead of the `emoji` generic. Here the
/// primary face is Space Grotesk, which lacks U+2B1C and U+2764, and the
/// `emoji` generic's first face is Inter, which has both. Appending the
/// `sans-serif` generic instead fails this on a host whose platform list
/// covers them (this one: FreeSans and DejaVu Sans).
#[test]
fn an_emoji_the_appended_face_lacks_still_reaches_the_emoji_generic() {
    use skrifa::MetadataProvider;
    let sans = skrifa::FontRef::new(SANS).unwrap().charmap();
    let emoji = skrifa::FontRef::new(EMOJI_STAND_IN).unwrap().charmap();
    for text in ["\u{2b1c}", "\u{2764}\u{fe0f}"] {
        let c = text.chars().next().unwrap();
        assert!(sans.map(c).is_none() && emoji.map(c).is_some());
        // Missing-only (finished) and resolving (left alone) stacks alike.
        for family in ["NoSuchFamily1198", "ProbeSans1198"] {
            let (faces, sans_id, emoji_id) = run_faces(family, text);
            assert_eq!(
                faces,
                vec![emoji_id],
                "{text:?} under `{family}`: sans-serif is {sans_id}, emoji {emoji_id}"
            );
        }
    }
}

/// On the host's own fonts, both branches of the rule, whichever this host
/// takes. Where the `emoji` generic draws U+1F600 in colour:
///
/// - the primary `sans-serif` face lacks U+1F600 (Noto Sans, Roboto): it is
///   drawn in colour under a missing-only stack and under a stack naming the
///   primary face;
/// - the primary face has its own U+1F600 (DejaVu Sans, a monochrome one): it
///   is drawn in that face under both stacks — the accepted consequence
///   (#1204), which every `sans-serif` stack already draws there.
///
/// On a host with no colour emoji face it asserts nothing and says so. The
/// second branch is also pinned host-independently by
/// `an_emoji_the_appended_face_has_is_drawn_in_it`.
#[test]
fn a_host_emoji_follows_the_primary_face_under_a_missing_only_stack() {
    let (doc, div) = block_on_host("emoji", "\u{1f600}");
    let host_emoji_is_colour = faces_of(&doc, div).iter().all(|f| f.1);
    let mut fcx = rinch_dom::fonts::new_font_context();
    let primary = fcx
        .collection
        .generic_families(GenericFamily::SansSerif)
        .next();
    let primary = primary.and_then(|id| fcx.collection.family_name(id).map(str::to_owned));
    eprintln!("host: emoji generic colour {host_emoji_is_colour}, primary sans {primary:?}");
    let (Some(primary), true) = (primary, host_emoji_is_colour) else {
        return;
    };
    let named = format!("'{primary}'");
    let (doc, div) = block_on_host(&named, "\u{1f600}");
    let primary_has_it = faces_of(&doc, div).iter().any(|f| f.2 == primary);
    eprintln!("host: primary sans has its own U+1F600: {primary_has_it}");
    for family in [named.as_str(), "NoSuchFamily1198"] {
        let (doc, div) = block_on_host(family, "\u{1f600}");
        let faces = faces_of(&doc, div);
        assert!(!faces.is_empty(), "U+1F600 under `{family}`");
        if primary_has_it {
            assert!(
                faces.iter().all(|f| f.2 == primary),
                "U+1F600 under `{family}` in the primary face {primary}: {faces:?}"
            );
        } else {
            assert!(
                faces.iter().all(|f| f.1),
                "U+1F600 under `{family}` in colour: {faces:?}"
            );
        }
    }
}

/// The DejaVu-primary case on bundled fonts: when the primary `sans-serif`
/// face has an emoji of its own, a missing-only stack draws it in that face,
/// not the `emoji` generic's — the accepted consequence (#1204), identical to
/// what a stack naming the face draws. Inter plays both roles here, from two
/// separately registered copies, so the two faces are told apart by blob.
#[test]
fn an_emoji_the_appended_face_has_is_drawn_in_it() {
    let mut doc = RinchDocument::new();
    let collection = &mut doc.font_cx.collection;
    let mut register = |name: &'static str| {
        let blob = Blob::new(std::sync::Arc::new(EMOJI_STAND_IN));
        let id = blob.id();
        let fams = collection.register_fonts(
            blob,
            Some(FontInfoOverride {
                family_name: Some(name),
                ..Default::default()
            }),
        );
        (id, fams[0].0)
    };
    let (sans_blob, sans) = register("PrimaryWithEmoji1198");
    let (emoji_blob, emoji) = register("EmojiCopy1198");
    assert_ne!(sans_blob, emoji_blob);
    collection.set_generic_families(GenericFamily::SansSerif, core::iter::once(sans));
    collection.set_generic_families(GenericFamily::Emoji, core::iter::once(emoji));
    let body = doc.body();
    let mut divs = Vec::new();
    for family in ["NoSuchFamily1198", "PrimaryWithEmoji1198"] {
        let div = doc.create_element("div");
        doc.set_attribute(
            div,
            "style",
            &format!("display: inline-block; font: 40px/48px {family}"),
        );
        let t = doc.create_text("\u{2b1c}");
        doc.append_child(div, t);
        doc.append_child(body, div);
        divs.push((family, div));
    }
    doc.resolve_layout(800.0, 600.0);
    for (family, div) in divs {
        let faces: Vec<u64> = faces_of(&doc, div).into_iter().map(|f| f.0).collect();
        assert_eq!(
            faces,
            vec![sans_blob],
            "U+2B1C under `{family}`: primary {sans_blob}, emoji {emoji_blob}"
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

/// Each glyph run's face: its blob id, whether it has a colour table, and its
/// family name.
fn faces_of(doc: &RinchDocument, div: NodeId) -> Vec<(u64, bool, String)> {
    use skrifa::MetadataProvider;
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
                let name = face
                    .localized_strings(skrifa::string::StringId::FAMILY_NAME)
                    .english_or_first()
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                faces.push((font.data.id(), colour, name));
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

/// The issue's own case on the host's fonts: digits in `font-family:
/// Helvetica` (and `Times`, `Courier`) measure as `sans-serif`'s, wherever
/// fontique does not have the family (it has no fontconfig aliases). Was
/// 199px against 92px for ten zeros at 16px on this host.
#[test]
fn digits_in_an_uninstalled_named_family_measure_as_sans_serif_on_the_host() {
    let width_on_host = |family: &str| {
        let (doc, div) = block_on_host(family, "0000000000");
        doc.tree.get(div.0).unwrap().layout.width
    };
    let sans = width_on_host("sans-serif");
    let mut fcx = rinch_dom::fonts::new_font_context();
    let mut asserted = 0;
    for family in ["Helvetica", "Times", "Courier", "NoSuchFamily1198"] {
        if fcx.collection.family_by_name(family).is_some() {
            continue;
        }
        assert_eq!(width_on_host(family), sans, "`{family}`");
        asserted += 1;
    }
    assert!(asserted > 0, "NoSuchFamily1198 is never installed");
}

/// Every parley layout rinch builds takes its family from
/// `fonts::parley_font_family`, so a new site cannot hand parley a raw stack
/// and bring #1198 back. Scans the non-test sources of `rinch-dom` and
/// `rinch` for a family built any other way.
#[test]
fn no_source_builds_a_parley_font_family_by_hand() {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for krate in ["../rinch-dom/src", "../rinch/src"] {
        walk(&manifest.join(krate), &mut files);
    }
    assert!(files.len() > 50, "the scan found the sources");
    let mut offenders = Vec::new();
    for file in &files {
        let name = file.file_name().unwrap().to_string_lossy();
        if name.ends_with("_tests.rs") || name == "tests.rs" || name == "fonts.rs" {
            continue;
        }
        let text = std::fs::read_to_string(file).unwrap();
        // Stop at an in-file test module: fixtures may name a family directly.
        let text = text.split("#[cfg(test)]").next().unwrap();
        for (i, line) in text.lines().enumerate() {
            if line.contains("FontFamily::Source(") || line.contains("FontFamily::Single(") {
                offenders.push(format!("{}:{}: {}", file.display(), i + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "build the family with fonts::parley_font_family:\n{}",
        offenders.join("\n")
    );
}
