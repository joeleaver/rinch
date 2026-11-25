//! The default preview, the gap, a refused place, a collapsed source and
//! a keyboard lift's gap.

use super::*;

#[wasm_bindgen_test]
fn the_default_preview_is_a_translucent_copy_at_the_grab_offset() {
    let board = mount(Options::default());
    let b = item("b");
    let r = rect(&b);
    let press_at = point(&b, 0.2, 0.3);
    let (gx, gy) = (press_at.0 as f64 - r.x(), press_at.1 as f64 - r.y());
    press(press_at);
    move_to((press_at.0 + 8, press_at.1 + 2));
    assert!(sortable_drag().is_some(), "the drag went live");
    // A real event reached rinch: the source is unchanged, a ghost is up.
    assert!(!b.class_name().contains("--dragging"), "{}", b.class_name());
    for to in [(300, 250), (420, 77), (40, 300)] {
        move_to(to);
        let gr = rect(&ghost_body());
        assert!(
            (gr.x() - (to.0 as f64 - gx)).abs() < 1.0 && (gr.y() - (to.1 as f64 - gy)).abs() < 1.0,
            "ghost at {},{} for {to:?}",
            gr.x(),
            gr.y()
        );
        assert!((gr.width() - r.width()).abs() < 1.0 && (gr.height() - r.height()).abs() < 1.0);
    }
    let g = ghost().unwrap();
    // On the document's body: rinch-web's is the mount's root, outside
    // every list.
    assert!(g.closest("[data-sortable-list]").unwrap().is_none());
    assert_eq!(g.text_content().as_deref(), Some("b"));
    assert!(
        g.query_selector("[data-sortable-key], [data-onclick], [tabindex]")
            .unwrap()
            .is_none()
    );
    let style = web_sys::window()
        .unwrap()
        .get_computed_style(&ghost_body())
        .unwrap()
        .unwrap();
    assert_eq!(style.get_property_value("opacity").unwrap(), "0.75");
    assert_eq!(style.get_property_value("pointer-events").unwrap(), "none");
    // The ghost is never what is under the pointer.
    let over = point(&item("e"), 0.5, 0.75);
    move_to(over);
    assert_eq!(
        sortable_drag().unwrap().target,
        Some(SortTarget {
            list: "c2".into(),
            index: 1
        })
    );
    release(over);
    assert_eq!(cards(board, "c2"), ["e", "b", "f"]);
    assert_clean();
}

#[wasm_bindgen_test]
fn a_gap_opens_at_the_place_closes_on_leave_and_not_at_a_refused_place() {
    let board = mount(Options {
        indicator: SortIndicator::Gap,
        ..Options::default()
    });
    let a = point(&item("a"), 0.5, 0.5);
    press(a);
    move_to((a.0 + 8, a.1 + 2));
    move_to(point(&item("e"), 0.5, 0.75));
    assert_eq!((slid("e"), slid("f")), (0.0, STEP));
    assert_eq!(count("rinch-sortable__item--drop-after"), 0, "no caret");
    move_to((590, 590));
    assert_eq!((slid("e"), slid("f")), (0.0, 0.0));
    let over = point(&item("c"), 0.5, 0.75);
    move_to(over);
    // In its own list: b and c make room, a goes to the place, d stays.
    assert_eq!(
        (slid("a"), slid("b"), slid("c"), slid("d")),
        (2.0 * STEP, -STEP, -STEP, 0.0)
    );
    release(over);
    assert_eq!(cards(board, "c1"), ["b", "c", "a", "d"]);
    assert_clean();

    // d into c2 is refused: no gap, the refused caret and the reason.
    let d = point(&item("d"), 0.5, 0.5);
    press(d);
    move_to((d.0 + 8, d.1 + 2));
    move_to(point(&item("e"), 0.5, 0.75));
    assert_eq!((slid("e"), slid("f")), (0.0, 0.0));
    assert!(item("e").class_name().contains("--refused"));
    let reason = document()
        .query_selector(".rinch-sortable__reason--shown")
        .unwrap()
        .expect("the reason");
    assert_eq!(reason.text_content().as_deref(), Some("No d in c2"));
    // Released off every list: nothing moves.
    move_to((590, 590));
    release((590, 590));
    assert_eq!(cards(board, "c2"), ["e", "f"]);
    assert_clean();
}

#[wasm_bindgen_test]
fn a_collapsed_source_closes_up_off_the_lists() {
    let board = mount(Options {
        source: SortSource::Collapse,
        indicator: SortIndicator::Gap,
        ..Options::default()
    });
    let b = point(&item("b"), 0.5, 0.5);
    press(b);
    move_to((b.0 + 8, b.1 + 2));
    assert!(
        item("b")
            .class_name()
            .contains("rinch-sortable__item--collapsed")
    );
    move_to((590, 590));
    assert_eq!((slid("a"), slid("c"), slid("d")), (0.0, -STEP, -STEP));
    let over = point(&item("f"), 0.5, 0.2);
    move_to(over);
    assert_eq!((slid("c"), slid("e"), slid("f")), (-STEP, 0.0, STEP));
    release(over);
    assert_eq!(cards(board, "c2"), ["e", "b", "f"]);
    assert_clean();
}

#[wasm_bindgen_test]
fn a_keyboard_lift_moves_the_gap() {
    let board = mount(Options {
        indicator: SortIndicator::Gap,
        ..Options::default()
    });
    item("a").focus().unwrap();
    assert!(key(" "));
    assert!(ghost().is_none(), "no pointer, no preview");
    assert!(sortable_drag().unwrap().keyboard);
    assert!(key("ArrowDown"));
    assert_eq!((slid("a"), slid("b"), slid("c")), (STEP, -STEP, 0.0));
    assert!(key("ArrowDown"));
    assert_eq!((slid("a"), slid("c"), slid("d")), (2.0 * STEP, -STEP, 0.0));
    assert!(key("Escape"));
    assert_clean();
    item("a").focus().unwrap();
    assert!(key(" "));
    assert!(key("ArrowDown"));
    assert!(key(" "));
    assert_eq!(cards(board, "c1"), ["b", "a", "c", "d"]);
    assert_clean();
    let seen = SEEN.with(|s| s.borrow().clone());
    assert!(
        seen.iter().flatten().any(|s| s.keyboard
            && s.target
                == Some(SortTarget {
                    list: "c1".into(),
                    index: 1
                })),
        "{seen:?}"
    );
}
