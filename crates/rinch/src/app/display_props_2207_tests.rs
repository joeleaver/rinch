//! #2207 on a laid-out tree: what the display components' new props and fixes
//! do once rinch-dom computes them. (Their rendered DOM is pinned in
//! `rinch-components/tests/display_props_2207.rs`.)
//!
//! Every box is declared (children are fixed-size blocks, text has a declared
//! font size and line height), so no number moves with the host's fonts. The
//! grid gap is 10px, not 0 or the theme's 16px `md`, so a gap that went
//! missing or fell back to the default reads differently.

use super::*;

use rinch_components::{SimpleGrid, Skeleton, Text, Title};
use rinch_core::Component;

const W: f32 = 600.0;
const H: f32 = 400.0;

/// Mount `build` at `width`, with the theme and component sheets and `css`.
fn mount(
    width: f32,
    css: &'static str,
    build: impl Fn(&mut RenderScope) -> NodeHandle + 'static,
) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| build(scope));
    app.mount_component(width, H);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.load_css(css);
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(width, H);
    app
}

/// A `width`×`height` block with `id`.
fn block(scope: &mut RenderScope, id: &str, style: &str) -> NodeHandle {
    let b = scope.create_element("div");
    b.set_attribute("id", id);
    b.set_attribute("style", &format!("display: block; {style}"));
    b
}

fn by_id(app: &RinchApp, id: &str) -> u64 {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .nodes
        .iter()
        .find(|(_, n)| n.attributes.get("id").is_some_and(|v| v == id))
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no #{id}"))
}

/// `(x, y, w, h)` on screen.
fn box_of(app: &RinchApp, id: &str) -> (f32, f32, f32, f32) {
    let node = by_id(app, id);
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    painted_element_box(&d.tree, node)
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.5
}

/// A SimpleGrid of four 20px-high cells `c0`..`c3`, 10px apart, with `class`
/// on the grid.
fn grid(cols: u32, class: &'static str) -> impl Fn(&mut RenderScope) -> NodeHandle + 'static {
    move |scope: &mut RenderScope| {
        let cells: Vec<NodeHandle> = (0..4)
            .map(|i| block(scope, &format!("c{i}"), "height: 20px"))
            .collect();
        let g = SimpleGrid {
            cols: Some(cols),
            spacing: "10px".into(),
            ..Default::default()
        }
        .render(scope, &cells);
        g.add_class(class);
        g
    }
}

// ------------------------------------------------------------ SimpleGrid

#[test]
fn simple_grid_columns_and_gap_are_unchanged() {
    let app = mount(W, "", grid(3, "g"));
    let col = (W - 20.0) / 3.0;
    let c0 = box_of(&app, "c0");
    let c1 = box_of(&app, "c1");
    let c3 = box_of(&app, "c3");
    assert!(
        near(c0.2, col) && near(c1.0, c0.0 + col + 10.0) && near(c1.1, c0.1),
        "three equal columns 10px apart; c0 {c0:?} c1 {c1:?}"
    );
    assert!(
        near(c3.0, c0.0) && near(c3.1, c0.1 + 30.0),
        "the fourth cell starts row 2, 10px below; c3 {c3:?}"
    );
}

/// The template was written inline, so no stylesheet could change it; a
/// breakpoint rule now does.
#[test]
fn an_app_breakpoint_rule_overrides_simple_grid_columns() {
    const CSS: &str = "@media (max-width: 500px) { \
        .one-col.rinch-simple-grid { grid-template-columns: 1fr; } }";

    // Narrow: the media rule applies and the grid is one column.
    let app = mount(400.0, CSS, grid(3, "one-col"));
    let c0 = box_of(&app, "c0");
    let c1 = box_of(&app, "c1");
    assert!(
        near(c1.0, c0.0) && near(c1.1, c0.1 + 30.0) && near(c0.2, 400.0),
        "under 500px the app's rule stacks the cells; c0 {c0:?} c1 {c1:?}"
    );

    // Wide: the rule does not apply and the prop's three columns stand.
    let app = mount(W, CSS, grid(3, "one-col"));
    let c0 = box_of(&app, "c0");
    let c1 = box_of(&app, "c1");
    assert!(
        near(c1.1, c0.1) && c1.0 > c0.0 + 100.0,
        "over 500px the prop's columns stand; c0 {c0:?} c1 {c1:?}"
    );
}

/// The variables inherit; a SimpleGrid inside another must keep its own
/// default (one column), not take the outer grid's `cols`.
#[test]
fn a_nested_simple_grid_does_not_inherit_the_outer_grids_props() {
    let app = mount(W, "", |scope: &mut RenderScope| {
        let a = block(scope, "a", "height: 20px");
        let b = block(scope, "b", "height: 20px");
        let inner = SimpleGrid::default().render(scope, &[a, b]);
        SimpleGrid {
            cols: Some(3),
            spacing: "10px".into(),
            ..Default::default()
        }
        .render(scope, &[inner])
    });
    let a = box_of(&app, "a");
    let b = box_of(&app, "b");
    assert!(
        near(b.0, a.0) && b.1 > a.1 + 19.0,
        "the inner grid is one column; a {a:?} b {b:?}"
    );
}

// -------------------------------------------------------------- Skeleton

/// The loaded wrapper was an inline `<span>`: in a flex row it was one item,
/// and its block children stacked inside it. It generates no box now, so
/// each child is an item of the row.
#[test]
fn a_loaded_skeletons_children_are_items_of_a_flex_parent() {
    let app = mount(W, "", |scope: &mut RenderScope| {
        let row = scope.create_element("div");
        row.set_attribute("style", "display: flex; gap: 10px");
        let a = block(scope, "a", "width: 50px; height: 20px");
        let b = block(scope, "b", "width: 50px; height: 20px");
        let sk = Skeleton {
            visible: false,
            ..Default::default()
        }
        .render(scope, &[a, b]);
        row.append_child(&sk);
        row
    });
    let a = box_of(&app, "a");
    let b = box_of(&app, "b");
    assert!(
        near(b.0, a.0 + 60.0) && near(b.1, a.1),
        "the two children sit side by side, 10px apart; a {a:?} b {b:?}"
    );
}

// ------------------------------------------------------------ Text, Title

#[test]
fn text_line_clamp_holds_the_box_to_that_many_lines() {
    let text = "one two three four five six seven eight nine ten eleven twelve";
    let build = |clamp: Option<u32>| {
        move |scope: &mut RenderScope| {
            let t = scope.create_text(text);
            let el = Text {
                line_clamp: clamp,
                ..Default::default()
            }
            .render(scope, &[t]);
            el.set_attribute("id", "t");
            el.set_style("width", "120px");
            el.set_style("font-size", "16px");
            el.set_style("line-height", "20px");
            el
        }
    };
    let free = box_of(&mount(W, "", build(None)), "t");
    assert!(
        free.3 >= 60.0 - 0.5,
        "precondition: unclamped, the text takes three lines or more; {free:?}"
    );
    let clamped = box_of(&mount(W, "", build(Some(2))), "t");
    assert!(
        near(clamped.3, 40.0),
        "clamped to two 20px lines; {clamped:?}"
    );
}

#[test]
fn a_title_css_size_sets_the_font_size_and_keeps_the_levels_line_height() {
    let build = |size: &'static str| {
        move |scope: &mut RenderScope| {
            let t = scope.create_text("Hi");
            let el = Title {
                order: Some(2),
                size: size.into(),
                ..Default::default()
            }
            .render(scope, &[t]);
            el.set_attribute("id", "t");
            el
        }
    };
    // h2's line height is 1.35: a 40px font makes a 54px line.
    let custom = box_of(&mount(W, "", build("40px")), "t");
    assert!(near(custom.3, 54.0), "40px × 1.35; {custom:?}");
}
