//! #1120: the selection highlight beside an image, on desktop.

use super::*;

// ── #1120: the selection highlight beside an image ──────────────────────
//
// An image has no bytes, so a selection's flat byte range cannot say which
// side of it an endpoint is on, nor whether the image itself is selected:
// `[5, 7)` is both "from before the image over `br`" and "from after it", and
// `[5, 5)` is "just the image" and nothing. The view now hands the host the
// selected images by model position (`query_selection_rects_with_boxes`).

impl Page {
    /// Select chars `i..j` of the paragraph and answer the highlight rects.
    fn select(&mut self, i: usize, j: usize) -> Vec<(f32, f32, f32, f32)> {
        self.handle
            .set_selection(Selection::text(Pos(i + 1), Pos(j + 1)));
        idle(&mut self.app);
        self.highlights()
    }
}

#[track_caller]
fn assert_span(r: (f32, f32, f32, f32), left: f32, right: f32, what: &str) {
    assert!(
        (r.0 - left).abs() < 1.5 && (r.0 + r.2 - right).abs() < 1.5,
        "{what}: {}..{}, want {left}..{right}",
        r.0,
        r.0 + r.2
    );
}

/// From right after an image over `br`: the highlight starts at the image's
/// right edge, where the caret after it is drawn. It started at its left edge
/// and covered the image, which is not selected.
#[test]
fn a_selection_from_after_an_image_starts_at_its_right_edge() {
    let mut p = page(&format!("<p>alpha{IMG}bravo</p>"));
    let img = p.images()[0];
    let hl = p.select(6, 8);
    assert_eq!(hl.len(), 1, "{hl:?}");
    let after_r = p.local(8).0 + p.block_box().0;
    assert_span(hl[0], img.0 + img.2, after_r, "after the image over `br`");
}

/// Just the image (Shift+Right from before it): the highlight is its box,
/// one line high. It painted nothing: the byte range was empty.
#[test]
fn a_selection_of_just_an_image_covers_its_box() {
    let mut p = page(&format!("<p>alpha{IMG}bravo</p>"));
    let img = p.images()[0];
    let hl = p.select(5, 6);
    assert_eq!(hl.len(), 1, "{hl:?}");
    assert_span(hl[0], img.0, img.0 + img.2, "the image");
    let (_, by, _, _) = p.block_box();
    assert_eq!((hl[0].1, hl[0].3), (by, LINE), "its line box: {hl:?}");
}

/// A selection ending right after an image covers it, and one ending right
/// before it stops at its left edge (the control: that was right).
#[test]
fn a_selection_ending_beside_an_image_ends_on_its_side() {
    let mut p = page(&format!("<p>alpha{IMG}bravo</p>"));
    let img = p.images()[0];
    let bx = p.block_box().0;
    let start = bx + p.local(1).0;
    let hl = p.select(1, 6);
    assert_eq!(hl.len(), 1, "{hl:?}");
    assert_span(hl[0], start, img.0 + img.2, "`lpha` and the image");
    let hl = p.select(1, 5);
    assert_eq!(hl.len(), 1, "{hl:?}");
    assert_span(hl[0], start, img.0, "`lpha`, not the image");
}

/// Text, the image and text: one rect over all three.
#[test]
fn a_selection_across_an_image_is_one_rect_over_it() {
    let mut p = page(&format!("<p>alpha{IMG}bravo</p>"));
    let bx = p.block_box().0;
    let (l, r) = (bx + p.local(3).0, bx + p.local(8).0);
    let hl = p.select(3, 8);
    assert_eq!(hl.len(), 1, "{hl:?}");
    assert_span(hl[0], l, r, "`ha`, the image, `br`");
}

/// Two images in a row share one byte with all four of their sides: each
/// alone, both, and the second with the text after it.
#[test]
fn selections_of_two_adjacent_images_are_each_their_own() {
    let mut p = page(&format!("<p>ab{IMG}{IMG}cd</p>"));
    let imgs = p.images();
    assert_eq!(imgs.len(), 2, "positive control");
    let (a, b) = (imgs[0], imgs[1]);
    let hl = p.select(2, 3);
    assert_eq!(hl.len(), 1, "{hl:?}");
    assert_span(hl[0], a.0, a.0 + a.2, "the first image");
    let hl = p.select(3, 4);
    assert_eq!(hl.len(), 1, "{hl:?}");
    assert_span(hl[0], b.0, b.0 + b.2, "the second image");
    let hl = p.select(2, 4);
    assert_eq!(hl.len(), 1, "{hl:?}");
    assert_span(hl[0], a.0, b.0 + b.2, "both images");
    let c_end = p.block_box().0 + p.local(5).0;
    let hl = p.select(3, 5);
    assert_eq!(hl.len(), 1, "{hl:?}");
    assert_span(hl[0], b.0, c_end, "the second image and `c`");
}

/// An image that starts a soft-wrapped line: selected alone, it is
/// highlighted on line 2 and nothing on line 1; selected with the end of
/// line 1, both lines are.
#[test]
fn a_selected_image_that_starts_a_wrapped_line_is_highlighted_there() {
    let mut p = page(&format!("<p>mmmmmmmmmmmmmmmmmmmmmmm {IMG}bravo</p>"));
    let img = p.images()[0];
    let (_, by, _, _) = p.block_box();
    assert!(
        img.1 > by + LINE * 0.9,
        "positive control: the image wrapped {img:?}"
    );
    let hl = p.select(24, 25);
    assert_eq!(hl.len(), 1, "{hl:?}");
    assert_span(hl[0], img.0, img.0 + img.2, "the image alone");
    assert_eq!(hl[0].1, by + LINE, "on line 2: {hl:?}");
    let hl = p.select(20, 25);
    assert_eq!(hl.len(), 2, "{hl:?}");
    assert_eq!(hl[1].1, by + LINE, "line 2: {hl:?}");
    assert_span(hl[1], img.0, img.0 + img.2, "line 2: the image");
}

/// An image that ends a soft-wrapped line (the space after it hangs there):
/// selected alone, it is highlighted on line 1; from after it, no rect
/// covers it.
#[test]
fn a_selected_image_that_ends_a_wrapped_line_is_highlighted_there() {
    let mut p = page(&format!(
        "<p>mmmmmmmmmmmmmmmmmmmm{IMG} bravocharliedeltaecho</p>"
    ));
    let img = p.images()[0];
    let (_, by, _, _) = p.block_box();
    assert!(img.1 < by + LINE, "positive control: on line 1 {img:?}");
    assert!(
        p.local(22).1 >= LINE,
        "positive control: `bravo…` wrapped to line 2"
    );
    let hl = p.select(20, 21);
    assert_eq!(hl.len(), 1, "{hl:?}");
    assert_span(hl[0], img.0, img.0 + img.2, "the image alone");
    assert_eq!(hl[0].1, by, "on line 1: {hl:?}");
    let hl = p.select(21, 24);
    assert!(!hl.is_empty(), "{hl:?}");
    for r in &hl {
        assert!(
            r.1 >= by + LINE || r.0 >= img.0 + img.2 - 1.5,
            "a rect from after the image covers it: {r:?} vs {img:?}"
        );
    }
    let hl = p.select(19, 21);
    assert_eq!(hl.len(), 1, "{hl:?}");
    assert_span(
        hl[0],
        p.block_box().0 + p.local(19).0,
        img.0 + img.2,
        "the last `m` and the image",
    );
}

impl Page {
    /// The painted pixel at the image's centre on a full frame.
    fn pixel_at_image_centre(&mut self) -> [u8; 4] {
        let img = self.images()[0];
        let (x, y) = ((img.0 + img.2 / 2.0) as u32, (img.1 + img.3 / 2.0) as u32);
        self.app.has_previous_frame = false;
        self.app.scene_dirty = true;
        let (px, w, _) = self.app.build_pixels(1.0, VP, false);
        let i = ((y * w + x) * 4) as usize;
        [px[i], px[i + 1], px[i + 2], px[i + 3]]
    }
}

/// The pixels (#1120): with nothing selected, with the image selected alone,
/// and from after it. Only the second tints the image's box.
#[test]
fn only_a_selected_image_is_painted_selected() {
    let mut p = page(&format!("<p>alpha{IMG}bravo</p>"));
    p.handle.set_selection(Selection::cursor(Pos(1)));
    idle(&mut p.app);
    let plain = p.pixel_at_image_centre();
    p.select(5, 6);
    let selected = p.pixel_at_image_centre();
    assert_ne!(selected, plain, "the selected image is tinted");
    p.select(6, 8);
    assert_eq!(
        p.pixel_at_image_centre(),
        plain,
        "from after the image: it is not"
    );
    p.select(4, 5);
    assert_eq!(
        p.pixel_at_image_centre(),
        plain,
        "up to before the image: it is not"
    );
}

/// An image that ends a paragraph: a selection up to before it stops at its
/// left edge. The byte range ends at the line's end, whose caret Parley
/// draws after the image.
#[test]
fn a_selection_up_to_a_trailing_image_stops_before_it() {
    let mut p = page(&format!("<p>alpha{IMG}</p>"));
    let img = p.images()[0];
    let start = p.block_box().0 + p.local(1).0;
    let hl = p.select(1, 5);
    assert_eq!(hl.len(), 1, "{hl:?}");
    assert_span(hl[0], start, img.0, "`lpha`, not the image");
    let hl = p.select(1, 6);
    assert_span(hl[0], start, img.0 + img.2, "`lpha` and the image");
}

// ── review of #1730 (scratch) ──
mod review_1730 {
    use super::*;

    #[test]
    fn rv_leading_adjacent_images_each_their_own() {
        let mut p = page(&format!("<p>{IMG}{IMG}cd</p>"));
        let imgs = p.images();
        assert_eq!(imgs.len(), 2);
        let (a, b) = (imgs[0], imgs[1]);
        let hl = p.select(0, 1);
        assert_eq!(hl.len(), 1, "{hl:?}");
        assert_span(hl[0], a.0, a.0 + a.2, "first leading image");
        let hl = p.select(1, 2);
        assert_eq!(hl.len(), 1, "{hl:?}");
        assert_span(hl[0], b.0, b.0 + b.2, "second leading image");
    }

    #[test]
    fn rv_trailing_adjacent_images_each_their_own() {
        let mut p = page(&format!("<p>ab{IMG}{IMG}</p>"));
        let imgs = p.images();
        let (a, b) = (imgs[0], imgs[1]);
        let hl = p.select(2, 3);
        assert_eq!(hl.len(), 1, "{hl:?}");
        assert_span(hl[0], a.0, a.0 + a.2, "first trailing image");
        let hl = p.select(3, 4);
        assert_eq!(hl.len(), 1, "{hl:?}");
        assert_span(hl[0], b.0, b.0 + b.2, "second trailing image");
        let start = p.block_box().0 + p.local(1).0;
        let hl = p.select(1, 3);
        assert_eq!(hl.len(), 1, "{hl:?}");
        assert_span(hl[0], start, a.0 + a.2, "`b` and the first image");
    }

    #[test]
    fn rv_image_in_a_link_alone() {
        let mut p = page(&format!("<p>ab<a href=\"https://e.x\">{IMG}</a>cd</p>"));
        let a = p.images()[0];
        let hl = p.select(2, 3);
        assert_eq!(hl.len(), 1, "{hl:?}");
        assert_span(hl[0], a.0, a.0 + a.2, "linked image alone");
    }

    #[test]
    fn rv_image_after_hard_break_alone() {
        let mut p = page(&format!("<p>ab<br>{IMG}cd</p>"));
        let a = p.images()[0];
        let hl = p.select(3, 4);
        assert_eq!(hl.len(), 1, "{hl:?}");
        assert_span(hl[0], a.0, a.0 + a.2, "image after a break, alone");
        let hl = p.select(4, 5);
        assert_eq!(hl.len(), 1, "{hl:?}");
        assert!(
            hl[0].0 >= a.0 + a.2 - 1.5,
            "from after the image: {hl:?} {a:?}"
        );
    }

    #[test]
    fn rv_image_before_hard_break_alone() {
        let mut p = page(&format!("<p>ab{IMG}<br>cd</p>"));
        let a = p.images()[0];
        let hl = p.select(2, 3);
        assert_eq!(hl.len(), 1, "{hl:?}");
        assert_span(hl[0], a.0, a.0 + a.2, "image before a break, alone");
    }

    #[test]
    fn rv_only_images_paragraph() {
        let mut p = page(&format!("<p>{IMG}{IMG}{IMG}</p>"));
        let imgs = p.images();
        let hl = p.select(1, 2);
        assert_eq!(hl.len(), 1, "{hl:?}");
        assert_span(hl[0], imgs[1].0, imgs[1].0 + imgs[1].2, "middle image");
        let hl = p.select(0, 3);
        assert_eq!(hl.len(), 1, "{hl:?}");
        assert_span(hl[0], imgs[0].0, imgs[2].0 + imgs[2].2, "all three");
    }

    #[test]
    fn rv_two_paragraphs_trailing_image_selected_into_next() {
        // `ab<img>` | `cd`: from after `b` to after `c` in the next block.
        let mut p = page(&format!("<p>ab{IMG}</p><p>cd</p>"));
        let a = p.images()[0];
        // chars: p1 content 0..3, p2 content starts at model 5 → (i+1) offsets
        p.handle.set_selection(Selection::text(Pos(3), Pos(7)));
        idle(&mut p.app);
        let hl = p.highlights();
        assert!(!hl.is_empty());
        let first = hl.iter().find(|r| (r.1 - a.1).abs() < LINE).unwrap();
        assert!(
            first.0 + first.2 >= a.0 + a.2 - 1.5,
            "first line takes the image: {hl:?} {a:?}"
        );
    }

    #[test]
    #[ignore]
    fn rv_perf_many_images_select_all() {
        for n in [250usize, 500, 1000, 2000] {
            let html = format!("<p>{}</p>", format!("x{IMG}").repeat(n));
            let mut p = page(&html);
            let t = std::time::Instant::now();
            for k in 0..5 {
                p.handle
                    .set_selection(Selection::text(Pos(1), Pos(2 * n + 1 - k)));
                idle(&mut p.app);
            }
            eprintln!("n={n}: {:?} per selection change", t.elapsed() / 5);
        }
    }
}
