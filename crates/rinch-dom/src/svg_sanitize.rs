//! An SVG file, rebuilt within rinch's profile (#2111, the fourth review of
//! #2125): what usvg is given is not the file but a new document this module
//! writes from roxmltree's reading of it.
//!
//! Four rounds of review broke a checker that read the original document
//! while usvg read the same text its own way: every place the two readings
//! differed (an `<image>`'s children, which `url()` a duplicate id names,
//! how path data is tokenised) was a way past the checks. So the checks no
//! longer read the original. [`sanitize`] builds a new document holding only
//! the profile — the elements and attributes it allows, each value written
//! back from its parsed form — with every id unique and every reference
//! resolved by this module, and counts what usvg will expand on that
//! document. usvg then reads the text written here and nothing else: the
//! checker and the renderer read one document.
//!
//! - **Elements** outside the profile that a renderer draws refuse the file
//!   (see `svg_image`'s module doc for the list). Ignored ones (`text`,
//!   `image`, `foreignObject`, `title`, `desc`, `metadata`, `style`, other
//!   namespaces and unknown names) are left out of the output, children and
//!   all.
//! - **Ids**: the first emitted element with an id keeps it; a later one
//!   loses it. Ids are renamed (`i<n>`), and only referenced elements get
//!   one.
//! - **References** (`href`, and `url()` in `fill`, `stroke`, `clip-path`,
//!   `mask`, `filter` and the markers, from attributes, `style=""` and
//!   `<style>`) are resolved against the emitted elements and kept only when
//!   they name one of the right kind; otherwise they are dropped (a paint
//!   falls back to its fallback, or `none`).
//! - **CSS** is inlined: each `<style>` rule (within the profile's selector
//!   forms) is matched against the original tree, its declarations applied
//!   by specificity and order between the presentation attributes and the
//!   element's own `style=""`, and written out as attributes. No `<style>`
//!   reaches usvg, so its CSS matching never runs.
//! - **Values** are parsed and re-emitted: path data through svgtypes'
//!   simplifying parser (absolute `M`/`L`/`C`/`Q`/`Z`, which also gives the
//!   real vertex count), points, transforms (as `matrix`), lengths, colours,
//!   paints, number lists; a keyword only from a safe character set. A value
//!   that does not parse is dropped.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use resvg::usvg::roxmltree::{Document, Node};

use crate::svg_image::{
    MAX_DEPTH, MAX_FILTER_PRIMITIVES, MAX_GRADIENT_STOPS, MAX_MERGE_NODES, MAX_NODES, MAX_SELECTORS,
};

/// The SVG namespace; an element in no namespace counts as SVG, as usvg
/// reads it.
const SVG_NS: &str = "http://www.w3.org/2000/svg";

/// Elements written to the output (`a` as a `g`).
const KEPT: &[&str] = &[
    "svg",
    "g",
    "defs",
    "symbol",
    "use",
    "a",
    "switch",
    "path",
    "rect",
    "circle",
    "ellipse",
    "line",
    "polyline",
    "polygon",
    "linearGradient",
    "radialGradient",
    "stop",
    "clipPath",
    "mask",
    "pattern",
    "marker",
    "filter",
];

/// Filter primitives cheap per pixel (a drop shadow's parts).
const FILTERS: &[&str] = &[
    "feGaussianBlur",
    "feOffset",
    "feFlood",
    "feColorMatrix",
    "feComposite",
    "feBlend",
    "feDropShadow",
    "feMerge",
];

/// Every element usvg reads (its `EId`). Any other name it skips, subtree and
/// all; a name here that the profile does not keep or ignore refuses the
/// file.
const USVG_ELEMENTS: &[&str] = &[
    "a",
    "circle",
    "clipPath",
    "defs",
    "ellipse",
    "feBlend",
    "feColorMatrix",
    "feComponentTransfer",
    "feComposite",
    "feConvolveMatrix",
    "feDiffuseLighting",
    "feDisplacementMap",
    "feDistantLight",
    "feDropShadow",
    "feFlood",
    "feFuncA",
    "feFuncB",
    "feFuncG",
    "feFuncR",
    "feGaussianBlur",
    "feImage",
    "feMerge",
    "feMergeNode",
    "feMorphology",
    "feOffset",
    "fePointLight",
    "feSpecularLighting",
    "feSpotLight",
    "feTile",
    "feTurbulence",
    "filter",
    "g",
    "image",
    "line",
    "linearGradient",
    "marker",
    "mask",
    "path",
    "pattern",
    "polygon",
    "polyline",
    "radialGradient",
    "rect",
    "stop",
    "style",
    "svg",
    "switch",
    "symbol",
    "text",
    "textPath",
    "tref",
    "tspan",
    "use",
];

/// Left out of the output with everything inside them.
const DROPPED: &[&str] = &[
    "text",
    "image",
    "foreignObject",
    "style",
    "title",
    "desc",
    "metadata",
];

/// Elements that may be used only one level deep.
const ONE_LEVEL: &[&str] = &["clipPath", "mask", "pattern", "marker"];

/// The most steps CSS selector matching may take (an element, a rule, an
/// ancestor or sibling looked at).
const MAX_MATCH_STEPS: u64 = 20_000_000;

fn refuse<T>(why: impl std::fmt::Display) -> Result<T, String> {
    Err(format!("SVG refused: outside the supported profile: {why}"))
}

/// What a reference may name.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Want {
    PaintServer,
    Gradient,
    ClipPath,
    Mask,
    Filter,
    Marker,
    Drawable,
}

impl Want {
    fn accepts(self, name: &str) -> bool {
        match self {
            Want::PaintServer => {
                matches!(name, "linearGradient" | "radialGradient" | "pattern")
            }
            Want::Gradient => matches!(name, "linearGradient" | "radialGradient"),
            Want::ClipPath => name == "clipPath",
            Want::Mask => name == "mask",
            Want::Filter => name == "filter",
            Want::Marker => name == "marker",
            Want::Drawable => matches!(
                name,
                "g" | "a"
                    | "svg"
                    | "symbol"
                    | "use"
                    | "switch"
                    | "path"
                    | "rect"
                    | "circle"
                    | "ellipse"
                    | "line"
                    | "polyline"
                    | "polygon"
            ),
        }
    }
}

/// How a property's or attribute's value is parsed and written back.
#[derive(Clone, Copy)]
enum Kind {
    Paint,
    Ref(Want),
    Color,
    Number,
    NumberList,
    Length,
    LengthList,
    Keyword,
    Path,
    Points,
    Transform,
    Name,
}

/// Properties: from attributes, `style=""` and `<style>` alike.
fn property_kind(name: &str) -> Option<Kind> {
    Some(match name {
        "fill" | "stroke" => Kind::Paint,
        "clip-path" => Kind::Ref(Want::ClipPath),
        "mask" => Kind::Ref(Want::Mask),
        "filter" => Kind::Ref(Want::Filter),
        "marker-start" | "marker-mid" | "marker-end" => Kind::Ref(Want::Marker),
        "stop-color" | "flood-color" | "color" => Kind::Color,
        "opacity" | "fill-opacity" | "stroke-opacity" | "stop-opacity" | "flood-opacity"
        | "stroke-miterlimit" => Kind::Number,
        "stroke-width" | "stroke-dashoffset" => Kind::Length,
        "stroke-dasharray" => Kind::LengthList,
        "fill-rule"
        | "clip-rule"
        | "stroke-linecap"
        | "stroke-linejoin"
        | "display"
        | "visibility"
        | "color-interpolation-filters"
        | "color-interpolation"
        | "shape-rendering"
        | "paint-order"
        | "mix-blend-mode"
        | "isolation"
        | "mask-type"
        | "overflow"
        | "vector-effect" => Kind::Keyword,
        _ => return None,
    })
}

/// Attributes that are not properties.
fn attribute_kind(name: &str) -> Option<Kind> {
    Some(match name {
        "x" | "y" | "width" | "height" | "rx" | "ry" | "cx" | "cy" | "r" | "x1" | "y1" | "x2"
        | "y2" | "fx" | "fy" | "fr" | "refX" | "refY" | "markerWidth" | "markerHeight" => {
            Kind::Length
        }
        "viewBox" | "stdDeviation" | "values" | "k1" | "k2" | "k3" | "k4" | "dx" | "dy" => {
            Kind::NumberList
        }
        "offset" => Kind::Number,
        "preserveAspectRatio"
        | "gradientUnits"
        | "spreadMethod"
        | "patternUnits"
        | "patternContentUnits"
        | "clipPathUnits"
        | "maskUnits"
        | "maskContentUnits"
        | "markerUnits"
        | "orient"
        | "filterUnits"
        | "primitiveUnits"
        | "type"
        | "operator"
        | "mode"
        | "edgeMode" => Kind::Keyword,
        "in" | "in2" | "result" => Kind::Name,
        "d" => Kind::Path,
        "points" => Kind::Points,
        "transform" | "gradientTransform" | "patternTransform" => Kind::Transform,
        _ => return None,
    })
}

/// An element of the output.
struct Out<'a, 'i> {
    node: Node<'a, 'i>,
    name: &'static str,
    children: Vec<usize>,
    /// Attributes written as they are, value already re-emitted.
    attrs: Vec<(&'static str, String)>,
    /// References to resolve: (attribute, kind wanted, id, paint fallback).
    refs: Vec<(&'static str, Want, String, Option<String>)>,
    /// References resolved: (attribute, target, copies usvg makes).
    resolved: Vec<(&'static str, usize)>,
    /// The vertices of its geometry (path segments, points, a line's two).
    vertices: u64,
}

fn static_name(name: &str) -> Option<&'static str> {
    KEPT.iter()
        .chain(FILTERS.iter())
        .chain(["feMergeNode"].iter())
        .find(|k| **k == name)
        .copied()
}

fn static_attr(name: &str) -> &'static str {
    // Leaked once per distinct name: the names are a fixed vocabulary.
    thread_local! {
        static NAMES: std::cell::RefCell<HashMap<String, &'static str>> =
            std::cell::RefCell::new(HashMap::new());
    }
    NAMES.with_borrow_mut(|m| {
        if let Some(n) = m.get(name) {
            return *n;
        }
        let n: &'static str = Box::leak(name.to_string().into_boxed_str());
        m.insert(name.to_string(), n);
        n
    })
}

/// A number written back exactly (Rust's shortest round-trip form, never
/// an exponent).
fn num(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    if v == 0.0 { "0".into() } else { format!("{v}") }
}

fn length(l: svgtypes::Length) -> String {
    use svgtypes::LengthUnit as U;
    let unit = match l.unit {
        U::None => "",
        U::Em => "em",
        U::Ex => "ex",
        U::Px => "px",
        U::In => "in",
        U::Cm => "cm",
        U::Mm => "mm",
        U::Pt => "pt",
        U::Pc => "pc",
        U::Percent => "%",
    };
    format!("{}{unit}", num(l.number))
}

fn color(c: svgtypes::Color) -> String {
    if c.alpha == 255 {
        format!("#{:02x}{:02x}{:02x}", c.red, c.green, c.blue)
    } else {
        format!(
            "rgba({},{},{},{})",
            c.red,
            c.green,
            c.blue,
            num(f64::from(c.alpha) / 255.0)
        )
    }
}

/// The id a `url(#id)` (or `url("#id")`) names.
fn url_id(v: &str) -> Option<&str> {
    let v = v.trim();
    let inner = v.strip_prefix("url(")?.strip_suffix(')')?;
    inner.trim().trim_matches(['"', '\'']).strip_prefix('#')
}

/// The vertices of a path's data, and the data written back.
fn path_data(d: &str) -> (String, u64) {
    use svgtypes::SimplePathSegment as S;
    let mut out = String::new();
    let mut n = 0;
    for seg in svgtypes::SimplifyingPathParser::from(d) {
        let Ok(seg) = seg else { break };
        n += 1;
        match seg {
            S::MoveTo { x, y } => write!(out, "M{} {} ", num(x), num(y)),
            S::LineTo { x, y } => write!(out, "L{} {} ", num(x), num(y)),
            S::CurveTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => write!(
                out,
                "C{} {} {} {} {} {} ",
                num(x1),
                num(y1),
                num(x2),
                num(y2),
                num(x),
                num(y)
            ),
            S::Quadratic { x1, y1, x, y } => {
                write!(out, "Q{} {} {} {} ", num(x1), num(y1), num(x), num(y))
            }
            S::ClosePath => write!(out, "Z "),
        }
        .expect("a String");
    }
    (out.trim_end().to_string(), n)
}

/// A value of `kind` parsed and written back; `None` when it does not parse.
/// A reference is returned as `Err((want, id, fallback))` for the caller to
/// resolve.
#[allow(clippy::type_complexity)]
fn sanitize_value(kind: Kind, v: &str) -> Option<Result<String, (Want, String, Option<String>)>> {
    let v = v.trim();
    Some(Ok(match kind {
        Kind::Paint => match svgtypes::Paint::from_str(v).ok()? {
            svgtypes::Paint::None => "none".into(),
            svgtypes::Paint::Inherit => "inherit".into(),
            svgtypes::Paint::CurrentColor => "currentColor".into(),
            svgtypes::Paint::Color(c) => color(c),
            svgtypes::Paint::ContextFill => "context-fill".into(),
            svgtypes::Paint::ContextStroke => "context-stroke".into(),
            svgtypes::Paint::FuncIRI(id, fallback) => {
                let fallback = fallback.map(|f| match f {
                    svgtypes::PaintFallback::None => "none".to_string(),
                    svgtypes::PaintFallback::CurrentColor => "currentColor".to_string(),
                    svgtypes::PaintFallback::Color(c) => color(c),
                });
                return Some(Err((Want::PaintServer, id.to_string(), fallback)));
            }
        },
        Kind::Ref(want) => {
            if v == "none" {
                "none".into()
            } else {
                return Some(Err((want, url_id(v)?.to_string(), None)));
            }
        }
        Kind::Color => {
            if v.eq_ignore_ascii_case("currentcolor") {
                "currentColor".into()
            } else {
                color(v.parse::<svgtypes::Color>().ok()?)
            }
        }
        Kind::Number => {
            let (n, pct) = match v.strip_suffix('%') {
                Some(p) => (p, true),
                None => (v, false),
            };
            let n: f64 = n.trim().parse().ok()?;
            if pct { format!("{}%", num(n)) } else { num(n) }
        }
        Kind::NumberList => {
            let mut out = Vec::new();
            for n in svgtypes::NumberListParser::from(v) {
                out.push(num(n.ok()?));
            }
            if out.is_empty() {
                return None;
            }
            out.join(" ")
        }
        Kind::Length => length(v.parse::<svgtypes::Length>().ok()?),
        Kind::LengthList => {
            if v == "none" {
                "none".into()
            } else {
                let mut out = Vec::new();
                for l in svgtypes::LengthListParser::from(v) {
                    out.push(length(l.ok()?));
                }
                if out.is_empty() {
                    return None;
                }
                out.join(" ")
            }
        }
        Kind::Keyword => {
            if v.len() > 64
                || !v
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || " .,%+-_".contains(c))
            {
                return None;
            }
            v.to_string()
        }
        Kind::Name => {
            if v.len() > 64
                || !v
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
            {
                return None;
            }
            v.to_string()
        }
        Kind::Path => path_data(v).0,
        Kind::Points => {
            let pts: Vec<String> = svgtypes::PointsParser::from(v)
                .map(|(x, y)| format!("{},{}", num(x), num(y)))
                .collect();
            pts.join(" ")
        }
        Kind::Transform => {
            let t = v.parse::<svgtypes::Transform>().ok()?;
            format!(
                "matrix({} {} {} {} {} {})",
                num(t.a),
                num(t.b),
                num(t.c),
                num(t.d),
                num(t.e),
                num(t.f)
            )
        }
    }))
}

// ---------------------------------------------------------------- CSS

#[derive(Default)]
struct Compound {
    tag: Option<String>,
    classes: Vec<String>,
    ids: Vec<String>,
}

struct Selector {
    /// `(first, combinator)` for a two-compound selector.
    first: Option<(Compound, char)>,
    last: Compound,
    specificity: (u32, u32, u32),
}

struct Rule {
    selectors: Vec<Selector>,
    declarations: Vec<(String, String, bool)>,
}

fn is_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

fn compound(text: &str) -> Option<Compound> {
    let at = text.find(['.', '#']).unwrap_or(text.len());
    let (head, mut rest) = text.split_at(at);
    let mut c = Compound::default();
    if head == "*" {
    } else if is_ident(head) {
        c.tag = Some(head.to_string());
    } else if !(head.is_empty() && !rest.is_empty()) {
        return None;
    }
    while !rest.is_empty() {
        let (lead, body) = rest.split_at(1);
        let end = body.find(['.', '#']).unwrap_or(body.len());
        let name = &body[..end];
        if !is_ident(name) {
            return None;
        }
        if lead == "." {
            c.classes.push(name.to_string());
        } else {
            c.ids.push(name.to_string());
        }
        rest = &body[end..];
    }
    Some(c)
}

fn parse_selector(text: &str) -> Option<Selector> {
    let text = text.trim();
    // At most one combinator: split at the first one.
    let comb_at = text.find(|c: char| c.is_whitespace() || matches!(c, '>' | '+' | '~'));
    let (first, last) = match comb_at {
        None => (None, compound(text)?),
        Some(i) => {
            let (a, b) = text.split_at(i);
            let b = b.trim_start();
            let (comb, b) = match b.chars().next() {
                Some(c @ ('>' | '+' | '~')) => (c, b[1..].trim_start()),
                _ => (' ', b),
            };
            if b.contains(|c: char| c.is_whitespace() || matches!(c, '>' | '+' | '~')) {
                return None;
            }
            (Some((compound(a)?, comb)), compound(b)?)
        }
    };
    let spec = |c: &Compound| {
        (
            c.ids.len() as u32,
            c.classes.len() as u32,
            u32::from(c.tag.is_some()),
        )
    };
    let mut s = spec(&last);
    if let Some((f, _)) = &first {
        let f = spec(f);
        s = (s.0 + f.0, s.1 + f.1, s.2 + f.2);
    }
    Some(Selector {
        first,
        last,
        specificity: s,
    })
}

fn parse_declarations(body: &str) -> Vec<(String, String, bool)> {
    body.split(';')
        .filter_map(|d| {
            let (p, v) = d.split_once(':')?;
            let v = v.trim();
            let (v, important) = match v.strip_suffix("!important") {
                Some(v) => (v.trim(), true),
                None => (v, false),
            };
            Some((p.trim().to_ascii_lowercase(), v.to_string(), important))
        })
        .collect()
}

/// A `<style>` sheet's rules, within the profile: no `@` rule; selectors of
/// type, `*`, `.class` and `#id` compounds with at most one combinator.
fn parse_sheet(sheet: &str) -> Result<Vec<Rule>, String> {
    let mut css = String::with_capacity(sheet.len());
    let mut rest = sheet;
    while let Some(at) = rest.find("/*") {
        css.push_str(&rest[..at]);
        rest = match rest[at + 2..].find("*/") {
            Some(end) => &rest[at + 2 + end + 2..],
            None => "",
        };
    }
    css.push_str(rest);
    if css.contains('@') {
        return refuse("a CSS @ rule");
    }
    let mut rules = Vec::new();
    for rule in css.split('}') {
        if rule.trim().is_empty() {
            continue;
        }
        let Some((head, body)) = rule.split_once('{') else {
            return refuse("malformed CSS");
        };
        if body.contains('{') {
            return refuse("nested CSS");
        }
        let mut selectors = Vec::new();
        for s in head.split(',') {
            match parse_selector(s) {
                Some(sel) => selectors.push(sel),
                None => return refuse(format!("the CSS selector `{}`", s.trim())),
            }
        }
        rules.push(Rule {
            selectors,
            declarations: parse_declarations(body),
        });
    }
    Ok(rules)
}

fn compound_matches(c: &Compound, n: Node) -> bool {
    if !n.is_element() {
        return false;
    }
    if let Some(tag) = &c.tag
        && n.tag_name().name() != tag
    {
        return false;
    }
    let classes = n.attribute("class").unwrap_or("");
    c.classes
        .iter()
        .all(|k| classes.split_ascii_whitespace().any(|c| c == k))
        && c.ids
            .iter()
            .all(|id| n.attribute("id") == Some(id.as_str()))
}

fn selector_matches(s: &Selector, n: Node, steps: &mut u64) -> bool {
    *steps += 1;
    if !compound_matches(&s.last, n) {
        return false;
    }
    let Some((first, comb)) = &s.first else {
        return true;
    };
    let mut hit = |m: Node| {
        *steps += 1;
        compound_matches(first, m)
    };
    match comb {
        '>' => n.parent_element().is_some_and(&mut hit),
        '+' => n.prev_sibling_element().is_some_and(&mut hit),
        '~' => {
            let mut p = n.prev_sibling_element();
            while let Some(m) = p {
                if hit(m) {
                    return true;
                }
                p = m.prev_sibling_element();
            }
            false
        }
        _ => n.ancestors().skip(1).filter(|a| a.is_element()).any(hit),
    }
}

// ---------------------------------------------------------------- the build

/// Where a declaration comes from, for the cascade: (origin — attribute 0,
/// stylesheet 1, `style=""` 2, each plus 2 when `!important` — unused,
/// specificity, rule order).
type Priority = (u8, u8, (u32, u32, u32), usize);

/// Per element while counting: `None` before, `Some(None)` in progress (a
/// cycle, which usvg drops), else (count, height, vertices below).
type Memo = Vec<Option<Option<(u64, usize, u64)>>>;

/// The sanitized document's text (see the module doc), or why the file is
/// refused.
pub(crate) fn sanitize(doc: &Document) -> Result<String, String> {
    // Every `<style>` in the document, wherever it is.
    let mut rules = Vec::new();
    for style in doc.descendants().filter(|n| n.has_tag_name("style")) {
        for text in style.children().filter_map(|c| c.text()) {
            rules.extend(parse_sheet(text)?);
        }
    }
    if rules.iter().map(|r| r.selectors.len()).sum::<usize>() > MAX_SELECTORS {
        return refuse(format!("more than {MAX_SELECTORS} CSS selectors"));
    }

    // 1. The elements kept, in document order.
    let mut outs: Vec<Out> = Vec::new();
    let root = doc.root_element();
    let mut stack: Vec<(Node, Option<usize>)> = vec![(root, None)];
    while let Some((node, parent)) = stack.pop() {
        if !node.is_element() {
            continue;
        }
        if !matches!(node.tag_name().namespace(), None | Some(SVG_NS)) {
            continue;
        }
        let name = node.tag_name().name();
        if DROPPED.contains(&name) || !USVG_ELEMENTS.contains(&name) {
            continue;
        }
        let parent_name = parent.map(|p| outs[p].name);
        let ok = match parent_name {
            Some("filter") => FILTERS.contains(&name),
            Some("feMerge") => name == "feMergeNode",
            Some(p) if p.starts_with("fe") => false,
            _ => KEPT.contains(&name),
        };
        if !ok {
            return refuse(format!("the element <{name}>"));
        }
        let count = |pred: &dyn Fn(Node) -> bool| node.children().filter(|c| pred(*c)).count();
        match name {
            "filter" => {
                let n = count(&|c| c.is_element());
                if n > MAX_FILTER_PRIMITIVES {
                    return refuse(format!(
                        "a filter of {n} primitives (at most {MAX_FILTER_PRIMITIVES})"
                    ));
                }
            }
            "feMerge" => {
                let n = count(&|c| c.is_element());
                if n > MAX_MERGE_NODES {
                    return refuse(format!(
                        "an feMerge of {n} inputs (at most {MAX_MERGE_NODES})"
                    ));
                }
            }
            "linearGradient" | "radialGradient" => {
                let n = count(&|c| c.has_tag_name("stop"));
                if n > MAX_GRADIENT_STOPS {
                    return refuse(format!(
                        "a gradient of {n} stops (at most {MAX_GRADIENT_STOPS})"
                    ));
                }
            }
            _ => {}
        }
        let index = outs.len();
        outs.push(Out {
            node,
            name: static_name(name).expect("a kept name"),
            children: Vec::new(),
            attrs: Vec::new(),
            refs: Vec::new(),
            resolved: Vec::new(),
            vertices: 0,
        });
        if let Some(p) = parent {
            outs[p].children.push(index);
        }
        let kids: Vec<Node> = node.children().filter(|c| c.is_element()).collect();
        for kid in kids.into_iter().rev() {
            stack.push((kid, Some(index)));
        }
    }
    if outs.is_empty() {
        return refuse("no root");
    }

    // 2. Ids: each to the emitted elements that carry it, in document order.
    // A reference resolves to the first of them of the kind it wants (a
    // file that reuses an id for a path and its gradient means the
    // gradient), and the output gives every element its own id.
    let mut ids: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, o) in outs.iter().enumerate() {
        if let Some(id) = o.node.attribute("id") {
            ids.entry(id).or_default().push(i);
        }
    }

    // 3. Attributes: the cascade, each value re-emitted.
    let mut steps = 0u64;
    for o in outs.iter_mut() {
        let node = o.node;
        let name = o.name;
        // (value, priority): attribute 0, CSS (1, important, specificity,
        // order), `style=""` 2 (or 3 when important).
        let mut props: HashMap<String, (String, Priority)> = HashMap::new();
        let mut set = |p: &str, v: &str, prio: (u8, u8, (u32, u32, u32), usize)| {
            let p = if p == "marker" { "marker-*" } else { p };
            let names: Vec<&str> = if p == "marker-*" {
                vec!["marker-start", "marker-mid", "marker-end"]
            } else {
                vec![p]
            };
            for p in names {
                let e = props
                    .entry(p.to_string())
                    .or_insert((String::new(), (0, 0, (0, 0, 0), 0)));
                if e.0.is_empty() || prio >= e.1 {
                    *e = (v.to_string(), prio);
                }
            }
        };
        // Attributes in no namespace only: an editor's `sodipodi:cx` is not
        // `cx`.
        let plain = || node.attributes().filter(|a| a.namespace().is_none());
        for a in plain() {
            if property_kind(a.name()).is_some() {
                set(a.name(), a.value(), (0, 0, (0, 0, 0), 0));
            }
        }
        for (order, rule) in rules.iter().enumerate() {
            for sel in &rule.selectors {
                if steps > MAX_MATCH_STEPS {
                    return refuse("CSS that takes too long to match");
                }
                if selector_matches(sel, node, &mut steps) {
                    for (p, v, important) in &rule.declarations {
                        set(
                            p,
                            v,
                            (1 + 2 * u8::from(*important), 0, sel.specificity, order),
                        );
                    }
                }
            }
        }
        if let Some(style) = node.attribute("style") {
            for (p, v, important) in parse_declarations(style) {
                set(&p, &v, (2 + 2 * u8::from(important), 0, (0, 0, 0), 0));
            }
        }
        let mut attrs: Vec<(&'static str, String)> = Vec::new();
        let mut refs = Vec::new();
        let mut handle = |attr: &str, kind: Kind, value: &str| match sanitize_value(kind, value) {
            Some(Ok(v)) => attrs.push((static_attr(attr), v)),
            Some(Err((want, id, fallback))) => refs.push((static_attr(attr), want, id, fallback)),
            None => {}
        };
        let mut names: Vec<&String> = props.keys().collect();
        names.sort();
        for p in names {
            if let Some(kind) = property_kind(p) {
                handle(p, kind, &props[p].0);
            }
        }
        for a in plain() {
            if let Some(kind) = attribute_kind(a.name()) {
                handle(a.name(), kind, a.value());
            }
        }
        // `href` / `xlink:href` (the namespace is the attribute's, not the
        // name's): only on `<use>` and a gradient.
        if let Some(href) = node
            .attributes()
            .find(|a| {
                a.name() == "href"
                    && matches!(a.namespace(), None | Some("http://www.w3.org/1999/xlink"))
            })
            .map(|a| a.value())
        {
            let want = match name {
                "use" => Some(Want::Drawable),
                "linearGradient" | "radialGradient" => Some(Want::Gradient),
                _ => None,
            };
            if let (Some(want), Some(id)) = (want, href.trim().strip_prefix('#')) {
                refs.push(("href", want, id.to_string(), None));
            }
        }
        let vertices = match name {
            "path" => node.attribute("d").map_or(0, |d| path_data(d).1),
            "polyline" | "polygon" => node
                .attribute("points")
                .map_or(0, |p| svgtypes::PointsParser::from(p).count() as u64),
            "line" => 2,
            _ => 0,
        };
        o.attrs = attrs;
        o.refs = refs;
        o.vertices = vertices;
    }

    // 4. References: to an emitted element of the right kind, or dropped.
    for i in 0..outs.len() {
        let refs = std::mem::take(&mut outs[i].refs);
        for (attr, want, id, fallback) in refs {
            let target = ids.get(id.as_str()).and_then(|ts| {
                ts.iter()
                    .copied()
                    .find(|&t| want.accepts(outs[t].name) && t != i)
            });
            match target {
                Some(t) => {
                    outs[i].resolved.push((attr, t));
                    let mut v = format!("url(#i{t})");
                    if let Some(f) = fallback {
                        v.push(' ');
                        v.push_str(&f);
                    }
                    if attr != "href" {
                        outs[i].attrs.push((attr, v));
                    }
                }
                None => {
                    if want == Want::PaintServer {
                        outs[i]
                            .attrs
                            .push((attr, fallback.unwrap_or_else(|| "none".into())));
                    }
                }
            }
        }
    }

    // 5. One level: nothing a clip path, mask, pattern or marker draws
    // (following `<use>`) uses another.
    for i in 0..outs.len() {
        if !ONE_LEVEL.contains(&outs[i].name) {
            continue;
        }
        let mut seen = HashSet::new();
        let mut inner = vec![i];
        while let Some(n) = inner.pop() {
            if !seen.insert(n) {
                continue;
            }
            for &(attr, t) in &outs[n].resolved {
                if ONE_LEVEL.contains(&outs[t].name) {
                    return refuse(format!(
                        "a <{}> that uses a <{}>",
                        outs[i].name, outs[t].name
                    ));
                }
                if attr == "href" {
                    inner.push(t);
                }
            }
            inner.extend(outs[n].children.iter().copied());
        }
    }

    // 6. What usvg will expand: `<use>` copies its target, a marker is drawn
    // at every vertex of what it is set on; the rest are shared.
    expansion(&outs)?;

    // 7. The text usvg is given.
    let referenced: HashSet<usize> = outs
        .iter()
        .flat_map(|o| o.resolved.iter().map(|&(_, t)| t))
        .collect();
    let mut text = String::new();
    write_element(&outs, 0, &referenced, &mut text);
    Ok(text)
}

/// Refuse what usvg would expand past [`MAX_DEPTH`] or [`MAX_NODES`].
fn expansion(outs: &[Out]) -> Result<(), String> {
    // Per element: (count, height, vertices below), memoised; `None` while
    // in progress (a cycle, which usvg drops).
    fn walk(
        outs: &[Out],
        memo: &mut Memo,
        n: usize,
        depth: usize,
    ) -> Result<(u64, usize, u64), String> {
        let too_deep = || {
            Err(format!(
                "SVG refused: nested more than {MAX_DEPTH} deep once its references are expanded"
            ))
        };
        if depth > MAX_DEPTH {
            return too_deep();
        }
        match memo[n] {
            Some(Some((c, h, v))) => {
                return if depth + h > MAX_DEPTH {
                    too_deep()
                } else {
                    Ok((c, h, v))
                };
            }
            Some(None) => return Ok((0, 0, 0)),
            None => {}
        }
        memo[n] = Some(None);
        let o = &outs[n];
        let (mut count, mut height, mut vertices) = (1u64, 0usize, o.vertices);
        for &k in &o.children {
            let (c, h, v) = walk(outs, memo, k, depth + 1)?;
            count = count.saturating_add(c);
            height = height.max(h + 1);
            vertices = vertices.saturating_add(v);
        }
        for &(attr, t) in &o.resolved {
            if attr == "href" && o.name == "use" {
                let (c, h, v) = walk(outs, memo, t, depth + 1)?;
                count = count.saturating_add(c);
                height = height.max(h + 1);
                vertices = vertices.saturating_add(v);
            }
        }
        // Markers: a copy of the marker at every vertex below, set here or
        // inherited (charged where it is set).
        for &(attr, t) in &o.resolved {
            if attr.starts_with("marker") {
                let (c, h, _) = walk(outs, memo, t, depth + 1)?;
                count = count.saturating_add(c.saturating_mul(vertices.max(1)));
                height = height.max(h + 1);
            }
        }
        if count > MAX_NODES {
            return Err(format!(
                "SVG refused: it expands to more than {MAX_NODES} elements"
            ));
        }
        memo[n] = Some(Some((count, height, vertices)));
        Ok((count, height, vertices))
    }
    let mut memo = vec![None; outs.len()];
    walk(outs, &mut memo, 0, 0).map(|_| ())
}

fn escape(v: &str, out: &mut String) {
    for c in v.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
}

/// Write the output, iteratively (it is as deep as the input, bounded by
/// [`MAX_DEPTH`], but a loop costs nothing).
fn write_element(outs: &[Out], root: usize, referenced: &HashSet<usize>, text: &mut String) {
    enum Step {
        Open(usize),
        Close(&'static str),
    }
    let mut stack = vec![Step::Open(root)];
    while let Some(step) = stack.pop() {
        match step {
            Step::Close(name) => {
                text.push_str("</");
                text.push_str(name);
                text.push('>');
            }
            Step::Open(i) => {
                let o = &outs[i];
                let name = if o.name == "a" { "g" } else { o.name };
                text.push('<');
                text.push_str(name);
                if i == root {
                    text.push_str(" xmlns=\"http://www.w3.org/2000/svg\"");
                }
                if referenced.contains(&i) {
                    let _ = write!(text, " id=\"i{i}\"");
                }
                for (a, v) in &o.attrs {
                    text.push(' ');
                    text.push_str(a);
                    text.push_str("=\"");
                    escape(v, text);
                    text.push('"');
                }
                for &(attr, t) in &o.resolved {
                    if attr == "href" {
                        let _ = write!(text, " href=\"#i{t}\"");
                    }
                }
                text.push('>');
                stack.push(Step::Close(name));
                for &k in o.children.iter().rev() {
                    stack.push(Step::Open(k));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(svg: &str) -> Result<String, String> {
        let doc = Document::parse(svg).unwrap();
        sanitize(&doc)
    }

    #[test]
    fn path_data_counts_real_vertices() {
        // `.1.1.1…` is many numbers, `h`/`v` one each.
        let (_, n) = path_data(&format!("M0 0{}", "l.1.1".repeat(100)));
        assert_eq!(n, 101);
        let (_, n) = path_data("M0 0h1v1h1v1z");
        assert_eq!(n, 6);
    }

    #[test]
    fn a_duplicate_id_resolves_to_the_first_of_the_kind_wanted() {
        let out = run(r##"<svg xmlns="http://www.w3.org/2000/svg"><path id="a"/><linearGradient id="a"><stop/></linearGradient><linearGradient id="a"><stop/></linearGradient><mask id="b"/><rect fill="url(#a)" mask="url(#a)" stroke="url(#b)"/></svg>"##).unwrap();
        // The fill reaches the first gradient `a` (i2), not the path or the
        // second gradient; no `a` is a mask, so the mask is dropped; `b` is
        // a mask, not a paint server: the stroke is none.
        assert!(out.contains("fill=\"url(#i2)\""), "{out}");
        assert!(!out.contains("mask=\"url"), "{out}");
        assert!(out.contains("stroke=\"none\""), "{out}");
        // Every id in the output is distinct.
        let ids: Vec<&str> = out
            .match_indices(" id=\"")
            .map(|(i, _)| &out[i..i + 8])
            .collect();
        let unique: HashSet<&&str> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len(), "{out}");
    }

    #[test]
    fn ignored_subtrees_are_left_out_and_their_ids_resolve_to_nothing() {
        let out = run(r##"<svg xmlns="http://www.w3.org/2000/svg"><image><filter id="f"><feTurbulence/></filter></image><rect filter="url(#f)"/></svg>"##).unwrap();
        assert!(!out.contains("filter"), "{out}");
        assert!(!out.contains("image"), "{out}");
    }

    #[test]
    fn css_is_inlined_by_specificity_and_order() {
        let out = run(r##"<svg xmlns="http://www.w3.org/2000/svg"><style>rect { fill: red } .a { fill: blue } #b { stroke: green }</style><rect class="a" id="b" fill="yellow"/></svg>"##).unwrap();
        assert!(out.contains("fill=\"#0000ff\""), "{out}");
        assert!(out.contains("stroke=\"#008000\""), "{out}");
        assert!(!out.contains("style"), "{out}");
    }
}
