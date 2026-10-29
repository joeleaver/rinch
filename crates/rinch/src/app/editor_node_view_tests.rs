//! Node views on the desktop (PR B), driven through the real `PlatformEvent`
//! path: a press on an atom's frame node-selects it (any selectable leaf atom,
//! by the schema — review of #1565, F1); a press inside a view's component is the
//! component's (an `<input>` in it takes the keyboard and the editor's caret
//! goes); keys there reach the view first and an unclaimed Escape gives the
//! keyboard back with the caret after the node; typing elsewhere keeps the
//! mount; a view that grows moves the caret below it; Down onto the node
//! offers it the caret.

use super::hit_testing::painted_element_box;
use super::*;
use rinch_core::reactive::Signal;
use rinch_editor_core::{NodeSelection, Pos, Selection};
use std::cell::Cell;

const VP: (u32, u32) = (800, 600);

/// `ab` 0..4, the embed 4..5, `cd` 5..9.
const EMBED: usize = 4;

struct Page {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
}

#[derive(Default)]
struct View {
    renders: Cell<usize>,
    input: RefCell<Option<NodeHandle>>,
    grow: RefCell<Option<Signal<u32>>>,
    keys: RefCell<Vec<String>>,
    entered: RefCell<Vec<crate::editor::CaretFrom>>,
}

fn page(with_view: Option<Rc<View>>) -> Page {
    let slot: Rc<RefCell<Option<crate::editor::EditorHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "padding: 30px 0 0 40px");
        let (container, handle) = crate::editor::mount_editor(scope);
        handle.load_html(
            r#"<p>ab</p><div data-pm-type="embed" data-sheet="s1"></div><p>cd</p><hr><p>ef</p>"#,
        );
        container.set_attribute(
            "style",
            "width: 400px; font-size: 16px; line-height: 24px; font-family: sans-serif",
        );
        root.append_child(&container);
        if let Some(view) = &with_view {
            let v = view.clone();
            let k = view.clone();
            let e = view.clone();
            handle
                .register_node_view(
                    crate::editor::NodeViewSpec::new("embed", move |s: &mut RenderScope, _ctx| {
                        v.renders.set(v.renders.get() + 1);
                        let grow = Signal::new(20u32);
                        *v.grow.borrow_mut() = Some(grow);
                        let root = s.create_element("div");
                        root.set_attribute("style", "padding: 4px");
                        let input = s.create_element("input");
                        input.set_attribute("style", "width: 200px; height: 30px; font-size: 16px");
                        // A text field takes the keyboard on a press (as `TextInput` does).
                        let typed = s.register_input_handler(|_| {});
                        input.set_attribute("data-oninput", &typed.0.to_string());
                        root.append_child(&input);
                        let filler = s.create_element("div");
                        s.create_effect({
                            let filler = filler.clone();
                            move || filler.set_style("height", &format!("{}px", grow.get()))
                        });
                        root.append_child(&filler);
                        *v.input.borrow_mut() = Some(input);
                        root
                    })
                    .on_key(move |key| {
                        k.keys.borrow_mut().push(key.key.to_string());
                        false
                    })
                    .on_caret_enter(move |from| {
                        e.entered.borrow_mut().push(from);
                        false
                    }),
                )
                .keep();
        }
        *slot_in.borrow_mut() = Some(handle);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let handle = slot.borrow_mut().take().expect("captured at mount");
    Page { app, handle }
}

fn ev(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
    app.handle_event(event, VP, 1.0)
}

fn press(app: &mut RinchApp, (x, y): (f32, f32)) {
    let button = MouseButton::Left;
    ev(app, PlatformEvent::MouseDown { x, y, button });
    ev(app, PlatformEvent::MouseUp { x, y, button });
    app.resolve_and_repaint(800.0, 600.0);
}

fn key(app: &mut RinchApp, key: KeyCode, text: Option<&str>) {
    let modifiers = Modifiers::default();
    ev(
        app,
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers,
            repeat: KeyRepeat::Fresh,
        },
    );
    ev(
        app,
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers,
        },
    );
    app.resolve_and_repaint(800.0, 600.0);
}

/// The first element whose `data-pm-type` is `ty`.
fn host(page: &Page, ty: &str) -> u64 {
    let doc = page.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .nodes
        .iter()
        .map(|(id, _)| id)
        .find(|&id| {
            d.tree
                .nodes
                .get(id)
                .is_some_and(|n| n.attributes.get("data-pm-type").map(String::as_str) == Some(ty))
        })
        .expect("a host")
}

fn box_of(page: &Page, id: u64) -> (f32, f32, f32, f32) {
    let doc = page.app.doc.as_ref().unwrap();
    painted_element_box(&doc.borrow().tree, id)
}

/// A point inside `id`'s box, `inset` px from its top-left corner.
fn point_in(page: &Page, id: u64, inset: f32) -> (f32, f32) {
    let (x, y, _, _) = box_of(page, id);
    (x + inset, y + inset)
}

fn caret_point(page: &Page, pos: usize) -> (f32, f32) {
    let (x, y, h) = page
        .app
        .editor_caret_point(&page.handle, Pos(pos))
        .expect("a caret");
    (x + 1.0, y + h / 2.0)
}

fn block_text(page: &Page, i: usize) -> String {
    let doc = page.handle.doc();
    let block = doc.child(i);
    (0..block.child_count())
        .filter_map(|j| block.child(j).text().map(str::to_string))
        .collect()
}

fn node_sel(anchor: usize) -> Selection {
    Selection::Node(NodeSelection {
        anchor: Pos(anchor),
        head: Pos(anchor + 1),
    })
}

/// The caret overlay's painted top, while it is shown.
fn caret_top(page: &Page) -> Option<f32> {
    let doc = page.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let id = d.tree.nodes.iter().map(|(id, _)| id).find(|&id| {
        d.tree
            .nodes
            .get(id)
            .is_some_and(|n| n.attributes.contains_key("data-pm-caret"))
    })?;
    let visible = d.tree.nodes.get(id)?.computed_style.visibility
        == rinch_dom::computed_style::VisibilityValue::Visible;
    visible.then(|| painted_element_box(&d.tree, id).1)
}

/// Review of #1565, F1: a press on an `embed` with no view node-selects it, as a
/// press on a horizontal rule does — by the schema's leaf/selectable flags, not a
/// list of type names. It used to leave a text caret between the blocks.
#[test]
fn a_press_on_an_embed_node_selects_it() {
    let mut p = page(None);
    let embed = host(&p, "embed");
    let at = point_in(&p, embed, 3.0);
    press(&mut p.app, at);
    assert_eq!(
        p.handle.selection(),
        node_sel(EMBED),
        "the embed is selected"
    );
    // Control: the rule still is, and a paragraph still takes a caret.
    let hr = host(&p, "horizontal_rule");
    let (x, y, w, h) = box_of(&p, hr);
    press(&mut p.app, (x + w / 2.0, y + h / 2.0));
    assert_eq!(p.handle.selection(), node_sel(9));
    let at = caret_point(&p, 7);
    press(&mut p.app, at);
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(7)));
}

#[test]
fn a_press_on_the_views_frame_node_selects_it() {
    let view = Rc::new(View::default());
    let mut p = page(Some(view.clone()));
    assert_eq!(view.renders.get(), 1);
    let embed = host(&p, "embed");
    // The frame: the host's own padding around the component (the default
    // stylesheet gives a mounted view's host one, so it can be pressed).
    let (x, y, _, _) = box_of(&p, embed);
    press(&mut p.app, (x + 2.0, y + 2.0));
    assert_eq!(p.handle.selection(), node_sel(EMBED));
    // Control: a press on the component itself (beside its input) is the
    // component's and selects nothing.
    let at = caret_point(&p, 2);
    press(&mut p.app, at);
    let root = view.input.borrow().clone().unwrap().node_id().0;
    let (ix, iy, iw, _) = box_of(&p, root);
    press(&mut p.app, (ix + iw + 20.0, iy + 5.0));
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(2)));
}

#[test]
fn an_input_in_a_view_takes_the_keyboard_and_escape_gives_it_back() {
    let view = Rc::new(View::default());
    let mut p = page(Some(view.clone()));
    let at = caret_point(&p, 2);
    press(&mut p.app, at);
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(2)));
    assert!(caret_top(&p).is_some(), "the editor's caret is shown");

    let input = view.input.borrow().clone().unwrap().node_id().0;
    let at = point_in(&p, input, 5.0);
    press(&mut p.app, at);
    assert_eq!(
        p.app.focus_target,
        FocusTarget::Input(input),
        "the input has the keyboard"
    );
    assert_eq!(
        p.handle.selection(),
        Selection::cursor(Pos(2)),
        "the press moved no editor caret"
    );
    assert!(caret_top(&p).is_none(), "the editor's caret is hidden");

    // Typing goes to the input; the view hears every key first.
    key(&mut p.app, KeyCode::KeyX, Some("x"));
    assert_eq!(block_text(&p, 0), "ab", "not the editor's");
    assert_eq!(*view.keys.borrow(), ["x"]);

    key(&mut p.app, KeyCode::Escape, None);
    assert_eq!(*view.keys.borrow(), ["x", "Escape"]);
    assert!(
        matches!(p.app.focus_target, FocusTarget::Editor(_)),
        "{:?}",
        p.app.focus_target
    );
    assert_eq!(
        p.handle.selection(),
        Selection::cursor(Pos(EMBED + 2)),
        "the caret is right after the node"
    );
    // And the editor takes the next key.
    key(&mut p.app, KeyCode::KeyZ, Some("z"));
    assert_eq!(block_text(&p, 2), "zcd");
    assert_eq!(view.renders.get(), 1, "typing elsewhere kept the mount");
}

#[test]
fn a_view_that_grows_moves_the_caret_below_it() {
    let view = Rc::new(View::default());
    let mut p = page(Some(view.clone()));
    let at = caret_point(&p, 7);
    press(&mut p.app, at);
    let before = caret_top(&p).expect("a caret");
    view.grow.borrow().unwrap().set(120);
    p.app.resolve_and_repaint(800.0, 600.0);
    let after = caret_top(&p).expect("a caret");
    assert!(
        (after - before - 100.0).abs() < 0.5,
        "the caret moved down by the growth: {before} -> {after}"
    );
    assert_eq!(view.renders.get(), 1);
}

#[test]
fn down_onto_a_view_node_selects_it_and_offers_the_caret() {
    let view = Rc::new(View::default());
    let mut p = page(Some(view.clone()));
    let at = caret_point(&p, 2);
    press(&mut p.app, at);
    key(&mut p.app, KeyCode::ArrowDown, None);
    assert_eq!(p.handle.selection(), node_sel(EMBED));
    assert_eq!(*view.entered.borrow(), [crate::editor::CaretFrom::Above]);
    // Control: Down from "cd" still steps over the rule.
    let at = caret_point(&p, 7);
    press(&mut p.app, at);
    key(&mut p.app, KeyCode::ArrowDown, None);
    let head = p.handle.selection().head().0;
    assert!(
        (11..=13).contains(&head),
        "into \"ef\" past the rule: {head}"
    );
}

/// Review of #1575: events inside a view never reach text editing.
mod review_1575 {
    use super::*;

    fn key_mod(app: &mut RinchApp, key: KeyCode, text: Option<&str>, modifiers: Modifiers) {
        ev(
            app,
            PlatformEvent::KeyDown {
                key,
                logical_key: None,
                text: text.map(str::to_string),
                modifiers,
                repeat: KeyRepeat::Fresh,
            },
        );
        ev(
            app,
            PlatformEvent::KeyUp {
                key,
                logical_key: None,
                modifiers,
            },
        );
        app.resolve_and_repaint(800.0, 600.0);
    }

    fn input_value(p: &Page, id: u64) -> String {
        let doc = p.app.doc.as_ref().unwrap();
        let d = doc.borrow();
        d.tree
            .get(id)
            .and_then(|n| n.attributes.get("value").cloned())
            .unwrap_or_default()
    }

    fn focus_input(p: &mut Page, view: &Rc<View>) -> u64 {
        let input = view.input.borrow().clone().unwrap().node_id().0;
        let at = point_in(p, input, 5.0);
        press(&mut p.app, at);
        assert_eq!(p.app.focus_target, FocusTarget::Input(input));
        input
    }

    #[test]
    fn typing_ctrl_z_backspace_and_tab_in_the_views_input_never_edit_the_document() {
        let view = Rc::new(View::default());
        let mut p = page(Some(view.clone()));
        // Give the editor an undoable edit first.
        let at = caret_point(&p, 2);
        press(&mut p.app, at);
        key(&mut p.app, KeyCode::KeyQ, Some("q"));
        assert_eq!(block_text(&p, 0), "aqb");
        let doc_before = p.handle.doc();
        let input = focus_input(&mut p, &view);
        key(&mut p.app, KeyCode::KeyX, Some("x"));
        assert_eq!(
            input_value(&p, input),
            "x",
            "positive control: the input took it"
        );
        let ctrl = Modifiers {
            ctrl: true,
            ..Modifiers::default()
        };
        key_mod(&mut p.app, KeyCode::KeyZ, None, ctrl);
        key(&mut p.app, KeyCode::Backspace, None);
        key(&mut p.app, KeyCode::Backspace, None);
        key(&mut p.app, KeyCode::Delete, None);
        key(&mut p.app, KeyCode::Tab, None);
        assert!(
            p.handle.doc().same_ref(&doc_before) || p.handle.doc() == doc_before,
            "the editor's document is untouched: {:?}",
            p.handle.doc()
        );
        assert_eq!(block_text(&p, 0), "aqb", "no editor undo");
        assert_eq!(view.renders.get(), 1);
    }

    #[test]
    fn a_drag_that_starts_inside_the_view_selects_no_editor_text() {
        let view = Rc::new(View::default());
        let mut p = page(Some(view.clone()));
        let at = caret_point(&p, 2);
        press(&mut p.app, at);
        let input = view.input.borrow().clone().unwrap().node_id().0;
        let (ix, iy, iw, _) = box_of(&p, input);
        let start = (ix + iw + 20.0, iy + 5.0);
        let end = caret_point(&p, 7);
        let button = MouseButton::Left;
        ev(
            &mut p.app,
            PlatformEvent::MouseDown {
                x: start.0,
                y: start.1,
                button,
            },
        );
        for i in 1..=5 {
            let t = i as f32 / 5.0;
            ev(
                &mut p.app,
                PlatformEvent::MouseMove {
                    x: start.0 + (end.0 - start.0) * t,
                    y: start.1 + (end.1 - start.1) * t,
                },
            );
            p.app.resolve_and_repaint(800.0, 600.0);
        }
        ev(
            &mut p.app,
            PlatformEvent::MouseUp {
                x: end.0,
                y: end.1,
                button,
            },
        );
        p.app.resolve_and_repaint(800.0, 600.0);
        assert_eq!(p.handle.selection(), Selection::cursor(Pos(2)));
    }

    /// The overlay half of "a view that grows moves what is below it": a range
    /// highlight in the paragraph below follows the growth.
    #[test]
    fn a_view_that_grows_moves_the_selection_highlight_below_it() {
        let view = Rc::new(View::default());
        let mut p = page(Some(view.clone()));
        let at = caret_point(&p, 6);
        press(&mut p.app, at);
        p.handle.set_selection(Selection::text(Pos(6), Pos(8)));
        p.app.resolve_and_repaint(800.0, 600.0);
        let top = |p: &Page| {
            let doc = p.app.doc.as_ref().unwrap();
            let d = doc.borrow();
            d.tree
                .nodes
                .iter()
                .map(|(id, _)| id)
                .find(|&id| {
                    d.tree.nodes.get(id).is_some_and(|n| {
                        n.attributes.get("data-pm-selection").map(String::as_str) == Some("true")
                    })
                })
                .map(|id| painted_element_box(&d.tree, id).1)
        };
        let before = top(&p).expect("a highlight");
        view.grow.borrow().unwrap().set(120);
        p.app.resolve_and_repaint(800.0, 600.0);
        let after = top(&p).expect("a highlight");
        assert!(
            (after - before - 100.0).abs() < 0.5,
            "the highlight moved down by the growth: {before} -> {after}"
        );
    }
}
