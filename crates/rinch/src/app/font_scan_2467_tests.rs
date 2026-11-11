//! One enumeration of the system fonts per process (#2467).
//!
//! An app's start-up builds two font contexts, the document's (layout and
//! paint) and the app's hit-test one, and each used to make fontique list
//! every installed font: two scans, 64% of a small app's software start-up.
//! They now share one system-font collection (`rinch_dom::fonts::
//! new_font_context`), and stay independent in everything else, which is what
//! the hit-test context exists for: it is reached while the document is
//! borrowed, and it must agree with the document's glyph for glyph
//! (`font_tests.rs`, #492).
//!
//! The count is per process and other tests in this binary build contexts
//! too, so the assertion is that it is exactly one after a start-up: a
//! context that builds its own collection again makes it two, and a
//! `new_font_context` that stopped going through the shared collection
//! leaves it at zero.

use super::*;
use parley::fontique::SourceKind;

const FACE: &[u8] = include_bytes!("../../assets/fonts/Inter-Bold.ttf");

/// Whether `family` in `font_cx` has a face read from memory, which is what a
/// registered face is: a system face is a file.
fn has_registered_face(font_cx: &mut parley::FontContext, family: &str) -> bool {
    font_cx.collection.family_by_name(family).is_some_and(|f| {
        f.fonts()
            .iter()
            .any(|font| matches!(font.source().kind, SourceKind::Memory(_)))
    })
}

#[test]
fn an_app_start_up_scans_the_system_fonts_once() {
    let mut app = RinchApp::new(|scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 100%; height: 100%");
        root
    });
    app.register_font_data(FACE);
    app.mount_component(800.0, 600.0);
    assert!(app.doc().is_some(), "mounted");
    assert_eq!(
        rinch_dom::fonts::system_font_scans(),
        1,
        "an app's start-up (its document's and its hit-test font context) \
         scans the system fonts exactly once"
    );

    // A later document, as a second window or a remount builds, reuses it.
    let _second = rinch_dom::RinchDocument::new();
    assert_eq!(rinch_dom::fonts::system_font_scans(), 1);

    // Sharing the scan shares nothing else: the face the app registered is
    // on both of its contexts and on no other.
    assert!(has_registered_face(&mut app.hit_test_font_cx, "Inter"));
    {
        let doc = app.doc().expect("mounted");
        assert!(has_registered_face(&mut doc.borrow_mut().font_cx, "Inter"));
    }
    let mut fresh = rinch_dom::fonts::new_font_context();
    assert!(
        !has_registered_face(&mut fresh, "Inter"),
        "a font registered on one context must not reach another"
    );
}
