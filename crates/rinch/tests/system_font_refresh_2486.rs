//! A font installed while an app runs reaches the app after a refresh (#2486).
//!
//! The process scans the system fonts once and every font context shares the
//! scan (#2467). `rinch_dom::fonts::refresh_system_fonts` (behind
//! `rinch::font::refresh_system_fonts` and the fontconfig watch) scans again;
//! the app's next frame rebuilds its font contexts over the new scan,
//! registers its own fonts on them again, and invalidates what was measured
//! against the old fonts: text widths, font-metric units (the #2439 cache).
//!
//! This test runs fontconfig on a configuration of its own: `FONTCONFIG_FILE`
//! names one whose only font directory is a temporary one, empty at start-up,
//! so before the install no face answers at all. It is its own test binary so
//! the variable is set before fontconfig first loads. Linux and the BSDs only
//! (fontique's fontconfig backend); not Android.

#![cfg(all(
    unix,
    not(any(target_os = "android", target_os = "macos", target_os = "ios"))
))]

use parley::fontique::SourceKind;
use rinch::app::RinchApp;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::DimensionValue;

const INSTALLED: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const BUNDLED: &[u8] =
    include_bytes!("../../rinch-dom/assets/fonts/SpaceGrotesk-VariableFont_wght.ttf");

fn component(scope: &mut RenderScope) -> NodeHandle {
    let root = scope.create_element("div");
    // `ex` with no face is Stylo's 0.5em (80px); Inter's x-height is
    // 1118/2048 em (87.3px).
    let ex = scope.create_element("div");
    ex.set_attribute("data-m", "ex");
    ex.set_attribute(
        "style",
        "font-family: Inter; font-size: 16px; line-height: 20px; width: 10ex; height: 4px",
    );
    root.append_child(&ex);
    // Shrink-to-fit around text: no face, no glyph advances.
    let text = scope.create_element("div");
    text.set_attribute("data-m", "text");
    text.set_attribute(
        "style",
        "display: inline-block; font-family: Inter; font-size: 16px; line-height: 20px",
    );
    text.append_child(&scope.create_text("Installed later"));
    root.append_child(&text);
    root
}

fn find(doc: &RinchDocument, m: &str) -> u64 {
    let found = rinch_dom::testing::query_selector(&doc.tree, &format!("[data-m={m}]"));
    assert_eq!(found.len(), 1, "[data-m={m}]");
    found[0]
}

fn ex_width(doc: &RinchDocument) -> f32 {
    match doc.tree.get(find(doc, "ex")).unwrap().computed_style.width {
        DimensionValue::Length(px) => px,
        other => panic!("width computed to {other:?}"),
    }
}

fn text_width(doc: &RinchDocument) -> f32 {
    doc.tree.get(find(doc, "text")).unwrap().layout.width
}

/// Whether `family` has a face from a file (a system face) and one from
/// memory (a registered one) in `font_cx`.
fn sources(font_cx: &mut parley::FontContext, family: &str) -> (bool, bool) {
    let Some(f) = font_cx.collection.family_by_name(family) else {
        return (false, false);
    };
    let file = f
        .fonts()
        .iter()
        .any(|font| matches!(font.source().kind, SourceKind::Path(_)));
    let memory = f
        .fonts()
        .iter()
        .any(|font| matches!(font.source().kind, SourceKind::Memory(_)));
    (file, memory)
}

#[test]
fn a_font_installed_while_the_app_runs_is_used_after_a_refresh() {
    let dir = std::env::temp_dir().join(format!("rinch-fc-2486-{}", std::process::id()));
    let fonts = dir.join("fonts");
    std::fs::create_dir_all(&fonts).unwrap();
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

    let mut app = RinchApp::new(component);
    // A bundled face, under a family of its own: it must survive the rebuild.
    app.register_app_font(rinch::font::AppFont::new(BUNDLED));
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);

    let doc = app.doc().expect("mounted").clone();
    {
        let mut d = doc.borrow_mut();
        // Positive control: fontconfig read the empty test configuration.
        assert_eq!(
            sources(&mut d.font_cx, "Inter"),
            (false, false),
            "no Inter yet"
        );
        assert_eq!(sources(&mut d.font_cx, "Space Grotesk"), (false, true));
        assert_eq!(ex_width(&d), 80.0, "no face: ex is 0.5em");
        assert_eq!(text_width(&d), 0.0, "no face: no advances");
    }

    // The install. Nothing changes until a refresh: the scan is shared.
    std::fs::write(fonts.join("Inter-Regular.ttf"), INSTALLED).unwrap();
    assert!(!app.has_pending_layout());
    let generation = rinch_dom::fonts::system_fonts_generation();
    let scans = rinch_dom::fonts::system_font_scans();

    // The public refresh: it scans off this thread and returns.
    rinch::font::refresh_system_fonts();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while rinch_dom::fonts::system_fonts_generation() == generation {
        assert!(
            std::time::Instant::now() < deadline,
            "the refresh never finished"
        );
        std::thread::yield_now();
    }
    assert_eq!(
        rinch_dom::fonts::system_font_scans(),
        scans + 1,
        "one rescan"
    );
    // A host that resolves only when something is owed (the desktop shell)
    // must see it.
    assert!(
        app.has_pending_layout(),
        "a refreshed scan is owed to the app"
    );
    app.resolve_and_repaint(800.0, 600.0);
    assert!(!app.has_pending_layout(), "and adopted once");

    let mut d = doc.borrow_mut();
    assert_eq!(
        sources(&mut d.font_cx, "Inter"),
        (true, false),
        "the installed Inter, from its file"
    );
    assert_eq!(
        sources(&mut d.font_cx, "Space Grotesk"),
        (false, true),
        "the app's own face is registered again on the rebuilt context"
    );
    let ex = ex_width(&d);
    assert!(
        (ex - 87.3).abs() < 0.2,
        "ex is Inter's x-height now (the font-metric cache was dropped): {ex}"
    );
    assert!(
        text_width(&d) > 50.0,
        "the text is shaped in Inter: {}",
        text_width(&d)
    );
    drop(d);

    // A document built after the refresh starts from the new scan.
    let mut later = RinchDocument::new();
    assert_eq!(sources(&mut later.font_cx, "Inter"), (true, false));

    let _ = std::fs::remove_dir_all(&dir);
}
