//! Review fixtures for PR #1589 (Tree drop places). Each test pins a finding
//! of the review; the ones marked FINDING fail at the PR head.

use super::*;
use std::cell::RefCell;
use std::collections::HashSet;

use rinch_components::{
    DropVerdict, Tree, TreeDrop, TreeNodeData, TreeSlot, UseTreeOptions, UseTreeReturn,
};
use rinch_core::Component;
use rinch_core::element::ValueCallback;

const VP: (f32, f32) = (800.0, 600.0);

type Offers = Rc<RefCell<Vec<Option<(TreeDrop, DropVerdict)>>>>;

fn mount(data: Vec<TreeNodeData>, open: &[&str], indent: Option<f32>, offers: Offers) -> RinchApp {
    let open: HashSet<String> = open.iter().map(|s| s.to_string()).collect();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let o = offers.clone();
        let tree = Tree {
            data,
            tree: Some(UseTreeReturn::new(UseTreeOptions {
                initial_expanded: open,
                ..Default::default()
            })),
            indent,
            draggable: true,
            ondropchange: Some(ValueCallback::new(move |offer| o.borrow_mut().push(offer))),
            ..Default::default()
        }
        .render(scope, &[]);
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 300px");
        root.append_child(&tree);
        root
    });
    app.mount_component(VP.0, VP.1);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VP.0, VP.1);
    app
}

fn send(app: &mut RinchApp, event: PlatformEvent) {
    app.handle_event(event, (VP.0 as u32, VP.1 as u32), 1.0);
    app.resolve_and_repaint(VP.0, VP.1);
}

fn has_class(n: &rinch_dom::Node, class: &str) -> bool {
    n.attributes
        .get("class")
        .is_some_and(|c| c.split_whitespace().any(|one| one == class))
}

fn row_box(app: &RinchApp, value: &str) -> (f32, f32, f32, f32) {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let li = d
        .tree
        .nodes
        .iter()
        .find(|(_, n)| n.attributes.get("data-value").map(String::as_str) == Some(value))
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no row {value}"));
    let content = d.tree.nodes[li]
        .children
        .iter()
        .copied()
        .find(|&c| has_class(&d.tree.nodes[c], "rinch-tree__node-content"))
        .unwrap();
    hit_testing::painted_element_box(&d.tree, content)
}

fn at(app: &RinchApp, value: &str, x: f32, fraction: f32) -> (f32, f32) {
    let (bx, by, _, h) = row_box(app, value);
    (bx + x, by + h * fraction)
}

fn start(app: &mut RinchApp, from: &str) {
    let (x, y) = at(app, from, 120.0, 0.5);
    send(
        app,
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
    );
    send(
        app,
        PlatformEvent::MouseMove {
            x: x + 20.0,
            y: y + 1.0,
        },
    );
    assert!(
        app.active_dnd.is_some(),
        "positive control: the drag went live"
    );
}

fn move_to(app: &mut RinchApp, value: &str, x: f32, fraction: f32) {
    let (x, y) = at(app, value, x, fraction);
    send(app, PlatformEvent::MouseMove { x, y });
}

/// FINDING 1. An expanded row that is the last of its group cannot be dragged
/// out of the group when nothing follows it: its own bottom edge is
/// "first inside itself" (refused as a cycle) and every row under it offers
/// nothing.
///
/// x (open)
/// ├ x1
/// └ y (open)   <- dragged
///   └ z
#[test]
fn an_expanded_last_row_outdents_over_its_own_bottom_edge() {
    let offers: Offers = Rc::default();
    let data = vec![TreeNodeData::new("x", "Ex").with_children(vec![
        TreeNodeData::new("x1", "Ex one"),
        TreeNodeData::new("y", "Why").with_child(TreeNodeData::new("z", "Zed")),
    ])];
    let mut app = mount(data, &["x", "y"], Some(20.0), offers.clone());
    start(&mut app, "y");
    move_to(&mut app, "y", 2.0, 0.9);
    let got = offers.borrow().last().cloned().flatten();
    let (drop, verdict) = got.expect("an offer over y's own bottom edge");
    assert_eq!(
        (drop.parent.clone(), drop.slot.clone(), verdict),
        (None, TreeSlot::After("x".into()), DropVerdict::Allowed),
        "y's bottom edge at column 0 should move y out of x, after it"
    );
}

/// FINDING 2. Without `indent` the pointer column is never read, so when the
/// last root row is expanded no place offers "last at the top level".
///
/// a
/// c (open)
/// └ c1
#[test]
fn without_indent_the_end_of_the_top_level_is_reachable() {
    let offers: Offers = Rc::default();
    let data = vec![
        TreeNodeData::new("a", "Ay"),
        TreeNodeData::new("c", "Cee").with_child(TreeNodeData::new("c1", "Cee one")),
    ];
    let mut app = mount(data, &["c"], None, offers.clone());
    start(&mut app, "a");
    for value in ["a", "c", "c1"] {
        for x in [1.0, 8.0, 120.0] {
            for f in [0.05, 0.5, 0.95] {
                move_to(&mut app, value, x, f);
            }
        }
    }
    let seen: Vec<_> = offers
        .borrow()
        .iter()
        .flatten()
        .map(|(d, _)| (d.parent.clone(), d.slot.clone()))
        .collect();
    assert!(!seen.is_empty(), "positive control: offers were made");
    assert!(
        seen.iter()
            .any(|(p, s)| p.is_none() && *s == TreeSlot::After("c".into())),
        "no place offers the end of the top level: {seen:?}"
    );
}

/// FINDING 3. Moving from one row straight onto the next reports `None` in
/// between (the leave of the old row runs before the over of the new one),
/// so an app announcing the offer hears "nothing" at every row crossing.
#[test]
fn crossing_between_adjacent_rows_reports_no_none_in_between() {
    let offers: Offers = Rc::default();
    let data = vec![
        TreeNodeData::new("p", "Pea"),
        TreeNodeData::new("q", "Cue"),
        TreeNodeData::new("r", "Arr"),
    ];
    let mut app = mount(data, &[], Some(20.0), offers.clone());
    start(&mut app, "p");
    move_to(&mut app, "q", 120.0, 0.9);
    let mark = offers.borrow().len();
    assert!(
        offers.borrow().last().cloned().flatten().is_some(),
        "positive control: an offer over q"
    );
    move_to(&mut app, "r", 120.0, 0.1);
    let after: Vec<_> = offers.borrow()[mark..].to_vec();
    assert!(
        after.last().cloned().flatten().is_some(),
        "positive control: an offer over r"
    );
    assert!(
        after.iter().all(|o| o.is_some()),
        "a None was reported between two rows: {after:?}"
    );
}

/// FINDING 4 (claim): "Nothing changes for a Tree that does not set
/// `draggable`". Every row is now `position: relative`, draggable or not.
#[test]
fn a_plain_tree_row_is_not_positioned() {
    let offers: Offers = Rc::default();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let _ = &offers;
        let tree = Tree {
            data: vec![TreeNodeData::new("a", "Ay"), TreeNodeData::new("b", "Bee")],
            tree: Some(UseTreeReturn::new(UseTreeOptions::default())),
            ..Default::default()
        }
        .render(scope, &[]);
        let root = scope.create_element("div");
        root.append_child(&tree);
        root
    });
    app.mount_component(VP.0, VP.1);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VP.0, VP.1);
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let rows: Vec<_> = d
        .tree
        .nodes
        .iter()
        .filter(|(_, n)| has_class(n, "rinch-tree__node-content"))
        .collect();
    assert_eq!(rows.len(), 2, "positive control");
    for (_, n) in rows {
        assert_eq!(
            format!("{:?}", n.computed_style.position),
            "Static",
            "a non-draggable tree's row became positioned"
        );
    }
}

/// Measurement only (prints): a 1000-row plain (non-draggable) Tree in a
/// scroller, second wheel notch and a warm pointer move, with the PR's
/// component CSS and with its `position: relative` row rule removed.
#[cfg(software_shell)]
#[test]
fn measure_plain_tree_1000_rows_with_and_without_positioned_rows() {
    use super::perf_expect::*;
    for strip in [false, true] {
        let mut app = new_app(move |scope: &mut RenderScope| {
            let data: Vec<TreeNodeData> = (0..1000)
                .map(|i| TreeNodeData::new(format!("v{i}"), format!("row {i}")))
                .collect();
            let tree = Tree {
                data,
                tree: Some(UseTreeReturn::new(UseTreeOptions::default())),
                ..Default::default()
            }
            .render(scope, &[]);
            let root = scope.create_element("div");
            root.set_attribute("style", "width: 300px; height: 400px; overflow-y: auto; font-family: sans-serif; line-height: 20px");
            root.append_child(&tree);
            root
        });
        app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
        {
            let doc = app.doc.as_ref().unwrap();
            let mut d = doc.borrow_mut();
            d.load_css(&rinch_theme::generate_theme_css(
                &rinch_theme::Theme::default(),
            ));
            let mut css = rinch_components::generate_component_css();
            if strip {
                let rule = ".rinch-tree__node-content {\n    position: relative;\n}";
                assert!(
                    css.contains(rule),
                    "positive control: the rule is there to strip"
                );
                css = css.replace(rule, "");
            }
            d.load_css(&css);
            d.recompute_all_styles_full();
        }
        settle(&mut app);
        let wheel = |app: &mut RinchApp| {
            app.handle_event(
                PlatformEvent::MouseWheel {
                    x: 50.0,
                    y: 100.0,
                    delta_x: 0.0,
                    delta_y: -100.0,
                },
                SIZE,
                1.0,
            );
        };
        interaction(&mut app, wheel);
        let notch = interaction(&mut app, wheel);
        interaction(&mut app, |a| {
            a.handle_event(PlatformEvent::MouseMove { x: 60.0, y: 150.0 }, SIZE, 1.0);
        });
        let mv = interaction(&mut app, |a| {
            a.handle_event(PlatformEvent::MouseMove { x: 61.0, y: 151.0 }, SIZE, 1.0);
        });
        println!("strip={strip} NOTCH:\n{}", baseline_text(&notch));
        println!("strip={strip} MOVE:\n{}", baseline_text(&mv));
    }
}

/// Kills M8 (the shape is not rebuilt when the data changes mid-drag): a row
/// that arrives during a drag (children loaded after a hover-to-expand, the
/// case the PR names) is a drop target like any other.
#[test]
fn a_row_that_arrives_mid_drag_offers_drops() {
    let data = rinch_core::Signal::new(vec![
        TreeNodeData::new("x", "Ex"),
        TreeNodeData::new("y", "Why"),
    ]);
    let offers: Offers = Rc::default();
    let o = offers.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let o = o.clone();
        let tree = Tree {
            tree: Some(UseTreeReturn::new(UseTreeOptions {
                initial_expanded: ["y".to_string()].into_iter().collect(),
                ..Default::default()
            })),
            data_source: Some(Rc::new(move || data.get())),
            indent: Some(20.0),
            draggable: true,
            ondropchange: Some(ValueCallback::new(move |offer| o.borrow_mut().push(offer))),
            ..Default::default()
        }
        .render(scope, &[]);
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 300px");
        root.append_child(&tree);
        root
    });
    app.mount_component(VP.0, VP.1);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VP.0, VP.1);
    start(&mut app, "x");
    move_to(&mut app, "y", 120.0, 0.5);
    data.set(vec![
        TreeNodeData::new("x", "Ex"),
        TreeNodeData::new("y", "Why").with_child(TreeNodeData::new("y1", "Child")),
    ]);
    app.resolve_and_repaint(VP.0, VP.1);
    move_to(&mut app, "y1", 120.0, 0.5);
    let got = offers.borrow().last().cloned().flatten();
    let (drop, _) = got.expect("an offer over the row that arrived mid-drag");
    assert_eq!(
        (drop.over.as_str(), drop.parent.as_deref()),
        ("y1", Some("y1"))
    );
}

mod hover_expand {
    //! Kills M9: resting inside a collapsed row with children expands it.
    //! Uses a manual timer backend (process-global; run filtered, or share
    //! `overlay_dismiss_tests::auto_close`'s lock when landing this).
    use super::*;

    thread_local! {
        static SCHEDULED: RefCell<Vec<(u64, u32)>> = const { RefCell::new(Vec::new()) };
    }
    fn manual(id: u64, delay_ms: u32) {
        SCHEDULED.with(|s| s.borrow_mut().push((id, delay_ms)));
    }

    #[test]
    fn resting_inside_a_collapsed_row_expands_it() {
        rinch_core::set_timer_backend(manual);
        let state_out: Rc<RefCell<Option<UseTreeReturn>>> = Rc::default();
        let s_out = state_out.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let state = UseTreeReturn::new(UseTreeOptions::default());
            *s_out.borrow_mut() = Some(state);
            let tree = Tree {
                data: vec![
                    TreeNodeData::new("x", "Ex"),
                    TreeNodeData::new("y", "Why").with_child(TreeNodeData::new("y1", "Child")),
                ],
                tree: Some(state),
                draggable: true,
                hover_expand_ms: 321,
                ..Default::default()
            }
            .render(scope, &[]);
            let root = scope.create_element("div");
            root.set_attribute("style", "width: 300px");
            root.append_child(&tree);
            root
        });
        app.mount_component(VP.0, VP.1);
        {
            let doc = app.doc.as_ref().unwrap();
            let mut d = doc.borrow_mut();
            d.load_css(&rinch_components::generate_component_css());
            d.recompute_all_styles_full();
        }
        app.resolve_and_repaint(VP.0, VP.1);
        let state = state_out.borrow().unwrap();
        SCHEDULED.with(|s| s.borrow_mut().clear());
        start(&mut app, "x");
        move_to(&mut app, "y", 120.0, 0.5);
        let armed: Vec<(u64, u32)> = SCHEDULED.with(|s| std::mem::take(&mut *s.borrow_mut()));
        assert_eq!(
            armed.iter().filter(|(_, ms)| *ms == 321).count(),
            1,
            "one hover timer armed: {armed:?}"
        );
        assert!(
            !state.expanded.get().contains("y"),
            "not expanded before the deadline"
        );
        for (id, _) in armed {
            rinch_core::fire_timeout(id);
        }
        assert!(
            state.expanded.get().contains("y"),
            "expanded at the deadline"
        );
    }
}
