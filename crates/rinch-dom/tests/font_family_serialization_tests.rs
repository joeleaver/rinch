//! A computed `font-family` reaches parley as the list the author wrote (#1223).
//!
//! rinch keeps a computed `font-family` as a CSS-list string, and parley
//! re-parses it (`FontFamilyName::parse_css_list`, stopping at the first
//! parse error). The string used to be written with every family name
//! **unquoted**, so a name that is not a valid bare list item broke the parse
//! of everything after it:
//!
//! - `"", Foo` computed to `, Foo`, which fails at byte 0: nothing resolved,
//!   even with `Foo` installed;
//! - `"\"Quoted", Foo` computed to `"Quoted, Foo`, an unterminated string;
//! - `NoSuch, ""` computed to `NoSuch, `, so the face #1198 appends to a stack
//!   that resolves to nothing sat behind a `, ,` error;
//! - `"A, B"` was split into two names, and `"serif"` became the generic.
//!
//! In every broken case the digits reached the `emoji` generic, as in #1198.
//!
//! The documents are host-independent: `sans-serif` is the bundled Space
//! Grotesk, the `emoji` generic is the bundled Inter, and the named family the
//! stacks ask for is a second registration of Inter under its own blob, so
//! each run's face is told apart by blob id.

use parley::fontique::{Blob, FontInfoOverride, GenericFamily};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const SANS: &[u8] = include_bytes!("../assets/fonts/SpaceGrotesk-VariableFont_wght.ttf");
const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

struct Faces {
    sans: u64,
    emoji: u64,
    named: u64,
}

/// `sans-serif` (and `serif`) are Space Grotesk, `emoji` is Inter, and
/// `named` is another copy of Inter registered as the family `named`.
fn document(named: &str) -> (RinchDocument, Faces) {
    let mut doc = RinchDocument::new();
    let collection = &mut doc.font_cx.collection;
    let mut register = |data: &'static [u8], name: &str| {
        let blob = Blob::new(std::sync::Arc::new(data));
        let id = blob.id();
        let families = collection.register_fonts(
            blob,
            Some(FontInfoOverride {
                family_name: Some(name),
                ..Default::default()
            }),
        );
        (id, families.into_iter().map(|(f, _)| f).collect::<Vec<_>>())
    };
    let (sans, sans_families) = register(SANS, "ProbeSans1223");
    let (emoji, emoji_families) = register(INTER, "ProbeEmoji1223");
    let (named_id, _) = register(INTER, named);
    collection.set_generic_families(GenericFamily::SansSerif, sans_families.iter().copied());
    collection.set_generic_families(GenericFamily::Serif, sans_families.iter().copied());
    collection.set_generic_families(GenericFamily::Emoji, emoji_families.iter().copied());
    (
        doc,
        Faces {
            sans,
            emoji,
            named: named_id,
        },
    )
}

/// The blob id of each glyph run of `text` in an `inline-block` whose
/// `font-family` declaration is `family`, in a document whose named family
/// is `named`.
fn run_faces(named: &str, family: &str, text: &str) -> (Vec<u64>, Faces) {
    let (mut doc, faces) = document(named);
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        &format!(
            "display: inline-block; font-size: 40px; line-height: 48px; font-family: {family}"
        ),
    );
    let t = doc.create_text(text);
    doc.append_child(div, t);
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);
    (blobs(&doc, div), faces)
}

fn blobs(doc: &RinchDocument, div: NodeId) -> Vec<u64> {
    let layout = doc
        .tree
        .get(div.0)
        .and_then(|n| n.text_layout.as_ref())
        .expect("the inline-block is an IFC root");
    let mut out = Vec::new();
    for line in layout.layout.lines() {
        for item in line.items() {
            if let parley::layout::PositionedLayoutItem::GlyphRun(run) = item {
                out.push(run.run().font().data.id());
            }
        }
    }
    out
}

/// Every run of `text` under `family` is drawn in the face `pick` names.
fn assert_all(named: &str, family: &str, pick: fn(&Faces) -> u64) {
    let (runs, faces) = run_faces(named, family, "a1b2#3*c");
    assert!(!runs.is_empty());
    let want = pick(&faces);
    assert!(
        runs.iter().all(|r| *r == want),
        "`font-family: {family}`: runs {runs:?}, want {want} \
         (sans {}, emoji {}, named {})",
        faces.sans,
        faces.emoji,
        faces.named
    );
}

/// An empty family name before an installed one is skipped, not a parse
/// error that loses the whole stack.
#[test]
fn an_empty_family_name_does_not_hide_the_family_after_it() {
    assert_all("Foo1223", "\"\", Foo1223", |f| f.named);
}

/// A family name that starts with a quote character is written in the other
/// quote, not left as an unterminated string.
#[test]
fn a_quote_led_family_name_does_not_hide_the_family_after_it() {
    assert_all("Foo1223", "\"\\\"Quoted\", Foo1223", |f| f.named);
    assert_all("Foo1223", "'\\'Quoted', Foo1223", |f| f.named);
}

/// A trailing empty name in a stack that resolves to nothing leaves the
/// appended `sans-serif` face (#1198) reachable.
#[test]
fn a_trailing_empty_family_name_keeps_the_appended_sans_serif_face() {
    assert_all("Foo1223", "NoSuchFamily1223, \"\"", |f| f.sans);
}

/// A family name holding a comma is one name.
#[test]
fn a_family_name_with_a_comma_is_one_family() {
    assert_all("Comma, 1223", "\"Comma, 1223\"", |f| f.named);
}

/// A quoted `"serif"` is a family named `serif`, not the generic (here the
/// generic is the sans face).
#[test]
fn a_quoted_generic_keyword_is_a_family_name() {
    assert_all("serif", "\"serif\"", |f| f.named);
    // And the bare keyword is still the generic.
    assert_all("serif", "serif", |f| f.sans);
}

/// The computed value reads back as a CSS list parley parses whole.
#[test]
fn the_computed_family_parses_back_to_the_authored_names() {
    use parley::style::FontFamilyName;
    let (mut doc, _) = document("Foo1223");
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "font-family: \"\", 'A, B', \"'x\", \"serif\", Foo1223, monospace, \"\"",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);
    let stack = doc
        .tree
        .get(div.0)
        .unwrap()
        .computed_style
        .font_family
        .clone();
    let parsed: Vec<_> = FontFamilyName::parse_css_list(&stack)
        .map(|r| r.map(FontFamilyName::into_owned))
        .collect::<Result<_, _>>()
        .unwrap_or_else(|e| panic!("`{stack}` does not parse: {e}"));
    assert_eq!(
        parsed,
        vec![
            FontFamilyName::named("A, B").into_owned(),
            FontFamilyName::named("'x").into_owned(),
            FontFamilyName::named("serif").into_owned(),
            FontFamilyName::named("Foo1223").into_owned(),
            FontFamilyName::Generic(GenericFamily::Monospace),
        ],
        "computed `{stack}`"
    );
}
