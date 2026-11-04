//! Sortable drag presentation options (#2154), through the real event path:
//! `preview` (none / runtime snapshot / a copy / the app's element),
//! `indicator` (caret / gap / both / none), `source` (dim / collapse / hide /
//! unchanged), and the drag-state signals an app draws its own from.
//!
//! Two vertical lists: c1 [a b c d] and c2 [e f], cards 30px tall with a 10px
//! gap between them (so a gap is 40px: the card and the spacing), side by
//! side. c2 refuses `d` with a reason.

use super::*;
use crate as rinch;
use std::cell::{Cell, RefCell};

use rinch_components::{
    SortDragState, SortIndicator, SortJudge, SortMove, SortPreview, SortPreviewFn, SortSource,
    SortTarget, SortVerdict, SortableItem, SortableList, sortable_drag, sortable_drag_pointer,
};
use rinch_core::Signal;
use rinch_core::element::IntoEventHandler;
use rinch_macros::rsx;
use rinch_platform::{KeyCode, KeyRepeat, Modifiers};

const VIEWPORT: (f32, f32) = (800.0, 600.0);
/// A card and the spacing after it.
const STEP: f32 = 40.0;

type Board = Vec<(String, Vec<String>)>;

#[derive(Clone)]
struct Options {
    preview: SortPreview,
    indicator: SortIndicator,
    source: SortSource,
}

/// The component's own defaults.
impl Default for Options {
    fn default() -> Self {
        let list = SortableList::default();
        Self {
            preview: list.preview,
            indicator: list.indicator,
            source: list.source,
        }
    }
}

thread_local! {
    static OPTIONS: RefCell<Options> = RefCell::new(Options::default());
    static DROPS: RefCell<Vec<SortMove>> = const { RefCell::new(Vec::new()) };
    /// What an app's effect saw of the drag state, in order.
    static SEEN: RefCell<Vec<Option<SortDragState>>> = const { RefCell::new(Vec::new()) };
    static BOARD: Cell<Option<Signal<Board>>> = const { Cell::new(None) };
}

fn judge(m: &SortMove) -> SortVerdict {
    if m.to_list == "c2" && m.keys.iter().any(|k| k == "d") {
        SortVerdict::Refuse("No d in c2".into())
    } else {
        SortVerdict::Accept
    }
}

fn apply(board: &mut Board, m: &SortMove) {
    for (_, cards) in board.iter_mut() {
        cards.retain(|k| !m.keys.contains(k));
    }
    let (_, cards) = board.iter_mut().find(|(id, _)| *id == m.to_list).unwrap();
    let at = m
        .place
        .after
        .as_ref()
        .and_then(|a| cards.iter().position(|k| k == a))
        .map_or(0, |i| i + 1);
    for (i, k) in m.keys.iter().enumerate() {
        cards.insert(at + i, k.clone());
    }
}

struct Fixture {
    app: RinchApp,
    board: Signal<Board>,
}

fn fixture(options: Options) -> Fixture {
    OPTIONS.with(|o| *o.borrow_mut() = options);
    DROPS.with(|d| d.borrow_mut().clear());
    SEEN.with(|d| d.borrow_mut().clear());
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let board = Signal::new(vec![
            (
                "c1".to_string(),
                vec!["a", "b", "c", "d"]
                    .into_iter()
                    .map(String::from)
                    .collect(),
            ),
            ("c2".to_string(), vec!["e".to_string(), "f".to_string()]),
        ]);
        BOARD.with(|b| b.set(Some(board)));
        // The app reading the drag state, as one drawing its own would.
        __scope.create_effect(move || {
            let state = sortable_drag();
            SEEN.with(|s| s.borrow_mut().push(state));
        });
        let o = OPTIONS.with(|o| o.borrow().clone());
        rsx! {
            div { style: "display: flex; gap: 100px; width: 800px; height: 600px; padding: 20px",
                for (id, cards) in board.get() {
                    SortableList {
                        key: id.clone(), group: "cards", id: id.clone(),
                        style: "--rinch-sortable-gap: 10px; width: 150px",
                        judge: Some(Rc::new(judge) as SortJudge),
                        preview: o.preview.clone(), indicator: o.indicator, source: o.source,
                        ondrop: move |m: SortMove| {
                            DROPS.with(|d| d.borrow_mut().push(m.clone()));
                            board.update(|b| apply(b, &m));
                        },
                        for card in cards.clone() {
                            SortableItem {
                                key: card.clone(), value: card.clone(),
                                style: "height: 30px; flex-shrink: 0; font-size: 12px; line-height: 14px",
                                span { class: "card-text", {card.clone()} }
                            }
                        }
                    }
                }
            }
        }
    });
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        // Painted boxes without a running clock: no slide, no tilt.
        d.load_css(
            ":root { --rinch-sortable-reflow-duration: 0ms; \
             --rinch-sortable-ghost-tilt: 0deg; --rinch-sortable-ghost-scale: 1 }",
        );
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let board = BOARD.with(|b| b.get()).unwrap();
    Fixture { app, board }
}

fn send(f: &mut Fixture, event: PlatformEvent) {
    f.app
        .handle_event(event, (VIEWPORT.0 as u32, VIEWPORT.1 as u32), 1.0);
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
}

fn connected(d: &RinchDocument, mut id: u64) -> bool {
    loop {
        if id == d.tree.body_id {
            return true;
        }
        match d.tree.nodes.get(id).and_then(|n| n.parent) {
            Some(p) => id = p,
            None => return false,
        }
    }
}

fn has_class(n: &rinch_dom::Node, class: &str) -> bool {
    n.attributes
        .get("class")
        .is_some_and(|c| c.split_whitespace().any(|one| one == class))
}

fn item(f: &Fixture, key: &str) -> u64 {
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let found: Vec<u64> = d
        .tree
        .nodes
        .iter()
        .filter(|(id, n)| {
            n.attributes.get("data-sortable-key").map(String::as_str) == Some(key)
                && connected(&d, *id)
        })
        .map(|(id, _)| id)
        .collect();
    assert_eq!(found.len(), 1, "one item keyed {key}");
    found[0]
}

fn list(f: &Fixture, id: &str) -> u64 {
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .nodes
        .iter()
        .find(|(nid, n)| {
            n.attributes.contains_key("data-sortable-list")
                && n.attributes.get("data-sortable-group").map(String::as_str) == Some("cards")
                && connected(&d, *nid)
                && n.children.iter().any(|c| {
                    d.tree.nodes[*c]
                        .attributes
                        .get("data-sortable-key")
                        .is_some_and(|k| {
                            (id == "c1" && "abcd".contains(k.as_str()))
                                || (id == "c2" && "ef".contains(k.as_str()))
                        })
                })
        })
        .map(|(nid, _)| nid)
        .unwrap()
}

fn box_of(f: &Fixture, id: u64) -> (f32, f32, f32, f32) {
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    hit_testing::painted_element_box(&d.tree, id)
}

fn point(f: &Fixture, id: u64, fx: f32, fy: f32) -> (f32, f32) {
    let (x, y, w, h) = box_of(f, id);
    (x + w * fx, y + h * fy)
}

fn classes(f: &Fixture, id: u64) -> String {
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.nodes[id]
        .attributes
        .get("class")
        .cloned()
        .unwrap_or_default()
}

fn carrying(f: &Fixture, class: &str) -> Vec<u64> {
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .nodes
        .iter()
        .filter(|(id, n)| has_class(n, class) && connected(&d, *id))
        .map(|(id, _)| id)
        .collect()
}

/// The shift written on `key`'s item, (x, y), `None` when it carries none.
fn shift(f: &Fixture, key: &str) -> Option<(f32, f32)> {
    let id = item(f, key);
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let style = d.tree.nodes[id].attributes.get("style")?.clone();
    let decls = rinch_core::dom::split_declarations(&style);
    let read = |name: &str| {
        decls
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.trim_end_matches("px").parse::<f32>().unwrap())
    };
    Some((
        read("--rinch-sortable-shift-x")?,
        read("--rinch-sortable-shift-y")?,
    ))
}

fn dy(f: &Fixture, key: &str) -> f32 {
    shift(f, key).map_or(0.0, |s| s.1)
}

fn press_at(f: &mut Fixture, (x, y): (f32, f32)) {
    send(
        f,
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
    );
}

fn move_to(f: &mut Fixture, (x, y): (f32, f32)) {
    send(f, PlatformEvent::MouseMove { x, y });
}

fn release_at(f: &mut Fixture, (x, y): (f32, f32)) {
    send(
        f,
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        },
    );
}

fn key(f: &mut Fixture, key: KeyCode, text: Option<&str>) {
    send(
        f,
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Unknown,
        },
    );
}

/// Press `key`'s card `(fx, fy)` of the way into it and pass the threshold.
fn grab(f: &mut Fixture, k: &str, fx: f32, fy: f32) -> (f32, f32) {
    let at = point(f, item(f, k), fx, fy);
    press_at(f, at);
    move_to(f, (at.0 + 8.0, at.1 + 3.0));
    assert!(f.app.active_dnd.is_some(), "the drag went live");
    at
}

fn ghosts(f: &Fixture) -> Vec<u64> {
    carrying(f, "rinch-sortable__ghost")
}

/// The preview's one box.
fn ghost_body(f: &Fixture) -> u64 {
    let b = carrying(f, "rinch-sortable__ghost-body");
    assert_eq!(b.len(), 1, "one preview body");
    b[0]
}

/// A ghost wrapper's (x, y) custom properties.
fn ghost_xy(f: &Fixture) -> (f32, f32) {
    let g = ghosts(f);
    assert_eq!(g.len(), 1, "one ghost");
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let style = d.tree.nodes[g[0]].attributes.get("style").unwrap().clone();
    let decls = rinch_core::dom::split_declarations(&style);
    let read = |name: &str| {
        decls
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.trim_end_matches("px").parse::<f32>().unwrap())
            .unwrap()
    };
    (
        read("--rinch-sortable-ghost-x"),
        read("--rinch-sortable-ghost-y"),
    )
}

/// Nothing of a drag is left on the page.
fn assert_clean(f: &Fixture) {
    for class in [
        "rinch-sortable__ghost",
        "rinch-sortable__item--shifted",
        "rinch-sortable--reflowing",
        "rinch-sortable__item--dragging",
        "rinch-sortable__item--collapsed",
        "rinch-sortable__item--drag-hidden",
        "rinch-sortable__item--lifted",
        "rinch-sortable__item--drop-before",
        "rinch-sortable__item--drop-after",
    ] {
        assert!(carrying(f, class).is_empty(), "{class} left behind");
    }
    for k in ["a", "b", "c", "d", "e", "f"] {
        assert_eq!(shift(f, k), None, "{k}'s shift left behind");
    }
    assert_eq!(sortable_drag(), None);
    assert_eq!(sortable_drag_pointer(), None);
}

fn cards(f: &Fixture, id: &str) -> Vec<String> {
    f.board.get().into_iter().find(|(l, _)| l == id).unwrap().1
}

fn gap_options() -> Options {
    Options {
        indicator: SortIndicator::Gap,
        ..Options::default()
    }
}

// ── Defaults: a browser's native drag, and the caret.

#[test]
fn the_defaults_are_a_browsers_drag_with_a_caret() {
    let mut f = fixture(Options::default());
    let press = grab(&mut f, "a", 0.3, 0.6);
    let (ax, ay, _, _) = box_of(&f, item(&f, "a"));
    let over = point(&f, item(&f, "e"), 0.5, 0.75);
    move_to(&mut f, over);
    // A copy of the item at the grab offset, the runtime's picture hidden.
    let (x, y) = ghost_xy(&f);
    assert!((x - (over.0 - (press.0 - ax))).abs() < 0.01);
    assert!((y - (over.1 - (press.1 - ay))).abs() < 0.01);
    assert!(!events::is_drag_ghost_visible(), "the runtime's is hidden");
    // The source as it was; the place a caret.
    let a = classes(&f, item(&f, "a"));
    assert!(
        !a.contains("--dragging") && !a.contains("--drag-hidden"),
        "{a}"
    );
    assert!(classes(&f, item(&f, "e")).contains("rinch-sortable__item--drop-after"));
    assert!(
        carrying(&f, "rinch-sortable__item--shifted").is_empty(),
        "no gap"
    );
    release_at(&mut f, over);
    assert_eq!(cards(&f, "c2"), ["e", "a", "f"]);
    assert_clean(&f);
}

// ── Preview.

#[test]
fn a_copy_follows_the_pointer_keeping_the_grab_point() {
    let mut f = fixture(Options {
        ..Options::default()
    });
    let (ax, ay, aw, ah) = box_of(&f, item(&f, "b"));
    // Off the middle: 20% across, 30% down.
    let press = grab(&mut f, "b", 0.2, 0.3);
    let (gx, gy) = (press.0 - ax, press.1 - ay);
    assert!((gx - aw * 0.2).abs() < 0.01 && (gy - ah * 0.3).abs() < 0.01);
    for to in [(300.0, 250.0), (420.5, 77.25), (40.0, 500.0)] {
        move_to(&mut f, to);
        let (x, y) = ghost_xy(&f);
        assert!((x - (to.0 - gx)).abs() < 0.01, "x {x} for {to:?}");
        assert!((y - (to.1 - gy)).abs() < 0.01, "y {y} for {to:?}");
        // Painted where the properties say, at the item's size.
        let (px, py, pw, ph) = box_of(&f, ghost_body(&f));
        assert!((px - x).abs() < 0.5 && (py - y).abs() < 0.5, "{px},{py}");
        assert!((pw - aw).abs() < 0.5 && (ph - ah).abs() < 0.5, "{pw}x{ph}");
    }
    let p = sortable_drag_pointer().unwrap();
    assert_eq!((p.x, p.y), (40.0, 500.0));
    assert!((p.grab_x - gx).abs() < 0.01 && (p.grab_y - gy).abs() < 0.01);
    // A copy: its text, none of the item's wiring, and it is never a target.
    let doc = f.app.doc.as_ref().unwrap().clone();
    {
        let d = doc.borrow();
        let g = ghosts(&f)[0];
        assert_eq!(d.tree.nodes[g].parent, Some(d.tree.body_id), "on <body>");
        let mut stack = vec![g];
        let mut text = String::new();
        while let Some(id) = stack.pop() {
            let n = &d.tree.nodes[id];
            if let Some(t) = n.text_content() {
                text.push_str(t);
            }
            for name in n.attributes.keys() {
                assert!(
                    !name.starts_with("data-on")
                        && name != "data-sortable-key"
                        && name != "tabindex"
                        && name != "draggable",
                    "the copy carries {name}"
                );
            }
            stack.extend(n.children.iter().copied());
        }
        assert_eq!(text, "b");
    }
    // Over the ghost the drag still finds c2's place under it.
    let over = point(&f, item(&f, "e"), 0.5, 0.75);
    move_to(&mut f, over);
    assert_eq!(
        sortable_drag().unwrap().target,
        Some(SortTarget {
            list: "c2".into(),
            index: 1
        })
    );
    release_at(&mut f, over);
    assert_eq!(cards(&f, "c2"), ["e", "b", "f"]);
    assert_clean(&f);
}

#[test]
fn the_apps_own_preview_gets_the_key_and_goes_at_the_end() {
    let built: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let seen = built.clone();
    let render: SortPreviewFn = Rc::new(move |scope: &mut RenderScope, key: &str| {
        seen.borrow_mut().push(key.to_string());
        let el = scope.create_element("div");
        el.set_attribute("class", "my-preview");
        el.set_style("width", "50px");
        el.set_style("height", "20px");
        el
    });
    let mut f = fixture(Options {
        preview: SortPreview::Custom(render),
        ..Options::default()
    });
    let press = grab(&mut f, "c", 0.9, 0.1);
    assert_eq!(*built.borrow(), ["c"]);
    let mine = carrying(&f, "my-preview");
    assert_eq!(mine.len(), 1);
    move_to(&mut f, (500.0, 400.0));
    let (cx, cy, cw, ch) = box_of(&f, item(&f, "c"));
    let (x, y) = ghost_xy(&f);
    assert!((x - (500.0 - (press.0 - cx))).abs() < 0.01);
    assert!((y - (400.0 - (press.1 - cy))).abs() < 0.01);
    let (_, _, w, h) = box_of(&f, mine[0]);
    assert_eq!(
        (w, h),
        (50.0, 20.0),
        "its own size, not the item's {cw}x{ch}"
    );
    key(&mut f, KeyCode::Escape, None);
    assert!(carrying(&f, "my-preview").is_empty());
    assert_clean(&f);
}

#[test]
fn none_shows_no_preview_and_dim_fades_the_source() {
    let mut f = fixture(Options {
        preview: SortPreview::None,
        source: SortSource::Dim,
        ..Options::default()
    });
    grab(&mut f, "a", 0.5, 0.5);
    move_to(&mut f, (300.0, 300.0));
    assert!(ghosts(&f).is_empty());
    assert!(!events::is_drag_ghost_visible());
    assert!(classes(&f, item(&f, "a")).contains("rinch-sortable__item--dragging"));
    key(&mut f, KeyCode::Escape, None);
    assert_clean(&f);
}

#[test]
fn snapshot_leaves_the_runtime_ghost_on() {
    let mut f = fixture(Options {
        preview: SortPreview::Snapshot,
        ..Options::default()
    });
    {
        let doc = f.app.doc.as_ref().unwrap();
        let d = doc.borrow();
        assert!(
            !d.tree.nodes[list(&f, "c1")]
                .attributes
                .contains_key("data-drag-ghost")
        );
    }
    grab(&mut f, "a", 0.5, 0.5);
    assert!(events::is_drag_ghost_visible());
    assert!(ghosts(&f).is_empty(), "no DOM preview besides");
    key(&mut f, KeyCode::Escape, None);
    assert_clean(&f);
}

// ── Indicator.

#[test]
fn a_gap_opens_at_the_place_and_closes_when_the_pointer_leaves() {
    let mut f = fixture(gap_options());
    grab(&mut f, "a", 0.5, 0.5);
    // Into c2 after e: f slides down by a card and its spacing.
    let over = point(&f, item(&f, "e"), 0.5, 0.75);
    move_to(&mut f, over);
    assert_eq!(dy(&f, "e"), 0.0);
    assert_eq!(dy(&f, "f"), STEP);
    assert!(classes(&f, list(&f, "c2")).contains("rinch-sortable--reflowing"));
    assert!(
        carrying(&f, "rinch-sortable__item--drop-after").is_empty(),
        "a gap without a caret"
    );
    // Painted there.
    let (_, fy, _, _) = box_of(&f, item(&f, "f"));
    let (_, ey, _, eh) = box_of(&f, item(&f, "e"));
    assert!((fy - (ey + eh + 10.0 + STEP)).abs() < 0.5, "f at {fy}");
    // Before e: both slide.
    let over = point(&f, item(&f, "e"), 0.5, 0.2);
    move_to(&mut f, over);
    assert_eq!((dy(&f, "e"), dy(&f, "f")), (STEP, STEP));
    // Off every list: the gap closes.
    move_to(&mut f, (790.0, 590.0));
    assert_eq!((dy(&f, "e"), dy(&f, "f")), (0.0, 0.0));
    assert_eq!(sortable_drag().unwrap().target, None);
    // Within c1, after c: b and c make room, a goes to the place, d stays.
    let over = point(&f, item(&f, "c"), 0.5, 0.75);
    move_to(&mut f, over);
    assert_eq!(
        (dy(&f, "a"), dy(&f, "b"), dy(&f, "c"), dy(&f, "d")),
        (2.0 * STEP, -STEP, -STEP, 0.0)
    );
    release_at(&mut f, over);
    assert_eq!(cards(&f, "c1"), ["b", "c", "a", "d"]);
    assert_clean(&f);
}

#[test]
fn both_draws_the_caret_with_the_gap_and_none_draws_neither() {
    let mut f = fixture(Options {
        indicator: SortIndicator::Both,
        ..Options::default()
    });
    grab(&mut f, "a", 0.5, 0.5);
    let over = point(&f, item(&f, "e"), 0.5, 0.75);
    move_to(&mut f, over);
    assert_eq!(dy(&f, "f"), STEP);
    assert!(classes(&f, item(&f, "e")).contains("rinch-sortable__item--drop-after"));
    key(&mut f, KeyCode::Escape, None);
    assert_clean(&f);

    let mut f = fixture(Options {
        indicator: SortIndicator::None,
        ..Options::default()
    });
    grab(&mut f, "a", 0.5, 0.5);
    let over = point(&f, item(&f, "e"), 0.5, 0.75);
    move_to(&mut f, over);
    assert_eq!(dy(&f, "f"), 0.0);
    assert!(carrying(&f, "rinch-sortable__item--drop-after").is_empty());
    // The app still knows where it would land.
    let state = sortable_drag().unwrap();
    assert_eq!(
        state.target,
        Some(SortTarget {
            list: "c2".into(),
            index: 1
        })
    );
    assert_eq!(state.verdict, Some(SortVerdict::Accept));
    release_at(&mut f, over);
    assert_eq!(cards(&f, "c2"), ["e", "a", "f"]);
}

#[test]
fn a_refused_place_opens_no_gap_and_still_says_why() {
    let mut f = fixture(gap_options());
    grab(&mut f, "d", 0.5, 0.5);
    let over = point(&f, item(&f, "e"), 0.5, 0.75);
    move_to(&mut f, over);
    assert_eq!((dy(&f, "e"), dy(&f, "f")), (0.0, 0.0), "no gap");
    let e = classes(&f, item(&f, "e"));
    assert!(
        e.contains("rinch-sortable__item--drop-after") && e.contains("--refused"),
        "{e}"
    );
    let shown = carrying(&f, "rinch-sortable__reason--shown");
    assert_eq!(shown.len(), 1);
    let state = sortable_drag().unwrap();
    assert_eq!(
        state.verdict,
        Some(SortVerdict::Refuse("No d in c2".into()))
    );
    // Back to an accepted place: the gap opens there.
    let over = point(&f, item(&f, "b"), 0.5, 0.25);
    move_to(&mut f, over);
    assert_eq!((dy(&f, "b"), dy(&f, "c")), (STEP, STEP));
    assert!(carrying(&f, "rinch-sortable__reason--shown").is_empty());
    key(&mut f, KeyCode::Escape, None);
    assert_clean(&f);
    assert_eq!(cards(&f, "c1"), ["a", "b", "c", "d"]);
}

/// The place is found from the boxes as painted, gap included: sweeping the
/// pointer down over c2 and back moves the place once each way, never back
/// and forth (a shift always moves the next threshold away from the pointer).
#[test]
fn a_sweep_over_a_shifting_list_changes_the_place_once_each_way() {
    let mut f = fixture(gap_options());
    grab(&mut f, "a", 0.5, 0.5);
    let (ex, ey, ew, _) = box_of(&f, item(&f, "e"));
    let x = ex + ew / 2.0;
    let index = || sortable_drag().unwrap().over.map(|t| t.index);
    let mut down = Vec::new();
    let mut y = ey + 2.0;
    while y < ey + 110.0 {
        move_to(&mut f, (x, y));
        down.push(index());
        y += 1.5;
    }
    let mut up = Vec::new();
    while y > ey + 2.0 {
        y -= 1.5;
        move_to(&mut f, (x, y));
        up.push(index());
    }
    let changes = |v: &[Option<usize>]| v.windows(2).filter(|w| w[0] != w[1]).count();
    assert!(down.windows(2).all(|w| w[0] <= w[1]), "down: {down:?}");
    assert!(up.windows(2).all(|w| w[0] >= w[1]), "up: {up:?}");
    assert_eq!(down.first(), Some(&Some(0)));
    assert_eq!(down.last(), Some(&Some(2)));
    assert_eq!(changes(&down), 2, "{down:?}");
    assert_eq!(changes(&up), 2, "{up:?}");
    key(&mut f, KeyCode::Escape, None);
}

/// In its own list a visible source (dimmed or unchanged) is never painted
/// over: dragging d up over c1 and back, the items between the place and d
/// make room and d sits at the place, every step; no two of c1's cards
/// overlap, and the place still changes once each way.
#[test]
fn a_sweep_in_the_sources_own_list_never_paints_one_card_over_another() {
    for source in [SortSource::Dim, SortSource::Unchanged] {
        let mut f = fixture(Options {
            indicator: SortIndicator::Gap,
            source,
            preview: SortPreview::None,
        });
        grab(&mut f, "d", 0.5, 0.5);
        let (ax, ay, aw, _) = box_of(&f, item(&f, "a"));
        let (_, dy0, _, dh) = box_of(&f, item(&f, "d"));
        let x = ax + aw / 2.0;
        let index = || sortable_drag().unwrap().over.map(|t| t.index);
        let no_overlap = |f: &Fixture, at: f32| {
            let mut boxes: Vec<(String, f32, f32)> = ["a", "b", "c", "d"]
                .iter()
                .map(|k| {
                    let (_, y, _, h) = box_of(f, item(f, k));
                    (k.to_string(), y, y + h)
                })
                .collect();
            boxes.sort_by(|p, q| p.1.total_cmp(&q.1));
            for w in boxes.windows(2) {
                assert!(
                    w[0].2 <= w[1].1 + 0.5,
                    "{source:?} at y {at}: {} ({}..{}) over {} ({}..{})",
                    w[0].0,
                    w[0].1,
                    w[0].2,
                    w[1].0,
                    w[1].1,
                    w[1].2
                );
            }
        };
        let mut up = Vec::new();
        let mut y = dy0 + dh / 2.0;
        while y > ay + 2.0 {
            y -= 1.5;
            move_to(&mut f, (x, y));
            no_overlap(&f, y);
            up.push(index());
        }
        // At the top: d painted first, where a was.
        let (_, d_now, _, _) = box_of(&f, item(&f, "d"));
        assert!(
            (d_now - ay).abs() < 0.5,
            "{source:?}: d at {d_now}, a was at {ay}"
        );
        let mut down = Vec::new();
        while y < dy0 + dh / 2.0 {
            y += 1.5;
            move_to(&mut f, (x, y));
            no_overlap(&f, y);
            down.push(index());
        }
        let changes = |v: &[Option<usize>]| v.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(up.windows(2).all(|w| w[0] >= w[1]), "{source:?} up: {up:?}");
        assert!(
            down.windows(2).all(|w| w[0] <= w[1]),
            "{source:?} down: {down:?}"
        );
        assert_eq!(changes(&up), 3, "{source:?} {up:?}");
        assert_eq!(changes(&down), 3, "{source:?} {down:?}");
        release_at(&mut f, (x, ay + 2.0));
        assert_clean(&f);
    }
}

// ── Source.

#[test]
fn a_collapsed_source_closes_up_and_reopens_over_its_own_place() {
    let mut f = fixture(Options {
        source: SortSource::Collapse,
        indicator: SortIndicator::Gap,
        ..Options::default()
    });
    grab(&mut f, "b", 0.5, 0.5);
    let b = classes(&f, item(&f, "b"));
    assert!(
        b.contains("rinch-sortable__item--collapsed") && !b.contains("--dragging"),
        "{b}"
    );
    // Over its own place: the space is where it was.
    assert_eq!((dy(&f, "a"), dy(&f, "c"), dy(&f, "d")), (0.0, 0.0, 0.0));
    // Off every list: c and d close up over b's space.
    move_to(&mut f, (790.0, 590.0));
    assert_eq!((dy(&f, "a"), dy(&f, "c"), dy(&f, "d")), (0.0, -STEP, -STEP));
    // Into c2 before f: c1 stays closed, c2 opens.
    let over = point(&f, item(&f, "f"), 0.5, 0.2);
    move_to(&mut f, over);
    assert_eq!(
        (dy(&f, "c"), dy(&f, "d"), dy(&f, "e"), dy(&f, "f")),
        (-STEP, -STEP, 0.0, STEP)
    );
    release_at(&mut f, over);
    assert_eq!(cards(&f, "c2"), ["e", "b", "f"]);
    assert_eq!(cards(&f, "c1"), ["a", "c", "d"]);
    assert_clean(&f);
}

#[test]
fn hide_and_unchanged_style_the_source_as_named() {
    let mut f = fixture(Options {
        source: SortSource::Hide,
        ..Options::default()
    });
    grab(&mut f, "b", 0.5, 0.5);
    move_to(&mut f, (790.0, 590.0));
    let b = classes(&f, item(&f, "b"));
    assert!(
        b.contains("rinch-sortable__item--drag-hidden") && !b.contains("--dragging"),
        "{b}"
    );
    assert_eq!(dy(&f, "c"), 0.0, "its space is kept");
    key(&mut f, KeyCode::Escape, None);
    assert_clean(&f);

    let mut f = fixture(Options {
        source: SortSource::Unchanged,
        ..Options::default()
    });
    grab(&mut f, "b", 0.5, 0.5);
    let b = classes(&f, item(&f, "b"));
    assert!(
        !b.contains("--dragging") && !b.contains("--drag-hidden") && !b.contains("--collapsed"),
        "{b}"
    );
    key(&mut f, KeyCode::Escape, None);
    assert_clean(&f);
}

// ── Keyboard, cancel, and the app's view.

#[test]
fn a_keyboard_lift_moves_the_gap_and_escape_puts_everything_back() {
    let _guard = crate::app::timer_backend_lock();
    rinch_core::set_timer_backend(|_, _| {});
    let mut f = fixture(Options {
        preview: SortPreview::Clone,
        indicator: SortIndicator::Gap,
        source: SortSource::Collapse,
    });
    let at = point(&f, item(&f, "a"), 0.5, 0.5);
    press_at(&mut f, at);
    release_at(&mut f, at);
    key(&mut f, KeyCode::Space, Some(" "));
    assert!(classes(&f, item(&f, "a")).contains("rinch-sortable__item--lifted"));
    assert!(ghosts(&f).is_empty(), "no pointer, no preview");
    assert!(sortable_drag().unwrap().keyboard);
    assert_eq!(sortable_drag_pointer(), None);
    key(&mut f, KeyCode::ArrowDown, None);
    // After b: b makes room and the lifted a (which does not collapse) moves
    // to the place.
    assert_eq!((dy(&f, "a"), dy(&f, "b"), dy(&f, "c")), (STEP, -STEP, 0.0));
    key(&mut f, KeyCode::ArrowDown, None);
    assert_eq!(
        (dy(&f, "a"), dy(&f, "c"), dy(&f, "d")),
        (2.0 * STEP, -STEP, 0.0)
    );
    key(&mut f, KeyCode::ArrowRight, None);
    // Across into c2 at the same index, its end: c1 is as it was, and the
    // gap is after f (nothing to move).
    assert_eq!((dy(&f, "a"), dy(&f, "c"), dy(&f, "e")), (0.0, 0.0, 0.0));
    assert_eq!(
        sortable_drag().unwrap().target.map(|t| t.list),
        Some("c2".into())
    );
    key(&mut f, KeyCode::Escape, None);
    assert_clean(&f);
    assert_eq!(cards(&f, "c1"), ["a", "b", "c", "d"]);
    // Lift again and drop: the same order a caret drop gives.
    key(&mut f, KeyCode::Space, Some(" "));
    key(&mut f, KeyCode::ArrowDown, None);
    key(&mut f, KeyCode::Space, Some(" "));
    assert_eq!(cards(&f, "c1"), ["b", "a", "c", "d"]);
    assert_clean(&f);
}

#[test]
fn escape_mid_pointer_drag_restores_every_option() {
    let mut f = fixture(Options {
        preview: SortPreview::Clone,
        indicator: SortIndicator::Both,
        source: SortSource::Collapse,
    });
    grab(&mut f, "a", 0.5, 0.5);
    let over = point(&f, item(&f, "e"), 0.5, 0.25);
    move_to(&mut f, over);
    assert_eq!(ghosts(&f).len(), 1);
    assert_eq!(dy(&f, "e"), STEP);
    key(&mut f, KeyCode::Escape, None);
    assert_clean(&f);
    release_at(&mut f, over);
    assert!(DROPS.with(|d| d.borrow().is_empty()));
}

#[test]
fn the_drag_state_signal_tells_an_app_every_step() {
    let mut f = fixture(Options {
        indicator: SortIndicator::None,
        source: SortSource::Unchanged,
        ..Options::default()
    });
    grab(&mut f, "d", 0.5, 0.5);
    let over = point(&f, item(&f, "e"), 0.5, 0.75);
    move_to(&mut f, over);
    let over = point(&f, item(&f, "a"), 0.5, 0.25);
    move_to(&mut f, over);
    release_at(&mut f, over);
    let seen = SEEN.with(|s| s.borrow().clone());
    let target = |l: &str, i| {
        Some(SortTarget {
            list: l.into(),
            index: i,
        })
    };
    assert_eq!(seen.first(), Some(&None), "the effect ran before any drag");
    assert!(
        seen.iter().flatten().any(|s| s.target == target("c2", 1)
            && s.verdict == Some(SortVerdict::Refuse("No d in c2".into()))
            && s.source_key == "d"
            && s.from_list == "c1"
            && !s.keyboard),
        "{seen:?}"
    );
    assert!(
        seen.iter()
            .flatten()
            .any(|s| s.target == target("c1", 0) && s.verdict == Some(SortVerdict::Accept)),
        "{seen:?}"
    );
    assert_eq!(seen.last(), Some(&None), "and the end");
    assert_eq!(cards(&f, "c1"), ["d", "a", "b", "c"]);
}

// ── Each list's preview takes its own styling (#2154 review).

mod preview_cascade {
    //! c1 [a b] sets its ghost look inline on the list; c2 [e f] sits in a
    //! wrapper whose *class* sets it from a stylesheet, and f sets its own
    //! inline. Each drag's preview, on `<body>`, shows the values its item
    //! inherits — as if it were still inside its list.
    use super::*;

    fn styled() -> Fixture {
        DROPS.with(|d| d.borrow_mut().clear());
        let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
            let board: Signal<Board> = Signal::new(vec![
                ("c1".into(), vec!["a".into(), "b".into()]),
                ("c2".into(), vec!["e".into(), "f".into()]),
            ]);
            BOARD.with(|b| b.set(Some(board)));
            rsx! {
                div { style: "display: flex; gap: 100px; width: 800px; height: 600px; padding: 20px",
                    SortableList {
                        group: "cards", id: "c1",
                        style: "width: 150px; --rinch-sortable-ghost-opacity: 0.5; --rinch-sortable-ghost-shadow: 0 0 0 3px red",
                        SortableItem { value: "a", style: "height: 30px", "a" }
                        SortableItem { value: "b", style: "height: 30px", "b" }
                    }
                    div { class: "lane-b",
                        SortableList {
                            group: "cards", id: "c2", style: "width: 150px",
                            SortableItem { value: "e", style: "height: 30px", "e" }
                            SortableItem {
                                value: "f",
                                style: "height: 30px; --rinch-sortable-ghost-opacity: 0.9",
                                "f"
                            }
                        }
                    }
                }
            }
        });
        app.mount_component(VIEWPORT.0, VIEWPORT.1);
        {
            let doc = app.doc.as_ref().unwrap();
            let mut d = doc.borrow_mut();
            d.load_css(&rinch_theme::generate_theme_css(
                &rinch_theme::Theme::default(),
            ));
            d.load_css(&rinch_components::generate_component_css());
            d.load_css(
                ".lane-b { --rinch-sortable-ghost-opacity: 0.3; \
                 --rinch-sortable-ghost-shadow: 0 0 0 5px blue }",
            );
            d.recompute_all_styles_full();
        }
        app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
        let board = BOARD.with(|b| b.get()).unwrap();
        Fixture { app, board }
    }

    /// The preview's opacity and its copy's shadow spread, as painted.
    fn look(f: &Fixture) -> (f32, Option<f32>) {
        let g = ghosts(f);
        assert_eq!(g.len(), 1, "one preview");
        let doc = f.app.doc.as_ref().unwrap();
        let d = doc.borrow();
        let _ = g;
        let body = &d.tree.nodes[ghost_body(f)];
        (
            body.computed_style.opacity,
            body.computed_style
                .box_shadow
                .first()
                .map(|s| s.spread_radius),
        )
    }

    fn drag_look(k: &str) -> (f32, Option<f32>) {
        let mut f = styled();
        grab(&mut f, k, 0.5, 0.5);
        move_to(&mut f, (500.0, 400.0));
        let seen = look(&f);
        key(&mut f, KeyCode::Escape, None);
        assert!(ghosts(&f).is_empty());
        seen
    }

    #[test]
    fn set_inline_on_the_list() {
        assert_eq!(drag_look("a"), (0.5, Some(3.0)));
    }

    #[test]
    fn set_by_a_class_on_an_ancestor() {
        assert_eq!(drag_look("e"), (0.3, Some(5.0)));
    }

    #[test]
    fn set_on_the_item_itself() {
        assert_eq!(drag_look("f"), (0.9, Some(5.0)));
    }
}

/// What an app's preview function makes (a signal, an effect) belongs to the
/// preview and goes when its drag ends: none of it outlives its drag.
#[test]
fn what_a_custom_preview_makes_goes_with_its_drag() {
    thread_local! {
        static MADE: RefCell<Vec<Signal<u32>>> = const { RefCell::new(Vec::new()) };
        static RUNS: Cell<u32> = const { Cell::new(0) };
    }
    MADE.with(|m| m.borrow_mut().clear());
    let render: SortPreviewFn = Rc::new(|scope: &mut RenderScope, key: &str| {
        let count = Signal::new(0u32);
        MADE.with(|m| m.borrow_mut().push(count));
        scope.create_effect(move || {
            let _ = count.get();
            RUNS.with(|r| r.set(r.get() + 1));
        });
        let el = scope.create_element("div");
        el.set_text(key);
        el
    });
    let mut f = fixture(Options {
        preview: SortPreview::Custom(render),
        ..Options::default()
    });
    for k in ["a", "b", "c"] {
        grab(&mut f, k, 0.5, 0.5);
        key(&mut f, KeyCode::Escape, None);
    }
    let made = MADE.with(|m| m.borrow().clone());
    assert_eq!(made.len(), 3);
    assert!(
        made.iter().all(|s| !s.is_alive()),
        "alive after their drags: {}",
        made.iter().filter(|s| s.is_alive()).count()
    );
    assert_eq!(
        RUNS.with(Cell::get),
        3,
        "each effect ran once, at its build"
    );
}
