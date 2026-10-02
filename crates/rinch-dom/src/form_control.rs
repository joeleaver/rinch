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
//! gutter — at a device scale factor of 1, where Chrome's device-px arithmetic
//! and rinch's CSS-px arithmetic agree. See [`form_control_content_width`] for
//! the formula, the averages Chrome does not trust, and what it does not
//! model. A face registered after layout re-sizes the controls
//! (`RinchDocument::note_fonts_registered`). The measure answers it as the content-box width wherever the
//! width is intrinsic — inline, a flex-row item, inside an `inline-block` —
//! and an author `width` (or `max-width`) wins as for any box. A control's
//! value never sizes it. One difference is left: a `display: block` control
//! with an `auto` width keeps its intrinsic width in Chrome (248px at 16px
//! Inter); rinch now matches it (#1195): [`crate::replaced::is_unstretched_replaced`]
//! marks every line-sized control's Taffy style `item_is_replaced`, the same
//! flag an `<img>` carries, so a block container no longer stretches it
//! (CSS 2.1 §10.3.4).
//!
//! **Every other `<input>` type [`input_width_factor`] does not size has an
//! intrinsic width of its own in Chrome 153 too** (#1195):
//! [`picker_input_content_width`] sizes the date/time family and `file` from
//! a representative string shaped in the control's own font plus a small
//! font-size-only chrome term (an icon this crate does not paint; review of
//! #1302 — the font-family blindness a pure font-size fit had), and
//! [`button_label_content_width`] sizes `submit`/`reset`/`button` to their
//! label text the same way. Both cache their shaped width on the node
//! ([`cached_label_width`]), the way [`cached_char_metrics`] already does
//! for the `size`/`cols` path, so a restyle that moves neither the label nor
//! the font reshapes nothing. `checkbox`, `radio`, `range`, `color`, `image`
//! and `hidden` are untouched — not line-sized controls at all
//! ([`form_control_content_height`] already excludes them), and their own
//! intrinsic sizes (13x13, 16, 27, the image's) stay unmodelled.

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
/// and the `head` table's `xMax - xMin` (`max_em`), or — for a face whose
/// average Chrome does not trust — the advance of its `0` and no extent. What
/// Chrome sizes a control from (`SimpleFontData::AvgCharWidth` /
/// `MaxCharWidth`, or the width of `0`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CharMetrics {
    /// The average character width, per em.
    pub(crate) avg_em: f32,
    /// The widest glyph extent (`xMax - xMin`), per em; 0 when the average is
    /// the `0` fallback, which Chrome sizes with no extent term.
    pub(crate) max_em: f32,
    /// Whether `avg_em` is the OS/2 average (`true`) or the `0` fallback.
    /// Chrome rounds only the former (`max(avg, round(avg))`).
    pub(crate) from_os2: bool,
}

/// The largest `xAvgCharWidth`, as a multiple of the face's `0` advance,
/// that Chrome 153 trusts. A CJK face declares its full-width ideograph as its
/// average (Noto Sans CJK: 979 units against a `0` of 555), and Chrome sizes a
/// control in such a face from its `0` instead. Measured by rewriting the
/// OS/2 value and nothing else: Noto Sans CJK is trusted at 942 (1.697 × 555)
/// and not at 944 (1.701), the bundled Inter at 2190 (1.695 × 1292) and not at
/// 2200 (1.703), at every font size. Nothing else about the face matters:
/// removing its ideographs, or adding `水` and full-width forms to a Latin
/// face, moves nothing.
const MAX_TRUSTED_AVG_PER_ZERO: f32 = 1.7;

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
    let want =
        form_control_content_size(node, font_cx, layout_cx, tree.font_generation, &tree.perf);
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
/// `paint_input_value` draws the value. An **element** child of either is
/// `display: none !important` by the UA sheet and the control is a hollow root
/// whatever its children are, so a block, atomic-inline or out-of-flow child
/// is not laid out or painted either (#1178).
pub fn is_value_control(node: &Node) -> bool {
    matches!(node.tag(), Some("input" | "textarea"))
}

/// The text a field holds: its `value` attribute, else a `<textarea>`'s
/// **child text content** (HTML: the concatenation of its direct `Text`
/// children — its default value), else `None`. Any element with a `value`
/// attribute answers it, so the desktop shell reads every field through this,
/// custom `data-oninput` controls included.
///
/// rinch keeps a control's live value in its `value` attribute, and for a
/// `<textarea>` the attribute's **presence is the dirty value flag**: the
/// shell writes it at the user's first edit, an app writes it with
/// `value_fn`, and focusing or blurring the field writes nothing (#1186). So
/// an attribute that is present wins even when it is empty — once a field has
/// been edited or written, its children no longer show — and while it is
/// absent the field follows its children, as a browser's `.value` does.
/// Removing the attribute is a write of `""` and leaves the field dirty
/// ([`Node::value_dirty`], #1222): it answers `""`, not its children. An
/// `<input>` has no default value in its children, so it answers only its
/// attribute. `None` for any other element.
pub fn control_value(
    nodes: &slab::Slab<Node>,
    node_id: usize,
) -> Option<std::borrow::Cow<'_, str>> {
    let node = nodes.get(node_id)?;
    if let Some(v) = node.attributes.get("value") {
        return Some(std::borrow::Cow::Borrowed(v.as_str()));
    }
    if node.tag() != Some("textarea") {
        return None;
    }
    if node.value_dirty {
        // Its `value` was removed: an empty dirty field (#1222).
        return Some(std::borrow::Cow::Borrowed(""));
    }
    Some(textarea_child_text(nodes, node))
}

/// Whether `node` is a `<textarea>` whose value is still its default value —
/// no `value` attribute and never had one removed, so its dirty value flag is
/// clear (#1186, #1222).
pub fn is_pristine_textarea(node: &Node) -> bool {
    node.tag() == Some("textarea") && !node.value_dirty && !node.attributes.contains_key("value")
}

/// A `<textarea>`'s child text content: its direct `Text` children,
/// concatenated.
fn textarea_child_text<'a>(nodes: &'a slab::Slab<Node>, node: &Node) -> std::borrow::Cow<'a, str> {
    use std::borrow::Cow;
    let mut texts = node
        .children
        .iter()
        .filter_map(|&c| nodes.get(c))
        .filter_map(|c| c.text_content());
    let Some(first) = texts.next() else {
        return Cow::Borrowed("");
    };
    match texts.next() {
        None => Cow::Borrowed(first),
        Some(second) => {
            let mut all = String::from(first);
            all.push_str(second);
            texts.for_each(|t| all.push_str(t));
            Cow::Owned(all)
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
    font_generation: u64,
    perf: &crate::perf::PerfCounters,
) -> Option<NodeContext> {
    form_control_content_size(node, font_cx, layout_cx, font_generation, perf).map(
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
    font_generation: u64,
    perf: &crate::perf::PerfCounters,
) -> Option<(f32, f32)> {
    let height = form_control_content_height(node)?;
    let width = form_control_content_width_any(node, font_cx, layout_cx, font_generation, perf);
    Some((width, height))
}

/// The content-box width of any line-sized control
/// ([`form_control_content_height`]): dispatches to whichever of the three
/// width rules this module implements applies — a text-like `size`/`cols`
/// factor times an average character ([`form_control_content_width`]), a
/// date/time-family or `file` picker's representative-string shape
/// ([`picker_input_content_width`]), or a button's label text
/// ([`button_label_content_width`]) — and 0 for the six types
/// [`form_control_content_height`] excludes (never reached: it already
/// answered `None` for them) and `hidden` (reached, but a `hidden` input is
/// `display: none` in the UA sheet and never a line-sized control in Chrome
/// either; #1195 does not change it).
fn form_control_content_width_any(
    node: &Node,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<peniko::Brush>,
    font_generation: u64,
    perf: &crate::perf::PerfCounters,
) -> f32 {
    if node.tag() == Some("textarea") {
        let factor = size_attribute(node, "cols");
        let metrics = cached_char_metrics(node, font_cx, layout_cx, font_generation, perf);
        return form_control_content_width(node, &node.computed_style, metrics, factor);
    }
    if node.tag() != Some("input") {
        return 0.0;
    }
    if let Some(factor) = input_width_factor(node) {
        let metrics = cached_char_metrics(node, font_cx, layout_cx, font_generation, perf);
        return form_control_content_width(node, &node.computed_style, metrics, factor);
    }
    let ty = node
        .attributes
        .get("type")
        .map(|t| t.trim().to_ascii_lowercase());
    match ty.as_deref() {
        Some(t @ ("date" | "month" | "week" | "time" | "datetime-local" | "file")) => {
            picker_input_content_width(node, t, font_cx, layout_cx, font_generation, perf)
        }
        Some(t @ ("submit" | "reset" | "button")) => {
            button_label_content_width(node, t, font_cx, layout_cx, font_generation, perf)
        }
        _ => 0.0,
    }
}

/// The content-box width of a date/time-family `<input>` or a `file` input
/// (#1195; this formula replaced by the review of #1302 — see below).
/// Chrome 153 paints each with its own control: a calendar/clock icon plus a
/// locale date/time format string for the first five, a "Choose File"
/// button plus a "No file chosen" label for `file`.
///
/// **The icon/chrome scales with font-size alone; the text scales with the
/// actual font.** Rewriting this crate's bundled Inter's OS/2
/// `xAvgCharWidth` (same trick [`form_control_content_width`]'s own tests
/// use) changes **nothing** about these widths in Chrome 153 — proof that
/// Chrome does not size a picker from a statistical average the way it sizes
/// a text `<input>` (#1177) — while swapping the font-*family* for a
/// genuinely different face (this crate's bundled Space Grotesk; the system
/// `serif`/`sans-serif`/`monospace` generics) moves them by up to 19px at
/// 16px. So the original fit here (a pure font-size affine, calibrated only
/// against the bundled Inter) was silently wrong for every other
/// font-family — including rinch's own default theme font, which is the
/// generic `sans-serif`, not Inter.
///
/// This now shapes a representative string per type — [`REPRESENTATIVE`] —
/// in the control's own font (cached by [`cached_label_width`], the same way
/// [`button_label_content_width`] below does), and adds a much smaller
/// **chrome** term that *is* font-size-only (the icon and internal UA
/// padding, which measurably do not depend on the glyph shapes of whatever
/// text is shown): `round(a + b × font-size + shape(string))`. Fit by
/// subtracting [`cached_label_width`]'s own (bundled-Inter) shape from each
/// Chrome 153 measurement at 8–40px, then least-squares over the residual
/// against font-size; worst residual ~1px over the sweep for **Inter, the
/// font this was calibrated against** — about the same residual the old
/// pure-affine fit had, so no accuracy was given up there.
///
/// **A different font now moves the answer, but is not pixel-exact.**
/// Measured against real Chrome 153 (a fresh page per measurement, this
/// crate's bundled Space Grotesk loaded through `@font-face`, 16px): `date`
/// is 143 in Chrome and 147 here (+4px), `week` is 154 in Chrome and 151
/// here (−3px) — the representative-string guess shapes close to, but not
/// exactly as, whatever Chrome's own `-webkit-datetime-edit` shadow tree
/// actually lays out in a face that isn't the calibration one. The system
/// `serif`/`sans-serif`/`monospace` generics are worse — up to several tens
/// of pixels off at `file` — because a generic name's own font-fallback
/// resolution is a separate, unmodelled problem on top of the
/// representative-string guess (this crate's own fallback choice for a bare
/// generic name is not guaranteed to be the same physical font Chrome
/// picks). None of this is pixel-exact for anything but the bundled Inter;
/// what changed from the font-size-only fit is that a different font now
/// moves the number *at all*, not that it moves to the right one.
///
/// ```text
/// date           = round(10.7594 + 2.3112 × font-size + shape("mm/dd/yyyy"))
/// time           = round(20.5196 + 1.3435 × font-size + shape("--:-- AM"))
/// datetime-local = round(16.2609 + 2.9022 × font-size + shape("mm/dd/yyyy --:-- AM"))
/// month          = round( 8.6714 + 5.6581 × font-size + shape("mm/yyyy"))
/// week           = round( 8.7432 + 2.7155 × font-size + shape("Week -- , ----"))
/// file           = round( 0.7128 + 8.7705 × font-size + shape("Choose File No file chosen"))
/// ```
///
/// Not a derivation of Chrome's own layout, and the representative strings
/// are a guess at what Chrome's internal `-webkit-datetime-edit` shadow tree
/// paints (locale-dependent; this crate has no locale support and assumes
/// `en-US`), checked only by how well they explain the Chrome 153 sweep
/// above — not by reading Chromium's source or its shadow DOM. Two things
/// this does not model, both measured rather than estimated: Chrome lays the
/// date/time family out as several separate editable fields with their own
/// letter-spacing edges, where this shapes one string, so `letter-spacing`
/// (which this *does* read, via [`cached_label_width`] — it used to be
/// silently dropped) under-counts the number of spacing gaps relative to
/// Chrome — `letter-spacing: 2px` at 16px Inter: `month` is 201 in Chrome
/// and 183 here (−18px), `file` 412 and 396 (−16px); and the per-type
/// `a`/`b` chrome terms are fit against Inter alone, so they carry whatever
/// Inter-specific residual the representative-string guess left behind,
/// same as the text-width ceiling does for `<input>` (#1177). `ty` is
/// already lowercased.
fn picker_input_content_width(
    node: &Node,
    ty: &str,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<peniko::Brush>,
    font_generation: u64,
    perf: &crate::perf::PerfCounters,
) -> f32 {
    let Some((a, b, s)) = REPRESENTATIVE
        .iter()
        .find_map(|&(t, a, b, s)| (t == ty).then_some((a, b, s)))
    else {
        return 0.0;
    };
    let chrome = a + b * f64::from(node.computed_style.font_size);
    let shaped = cached_label_width(node, s, font_cx, layout_cx, font_generation, perf);
    (chrome + f64::from(shaped)).round().max(0.0) as f32
}

/// `(type, a, b, representative string)` for [`picker_input_content_width`].
const REPRESENTATIVE: &[(&str, f64, f64, &str)] = &[
    ("date", 10.7594, 2.3112, "mm/dd/yyyy"),
    ("time", 20.5196, 1.3435, "--:-- AM"),
    ("datetime-local", 16.2609, 2.9022, "mm/dd/yyyy --:-- AM"),
    ("month", 8.6714, 5.6581, "mm/yyyy"),
    ("week", 8.7432, 2.7155, "Week -- , ----"),
    ("file", 0.7128, 8.7705, "Choose File No file chosen"),
];

/// The content-box width of a `submit` / `reset` / `button` `<input>`
/// (#1195): the width of its label, shaped in the control's own font —
/// unlike the picker types above, such a button has **no** chrome of its own
/// once the author zeroes its padding and border, so Chrome 153 sizes it to
/// its label text alone (`padding: 0; border: 0`, measured over every font
/// size from 8px to 40px).
///
/// HTML: the label is the `value` attribute when one is present — even an
/// empty one, so `value=""` is a 0-wide button in Chrome, not its localized
/// default — else `"Submit"` for `submit`, `"Reset"` for `reset`, and
/// nothing for a bare `button` (Chrome 153: 0 wide with no value). `ty` is
/// already lowercased.
///
/// Rounded, because every node's final layout is (`taffy::round_layout`,
/// called by `compute_layout_with_measure`): leaving this fractional would
/// still come out whole-pixel at `node.layout.width`, just rounded against
/// the node's rounded *position* too (`round(x + width) - round(x)`) rather
/// than against zero here. Rounding here keeps the contract self-contained.
/// This crate's text shaping is close to Chrome's but not bit-identical —
/// "Submit" at 16px Inter shapes to 52.648438 here against Chrome's
/// 52.65625 — and both round to the same whole pixel (53) at every font size
/// this module's fixtures sample; a font size where the two straddle a
/// `.5` boundary and round to different pixels is not known to exist but is
/// not ruled out either.
fn button_label_content_width(
    node: &Node,
    ty: &str,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<peniko::Brush>,
    font_generation: u64,
    perf: &crate::perf::PerfCounters,
) -> f32 {
    use std::borrow::Cow;
    let label: Cow<'_, str> = match node.attributes.get("value") {
        Some(v) => Cow::Borrowed(v.as_str()),
        None => match ty {
            "submit" => Cow::Borrowed("Submit"),
            "reset" => Cow::Borrowed("Reset"),
            _ => Cow::Borrowed(""),
        },
    };
    if label.is_empty() {
        return 0.0;
    }
    cached_label_width(node, &label, font_cx, layout_cx, font_generation, perf).round()
}

/// [`label_text_width`] from [`Node::form_label_width`], re-shaping only when
/// the hash of what the label is shaped from — the label text itself, the
/// font generation (a face registered after layout re-sizes the control,
/// same as [`cached_char_metrics`]), family, weight, style, letter-spacing
/// and word-spacing — has moved since the control was last sized (review of
/// #1302). A restyle that moves none of them shapes nothing: the sibling gap
/// [`cached_char_metrics`] closed for `size`/`cols` controls under #1177,
/// left open here until now — `button_label_content_width` and
/// `picker_input_content_width` used to call [`label_text_width`] on every
/// cascade of the control, measured costing ~5µs/node/restyle at 2000
/// `<input type=submit>` nodes.
fn cached_label_width(
    node: &Node,
    label: &str,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<peniko::Brush>,
    font_generation: u64,
    perf: &crate::perf::PerfCounters,
) -> f32 {
    use std::hash::{Hash, Hasher};
    let style = &node.computed_style;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    font_generation.hash(&mut h);
    label.hash(&mut h);
    style.font_family.hash(&mut h);
    style.font_size.to_bits().hash(&mut h);
    style.font_weight.to_bits().hash(&mut h);
    (style.font_style as u8).hash(&mut h);
    style.letter_spacing.to_bits().hash(&mut h);
    style.word_spacing.to_bits().hash(&mut h);
    let key = h.finish();
    if let Some((k, w)) = node.form_label_width.get()
        && k == key
    {
        return w;
    }
    perf.bump(crate::perf::Counter::ShapeFormControlLabel);
    let w = label_text_width(node, label, font_cx, layout_cx);
    node.form_label_width.set(Some((key, w)));
    w
}

/// The width of `label` shaped in `node`'s own font, **unquantized** — the
/// shape [`crate::select::select_label_layout`] does not have: that builder
/// quantizes (snaps glyph positions to the pixel grid) because it is shared
/// with paint, where that is wanted, and a quantized width can be a few
/// tenths of a pixel off the unquantized one (measured: "Submit" at 16px
/// Inter is 52.648438 unquantized, 53 quantized — which happens to be where
/// the unquantized width rounds to anyway, but is not the same number).
/// Reads `letter-spacing`/`word-spacing` the way `ifc.rs`'s
/// `push_spacing` does — unconditionally, since 0 is parley's own default —
/// so a representative multi-word string (`file`'s) or an author's
/// letter-spacing on a button label is not silently dropped.
fn label_text_width(
    node: &Node,
    label: &str,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<peniko::Brush>,
) -> f32 {
    let style = &node.computed_style;
    let font_family = crate::fonts::parley_text_family(font_cx, &style.font_family, label);
    let mut builder = layout_cx.ranged_builder(font_cx, label, 1.0, false);
    builder.push_default(parley::style::StyleProperty::FontSize(style.font_size));
    font_family.push_to(&mut builder);
    if (style.font_weight - 400.0).abs() > 1.0 {
        builder.push_default(parley::style::StyleProperty::FontWeight(
            parley::style::FontWeight::new(style.font_weight),
        ));
    }
    builder.push_default(parley::style::StyleProperty::LetterSpacing(
        style.letter_spacing,
    ));
    builder.push_default(parley::style::StyleProperty::WordSpacing(
        style.word_spacing,
    ));
    let mut layout: parley::Layout<peniko::Brush> = builder.build(label);
    layout.break_all_lines(None);
    layout.full_width()
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
/// which this does not model). The date/time family, `file`, and the three
/// buttons `None` here are sized by [`form_control_content_width_any`]'s
/// other two rules instead ([`picker_input_content_width`],
/// [`button_label_content_width`], #1195); `hidden` and the five
/// [`form_control_content_height`] leaves out besides it are never reached —
/// that function already answered `None` for them.
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
/// a single-line control. A face whose average Chrome distrusts
/// ([`MAX_TRUSTED_AVG_PER_ZERO`]) is sized from its `0` advance, unrounded and
/// with no extent term: `ceil(zero × size)`, `ceil(zero × cols) + 15`.
///
/// **At a device scale factor of 1.** Chrome runs the same formula in device
/// pixels and divides back, so at 1.5 or 2 its widths differ from these by
/// the rounding of each step (measured up to 11px at 1.5: Noto Serif 11px
/// `cols=33`, 235.33 in Chrome where this gives 246); rinch computes in CSS px
/// whatever the scale. And "to the pixel" was sampled at whole and half pixel
/// font sizes: a sweep in 0.1px steps finds a few sizes one pixel off (3 of
/// 846 cases — DejaVu Serif textarea at 19.8px, Liberation Sans input at 17.8
/// and 19.8px).
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
    let avg = if metrics.from_os2 {
        avg.max(avg.round())
    } else {
        avg
    };
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
/// family, weight and style have not moved since, and no face has been
/// registered on the document since (`font_generation`,
/// `RinchDocument::note_fonts_registered`).
fn cached_char_metrics(
    node: &Node,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<peniko::Brush>,
    font_generation: u64,
    perf: &crate::perf::PerfCounters,
) -> CharMetrics {
    use std::hash::{Hash, Hasher};
    let style = &node.computed_style;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    font_generation.hash(&mut h);
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

/// The text-control metrics of `style`'s **primary font**: the face its stack
/// resolves a Latin `x` to, at its weight and style.
///
/// An `x`, not the `0` a control's width is about: parley sends a digit (it
/// has the Unicode `Emoji` property) to the `emoji` generic whenever nothing
/// in the stack covers it, which `fonts::parley_font_family` now prevents for
/// a stack that resolves to nothing (#1198) but not for one whose only face
/// lacks digits. An `x` is in no emoji face. The `0` advance is then read from
/// the chosen face itself.
pub(crate) fn char_metrics(
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<peniko::Brush>,
    style: &ComputedStyle,
) -> CharMetrics {
    use skrifa::MetadataProvider;
    use skrifa::raw::TableProvider;
    const PROBE: &str = "x";
    const UNKNOWN: CharMetrics = CharMetrics {
        avg_em: 0.0,
        max_em: 0.0,
        from_os2: false,
    };
    let family = crate::fonts::parley_text_family(font_cx, &style.font_family, PROBE);
    let mut builder = layout_cx.ranged_builder(font_cx, PROBE, 1.0, true);
    builder.push_default(parley::style::StyleProperty::FontSize(16.0));
    family.push_to(&mut builder);
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
        return UNKNOWN;
    };
    let font = run.run().font();
    let Ok(face) = skrifa::FontRef::from_index(font.data.as_ref(), font.index) else {
        return UNKNOWN;
    };
    let Some(head) = face.head().ok().filter(|h| h.units_per_em() > 0) else {
        return UNKNOWN;
    };
    let upem = f32::from(head.units_per_em());
    let zero = face
        .charmap()
        .map('0')
        .and_then(|gid| {
            face.glyph_metrics(
                skrifa::instance::Size::unscaled(),
                skrifa::instance::LocationRef::default(),
            )
            .advance_width(gid)
        })
        .unwrap_or(0.0);
    let zero_fallback = CharMetrics {
        avg_em: zero / upem,
        max_em: 0.0,
        from_os2: false,
    };
    let avg = f32::from(face.os2().ok().map_or(0, |t| t.x_avg_char_width()));
    if avg <= 0.0 || (zero > 0.0 && avg > MAX_TRUSTED_AVG_PER_ZERO * zero) {
        return zero_fallback;
    }
    CharMetrics {
        avg_em: avg / upem,
        max_em: (f32::from(head.x_max()) - f32::from(head.x_min())).max(0.0) / upem,
        from_os2: true,
    }
}
