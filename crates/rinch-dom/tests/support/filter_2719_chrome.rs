//! The Chrome 157 screenshots of `fixtures/filter_2719.html` and how a
//! painter's pixels are compared with them (#2719), shared by the software
//! painter's tests (`filter_blur_2719_tests.rs`) and the GPU painter's
//! (`crates/rinch/tests/gpu_filter_2719.rs`). See the first's module doc for
//! the tolerances and why.
#![allow(dead_code)]

/// The page Chrome drew.
pub const PAGE: &str = include_str!("../fixtures/filter_2719.html");

/// Chrome's screenshot at `scale` (1 or 2), a PNG of RGB rows.
pub fn chrome_png(scale: u32) -> &'static [u8] {
    if scale == 1 {
        include_bytes!("../fixtures/filter_2719_chrome_1x.png")
    } else {
        include_bytes!("../fixtures/filter_2719_chrome_2x.png")
    }
}

/// How a case is compared (the module doc says why).
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    /// Every channel of every pixel within `max` levels of Chrome's.
    Exact(i32),
    /// Every channel within `nb` levels of the range of Chrome's 3x3
    /// neighbourhood, and the mean difference at most `mean`.
    Edges { nb: i32, mean: f64 },
    /// The mean difference at most this.
    Mean(f64),
}

/// `(id, CSS px region x0 y0 x1 y1, kind)`.
pub const CASES: &[(&str, [u32; 4], Kind)] = &[
    ("blur(1.3px)", [0, 0, 62, 52], Kind::Exact(3)),
    ("blur(4.7px)", [55, 0, 130, 60], Kind::Exact(3)),
    ("blur(9.3px)", [120, 0, 210, 75], Kind::Exact(3)),
    (
        "disc, drop-shadow(5px 7px 3.5px)",
        [212, 4, 280, 70],
        Kind::Edges { nb: 8, mean: 0.6 },
    ),
    (
        "ring, drop-shadow(-4px 3px 0 translucent)",
        [280, 0, 350, 62],
        Kind::Edges { nb: 32, mean: 1.0 },
    ),
    (
        "blur() drop-shadow() brightness()",
        [5, 75, 80, 130],
        Kind::Exact(3),
    ),
    ("gradient, blur(3.3px)", [78, 70, 165, 135], Kind::Exact(3)),
    ("text, drop-shadow()", [170, 75, 320, 125], Kind::Mean(4.5)),
    ("text, blur(1.1px)", [170, 128, 290, 165], Kind::Mean(4.5)),
    (
        "translucent background, opacity 0.7, blur()",
        [5, 145, 80, 205],
        Kind::Exact(3),
    ),
    (
        "translucent box, translucent drop-shadow()",
        [85, 140, 160, 205],
        Kind::Exact(3),
    ),
    (
        "overflowing child under drop-shadow()",
        [15, 195, 110, 245],
        Kind::Exact(3),
    ),
];

/// Compare `mine` (`w` device px wide, RGBA rows) with `theirs` (Chrome's
/// RGB rows at `scale`, decoded from [`chrome_png`]), case by case.
pub fn check_against_chrome(
    scale: u32,
    w: u32,
    mine: &[u8],
    theirs: &[[u8; 3]],
    cases: &[(&str, [u32; 4], Kind)],
) {
    assert_eq!(theirs.len(), (360 * 260 * scale * scale) as usize);
    let w = w as usize;
    let at = |v: &[[u8; 3]], x: usize, y: usize| v[y * w + x];
    let theirs = theirs.to_vec();
    let mine: Vec<[u8; 3]> = mine.chunks(4).map(|p| [p[0], p[1], p[2]]).collect();
    let h = theirs.len() / w;
    let mut failures = Vec::new();
    for &(name, [x0, y0, x1, y1], kind) in cases {
        let (x0, y0, x1, y1) = (
            (x0 * scale) as usize,
            (y0 * scale) as usize,
            (x1 * scale) as usize,
            (y1 * scale) as usize,
        );
        let (mut worst, mut worst_at, mut sum, mut n) = (0, (0, 0), 0i64, 0i64);
        for y in y0..y1 {
            for x in x0..x1 {
                let m = at(&mine, x, y);
                let c = at(&theirs, x, y);
                for ch in 0..3 {
                    let direct = (m[ch] as i32 - c[ch] as i32).abs();
                    sum += direct as i64;
                    n += 1;
                    let err = match kind {
                        Kind::Exact(_) | Kind::Mean(_) => direct,
                        Kind::Edges { .. } => {
                            let (mut lo, mut hi) = (255, 0);
                            for dy in -1i32..=1 {
                                for dx in -1i32..=1 {
                                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                                    if nx < 0 || ny < 0 || nx as usize >= w || ny as usize >= h {
                                        continue;
                                    }
                                    let v = at(&theirs, nx as usize, ny as usize)[ch] as i32;
                                    lo = lo.min(v);
                                    hi = hi.max(v);
                                }
                            }
                            (lo - m[ch] as i32).max(m[ch] as i32 - hi).max(0)
                        }
                    };
                    if err > worst {
                        worst = err;
                        worst_at = (x, y);
                    }
                }
            }
        }
        let mean = sum as f64 / n as f64;
        let ok = match kind {
            Kind::Exact(max) => worst <= max,
            Kind::Edges { nb, mean: m } => worst <= nb && mean <= m,
            Kind::Mean(m) => mean <= m,
        };
        if !ok {
            failures.push(format!(
                "{scale}x {name}: worst {worst} at {worst_at:?} (device px), mean {mean:.2}, \
                 allowed {kind:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
