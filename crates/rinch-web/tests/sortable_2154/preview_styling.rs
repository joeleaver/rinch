//! Each list's preview takes its own styling (#2154 review).

use super::*;

// ── Each list's preview takes its own styling (#2154 review): c1 sets it
// inline on the list, c2 inherits it from a wrapper's class in a stylesheet,
// and f sets its own.

fn mount_styled() {
    PREVIOUS.with(|p| {
        if let Some(h) = p.borrow_mut().take() {
            h.unmount();
        }
    });
    while let Ok(Some(el)) = document().query_selector(&format!("[{HOST_ATTR}]")) {
        el.remove();
    }
    if let Some(root) = document().document_element() {
        pointer(&root, "pointerup", (0, 0));
    }
    if document()
        .get_element_by_id("sortable-2154-sheet")
        .is_none()
    {
        let sheet = document().create_element("style").unwrap();
        sheet.set_id("sortable-2154-sheet");
        sheet.set_text_content(Some(
            ".lane-b { --rinch-sortable-ghost-opacity: 0.3; \
             --rinch-sortable-ghost-shadow: rgb(0, 0, 255) 0px 0px 0px 5px; \
             color: rgb(0, 128, 0); } \
             .lane-b .rinch-sortable__item { font-size: 17px; }",
        ));
        document().head().unwrap().append_child(&sheet).unwrap();
    }
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST_ATTR, "true").unwrap();
    host.set_attribute(
        "style",
        "position: fixed; left: 0px; top: 0px; width: 600px",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let handle = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |__scope: &mut RenderScope| {
            rsx! {
                div { style: "display: flex; gap: 100px; align-items: flex-start; padding: 20px",
                    SortableList {
                        keyboard: true,
                        group: "cards", id: "c1",
                        style: "width: 150px; --rinch-sortable-ghost-opacity: 0.5; --rinch-sortable-ghost-shadow: rgb(255, 0, 0) 0px 0px 0px 3px",
                        SortableItem { value: "a", style: "height: 30px", "a" }
                    }
                    div { style: "transform: scale(0.5); transform-origin: 0 0",
                        SortableList {
                            keyboard: true,
                            group: "cards", id: "c3", style: "width: 150px",
                            SortableItem { value: "g", style: "height: 30px", "g" }
                        }
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
                }
            }
        },
    );
    PREVIOUS.with(|p| *p.borrow_mut() = Some(handle));
}

/// Drag `k` and read its preview's opacity and its copy's box-shadow.
fn drag_look(k: &str) -> (String, String) {
    mount_styled();
    let p = point(&item(k), 0.5, 0.5);
    press(p);
    move_to((p.0 + 8, p.1 + 2));
    move_to((500, 400));
    let body = ghost_body();
    let window = web_sys::window().unwrap();
    let opacity = window
        .get_computed_style(&body)
        .unwrap()
        .unwrap()
        .get_property_value("opacity")
        .unwrap();
    let shadow = window
        .get_computed_style(&body)
        .unwrap()
        .unwrap()
        .get_property_value("box-shadow")
        .unwrap();
    move_to((590, 590));
    release((590, 590));
    assert!(ghost().is_none());
    (opacity, shadow)
}

/// Inherited values and descendant selectors from the item's ancestors reach
/// the preview, and inside a scaled ancestor it is the size the item is
/// painted at, kept at the grab point.
#[wasm_bindgen_test]
fn a_preview_inherits_from_its_ancestors_and_keeps_their_scale() {
    mount_styled();
    let e = item("e");
    let p = point(&e, 0.5, 0.5);
    press(p);
    move_to((p.0 + 8, p.1 + 2));
    move_to((400, 300));
    let style = web_sys::window()
        .unwrap()
        .get_computed_style(&ghost_body())
        .unwrap()
        .unwrap();
    assert_eq!(style.get_property_value("color").unwrap(), "rgb(0, 128, 0)");
    assert_eq!(style.get_property_value("font-size").unwrap(), "17px");
    move_to((590, 590));
    release((590, 590));

    mount_styled();
    let g = item("g");
    let r = rect(&g);
    assert!(
        (r.width() - 75.0).abs() < 0.5,
        "painted at half: {}",
        r.width()
    );
    let press_at = point(&g, 0.25, 0.5);
    let (gx, gy) = (press_at.0 as f64 - r.x(), press_at.1 as f64 - r.y());
    press(press_at);
    move_to((press_at.0 + 8, press_at.1 + 2));
    move_to((400, 300));
    let b = rect(&ghost_body());
    assert!(
        (b.width() - r.width()).abs() < 0.5 && (b.height() - r.height()).abs() < 0.5,
        "{}x{} for {}x{}",
        b.width(),
        b.height(),
        r.width(),
        r.height()
    );
    assert!(
        (b.x() - (400.0 - gx)).abs() < 1.0 && (b.y() - (300.0 - gy)).abs() < 1.0,
        "at {},{}",
        b.x(),
        b.y()
    );
    move_to((590, 590));
    release((590, 590));
}

#[wasm_bindgen_test]
fn each_preview_takes_its_own_lists_and_items_styling() {
    assert_eq!(
        drag_look("a"),
        (
            "0.5".to_string(),
            "rgb(255, 0, 0) 0px 0px 0px 3px".to_string()
        )
    );
    assert_eq!(
        drag_look("e"),
        (
            "0.3".to_string(),
            "rgb(0, 0, 255) 0px 0px 0px 5px".to_string()
        )
    );
    assert_eq!(
        drag_look("f"),
        (
            "0.9".to_string(),
            "rgb(0, 0, 255) 0px 0px 0px 5px".to_string()
        )
    );
}
