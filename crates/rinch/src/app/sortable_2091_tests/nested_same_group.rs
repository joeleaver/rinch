//! An outline: a list of group "n" whose items hold lists of group "n".
//! A drag never lands in a list inside an item it carries.
use super::*;

thread_local! {
    static NDROPS: RefCell<Vec<SortMove>> = const { RefCell::new(Vec::new()) };
    static NOFFERS: RefCell<Vec<Offer>> = const { RefCell::new(Vec::new()) };
}

fn outline() -> RinchApp {
    NDROPS.with(|d| d.borrow_mut().clear());
    NOFFERS.with(|d| d.borrow_mut().clear());
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        rsx! {
            div { style: "width: 400px",
                SortableList {
                    keyboard: true,
                    group: "n", id: "root", style: "--rinch-sortable-gap: 0px",
                    ondrop: move |m: SortMove| NDROPS.with(|d| d.borrow_mut().push(m)),
                    ondropchange: move |o: Offer| NOFFERS.with(|v| v.borrow_mut().push(o)),
                    SortableItem { value: "x", style: "min-height: 30px",
                        div { style: "height: 30px" }
                        SortableList {
                            keyboard: true,
                            group: "n", id: "x-children",
                            style: "--rinch-sortable-gap: 0px; margin-left: 250px; width: 120px",
                            ondrop: move |m: SortMove| NDROPS.with(|d| d.borrow_mut().push(m)),
                            SortableItem { value: "x1", style: "height: 30px" }
                        }
                    }
                    SortableItem { value: "y", style: "height: 30px" }
                }
            }
        }
    });
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    app
}

fn node(app: &RinchApp, key: &str) -> u64 {
    let d = app.doc.as_ref().unwrap().borrow();
    item_in(&d, key)
}

fn mid(app: &RinchApp, id: u64, fy: f32) -> (f32, f32) {
    let d = app.doc.as_ref().unwrap().borrow();
    let (x, y, w, h) = hit_testing::painted_element_box(&d.tree, id);
    (x + w / 2.0, y + h * fy)
}

fn go(app: &mut RinchApp, e: PlatformEvent) {
    app.handle_event(e, (VIEWPORT.0 as u32, VIEWPORT.1 as u32), 1.0);
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
}

#[test]
fn a_drag_over_a_list_inside_the_carried_item_offers_nothing_there() {
    let mut app = outline();
    let (x, y) = mid(&app, node(&app, "x"), 0.1);
    go(
        &mut app,
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
    );
    go(
        &mut app,
        PlatformEvent::MouseMove {
            x: x + 8.0,
            y: y + 1.0,
        },
    );
    let (x1x, x1y) = mid(&app, node(&app, "x1"), 0.75);
    go(&mut app, PlatformEvent::MouseMove { x: x1x, y: x1y });
    let offered = NOFFERS.with(|o| o.borrow().last().cloned().flatten());
    assert!(
        offered
            .as_ref()
            .is_none_or(|(m, _)| m.to_list != "x-children"),
        "never into x's own children: {offered:?}"
    );
    go(
        &mut app,
        PlatformEvent::MouseUp {
            x: x1x,
            y: x1y,
            button: MouseButton::Left,
        },
    );
    assert!(
        NDROPS.with(|d| d.borrow().iter().all(|m| m.to_list != "x-children")),
        "{:?}",
        NDROPS.with(|d| d.borrow().clone())
    );
}

#[test]
fn a_keyboard_move_skips_a_list_inside_the_carried_item() {
    let mut app = outline();
    let (x, y) = mid(&app, node(&app, "x"), 0.1);
    go(
        &mut app,
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
    );
    go(
        &mut app,
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        },
    );
    for (k, t) in [
        (KeyCode::Space, Some(" ")),
        (KeyCode::ArrowRight, None),
        (KeyCode::ArrowDown, None),
    ] {
        go(
            &mut app,
            PlatformEvent::KeyDown {
                key: k,
                logical_key: None,
                text: t.map(str::to_string),
                modifiers: Modifiers::default(),
                repeat: KeyRepeat::Unknown,
            },
        );
        let offered = NOFFERS.with(|o| o.borrow().last().cloned().flatten());
        assert!(
            offered
                .as_ref()
                .is_none_or(|(m, _)| m.to_list != "x-children"),
            "{offered:?}"
        );
    }
}
