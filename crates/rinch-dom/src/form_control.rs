//! The intrinsic content size of a line-sized control (#297, #1177): `<input>`,
//! `<textarea>`, and a blockified `<br>`.
//!
//! An `<input>` and a `<textarea>` hold their value in an **attribute**, not in
//! a child, so they are childless however much text they show and nothing in
//! the box tree gives them a height. (A textarea's text *child* is its default
//! value, and does not size it either — [`is_value_control`].) A browser sizes them from their own
//! metrics: one line box for a single-line `<input>`, `rows` line boxes for a
//! `<textarea>` (2 when `rows` is absent or invalid), plus padding and border.
//! Measured in Chrome 153 with `line-height: 20px; padding: 6px 10px; border:
//! 1px`: an `<input>` is 34px tall whether it is inline, a flex item or
//! `display: block`, and a two-row `<textarea>` 54px.
//!
//! rinch answers that through a Taffy **measure**, not a `min-height`
//! (`NodeContext::FormControl`), because the difference is observable: a
//! column-flex item's `min-height: 0` lets the control shrink below its
//! content (Chrome: 10px in a 10px column), and `min-height: auto` does not
//! (20px). A `min-height` written by rinch cannot tell the two apart. Taffy
//! adds the padding and border to what the measure answers, so the measure
//! answers the content box alone.
//!
//! **Width comes from `size` and `cols`** (#1177), as in Chrome: the primary
//! font's average character width (`avg`, the OS/2 `xAvgCharWidth` scaled to
//! the font size) times `size` (an `<input>`, default 20) or `cols` (a
//! `<textarea>`, default 20), plus, for an `<input>`, its widest glyph extent
//! less one average character and, for a `<textarea>`, a 15px scrollbar
//! gutter. See [`form_control_content_width`] for the formula and what it does
//! not model. The measure answers it as the content-box width wherever the
//! width is intrinsic — inline, a flex-row item, inside an `inline-block` —
//! and an author `width` (or `max-width`) wins as for any box. A control's
//! value never sizes it. One difference is left: a `display: block` control
//! with an `auto` width keeps its intrinsic width in Chrome (248px at 16px
//! Inter) and fills its container in rinch, which lays it out as any block.

use crate::computed_style::{ComputedStyle, OverflowValue};
use crate::node::{Node, NodeContext, NodeTree};

/// The scrollbar gutter Chrome 153 reserves in a `<textarea>`'s intrinsic
/// width (its classic scrollbar, measured on Linux: a `cols=20` textarea is
/// `ceil(20 × avg) + 15` wide, and `ceil(20 × avg)` with `overflow-y:
/// hidden`). Reserved whether or not the value overflows, so the control does
/// not change size when it starts to scroll.
pub const TEXTAREA_SCROLLBAR_GUTTER: f32 = 15.0;

/// The largest width Chrome gives a control, `LayoutUnit`'s maximum:
/// `size="2147483647"` is 33554432px wide in Chrome 153.
const MAX_CONTROL_WIDTH: f32 = 33_554_432.0;

/// A font's text-control metrics, in em: the OS/2 `xAvgCharWidth` (`avg_em`)
/// and the `head` table's `xMax - xMin` (`max_em`). What Chrome sizes a
/// control from (`SimpleFontData::AvgCharWidth` / `MaxCharWidth`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CharMetrics {
    /// The average character width, per em. A font with no OS/2 table, or a
    /// zero `xAvgCharWidth`, answers the advance of its `0` instead.
    pub(crate) avg_em: f32,
    /// The widest glyph extent (`xMax - xMin`), per em; 0 when unknown.
    pub(crate) max_em: f32,
}

/// The content-box height the control at `node` is owed, or `None` when it is
/// not a line-sized control.
///
/// - `<textarea>`: `rows` lines, 2 when `rows` is absent or below 1. (`rows` is
///   read by HTML's rules for parsing non-negative integers — #1153.)
/// - `<input>`: one line for **every** type but `checkbox`, `radio`, `range`,
///   `color`, `image` and `hidden`. That is HTML's rule read from the other
///   end: an absent or invalid `type` is the Text state, and the date/time
///   family, the button types and `file` all lay out one line in Chrome 153
///   (20–22px at `line-height: 20px`). The six excluded types have intrinsic
///   sizes of their own (13x13, 16, 27, the image's) that this does not model.
///   `type` is ASCII case-insensitive.
/// - `<br>`: one line. An in-flow `<br>` is IFC content and never reaches a
///   Taffy measure; one that is blockified — a flex item, `display: block` — is
///   a line tall in Chrome 153, which the removed empty-block floor used to
///   give it (#296).
pub(crate) fn form_control_content_height(node: &Node) -> Option<f32> {
    let rows = match node.tag()? {
        // HTML's rules for parsing non-negative integers, and 2 unless that
        // gives more than zero (#1153). A float parse took `"1e1"` as 10 rows
        // and `"inf"` / `"1e30"` as infinite / NaN, which made every box laid
        // out after the textarea NaN.
        "textarea" => node
            .attributes
            .get("rows")
            .and_then(|r| rinch_core::dom::parse_html_non_negative_integer(r))
            .filter(|&r| r > 0)
            .map_or(2.0, |r| r as f32),
        "input" => {
            let sized_otherwise = node.attributes.get("type").is_some_and(|t| {
                matches!(
                    t.trim().to_ascii_lowercase().as_str(),
                    "checkbox" | "radio" | "range" | "color" | "image" | "hidden"
                )
            });
            if sized_otherwise {
                return None;
            }
            1.0
        }
        "br" => 1.0,
        _ => return None,
    };
    Some(rows * node.computed_style.line_height_px())
}

/// Make the Taffy measure context of `node_id` say what
/// [`form_control_content_height`] says, and answer whether it had to change
/// (the caller then owes a layout: `set_node_context` has already marked the
/// Taffy node dirty).
///
/// Called wherever a node's computed style is (re)written — the cascade and
/// the two animation ticks — because the line height is part of the answer and
/// is not a Taffy property: a `line-height` or `font-size` change moves no
/// Taffy style, so nothing else would dirty the node.
pub(crate) fn sync_form_control_measure(
    tree: &mut NodeTree,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<peniko::Brush>,
    node_id: usize,
) -> bool {
    let Some(node) = tree.nodes.get(node_id) else {
        return false;
    };
    let Some(taffy_id) = node.taffy_id else {
        return false;
    };
    let want = form_control_content_size(node, font_cx, layout_cx, &tree.perf);
    let have = match tree.taffy.get_node_context(taffy_id) {
        Some(NodeContext::FormControl {
            content_width,
            content_height,
        }) => Some((*content_width, *content_height)),
        // A context this module did not write is never overwritten: an IFC
        // root's `InlineRoot`, or a text leaf's. A control whose children form
        // an inline formatting context is not one of those — the IFC pass
        // leaves it hollow, carrying the `FormControl` context this function
        // compares against (#1159) — so its `rows`, line height and `type` are
        // tracked here like a childless control's.
        Some(NodeContext::InlineRoot(_)) | Some(NodeContext::Text(_)) => return false,
        _ => None,
    };
    if want == have {
        return false;
    }
    match want {
        Some((content_width, content_height)) => {
            let _ = tree.taffy.set_node_context(
                taffy_id,
                Some(NodeContext::FormControl {
                    content_width,
                    content_height,
                }),
            );
        }
        // A control that stopped being one (its `type` became `checkbox`):
        // take back only the context this module wrote.
        None if have.is_some() => {
            let _ = tree.taffy.set_node_context(taffy_id, None);
        }
        None => return false,
    }
    true
}

/// The measure answer for [`NodeContext::FormControl`], shared by the root
/// compute and the atomic-inline sizer. The same answer for every available
/// space: a control's min-content width is its max-content width, so an
/// `<input>` in a container narrower than it overflows rather than shrinks
/// (Chrome 153: 248px in a 50px div).
pub(crate) fn measure(
    content_width: f32,
    content_height: f32,
    known: taffy::Size<Option<f32>>,
) -> taffy::Size<f32> {
    taffy::Size {
        width: known.width.unwrap_or(content_width),
        height: known.height.unwrap_or(content_height),
    }
}

/// Whether `node` is an `<input>` or a `<textarea>`: an element whose children
/// are not content (#1159).
///
/// A browser renders neither element's children. A `<textarea>`'s direct text
/// children are its **default value** ([`control_value`]); an `<input>` has no
/// content model and its children are ignored. Measured in Chrome 153: a
/// textarea holding "hello world foo" as a child is the size of an empty one
/// and draws the text once, as its value; an `<input>` with a text child draws
/// nothing. So when such a control's children form an inline formatting
/// context, the IFC pass leaves it **hollow**: its Taffy node carries
/// [`NodeContext::FormControl`] (or no context) instead of `InlineRoot`, so it
/// is measured exactly as a childless control is, and `build_ifc_layouts`
/// shapes no text for it, so nothing lays its children out or paints them.
/// `paint_input_value` draws the value. Only inline children reach that: a
/// block-level element child is still laid out as content (#1178).
pub fn is_value_control(node: &Node) -> bool {
    matches!(node.tag(), Some("input" | "textarea"))
}

/// The text a field holds: its `value` attribute, else a `<textarea>`'s
/// **child text content** (HTML: the concatenation of its direct `Text`
/// children — its default value), else `None`. Any element with a `value`
/// attribute answers it, so the desktop shell reads every field through this,
/// custom `data-oninput` controls included.
///
/// rinch keeps a control's live value in its `value` attribute (typing writes
/// it, and so does `value_fn`), so an attribute that is present wins even when
/// it is empty — the web's dirty value flag, read from the other end: once a
/// field has been typed into or written, its children no longer show. An
/// `<input>` has no default value in its children, so it answers only its
/// attribute. `None` for any other element.
pub fn control_value(
    nodes: &slab::Slab<Node>,
    node_id: usize,
) -> Option<std::borrow::Cow<'_, str>> {
    use std::borrow::Cow;
    let node = nodes.get(node_id)?;
    if let Some(v) = node.attributes.get("value") {
        return Some(Cow::Borrowed(v.as_str()));
    }
    if node.tag() != Some("textarea") {
        return None;
    }
    let mut texts = node
        .children
        .iter()
        .filter_map(|&c| nodes.get(c))
        .filter_map(|c| c.text_content());
    let Some(first) = texts.next() else {
        return Some(Cow::Borrowed(""));
    };
    match texts.next() {
        None => Some(Cow::Borrowed(first)),
        Some(second) => {
            let mut all = String::from(first);
            all.push_str(second);
            texts.for_each(|t| all.push_str(t));
            Some(Cow::Owned(all))
        }
    }
}

/// The Taffy context a hollow control root carries in place of `InlineRoot`
/// ([`is_value_control`]): what [`sync_form_control_measure`] gives a
/// childless one.
pub(crate) fn hollow_control_context(
    node: &Node,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<peniko::Brush>,
    perf: &crate::perf::PerfCounters,
) -> Option<NodeContext> {
    form_control_content_size(node, font_cx, layout_cx, perf).map(
        |(content_width, content_height)| NodeContext::FormControl {
            content_width,
            content_height,
        },
    )
}

/// The content-box size `(width, height)` the control at `node` is owed, or
/// `None` when it is not a line-sized control
/// ([`form_control_content_height`]). The width needs the node's primary
/// font, which is looked up only for a control, and cached on the node.
pub(crate) fn form_control_content_size(
    node: &Node,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<peniko::Brush>,
    perf: &crate::perf::PerfCounters,
) -> Option<(f32, f32)> {
    let height = form_control_content_height(node)?;
    let factor = match node.tag() {
        Some("textarea") => Some(size_attribute(node, "cols")),
        Some("input") => input_width_factor(node),
        _ => None,
    };
    let width = match factor {
        Some(factor) => {
            let metrics = cached_char_metrics(node, font_cx, layout_cx, perf);
            form_control_content_width(node, &node.computed_style, metrics, factor)
        }
        None => 0.0,
    };
    Some((width, height))
}

/// `size` or `cols` by HTML's rules for parsing non-negative integers: the
/// value when that gives more than zero, else 20 — which is what Chrome 153
/// gives `0`, `-3` and `abc`, while `" 7"` and `"7px"` are 7 and `"3.9"` is 3.
fn size_attribute(node: &Node, name: &str) -> u32 {
    node.attributes
        .get(name)
        .and_then(|v| rinch_core::dom::parse_html_non_negative_integer(v))
        .filter(|&n| n > 0)
        .unwrap_or(20)
}

/// How many average characters wide an `<input>` is, or `None` for a type
/// this rule does not size.
///
/// `size` applies to the Text, Search, URL, Telephone, E-mail and Password
/// states, and an absent, empty or unknown `type` is the Text state. `number`
/// ignores `size` and is 20 characters wide (Chrome 153: 248px at 16px Inter
/// with or without `size=5`; Chrome narrows it to its `min`/`max` digits,
/// which this does not model). Every other type — the date/time family, the
/// buttons, `file`, and the six [`form_control_content_height`] leaves out —
/// has an intrinsic width of its own in Chrome that is not modelled here, and
/// stays 0.
fn input_width_factor(node: &Node) -> Option<u32> {
    let ty = node
        .attributes
        .get("type")
        .map(|t| t.trim().to_ascii_lowercase());
    match ty.as_deref() {
        None | Some("text" | "search" | "url" | "tel" | "email" | "password") => {
            Some(size_attribute(node, "size"))
        }
        Some("number") => Some(20),
        Some(
            "hidden" | "date" | "month" | "week" | "time" | "datetime-local" | "range" | "color"
            | "checkbox" | "radio" | "file" | "submit" | "image" | "reset" | "button",
        ) => None,
        // An unknown type is the Text state.
        Some(_) => Some(size_attribute(node, "size")),
    }
}

/// The content-box width of a text control `factor` characters wide, in
/// `style`'s font with `metrics`. Chrome 153 on Linux, measured with Inter,
/// DejaVu Sans, Liberation Sans, DejaVu Sans Mono and Space Grotesk from 10px
/// to 24px:
///
/// ```text
/// avg      = avg_em × font-size;  avg' = max(avg, round(avg))
/// input    = ceil(avg' × size + round(max_em × font-size) − avg')
/// textarea = ceil(avg' × cols) + 15   (no 15 when overflow-y is hidden/clip)
/// ```
///
/// `avg'` is read off Chrome's output: sweeping 8–40px in half pixels, the
/// average it uses is the unrounded one where its fraction is below one half
/// and the rounded one where it is at or above (Inter at 20px: 12.80, used as
/// 13). The `− avg'` term is Chrome's own "IE adds some extra width" rule for
/// a single-line control; a font whose extent is unknown gets `avg' × size`.
///
/// Not modelled: the exceptions Chrome makes for particular families (a list
/// of faces whose `xAvgCharWidth` it does not trust, and a fixed extent for
/// `system-ui` — measured on this host, a `system-ui` input is not what the
/// formula gives for the face `system-ui` resolves to), and a vertical writing
/// mode, where Chrome turns the width into a height.
pub(crate) fn form_control_content_width(
    node: &Node,
    style: &ComputedStyle,
    metrics: CharMetrics,
    factor: u32,
) -> f32 {
    let size = f64::from(style.font_size);
    let avg = f64::from(metrics.avg_em) * size;
    let avg = avg.max(avg.round());
    let factor = f64::from(factor);
    let width = if node.tag() == Some("textarea") {
        let gutter = if matches!(
            style.overflow_y,
            OverflowValue::Hidden | OverflowValue::Clip
        ) {
            0.0
        } else {
            f64::from(TEXTAREA_SCROLLBAR_GUTTER)
        };
        ceil_px(avg * factor) + gutter
    } else {
        let max = (f64::from(metrics.max_em) * size).round();
        if max > 0.0 {
            // Chrome's `avg × factor + max − avg`, gathered so `size=1` is
            // `round(max)` exactly.
            ceil_px(avg * (factor - 1.0) + max)
        } else {
            ceil_px(avg * factor)
        }
    };
    (width as f32).min(MAX_CONTROL_WIDTH)
}

/// Chrome's final rounding of the intrinsic width: up to a whole pixel.
fn ceil_px(v: f64) -> f64 {
    v.ceil().max(0.0)
}

/// [`char_metrics`] for `node`'s style, from the node's cache when its font
/// family, weight and style have not moved since.
fn cached_char_metrics(
    node: &Node,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<peniko::Brush>,
    perf: &crate::perf::PerfCounters,
) -> CharMetrics {
    use std::hash::{Hash, Hasher};
    let style = &node.computed_style;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    style.font_family.hash(&mut h);
    style.font_weight.to_bits().hash(&mut h);
    (style.font_style as u8).hash(&mut h);
    let key = h.finish();
    if let Some((k, m)) = node.form_char_metrics.get()
        && k == key
    {
        return m;
    }
    perf.bump(crate::perf::Counter::ShapeFormControlMetrics);
    let m = char_metrics(font_cx, layout_cx, style);
    node.form_char_metrics.set(Some((key, m)));
    m
}

/// The text-control metrics of `style`'s **primary font** — the face its
/// font stack resolves a `0` to, at its weight and style, which is the first
/// available family for any stack whose first family has digits.
pub(crate) fn char_metrics(
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<peniko::Brush>,
    style: &ComputedStyle,
) -> CharMetrics {
    use skrifa::raw::TableProvider;
    const PROBE: &str = "0";
    const PROBE_PX: f32 = 16.0;
    let family = if style.font_family.is_empty() {
        "sans-serif".to_string()
    } else {
        style.font_family.clone()
    };
    let mut builder = layout_cx.ranged_builder(font_cx, PROBE, 1.0, true);
    builder.push_default(parley::style::StyleProperty::FontSize(PROBE_PX));
    builder.push_default(parley::style::StyleProperty::FontFamily(
        parley::style::FontFamily::Source(std::borrow::Cow::Owned(family)),
    ));
    if (style.font_weight - 400.0).abs() > 1.0 {
        builder.push_default(parley::style::StyleProperty::FontWeight(
            parley::style::FontWeight::new(style.font_weight),
        ));
    }
    if style.font_style != crate::computed_style::FontStyleValue::Normal {
        builder.push_default(parley::style::StyleProperty::FontStyle(
            style.font_style.to_parley(),
        ));
    }
    let mut layout = builder.build(PROBE);
    layout.break_all_lines(None);
    let run = layout.lines().next().and_then(|line| {
        line.items().find_map(|item| match item {
            parley::layout::PositionedLayoutItem::GlyphRun(run) => Some(run),
            _ => None,
        })
    });
    let Some(run) = run else {
        return CharMetrics {
            avg_em: 0.0,
            max_em: 0.0,
        };
    };
    let zero_em = run.glyphs().next().map_or(0.0, |g| g.advance) / PROBE_PX;
    let font = run.run().font();
    let Ok(face) = skrifa::FontRef::from_index(font.data.as_ref(), font.index) else {
        return CharMetrics {
            avg_em: zero_em,
            max_em: 0.0,
        };
    };
    let Some(head) = face.head().ok().filter(|h| h.units_per_em() > 0) else {
        return CharMetrics {
            avg_em: zero_em,
            max_em: 0.0,
        };
    };
    let upem = f32::from(head.units_per_em());
    let avg = face.os2().ok().map_or(0, |t| t.x_avg_char_width());
    CharMetrics {
        avg_em: if avg > 0 {
            f32::from(avg) / upem
        } else {
            zero_em
        },
        max_em: (f32::from(head.x_max()) - f32::from(head.x_min())).max(0.0) / upem,
    }
}
