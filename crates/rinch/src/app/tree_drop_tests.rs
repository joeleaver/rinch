//! A `Tree` with `draggable` rows, driven through the real drag path: the
//! drop on offer follows the part of the row under the pointer, one
//! indicator is drawn inside that row, a refused drop is drawn as refused
//! and never delivered, and an allowed one arrives as a `TreeDrop`.

use super::*;
use std::cell::RefCell;

use rinch_components::{
    DropVerdict, DropZone, Tree, TreeDrop, TreeNodeData, TreeSlot, UseTreeOptions, UseTreeReturn,
};
use rinch_core::Component;
use rinch_core::element::ValueCallback;

const VIEWPORT: (f32, f32) = (800.0, 600.0);
const INDENT: f32 = 20.0;

struct Fixture {
    app: RinchApp,
    drops: Rc<RefCell<Vec<TreeDrop>>>,
    offers: Rc<RefCell<Vec<Option<(TreeDrop, DropVerdict)>>>>,
    started: Rc<RefCell<Vec<Vec<String>>>>,
}

/// a (open)
/// ├ a1
/// └ a2
/// b
/// Drops into `b` are refused by the app.
fn fixture() -> Fixture {
    let drops: Rc<RefCell<Vec<TreeDrop>>> = Rc::default();
    let offers: Rc<RefCell<Vec<Option<(TreeDrop, DropVerdict)>>>> = Rc::default();
    let started: Rc<RefCell<Vec<Vec<String>>>> = Rc::default();
    let (d, o, s) = (drops.clone(), offers.clone(), started.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let state = UseTreeReturn::new(UseTreeOptions {
            initial_expanded: ["a".to_string()].into_iter().collect(),
            ..Default::default()
        });
        let (d, o, s) = (d.clone(), o.clone(), s.clone());
        let tree = Tree {
            data: vec![
                TreeNodeData::new("a", "Alpha").with_children(vec![
                    TreeNodeData::new("a1", "One"),
                    TreeNodeData::new("a2", "Two"),
                ]),
                TreeNodeData::new("b", "Beta"),
            ],
            tree: Some(state),
            indent: Some(INDENT),
            draggable: true,
            judge_drop: Some(Rc::new(|drop: &TreeDrop| {
                if drop.parent.as_deref() == Some("b") {
                    DropVerdict::Refused
                } else {
                    DropVerdict::Allowed
                }
            })),
            ondropchange: Some(ValueCallback::new(move |offer| o.borrow_mut().push(offer))),
            ondrop: Some(ValueCallback::new(move |drop| d.borrow_mut().push(drop))),
            ondragstart: Some(ValueCallback::new(move |values| s.borrow_mut().push(values))),
            ..Default::default()
        }
        .render(scope, &[]);
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 300px");
        root.append_child(&tree);
        root
    });
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(&rinch_theme::Theme::default()));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    Fixture { app, drops, offers, started }
}

fn send(app: &mut RinchApp, event: PlatformEvent) {
    app.handle_event(event, (VIEWPORT.0 as u32, VIEWPORT.1 as u32), 1.0);
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
}

fn has_class(n: &rinch_dom::Node, class: &str) -> bool {
    n.attributes
        .get("class")
        .is_some_and(|c| c.split_whitespace().any(|one| one == class))
}

/// The content box (the row) of the tree item whose value is `value`.
fn row(app: &RinchApp, value: &str) -> usize {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let li = d
        .tree
        .nodes
        .iter()
        .find(|(_, n)| n.attributes.get("data-value").map(String::as_str) == Some(value))
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no row for {value}"));
    d.tree.nodes[li]
        .children
        .iter()
        .copied()
        .find(|&c| has_class(&d.tree.nodes[c], "rinch-tree__node-content"))
        .expect("the item has a content box")
}

fn box_of(app: &RinchApp, id: usize) -> (f32, f32, f32, f32) {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    hit_testing::painted_element_box(&d.tree, id)
}

/// A point `fraction` of the way down the row of `value`, `x` pixels in.
fn at(app: &RinchApp, value: &str, x: f32, fraction: f32) -> (f32, f32) {
    let (bx, by, _, h) = box_of(app, row(app, value));
    (bx + x, by + h * fraction)
}

/// The indicator's host row and its classes, when it is drawn.
fn indicator(app: &RinchApp) -> Option<(usize, String)> {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .nodes
        .iter()
        .find(|(_, n)| has_class(n, "rinch-tree__drop-indicator") && n.parent.is_some())
        .map(|(_, n)| (n.parent.unwrap(), n.attributes.get("class").cloned().unwrap_or_default()))
}

/// Press on `from`'s row and move far enough to start a drag.
fn start(f: &mut Fixture, from: &str) {
    let (x, y) = at(&f.app, from, 120.0, 0.5);
    send(&mut f.app, PlatformEvent::MouseDown { x, y, button: MouseButton::Left });
    send(&mut f.app, PlatformEvent::MouseMove { x: x + 20.0, y: y + 1.0 });
    assert!(f.app.active_dnd.is_some(), "the drag went live");
}

/// Move the pointer `fraction` of the way down `value`'s row, `x` pixels in.
fn move_to(f: &mut Fixture, value: &str, x: f32, fraction: f32) {
    let (x, y) = at(&f.app, value, x, fraction);
    send(&mut f.app, PlatformEvent::MouseMove { x, y });
}

fn last_offer(f: &Fixture) -> Option<(TreeDrop, DropVerdict)> {
    f.offers.borrow().last().cloned().flatten()
}

#[test]
fn the_offer_follows_the_part_of_the_row_under_the_pointer() {
    let mut f = fixture();
    start(&mut f, "a1");
    assert_eq!(*f.started.borrow(), vec![vec!["a1".to_string()]]);

    // The top quarter of a2: before it, so after a1, which is the dragged row
    // itself; the tree offers it as it is.
    move_to(&mut f, "a2", 120.0, 0.1);
    let (drop, verdict) = last_offer(&f).expect("an offer");
    assert_eq!((drop.parent.as_deref(), drop.slot.clone(), drop.zone), (Some("a"), TreeSlot::After("a1".into()), DropZone::Before));
    assert_eq!(verdict, DropVerdict::Allowed);
    let (host, class) = indicator(&f.app).expect("the indicator is drawn");
    assert_eq!(host, row(&f.app, "a2"));
    assert!(class.contains("rinch-tree__drop-indicator--line"), "{class}");

    // The middle of a2: into it, last; drawn as an outline.
    move_to(&mut f, "a2", 120.0, 0.5);
    let (drop, _) = last_offer(&f).expect("an offer");
    assert_eq!((drop.parent.as_deref(), drop.slot.clone()), (Some("a2"), TreeSlot::Last));
    assert!(indicator(&f.app).unwrap().1.contains("rinch-tree__drop-indicator--inside"));

    // The bottom of a2, the last row of a, with the pointer at the first
    // indent column: out of a, after it.
    move_to(&mut f, "a2", INDENT * 0.5, 0.9);
    let (drop, _) = last_offer(&f).expect("an offer");
    assert_eq!((drop.parent.clone(), drop.slot.clone()), (None, TreeSlot::After("a".into())));
    // And with the pointer further in: after a2, inside a.
    move_to(&mut f, "a2", 120.0, 0.9);
    let (drop, _) = last_offer(&f).expect("an offer");
    assert_eq!((drop.parent.as_deref(), drop.slot.clone()), (Some("a"), TreeSlot::After("a2".into())));

    // Released there: delivered once, as offered.
    let (x, y) = at(&f.app, "a2", 120.0, 0.9);
    send(&mut f.app, PlatformEvent::MouseUp { x, y, button: MouseButton::Left });
    let drops = f.drops.borrow();
    assert_eq!(drops.len(), 1);
    assert_eq!((drops[0].dragged.clone(), drops[0].parent.as_deref(), drops[0].slot.clone()), (vec!["a1".to_string()], Some("a"), TreeSlot::After("a2".into())));
    assert!(indicator(&f.app).is_none(), "the indicator is gone after the drop");
    assert_eq!(f.offers.borrow().last().cloned(), Some(None), "the app was told the offer ended");
}

#[test]
fn a_refused_drop_is_drawn_as_refused_and_not_delivered() {
    let mut f = fixture();
    start(&mut f, "a1");
    move_to(&mut f, "b", 120.0, 0.5);
    let (drop, verdict) = last_offer(&f).expect("an offer");
    assert_eq!(drop.parent.as_deref(), Some("b"));
    assert_eq!(verdict, DropVerdict::Refused);
    assert!(indicator(&f.app).unwrap().1.contains("rinch-tree__drop-indicator--refused"));
    let (x, y) = at(&f.app, "b", 120.0, 0.5);
    send(&mut f.app, PlatformEvent::MouseUp { x, y, button: MouseButton::Left });
    assert!(f.drops.borrow().is_empty(), "a refused drop is not delivered");
}

#[test]
fn a_row_cannot_go_into_its_own_subtree_and_its_own_rows_offer_nothing() {
    let mut f = fixture();
    start(&mut f, "a");
    // Over a's own children: nothing on offer, nothing drawn.
    move_to(&mut f, "a2", 120.0, 0.5);
    assert!(last_offer(&f).is_none());
    assert!(indicator(&f.app).is_none());
    // The dragged row is dimmed while it is carried.
    {
        let doc = f.app.doc.as_ref().unwrap();
        let d = doc.borrow();
        assert!(has_class(&d.tree.nodes[row(&f.app, "a")], "rinch-tree__node-content--dragging"));
    }
    let (x, y) = at(&f.app, "a2", 120.0, 0.5);
    send(&mut f.app, PlatformEvent::MouseUp { x, y, button: MouseButton::Left });
    assert!(f.drops.borrow().is_empty());
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    assert!(!has_class(&d.tree.nodes[row(&f.app, "a")], "rinch-tree__node-content--dragging"));
}
