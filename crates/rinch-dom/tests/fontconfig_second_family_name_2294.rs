//! A family named only by a font's *second* fontconfig family name resolves
//! to the fonts that list it (#2294).
//!
//! fontconfig puts a font in family `X` when `X` is any of its `FC_FAMILY`
//! names. A font with a typographic family (`name` ID 16) lists it first and
//! its legacy family (ID 1) second: `/usr/local/share/fonts/a/ariblk.ttf` is
//! "Arial, Arial Black". fontique used only the first name, so on a host that
//! also had msttcorefonts' `ariblk.ttf` ("Arial Black" alone),
//! `font-family: 'Arial Black'` came out as Arial Regular (the second name
//! was an alias of Arial) or as nothing. The fix is in the parley fork
//! (fontique's fontconfig backend).
//!
//! This test is about fontconfig itself, so it runs fontconfig, but not on the
//! host's fonts: `FONTCONFIG_FILE` names a configuration whose only font
//! directory is a temporary one holding two copies of the bundled Inter with
//! rewritten `name` tables, laid out as the two Arial Black files are. It is
//! its own test binary so the variable is set before fontconfig first loads.
//! Linux and the BSDs only (fontique's fontconfig backend); not Android.

#![cfg(all(
    unix,
    not(any(target_os = "android", target_os = "macos", target_os = "ios"))
))]

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

#[test]
fn a_second_family_name_is_a_family_of_the_fonts_that_list_it() {
    let dir = std::env::temp_dir().join(format!("rinch-fc-2294-{}", std::process::id()));
    let fonts = dir.join("fonts");
    std::fs::create_dir_all(&fonts).unwrap();
    // As `/usr/local/share/fonts/a/ariblk.ttf`: "Probe2294, Probe2294 Black".
    std::fs::write(
        fonts.join("a-typographic.ttf"),
        with_names(&[
            (1, "Probe2294 Black"),
            (2, "Regular"),
            (4, "Probe2294 Black"),
            (6, "Probe2294-Black"),
            (16, "Probe2294"),
            (17, "Black"),
        ]),
    )
    .unwrap();
    // As msttcorefonts' `ariblk.ttf`: "Probe2294 Black" alone.
    std::fs::write(
        fonts.join("b-legacy.ttf"),
        with_names(&[
            (1, "Probe2294 Black"),
            (2, "Regular"),
            (4, "Probe2294 Black"),
            (6, "Probe2294Black"),
        ]),
    )
    .unwrap();
    let conf = dir.join("fonts.conf");
    std::fs::write(
        &conf,
        format!(
            "<?xml version=\"1.0\"?>\n<!DOCTYPE fontconfig SYSTEM \"fonts.dtd\">\n\
             <fontconfig><dir>{}</dir><cachedir>{}</cachedir></fontconfig>\n",
            fonts.display(),
            dir.join("cache").display()
        ),
    )
    .unwrap();
    // SAFETY: the only test in this binary, before any thread or fontconfig
    // call reads the environment.
    unsafe { std::env::set_var("FONTCONFIG_FILE", &conf) };

    let mut cx = parley::FontContext::new();
    let collection = &mut cx.collection;

    // Positive control: the configuration was read, and the first name is a
    // family holding the one font that lists it first.
    let probe = collection
        .family_by_name("Probe2294")
        .expect("fontconfig read the test configuration");
    assert_eq!(probe.name(), "Probe2294");
    assert_eq!(probe.fonts().len(), 1);

    // The second name: both fonts, under that name. Before the fix it was the
    // first name's family (one font, named "Probe2294") or one font only.
    let black = collection
        .family_by_name("Probe2294 Black")
        .expect("'Probe2294 Black' resolves");
    assert_eq!(black.name(), "Probe2294 Black");
    assert_eq!(
        black.fonts().len(),
        2,
        "both fonts list 'Probe2294 Black': {:?}",
        black.fonts()
    );
    // Matched case-insensitively, as fontconfig and the name map do.
    let lower = collection.family_by_name("probe2294 black").unwrap();
    assert_eq!(lower.fonts().len(), 2);

    let _ = std::fs::remove_dir_all(&dir);
}
