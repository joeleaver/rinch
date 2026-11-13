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
//! - `-webkit-box-flex: N` into `--rinch-webkit-box-flex: N` (#2581),
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
//!   shrink by `-webkit-box-flex` alone (`--rinch-webkit-box-flex`);
//! - `-webkit-line-clamp` or `-webkit-box-orient` on a box that is not
//!   `-webkit-box` does nothing.
//!
//! Not done (#2581): a `var()` in either property is not read. Known small
//! divergences: `getComputedStyle`-style readers see `flow-root`/
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

fn is_wide_keyword(v: &str) -> bool {
    ["inherit", "initial", "unset", "revert", "revert-layer"]
        .iter()
        .any(|k| v.eq_ignore_ascii_case(k))
}

/// A `-webkit-line-clamp` value as Chrome parses it: `none` or an integer
/// of at least 1.
fn valid_clamp(v: &str) -> bool {
    v.eq_ignore_ascii_case("none")
        || is_wide_keyword(v)
        || v.strip_prefix('+')
            .unwrap_or(v)
            .parse::<u32>()
            .is_ok_and(|n| n >= 1)
}

/// A `-webkit-box-flex` value: a non-negative number.
fn valid_flex(v: &str) -> bool {
    is_wide_keyword(v) || parse_flex(v).is_some()
}

fn parse_flex(v: &str) -> Option<f32> {
    v.parse::<f32>().ok().filter(|f| f.is_finite() && *f >= 0.0)
}

fn valid_orient(v: &str) -> bool {
    ["horizontal", "vertical", "inline-axis", "block-axis"]
        .iter()
        .any(|k| v.eq_ignore_ascii_case(k))
        || is_wide_keyword(v)
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
            "-webkit-box-orient" if valid_orient(v) => {
                out.push_str(&format!("{lead}--{ORIENT}: {v}{bang}"));
            }
            "-webkit-box-flex" if valid_flex(v) => {
                out.push_str(&format!("{lead}--{FLEX}: {v}{bang}"));
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
            .replace("-webkit-inline-box", "inline-block")
            .replace("-webkit-box", "flow-root")
            .replace("\u{0}C", &format!("--{CLAMP}"))
            .replace("\u{0}O", &format!("--{ORIENT}"))
            .replace("\u{0}F", &format!("--{FLEX}")),
    )
}

/// `css` with the three Gecko-only spellings rewritten into declarations
/// Stylo's servo build keeps (see the module docs). `css` is a style sheet,
/// or with `inline` an inline `style` declaration list. Borrowed unchanged
/// when it has no `-webkit-` in it.
pub(crate) fn rewrite(css: &str, inline: bool) -> Cow<'_, str> {
    if !contains_ci(css, "-webkit-") {
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
                if contains_ci(list, "-webkit-") {
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
    if contains_ci(rest, "-webkit-") {
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

/// A rewritten custom property's declared text, or its CSS-wide keyword.
enum Declared {
    Value(String),
    Keyword(style::properties::CSSWideKeyword),
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
    static NAMES: [style::Atom; 4] = [DISPLAY, CLAMP, ORIENT, FLEX].map(style::Atom::from);
}

/// Whether the element or an ancestor declares any of the three: custom
/// properties inherit, so one declared on the element is in its computed
/// map. Most elements have none, and [`apply`] is skipped for them.
pub(crate) fn may_declare(cv: &style::properties::ComputedValues) -> bool {
    let props = cv.custom_properties();
    if props.inherited.is_empty() && props.non_inherited.is_empty() {
        return false;
    }
    NAMES.with(|names| {
        names.iter().any(|name| {
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
    let clamp = own_winner(cv, guard, |d| custom(d, CLAMP)).and_then(|d| match d {
        Declared::Value(v) => v.strip_prefix('+').unwrap_or(&v).parse::<u32>().ok(),
        Declared::Keyword(k) if inherit(&k) => parent.and_then(|p| p.webkit_line_clamp),
        Declared::Keyword(_) => None,
    });
    let vertical = own_winner(cv, guard, |d| custom(d, ORIENT)).is_some_and(|d| match d {
        Declared::Value(v) => {
            v.eq_ignore_ascii_case("vertical") || v.eq_ignore_ascii_case("block-axis")
        }
        Declared::Keyword(k) => inherit(&k) && parent.is_some_and(|p| p.webkit_box_vertical),
    });
    style.webkit_line_clamp = clamp;
    style.webkit_box_vertical = vertical;
    style.line_clamp = None;
    style.webkit_box_flex = own_winner(cv, guard, |d| custom(d, FLEX))
        .and_then(|d| match d {
            Declared::Value(v) => parse_flex(&v),
            Declared::Keyword(k) if inherit(&k) => parent.map(|p| p.webkit_box_flex),
            Declared::Keyword(_) => None,
        })
        .unwrap_or(0.0);
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
        custom(d, DISPLAY).map(|d| match d {
            Declared::Value(v) => Some(v.eq_ignore_ascii_case("-webkit-inline-box")),
            Declared::Keyword(_) => None,
        })
    })
    .flatten();
    let Some(inline) = marker else {
        return;
    };
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
        tree.line_clamp_roots.retain(|&id| {
            nodes
                .get_live(id)
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
                    border_box
                };
                bottom = nodes[id].layout.y + height;
            }
        }

        // Take back what no clamp gives now, and give what changed.
        let mut changed_roots: Vec<RawNodeId> = Vec::new();
        let mut changed_boxes: Vec<RawNodeId> = Vec::new();
        let old = std::mem::take(&mut tree.line_clamped);
        for id in old {
            let Some(node) = tree.nodes.get_live_mut(id) else {
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
