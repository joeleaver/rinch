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
    /// Whether the app applies a drop to its board. Off, it records the drop
    /// and draws nothing new, so a keyboard drop's focus request stays parked.
    apply: bool,
}

/// The component's own defaults.
impl Default for Options {
    fn default() -> Self {
        let list = SortableList::default();
        Self {
            preview: list.preview,
            indicator: list.indicator,
            source: list.source,
            apply: true,
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
        let apply_drops = o.apply;
        rsx! {
            div { style: "display: flex; gap: 100px; width: 800px; height: 600px; padding: 20px",
                for (id, cards) in board.get() {
                    SortableList {
                        keyboard: true,
                        key: id.clone(), group: "cards", id: id.clone(),
                        style: "--rinch-sortable-gap: 10px; width: 150px",
                        judge: Some(Rc::new(judge) as SortJudge),
                        preview: o.preview.clone(), indicator: o.indicator, source: o.source,
                        ondrop: move |m: SortMove| {
                            DROPS.with(|d| d.borrow_mut().push(m.clone()));
                            if apply_drops {
                                board.update(|b| apply(b, &m));
                            }
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
                            (id == "c1" && "abcdz".contains(k.as_str()))
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

mod indicator;
mod keyboard_and_cancel;
mod preview;
mod preview_cascade;
mod review_round_2;
mod source;
