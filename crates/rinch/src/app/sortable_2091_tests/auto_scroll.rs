use super::*;
use std::time::Duration;

fn frames(f: &mut Fixture, n: usize) {
    for _ in 0..n {
        std::thread::sleep(Duration::from_millis(5));
        send(f, PlatformEvent::AboutToWait);
    }
}

fn scroll_offset(f: &Fixture, id: u64) -> (f64, f64) {
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let o = d.tree.nodes[id].scroll_offset;
    (o.0, o.1)
}

#[test]
fn a_card_near_the_bottom_of_its_column_scrolls_the_column() {
    let mut f = fixture();
    let list = list_holding(&f, "a");
    start_card(&mut f, "b");
    let (lx, ly, lw, lh) = box_of(&f, list);
    move_to(&mut f, (lx + lw * 0.5, ly + lh - 3.0));
    assert_eq!(scroll_offset(&f, list), (0.0, 0.0));
    frames(&mut f, 3);
    let (x, y) = scroll_offset(&f, list);
    assert!(y > 0.0, "the column scrolled down: {y}");
    assert_eq!(x, 0.0);
}

#[test]
fn a_column_near_the_right_edge_of_the_row_scrolls_the_row() {
    let mut f = fixture();
    let row = list_holding(&f, "c1");
    let header = point(&f, handle_of(&f, "c1"), 0.5, 0.5);
    start_at(&mut f, header);
    let (rx, _, rw, _) = box_of(&f, row);
    move_to(&mut f, (rx + rw - 3.0, header.1));
    frames(&mut f, 3);
    let (x, y) = scroll_offset(&f, row);
    assert!(x > 0.0, "the row scrolled right: {x}");
    assert_eq!(y, 0.0);
    // And back: the left edge.
    move_to(&mut f, (rx + 2.0, header.1));
    let before = scroll_offset(&f, row).0;
    frames(&mut f, 3);
    assert!(scroll_offset(&f, row).0 < before, "scrolled back left");
}

#[test]
fn a_card_in_a_corner_scrolls_its_column_and_the_row_together() {
    let mut f = fixture();
    let row = list_holding(&f, "c1");
    let column = list_holding(&f, "e");
    start_card(&mut f, "a");
    // c3's list runs past the row's right edge at 500: the pointer at the
    // row's right edge and c3's list's bottom is in both bands.
    let (rx, _, rw, _) = box_of(&f, row);
    let (_, cy, _, ch) = box_of(&f, column);
    move_to(&mut f, (rx + rw - 3.0, cy + ch - 3.0));
    frames(&mut f, 3);
    assert!(scroll_offset(&f, row).0 > 0.0, "the row scrolled");
    assert!(
        scroll_offset(&f, column).1 == 0.0,
        "e f g fit their list, the refusal reason drawn inside g included"
    );
    // c1's list is the one that overflows vertically: take a card to
    // its bottom-left corner, by the row's left edge (the row now
    // scrolled right): the row scrolls back left and c1 down, both in
    // every frame.
    let c1 = list_holding(&f, "b");
    let (_, by, _, bh) = box_of(&f, c1);
    move_to(&mut f, (rx + 2.0, by + bh - 3.0));
    // Frames are wall-clock (up to 60px each), so the row may reach its
    // start in a frame or two: every frame that begins with the row
    // still scrolled must move both, and there is at least one.
    let mut both = 0;
    for frame in 0..3 {
        let (row_before, c1_before) = (scroll_offset(&f, row).0, scroll_offset(&f, c1).1);
        frames(&mut f, 1);
        let (row_after, c1_after) = (scroll_offset(&f, row).0, scroll_offset(&f, c1).1);
        if row_before > 0.0 {
            assert!(
                row_after < row_before && c1_after > c1_before,
                "frame {frame}: row {row_before} -> {row_after}, c1 {c1_before} -> {c1_after}"
            );
            both += 1;
        }
    }
    assert!(both > 0, "the row had scrolled right before");
}
