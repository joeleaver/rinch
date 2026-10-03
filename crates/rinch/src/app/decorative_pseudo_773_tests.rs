//! Audit of the decorative `::before`/`::after` rules issue #773 names in
//! `rinch-components`' own stylesheet — now that `content: ''` creates a
//! generated box, do these five actually look right?
//!
//! Each fixture mounts the **real** component through the **real** stylesheet
//! (`rinch_components::generate_component_css()`), the same way
//! `css_hook_760_tests::mount` does, and asserts the generated box's geometry
//! and paint-relevant style against what the sheet declares (cross-checked
//! against Chrome's box model for a zero-content absolutely/flex-positioned
//! border box).
//!
//! | Site | Covered below |
//! |---|---|
//! | `styles/tooltip.rs:85` (`with_arrow` arrow) | [`a_tooltip_arrow_is_a_12px_border_triangle`] |
//! | `styles/popover.rs:154` (`with_arrow` arrow) | [`a_popover_arrow_is_an_8px_rotated_square`] |
//! | `styles/divider.rs:47` (`with_label` rules) | [`a_divider_with_a_label_gets_two_flex_strokes`] |
//! | `styles/badge.rs:73` (`--dot` variant) | [`a_dot_badge_gets_an_8px_round_dot`] |
//! | `styles/button.rs:166` (`.rinch-button__loader::after`) | **not reachable at all** — see [`a_loading_buttons_spinner_rule_matches_no_element`]; the markup it targets, `.rinch-button__loader`, is never created by `Button::render` (filed separately as #1334) |

use super::*;

use rinch_components::{
    Badge, Component, Divider, Popover, PopoverDropdown, PopoverTarget, Tooltip,
};
use rinch_dom::computed_style::DisplayValue;

const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// `--rinch-tooltip-color`/`--rinch-color-border`/`--rinch-primary-color`/
/// `--rinch-color-body`: concrete values for the custom properties these
/// rules read, so a color assertion is not "transparent either way" (the same
/// trap `css_hook_760_tests`' module doc explains for its own THEME).
const THEME: &str = ":root { --rinch-tooltip-color: rgb(40, 40, 40); \
                    --rinch-color-border: rgb(200, 200, 200); \
                    --rinch-primary-color: rgb(20, 90, 200); \
                    --rinch-color-body: rgb(250, 250, 250); }";

fn mount(build: impl Fn(&mut RenderScope) -> NodeHandle + 'static) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let child = build(scope);
        root.append_child(&child);
        root
    });
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_components::generate_component_css());
        d.load_css(THEME);
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    app
}

fn nodes_with_class(app: &RinchApp, class: &str) -> Vec<usize> {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let mut found: Vec<usize> = d
        .tree
        .nodes
        .iter()
        .filter(|(_, n)| {
            n.attributes
                .get("class")
                .is_some_and(|c| c.split_whitespace().any(|one| one == class))
        })
        .map(|(id, _)| id)
        .collect();
    found.sort_unstable();
    found
}

fn node_with_class(app: &RinchApp, class: &str) -> usize {
    let found = nodes_with_class(app, class);
    assert_eq!(found.len(), 1, "expected one `{class}`, found {found:?}");
    found[0]
}

/// The one pseudo-element child of `owner`.
fn pseudo_child(app: &RinchApp, owner: usize) -> usize {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let found: Vec<usize> = d
        .tree
        .get(owner)
        .unwrap()
        .children
        .iter()
        .copied()
        .filter(|&c| d.tree.get(c).unwrap().is_pseudo_element)
        .collect();
    assert_eq!(
        found.len(),
        1,
        "expected one pseudo-element child of {owner}, found {found:?}"
    );
    found[0]
}

fn layout_of(app: &RinchApp, node: usize) -> (f32, f32) {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let l = d.tree.get(node).unwrap().layout;
    (l.width, l.height)
}

fn display_of(app: &RinchApp, node: usize) -> DisplayValue {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.get(node).unwrap().computed_style.display
}

fn background_of(app: &RinchApp, node: usize) -> Option<peniko::Color> {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.get(node).unwrap().computed_style.background_color()
}

fn border_widths(app: &RinchApp, node: usize) -> (f32, f32, f32, f32) {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let s = &d.tree.get(node).unwrap().computed_style;
    (
        s.border_top_width.resolve(0.0),
        s.border_right_width.resolve(0.0),
        s.border_bottom_width.resolve(0.0),
        s.border_left_width.resolve(0.0),
    )
}

fn border_top_color(app: &RinchApp, node: usize) -> Option<peniko::Color> {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.get(node).unwrap().computed_style.border_top_color
}

// ── 1. Tooltip arrow ──────────────────────────────────────────────────────

/// `.rinch-tooltip--with-arrow .rinch-tooltip__content::after` is a classic
/// border-triangle: no declared size, `border: 0.375rem solid transparent`
/// (6px at the default 16px root font) on all four sides, with the pointing
/// side's color set by the position rule. With no content and no declared
/// width/height, the box's content area is 0x0, so the border box is
/// 12x12 — a 6px triangle on the colored side, transparent elsewhere, which
/// is exactly how a CSS triangle is drawn in a browser.
#[test]
fn a_tooltip_arrow_is_a_12px_border_triangle() {
    let app = mount(move |scope| {
        let target = scope.create_element("span");
        Tooltip {
            label: "Hi".to_string(),
            opened: true,
            with_arrow: true,
            position: "top".to_string(),
            ..Default::default()
        }
        .render(scope, &[target])
    });
    let content = node_with_class(&app, "rinch-tooltip__content");
    let arrow = pseudo_child(&app, content);
    assert_eq!(
        display_of(&app, arrow),
        DisplayValue::Block,
        "the arrow is positioned absolute — Chrome gives an absolutely \
         positioned box with no declared `display` a `block` used value"
    );
    let (w, h) = layout_of(&app, arrow);
    assert_eq!(
        (w, h),
        (12.0, 12.0),
        "6px border on all sides, 0 content = 12x12"
    );
    let (bt, br, bb, bl) = border_widths(&app, arrow);
    assert_eq!(
        (bt, br, bb, bl),
        (6.0, 6.0, 6.0, 6.0),
        "0.375rem = 6px at 16px root"
    );
    assert_eq!(
        border_top_color(&app, arrow),
        Some(peniko::Color::from_rgb8(40, 40, 40)),
        "the `--top` rule colors the top border toward the target"
    );
}

// ── 2. Popover arrow ──────────────────────────────────────────────────────

/// `.rinch-popover--with-arrow .rinch-popover__dropdown::before` is an 8px
/// square rotated 45deg, filled with the body color and bordered — a classic
/// rotated-square arrow. It has a declared width/height (unlike the tooltip's
/// border triangle), so this is the simpler case: the box is exactly
/// `var(--rinch-popover-arrow-size, 8px)` on each side.
#[test]
fn a_popover_arrow_is_an_8px_rotated_square() {
    let app = mount(move |scope| {
        let target = PopoverTarget.render(scope, &[]);
        let item = scope.create_text("content");
        let dropdown = PopoverDropdown.render(scope, &[item]);
        Popover {
            opened: true,
            with_arrow: true,
            ..Default::default()
        }
        .render(scope, &[target, dropdown])
    });
    let panel = node_with_class(&app, "rinch-popover__dropdown");
    let arrow = pseudo_child(&app, panel);
    let (w, h) = layout_of(&app, arrow);
    assert_eq!(
        (w, h),
        (8.0, 8.0),
        "the default `--rinch-popover-arrow-size`"
    );
    assert_eq!(
        background_of(&app, arrow),
        Some(peniko::Color::from_rgb8(250, 250, 250)),
        "filled with `--rinch-color-body`"
    );
    let (bt, _, _, _) = border_widths(&app, arrow);
    assert_eq!(bt, 1.0, "1px border all round");
}

// ── 3. Divider with a label ────────────────────────────────────────────────

/// `.rinch-divider--with-label::before,::after` are the two flex strokes
/// either side of the label — `flex: 1` so they grow to fill whatever space
/// the label text leaves, `height: 1px`, colored from `--rinch-color-border`.
/// Both now exist and both draw.
#[test]
fn a_divider_with_a_label_gets_two_flex_strokes() {
    let app = mount(move |scope| {
        Divider {
            label: "Section".to_string(),
            ..Default::default()
        }
        .render(scope, &[])
    });
    let root = node_with_class(&app, "rinch-divider");
    let doc = app.doc.as_ref().unwrap();
    let pseudo_children: Vec<usize> = {
        let d = doc.borrow();
        d.tree
            .get(root)
            .unwrap()
            .children
            .iter()
            .copied()
            .filter(|&c| d.tree.get(c).unwrap().is_pseudo_element)
            .collect()
    };
    assert_eq!(
        pseudo_children.len(),
        2,
        "one stroke before the label, one after"
    );
    for &stroke in &pseudo_children {
        let (_, h) = layout_of(&app, stroke);
        assert_eq!(h, 1.0, "each stroke is a 1px line");
        assert_eq!(
            background_of(&app, stroke),
            Some(peniko::Color::from_rgb8(200, 200, 200)),
            "colored from `--rinch-color-border`"
        );
    }
    // The two strokes split whatever width the label leaves (`flex: 1` on a
    // row with the label between them), so unlike the fixed-size arrows their
    // width is layout-dependent, not asserted here.
}

// ── 4. Dot badge ───────────────────────────────────────────────────────────

/// `.rinch-badge--dot::before` is an 8px round dot, filled with the primary
/// color.
///
/// `.rinch-badge` is `display: inline-flex` (`styles/badge.rs`), so the dot is
/// a flex item of it — and a flex item's specified `display: inline-block`
/// **blockifies** to `block` (css-display-3 §2.7), which Chrome does too
/// (measured: `getComputedStyle` of a `::before` with `display:
/// inline-block` inside a `display: flex` parent reports `block`). The #542
/// machinery in `resolve_pseudo_element` (`layout_parent_is_flex_or_grid`)
/// exists for exactly this.
#[test]
fn a_dot_badge_gets_an_8px_round_dot() {
    let app = mount(move |scope| {
        let text = scope.create_text("New");
        Badge {
            variant: "dot".to_string(),
            ..Default::default()
        }
        .render(scope, &[text])
    });
    let root = node_with_class(&app, "rinch-badge");
    let dot = pseudo_child(&app, root);
    let (w, h) = layout_of(&app, dot);
    assert_eq!((w, h), (8.0, 8.0), "0.5rem at 16px root");
    assert_eq!(
        display_of(&app, dot),
        DisplayValue::Block,
        "the rule declares `display: inline-block`, blockified to `block` \
         because the dot is a flex item of `.rinch-badge`"
    );
    assert_eq!(
        background_of(&app, dot),
        Some(peniko::Color::from_rgb8(20, 90, 200)),
        "filled with `--rinch-primary-color`"
    );
}

// ── 5. The one rule that still matches nothing ────────────────────────────

/// `.rinch-button__loader::after`'s spinner rule is still dead — not because
/// of #773, but because `Button::render` never creates the
/// `.rinch-button__loader` div the rule is scoped under in the first place.
/// `content: ''` creating a pseudo-element doesn't help a selector whose
/// ancestor half matches no element at all. Filed as a separate issue
/// (gh issue: Refs #773 in the audit) rather than fixed here, since it needs
/// a markup change to `Button::render`, not a pseudo-element fix.
#[test]
fn a_loading_buttons_spinner_rule_matches_no_element() {
    let app = mount(move |scope| {
        let text = scope.create_text("Save");
        rinch_components::Button {
            loading: true,
            ..Default::default()
        }
        .render(scope, &[text])
    });
    assert!(
        nodes_with_class(&app, "rinch-button__loader").is_empty(),
        "`Button::render` does not create a `.rinch-button__loader` div even \
         when `loading: true` — the spinner rule has nothing to match, with \
         or without #773's fix"
    );
}
