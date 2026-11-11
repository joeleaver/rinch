//! `Modal { footer }`: a pinned footer row below the scrolling body.
//!
//! #2508 made `.rinch-modal` a flex column (header, then a body that scrolls).
//! The footer is a third row after the body with `flex-shrink: 0`, so a tall
//! modal's buttons stay at the panel's bottom while the body scrolls between
//! the header and the footer, and a short window shrinks the body first.
//!
//! Every geometry is declared: the footer's content is a 36px box, and the
//! fixture pins the title's line box (`28px`), so no number moves with the
//! host's fonts. The inset is **40px** (not 0 or a multiple of the 32px `xl`),
//! as in `modal_tall_content_tests`, so a forgotten inset reads differently
//! from an off-by-one `xl`.

use super::*;

use rinch_components::{Modal, SectionRenderer};
use rinch_core::Component;

const W: f32 = 800.0;
/// `--rinch-spacing-xl` (2rem) and `--rinch-spacing-lg` (1.25rem).
const XL: f32 = 32.0;
const LG: f32 = 20.0;
/// `--rinch-spacing-sm` (0.75rem), the footer's gap.
const SM: f32 = 12.0;
const INSET: f32 = 40.0;
/// The footer's content box: the 36px row plus the `lg` padding either side
/// plus the 1px top border.
const FOOTER_H: f32 = 36.0 + 2.0 * LG + 1.0;
/// The header: `md` (16px) padding either side of the pinned 28px title, plus
/// its 1px bottom border.
const HEADER_H: f32 = 16.0 + 28.0 + 16.0 + 1.0;

/// A footer renderer making two declared-size buttons, `#cancel` and `#save`,
/// in the wrapper an rsx `Fragment { … }` builds (`<div data-fragment>`), so
/// the stylesheet's row, end alignment and `sm` gap are what place them.
fn two_buttons() -> SectionRenderer {
    Rc::new(|scope: &mut RenderScope| {
        let fragment = scope.create_element("div");
        fragment.set_attribute("data-fragment", "true");
        for id in ["cancel", "save"] {
            let b = scope.create_element("button");
            b.set_attribute("id", id);
            b.set_attribute("style", "display: block; width: 80px; height: 36px");
            fragment.append_child(&b);
        }
        fragment
    })
}

/// Mount an open modal with `content_px` of body content (holding one button,
/// `#in-body`, at its top) and, if `footer`, the two-button footer.
fn mount(inset: f32, h: f32, content_px: f32, footer: bool) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let page = scope.create_element("div");
        page.set_attribute("style", &format!("--rinch-window-top-inset: {inset}px"));

        let before = scope.create_element("button");
        before.set_attribute("id", "before");
        before.set_attribute("style", "display: block; width: 80px; height: 28px");
        page.append_child(&before);

        let in_body = scope.create_element("button");
        in_body.set_attribute("id", "in-body");
        in_body.set_attribute("style", "display: block; width: 80px; height: 28px");
        let tall = scope.create_element("div");
        tall.set_attribute("class", "tall-content");
        tall.set_attribute("style", &format!("height: {content_px}px"));

        let modal = Modal {
            opened: true,
            title: "Footer".into(),
            footer: if footer { Some(two_buttons()) } else { None },
            ..Default::default()
        }
        .render(scope, &[in_body, tall]);
        page.append_child(&modal);
        page
    });
    app.mount_component(W, h);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.load_css(".rinch-modal__title { font-size: 18px; line-height: 28px; }");
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(W, h);
    app
}

fn find(app: &RinchApp, pred: impl Fn(&rinch_dom::Node) -> bool, what: &str) -> u64 {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .nodes
        .iter()
        .find(|(_, n)| pred(n))
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no node is {what}"))
}

fn by_class(app: &RinchApp, class: &str) -> u64 {
    find(
        app,
        |n| {
            n.attributes
                .get("class")
                .is_some_and(|c| c.split_whitespace().any(|one| one == class))
        },
        class,
    )
}

fn by_id(app: &RinchApp, id: &str) -> u64 {
    find(app, |n| n.attributes.get("id").is_some_and(|v| v == id), id)
}

fn has_class(app: &RinchApp, class: &str) -> bool {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.nodes.iter().any(|(_, n)| {
        n.attributes
            .get("class")
            .is_some_and(|c| c.split_whitespace().any(|one| one == class))
    })
}

/// `(x0, y0, x1, y1)` on screen.
fn box_of(app: &RinchApp, id: u64) -> (f32, f32, f32, f32) {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let (x, y, w, h) = painted_element_box(&d.tree, id);
    (x, y, x + w, y + h)
}

fn can_scroll_down(app: &RinchApp, id: u64) -> bool {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    rinch_dom::paint::scrollbar::can_scroll_toward(&d.tree, id, 0.0, 1.0)
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.5
}

/// A tall modal with a footer: the panel ends one `xl` above the window
/// bottom, the footer is whole at its bottom, the header whole at its top, and
/// the body between them is the scroller.
fn assert_tall_with_footer(inset: f32) {
    let h = 600.0;
    let app = mount(inset, h, 2000.0, true);
    let panel = box_of(&app, by_class(&app, "rinch-modal"));
    let header = box_of(&app, by_class(&app, "rinch-modal__header"));
    let body_id = by_class(&app, "rinch-modal__body");
    let body = box_of(&app, body_id);
    let footer = box_of(&app, by_class(&app, "rinch-modal__footer"));
    let save = box_of(&app, by_id(&app, "save"));

    assert!(
        near(panel.1, inset + 2.0 * XL) && near(panel.3, h - XL),
        "inset {inset}: the panel spans inset + 2xl .. window - xl; got {panel:?}"
    );
    assert!(
        near(footer.3, panel.3) && near(footer.3 - footer.1, FOOTER_H),
        "inset {inset}: the footer is whole ({FOOTER_H}px) at the panel's bottom; \
         footer {footer:?}, panel {panel:?}"
    );
    assert!(
        save.1 >= footer.1 && save.3 <= footer.3 + 0.5 && save.3 <= h - XL + 0.5,
        "inset {inset}: the Save button is inside the footer and the window; \
         save {save:?}, footer {footer:?}"
    );
    // The buttons are at the end of the row, `sm` apart, in one row: Save's
    // right edge is the panel's right edge less the footer's `lg` padding.
    let cancel = box_of(&app, by_id(&app, "cancel"));
    assert!(
        near(save.2, panel.2 - LG) && near(save.0 - cancel.2, SM) && near(save.1, cancel.1),
        "inset {inset}: the footer's buttons sit at its end, {SM}px apart; \
         cancel {cancel:?}, save {save:?}, panel {panel:?}"
    );
    assert!(
        near(header.1, panel.1) && near(header.3 - header.1, HEADER_H),
        "inset {inset}: the header is whole at the panel's top; header {header:?}"
    );
    assert!(
        near(body.1, header.3) && near(body.3, footer.1),
        "inset {inset}: the body fills the space between header and footer; \
         body {body:?}, header {header:?}, footer {footer:?}"
    );
    assert!(
        can_scroll_down(&app, body_id),
        "inset {inset}: the body scrolls"
    );
    assert!(
        !can_scroll_down(&app, by_class(&app, "rinch-modal")),
        "inset {inset}: the panel itself does not scroll"
    );
}

#[test]
fn a_tall_modal_pins_its_footer_below_the_window_top_inset() {
    assert_tall_with_footer(INSET);
}

#[test]
fn a_tall_modal_pins_its_footer_without_an_inset() {
    assert_tall_with_footer(0.0);
}

/// A very short window: the header and footer keep their full height and the
/// body gives way, down to its own padding (its scrollport is empty).
///
/// The window is sized so the panel's area is exactly header + footer + the
/// body's `lg` padding: `h = inset + 2xl (root padding + margin) + xl (bottom)
/// + HEADER_H + 2lg + FOOTER_H`.
#[test]
fn a_very_short_window_shrinks_the_body_to_nothing_first() {
    for inset in [INSET, 0.0] {
        let h = inset + 3.0 * XL + HEADER_H + 2.0 * LG + FOOTER_H;
        let app = mount(inset, h, 2000.0, true);
        let panel = box_of(&app, by_class(&app, "rinch-modal"));
        let header = box_of(&app, by_class(&app, "rinch-modal__header"));
        let body = box_of(&app, by_class(&app, "rinch-modal__body"));
        let footer = box_of(&app, by_class(&app, "rinch-modal__footer"));
        let save = box_of(&app, by_id(&app, "save"));

        assert!(
            near(panel.3, h - XL),
            "inset {inset}, h {h}: the panel ends one xl above the window bottom; got {panel:?}"
        );
        assert!(
            near(header.3 - header.1, HEADER_H) && near(header.1, panel.1),
            "inset {inset}, h {h}: the header is whole; header {header:?}, panel {panel:?}"
        );
        assert!(
            near(footer.3 - footer.1, FOOTER_H) && near(footer.3, panel.3),
            "inset {inset}, h {h}: the footer is whole; footer {footer:?}, panel {panel:?}"
        );
        assert!(
            save.3 <= panel.3 + 0.5,
            "inset {inset}, h {h}: Save is in view; save {save:?}, panel {panel:?}"
        );
        assert!(
            near(body.3 - body.1, 2.0 * LG),
            "inset {inset}, h {h}: the body is down to its padding (no scrollport); body {body:?}"
        );
    }
}

/// No footer: no footer element, and #2508's numbers unchanged — the panel
/// spans inset + 2xl .. 600 - xl = 568, and the body runs to its bottom.
#[test]
fn no_footer_is_the_modal_as_before() {
    for inset in [INSET, 0.0] {
        let app = mount(inset, 600.0, 2000.0, false);
        assert!(
            !has_class(&app, "rinch-modal__footer"),
            "no footer prop renders no footer element"
        );
        let panel = box_of(&app, by_class(&app, "rinch-modal"));
        let body = box_of(&app, by_class(&app, "rinch-modal__body"));
        assert!(
            near(panel.1, inset + 2.0 * XL) && near(panel.3, 568.0),
            "inset {inset}: #2508's panel box; got {panel:?}"
        );
        assert!(
            near(body.3, panel.3),
            "inset {inset}: the body runs to the panel's bottom; body {body:?}, panel {panel:?}"
        );
    }
}

// ── Tab order and the focus trap ────────────────────────────────────────────

fn tab(app: &mut RinchApp, shift: bool) {
    let modifiers = Modifiers {
        shift,
        ..Default::default()
    };
    app.handle_event(
        PlatformEvent::KeyDown {
            key: KeyCode::Tab,
            logical_key: None,
            text: None,
            modifiers,
            repeat: KeyRepeat::Unknown,
        },
        (W as u32, 600),
        1.0,
    );
    app.handle_event(
        PlatformEvent::KeyUp {
            key: KeyCode::Tab,
            logical_key: None,
            modifiers,
        },
        (W as u32, 600),
        1.0,
    );
}

fn focused(app: &RinchApp) -> Option<u64> {
    match app.focus_target {
        FocusTarget::Input(id) | FocusTarget::Node(id) => Some(id),
        _ => None,
    }
}

/// Tab goes header (the close button), body, footer, and wraps back to the
/// close button without leaving the dialog; Shift+Tab from the close button
/// wraps to the footer's last button.
#[test]
fn tab_order_is_header_body_footer_and_the_trap_includes_the_footer() {
    let mut app = mount(INSET, 600.0, 120.0, true);
    let close = by_class(&app, "rinch-modal__close");
    let in_body = by_id(&app, "in-body");
    let cancel = by_id(&app, "cancel");
    let save = by_id(&app, "save");
    let before = by_id(&app, "before");

    let tour: Vec<_> = (0..5)
        .map(|_| {
            tab(&mut app, false);
            focused(&app)
        })
        .collect();
    assert_eq!(
        tour,
        vec![
            Some(close),
            Some(in_body),
            Some(cancel),
            Some(save),
            Some(close)
        ],
        "Tab: close {close}, body {in_body}, cancel {cancel}, save {save}; \
         `before` (outside) is {before}"
    );

    tab(&mut app, true);
    assert_eq!(
        focused(&app),
        Some(save),
        "Shift+Tab from the first stop wraps to the footer's last button"
    );
}
