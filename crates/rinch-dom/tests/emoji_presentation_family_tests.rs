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

/// The same rule on a text leaf inside an `inline-flex` (a `Button` label),
/// which the detached atomic-inline compute shapes.
#[test]
fn an_inline_flex_text_leaf_takes_the_same_rule() {
    let (mut doc, f) = document();
    let body = doc.body();
    let block = doc.create_element("div");
    let chip = doc.create_element("span");
    doc.set_attribute(
        chip,
        "style",
        "display: inline-flex; font: 40px/48px sans-serif",
    );
    let text = "a\u{2b1c}";
    let t = doc.create_text(text);
    doc.append_child(chip, t);
    doc.append_child(block, chip);
    doc.append_child(body, block);
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

/// Every ranged parley layout rinch builds pushes its families through
/// `fonts::TextFamily`, so a new site cannot shape an emoji in a generic's
/// text face again. Scans the non-test sources of `rinch-dom` and `rinch` for
/// a `FontFamily` style property pushed anywhere else; the IFC's emoji span
/// is the one allowed outside `fonts.rs`.
#[test]
fn no_source_pushes_a_font_family_by_hand() {
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
    let mut allowed = 0;
    for file in &files {
        let name = file.file_name().unwrap().to_string_lossy();
        if name.ends_with("_tests.rs") || name == "tests.rs" || name == "fonts.rs" {
            continue;
        }
        let text = std::fs::read_to_string(file).unwrap();
        let text = text.split("#[cfg(test)]").next().unwrap();
        for (i, line) in text.lines().enumerate() {
            if line.contains("StyleProperty::FontFamily(") || line.contains("P::FontFamily(") {
                if name == "ifc.rs" && line.contains("emoji_span") {
                    allowed += 1;
                    continue;
                }
                offenders.push(format!("{}:{}: {}", file.display(), i + 1, line.trim()));
            }
        }
    }
    assert_eq!(
        allowed, 1,
        "the IFC's emoji span is where the scan expects it"
    );
    assert!(offenders.is_empty(), "{offenders:#?}");
}

/// On the host's own fonts: under `sans-serif` and under the theme's default
/// stack, U+1F600 is drawn in the face the `emoji` generic draws it in, unless
/// a face that stack names (or a generic's primary face) covers it. At main,
/// on a Linux desktop with Noto Color Emoji, it was DejaVu Sans. On a host
/// with no emoji face for it (CI) this asserts nothing and says so.
#[test]
fn on_the_host_u1f600_under_sans_serif_is_the_emoji_generics() {
    let face_of = |family: &str| {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let div = doc.create_element("div");
        doc.set_attribute(
            div,
            "style",
            &format!("display: inline-block; font: 40px/48px {family}"),
        );
        let t = doc.create_text("\u{1f600}");
        doc.append_child(div, t);
        doc.append_child(body, div);
        doc.resolve_layout(800.0, 600.0);
        let il = doc.tree.get(div.0).unwrap().text_layout.as_ref().unwrap();
        let mut names = Vec::new();
        for line in il.layout.lines() {
            for item in line.items() {
                if let parley::layout::PositionedLayoutItem::GlyphRun(run) = item {
                    use skrifa::MetadataProvider;
                    let font = run.run().font();
                    let face = skrifa::FontRef::from_index(font.data.as_ref(), font.index).unwrap();
                    names.push(
                        face.localized_strings(skrifa::string::StringId::FAMILY_NAME)
                            .english_or_first()
                            .map(|s| s.to_string())
                            .unwrap_or_default(),
                    );
                }
            }
        }
        names
    };
    let mut fcx = rinch_dom::fonts::new_font_context();
    let primary = fcx
        .collection
        .generic_families(GenericFamily::SansSerif)
        .next();
    let primary = primary.and_then(|id| fcx.collection.family_name(id).map(str::to_owned));
    let Some(primary) = primary else {
        eprintln!("host: no sans-serif face, nothing to assert");
        return;
    };
    // The emoji cluster's stack under `sans-serif` is the primary face and
    // then the `emoji` generic — what a stack naming the primary face draws.
    let named = face_of(&format!("'{primary}'"));
    let sans = face_of("sans-serif");
    eprintln!(
        "host: U+1F600 under sans-serif {sans:?}, under '{primary}' {named:?}, emoji {:?}",
        face_of("emoji")
    );
    assert!(!sans.is_empty());
    assert_eq!(sans, named, "under sans-serif against '{primary}'");
}

/// Review #1270: every emoji range of one text op gets the span, not only the
/// first (kills "break after the first range" in `IfcText::finish`).
#[test]
fn review_1270_two_emoji_in_one_text_node_both_move() {
    let (runs, f) = ifc_runs("sans-serif", "a\u{2b1c}b\u{2b1c}");
    assert_eq!(
        runs,
        vec![
            (f.primary, "a".to_owned()),
            (f.emoji, "\u{2b1c}".to_owned()),
            (f.primary, "b".to_owned()),
            (f.emoji, "\u{2b1c}".to_owned()),
        ],
        "{f:?}"
    );
}

/// Review #1270: the emoji split happens after white-space collapse and
/// across an inline element, and leaves the flat text (and so every byte
/// offset) exactly as it was.
#[test]
fn review_1270_collapsed_text_and_an_inline_span_keep_their_bytes() {
    let (mut doc, f) = document();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "display: inline-block; font: 40px/48px sans-serif",
    );
    let t1 = doc.create_text("  a   \u{2b1c}   ");
    let span = doc.create_element("span");
    let t2 = doc.create_text("\u{2b1c}b");
    doc.append_child(span, t2);
    doc.append_child(div, t1);
    doc.append_child(div, span);
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);
    let il = doc.tree.get(div.0).unwrap().text_layout.as_ref().unwrap();
    let expected = "a \u{2b1c} \u{2b1c}b";
    let r = runs(&il.layout, expected);
    let joined: String = r.iter().map(|x| x.1.as_str()).collect();
    assert_eq!(joined, expected);
    assert_eq!(
        r.iter()
            .filter(|x| x.1.contains('\u{2b1c}'))
            .map(|x| x.0)
            .collect::<Vec<_>>(),
        vec![f.emoji, f.emoji],
        "{r:?} {f:?}"
    );
}

/// U+FE0F after a base with no `Emoji` property is not an emoji (UTS #51:
/// only emoji variation sequences take it), so the cluster keeps the stack as
/// written: `Ж` U+FE0F is not drawn from the `emoji` generic. Review of #1270: 𝄞 U+FE0F and Thai ก U+FE0F
/// were drawn as `.notdef` on this host when every such cluster was moved.
#[test]
fn a_selector_after_a_non_emoji_base_keeps_the_stack_as_written() {
    use skrifa::MetadataProvider;
    let sg = skrifa::FontRef::new(SPACE_GROTESK).unwrap().charmap();
    let inter = skrifa::FontRef::new(INTER).unwrap().charmap();
    assert!(sg.map('\u{416}').is_none() && inter.map('\u{416}').is_some());
    // Shaped as text, parley wants a face for the selector too, which Inter
    // lacks, so the face that answers is the host's. The rule itself is pinned
    // by `fonts::tests::emoji_presentation::a_selector_after_a_non_emoji_base_is_text`;
    // this is the end-to-end shape (an emoji span would not reach the
    // `emoji` stand-in either, since it lacks U+FE0F too).
    let (runs, f) = ifc_runs("sans-serif", "\u{416}\u{fe0f}");
    assert!(!runs.is_empty());
    assert!(!faces_only(&runs).contains(&f.emoji), "{runs:?} {f:?}");
}

/// Review of #1270: an emoji span keeps the rest of each generic behind the
/// `emoji` generic. A context with no system fonts and no script fallback
/// (embed or wasm with app fonts) and no `emoji` face, whose `sans-serif` is
/// [Space Grotesk, Inter]: U+2B1C and U+2764 U+FE0F are Inter's, never glyph 0
/// in Space Grotesk, which is what the first face alone gave.
#[test]
fn an_emoji_falls_back_to_the_rest_of_the_generic_not_to_notdef() {
    use parley::fontique::{Collection, CollectionOptions};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: false,
    });
    let (primary, primary_f) = register(&mut doc, SPACE_GROTESK, "Primary1204");
    let (second, second_f) = register(&mut doc, INTER, "Second1204");
    doc.font_cx
        .collection
        .set_generic_families(GenericFamily::SansSerif, [primary_f, second_f].into_iter());
    let body = doc.body();
    let mut divs = Vec::new();
    for text in ["\u{2b1c}", "x\u{2764}\u{fe0f}"] {
        let div = doc.create_element("div");
        doc.set_attribute(
            div,
            "style",
            "display: inline-block; font: 40px/48px sans-serif",
        );
        let t = doc.create_text(text);
        doc.append_child(div, t);
        doc.append_child(body, div);
        divs.push((text, div));
    }
    doc.resolve_layout(800.0, 600.0);
    for (text, div) in divs {
        let il = doc.tree.get(div.0).unwrap().text_layout.as_ref().unwrap();
        let mut seen = Vec::new();
        for line in il.layout.lines() {
            for item in line.items() {
                if let parley::layout::PositionedLayoutItem::GlyphRun(run) = item {
                    let id = run.run().font().data.id();
                    for g in run.glyphs() {
                        assert_ne!(g.id, 0, "{text:?}: .notdef in {id} (primary {primary})");
                    }
                    seen.push(id);
                }
            }
        }
        assert_eq!(
            seen.last(),
            Some(&second),
            "{text:?}: {seen:?}, primary {primary}"
        );
    }
}
