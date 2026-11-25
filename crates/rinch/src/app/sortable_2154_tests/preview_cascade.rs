//! Each list's preview takes its own styling (#2154 review).
//!
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
                    keyboard: true,
                    group: "cards", id: "c1",
                    style: "width: 150px; --rinch-sortable-ghost-opacity: 0.5; --rinch-sortable-ghost-shadow: 0 0 0 3px red",
                    SortableItem { value: "a", style: "height: 30px", "a" }
                    SortableItem { value: "b", style: "height: 30px", "b" }
                }
                div { class: "lane-b",
                    SortableList {
                        keyboard: true,
                        group: "cards", id: "c2", style: "width: 150px",
                        SortableItem { value: "e", style: "height: 30px", "e" }
                        SortableItem {
                            value: "f",
                            style: "height: 30px; --rinch-sortable-ghost-opacity: 0.9",
                            "f"
                        }
                    }
                }
                div { style: "opacity: 0.5",
                    SortableList {
                        keyboard: true,
                        group: "cards", id: "c4",
                        style: "width: 150px; --rinch-sortable-ghost-opacity: 1",
                        SortableItem { value: "k", style: "height: 30px; background: rgb(0, 0, 0)" }
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

/// An ancestor's `opacity` is carried onto its stand-in, which draws no
/// box: it is not applied to the preview a second time (nor at all), so
/// an opaque black card is painted black.
#[cfg(software_shell)]
#[test]
fn an_ancestors_opacity_is_not_applied_to_the_preview() {
    let mut f = styled();
    grab(&mut f, "k", 0.5, 0.5);
    move_to(&mut f, (300.0, 400.0));
    let (x, y, w, h) = box_of(&f, ghost_body(&f));
    f.app.has_previous_frame = false;
    f.app.scene_dirty = true;
    let (px, width, _) = f
        .app
        .build_pixels(1.0, (VIEWPORT.0 as u32, VIEWPORT.1 as u32), false);
    let (cx, cy) = ((x + w / 2.0) as u32, (y + h / 2.0) as u32);
    let i = ((cy * width + cx) * 4) as usize;
    assert_eq!(&px[i..i + 3], &[0, 0, 0], "painted at {x},{y} {w}x{h}");
    key(&mut f, KeyCode::Escape, None);
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
