//! #1865: whole pages through the split page oracle
//! (`split_pages_1865_oracle.rs`): the #1894 review's curated pages, its
//! random page generator (and minimiser, for a failing seed), and the
//! unsliced-parity check over random pages, adopted into CI.
//!
//! `cargo test -p rinch --features gpu --lib -- --ignored gpu_split_pages`;
//! a failing random seed is minimised with `SPLIT_PAGES_MIN=1` and replayed
//! with `SPLIT_PAGES_FROM=<seed> SPLIT_PAGES_COUNT=1`.
use super::split_pages_1865_oracle::*;

const BASE: &str = r#"<div style="position:absolute;left:0;top:0;width:200px;height:120px;background:rgb(60,70,90)"></div>"#;

fn surf(id: u64, l: f64, t: f64, w: u32, h: u32, extra: &str) -> String {
    format!(
        r#"<div data-render-surface="{id}" style="position:absolute;left:{l}px;top:{t}px;width:{w}px;height:{h}px;{extra}"></div>"#
    )
}

fn abs(l: f64, t: f64, w: f64, h: f64, extra: &str, inner: &str) -> String {
    format!(
        r#"<div style="position:absolute;left:{l}px;top:{t}px;width:{w}px;height:{h}px;{extra}">{inner}</div>"#
    )
}

thread_local! {
    /// The curated pages' failures, collected by `run`.
    static FAILURES: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// One page through the oracle; a page past its allowance (one level, D8 on
/// a ring) and not tainted is a failure.
fn run(name: &str, html: &str, frames: &[Frame]) -> Outcome {
    let o = oracle(html, frames);
    eprintln!("CASE {}", o.summary(name));
    if !o.tainted.iter().any(|t| *t) && !o.over(1).is_empty() {
        FAILURES.with(|f| f.borrow_mut().push(o.summary(name)));
    }
    o
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_pages_curated() {
    FAILURES.with(|f| f.borrow_mut().clear());
    let f1 = || vec![opaque(1, 40, 26)];
    // c0 plain
    run(
        "c0_plain",
        &format!("{BASE}{}", surf(1, 20.0, 12.0, 40, 26, "")),
        &f1(),
    );
    // c1 bar straddling R
    run(
        "c1_bar",
        &format!(
            "{BASE}{}{}",
            surf(1, 20.0, 12.0, 40, 26, ""),
            abs(10.0, 30.0, 80.0, 12.0, "background:rgba(0,0,0,0.55)", "")
        ),
        &f1(),
    );
    // c2: a split while below's inverse clip is open
    run(
        "c2_split_while_inverse_open",
        &format!(
            "{BASE}{}{}{}{}",
            surf(1, 20.0, 12.0, 40, 26, ""),
            abs(
                10.0,
                30.0,
                40.0,
                12.0,
                "background:rgba(255,255,255,0.5)",
                ""
            ),
            surf(2, 100.0, 12.0, 40, 26, ""),
            abs(90.0, 32.0, 80.0, 12.0, "background:rgba(255,255,0,0.5)", "")
        ),
        &[opaque(1, 40, 26), opaque(2, 40, 20)],
    );
    // c2b control: same without the straddler before surface 2
    run(
        "c2b_control",
        &format!(
            "{BASE}{}{}{}",
            surf(1, 20.0, 12.0, 40, 26, ""),
            surf(2, 100.0, 12.0, 40, 26, ""),
            abs(90.0, 32.0, 80.0, 12.0, "background:rgba(255,255,0,0.5)", "")
        ),
        &[opaque(1, 40, 26), opaque(2, 40, 20)],
    );
    // c2c: the straddler is caption text within 2em of R1
    run(
        "c2c_caption_text",
        &format!(
            "{BASE}{}{}{}{}",
            surf(1, 20.0, 12.0, 40, 26, ""),
            abs(
                20.0,
                42.0,
                60.0,
                14.0,
                "font-size:12px;line-height:14px;color:white",
                "Caption"
            ),
            surf(2, 100.0, 12.0, 40, 26, ""),
            abs(90.0, 32.0, 80.0, 12.0, "background:rgba(255,255,0,0.5)", "")
        ),
        &[opaque(1, 40, 26), opaque(2, 40, 20)],
    );
    // c3 rounded card with controls inside and tooltip after
    run(
        "c3_card",
        &format!(
            "{BASE}{}{}",
            abs(
                10.0,
                6.0,
                60.0,
                38.0,
                "border-radius:10px;overflow:hidden;background:rgb(200,40,40)",
                &format!(
                    "{}{}",
                    surf(1, 10.0, 6.0, 40, 26, ""),
                    abs(5.0, 24.0, 50.0, 10.0, "background:rgba(0,0,0,0.5)", "")
                )
            ),
            abs(
                30.0,
                20.0,
                50.0,
                10.0,
                "background:rgba(255,255,255,0.6)",
                ""
            )
        ),
        &f1(),
    );
    // c4 box-shadow of a disjoint box cast onto R
    run(
        "c4_shadow",
        &format!(
            "{BASE}{}{}",
            surf(1, 20.0, 12.0, 40, 26, ""),
            abs(
                70.0,
                12.0,
                30.0,
                26.0,
                "background:white;box-shadow:-14px 0 10px rgba(0,0,0,0.9)",
                ""
            )
        ),
        &f1(),
    );
    // c4b text-shadow with blur from text disjoint from R
    run(
        "c4b_text_shadow",
        &format!(
            "{BASE}{}{}",
            surf(1, 20.0, 12.0, 40, 26, ""),
            abs(
                64.0,
                14.0,
                80.0,
                20.0,
                "font-size:16px;line-height:20px;color:white;text-shadow:-30px 0 6px rgba(255,0,0,0.9)",
                "Shadowed"
            )
        ),
        &f1(),
    );
    // c5 big text overlapping R
    run(
        "c5_text",
        &format!(
            "{BASE}{}{}",
            surf(1, 20.0, 12.0, 40, 26, ""),
            abs(
                30.0,
                0.0,
                120.0,
                50.0,
                "font-size:36px;line-height:40px;color:rgba(255,255,0,0.7)",
                "Jg"
            )
        ),
        &f1(),
    );
    // c6 thick dashed border near R
    run(
        "c6_border",
        &format!(
            "{BASE}{}{}",
            surf(1, 20.0, 12.0, 40, 26, ""),
            abs(
                52.0,
                4.0,
                30.0,
                30.0,
                "border:6px dashed rgba(0,0,255,0.6)",
                ""
            )
        ),
        &f1(),
    );
    // c6b outline after
    run(
        "c6b_outline",
        &format!(
            "{BASE}{}{}",
            surf(1, 20.0, 12.0, 40, 26, ""),
            abs(
                64.0,
                10.0,
                20.0,
                20.0,
                "outline:5px solid rgba(0,255,0,0.6);outline-offset:3px",
                ""
            )
        ),
        &f1(),
    );
    // c7 scaled element straddling
    run(
        "c7_scaled",
        &format!(
            "{BASE}{}{}",
            surf(1, 20.0, 12.0, 40, 26, ""),
            abs(
                64.0,
                20.0,
                20.0,
                10.0,
                "background:rgba(255,0,255,0.6);transform:scale(2.5)",
                ""
            )
        ),
        &f1(),
    );
    // c8 opacity group straddling
    run(
        "c8_opacity_group",
        &format!(
            "{BASE}{}{}",
            surf(1, 20.0, 12.0, 40, 26, ""),
            abs(
                10.0,
                20.0,
                80.0,
                30.0,
                "opacity:0.5",
                &abs(0.0, 0.0, 60.0, 10.0, "background:rgb(255,128,0)", "")
            )
        ),
        &f1(),
    );
    // c9 fractional position
    run(
        "c9_fractional",
        &format!(
            "{BASE}{}{}",
            surf(1, 20.5, 12.25, 40, 26, ""),
            abs(10.0, 30.0, 80.0, 12.0, "background:rgba(0,0,0,0.55)", "")
        ),
        &f1(),
    );
    // c10 whole-pixel clip cutting R
    run(
        "c10_clip",
        &format!(
            "{BASE}{}",
            abs(
                0.0,
                0.0,
                50.0,
                30.0,
                "overflow:hidden",
                &format!(
                    "{}{}",
                    surf(1, 20.0, 12.0, 40, 26, ""),
                    abs(
                        10.0,
                        14.0,
                        60.0,
                        8.0,
                        "background:rgba(255,255,255,0.5)",
                        ""
                    )
                )
            )
        ),
        &f1(),
    );
    // c11 z-index child in a rounded card, painted later
    run(
        "c11_zindex",
        &format!(
            "{BASE}{}",
            abs(
                10.0,
                6.0,
                60.0,
                38.0,
                "border-radius:10px;overflow:hidden;background:rgb(200,40,40)",
                &format!(
                    "{}{}{}",
                    abs(
                        0.0,
                        20.0,
                        60.0,
                        8.0,
                        "z-index:2;background:rgba(0,255,0,0.5)",
                        ""
                    ),
                    surf(1, 10.0, 6.0, 40, 26, ""),
                    abs(5.0, 24.0, 50.0, 10.0, "background:rgba(0,0,0,0.5)", "")
                )
            )
        ),
        &f1(),
    );
    let failures = FAILURES.with(|f| f.borrow().clone());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }
    fn pos(&mut self, max: u64) -> f64 {
        let v = self.below(max) as f64;
        if self.chance(25) {
            v + [0.25, 0.5, 0.75][self.below(3) as usize]
        } else {
            v
        }
    }
    fn rgba(&mut self) -> String {
        let a = if self.chance(30) {
            1.0
        } else {
            (self.below(80) as f64 + 15.0) / 100.0
        };
        format!(
            "rgba({},{},{},{a})",
            self.below(256),
            self.below(256),
            self.below(256)
        )
    }
}

#[derive(Clone)]
struct N {
    style: String,
    surface: Option<usize>,
    text: String,
    children: Vec<N>,
}

impl N {
    fn html(&self) -> String {
        let mut s = String::new();
        match self.surface {
            Some(id) => s += &format!(r#"<div data-render-surface="{id}" style="{}">"#, self.style),
            None => s += &format!(r#"<div style="{}">"#, self.style),
        }
        s += &self.text;
        for c in &self.children {
            s += &c.html();
        }
        s += "</div>";
        s
    }
    fn count(&self) -> usize {
        1 + self.children.iter().map(N::count).sum::<usize>()
    }
    /// Remove the `k`th node in pre-order (not self); true if removed.
    fn remove(&mut self, k: &mut usize) -> bool {
        let mut i = 0;
        while i < self.children.len() {
            if *k == 0 {
                self.children.remove(i);
                return true;
            }
            *k -= 1;
            if self.children[i].remove(k) {
                return true;
            }
            i += 1;
        }
        false
    }
    fn surfaces(&self, out: &mut Vec<usize>) {
        if let Some(id) = self.surface {
            out.push(id);
        }
        for c in &self.children {
            c.surfaces(out);
        }
    }
}

struct Gen {
    rng: Rng,
    next_id: usize,
    frames: Vec<Frame>,
}

impl Gen {
    fn style(&mut self, depth: u32) -> String {
        let r = &mut self.rng;
        let mut s = String::new();
        if r.chance(60) {
            s += &format!("background:{};", r.rgba());
        }
        if r.chance(30) {
            s += &format!("border-radius:{}px;", r.below(14));
        }
        if r.chance(15) {
            s += &format!(
                "border:{}px {} {};",
                r.below(6) + 1,
                ["solid", "dashed", "dotted", "double"][r.below(4) as usize],
                r.rgba()
            );
        }
        if r.chance(15) {
            s += &format!(
                "box-shadow:{}px {}px {}px {};",
                r.below(30) as i64 - 15,
                r.below(30) as i64 - 15,
                r.below(12),
                r.rgba()
            );
        }
        if r.chance(10) {
            s += &format!("opacity:{};", (r.below(80) + 10) as f64 / 100.0);
        }
        if r.chance(10) {
            s += &format!(
                "transform:scale({});",
                [1.5, 2.0, 0.5, 1.25][r.below(4) as usize]
            );
        } else if r.chance(5) {
            s += &format!("transform:translate({}px,{}px);", r.pos(20), r.pos(10));
        } else if r.chance(3) {
            s += "transform:rotate(10deg);";
        }
        if r.chance(10) {
            s += &format!("z-index:{};", r.below(4));
        }
        if r.chance(4) {
            s += "filter:grayscale(1);";
        }
        if r.chance(6) {
            s += &format!("outline:{}px solid {};", r.below(5) + 1, r.rgba());
        }
        if depth < 3 && r.chance(35) {
            s += "overflow:hidden;";
        }
        s
    }

    fn element(&mut self, depth: u32, pw: u64, ph: u64) -> N {
        let l = self.rng.pos(pw);
        let t = self.rng.pos(ph);
        let w = self.rng.below(90) + 6;
        let h = self.rng.below(60) + 6;
        if self.next_id < 4 && self.rng.chance(if depth == 0 { 30 } else { 25 }) {
            let id = self.next_id;
            self.next_id += 1;
            let (cw, ch) = (
                self.rng.below(60) as u32 + 10,
                self.rng.below(40) as u32 + 8,
            );
            let mut extra = String::new();
            if self.rng.chance(25) {
                extra += &format!("border-radius:{}px;", self.rng.below(12) + 1);
            }
            if self.rng.chance(20) {
                extra += &format!("padding:{}px;", self.rng.below(5));
            }
            if self.rng.chance(20) {
                extra += &format!("background:{};", self.rng.rgba());
            }
            if self.rng.chance(8) {
                extra += "opacity:0.5;";
            }
            let (fw, fh) = match self.rng.below(3) {
                0 => (cw, ch),
                1 => (cw, (ch * 3 / 4).max(1)),
                _ => (cw / 2 + 1, ch),
            };
            let translucent = self.rng.chance(30);
            self.frames.push(Frame {
                id,
                w: fw,
                h: fh,
                px: frame_pixels(fw, fh, translucent, id as u32 * 17),
            });
            return N {
                style: format!(
                    "position:absolute;left:{l}px;top:{t}px;width:{cw}px;height:{ch}px;{extra}"
                ),
                surface: Some(id),
                text: String::new(),
                children: Vec::new(),
            };
        }
        let style = self.style(depth);
        let mut text = String::new();
        if self.rng.chance(20) {
            text = format!(
                r#"<span style="font-size:{}px;line-height:{}px;color:{}">Ag Jy</span>"#,
                self.rng.below(24) + 8,
                self.rng.below(24) + 10,
                self.rng.rgba()
            );
        }
        let mut children = Vec::new();
        if depth < 3 {
            let n = self.rng.below(4);
            for _ in 0..n {
                children.push(self.element(depth + 1, w, h));
            }
        }
        N {
            style: format!(
                "position:absolute;left:{l}px;top:{t}px;width:{w}px;height:{h}px;{style}"
            ),
            surface: None,
            text,
            children,
        }
    }
}

fn generate(seed: u64) -> (N, Vec<Frame>) {
    let mut g = Gen {
        rng: Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) ^ 0xDEADBEEF),
        next_id: 1,
        frames: Vec::new(),
    };
    let mut root = N {
        style: "position:absolute;left:0;top:0;width:200px;height:120px".into(),
        surface: None,
        text: String::new(),
        children: vec![N {
            style:
                "position:absolute;left:0;top:0;width:200px;height:120px;background:rgb(60,70,90)"
                    .into(),
            surface: None,
            text: String::new(),
            children: vec![],
        }],
    };
    let n = g.rng.below(8) + 3;
    for _ in 0..n {
        let e = g.element(0, 190, 110);
        root.children.push(e);
    }
    (root, g.frames)
}

fn frames_for(root: &N, frames: &[Frame]) -> Vec<Frame> {
    let mut ids = Vec::new();
    root.surfaces(&mut ids);
    frames
        .iter()
        .filter(|f| ids.contains(&f.id))
        .map(|f| Frame {
            id: f.id,
            w: f.w,
            h: f.h,
            px: f.px.clone(),
        })
        .collect()
}

/// Bad: untainted with a pixel past its allowance: `tol` levels when given,
/// else its D10 classes'.
fn bad(root: &N, frames: &[Frame], tol: Option<i32>) -> Option<Outcome> {
    let fr = frames_for(root, frames);
    let o = oracle(&root.html(), &fr);
    let tainted = o.tainted.iter().any(|t| *t);
    (!tainted && !past(&o, tol).is_empty()).then_some(o)
}

fn past(o: &Outcome, tol: Option<i32>) -> Vec<super::split_pages_1865_oracle::Off> {
    match tol {
        Some(t) => o.over(t),
        None => o.over_classes(),
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_pages_random() {
    let env = |k: &str, d: u64| {
        std::env::var(k)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(d)
    };
    let from = env("SPLIT_PAGES_FROM", 0);
    let count = env("SPLIT_PAGES_COUNT", 60);
    // Each pixel to its D10 classes (`Outcome::over_classes`), as the hand
    // oracles hold theirs; `SPLIT_PAGES_TOL` sets a flat tolerance instead,
    // for exploring.
    let tol = std::env::var("SPLIT_PAGES_TOL")
        .ok()
        .and_then(|v| v.parse::<i32>().ok());
    let minimize = std::env::var("SPLIT_PAGES_MIN").is_ok();
    let mut nbad = 0;
    let mut split_cases = 0;
    for seed in from..from + count {
        let (mut root, frames) = generate(seed);
        if frames.is_empty() {
            continue;
        }
        let o = oracle(&root.html(), &frames);
        if o.regions.is_empty() {
            continue;
        }
        split_cases += 1;
        let tainted = o.tainted.iter().any(|t| *t);
        if tainted || past(&o, tol).is_empty() {
            continue;
        }
        nbad += 1;
        eprintln!("SEED {seed} {}", o.summary("rand"));
        for (x, y, a, b, d) in past(&o, tol).into_iter().take(8) {
            let near = o
                .regions
                .iter()
                .map(|&(rx, ry, rw, rh)| {
                    let dx = (rx as i64 - x as i64)
                        .max(x as i64 - (rx + rw) as i64 + 1)
                        .max(0);
                    let dy = (ry as i64 - y as i64)
                        .max(y as i64 - (ry + rh) as i64 + 1)
                        .max(0);
                    dx.max(dy)
                })
                .min();
            let i = (y * 200 + x) as usize;
            eprintln!(
                "  past ({x},{y}) d{d} allow {} split {a:?} scene {b:?} from R {near:?}",
                o.allow.get(i).copied().unwrap_or(-1)
            );
        }
        if minimize {
            loop {
                let mut changed = false;
                let total = root.count() - 1;
                let mut k = total;
                while k > 0 {
                    k -= 1;
                    let mut cand = root.clone();
                    let mut kk = k;
                    if !cand.remove(&mut kk) {
                        continue;
                    }
                    if bad(&cand, &frames, tol).is_some() {
                        root = cand;
                        changed = true;
                    }
                }
                if !changed {
                    break;
                }
            }
            let o = bad(&root, &frames, tol).unwrap();
            eprintln!("MIN {seed} {}", o.summary("min"));
            eprintln!("MINHTML {seed} {}", root.html());
            for f in frames_for(&root, &frames) {
                eprintln!(
                    "FRAME {seed} id={} {}x{} translucent={}",
                    f.id,
                    f.w,
                    f.h,
                    f.px.chunks(4).any(|p| p[3] != 255)
                );
            }
        }
    }
    eprintln!("RANDOM from={from} count={count} split_cases={split_cases} bad={nbad}");
    assert!(
        split_cases > count / 4,
        "too few pages split: {split_cases}"
    );
    assert_eq!(
        nbad, 0,
        "{nbad} random pages past their allowance (seeds above)"
    );
}

fn enc_fingerprint(s: &vello::Scene) -> String {
    let e = s.encoding();
    let mut out = format!(
        "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{}|{}|{}|{}|{}|",
        e.path_tags.iter().map(|t| t.0).collect::<Vec<_>>(),
        e.path_data,
        e.draw_tags.iter().map(|t| t.0).collect::<Vec<_>>(),
        e.draw_data,
        e.transforms,
        e.styles
            .iter()
            .map(|s| (s.flags_and_miter_limit, s.line_width))
            .collect::<Vec<_>>(),
        e.n_paths,
        e.n_path_segments,
        e.n_clips,
        e.n_open_clips,
        e.flags
    );
    let r = &e.resources;
    out += &format!(
        "{:?}|{:?}|{:?}|{}|",
        r.glyphs,
        r.color_stops,
        r.normalized_coords,
        r.patches.len()
    );
    for g in &r.glyph_runs {
        out += &format!(
            "run({:?},{:?},{},{},{:?},{:?},{:?},{})",
            g.transform,
            g.glyph_transform,
            g.font_size,
            g.hint,
            g.glyphs,
            g.normalized_coords,
            g.stream_offsets,
            g.font.data.id()
        );
    }
    out
}

/// A frame opened for slicing encodes the window's scene byte for byte as an
/// unsliced one wherever nothing splits (no GPU frame, or every surface
/// refused), glyph runs included; where something splits, every scene
/// balances and the window's scene differs (the control). No GPU.
#[test]
fn split_pages_unsliced_parity() {
    let count = std::env::var("SPLIT_PAGES_COUNT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(150u64);
    let (mut same_nosurf, mut same_refused, mut diff, mut split_seen) = (0, 0, 0, 0);
    for seed in 0..count {
        let (root, frames) = generate(seed);
        let html = root.html();
        // A: no GPU frames at all (surfaces paint nothing).
        let mut doc = doc_from_html(&html);
        let (a0, _) = paint(&mut doc, &[], false);
        let (a1, s) = paint(&mut doc, &[], true);
        assert!(s.is_empty());
        if enc_fingerprint(&a0) == enc_fingerprint(&a1) {
            same_nosurf += 1;
        } else {
            diff += 1;
            eprintln!("PARITY-DIFF nosurf seed {seed}");
        }
        // B: with frames; compare only when nothing split.
        let (b0, _) = paint(&mut doc, &frames, false);
        let (b1, s) = paint(&mut doc, &frames, true);
        if s.is_empty() {
            if enc_fingerprint(&b0) == enc_fingerprint(&b1) {
                same_refused += 1;
            } else {
                diff += 1;
                eprintln!("PARITY-DIFF refused seed {seed}");
            }
        } else {
            split_seen += 1;
            assert_eq!(
                b1.encoding().n_open_clips,
                0,
                "below unbalanced seed {seed}"
            );
            for sl in &s {
                for sc in [&sl.above_in, &sl.above_out].into_iter().flatten() {
                    assert_eq!(
                        sc.encoding().n_open_clips,
                        0,
                        "above unbalanced seed {seed}"
                    );
                }
                if let rinch_dom::paint::slicing::SliceCoverage::Mask(m) = &sl.own {
                    assert_eq!(m.encoding().n_open_clips, 0);
                }
                if let rinch_dom::paint::slicing::SliceCoverage::Mask(m) = &sl.ancestors {
                    assert_eq!(m.encoding().n_open_clips, 0);
                }
            }
            // positive control: a split frame must differ
            assert_ne!(enc_fingerprint(&b0), enc_fingerprint(&b1), "seed {seed}");
        }
    }
    eprintln!(
        "PARITY same_nosurf={same_nosurf} same_refused={same_refused} diff={diff} split_seen={split_seen}"
    );
    assert_eq!(diff, 0, "unsliced parity broken (seeds above)");
    assert!(
        split_seen > 0 && same_refused > 0,
        "control: the pages never split, or never refused"
    );
}
