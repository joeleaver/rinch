//! #674's blast radius on the component library, measured through a **real
//! cascade** rather than by reading the stylesheets.
//!
//! #674 gave `p`, `blockquote`, `figure`, `ul`, `ol` and `pre` the browser's
//! `margin-block: 1em`, plus a `margin-inline: 40px` on `blockquote`/`figure`
//! and a whole box on `<hr>`. Seven components render exactly those tags:
//!
//! | component | tag it renders |
//! |-----------|----------------|
//! | `Divider` | `<hr class="rinch-divider">` |
//! | `Code` (block) | `<pre class="rinch-code rinch-code--block">` |
//! | `List` | `<ul>` / `<ol class="rinch-list">` |
//! | `Breadcrumbs` | `<ol class="rinch-breadcrumbs__list">` |
//! | `Tree` | `<ul class="rinch-tree">` |
//! | `Image` | `<figure class="rinch-image__wrapper">` |
//! | `Blockquote` | `<blockquote class="rinch-blockquote">` |
//!
//! Every one of them declares `margin: 0` in its own CSS, so the author cascade
//! beats the new UA rules and none of them moves. That sentence is the claim
//! this file exists to check, and reading the stylesheets cannot check it:
//! `rinch-components`' own tests assert that a rule exists, which says nothing
//! about **origin**, and `.rinch-code`'s `margin: 0` did not exist until #674 —
//! a `Code` block *would* have grown a line of space above and below it.
//!
//! **The theme sheet is loaded as well as the component sheet**, for the reason
//! `component_radius_tests` records: a `var()` with no definition is invalid at
//! computed-value time, which for `margin` computes to `unset` = `0` and would
//! make every assertion here pass for the wrong reason. Loading the theme makes
//! the declarations real. `a_bare_paragraph_does_take_the_new_margin` is the
//! positive control that the UA rules are in play at all under this harness —
//! without it, a mount that failed to apply them would read as a clean bill of
//! health for all seven.

use super::*;

use rinch_components::{Blockquote, Breadcrumbs, Code, Divider, Image, List, Tree, TreeNodeData};
use rinch_core::Component;

const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// Mount `build`'s tree under the real theme **and** component stylesheets.
fn mount(build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> RinchApp {
    let mut app = RinchApp::new(build);
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

/// The resolved `(top, bottom, left, right)` margins of the one node carrying
/// `class`.
fn margins(app: &RinchApp, class: &str) -> (f32, f32, f32, f32) {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let matches: Vec<usize> = d
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
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one node carrying `{class}`, found {}",
        matches.len()
    );
    let style = &d
        .tree
        .get(matches[0])
        .expect("the node is in the tree")
        .computed_style;
    use rinch_dom::computed_style::values::LengthPercentageAutoValue as M;
    let read = |v: &M| match v {
        M::Length(px) => *px,
        other => panic!("`{class}` has a non-length margin: {other:?}"),
    };
    (
        read(&style.margin_top),
        read(&style.margin_bottom),
        read(&style.margin_left),
        read(&style.margin_right),
    )
}

fn border_widths(app: &RinchApp, class: &str) -> (f32, f32, f32, f32) {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let id = d
        .tree
        .nodes
        .iter()
        .find(|(_, n)| {
            n.attributes
                .get("class")
                .is_some_and(|c| c.split_whitespace().any(|one| one == class))
        })
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no node carries `{class}`"));
    let s = &d.tree.get(id).unwrap().computed_style;
    use rinch_dom::computed_style::LengthPercentageValue as L;
    let read = |v: &L| match v {
        L::Length(px) => *px,
        L::Zero => 0.0,
        other => panic!("`{class}` has a non-length border width: {other:?}"),
    };
    (
        read(&s.border_top_width),
        read(&s.border_bottom_width),
        read(&s.border_left_width),
        read(&s.border_right_width),
    )
}

/// The positive control. A bare `<p>` under this exact harness **does** take the
/// new UA margin, so the seven "it did not move" assertions below are about the
/// author cascade winning and not about the UA rules being absent.
#[test]
fn a_bare_paragraph_does_take_the_new_margin() {
    let app = mount(|scope: &mut RenderScope| {
        let p = scope.create_element("p");
        p.set_attribute("class", "control-paragraph");
        p
    });
    let (top, bottom, _, _) = margins(&app, "control-paragraph");
    assert_eq!(
        (top, bottom),
        (16.0, 16.0),
        "the UA `p` rule must be in play under this harness (1em at the 16px root)"
    );
}

/// `Divider` is an `<hr>`, the element #674 changed most, and its own CSS zeroes
/// both the border and the margins the new UA rule adds.
#[test]
fn a_divider_keeps_its_own_box() {
    let app = mount(|scope: &mut RenderScope| {
        Divider {
            ..Default::default()
        }
        .render(scope, &[])
    });
    assert_eq!(
        margins(&app, "rinch-divider"),
        (0.0, 0.0, 0.0, 0.0),
        "the UA <hr> margins (0.5em block, auto inline) must lose to the component"
    );
    assert_eq!(
        border_widths(&app, "rinch-divider"),
        (0.0, 0.0, 0.0, 0.0),
        "and so must the UA <hr> 1px border — a Divider draws itself with a background"
    );
}

/// `Code { block: true }` is a `<pre>`, so it takes the new `margin-block: 1em`
/// unless its own CSS says otherwise. It did not say otherwise before #674.
///
/// This is the one component the change actually moved, and the fixture that
/// goes red if `.rinch-code`'s `margin: 0` is ever dropped again.
#[test]
fn a_code_block_gained_no_space_around_it() {
    let app = mount(|scope: &mut RenderScope| {
        let t = scope.create_text("let x = 1;");
        Code {
            block: true,
            ..Default::default()
        }
        .render(scope, &[t])
    });
    assert_eq!(
        margins(&app, "rinch-code--block"),
        (0.0, 0.0, 0.0, 0.0),
        "a Code block is a <pre>; without `.rinch-code {{ margin: 0 }}` it would \
         grow a line of space above and below (#674)"
    );
}

/// The remaining five: `List`, `Breadcrumbs`, `Tree`, `Image`, `Blockquote`.
///
/// `Blockquote` and `Image` are the interesting pair — their tags take a
/// `margin-inline: 40px` as well as the block margins, so a missing `margin: 0`
/// would indent them, not merely space them.
#[test]
fn the_other_five_list_and_figure_components_are_unmoved() {
    let app = mount(|scope: &mut RenderScope| {
        let item = scope.create_text("one");
        let list = List {
            ..Default::default()
        }
        .render(scope, &[item]);

        let crumb = scope.create_text("home");
        let crumbs = Breadcrumbs {
            ..Default::default()
        }
        .render(scope, &[crumb]);

        let img = Image {
            src: "about:blank".into(),
            caption: "c".into(),
            ..Default::default()
        }
        .render(scope, &[]);

        let quoted = scope.create_text("quoted");
        let quote = Blockquote {
            ..Default::default()
        }
        .render(scope, &[quoted]);

        let tree = Tree {
            data: vec![TreeNodeData::new("a", "Alpha")],
            ..Default::default()
        }
        .render(scope, &[]);

        let root = scope.create_element("div");
        for child in [&list, &crumbs, &img, &quote, &tree] {
            root.append_child(child);
        }
        root
    });

    for class in [
        "rinch-list",
        "rinch-breadcrumbs__list",
        "rinch-image__wrapper",
        "rinch-blockquote",
        "rinch-tree",
    ] {
        assert_eq!(
            margins(&app, class),
            (0.0, 0.0, 0.0, 0.0),
            "`{class}` must keep its own `margin: 0` against the new UA rules"
        );
    }
}

/// The computed `font-size` of the one element with `tag` in the tree.
fn font_size_of_tag(app: &RinchApp, tag: &str) -> f32 {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let matches: Vec<usize> = d
        .tree
        .nodes
        .iter()
        .filter(|(_, n)| n.tag() == Some(tag))
        .map(|(id, _)| id)
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one <{tag}> in the tree, found {}",
        matches.len()
    );
    d.tree.get(matches[0]).unwrap().computed_style.font_size
}

/// **The rich-text editor is NOT unmoved: its `<sub>` and `<sup>` shrink.**
///
/// The editor's `subscript`/`superscript` marks serialise to literal `<sub>` and
/// `<sup>` elements (`rinch-editor-core`'s `MarkSpec::parse_html_tags`, consumed
/// by `rinch-editor-view`'s `view.rs`), and `DEFAULT_EDITOR_CSS` declares no
/// `font-size` for either tag — it covers `margin`, `font-family` and
/// `white-space` for every other tag #674 touched, which is why everything else
/// in the editor really is unmoved. So the new UA `small, sub, sup
/// { font-size: smaller }` reaches editor content, and shipped subscripts and
/// superscripts shrink by a factor of 1.2 on desktop.
///
/// That is **correct** — `rinch-web` has always rendered them that way, on the
/// browser's own UA sheet — and it is the one place #674 changes what the
/// editor draws. It had no test anywhere in the workspace, so a later change to
/// the `smaller` rule would have moved shipped editor content in silence.
///
/// This mounts a **real** `Editor`, so the stylesheet under test is the shipped
/// `DEFAULT_EDITOR_CSS` and not a paraphrase of it: adding a `sub`/`sup`
/// `font-size` to that sheet turns this fixture red, which is the point.
///
/// The editor container is 16px, so `smaller` is 13.3333px. The tolerance is
/// 0.005px, which is what separates the `smaller` keyword from a `0.83em`
/// approximation (13.28px).
#[test]
fn the_editors_sub_and_sup_do_take_the_new_smaller_rule() {
    let app = mount(|scope: &mut RenderScope| {
        crate::editor::Editor {
            content: "<p>x<sub>2</sub><sup>3</sup></p>".into(),
            ..Default::default()
        }
        .render(scope, &[])
    });

    // Positive control: the editor's own container size, which its stylesheet
    // does declare and the UA sheet must not disturb.
    let paragraph = font_size_of_tag(&app, "p");
    assert_eq!(
        paragraph, 16.0,
        "the editor's own `[data-pm-editor] {{ font-size: 16px }}` must still hold"
    );

    for tag in ["sub", "sup"] {
        let got = font_size_of_tag(&app, tag);
        assert!(
            (got - 13.3333).abs() < 0.005,
            "editor <{tag}> takes the UA `smaller` (Chrome's 13.3333px from 16px), got {got}"
        );
    }
}
