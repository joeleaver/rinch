//! A `Modal` taller than the window stays inside it (issue #2508).
//!
//! `.rinch-modal` used to size itself with
//! `max-height: calc(100vh - var(--rinch-spacing-xl) * 2)`, inside a root that
//! starts at `top: var(--rinch-window-top-inset)` and pads by `xl`, plus its own
//! `margin-top: xl`. The calc knew none of that, so a tall modal's bottom landed
//! at `100vh + inset` in a `BorderlessWindow` (the footer buttons cut off by the
//! window edge) and at `100vh` with no inset — one `xl` past the gap the root's
//! padding promises. The whole panel was the scroller, so the header scrolled
//! away with the content.
//!
//! The root now sizes the panel: it is a flex column filling the area below the
//! inset, the panel shrinks to fit it, and the body is the scroller. These
//! measure that through the real theme and component sheets.
//!
//! The inset is **40px**, not 0 or a multiple of the 32px `xl`: an off-by-one-
//! `xl` mix-up and a forgotten inset then read as different numbers.

use super::*;

use rinch_components::Modal;
use rinch_core::Component;

const VIEWPORT: (f32, f32) = (800.0, 600.0);
/// `--rinch-spacing-xl` in the default theme (2rem).
const XL: f32 = 32.0;
const INSET: f32 = 40.0;

/// Mount an open modal whose body holds `content_px` of content and a footer,
/// under a wrapper that publishes `inset` as `--rinch-window-top-inset` (the
/// way `BorderlessWindow` does).
fn mount(inset: f32, centered: bool, content_px: f32) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let page = scope.create_element("div");
        page.set_attribute("style", &format!("--rinch-window-top-inset: {inset}px"));

        let tall = scope.create_element("div");
        tall.set_attribute("class", "tall-content");
        tall.set_attribute("style", &format!("height: {content_px}px"));
        let footer = scope.create_element("div");
        footer.set_attribute("class", "modal-footer");
        footer.set_attribute("style", "height: 36px");

        let modal = Modal {
            opened: true,
            title: "Tall".into(),
            centered,
            ..Default::default()
        }
        .render(scope, &[tall, footer]);
        page.append_child(&modal);
        page
    });
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    app
}

fn find_by_class(app: &RinchApp, class: &str) -> u64 {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .nodes
        .iter()
        .find(|(_, n)| {
            n.attributes
                .get("class")
                .is_some_and(|c| c.split_whitespace().any(|one| one == class))
        })
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no node carries the class {class}"))
}

/// `(x0, y0, x1, y1)` of the node carrying `class`, on screen.
fn box_of(app: &RinchApp, class: &str) -> (f32, f32, f32, f32) {
    let id = find_by_class(app, class);
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let (x, y, w, h) = painted_element_box(&d.tree, id);
    (x, y, x + w, y + h)
}

fn can_scroll_down(app: &RinchApp, class: &str) -> bool {
    let id = find_by_class(app, class);
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    rinch_dom::paint::scrollbar::can_scroll_toward(&d.tree, id, 0.0, 1.0)
}

/// The panel's top and bottom gaps for a modal far taller than the window.
fn assert_fits(app: &RinchApp, inset: f32, top_gap: f32) {
    let modal = box_of(app, "rinch-modal");
    assert!(
        (modal.1 - (inset + top_gap)).abs() < 0.5,
        "the panel's top must sit {top_gap}px below the {inset}px inset; got {modal:?}"
    );
    assert!(
        (modal.3 - (VIEWPORT.1 - XL)).abs() < 0.5,
        "a tall panel must end one xl above the window bottom ({}); got {modal:?}",
        VIEWPORT.1 - XL
    );

    // The header stays in view, and the body is what scrolls.
    let header = box_of(app, "rinch-modal__header");
    assert!(
        header.1 >= modal.1 - 0.5 && header.3 <= modal.3 + 0.5,
        "the header must stay inside the panel; header {header:?}, panel {modal:?}"
    );
    let body = box_of(app, "rinch-modal__body");
    assert!(
        body.3 <= modal.3 + 0.5,
        "the body must be clipped to the panel, not run past it; body {body:?}, panel {modal:?}"
    );
    assert!(
        can_scroll_down(app, "rinch-modal__body"),
        "the body must scroll so the footer can be reached"
    );
    assert!(
        !can_scroll_down(app, "rinch-modal"),
        "the panel itself must not scroll (the header would scroll away with it)"
    );
}

#[test]
fn a_tall_modal_fits_below_the_window_top_inset() {
    let app = mount(INSET, false, 2000.0);
    assert_fits(&app, INSET, 2.0 * XL);
}

#[test]
fn a_tall_modal_keeps_the_bottom_gap_without_an_inset() {
    let app = mount(0.0, false, 2000.0);
    assert_fits(&app, 0.0, 2.0 * XL);
}

#[test]
fn a_tall_centered_modal_fits_below_the_window_top_inset() {
    let app = mount(INSET, true, 2000.0);
    assert_fits(&app, INSET, XL);
}

/// A short modal is untouched: its natural height, its footer in view, no
/// scroller, and still centred (horizontally always; vertically when asked).
#[test]
fn a_short_modal_keeps_its_height_and_centring() {
    for centered in [false, true] {
        let app = mount(INSET, centered, 120.0);
        let modal = box_of(&app, "rinch-modal");
        let footer = box_of(&app, "modal-footer");
        assert!(
            footer.3 <= modal.3 + 0.5,
            "centered={centered}: the footer must be inside the panel; footer {footer:?}, panel {modal:?}"
        );
        assert!(
            !can_scroll_down(&app, "rinch-modal__body"),
            "centered={centered}: a short body has nothing to scroll"
        );
        // md is 440px wide, centred in 800.
        assert!(
            (modal.0 - 180.0).abs() < 0.5 && (modal.2 - 620.0).abs() < 0.5,
            "centered={centered}: the md panel is centred horizontally; got {modal:?}"
        );
        let height = modal.3 - modal.1;
        if centered {
            let area_mid = (INSET + VIEWPORT.1) / 2.0;
            let mid = (modal.1 + modal.3) / 2.0;
            assert!(
                (mid - area_mid).abs() < 0.5,
                "a centered panel is centred in the area below the inset ({area_mid}); got {modal:?}"
            );
        } else {
            assert!(
                (modal.1 - (INSET + 2.0 * XL)).abs() < 0.5,
                "the panel sits two xl below the inset; got {modal:?}"
            );
        }
        assert!(
            height < 300.0,
            "the panel keeps its natural height; got {height}"
        );
    }
}
