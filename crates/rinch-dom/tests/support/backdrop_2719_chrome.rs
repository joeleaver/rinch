//! The Chrome 157 screenshots of `fixtures/backdrop_2719.html` (#2719) and
//! the cases compared on them, shared by the software painter's tests
//! (`backdrop_filter_2719_tests.rs`) and the GPU painter's
//! (`crates/rinch/tests/gpu_backdrop_2719.rs`). The comparison itself is
//! `filter_2719_chrome.rs`'s.
#![allow(dead_code)]

use super::chrome::Kind;

/// The page Chrome drew. Chrome scrolls `#scroller` to 37px with the
/// page's script; the tests scroll it with `set_scroll_top`.
pub const PAGE: &str = include_str!("../fixtures/backdrop_2719.html");

/// Chrome's screenshot at `scale` (1 or 2), a PNG of RGB rows.
pub fn chrome_png(scale: u32) -> &'static [u8] {
    if scale == 1 {
        include_bytes!("../fixtures/backdrop_2719_chrome_1x.png")
    } else {
        include_bytes!("../fixtures/backdrop_2719_chrome_2x.png")
    }
}

/// `(id, CSS px region x0 y0 x1 y1, kind)`. Measured worst cases (1x, 2x):
/// the plain, colour, greyscale, nested and translucent backdrops within 4
/// levels; the rounded ones off only on their anti-aliased corners (39
/// levels on an edge pixel, 1 against Chrome's neighbourhood); the one in
/// an `opacity` group within 18 levels of Chrome's neighbourhood on the rows
/// along its edges (inside the group Chrome reads one pixel past the glass
/// box: the 1px row directly above it bleeds in, by up to 18 levels at 1x and
/// 12 at 2x; the exact rule is still open; its interior is within 2); text behind a backdrop by the mean (the glyphs
/// are rinch's).
pub const CASES: &[(&str, [u32; 4], Kind)] = &[
    ("backdrop blur(5.3px)", [13, 11, 80, 54], Kind::Exact(4)),
    (
        "rounded, translucent background, blur(3.7px)",
        [97, 9, 168, 56],
        Kind::Edges { nb: 3, mean: 1.0 },
    ),
    (
        "blur() brightness() saturate()",
        [187, 13, 248, 54],
        Kind::Exact(4),
    ),
    ("grayscale() contrast()", [267, 11, 340, 56], Kind::Exact(4)),
    (
        "nested: a backdrop inside a backdrop",
        [17, 71, 148, 124],
        Kind::Exact(5),
    ),
    ("opacity 0.6", [171, 69, 234, 116], Kind::Exact(4)),
    (
        "in an opacity group (its backdrop root)",
        [251, 67, 342, 122],
        Kind::Edges { nb: 24, mean: 1.5 },
    ),
    (
        "rounded, in a scrolled scroller",
        [19, 145, 170, 242],
        Kind::Edges { nb: 3, mean: 1.0 },
    ),
    ("over text", [201, 151, 322, 200], Kind::Mean(4.5)),
];
