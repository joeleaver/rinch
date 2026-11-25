//! `source`: how the carried items look while they are dragged.

use super::*;

// ── Source.

#[test]
fn a_collapsed_source_closes_up_and_reopens_over_its_own_place() {
    let mut f = fixture(Options {
        source: SortSource::Collapse,
        indicator: SortIndicator::Gap,
        ..Options::default()
    });
    grab(&mut f, "b", 0.5, 0.5);
    let b = classes(&f, item(&f, "b"));
    assert!(
        b.contains("rinch-sortable__item--collapsed") && !b.contains("--dragging"),
        "{b}"
    );
    // Over its own place: the space is where it was.
    assert_eq!((dy(&f, "a"), dy(&f, "c"), dy(&f, "d")), (0.0, 0.0, 0.0));
    // Off every list: c and d close up over b's space.
    move_to(&mut f, (790.0, 590.0));
    assert_eq!((dy(&f, "a"), dy(&f, "c"), dy(&f, "d")), (0.0, -STEP, -STEP));
    // Into c2 before f: c1 stays closed, c2 opens.
    let over = point(&f, item(&f, "f"), 0.5, 0.2);
    move_to(&mut f, over);
    assert_eq!(
        (dy(&f, "c"), dy(&f, "d"), dy(&f, "e"), dy(&f, "f")),
        (-STEP, -STEP, 0.0, STEP)
    );
    release_at(&mut f, over);
    assert_eq!(cards(&f, "c2"), ["e", "b", "f"]);
    assert_eq!(cards(&f, "c1"), ["a", "c", "d"]);
    assert_clean(&f);
}

#[test]
fn hide_and_unchanged_style_the_source_as_named() {
    let mut f = fixture(Options {
        source: SortSource::Hide,
        ..Options::default()
    });
    grab(&mut f, "b", 0.5, 0.5);
    move_to(&mut f, (790.0, 590.0));
    let b = classes(&f, item(&f, "b"));
    assert!(
        b.contains("rinch-sortable__item--drag-hidden") && !b.contains("--dragging"),
        "{b}"
    );
    assert_eq!(dy(&f, "c"), 0.0, "its space is kept");
    key(&mut f, KeyCode::Escape, None);
    assert_clean(&f);

    let mut f = fixture(Options {
        source: SortSource::Unchanged,
        ..Options::default()
    });
    grab(&mut f, "b", 0.5, 0.5);
    let b = classes(&f, item(&f, "b"));
    assert!(
        !b.contains("--dragging") && !b.contains("--drag-hidden") && !b.contains("--collapsed"),
        "{b}"
    );
    key(&mut f, KeyCode::Escape, None);
    assert_clean(&f);
}
