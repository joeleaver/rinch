//! An `inline-flex` component beside text keeps its old width when its label
//! changes (issue #661's forecast, measured on a real component).
//!
//! The issue predicted this and did not test it: `Badge` declares
//! `display: inline-flex`, so a `Badge` inside a paragraph — rather than inside
//! a `Stack`, where CSS blockifies it and none of this applies — is an **atomic
//! inline**. Its box is detached from its parent's Taffy child list so the
//! enclosing inline formatting context can measure it as an `InlineBox`, which
//! means the root Taffy compute never reaches it and only the `ifc_dirty` pass
//! ever sized one. A reactive label update is a `set_text_content`, which sets
//! neither flag.
//!
//! Measured here through the **real** component and theme stylesheets, because
//! the `inline-flex` that makes this reachable comes from the component sheet
//! and not from anything the fixture declares.

use super::*;

use rinch_components::Badge;
use rinch_core::Component;
use rinch_core::dom::DomDocument;

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

/// `<p>Unread <Badge>{label}</Badge></p>` — the badge is inline-level beside
/// text, which is the shape the forecast is about.
fn badge_beside_text(label: &'static str) -> RinchApp {
    mount(move |scope: &mut RenderScope| {
        let para = scope.create_element("p");
        let lead = scope.create_text("Unread ");
        para.append_child(&lead);
        let text = scope.create_text(label);
        let badge = Badge {
            ..Default::default()
        }
        .render(scope, &[text]);
        para.append_child(&badge);
        para
    })
}

/// The one `.rinch-badge` node's id.
fn badge_id(app: &RinchApp) -> usize {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let matches: Vec<usize> = d
        .tree
        .nodes
        .iter()
        .filter(|(_, n)| {
            n.attributes
                .get("class")
                .is_some_and(|c| c.split_whitespace().any(|one| one == "rinch-badge"))
        })
        .map(|(id, _)| id)
        .collect();
    assert_eq!(matches.len(), 1, "expected exactly one `.rinch-badge` node");
    matches[0]
}

fn badge_box(app: &RinchApp) -> (f32, f32) {
    let id = badge_id(app);
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let n = d.tree.get(id).expect("the badge is in the tree");
    (n.layout.width, n.layout.height)
}

/// The badge's own text node (its only text descendant).
fn badge_text_node(app: &RinchApp) -> rinch_core::dom::NodeId {
    let badge = badge_id(app);
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let mut stack = vec![badge];
    while let Some(id) = stack.pop() {
        let n = d.tree.get(id).expect("live node");
        if matches!(n.kind, rinch_dom::node::NodeKind::Text(_)) {
            return rinch_core::dom::NodeId(id);
        }
        stack.extend(n.children.iter().copied());
    }
    panic!("the badge has a text descendant");
}

/// A `Badge` beside text re-measures when its label grows.
///
/// Kills the mutant that drops `remeasure_dirty_atomic_inlines` from
/// `resolve_layout`, and the one that drops `mark_atomic_inline_dirty` from
/// `set_text_content` (verified against both: the badge comes back at the `9`
/// width against a `9999999` oracle).
///
/// The counter-oracle is the same document built at the long label: if the
/// component's padding happened to dominate its own text, the two widths would
/// coincide and this fixture would pass without discriminating anything.
#[test]
fn an_inline_flex_badge_beside_text_remeasures_when_its_label_grows() {
    let mut app = badge_beside_text("9");
    let short = badge_box(&app);

    let text = badge_text_node(&app);
    {
        let doc = app.doc.as_ref().unwrap();
        doc.borrow_mut().set_text_content(text, "9999999");
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);

    let oracle = badge_beside_text("9999999");
    let expected = badge_box(&oracle);

    assert_ne!(
        short, expected,
        "counter-oracle: a seven-character label must make a wider badge than a \
         one-character one, or this fixture discriminates nothing"
    );
    assert_eq!(
        badge_box(&app),
        expected,
        "the badge must be re-measured around its new label; {short:?} is the \
         box it was first measured at"
    );
}
