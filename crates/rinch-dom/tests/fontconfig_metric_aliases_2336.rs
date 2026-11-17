//! fontconfig's substitutes for a missing family, judged as Chrome judges
//! them (#2336), and the synthetic-bold rule for a system face (#2343).
//!
//! On a Linux host with Liberation and no msttcorefonts, Chrome 156 draws
//! `Arial` in Liberation Sans, `Times New Roman` (so `serif` and the default
//! font) in Liberation Serif, and `Helvetica` in Liberation Sans (by its
//! alternate, Arial). It takes fontconfig's best match for a name only when
//! that is the family the configuration put first, the name itself, or a
//! metric-compatible replacement in Skia's table, so a family a rule merely
//! *appends* (what `fc-match` answers) is not used, and `Calibri` without
//! Carlito falls through. Measured with `CSS.getPlatformFontsForNode` and a
//! private fontconfig hiding msttcorefonts.
//!
//! This runs fontconfig, but not on the host's fonts or configuration: CI's
//! runner has DejaVu only, so the Liberation faces are **bundled**, made at
//! test time from the bundled Inter with rewritten `name` tables, and
//! `FONTCONFIG_FILE` names a configuration whose only font directory holds
//! them and whose rules are this file's own: the `30-metric-aliases.conf`
//! shape for Arial and Times New Roman (`<alias binding="same">` with
//! `<accept>`), two probe rules (one prepending a family, one appending it)
//! and a last-resort family appended to every pattern, so fontconfig's answer
//! for an unknown name is known. Its own test binary, so the variable is set
//! before fontconfig first loads. Linux and the BSDs only (fontique's
//! fontconfig backend); not Android.

#![cfg(all(
    unix,
    not(any(target_os = "android", target_os = "macos", target_os = "ios"))
))]

use parley::style::FontFamily;

const BASE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// `BASE` with its `name` table replaced by one holding `names` (name ID,
/// string), as Windows Unicode English records. The new table is appended and
/// the table directory pointed at it; checksums are left stale, which neither
/// fontconfig nor fontique verifies.
fn with_names(names: &[(u16, &str)]) -> Vec<u8> {
    let mut names = names.to_vec();
    names.sort_by_key(|(id, _)| *id);
    let mut records = Vec::new();
    let mut storage = Vec::new();
    for (id, s) in &names {
        let bytes: Vec<u8> = s.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for field in [
            3u16,
            1,
            0x409,
            *id,
            bytes.len() as u16,
            storage.len() as u16,
        ] {
            records.extend_from_slice(&field.to_be_bytes());
        }
        storage.extend_from_slice(&bytes);
    }
    let mut table = Vec::new();
    table.extend_from_slice(&0u16.to_be_bytes());
    table.extend_from_slice(&(names.len() as u16).to_be_bytes());
    table.extend_from_slice(&(6 + records.len() as u16).to_be_bytes());
    table.extend_from_slice(&records);
    table.extend_from_slice(&storage);

    let mut data = BASE.to_vec();
    while !data.len().is_multiple_of(4) {
        data.push(0);
    }
    let offset = data.len() as u32;
    data.extend_from_slice(&table);
    while !data.len().is_multiple_of(4) {
        data.push(0);
    }
    let num_tables = u16::from_be_bytes([data[4], data[5]]) as usize;
    let entry = (0..num_tables)
        .map(|i| 12 + 16 * i)
        .find(|&e| &data[e..e + 4] == b"name")
        .expect("Inter has a name table");
    data[entry + 8..entry + 12].copy_from_slice(&offset.to_be_bytes());
    data[entry + 12..entry + 16].copy_from_slice(&(table.len() as u32).to_be_bytes());
    data
}

/// A face of `BASE` named `family` (regular).
fn face(family: &str) -> Vec<u8> {
    let postscript: String = family.chars().filter(|c| !c.is_whitespace()).collect();
    with_names(&[
        (1, family),
        (2, "Regular"),
        (4, family),
        (6, &format!("{postscript}-Regular")),
    ])
}

const FALLBACK: &str = "Fallback Face 2336";
const TARGET: &str = "Target Face 2336";

/// The test's temporary directory, removed when dropped, so a failing
/// assertion does not leave it behind.
struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn configure() -> TempDir {
    let dir = std::env::temp_dir().join(format!("rinch-fc-2336-{}", std::process::id()));
    let fonts = dir.join("fonts");
    std::fs::create_dir_all(&fonts).unwrap();
    for family in ["Liberation Sans", "Liberation Serif", TARGET, FALLBACK] {
        let file: String = family.chars().filter(|c| !c.is_whitespace()).collect();
        std::fs::write(fonts.join(format!("{file}.ttf")), face(family)).unwrap();
    }
    let conf = dir.join("fonts.conf");
    std::fs::write(
        &conf,
        format!(
            r#"<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "fonts.dtd">
<fontconfig>
  <dir>{fonts}</dir>
  <cachedir>{cache}</cachedir>
  <alias binding="same">
    <family>Arial</family>
    <accept><family>Arimo</family><family>Liberation Sans</family></accept>
  </alias>
  <alias binding="same">
    <family>Times New Roman</family>
    <accept><family>Tinos</family><family>Liberation Serif</family></accept>
  </alias>
  <match target="pattern">
    <test name="family"><string>Probe Prepend</string></test>
    <edit name="family" mode="prepend" binding="strong"><string>{TARGET}</string></edit>
  </match>
  <match target="pattern">
    <test name="family"><string>Probe Append</string></test>
    <edit name="family" mode="append" binding="strong"><string>{TARGET}</string></edit>
  </match>
  <match target="pattern">
    <edit name="family" mode="append_last"><string>{FALLBACK}</string></edit>
  </match>
</fontconfig>
"#,
            fonts = fonts.display(),
            cache = dir.join("cache").display(),
        ),
    )
    .unwrap();
    TempDir(dir)
}

fn source(family: FontFamily<'_>) -> String {
    match family {
        FontFamily::Source(s) => s.into_owned(),
        other => panic!("not a source list: {other:?}"),
    }
}

fn first_name(cx: &mut parley::FontContext, generic: parley::fontique::GenericFamily) -> String {
    let id = cx.collection.generic_families(generic).next().unwrap();
    cx.collection.family_name(id).unwrap().to_owned()
}

/// Whether `Hamburg` in `family` at `weight` is laid out emboldened.
fn emboldened(cx: &mut parley::FontContext, family: &str, weight: f32) -> bool {
    use parley::layout::PositionedLayoutItem;
    use parley::style::StyleProperty;
    let mut layout_cx: parley::LayoutContext<()> = parley::LayoutContext::new();
    let mut b = layout_cx.ranged_builder(cx, "Hamburg", 1.0, true);
    b.push_default(StyleProperty::FontFamily(FontFamily::Source(
        format!("'{family}'").into(),
    )));
    b.push_default(StyleProperty::FontWeight(
        parley::fontique::FontWeight::new(weight),
    ));
    let mut layout = b.build("Hamburg");
    layout.break_all_lines(None);
    let runs: Vec<bool> = layout
        .lines()
        .flat_map(|line| line.items().collect::<Vec<_>>())
        .filter_map(|item| match item {
            PositionedLayoutItem::GlyphRun(g) => Some(g.run().synthesis().embolden()),
            _ => None,
        })
        .collect();
    assert!(!runs.is_empty(), "no glyph run for {family} at {weight}");
    runs[0]
}

#[test]
fn a_missing_family_takes_the_substitute_chrome_accepts() {
    let dir = configure();
    // SAFETY: the only test in this binary, before any thread or fontconfig
    // call reads the environment.
    unsafe { std::env::set_var("FONTCONFIG_FILE", dir.0.join("fonts.conf")) };

    let mut cx = rinch_dom::fonts::new_font_context();

    // Positive controls: fontconfig read this configuration (the stand-in
    // is a family, and Arial's alias rule ran), and no Arial is installed.
    assert!(cx.collection.family_by_name("Liberation Sans").is_some());
    assert!(cx.collection.family_by_name("Arial").is_none());
    let arial = cx
        .collection
        .system_family_match("Arial")
        .expect("fontconfig answers");
    assert_eq!(arial.configured, "Arial");
    assert_eq!(arial.matched, ["Liberation Sans"]);

    let mut finish = |stack: &str| source(rinch_dom::fonts::parley_font_family(&mut cx, stack));
    let fallback = format!("\"{FALLBACK}\"");
    // Metric-compatible replacements, in place.
    assert_eq!(
        finish(&format!("Arial, '{FALLBACK}'")),
        format!("\"Liberation Sans\", {fallback}")
    );
    assert_eq!(
        finish(&format!("'Times New Roman', '{FALLBACK}'")),
        format!("\"Liberation Serif\", {fallback}")
    );
    // Helvetica's match (the fallback) is not acceptable; its alternate
    // Arial's is.
    assert_eq!(
        finish(&format!("Helvetica, '{FALLBACK}'")),
        format!("\"Liberation Sans\", {fallback}")
    );
    // A family the configuration puts first is accepted...
    assert_eq!(
        finish(&format!("'Probe Prepend', '{FALLBACK}'")),
        format!("\"{TARGET}\", {fallback}")
    );
    // ...one it only appends is not, though fontconfig matches it, and
    // neither is a match outside Chrome's table: the list moves on.
    assert_eq!(
        finish(&format!("'Probe Append', '{FALLBACK}'")),
        format!("'Probe Append', '{FALLBACK}'")
    );
    assert_eq!(
        finish(&format!("Calibri, '{FALLBACK}'")),
        format!("Calibri, '{FALLBACK}'")
    );
    // The default font, Times New Roman, is Liberation Serif here.
    assert_eq!(
        finish("NoSuchFamily2336"),
        "NoSuchFamily2336, \"Liberation Serif\""
    );

    // Chrome's `serif` and `sans-serif` head their slots as their
    // substitutes (`new_font_context` applied the defaults).
    assert_eq!(
        first_name(&mut cx, parley::fontique::GenericFamily::Serif),
        "Liberation Serif"
    );
    assert_eq!(
        first_name(&mut cx, parley::fontique::GenericFamily::SansSerif),
        "Liberation Sans"
    );

    // The app can turn it off: Arial is missing, and the list ends in the
    // default font, which without Times New Roman is the sans face.
    rinch_dom::fonts::set_font_defaults(
        rinch_dom::fonts::FontDefaults::default().with_system_substitutes(false),
    );
    let mut off = rinch_dom::fonts::new_font_context();
    let sans = first_name(&mut off, parley::fontique::GenericFamily::SansSerif);
    assert_ne!(sans, "Liberation Sans");
    assert_eq!(
        source(rinch_dom::fonts::parley_font_family(
            &mut off,
            "Arial, NoSuch2336"
        )),
        format!("Arial, NoSuch2336, \"{sans}\"")
    );
    rinch_dom::fonts::set_font_defaults(Default::default());

    // #2343: these are system faces (400), so the system rule applies:
    // emboldened past 200 over the face, at 601 and not at 600, as Chrome
    // draws Liberation Sans Regular. A registered face is emboldened from
    // 600 (`faux_bold_threshold_2343.rs`).
    let mut cx = rinch_dom::fonts::new_font_context();
    assert!(!emboldened(&mut cx, "Liberation Sans", 600.0));
    assert!(emboldened(&mut cx, "Liberation Sans", 601.0));
}
