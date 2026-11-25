//! `indicator`: the caret, the gap, both or neither.

use super::*;

// ── Indicator.

#[test]
fn a_gap_opens_at_the_place_and_closes_when_the_pointer_leaves() {
    let mut f = fixture(gap_options());
    grab(&mut f, "a", 0.5, 0.5);
    // Into c2 after e: f slides down by a card and its spacing.
    let over = point(&f, item(&f, "e"), 0.5, 0.75);
    move_to(&mut f, over);
    assert_eq!(dy(&f, "e"), 0.0);
    assert_eq!(dy(&f, "f"), STEP);
    assert!(classes(&f, list(&f, "c2")).contains("rinch-sortable--reflowing"));
    assert!(
        carrying(&f, "rinch-sortable__item--drop-after").is_empty(),
        "a gap without a caret"
    );
    // Painted there.
    let (_, fy, _, _) = box_of(&f, item(&f, "f"));
    let (_, ey, _, eh) = box_of(&f, item(&f, "e"));
    assert!((fy - (ey + eh + 10.0 + STEP)).abs() < 0.5, "f at {fy}");
    // Before e: both slide.
    let over = point(&f, item(&f, "e"), 0.5, 0.2);
    move_to(&mut f, over);
    assert_eq!((dy(&f, "e"), dy(&f, "f")), (STEP, STEP));
    // Off every list: the gap closes.
    move_to(&mut f, (790.0, 590.0));
    assert_eq!((dy(&f, "e"), dy(&f, "f")), (0.0, 0.0));
    assert_eq!(sortable_drag().unwrap().target, None);
    // Within c1, after c: b and c make room, a goes to the place, d stays.
    let over = point(&f, item(&f, "c"), 0.5, 0.75);
    move_to(&mut f, over);
    assert_eq!(
        (dy(&f, "a"), dy(&f, "b"), dy(&f, "c"), dy(&f, "d")),
        (2.0 * STEP, -STEP, -STEP, 0.0)
    );
    release_at(&mut f, over);
    assert_eq!(cards(&f, "c1"), ["b", "c", "a", "d"]);
    assert_clean(&f);
}

#[test]
fn both_draws_the_caret_with_the_gap_and_none_draws_neither() {
    let mut f = fixture(Options {
        indicator: SortIndicator::Both,
        ..Options::default()
    });
    grab(&mut f, "a", 0.5, 0.5);
    let over = point(&f, item(&f, "e"), 0.5, 0.75);
    move_to(&mut f, over);
    assert_eq!(dy(&f, "f"), STEP);
    assert!(classes(&f, item(&f, "e")).contains("rinch-sortable__item--drop-after"));
    key(&mut f, KeyCode::Escape, None);
    assert_clean(&f);

    let mut f = fixture(Options {
        indicator: SortIndicator::None,
        ..Options::default()
    });
    grab(&mut f, "a", 0.5, 0.5);
    let over = point(&f, item(&f, "e"), 0.5, 0.75);
    move_to(&mut f, over);
    assert_eq!(dy(&f, "f"), 0.0);
    assert!(carrying(&f, "rinch-sortable__item--drop-after").is_empty());
    // The app still knows where it would land.
    let state = sortable_drag().unwrap();
    assert_eq!(
        state.target,
        Some(SortTarget {
            list: "c2".into(),
            index: 1
        })
    );
    assert_eq!(state.verdict, Some(SortVerdict::Accept));
    release_at(&mut f, over);
    assert_eq!(cards(&f, "c2"), ["e", "a", "f"]);
}

#[test]
fn a_refused_place_opens_no_gap_and_still_says_why() {
    let mut f = fixture(gap_options());
    grab(&mut f, "d", 0.5, 0.5);
    let over = point(&f, item(&f, "e"), 0.5, 0.75);
    move_to(&mut f, over);
    assert_eq!((dy(&f, "e"), dy(&f, "f")), (0.0, 0.0), "no gap");
    let e = classes(&f, item(&f, "e"));
    assert!(
        e.contains("rinch-sortable__item--drop-after") && e.contains("--refused"),
        "{e}"
    );
    let shown = carrying(&f, "rinch-sortable__reason--shown");
    assert_eq!(shown.len(), 1);
    let state = sortable_drag().unwrap();
    assert_eq!(
        state.verdict,
        Some(SortVerdict::Refuse("No d in c2".into()))
    );
    // Back to an accepted place: the gap opens there.
    let over = point(&f, item(&f, "b"), 0.5, 0.25);
    move_to(&mut f, over);
    assert_eq!((dy(&f, "b"), dy(&f, "c")), (STEP, STEP));
    assert!(carrying(&f, "rinch-sortable__reason--shown").is_empty());
    key(&mut f, KeyCode::Escape, None);
    assert_clean(&f);
    assert_eq!(cards(&f, "c1"), ["a", "b", "c", "d"]);
}

/// The place is found from the boxes as painted, gap included: sweeping the
/// pointer down over c2 and back moves the place once each way, never back
/// and forth (a shift always moves the next threshold away from the pointer).
#[test]
fn a_sweep_over_a_shifting_list_changes_the_place_once_each_way() {
    let mut f = fixture(gap_options());
    grab(&mut f, "a", 0.5, 0.5);
    let (ex, ey, ew, _) = box_of(&f, item(&f, "e"));
    let x = ex + ew / 2.0;
    let index = || sortable_drag().unwrap().over.map(|t| t.index);
    let mut down = Vec::new();
    let mut y = ey + 2.0;
    while y < ey + 110.0 {
        move_to(&mut f, (x, y));
        down.push(index());
        y += 1.5;
    }
    let mut up = Vec::new();
    while y > ey + 2.0 {
        y -= 1.5;
        move_to(&mut f, (x, y));
        up.push(index());
    }
    let changes = |v: &[Option<usize>]| v.windows(2).filter(|w| w[0] != w[1]).count();
    assert!(down.windows(2).all(|w| w[0] <= w[1]), "down: {down:?}");
    assert!(up.windows(2).all(|w| w[0] >= w[1]), "up: {up:?}");
    assert_eq!(down.first(), Some(&Some(0)));
    assert_eq!(down.last(), Some(&Some(2)));
    assert_eq!(changes(&down), 2, "{down:?}");
    assert_eq!(changes(&up), 2, "{up:?}");
    key(&mut f, KeyCode::Escape, None);
}

/// In its own list a visible source (dimmed or unchanged) is never painted
/// over: dragging d up over c1 and back, the items between the place and d
/// make room and d sits at the place, every step; no two of c1's cards
/// overlap, and the place still changes once each way.
#[test]
fn a_sweep_in_the_sources_own_list_never_paints_one_card_over_another() {
    for source in [SortSource::Dim, SortSource::Unchanged] {
        let mut f = fixture(Options {
            indicator: SortIndicator::Gap,
            source,
            preview: SortPreview::None,
            apply: true,
        });
        grab(&mut f, "d", 0.5, 0.5);
        let (ax, ay, aw, _) = box_of(&f, item(&f, "a"));
        let (_, dy0, _, dh) = box_of(&f, item(&f, "d"));
        let x = ax + aw / 2.0;
        let index = || sortable_drag().unwrap().over.map(|t| t.index);
        let no_overlap = |f: &Fixture, at: f32| {
            let mut boxes: Vec<(String, f32, f32)> = ["a", "b", "c", "d"]
                .iter()
                .map(|k| {
                    let (_, y, _, h) = box_of(f, item(f, k));
                    (k.to_string(), y, y + h)
                })
                .collect();
            boxes.sort_by(|p, q| p.1.total_cmp(&q.1));
            for w in boxes.windows(2) {
                assert!(
                    w[0].2 <= w[1].1 + 0.5,
                    "{source:?} at y {at}: {} ({}..{}) over {} ({}..{})",
                    w[0].0,
                    w[0].1,
                    w[0].2,
                    w[1].0,
                    w[1].1,
                    w[1].2
                );
            }
        };
        let mut up = Vec::new();
        let mut y = dy0 + dh / 2.0;
        while y > ay + 2.0 {
            y -= 1.5;
            move_to(&mut f, (x, y));
            no_overlap(&f, y);
            up.push(index());
        }
        // At the top: d painted first, where a was.
        let (_, d_now, _, _) = box_of(&f, item(&f, "d"));
        assert!(
            (d_now - ay).abs() < 0.5,
            "{source:?}: d at {d_now}, a was at {ay}"
        );
        let mut down = Vec::new();
        while y < dy0 + dh / 2.0 {
            y += 1.5;
            move_to(&mut f, (x, y));
            no_overlap(&f, y);
            down.push(index());
        }
        let changes = |v: &[Option<usize>]| v.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(up.windows(2).all(|w| w[0] >= w[1]), "{source:?} up: {up:?}");
        assert!(
            down.windows(2).all(|w| w[0] <= w[1]),
            "{source:?} down: {down:?}"
        );
        assert_eq!(changes(&up), 3, "{source:?} {up:?}");
        assert_eq!(changes(&down), 3, "{source:?} {down:?}");
        release_at(&mut f, (x, ay + 2.0));
        assert_clean(&f);
    }
}
