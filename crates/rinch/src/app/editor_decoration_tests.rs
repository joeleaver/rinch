//! Fixtures from the review of #836 (desktop): the claimed right-press caret, and an
//! inline decoration against the real rinch-dom host.

use super::*;
use rinch_editor_core::decoration::{Decoration, DecorationSet};
use rinch_editor_core::{Attrs, EditorState, Plugin, PluginKey, Pos, Selection};

const VP: (u32, u32) = (800, 600);

fn ev(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
    app.handle_event(event, VP, 1.0)
}

fn press(app: &mut RinchApp, x: f32, y: f32, button: MouseButton) {
    ev(app, PlatformEvent::MouseDown { x, y, button });
    ev(app, PlatformEvent::MouseUp { x, y, button });
}

struct Spell {
    ranges: RefCell<Vec<(usize, usize)>>,
}
impl Plugin for Spell {
    fn key(&self) -> PluginKey {
        PluginKey("review836.spell")
    }
    fn decorations(&self, _state: &EditorState) -> DecorationSet {
        DecorationSet::new(
            self.ranges
                .borrow()
                .iter()
                .map(|&(a, b)| {
                    Decoration::inline(Pos(a), Pos(b), Attrs::new().with("class", "pm-spell-error"))
                })
                .collect(),
        )
    }
}

struct Page {
    app: RinchApp,
    container: usize,
    handle: crate::editor::EditorHandle,
    seen: Rc<RefCell<Vec<Selection>>>,
}

/// An editor inside a `div` carrying a live `data-oncontextmenu` (when `claim`),
/// with an optional spell plugin.
fn page(html: &'static str, claim: bool, style: &'static str, spell: Option<Rc<Spell>>) -> Page {
    let slot: Rc<RefCell<Option<(usize, crate::editor::EditorHandle)>>> =
        Rc::new(RefCell::new(None));
    let seen: Rc<RefCell<Vec<Selection>>> = Rc::new(RefCell::new(Vec::new()));
    let slot_in = slot.clone();
    let seen_in = seen.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let (container, handle) = crate::editor::mount_editor(scope);
        if let Some(sp) = &spell {
            assert!(handle.add_plugin(sp.clone()));
        }
        handle.load_html(html);
        container.set_attribute("style", style);
        if claim {
            let h2 = handle.clone();
            let seen2 = seen_in.clone();
            let rid = rinch_core::register_handler(Rc::new(move || {
                seen2.borrow_mut().push(h2.selection());
            }));
            root.set_attribute("data-oncontextmenu", &rid.0.to_string());
        }
        root.append_child(&container);
        *slot_in.borrow_mut() = Some((container.node_id().0, handle));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (container, handle) = slot.borrow_mut().take().unwrap();
    Page {
        app,
        container,
        handle,
        seen,
    }
}

const STYLE: &str = "width: 400px; height: 200px; font-size: 16px; line-height: 24px; \
                     font-family: sans-serif";

fn point_at(p: &Page, pos: usize) -> (f32, f32) {
    let (x, y, h) = p.app.editor_caret_point(&p.handle, Pos(pos)).unwrap();
    (x + 1.0, y + h / 2.0)
}

#[test]
fn a_claimed_right_press_places_the_caret_before_the_handler_runs() {
    let mut p = page("<p>hello world</p>", true, STYLE, None);
    p.handle.set_selection(Selection::cursor(Pos(2)));
    let (x, y) = point_at(&p, 9);
    press(&mut p.app, x, y, MouseButton::Right);
    assert!(
        !p.app.is_text_context_menu_open(),
        "the app claimed the press"
    );
    let seen = p.seen.borrow().clone();
    assert_eq!(seen.len(), 1, "the handler ran once");
    assert_eq!(
        seen[0],
        Selection::cursor(Pos(9)),
        "handler saw the pressed caret"
    );
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(9)));
}

#[test]
fn a_claimed_right_press_inside_the_selection_keeps_it() {
    let mut p = page("<p>hello world</p>", true, STYLE, None);
    let sel = Selection::text(Pos(3), Pos(10));
    p.handle.set_selection(sel.clone());
    let (x, y) = point_at(&p, 6);
    press(&mut p.app, x, y, MouseButton::Right);
    assert_eq!(p.seen.borrow().clone(), vec![sel.clone()]);
    assert_eq!(p.handle.selection(), sel);
}

#[test]
fn a_decorated_run_keeps_every_caret_point_and_hit() {
    let plain = page("<p>hello world again</p>", false, STYLE, None);
    let spell = Rc::new(Spell {
        ranges: RefCell::new(vec![(7, 12)]),
    });
    let mut deco = page("<p>hello world again</p>", false, STYLE, Some(spell));
    {
        let d = deco.app.doc.as_ref().unwrap().borrow();
        let n = d
            .tree
            .nodes
            .iter()
            .filter(|(_, n)| n.attributes.contains_key("data-pm-deco"))
            .count();
        assert_eq!(n, 1, "positive control: one decoration segment is mounted");
    }
    for pos in 1..=18 {
        let a = plain.app.editor_caret_point(&plain.handle, Pos(pos));
        let b = deco.app.editor_caret_point(&deco.handle, Pos(pos));
        assert_eq!(a, b, "caret point for {pos}");
    }
    for pos in [2usize, 7, 9, 12, 15] {
        let (x, y) = point_at(&deco, pos);
        press(&mut deco.app, x, y, MouseButton::Left);
        assert_eq!(
            deco.handle.selection(),
            Selection::cursor(Pos(pos)),
            "hit at {pos}"
        );
    }
    let _ = deco.container;
}

#[cfg(software_shell)]
fn frame(app: &mut RinchApp) -> Vec<u8> {
    app.scene_dirty = true;
    app.has_previous_frame = false;
    app.build_pixels(1.0, VP, false).0.to_vec()
}

#[cfg(software_shell)]
fn diff_box(a: &[u8], b: &[u8]) -> Option<(u32, u32, u32, u32)> {
    let (w, h) = VP;
    let mut bb: Option<(u32, u32, u32, u32)> = None;
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            if (0..4).any(|k| (a[i + k] as i32 - b[i + k] as i32).abs() > 8) {
                bb = Some(match bb {
                    None => (x, y, x, y),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                });
            }
        }
    }
    bb
}

/// The squiggle paints under the decorated word and nowhere else.
#[cfg(software_shell)]
#[test]
fn the_squiggle_is_under_the_word_only() {
    let mut plain = page("<p>hello world again</p>", false, STYLE, None);
    let spell = Rc::new(Spell {
        ranges: RefCell::new(vec![(7, 12)]),
    });
    let mut deco = page("<p>hello world again</p>", false, STYLE, Some(spell));
    let a = frame(&mut plain.app);
    let b = frame(&mut deco.app);
    let (x0, y0, x1, y1) = diff_box(&a, &b).expect("the squiggle painted something");
    let (wx0, wy, _) = plain.app.editor_caret_point(&plain.handle, Pos(7)).unwrap();
    let (wx1, _, wh) = plain
        .app
        .editor_caret_point(&plain.handle, Pos(12))
        .unwrap();
    eprintln!("diff {x0},{y0}-{x1},{y1}; word x {wx0}..{wx1} y {wy}+{wh}");
    assert!(x0 as f32 >= wx0 - 2.0 && x1 as f32 <= wx1 + 2.0, "x extent");
    assert!(y0 as f32 >= wy + wh / 2.0, "below the text middle");
}

/// Dropping the decoration and repainting incrementally must leave nothing
/// behind: compare an incremental frame with a full one. `line-height` is tight
/// so the wave sits at (or past) the bottom of the paragraph's box.
#[cfg(software_shell)]
#[test]
fn removing_a_squiggle_leaves_no_streak_on_an_incremental_frame() {
    for style in [
        STYLE,
        "width: 400px; height: 200px; font-size: 16px; line-height: 16px; \
         font-family: sans-serif",
        "width: 400px; height: 200px; font-size: 16px; line-height: 12px; \
         font-family: sans-serif",
    ] {
        let spell = Rc::new(Spell {
            ranges: RefCell::new(vec![(7, 12)]),
        });
        let mut p = page(
            "<p>hello world again</p><p>next line here</p>",
            false,
            style,
            Some(spell.clone()),
        );
        let full_before = frame(&mut p.app);
        // Drop the decoration; any transaction makes the view re-sync.
        spell.ranges.borrow_mut().clear();
        p.handle.set_selection(Selection::cursor(Pos(3)));
        p.app.resolve_and_repaint(800.0, 600.0);
        p.app.scene_dirty = true;
        let incremental = p.app.build_pixels(1.0, VP, false).0.to_vec();
        let full = frame(&mut p.app);
        assert!(
            diff_box(&full_before, &full).is_some(),
            "positive control: dropping the decoration changed the frame"
        );
        let d = diff_box(&incremental, &full);
        eprintln!("style `{style}`: incremental vs full diff = {d:?}");
        assert_eq!(
            d, None,
            "stale squiggle pixels left by the partial repaint ({style})"
        );
    }
}

/// Decorates "world" (7..12) unless the caret is inside it — a spellchecker's
/// shape, which discards the pressed segment when the caret moves in.
struct Shy;
impl Plugin for Shy {
    fn key(&self) -> PluginKey {
        PluginKey("review836.shy")
    }
    fn decorations(&self, state: &EditorState) -> DecorationSet {
        if (7..=12).contains(&state.selection.head().0) {
            return DecorationSet::new(vec![]);
        }
        DecorationSet::new(vec![Decoration::inline(
            Pos(7),
            Pos(12),
            Attrs::new().with("class", "pm-spell-error"),
        )])
    }
}

fn shy_page(claim: bool) -> Page {
    let slot: Rc<RefCell<Option<(usize, crate::editor::EditorHandle)>>> =
        Rc::new(RefCell::new(None));
    let seen: Rc<RefCell<Vec<Selection>>> = Rc::new(RefCell::new(Vec::new()));
    let (slot_in, seen_in) = (slot.clone(), seen.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let (container, handle) = crate::editor::mount_editor(scope);
        assert!(handle.add_plugin(Rc::new(Shy)));
        handle.load_html("<p>hello world again</p>");
        container.set_attribute("style", STYLE);
        if claim {
            let (h2, s2) = (handle.clone(), seen_in.clone());
            let rid = rinch_core::register_handler(Rc::new(move || {
                s2.borrow_mut().push(h2.selection());
            }));
            root.set_attribute("data-oncontextmenu", &rid.0.to_string());
        }
        root.append_child(&container);
        *slot_in.borrow_mut() = Some((container.node_id().0, handle));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (container, handle) = slot.borrow_mut().take().unwrap();
    Page {
        app,
        container,
        handle,
        seen,
    }
}

#[test]
fn a_right_press_that_withdraws_the_squiggle_still_works() {
    for claim in [true, false] {
        let mut p = shy_page(claim);
        p.handle.set_selection(Selection::cursor(Pos(2)));
        p.app.resolve_and_repaint(800.0, 600.0);
        let segs = |p: &Page| {
            let d = p.app.doc.as_ref().unwrap().borrow();
            d.tree
                .nodes
                .iter()
                .filter(|(_, n)| n.attributes.contains_key("data-pm-deco"))
                .filter(|(_, n)| {
                    let mut cur = n.parent;
                    while let Some(id) = cur {
                        if id == p.container {
                            return true;
                        }
                        cur = d.tree.get(id).and_then(|m| m.parent);
                    }
                    false
                })
                .count()
        };
        assert_eq!(segs(&p), 1, "positive control: squiggle mounted");
        let (x, y) = point_at(&p, 9);
        press(&mut p.app, x, y, MouseButton::Right);
        assert_eq!(
            p.handle.selection(),
            Selection::cursor(Pos(9)),
            "claim={claim}"
        );
        assert_eq!(segs(&p), 0, "the squiggle withdrew (claim={claim})");
        // The editor holds the keyboard either way — the web's mousedown
        // focuses the capture textarea before an app claim is consulted.
        assert_eq!(
            p.app.focus_target,
            FocusTarget::Editor(p.container),
            "claim={claim}"
        );
        if claim {
            assert_eq!(p.seen.borrow().clone(), vec![Selection::cursor(Pos(9))]);
            assert!(!p.app.is_text_context_menu_open());
        } else {
            assert!(
                p.app.is_text_context_menu_open(),
                "the built-in menu opened"
            );
        }
    }
}
