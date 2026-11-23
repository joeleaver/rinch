//! `display: -webkit-box` / `-webkit-inline-box`, `-webkit-box-orient` and
//! `-webkit-line-clamp` (#1703).
//!
//! Stylo's servo build parses none of them: the two display values are
//! `#[cfg(feature = "gecko")]`, and the two properties are Gecko-only
//! longhands, so the declarations are dropped at parse and never reach the
//! cascade. rinch carries them through it as declarations Stylo keeps:
//! [`rewrite`] runs on every author sheet and every inline `style` (and
//! presentational hint) before Stylo parses it, and turns
//!
//! - `-webkit-line-clamp: N` into `--rinch-webkit-line-clamp: N`,
//! - `-webkit-box-orient: vertical` into `--rinch-webkit-box-orient: vertical`,
//! - `-webkit-box-flex: N` into `--rinch-webkit-box-flex: N` (#2581), and
//!   `-webkit-box-pack`, `-webkit-box-align` and `-webkit-box-direction`
//!   into `--rinch-webkit-box-pack` and so on,
//! - `display: -webkit-box` into `display: flow-root; --rinch-webkit-display:
//!   -webkit-box` (`inline-block` for `-webkit-inline-box`): Chrome computes
//!   a box that clamps `flow-root`, and blockifies no child of one that does
//!   not (a `<span>` in a vertical `-webkit-box` stays inline, Chrome 155),
//!   so Stylo is given a display that blockifies nothing,
//!
//! each with the declaration's own `!important`. The rewritten declarations
//! sit in the rule they were written in, so specificity, origin, order and
//! importance are the cascade's own. Custom properties inherit, and these
//! must not, so [`apply`] does not read their computed values: it walks
//! the element's own matched declarations in cascade order ([`own_winner`]),
//! the way Stylo's cascade iterates them, and takes the first. The display
//! marker only counts where it beats every `display` declaration.
//!
//! What Chrome 155 does, and so what [`apply`] makes of them:
//! - `-webkit-box` with `-webkit-box-orient: vertical` and a
//!   `-webkit-line-clamp` of N computes `display: flow-root`, and its lines
//!   stop at the Nth: the box is as tall as N lines, the Nth ends in "…",
//!   and the lines after it are still laid out and painted below (an
//!   `overflow: hidden` clips them). The lines are counted through the
//!   box's block children (#2581, `RinchDocument::resolve_block_line_clamps`);
//! - otherwise Chrome lays `-webkit-box` out as a flex container (a column
//!   under `vertical`) whose run of inline content is one anonymous item, so
//!   `text-overflow` draws no "…" (`ComputedStyle::webkit_box`). rinch keeps
//!   the block Stylo was given as its computed `display`, so that a run is
//!   one anonymous block box, and makes its Taffy style the flex container
//!   (`ComputedStyle::legacy_box_taffy_style`, #2581); its items grow and
//!   shrink by `-webkit-box-flex` alone (`--rinch-webkit-box-flex`), and it
//!   places them by `-webkit-box-pack`, `-webkit-box-align` and
//!   `-webkit-box-direction`, Chrome ignoring `justify-content`,
//!   `align-items`, `align-self` and `flex-direction` there;
//! - `-webkit-line-clamp` or `-webkit-box-orient` on a box that is not
//!   `-webkit-box` does nothing.
//!
//! `word-break: break-word` (#2628), the legacy spelling of `overflow-wrap:
//! anywhere` with `word-break: normal`, is Gecko-only too. Every `word-break`
//! declaration is mirrored into `--rinch-word-break` ([`WORD_BREAK`]) and
//! `break-word` itself handed to Stylo as `normal`; `ComputedStyle::from_stylo`
//! reads the custom property and makes the element's `overflow-wrap`
//! `anywhere` where it says `break-word`.
//!
//! So are `text-indent`'s `hanging` and `each-line` (#2628): Stylo's servo
//! build parses the length alone and drops a declaration with either. Every
//! `text-indent` is handed to Stylo as its length and mirrored into
//! `--rinch-text-indent` ([`TEXT_INDENT`]) as its keywords (`none` for
//! neither).
//!
//! `hyphens` (#2621) goes the same way: Gecko-only in Stylo's servo build,
//! it is rewritten into `--rinch-hyphens`, a custom property, which inherits
//! as `hyphens` does, so its computed value is read directly
//! ([`HYPHENS`], `ComputedStyle::hyphens_none`). `-webkit-hyphens` is not:
//! Chrome 157 ignores `-webkit-hyphens: none` (a soft hyphen still breaks).
//!
//! A `var()` in any of them is carried into the custom property as written
//! (#2820), and substituted there by the cascade: [`apply`] reads such a
//! declaration's substituted value ([`substituted`]), and a
//! `-webkit-text-stroke` shorthand holding one is split where it is read
//! ([`stroke_half`]). A `display` or `text-indent` holding a `var()` is
//! handed to Stylo as written and mirrored into its custom property
//! (#2870): [`apply`] makes a substituted `-webkit-box` the legacy box (Stylo
//! made the declaration `inline`), and `ComputedStyle::from_stylo` splits a
//! substituted `text-indent` ([`VAR_MARKER`]). A `var()` that leaves
//! `hyphens`, `word-break` or `text-indent` invalid inherits
//! (`ComputedStyle::inherit_invalid_text`).
//! Known small divergences: `getComputedStyle`-style readers see `flow-root`/
//! `inline-block`, not `-webkit-box`; in nested CSS, declarations before a
//! nested rule in the same block are not rewritten; a comment after the
//! value makes the declaration unrecognised (Stylo then drops it, as
//! before).

use std::borrow::Cow;

use crate::computed_style::{ComputedStyle, DisplayValue};
use crate::node::{Node, RawNodeId};

const DISPLAY: &str = "rinch-webkit-display";
const CLAMP: &str = "rinch-webkit-line-clamp";
const ORIENT: &str = "rinch-webkit-box-orient";
const FLEX: &str = "rinch-webkit-box-flex";
const PACK: &str = "rinch-webkit-box-pack";
const ALIGN: &str = "rinch-webkit-box-align";
const DIRECTION: &str = "rinch-webkit-box-direction";
/// The custom property `hyphens` is carried in (#2621), without its `--`.
pub(crate) const HYPHENS: &str = "rinch-hyphens";
/// The custom property every `word-break` declaration is mirrored into
/// (#2628), without its `--`: `break-word` is Gecko-only in Stylo's servo
/// build, so it reaches the cascade only here (see [`rewrite_list`]).
pub(crate) const WORD_BREAK: &str = "rinch-word-break";
/// The custom property the keywords of every `text-indent` declaration are
/// mirrored into (#2628), without its `--`: Stylo's servo build parses a
/// length or percentage alone, and drops `37px hanging` whole.
pub(crate) const TEXT_INDENT: &str = "rinch-text-indent";
/// The custom properties `-webkit-text-fill-color`, `-webkit-text-stroke-width`,
/// `-webkit-text-stroke-color` (the `-webkit-text-stroke` shorthand sets the
/// last two) and `paint-order` are carried in (#2719), without their `--`:
/// all four are Gecko-only in Stylo's servo build. They inherit, as custom
/// properties do, so their computed values are read directly
/// (`ComputedStyle::text_paint`).
pub(crate) const TEXT_FILL_COLOR: &str = "rinch-text-fill-color";
pub(crate) const TEXT_STROKE_WIDTH: &str = "rinch-text-stroke-width";
pub(crate) const TEXT_STROKE_COLOR: &str = "rinch-text-stroke-color";
pub(crate) const PAINT_ORDER: &str = "rinch-paint-order";
/// The custom property a `background-clip` (or `-webkit-background-clip`)
/// list holding `text` is carried in (#2719), without its `--`: Stylo's servo
/// build drops the whole declaration. Not inherited, so it is read from the
/// element's own declarations ([`apply`]), where any `background-clip`
/// longhand Stylo did keep (one without `text`, or the one the `background`
/// shorthand sets) beats it in cascade order.
pub(crate) const BACKGROUND_CLIP: &str = "rinch-background-clip";

/// Whether any style sheet or inline style has been rewritten to carry one of
/// the text-paint properties (#2719). Until one has, nothing reads them back:
/// a page that never writes them pays no custom-property look-up for them.
static TEXT_PAINT_SEEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether a declaration carrying a text-paint property has ever been
/// rewritten ([`TEXT_PAINT_SEEN`]).
pub(crate) fn text_paint_seen() -> bool {
    TEXT_PAINT_SEEN.load(std::sync::atomic::Ordering::Relaxed)
}

fn note_text_paint() {
    TEXT_PAINT_SEEN.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Whether any style sheet or inline style has carried a `var()` into
/// `hyphens`, `word-break` or `text-indent` (#2870). Until one has, no
/// element's value of them can be invalid at computed-value time, and
/// nothing looks for one (`ComputedStyle::inherit_invalid_text`).
static VAR_REWRITE_SEEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether a `var()` has ever been carried into `hyphens`, `word-break` or
/// `text-indent` ([`VAR_REWRITE_SEEN`]).
pub(crate) fn var_rewrite_seen() -> bool {
    VAR_REWRITE_SEEN.load(std::sync::atomic::Ordering::Relaxed)
}

fn note_var_rewrite() {
    VAR_REWRITE_SEEN.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// A `-webkit-text-stroke-width` in CSS px: a non-negative `px`, `em` (of
/// `font_size`), `rem` (of 16px), `pt`, `pc`, `in`, `cm`, `mm`, `q` length,
/// a bare `0`, `thin` / `medium` / `thick` (1, 3 and 5 px, as Chrome
/// computes them), or a `calc()` of lengths and numbers (#2870), clamped at
/// `0` as Chrome clamps it. `None` for anything else (a negative length, a
/// percentage).
pub(crate) fn parse_stroke_width(v: &str, font_size: f32) -> Option<f32> {
    let v = v.trim();
    let keyword = |k: &str| v.eq_ignore_ascii_case(k);
    if keyword("thin") {
        return Some(1.0);
    }
    if keyword("medium") {
        return Some(3.0);
    }
    if keyword("thick") {
        return Some(5.0);
    }
    if is_calc(v) {
        return match parse_length_percentage(v, font_size)? {
            (px, 0.0) => Some(px.max(0.0)),
            _ => None,
        };
    }
    parse_length_percentage(v, font_size)
        .filter(|&(px, pct)| pct == 0.0 && px >= 0.0)
        .map(|(px, _)| px)
}

fn is_calc(v: &str) -> bool {
    v.len() > 5 && v[..5].eq_ignore_ascii_case("calc(")
}

/// A length, a percentage or a `calc()` of them (and of numbers, `*` and
/// `/`), as `(px, fraction)`: the length in CSS px (`em` of `font_size`,
/// `rem` of 16px) plus a fraction of the basis. A bare `0` is a length.
/// `None` for anything else, a `calc()` that multiplies two lengths among it.
pub(crate) fn parse_length_percentage(v: &str, font_size: f32) -> Option<(f32, f32)> {
    let mut calc = Calc {
        s: v.trim().as_bytes(),
        i: 0,
        font_size,
    };
    let t = calc.term()?;
    calc.ws();
    (calc.i == calc.s.len() && !t.number).then_some((t.px, t.pct))
}

/// A `calc()` term: `px + pct × basis`, or a plain `number`.
#[derive(Clone, Copy)]
struct Term {
    px: f32,
    pct: f32,
    number: bool,
}

/// A recursive-descent reader of a length, a percentage or a `calc()`.
struct Calc<'a> {
    s: &'a [u8],
    i: usize,
    font_size: f32,
}

impl Calc<'_> {
    fn ws(&mut self) {
        while self.s.get(self.i).is_some_and(u8::is_ascii_whitespace) {
            self.i += 1;
        }
    }

    /// One value: a dimension, a percentage, a number, or a bracketed sum.
    fn term(&mut self) -> Option<Term> {
        self.ws();
        let rest = &self.s[self.i..];
        if rest.len() >= 5 && rest[..5].eq_ignore_ascii_case(b"calc(") {
            self.i += 5;
            return self.bracketed();
        }
        if rest.first() == Some(&b'(') {
            self.i += 1;
            return self.bracketed();
        }
        let start = self.i;
        let mut end = start;
        if matches!(self.s.get(end), Some(b'+' | b'-')) {
            end += 1;
        }
        while self
            .s
            .get(end)
            .is_some_and(|c| c.is_ascii_digit() || *c == b'.')
        {
            end += 1;
        }
        // An exponent (`1e2`), not a unit starting with `e` (`2em`).
        if matches!(self.s.get(end), Some(b'e' | b'E'))
            && self.s.get(end + 1).is_some_and(u8::is_ascii_digit)
        {
            end += 1;
            while self.s.get(end).is_some_and(u8::is_ascii_digit) {
                end += 1;
            }
        }
        let n: f32 = std::str::from_utf8(&self.s[start..end])
            .ok()?
            .parse()
            .ok()
            .filter(|n: &f32| n.is_finite())?;
        let unit_start = end;
        while self
            .s
            .get(end)
            .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'%')
        {
            end += 1;
        }
        self.i = end;
        let unit = std::str::from_utf8(&self.s[unit_start..end])
            .ok()?
            .to_ascii_lowercase();
        let length = |px: f32| Term {
            px,
            pct: 0.0,
            number: false,
        };
        Some(match unit.as_str() {
            "" => Term {
                px: n,
                pct: 0.0,
                // A bare `0` is a length.
                number: n != 0.0 || self.depth_is_calc(),
            },
            "%" => Term {
                px: 0.0,
                pct: n / 100.0,
                number: false,
            },
            "px" => length(n),
            "em" => length(n * self.font_size),
            "rem" => length(n * 16.0),
            "pt" => length(n * 96.0 / 72.0),
            "pc" => length(n * 16.0),
            "in" => length(n * 96.0),
            "cm" => length(n * 96.0 / 2.54),
            "mm" => length(n * 96.0 / 25.4),
            "q" => length(n * 96.0 / 101.6),
            _ => return None,
        })
    }

    /// Inside a `calc()` (or a bracket in one), where a bare `0` is a number.
    fn depth_is_calc(&self) -> bool {
        self.s[..self.i].contains(&b'(')
    }

    /// A sum up to its closing `)`.
    fn bracketed(&mut self) -> Option<Term> {
        let t = self.sum()?;
        self.ws();
        (self.s.get(self.i) == Some(&b')')).then(|| self.i += 1)?;
        Some(t)
    }

    /// Products joined by ` + ` and ` - ` (white space around both).
    fn sum(&mut self) -> Option<Term> {
        let mut acc = self.product()?;
        loop {
            let before = self.i;
            self.ws();
            let op = self.s.get(self.i).copied();
            if !matches!(op, Some(b'+' | b'-'))
                || self.i == before
                || !self.s.get(self.i + 1).is_some_and(u8::is_ascii_whitespace)
            {
                self.i = before;
                return Some(acc);
            }
            self.i += 1;
            let rhs = self.product()?;
            if rhs.number != acc.number {
                return None;
            }
            let sign = if op == Some(b'-') { -1.0 } else { 1.0 };
            acc.px += sign * rhs.px;
            acc.pct += sign * rhs.pct;
        }
    }

    /// Terms joined by `*` and `/`; one side of each is a number.
    fn product(&mut self) -> Option<Term> {
        let mut acc = self.term()?;
        loop {
            let before = self.i;
            self.ws();
            let op = self.s.get(self.i).copied();
            if !matches!(op, Some(b'*' | b'/')) {
                self.i = before;
                return Some(acc);
            }
            self.i += 1;
            let rhs = self.term()?;
            match op {
                Some(b'*') if rhs.number => {
                    acc.px *= rhs.px;
                    acc.pct *= rhs.px;
                }
                Some(b'*') if acc.number => {
                    let k = acc.px;
                    acc = Term {
                        px: rhs.px * k,
                        pct: rhs.pct * k,
                        number: false,
                    };
                }
                Some(b'/') if rhs.number && rhs.px != 0.0 => {
                    acc.px /= rhs.px;
                    acc.pct /= rhs.px;
                }
                _ => return None,
            }
        }
    }
}

fn valid_text_color(v: &str) -> bool {
    is_wide_keyword(v)
        || has_var(v)
        || crate::layout::parse_color_with_current(v, peniko::Color::BLACK).is_some()
}

fn valid_stroke_width(v: &str) -> bool {
    is_wide_keyword(v) || has_var(v) || parse_stroke_width(v, 16.0).is_some()
}

fn valid_paint_order(v: &str) -> bool {
    is_wide_keyword(v) || has_var(v) || crate::paint::parse_paint_order(v).is_some()
}

fn has_var(v: &str) -> bool {
    contains_ci(v, "var(")
}

/// What a `-webkit-text-stroke` shorthand holding a `var()` is carried
/// behind, in both `--rinch-text-stroke-width` and `--rinch-text-stroke-color`
/// (#2820): the cascade substitutes the `var()`s, and [`stroke_half`] splits
/// the result. No longhand value starts with it.
const STROKE_SHORTHAND: &str = "/";

/// What a `text-indent` holding a `var()` is carried behind in
/// `--rinch-text-indent` (#2870), as [`STROKE_SHORTHAND`] is for the stroke.
pub(crate) const VAR_MARKER: &str = STROKE_SHORTHAND;

/// The `-webkit-text-stroke-width` (`width`) or `-webkit-text-stroke-color`
/// half of a computed `--rinch-text-stroke-*` value: the value itself when a
/// longhand declared it, and the matching half of the substituted shorthand
/// when [`STROKE_SHORTHAND`] leads it (#2820). `None` when the substituted
/// shorthand is no `-webkit-text-stroke` value, which Chrome treats as
/// invalid at computed-value time, or a CSS-wide keyword that inherits.
///
/// A `var()` fallback can substitute a CSS-wide keyword (`var(--x,
/// initial)`), and Chrome then applies the keyword (#2870): `initial` is the
/// initial `0` and `currentcolor`; the rest inherit (both properties are
/// inherited, and an author `revert` has no user-agent value to revert to).
pub(crate) fn stroke_half(v: &str, width: bool) -> Option<Cow<'_, str>> {
    let Some(shorthand) = v.strip_prefix(STROKE_SHORTHAND) else {
        return Some(Cow::Borrowed(v));
    };
    let shorthand = shorthand.trim();
    if shorthand.eq_ignore_ascii_case("initial") {
        return Some(Cow::Borrowed(if width { "0" } else { "currentcolor" }));
    }
    if is_wide_keyword(shorthand) {
        return None;
    }
    let (w, c) = split_text_stroke(shorthand)?;
    Some(Cow::Owned(if width { w } else { c }))
}

/// The `-webkit-text-stroke` shorthand split into its width and colour, in
/// either order, each optional (an omitted one is its initial value, `0` and
/// `currentcolor`). `None` when it is not such a value. A CSS-wide keyword is
/// both halves.
fn split_text_stroke(v: &str) -> Option<(String, String)> {
    if is_wide_keyword(v) {
        return Some((v.to_string(), v.to_string()));
    }
    let words = top_level_words(v);
    let (mut width, mut color) = (None::<&str>, None::<&str>);
    for w in words {
        if width.is_none() && parse_stroke_width(w, 16.0).is_some() {
            width = Some(w);
        } else if color.is_none()
            && crate::layout::parse_color_with_current(w, peniko::Color::BLACK).is_some()
        {
            color = Some(w);
        } else {
            return None;
        }
    }
    if width.is_none() && color.is_none() {
        return None;
    }
    Some((
        width.unwrap_or("0").to_string(),
        color.unwrap_or("currentcolor").to_string(),
    ))
}

/// `v`'s top-level words: white space inside brackets (`rgb(0 0 0)`) does not
/// split.
fn top_level_words(v: &str) -> Vec<&str> {
    let (mut depth, mut start) = (0i32, None::<usize>);
    let mut words: Vec<&str> = Vec::new();
    for (i, c) in v.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if c.is_whitespace() && depth <= 0 {
            if let Some(b) = start.take() {
                words.push(&v[b..i]);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(b) = start {
        words.push(&v[b..]);
    }
    words
}

/// A `background-clip` list as `background_clip_text` holds it: one entry a
/// layer, `true` for `text`. `None` when no entry is `text` (Stylo keeps such
/// a declaration itself) or an entry is no `background-clip` value.
pub(crate) fn parse_background_clip_text(v: &str) -> Option<Vec<bool>> {
    let mut out = Vec::new();
    for entry in v.split(',') {
        let e = entry.trim();
        if e.eq_ignore_ascii_case("text") {
            out.push(true);
        } else if ["border-box", "padding-box", "content-box"]
            .iter()
            .any(|k| e.eq_ignore_ascii_case(k))
        {
            out.push(false);
        } else {
            return None;
        }
    }
    out.contains(&true).then_some(out)
}

/// Whether `css` holds anything [`rewrite_list`] rewrites.
fn wanted(css: &str) -> bool {
    contains_ci(css, "-webkit-")
        || contains_ci(css, "paint-order")
        || contains_ci(css, "background-clip")
        || contains_ci(css, "hyphens")
        || contains_ci(css, "word-break")
        || contains_ci(css, "text-indent")
        // `display: var(--d)` may name `-webkit-box` (#2870).
        || (contains_ci(css, "display") && has_var(css))
}

/// A `text-indent` value split into what Stylo's servo build parses (the
/// length or percentage) and its `hanging` and `each-line` keywords, as
/// `--rinch-text-indent` carries them (`none` for neither); `None` when it is
/// not a value Chrome takes (no length, or a keyword twice). A CSS-wide
/// keyword is both halves.
pub(crate) fn split_text_indent(v: &str) -> Option<(String, String)> {
    if is_wide_keyword(v) {
        return Some((v.to_string(), v.to_string()));
    }
    let mut length: Vec<&str> = Vec::new();
    let (mut hanging, mut each_line) = (false, false);
    // Top-level words: a `calc( … )` is one.
    let (mut depth, mut start) = (0i32, None::<usize>);
    let mut words: Vec<&str> = Vec::new();
    for (i, c) in v.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if c.is_whitespace() && depth <= 0 {
            if let Some(b) = start.take() {
                words.push(&v[b..i]);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(b) = start {
        words.push(&v[b..]);
    }
    for w in words {
        if w.eq_ignore_ascii_case("hanging") {
            if hanging {
                return None;
            }
            hanging = true;
        } else if w.eq_ignore_ascii_case("each-line") {
            if each_line {
                return None;
            }
            each_line = true;
        } else {
            length.push(w);
        }
    }
    if length.is_empty() {
        return None;
    }
    let keywords = match (hanging, each_line) {
        (false, false) => "none",
        (true, false) => "hanging",
        (false, true) => "each-line",
        (true, true) => "hanging each-line",
    };
    Some((length.join(" "), keywords.to_string()))
}

/// Whether `haystack` contains `needle` (ASCII lower case), ignoring ASCII
/// case.
fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

/// `value` without a trailing `!important`, and whether it had one.
fn split_important(value: &str) -> (&str, bool) {
    let t = value.trim_end();
    if let Some(bang) = t.rfind('!')
        && t[bang + 1..].trim().eq_ignore_ascii_case("important")
    {
        return (&t[..bang], true);
    }
    (value, false)
}

pub(crate) fn is_wide_keyword(v: &str) -> bool {
    ["inherit", "initial", "unset", "revert", "revert-layer"]
        .iter()
        .any(|k| v.eq_ignore_ascii_case(k))
}

/// A `-webkit-line-clamp` value as Chrome parses it: `none` or an integer
/// of at least 1.
fn valid_clamp(v: &str) -> bool {
    v.eq_ignore_ascii_case("none")
        || is_wide_keyword(v)
        || has_var(v)
        || v.strip_prefix('+')
            .unwrap_or(v)
            .parse::<u32>()
            .is_ok_and(|n| n >= 1)
}

/// A `-webkit-box-flex` value: a non-negative number.
fn valid_flex(v: &str) -> bool {
    is_wide_keyword(v) || has_var(v) || parse_flex(v).is_some()
}

fn parse_flex(v: &str) -> Option<f32> {
    v.parse::<f32>().ok().filter(|f| f.is_finite() && *f >= 0.0)
}

/// A `-webkit-box-pack` value: `start`, `end`, `center` or `justify`.
fn parse_pack(v: &str) -> Option<crate::computed_style::WebkitBoxPack> {
    use crate::computed_style::WebkitBoxPack as P;
    [
        ("start", P::Start),
        ("end", P::End),
        ("center", P::Center),
        ("justify", P::Justify),
    ]
    .into_iter()
    .find_map(|(k, p)| v.eq_ignore_ascii_case(k).then_some(p))
}

/// A `-webkit-box-align` value: `stretch`, `start`, `end`, `center` or
/// `baseline`.
fn parse_align(v: &str) -> Option<crate::computed_style::WebkitBoxAlign> {
    use crate::computed_style::WebkitBoxAlign as A;
    [
        ("stretch", A::Stretch),
        ("start", A::Start),
        ("end", A::End),
        ("center", A::Center),
        ("baseline", A::Baseline),
    ]
    .into_iter()
    .find_map(|(k, a)| v.eq_ignore_ascii_case(k).then_some(a))
}

/// A `-webkit-box-direction` value: `true` for `reverse`, `false` for
/// `normal`.
fn parse_direction(v: &str) -> Option<bool> {
    if v.eq_ignore_ascii_case("reverse") {
        Some(true)
    } else if v.eq_ignore_ascii_case("normal") {
        Some(false)
    } else {
        None
    }
}

fn valid_keyword<T>(v: &str, parse: impl Fn(&str) -> Option<T>) -> bool {
    is_wide_keyword(v) || has_var(v) || parse(v).is_some()
}

fn valid_hyphens(v: &str) -> bool {
    ["none", "manual", "auto"]
        .iter()
        .any(|k| v.eq_ignore_ascii_case(k))
        || is_wide_keyword(v)
        || has_var(v)
}

fn valid_word_break(v: &str) -> bool {
    ["normal", "break-all", "keep-all", "break-word"]
        .iter()
        .any(|k| v.eq_ignore_ascii_case(k))
        || is_wide_keyword(v)
        || has_var(v)
}

fn valid_orient(v: &str) -> bool {
    ["horizontal", "vertical", "inline-axis", "block-axis"]
        .iter()
        .any(|k| v.eq_ignore_ascii_case(k))
        || is_wide_keyword(v)
        || has_var(v)
}

/// Split a declaration list at its top-level `;`s (outside strings,
/// brackets and comments). The pieces keep their text; the `;`s are not in
/// them.
fn split_declarations(list: &str) -> Vec<&str> {
    let bytes = list.as_bytes();
    let mut out = Vec::new();
    let (mut start, mut depth, mut i) = (0, 0i32, 0);
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => {
                let q = bytes[i];
                i += 1;
                while i < bytes.len() && bytes[i] != q {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = list[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |e| i + 2 + e + 1);
            }
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b';' if depth <= 0 => {
                out.push(&list[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(&list[start.min(list.len())..]);
    out
}

/// Rewrite one declaration list (a rule's body or an inline `style`).
fn rewrite_list(list: &str, out: &mut String) {
    let decls = split_declarations(list);
    // `(lead, name, value)`: the lead is the text before the name (white
    // space, a comment), kept as it is.
    let parsed: Vec<Option<(&str, String, &str)>> = decls
        .iter()
        .map(|d| {
            let colon = d.find(':')?;
            let pre = &d[..colon];
            let after_comment = pre.rfind("*/").map_or(0, |e| e + 2);
            let name = pre[after_comment..].trim_start();
            let lead = &d[..colon - name.len()];
            Some((lead, name.trim_end().to_ascii_lowercase(), &d[colon + 1..]))
        })
        .collect();
    for (i, (decl, parsed)) in decls.iter().zip(&parsed).enumerate() {
        if i > 0 {
            out.push(';');
        }
        let Some((lead, name, value)) = parsed else {
            out.push_str(decl);
            continue;
        };
        let (v, important) = split_important(value);
        let v = v.trim();
        let bang = if important { " !important" } else { "" };
        match name.as_str() {
            "-webkit-line-clamp" if valid_clamp(v) => {
                out.push_str(&format!("{lead}--{CLAMP}: {v}{bang}"));
            }
            "hyphens" if valid_hyphens(v) => {
                if has_var(v) {
                    note_var_rewrite();
                }
                out.push_str(&format!("{lead}--{HYPHENS}: {v}{bang}"));
            }
            // Every `word-break` is mirrored into `--rinch-word-break`, in
            // the same rule with the same importance, so its computed value
            // is the winning declaration's (both inherit): an element's own
            // `normal` replaces an inherited `break-word`. Stylo is given
            // `normal` for `break-word`, its `word-break` half.
            "word-break" if valid_word_break(v) => {
                if has_var(v) {
                    note_var_rewrite();
                }
                let stylo = if v.eq_ignore_ascii_case("break-word") {
                    "normal"
                } else {
                    v
                };
                out.push_str(&format!(
                    "{lead}word-break: {stylo}{bang}; --{WORD_BREAK}: {v}{bang}"
                ));
            }
            // Every `text-indent` is mirrored into `--rinch-text-indent`
            // (its keywords, or `none`), so an element's own value replaces
            // an inherited `hanging`; Stylo is given the length alone.
            // A `var()` may substitute keywords Stylo's servo build drops the
            // declaration over (#2870): Stylo gets it as written, and the
            // mirror carries it behind [`VAR_MARKER`] to be split once the
            // cascade has substituted it (`ComputedStyle::from_stylo`).
            "text-indent" if has_var(v) => {
                note_var_rewrite();
                out.push_str(&format!("{decl}; --{TEXT_INDENT}: {VAR_MARKER} {v}{bang}"));
            }
            "text-indent" => match split_text_indent(v) {
                Some((length, keywords)) => out.push_str(&format!(
                    "{lead}text-indent: {length}{bang}; --{TEXT_INDENT}: {keywords}{bang}"
                )),
                None => out.push_str(decl),
            },
            // The text-paint properties (#2719): each into its custom
            // property, in the same rule with the same importance.
            "-webkit-text-fill-color" if valid_text_color(v) => {
                note_text_paint();
                out.push_str(&format!("{lead}--{TEXT_FILL_COLOR}: {v}{bang}"));
            }
            "-webkit-text-stroke-color" if valid_text_color(v) => {
                note_text_paint();
                out.push_str(&format!("{lead}--{TEXT_STROKE_COLOR}: {v}{bang}"));
            }
            "-webkit-text-stroke-width" if valid_stroke_width(v) => {
                note_text_paint();
                out.push_str(&format!("{lead}--{TEXT_STROKE_WIDTH}: {v}{bang}"));
            }
            // A shorthand with a `var()` in it cannot be split until the
            // cascade has substituted it (#2820): both custom properties
            // carry the whole value behind [`STROKE_SHORTHAND`], and each is
            // split where it is read ([`stroke_half`]).
            "-webkit-text-stroke" if has_var(v) => {
                note_text_paint();
                out.push_str(&format!(
                    "{lead}--{TEXT_STROKE_WIDTH}: {STROKE_SHORTHAND} {v}{bang}; --{TEXT_STROKE_COLOR}: {STROKE_SHORTHAND} {v}{bang}"
                ));
            }
            "-webkit-text-stroke" => match split_text_stroke(v) {
                Some((width, color)) => {
                    note_text_paint();
                    out.push_str(&format!(
                        "{lead}--{TEXT_STROKE_WIDTH}: {width}{bang}; --{TEXT_STROKE_COLOR}: {color}{bang}"
                    ));
                }
                None => out.push_str(decl),
            },
            "paint-order" if valid_paint_order(v) => {
                note_text_paint();
                out.push_str(&format!("{lead}--{PAINT_ORDER}: {v}{bang}"));
            }
            // A list holding `text`, which Stylo's servo build drops whole:
            // into `--rinch-background-clip`. Any other list is Stylo's own
            // `background-clip`, which [`apply`] reads as "not text".
            "background-clip" | "-webkit-background-clip"
                if parse_background_clip_text(v).is_some() =>
            {
                note_text_paint();
                out.push_str(&format!("{lead}--{BACKGROUND_CLIP}: {v}{bang}"));
            }
            // A list holding a `var()` (#2820) is Stylo's, and mirrored: the
            // marker's substituted value says whether it holds `text`
            // ([`apply`]); Stylo's own longhand draws any other list.
            "background-clip" | "-webkit-background-clip" if has_var(v) => {
                note_text_paint();
                out.push_str(&format!("{decl}; --{BACKGROUND_CLIP}: {v}{bang}"));
            }
            // A CSS-wide keyword is Stylo's too, and mirrored, so that
            // `inherit` under a text-clipped parent inherits `text`
            // ([`apply`] reads the keyword off the marker, which follows).
            "background-clip" | "-webkit-background-clip" if is_wide_keyword(v) => {
                out.push_str(&format!("{decl}; --{BACKGROUND_CLIP}: {v}{bang}"));
            }
            "-webkit-box-orient" if valid_orient(v) => {
                out.push_str(&format!("{lead}--{ORIENT}: {v}{bang}"));
            }
            "-webkit-box-flex" if valid_flex(v) => {
                out.push_str(&format!("{lead}--{FLEX}: {v}{bang}"));
            }
            "-webkit-box-pack" if valid_keyword(v, parse_pack) => {
                out.push_str(&format!("{lead}--{PACK}: {v}{bang}"));
            }
            "-webkit-box-align" if valid_keyword(v, parse_align) => {
                out.push_str(&format!("{lead}--{ALIGN}: {v}{bang}"));
            }
            "-webkit-box-direction" if valid_keyword(v, parse_direction) => {
                out.push_str(&format!("{lead}--{DIRECTION}: {v}{bang}"));
            }
            "display"
                if v.eq_ignore_ascii_case("-webkit-box")
                    || v.eq_ignore_ascii_case("-webkit-inline-box") =>
            {
                let (stylo, marker) = if v.eq_ignore_ascii_case("-webkit-inline-box") {
                    ("inline-block", "-webkit-inline-box")
                } else {
                    ("flow-root", "-webkit-box")
                };
                out.push_str(&format!(
                    "{lead}display: {stylo}{bang}; --{DISPLAY}: {marker}{bang}"
                ));
            }
            // A `var()` may substitute `-webkit-box` (#2870): Stylo gets the
            // declaration as written (and makes it `inline`, invalid at
            // computed-value time, when it does), and the marker carries
            // the `var()` for [`apply`] to read substituted.
            "display" if has_var(v) => {
                out.push_str(&format!("{decl}; --{DISPLAY}: {v}{bang}"));
            }
            _ => out.push_str(decl),
        }
    }
}

/// An `@supports` prelude with the spellings [`rewrite_list`] rewrites
/// rewritten the same way, so `@supports (-webkit-line-clamp: 2)` holds as
/// it does in Chrome; any other prelude as it is.
fn supports_prelude(prelude: &str) -> Cow<'_, str> {
    if !contains_ci(prelude, "@supports") || !contains_ci(prelude, "-webkit-") {
        return Cow::Borrowed(prelude);
    }
    Cow::Owned(
        // The property names first, held apart so the display value's
        // `-webkit-box` does not match inside them.
        prelude
            .replace("-webkit-line-clamp", "\u{0}C")
            .replace("-webkit-box-orient", "\u{0}O")
            .replace("-webkit-box-flex", "\u{0}F")
            .replace("-webkit-box-pack", "\u{0}P")
            .replace("-webkit-box-align", "\u{0}A")
            .replace("-webkit-box-direction", "\u{0}D")
            .replace("-webkit-inline-box", "inline-block")
            .replace("-webkit-box", "flow-root")
            .replace("\u{0}C", &format!("--{CLAMP}"))
            .replace("\u{0}O", &format!("--{ORIENT}"))
            .replace("\u{0}F", &format!("--{FLEX}"))
            .replace("\u{0}P", &format!("--{PACK}"))
            .replace("\u{0}A", &format!("--{ALIGN}"))
            .replace("\u{0}D", &format!("--{DIRECTION}")),
    )
}

/// `css` with the three Gecko-only spellings rewritten into declarations
/// Stylo's servo build keeps (see the module docs). `css` is a style sheet,
/// or with `inline` an inline `style` declaration list. Borrowed unchanged
/// when it has neither `-webkit-` nor `hyphens` in it.
pub(crate) fn rewrite(css: &str, inline: bool) -> Cow<'_, str> {
    if !wanted(css) {
        return Cow::Borrowed(css);
    }
    let mut out = String::with_capacity(css.len() + 64);
    if inline {
        rewrite_list(css, &mut out);
        return Cow::Owned(out);
    }
    // A sheet: each run of text up to a `}` (or the end) after the last
    // `{` is a declaration list; up to a `{`, a prelude, copied as it is.
    let bytes = css.as_bytes();
    let (mut start, mut i) = (0, 0);
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => {
                let q = bytes[i];
                i += 1;
                while i < bytes.len() && bytes[i] != q {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = css[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |e| i + 2 + e + 1);
            }
            b'{' => {
                out.push_str(&supports_prelude(&css[start..=i]));
                start = i + 1;
            }
            b'}' => {
                let list = &css[start..i];
                if wanted(list) {
                    rewrite_list(list, &mut out);
                } else {
                    out.push_str(list);
                }
                out.push('}');
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    let rest = &css[start.min(css.len())..];
    if wanted(rest) {
        rewrite_list(rest, &mut out);
    } else {
        out.push_str(rest);
    }
    Cow::Owned(out)
}

/// What wins the cascade on the element itself among the declarations
/// `pick` recognises: Stylo's own order (`DeclarationIterator` in its
/// `cascade.rs`) — the rule nodes from the most to the least important, and
/// in a block the later declaration first, each at its node's importance.
fn own_winner<T>(
    cv: &style::properties::ComputedValues,
    guard: &style::shared_lock::SharedRwLockReadGuard,
    mut pick: impl FnMut(&style::properties::PropertyDeclaration) -> Option<T>,
) -> Option<T> {
    for node in cv.rules().self_and_ancestors() {
        let Some(source) = node.style_source() else {
            continue;
        };
        let important = node.importance().important();
        for (decl, importance) in source.read(guard).declaration_importance_iter().rev() {
            if importance.important() != important {
                continue;
            }
            if let Some(t) = pick(decl) {
                return Some(t);
            }
        }
    }
    None
}

/// Whether an element with no computed value for the rewritten custom
/// property `name` takes its parent's value of the property it carries
/// (#2820): unless its own winning declaration is `initial` (or reverts),
/// the value is missing because a `var()` in it failed to substitute (or an
/// ancestor's did), which Chrome treats as `unset` and so, for these
/// inherited properties, `inherit`.
pub(crate) fn own_declaration_inherits(
    cv: &style::properties::ComputedValues,
    guard: &style::shared_lock::SharedRwLockReadGuard,
    name: &str,
) -> bool {
    use style::properties::CSSWideKeyword;
    let keyword = match own_winner(cv, guard, |d| custom(d, name)) {
        None => return true,
        // A `var()` whose fallback is a CSS-wide keyword and whose custom
        // property is missing substitutes the keyword, which Chrome applies
        // (#2870).
        Some(Declared::Value(v)) => match fallback_keyword(cv, &v) {
            Some(k) => k,
            None => return true,
        },
        Some(Declared::Keyword(k)) => k,
    };
    matches!(keyword, CSSWideKeyword::Inherit | CSSWideKeyword::Unset)
}

/// The CSS-wide keyword `v` substitutes when it is a `var()` (or a chain of
/// them, `var(--a, var(--b, initial))`) whose custom properties are missing
/// on the element and whose last fallback is the keyword (#2870).
fn fallback_keyword(
    cv: &style::properties::ComputedValues,
    v: &str,
) -> Option<style::properties::CSSWideKeyword> {
    use style::properties::CSSWideKeyword;
    let v = v.trim();
    let inner = v
        .get(..4)
        .filter(|p| p.eq_ignore_ascii_case("var("))
        .and_then(|_| v[4..].strip_suffix(')'))?;
    let (name, fallback) = inner.split_once(',')?;
    let name = name.trim().strip_prefix("--")?;
    let defined = cv
        .custom_properties()
        .get(
            style::properties_and_values::rule::Descriptors::unregistered(),
            &style::Atom::from(name),
        )
        .is_some();
    if defined {
        return None;
    }
    let fallback = fallback.trim();
    CSSWideKeyword::from_ident(fallback)
        .ok()
        .or_else(|| fallback_keyword(cv, fallback))
}

/// Whether the element itself declares the rewritten custom property
/// `name`.
pub(crate) fn own_declares(
    cv: &style::properties::ComputedValues,
    guard: &style::shared_lock::SharedRwLockReadGuard,
    name: &str,
) -> bool {
    own_winner(cv, guard, |d| custom(d, name)).is_some()
}

/// A rewritten custom property's declared text, or its CSS-wide keyword.
enum Declared {
    Value(String),
    Keyword(style::properties::CSSWideKeyword),
}

/// `declared`, with a value holding a `var()` replaced by the custom
/// property's computed value on the element (#2820): the cascade has
/// substituted it there, and since the element's own declaration won, that
/// value is this declaration's. A substitution that failed left the custom
/// property unset, which for these properties (none inherits) is `initial`.
fn substituted(cv: &style::properties::ComputedValues, name: &str, declared: Declared) -> Declared {
    match declared {
        Declared::Value(v) if has_var(&v) => cv
            .custom_properties()
            .get(
                style::properties_and_values::rule::Descriptors::unregistered(),
                &style::Atom::from(name),
            )
            .map_or(
                Declared::Keyword(style::properties::CSSWideKeyword::Initial),
                |v| Declared::Value(v.to_variable_value().css.trim().to_owned()),
            ),
        d => d,
    }
}

fn custom(decl: &style::properties::PropertyDeclaration, name: &str) -> Option<Declared> {
    use style::properties::{CustomDeclarationValue, PropertyDeclaration};
    let PropertyDeclaration::Custom(c) = decl else {
        return None;
    };
    if &*c.name != name {
        return None;
    }
    Some(match &c.value {
        CustomDeclarationValue::Unparsed(v) => Declared::Value(v.css.trim().to_owned()),
        CustomDeclarationValue::Parsed(_) => return None,
        CustomDeclarationValue::CSSWideKeyword(k) => Declared::Keyword(*k),
    })
}

thread_local! {
    static NAMES: [style::Atom; 8] =
        [DISPLAY, CLAMP, ORIENT, FLEX, PACK, ALIGN, DIRECTION, BACKGROUND_CLIP]
            .map(style::Atom::from);
}

/// Whether the element or an ancestor declares any of them: custom
/// properties inherit, so one declared on the element is in its computed
/// map. Most elements have none, and [`apply`] is skipped for them.
pub(crate) fn may_declare(cv: &style::properties::ComputedValues) -> bool {
    let props = cv.custom_properties();
    if props.inherited.is_empty() && props.non_inherited.is_empty() {
        return false;
    }
    // `--rinch-background-clip` (the last name) is looked for only once a
    // sheet has carried it (#2719): a page that never writes it pays nothing.
    let names = if text_paint_seen() { 8 } else { 7 };
    NAMES.with(|all| {
        all[..names].iter().any(|name| {
            props
                .get(
                    style::properties_and_values::rule::Descriptors::unregistered(),
                    name,
                )
                .is_some()
        })
    })
}

/// Resolve the three properties on `style`, just converted from `cv`;
/// `parent` is the parent element's style (for `inherit`).
pub(crate) fn apply(
    cv: &style::properties::ComputedValues,
    guard: &style::shared_lock::SharedRwLockReadGuard,
    parent: Option<&ComputedStyle>,
    style: &mut ComputedStyle,
) {
    use style::properties::CSSWideKeyword;
    let inherit = |k: &CSSWideKeyword| matches!(k, CSSWideKeyword::Inherit);
    // `background-clip: text` (#2719): the marker where it beats every
    // `background-clip` longhand Stylo kept, the `background` shorthand's
    // included.
    style.background_clip_text = own_winner(cv, guard, |d| {
        if matches!(
            d.id(),
            style::properties::PropertyDeclarationId::Longhand(
                style::properties::LonghandId::BackgroundClip
            )
        ) {
            return Some(Vec::new());
        }
        custom(d, BACKGROUND_CLIP).map(|d| match substituted(cv, BACKGROUND_CLIP, d) {
            Declared::Value(v) => parse_background_clip_text(&v).unwrap_or_default(),
            Declared::Keyword(k) if inherit(&k) => parent
                .map(|p| p.background_clip_text.clone())
                .unwrap_or_default(),
            Declared::Keyword(_) => Vec::new(),
        })
    })
    .unwrap_or_default();
    let clamp =
        own_winner(cv, guard, |d| custom(d, CLAMP)).and_then(|d| match substituted(cv, CLAMP, d) {
            Declared::Value(v) => v.strip_prefix('+').unwrap_or(&v).parse::<u32>().ok(),
            Declared::Keyword(k) if inherit(&k) => parent.and_then(|p| p.webkit_line_clamp),
            Declared::Keyword(_) => None,
        });
    let vertical =
        own_winner(cv, guard, |d| custom(d, ORIENT)).is_some_and(|d| {
            match substituted(cv, ORIENT, d) {
                Declared::Value(v) => {
                    v.eq_ignore_ascii_case("vertical") || v.eq_ignore_ascii_case("block-axis")
                }
                Declared::Keyword(k) => {
                    inherit(&k) && parent.is_some_and(|p| p.webkit_box_vertical)
                }
            }
        });
    style.webkit_line_clamp = clamp;
    style.webkit_box_vertical = vertical;
    style.line_clamp = None;
    style.webkit_box_flex = own_winner(cv, guard, |d| custom(d, FLEX))
        .and_then(|d| match substituted(cv, FLEX, d) {
            Declared::Value(v) => parse_flex(&v),
            Declared::Keyword(k) if inherit(&k) => parent.map(|p| p.webkit_box_flex),
            Declared::Keyword(_) => None,
        })
        .unwrap_or(0.0);
    // `-webkit-box-pack`, `-webkit-box-align`, `-webkit-box-direction`
    // (#2581): read where the box is a legacy flex one.
    style.webkit_box_pack = own_winner(cv, guard, |d| custom(d, PACK))
        .and_then(|d| match substituted(cv, PACK, d) {
            Declared::Value(v) => parse_pack(&v),
            Declared::Keyword(k) if inherit(&k) => parent.map(|p| p.webkit_box_pack),
            Declared::Keyword(_) => None,
        })
        .unwrap_or_default();
    style.webkit_box_align = own_winner(cv, guard, |d| custom(d, ALIGN))
        .and_then(|d| match substituted(cv, ALIGN, d) {
            Declared::Value(v) => parse_align(&v),
            Declared::Keyword(k) if inherit(&k) => parent.map(|p| p.webkit_box_align),
            Declared::Keyword(_) => None,
        })
        .unwrap_or_default();
    style.webkit_box_reverse = own_winner(cv, guard, |d| custom(d, DIRECTION))
        .and_then(|d| match substituted(cv, DIRECTION, d) {
            Declared::Value(v) => parse_direction(&v),
            Declared::Keyword(k) if inherit(&k) => parent.map(|p| p.webkit_box_reverse),
            Declared::Keyword(_) => None,
        })
        .unwrap_or(false);
    // An item of a legacy box laid out as a flex container (#2581).
    style.webkit_box_item = parent
        .filter(|p| p.webkit_box && p.line_clamp.is_none())
        .map(|p| p.webkit_box_vertical);
    // The marker counts only where it beats every `display` declaration.
    let marker = own_winner(cv, guard, |d| {
        if matches!(
            d.id(),
            style::properties::PropertyDeclarationId::Longhand(
                style::properties::LonghandId::Display
            )
        ) {
            return Some(None);
        }
        // A `var()` (#2870) counts where it substitutes a legacy box.
        custom(d, DISPLAY).map(|d| {
            let via_var = matches!(&d, Declared::Value(v) if has_var(v));
            match substituted(cv, DISPLAY, d) {
                Declared::Value(v) if v.eq_ignore_ascii_case("-webkit-inline-box") => {
                    Some((true, via_var))
                }
                Declared::Value(v) if v.eq_ignore_ascii_case("-webkit-box") => {
                    Some((false, via_var))
                }
                _ => None,
            }
        })
    })
    .flatten();
    let Some((inline, via_var)) = marker else {
        return;
    };
    // Stylo cannot parse a substituted `-webkit-box`, and made the
    // declaration invalid at computed-value time: `inline`, or the `block`
    // it blockifies to. The box is what the rewrite gives a written-out one,
    // `flow-root` or `inline-block` (or `flow-root` blockified).
    if via_var {
        match style.display {
            DisplayValue::Inline => {
                style.display = if inline {
                    DisplayValue::InlineBlock
                } else {
                    DisplayValue::Block
                };
                style.flow_root = !inline;
                style.inline_level_before_blockify = inline;
            }
            DisplayValue::Block => style.flow_root = true,
            _ => {}
        }
    }
    // `display: none` and the like were decided by Stylo; only a box Stylo
    // made of the rewrite is changed.
    if !matches!(
        style.display,
        DisplayValue::Flex
            | DisplayValue::InlineFlex
            | DisplayValue::Block
            | DisplayValue::InlineBlock
    ) {
        return;
    }
    style.webkit_box = true;
    // Chrome lays a legacy box that does not clamp out with the flex
    // algorithm, a run of inline content as one anonymous item. Its computed
    // `display` stays the block Stylo was given: as a flex container here
    // each inline child was an item of its own (`Hello <b>world</b> and more`
    // in a vertical one: 72 tall, Chrome 24), where a block container makes
    // a run one anonymous block box. Its Taffy style is the flex container
    // (`ComputedStyle::legacy_box_taffy_style`, #2581), and its text draws no
    // `text-overflow` "…", as an anonymous flex item's does not
    // (`ComputedStyle::webkit_box`).
    if let Some(n) = clamp.filter(|_| vertical) {
        style.display = if inline {
            DisplayValue::InlineBlock
        } else {
            DisplayValue::Block
        };
        style.line_clamp = Some(n);
    }
}

/// Where a block line clamp ends (#2581): the IFC root holding its last
/// line, which of the root's lines that is, whether anything follows it, and
/// the boxes from the clamp down to the root's parent.
struct ClampEnd {
    root: RawNodeId,
    line: usize,
    more: bool,
    path: Vec<RawNodeId>,
}

/// Whether `node` lays its lines out in the block formatting context of its
/// parent, so a clamp around it counts them: an in-flow block-level box
/// that starts no context of its own (Chrome 155: a `flow-root` child's
/// lines are not counted, an `inline-block`'s line is one line of its
/// parent's).
fn in_clamp_flow(node: &Node) -> bool {
    use crate::computed_style::OverflowValue;
    let cs = &node.computed_style;
    if node.is_anonymous_block_box {
        return true;
    }
    node.is_element()
        && matches!(cs.display, DisplayValue::Block | DisplayValue::ListItem)
        && !cs.flow_root
        && cs.table_part == crate::computed_style::TablePart::None
        && !node.is_out_of_flow()
        && !cs.float.is_floated()
        && matches!(cs.overflow_x, OverflowValue::Visible)
        && matches!(cs.overflow_y, OverflowValue::Visible)
}

/// The lines of an IFC root a clamp counts: none for a run of nothing but
/// collapsed white space between blocks, which has no line box (its layout
/// holds one empty line), however it is written.
fn counted_lines(il: &crate::node::InlineLayout) -> usize {
    if il.text_content.is_empty() && il.atomic_inlines().next().is_none() {
        0
    } else {
        il.layout.len()
    }
}

/// The border-box height of a box whose content box would be `content` tall,
/// held to its `min-height` and `max-height` (the minimum winning, as in
/// CSS); `insets` is its padding and border, top and bottom together. Only a
/// length bound is applied: a percentage one (or a `calc()` holding one)
/// counts as none here, though Taffy, which lays the box out, applies it.
fn min_max_height(cs: &ComputedStyle, content: f32, insets: f32) -> f32 {
    use crate::computed_style::DimensionValue as D;
    let length = |d: &D| match *d {
        D::Length(px) => Some(px),
        D::Calc { px, pct: 0.0 } => Some(px.max(0.0)),
        _ => None,
    };
    // The bounds as border-box heights.
    let border = |px: f32| {
        if cs.box_sizing_border_box {
            px.max(insets)
        } else {
            px + insets
        }
    };
    let mut height = content + insets;
    if let Some(max) = length(&cs.max_height) {
        height = height.min(border(max));
    }
    if let Some(min) = length(&cs.min_height) {
        height = height.max(border(min));
    }
    height
}

/// Whether `node`, a box-tree child, is in-flow content: anything that
/// follows a clamp's last line and so ends it in "…".
fn is_in_flow_box(node: &Node) -> bool {
    if node.is_anonymous_block_box {
        return node
            .text_layout
            .as_ref()
            .is_some_and(|il| counted_lines(il) > 0);
    }
    node.is_element()
        && !matches!(
            node.computed_style.display,
            DisplayValue::None | DisplayValue::Contents
        )
        && !node.is_out_of_flow()
        && !node.computed_style.float.is_floated()
}

/// Count the lines of `id`'s block children, in order, until the `n`th:
/// `seen` is how many came before. `path` holds the boxes from the clamp
/// down to `id`.
fn find_clamp_end(
    nodes: &crate::node_slab::NodeSlab,
    id: RawNodeId,
    n: usize,
    seen: &mut usize,
    path: &mut Vec<RawNodeId>,
) -> Option<ClampEnd> {
    path.push(id);
    let children = crate::RinchDocument::box_tree_children(nodes, id);
    for (i, &c) in children.iter().enumerate() {
        let Some(child) = nodes.get_live(c) else {
            continue;
        };
        if !in_clamp_flow(child) {
            continue;
        }
        let found = if let Some(il) = child.text_layout.as_ref() {
            let lines = counted_lines(il);
            if *seen + lines >= n {
                Some(ClampEnd {
                    root: c,
                    line: n - *seen,
                    more: lines > n - *seen,
                    path: path.clone(),
                })
            } else {
                *seen += lines;
                None
            }
        } else {
            find_clamp_end(nodes, c, n, seen, path)
        };
        if let Some(mut end) = found {
            // Anything in flow after the line, at any level, ends it in "…"
            // (Chrome 155: even an empty block of no height).
            end.more = end.more
                || children[i + 1..]
                    .iter()
                    .any(|&s| nodes.get_live(s).is_some_and(is_in_flow_box));
            return Some(end);
        }
    }
    path.pop();
    None
}

impl crate::RinchDocument {
    /// Resolve every `-webkit-line-clamp` whose lines are in its block
    /// children (#2581), from the lines just built; `true` when a box's
    /// clamp changed and the layout is owed another compute.
    ///
    /// Chrome 155 counts the lines of the clamp's block formatting context in
    /// order, through nested blocks (not into a box that starts a context of
    /// its own, `in_clamp_flow`). The IFC root holding the Nth line ends
    /// after it (`Node::clamp_lines`, which `InlineLayout::line_clamp`
    /// carries), with the "…" on it when anything follows; every box from
    /// the clamp down to it ends right after the box it holds, its own
    /// padding and border after that and that box's margin dropped
    /// (`Node::clamp_height`). What follows is laid out as it is, below.
    pub(crate) fn resolve_block_line_clamps(&mut self) -> bool {
        let tree = &mut self.tree;
        if tree.line_clamp_roots.is_empty() && tree.line_clamped.is_empty() {
            return false;
        }
        let nodes = &tree.nodes;
        // Both lists are kept across layouts and name nodes that can have
        // been freed since — and their slots handed to new nodes — so they are
        // read through the generation-checked lookups (#2748): a stale id
        // names nothing and drops out.
        tree.line_clamp_roots.retain(|&id| {
            nodes
                .get(id)
                .is_some_and(|n| n.computed_style.line_clamp.is_some())
        });
        let mut lines: Vec<(RawNodeId, (usize, bool))> = Vec::new();
        let mut heights: Vec<(RawNodeId, (f32, f32))> = Vec::new();
        for &clamp in &tree.line_clamp_roots {
            let node = &nodes[clamp];
            // One whose lines are its own is clamped by its own layout.
            if node.text_layout.is_some() {
                continue;
            }
            let n = node.computed_style.line_clamp.unwrap_or(0) as usize;
            if n == 0 {
                continue;
            }
            let Some(end) = find_clamp_end(nodes, clamp, n, &mut 0, &mut Vec::new()) else {
                continue;
            };
            lines.push((end.root, (end.line, end.more)));
            let insets = |id: RawNodeId| {
                nodes[id]
                    .taffy_id
                    .and_then(|t| tree.taffy.layout(t).ok())
                    .map_or((0.0, 0.0), |l| {
                        (
                            l.padding.top + l.border.top,
                            l.padding.bottom + l.border.bottom,
                        )
                    })
            };
            let declared = |id: RawNodeId| {
                !matches!(
                    nodes[id].computed_style.height,
                    crate::computed_style::DimensionValue::Auto
                )
            };
            // The root's border box, ending after its last kept line.
            let root = &nodes[end.root];
            let mut bottom = if declared(end.root) {
                root.layout.y + root.layout.height
            } else {
                let il = root.text_layout.as_ref().expect("an IFC root");
                let last = il
                    .layout
                    .get(end.line - 1)
                    .map_or(il.layout.height(), |l| l.metrics().block_max_coord);
                let (top, bottom) = insets(end.root);
                root.layout.y + top + last + il.float_drop + bottom
            };
            for &id in end.path.iter().rev() {
                let (top, end_inset) = insets(id);
                let content = (bottom - top).max(0.0);
                let border_box = bottom + end_inset;
                let height = if declared(id) {
                    nodes[id].layout.height
                } else {
                    heights.push((id, (border_box, content)));
                    // Taffy holds the box to its `min-height` and
                    // `max-height`, so the box around it ends where this one
                    // does after that (#2581: a `max-height: 30px` wrapper
                    // around the clamped line).
                    min_max_height(&nodes[id].computed_style, content, top + end_inset)
                };
                bottom = nodes[id].layout.y + height;
            }
        }

        // Take back what no clamp gives now, and give what changed.
        let mut changed_roots: Vec<RawNodeId> = Vec::new();
        let mut changed_boxes: Vec<RawNodeId> = Vec::new();
        let old = std::mem::take(&mut tree.line_clamped);
        for id in old {
            let Some(node) = tree.nodes.get_mut(id) else {
                continue;
            };
            if node.clamp_lines.is_some() && !lines.iter().any(|&(r, _)| r == id) {
                node.clamp_lines = None;
                changed_roots.push(id);
            }
            if node.clamp_height.is_some() && !heights.iter().any(|&(b, _)| b == id) {
                node.clamp_height = None;
                changed_boxes.push(id);
            }
        }
        for &(id, value) in &lines {
            let node = &mut tree.nodes[id];
            if node.clamp_lines != Some(value) {
                node.clamp_lines = Some(value);
                changed_roots.push(id);
            }
            tree.line_clamped.push(id);
        }
        for &(id, (border_box, content)) in &heights {
            let node = &mut tree.nodes[id];
            let same = node.clamp_height.is_some_and(|(b, c)| {
                (b - border_box).abs() < 0.001 && (c - content).abs() < 0.001
            });
            if !same {
                node.clamp_height = Some((border_box, content));
                changed_boxes.push(id);
            }
            tree.line_clamped.push(id);
        }
        let changed = !changed_roots.is_empty() || !changed_boxes.is_empty();
        for id in changed_roots {
            self.invalidate_ifc_root(id);
            self.mark_atomic_inline_dirty(id);
        }
        for id in changed_boxes {
            self.rewrite_taffy_style(id);
            self.mark_atomic_inline_dirty(id);
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::rewrite;

    #[test]
    fn a_sheet_without_webkit_is_borrowed() {
        assert!(matches!(
            rewrite("a { display: flex }", false),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    #[test]
    fn box_flex_becomes_a_custom_property_and_a_negative_one_is_left() {
        assert_eq!(
            rewrite("-webkit-box-flex: 1.5; -webkit-box-flex: -1", true),
            "--rinch-webkit-box-flex: 1.5; -webkit-box-flex: -1"
        );
    }

    #[test]
    fn pack_align_and_direction_become_custom_properties_issue_2581() {
        assert_eq!(
            rewrite(
                "-webkit-box-pack: Center; -webkit-box-align: baseline !important; \
                 -webkit-box-direction: reverse; -webkit-box-pack: left",
                true
            ),
            "--rinch-webkit-box-pack: Center; --rinch-webkit-box-align: baseline !important; \
             --rinch-webkit-box-direction: reverse; -webkit-box-pack: left"
        );
        assert_eq!(
            super::supports_prelude("@supports (-webkit-box-pack: end) {"),
            "@supports (--rinch-webkit-box-pack: end) {"
        );
    }

    #[test]
    fn the_clamp_pattern_becomes_flow_root_and_custom_properties() {
        let got = rewrite(
            ".c { display: -webkit-box; -webkit-box-orient: vertical; -WEBKIT-line-clamp: 2 !important }",
            false,
        );
        assert_eq!(
            got,
            ".c { display: flow-root; --rinch-webkit-display: -webkit-box; \
             --rinch-webkit-box-orient: vertical; --rinch-webkit-line-clamp: 2 !important}"
        );
    }

    #[test]
    fn an_inline_box_becomes_inline_block_and_keeps_its_importance() {
        assert_eq!(
            rewrite("display:-webkit-inline-box !important", true),
            "display: inline-block !important; --rinch-webkit-display: -webkit-inline-box !important"
        );
    }

    #[test]
    fn an_invalid_clamp_is_left_for_stylo_to_drop() {
        assert_eq!(
            rewrite("a{-webkit-line-clamp: 0}b{-webkit-line-clamp: 1.5}", false),
            "a{-webkit-line-clamp: 0}b{-webkit-line-clamp: 1.5}"
        );
    }

    #[test]
    fn preludes_strings_and_comments_are_copied() {
        let css = "@media (min-width: 1px) { .a[title=\"x;y{\"] { /* display: -webkit-box */ \
                   content: \"display: -webkit-box\"; color: red } } .b{display:-webkit-box}";
        assert_eq!(
            rewrite(css, false),
            "@media (min-width: 1px) { .a[title=\"x;y{\"] { /* display: -webkit-box */ \
             content: \"display: -webkit-box\"; color: red } } .b{display: flow-root; --rinch-webkit-display: -webkit-box}"
        );
    }
}
