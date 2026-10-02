//! An emoji-presentation character is drawn from the `emoji` generic ahead of
//! the text faces a generic in its stack expands to (#1204).
//!
//! parley queries a cluster with the Unicode `Emoji` property against the
//! stack's families and then the `emoji` generic. A generic such as
//! `sans-serif` expands to the platform's whole list (fontconfig's trimmed
//! sort list: about 180 families on a Linux desktop), whose text faces —
//! DejaVu Sans, FreeSans — cover many emoji, so U+1F600 under `sans-serif`
//! was drawn in DejaVu's monochrome glyph, and U+2B1C in FreeSans's.
//!
//! Chrome 153 on the same host (`CSS.getPlatformFontsForNode`):
//!
//! | stack | U+1F600, U+2B1C, U+2764 U+FE0F | U+2764 | `123#` | U+2B1C U+FE0E |
//! |---|---|---|---|---|
//! | `sans-serif` | Noto Color Emoji | DejaVu Sans | Arial | text face |
//! | `'DejaVu Sans'` | DejaVu Sans (U+1F600), Noto Color Emoji (the others) | DejaVu Sans | DejaVu Sans | text face |
//! | the theme's `…, sans-serif, 'Apple Color Emoji', 'Segoe UI Emoji'` | Segoe UI Emoji | Segoe UI Emoji | Segoe UI | text face |
//!
//! So a generic in the stack stands for its one primary face, and a named
//! family keeps its place: a named face that covers an emoji draws it
//! (`'DejaVu Sans'` above). An emoji-presentation cluster (Emoji_Presentation,
//! or followed by U+FE0F, and not by U+FE0E) is shaped with the stack whose
//! generics are replaced by their primary face; a text-default `Emoji`
//! character (U+2764, the digits, `#`) is left to the stack as written, so
//! #1198's digits stay where they were.
//!
//! The stand-ins are bundled, so nothing here depends on the host's fonts:
//! `sans-serif` is Space Grotesk (no U+2B1C, no U+2764) followed by a copy of
//! Inter (which has both) playing DejaVu Sans; the `emoji` generic is a
//! second copy of Inter, and a third copy is a named family in no generic,
//! playing Segoe UI Emoji. The copies are told apart by blob id.

use parley::fontique::{Blob, FontInfoOverride, GenericFamily};
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const SPACE_GROTESK: &[u8] = include_bytes!("../assets/fonts/SpaceGrotesk-VariableFont_wght.ttf");
const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// The blob ids of the four faces.
#[derive(Clone, Copy, Debug)]
struct Faces {
    /// `sans-serif`'s primary face; covers neither U+2B1C nor U+2764.
    primary: u64,
    /// `sans-serif`'s second face; covers both (DejaVu Sans's part).
    text_cover: u64,
    /// The `emoji` generic.
    emoji: u64,
    /// A named family in no generic (Segoe UI Emoji's part).
    named_emoji: u64,
}

fn register(
    doc: &mut RinchDocument,
    bytes: &'static [u8],
    name: &'static str,
) -> (u64, parley::fontique::FamilyId) {
    let blob = Blob::new(std::sync::Arc::new(bytes));
    let id = blob.id();
    let fams = doc.font_cx.collection.register_fonts(
        blob,
        Some(FontInfoOverride {
            family_name: Some(name),
            ..Default::default()
        }),
    );
    (id, fams[0].0)
}

fn document() -> (RinchDocument, Faces) {
    let mut doc = RinchDocument::new();
    let (primary, primary_f) = register(&mut doc, SPACE_GROTESK, "Primary1204");
    let (text_cover, text_cover_f) = register(&mut doc, INTER, "TextCover1204");
    let (emoji, emoji_f) = register(&mut doc, INTER, "EmojiStandIn1204");
    let (named_emoji, _) = register(&mut doc, INTER, "NamedEmoji1204");
    let collection = &mut doc.font_cx.collection;
    // Set, not claimed: the platform's families behind a claim would make
    // what the slots cover depend on the host.
    collection.set_generic_families(
        GenericFamily::SansSerif,
        [primary_f, text_cover_f].into_iter(),
    );
    collection.set_generic_families(GenericFamily::Emoji, core::iter::once(emoji_f));
    (
        doc,
        Faces {
            primary,
            text_cover,
            emoji,
            named_emoji,
        },
    )
}

/// The blob id of each glyph run of `layout`, with the run's text.
fn runs(layout: &parley::Layout<peniko::Brush>, text: &str) -> Vec<(u64, String)> {
    let mut out = Vec::new();
    for line in layout.lines() {
        for item in line.items() {
            if let parley::layout::PositionedLayoutItem::GlyphRun(run) = item {
                let r = run.run();
                let range = r.text_range();
                out.push((r.font().data.id(), text[range].to_owned()));
            }
        }
    }
    out
}

/// `text` in an `inline-block` (an IFC root) under `family`.
fn ifc_runs(family: &str, text: &str) -> (Vec<(u64, String)>, Faces) {
    let (mut doc, faces) = document();
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
    let layout = &doc
        .tree
        .get(div.0)
        .and_then(|n| n.text_layout.as_ref())
        .expect("the inline-block is an IFC root")
        .layout;
    (runs(layout, text), faces)
}

fn faces_only(runs: &[(u64, String)]) -> Vec<u64> {
    runs.iter().map(|r| r.0).collect()
}

/// The issue's case: under `sans-serif`, an emoji-presentation character goes
/// to the `emoji` generic, not to a text face further down `sans-serif`'s
/// list. At main it was drawn in the second `sans-serif` face.
#[test]
fn an_emoji_presentation_character_under_a_generic_goes_to_the_emoji_generic() {
    for text in ["\u{2b1c}", "\u{2764}\u{fe0f}"] {
        let (runs, f) = ifc_runs("sans-serif", text);
        assert_eq!(faces_only(&runs), vec![f.emoji], "{text:?}: {runs:?} {f:?}");
    }
}

/// The text-default `Emoji` characters keep the stack as written: U+2764
/// alone is text in Chrome (DejaVu Sans there), and the digits and `#` stay in
/// the primary face (#1198).
#[test]
fn text_default_emoji_characters_keep_the_stack_as_written() {
    let (runs, f) = ifc_runs("sans-serif", "\u{2764}");
    assert_eq!(faces_only(&runs), vec![f.text_cover], "{runs:?} {f:?}");
    let (runs, f) = ifc_runs("sans-serif", "12#*");
    assert_eq!(faces_only(&runs), vec![f.primary], "{runs:?} {f:?}");
}

/// U+FE0E asks for text presentation: an Emoji_Presentation character
/// followed by it is shaped with the stack as written.
#[test]
fn a_text_variation_selector_keeps_the_stack_as_written() {
    let (runs, f) = ifc_runs("sans-serif", "\u{2b1c}\u{fe0e}");
    assert_eq!(faces_only(&runs), vec![f.text_cover], "{runs:?} {f:?}");
}

/// Text around an emoji keeps its own face: only the emoji's cluster moves.
#[test]
fn only_the_emoji_cluster_moves() {
    let (runs, f) = ifc_runs("sans-serif", "ab\u{2b1c}cd\u{2764}");
    assert_eq!(
        runs,
        vec![
            (f.primary, "ab".to_owned()),
            (f.emoji, "\u{2b1c}".to_owned()),
            (f.primary, "cd".to_owned()),
            (f.text_cover, "\u{2764}".to_owned()),
        ],
        "{f:?}"
    );
}

/// The theme's shape: a named emoji family after the generic wins the emoji,
/// as Segoe UI Emoji does in Chrome under `DEFAULT_FONT_FAMILY`.
#[test]
fn a_named_family_after_the_generic_wins_the_emoji() {
    let (runs, f) = ifc_runs("Missing1204, sans-serif, NamedEmoji1204", "\u{2b1c}");
    assert_eq!(faces_only(&runs), vec![f.named_emoji], "{runs:?} {f:?}");
}

/// A named family keeps its place: one that covers the emoji draws it, as
/// `'DejaVu Sans'` draws U+1F600 in Chrome.
#[test]
fn a_named_family_that_covers_the_emoji_keeps_it() {
    let (runs, f) = ifc_runs("TextCover1204, sans-serif", "\u{2b1c}");
    assert_eq!(faces_only(&runs), vec![f.text_cover], "{runs:?} {f:?}");
}

/// The same rule on a flex item's text leaf, which is shaped by its measure
/// rather than by an IFC.
#[test]
fn a_flex_item_text_leaf_takes_the_same_rule() {
    let (mut doc, f) = document();
    let body = doc.body();
    let flex = doc.create_element("div");
    doc.set_attribute(flex, "style", "display: flex; font: 40px/48px sans-serif");
    let text = "a\u{2b1c}";
    let t = doc.create_text(text);
    doc.append_child(flex, t);
    doc.append_child(body, flex);
    doc.resolve_layout(800.0, 600.0);
    let node = doc.tree.get(t.0).unwrap();
    assert!(node.ifc_root.is_none(), "a text leaf, not an IFC member");
    let layout = node.cached_text_parley.as_ref().expect("the leaf's layout");
    assert_eq!(
        runs(layout, text),
        vec![
            (f.primary, "a".to_owned()),
            (f.emoji, "\u{2b1c}".to_owned())
        ],
        "{f:?}"
    );
}

/// The `text-overflow: ellipsis` rebuild shapes the cut line with the same
/// families as the line it replaces.
#[test]
fn the_ellipsis_rebuild_takes_the_same_rule() {
    let (mut doc, f) = document();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "width: 100px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; \
         font: 40px/48px sans-serif",
    );
    let text = "\u{2b1c}\u{2b1c}\u{2b1c}\u{2b1c}\u{2b1c}\u{2b1c}";
    let t = doc.create_text(text);
    doc.append_child(div, t);
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);
    let il = doc
        .tree
        .get(div.0)
        .and_then(|n| n.text_layout.as_ref())
        .expect("an IFC root");
    let mut faces = Vec::new();
    let mut chars = String::new();
    for line in il.layout.lines() {
        for item in line.items() {
            if let parley::layout::PositionedLayoutItem::GlyphRun(run) = item {
                faces.push(run.run().font().data.id());
                chars.push_str(&format!("{} glyphs; ", run.glyphs().count()));
            }
        }
    }
    assert!(!faces.is_empty());
    // The "…" itself is text (U+2026 has no Emoji property); every square is
    // the emoji face. Space Grotesk has the ellipsis, so it is the primary.
    assert_eq!(faces.first(), Some(&f.emoji), "{faces:?} {chars} {f:?}");
    assert!(
        faces.iter().all(|x| *x == f.emoji || *x == f.primary),
        "{faces:?} {f:?}"
    );
    assert!(!faces.contains(&f.text_cover), "{faces:?} {f:?}");
}
