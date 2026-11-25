//! The defaults (a browser's native drag and the caret) and `preview`.

use super::*;

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
