//! A keyboard lift, cancelling a drag, and the drag-state signal an app draws from.

use super::*;

// ── Keyboard, cancel, and the app's view.

#[test]
fn a_keyboard_lift_moves_the_gap_and_escape_puts_everything_back() {
    let _guard = crate::app::timer_backend_lock();
    rinch_core::set_timer_backend(|_, _| {});
    let mut f = fixture(Options {
        preview: SortPreview::Clone,
        indicator: SortIndicator::Gap,
        source: SortSource::Collapse,
        apply: true,
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
        apply: true,
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
