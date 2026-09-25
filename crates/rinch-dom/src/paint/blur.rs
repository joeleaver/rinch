//! The one Gaussian blur paint uses: a separable approximation over `f32`
//! coverage, shared by the inset `box-shadow` (#974) and the blurred
//! `text-shadow` mask (#980).

/// A 1-D approximation of a Gaussian blur of standard deviation `sigma`: a
/// sampled kernel out to `3 sigma` for a small `sigma`, three box blurs
/// (Kovesi, "Fast Almost-Gaussian Filtering" — Skia's own method) for a large
/// one. Values past either end of a line count as zero.
pub(super) enum Blur1d {
    Kernel(Vec<f32>),
    Boxes([usize; 3]),
}

impl Blur1d {
    /// A blur of standard deviation `sigma`, sampled up to `direct_max_sigma`
    /// and three box blurs past it. The kernel costs `6 sigma` per sample,
    /// the boxes a constant; above about 2.5 the boxes are within a level of
    /// the Gaussian.
    pub(super) fn new(sigma: f64, direct_max_sigma: f64) -> Self {
        if sigma <= direct_max_sigma {
            let radius = (3.0 * sigma).ceil().max(1.0) as i64;
            let mut k: Vec<f64> = (-radius..=radius)
                .map(|i| (-(i as f64).powi(2) / (2.0 * sigma * sigma)).exp())
                .collect();
            let sum: f64 = k.iter().sum();
            k.iter_mut().for_each(|v| *v /= sum);
            Blur1d::Kernel(k.into_iter().map(|v| v as f32).collect())
        } else {
            const N: f64 = 3.0;
            let ideal = (12.0 * sigma * sigma / N + 1.0).sqrt();
            let mut wl = ideal.floor() as i64;
            if wl % 2 == 0 {
                wl -= 1;
            }
            let wl = wl.max(1);
            let wlf = wl as f64;
            let m = ((12.0 * sigma * sigma - N * wlf * wlf - 4.0 * N * wlf - 3.0 * N)
                / (-4.0 * wlf - 4.0))
                .round() as i64;
            let mut widths = [0usize; 3];
            for (i, w) in widths.iter_mut().enumerate() {
                *w = if (i as i64) < m { wl } else { wl + 2 } as usize;
            }
            Blur1d::Boxes(widths)
        }
    }

    /// How far one sample's value spreads, in samples.
    pub(super) fn reach(&self) -> usize {
        match self {
            Blur1d::Kernel(k) => k.len() / 2,
            Blur1d::Boxes(w) => w.iter().map(|w| w / 2).sum(),
        }
    }

    /// Blur `line` in place; `tmp` is scratch, reused across calls.
    pub(super) fn apply(&self, line: &mut [f32], tmp: &mut Vec<f32>) {
        let n = line.len();
        tmp.clear();
        tmp.extend_from_slice(line);
        match self {
            Blur1d::Kernel(k) => {
                let r = k.len() / 2;
                for (i, out) in line.iter_mut().enumerate() {
                    let lo = i.saturating_sub(r);
                    let hi = (i + r).min(n - 1);
                    let mut acc = 0.0f32;
                    for (s, v) in tmp[lo..=hi].iter().enumerate() {
                        acc += v * k[lo + s + r - i];
                    }
                    *out = acc;
                }
            }
            Blur1d::Boxes(widths) => {
                for &w in widths {
                    let r = w / 2;
                    if r == 0 {
                        continue;
                    }
                    let norm = 1.0 / w as f32;
                    let mut sum: f32 = tmp.iter().take(r.min(n)).sum();
                    for i in 0..n {
                        if i + r < n {
                            sum += tmp[i + r];
                        }
                        line[i] = sum * norm;
                        if i >= r {
                            sum -= tmp[i - r];
                        }
                    }
                    tmp.copy_from_slice(line);
                }
            }
        }
    }

    /// Blur every column of the `w` x `h` row-major `mask` in place,
    /// sweeping rows so the mask is read in memory order. Column for column
    /// the same sums as [`apply`](Self::apply), in the same order.
    pub(super) fn apply_columns(&self, mask: &mut [f32], w: usize, h: usize, src: &mut Vec<f32>) {
        if w == 0 || h == 0 {
            return;
        }
        src.clear();
        src.extend_from_slice(mask);
        match self {
            Blur1d::Kernel(k) => {
                let r = k.len() / 2;
                for y in 0..h {
                    let out = &mut mask[y * w..(y + 1) * w];
                    out.fill(0.0);
                    let lo = y.saturating_sub(r);
                    let hi = (y + r).min(h - 1);
                    for sy in lo..=hi {
                        let kv = k[sy + r - y];
                        let row = &src[sy * w..(sy + 1) * w];
                        for (o, v) in out.iter_mut().zip(row) {
                            *o += kv * v;
                        }
                    }
                }
            }
            Blur1d::Boxes(widths) => {
                let mut sum = vec![0.0_f32; w];
                for &bw in widths {
                    let r = bw / 2;
                    if r == 0 {
                        continue;
                    }
                    let norm = 1.0 / bw as f32;
                    sum.fill(0.0);
                    for sy in 0..r.min(h) {
                        for (s, v) in sum.iter_mut().zip(&src[sy * w..(sy + 1) * w]) {
                            *s += v;
                        }
                    }
                    for y in 0..h {
                        if y + r < h {
                            let add = &src[(y + r) * w..(y + r + 1) * w];
                            for (s, v) in sum.iter_mut().zip(add) {
                                *s += v;
                            }
                        }
                        let out = &mut mask[y * w..(y + 1) * w];
                        for (o, s) in out.iter_mut().zip(&sum) {
                            *o = s * norm;
                        }
                        if y >= r {
                            let sub = &src[(y - r) * w..(y - r + 1) * w];
                            for (s, v) in sum.iter_mut().zip(sub) {
                                *s -= v;
                            }
                        }
                    }
                    src.copy_from_slice(mask);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Blur1d;

    /// The column sweep is the row blur turned on its side: a mask blurred
    /// by columns equals its transpose blurred by rows, bit for bit.
    ///
    /// Kills: a column pass that drops a box, skips a row or reads the
    /// wrong window.
    #[test]
    fn columns_are_rows_transposed() {
        for sigma in [0.6, 1.2, 3.0, 7.5] {
            let blur = Blur1d::new(sigma, 1.25);
            let (w, h) = (7, 41);
            let mask: Vec<f32> = (0..w * h)
                .map(|i| if (i * 7919) % 13 < 4 { 1.0 } else { 0.0 })
                .collect();
            let mut by_cols = mask.clone();
            blur.apply_columns(&mut by_cols, w, h, &mut Vec::new());
            let mut tmp = Vec::new();
            for x in 0..w {
                let mut col: Vec<f32> = (0..h).map(|y| mask[y * w + x]).collect();
                blur.apply(&mut col, &mut tmp);
                for y in 0..h {
                    assert_eq!(col[y], by_cols[y * w + x], "sigma {sigma} at ({x}, {y})");
                }
            }
        }
    }

    /// The sampled kernel, which blurs every `0 1px 2px` text shadow at scale
    /// 1, is a Gaussian of the asked-for `sigma` out to three of them: an
    /// impulse comes back as the analytic profile, within 6% of its peak.
    ///
    /// Kills: the kernel cut at `2 sigma`; a `sigma` off by a factor.
    #[test]
    fn a_small_blur_is_the_gaussian_out_to_three_sigma() {
        for sigma in [0.5, 1.0, 1.25] {
            let blur = Blur1d::new(sigma, 1.25);
            assert!(matches!(blur, Blur1d::Kernel(_)));
            let n = 41;
            let mut line = vec![0.0_f32; n];
            line[n / 2] = 1.0;
            blur.apply(&mut line, &mut Vec::new());
            let reach = (3.0 * sigma).ceil() as i64;
            let norm: f64 = (-reach..=reach)
                .map(|i| (-(i * i) as f64 / (2.0 * sigma * sigma)).exp())
                .sum();
            let peak = 1.0 / norm;
            for (i, &v) in line.iter().enumerate() {
                let d = i as i64 - (n / 2) as i64;
                let want = if d.abs() <= reach {
                    (-(d * d) as f64 / (2.0 * sigma * sigma)).exp() / norm
                } else {
                    0.0
                };
                assert!(
                    (v as f64 - want).abs() <= 0.06 * peak,
                    "sigma {sigma}, offset {d}: {v} against the Gaussian's {want}"
                );
            }
            // Its tail reaches three sigma: the sample just inside it is lit.
            let edge = line[(n / 2) as usize + reach as usize];
            assert!(
                edge > 0.0,
                "sigma {sigma}: nothing at {reach} samples, the kernel stops short of 3 sigma"
            );
        }
    }
}
