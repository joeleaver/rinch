//! What a copy keeps and leaves (#2154 review), and the second review round:
//! what a cancelled drag's copies leave registered.

use super::*;

// ── What a copy keeps and leaves (#2154 review).

#[wasm_bindgen_test]
fn a_copy_keeps_svg_tag_case_and_leaves_a_radio_group_alone() {
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
                SortableList {
                    keyboard: true,
                    group: "cards", id: "c1", style: "width: 150px",
                    SortableItem { value: "a", style: "height: 40px",
                        input { r#type: "radio", name: "pick-2154", checked: true, class: "pick" }
                        svg { width: "20", height: "20",
                            defs { clipPath { id: "clip-2154", rect { width: "10", height: "10" } } }
                            rect { width: "20", height: "20", clip_path: "url(#clip-2154)" }
                        }
                    }
                }
            }
        },
    );
    PREVIOUS.with(|p| *p.borrow_mut() = Some(handle));
    let original: web_sys::HtmlInputElement = document()
        .query_selector(&format!("[{HOST_ATTR}] .pick"))
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    assert!(original.checked());
    let p = point(&item("a"), 0.5, 0.5);
    press(p);
    move_to((p.0 + 8, p.1 + 2));
    move_to((400, 300));
    let g = ghost().expect("a preview");
    // The copy's radio is in no group, so the original keeps its check.
    let copy: web_sys::HtmlInputElement = g
        .query_selector(".pick")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    assert!(!copy.has_attribute("name") && !copy.has_attribute("checked"));
    assert!(original.checked(), "the original radio is still checked");
    // SVG's camel-case element survives the copy (an HTML-cased `clippath`
    // is an unknown element that clips nothing).
    let clip = g
        .query_selector("defs > *")
        .unwrap()
        .expect("the clip path");
    assert_eq!(clip.local_name(), "clipPath");
    assert_eq!(
        clip.namespace_uri().as_deref(),
        Some("http://www.w3.org/2000/svg")
    );
    move_to((590, 590));
    release((590, 590));
    assert!(ghost().is_none());
}

// ── Second review round.

/// A preview's nodes leave the page's registry when its drag ends.
#[wasm_bindgen_test]
fn cancelled_drags_with_a_copy_leave_no_nodes_registered() {
    let _board = mount(Options::default());
    let drag = |k: &str| {
        let p = point(&item(k), 0.5, 0.5);
        press(p);
        move_to((p.0 + 8, p.1 + 2));
        move_to((500, 400));
        move_to((590, 590));
        release((590, 590));
    };
    drag("a");
    let base = rinch_web::web_document::__node_registry_len();
    for k in ["a", "b", "c", "d", "e", "f", "a", "b"] {
        drag(k);
    }
    assert_eq!(rinch_web::web_document::__node_registry_len(), base);
}

/// The copies carry no attribute the runtime acts on.
#[wasm_bindgen_test]
fn the_copies_carry_no_runtime_attributes() {
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
                div { "data-trap-focus": "", "data-render-surface": "1", "data-viewport": "1",
                    "data-state": "open",
                    SortableList {
                        keyboard: true,
                        group: "cards", id: "c1", style: "width: 150px",
                        SortableItem { value: "a", style: "height: 40px",
                            div { "contenteditable": "true", "data-pm-editor": "", "data-scroll-lock-exempt": "", style: "height: 15px", "a" }
                        }
                    }
                }
            }
        },
    );
    PREVIOUS.with(|p| *p.borrow_mut() = Some(handle));
    // Pressed below the editable part.
    let p = point(&item("a"), 0.5, 0.85);
    press(p);
    move_to((p.0 + 8, p.1 + 2));
    move_to((400, 300));
    let g = ghost().expect("a preview");
    for attr in [
        "data-trap-focus",
        "data-render-surface",
        "data-viewport",
        "contenteditable",
        "data-pm-editor",
        "data-scroll-lock-exempt",
    ] {
        assert!(
            g.query_selector(&format!("[{attr}]")).unwrap().is_none(),
            "a copy carries {attr}"
        );
    }
    // A style hook stays.
    assert!(g.query_selector("[data-state=open]").unwrap().is_some());
    move_to((590, 590));
    release((590, 590));
}
