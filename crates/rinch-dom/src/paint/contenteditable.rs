//! `<input>`/`<textarea>` value rendering (text, placeholder, caret, selection,
//! and IME preedit), the read-only text-selection highlight (`user-select: text`),
//! plus the small style/px helpers they share.

use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, Rect};
use peniko::{Brush, Fill};

use super::painter::Painter;
use crate::node::Node;

use super::text::render_text_with_shadow;

/// The translucent wash both selection painters draw.
const SELECTION_WASH: (u8, u8, u8, u8) = (51, 154, 240, 100);

/// Paint the selection `a..b` (byte offsets, either order) over `layout`, whose
/// origin is at `(text_x, text_y)` and whose units are `units` device px each:
/// one rect per line (per visual piece on a line holding a right-to-left run,
/// #1114), from [`crate::text_query::selection_rects_for_layout`] —
/// the editor's rule, so each line's rect is its line box and ends where the
/// next line's begins, the line before a hard break carries a newline's width
/// and a soft-wrapped line runs to its trailing edge (#1106; #1008, #1010,
/// #1024). Two neighbours used to be derived here from the glyph top and a
/// fractional `line_height` (or `1.2 × font-size` in a `<textarea>`), so they
/// overlapped by a pixel — a darker row across a translucent highlight — or
/// left one unpainted. A line the selection carries on past is not extended to
/// the content box's edge: Chrome 155 ends it at its text (a soft wrap's
/// trailing space) or a newline's width, whether or not it is the last.
#[allow(clippy::too_many_arguments)]
fn paint_selection_rects(
    painter: &mut dyn Painter,
    transform: Affine,
    layout: &parley::layout::Layout<Brush>,
    a: usize,
    b: usize,
    text_x: f64,
    text_y: f64,
    units: f64,
) {
    let (r, g, bl, al) = SELECTION_WASH;
    let color = AlphaColor::<Srgb>::from_rgba8(r, g, bl, al);
    for (left, top, width, height) in crate::text_query::selection_rects_for_layout(layout, a, b) {
        let rect = Rect::new(
            text_x + left as f64 * units,
            text_y + top as f64 * units,
            text_x + (left + width) as f64 * units,
            text_y + (top + height) as f64 * units,
        );
        painter.fill_color(Fill::NonZero, transform, color, &rect.into());
    }
}

/// The **read-only** text selection (`user-select: text`) IFC root `node`
/// carries — `data-text-sel`, `-start`, `-end` — as byte offsets into its
/// laid-out text, clamped to it. `None` when it carries none, or an empty one.
pub(crate) fn read_only_selection(node: &Node) -> Option<(usize, usize)> {
    if !node.carries_text_selection() {
        return None;
    }
    let layout = node.text_layout.as_ref()?;
    let len = layout.text_content.len();
    let at = |k: &str| {
        node.attributes
            .get(k)
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0)
            .min(len)
    };
    let (a, b) = (at("data-text-sel-start"), at("data-text-sel-end"));
    (a != b).then_some((a, b))
}

/// How far `node`'s read-only selection highlight reaches past its border box,
/// in CSS px: `[left, top, right, bottom]`, never negative. Under a
/// `line-height` smaller than the text, each line's rect is its content area
/// (#1008), and the first and last stand out of the box above and below it, as
/// Chrome 155 paints them; the highlight is the node's own paint, so its damage
/// has to reach that far, or a partial frame leaves stale rows of wash (review
/// of #1706). Zero for a node with no selection.
pub(crate) fn read_only_selection_outsets(node: &Node) -> [f32; 4] {
    let mut o = [0.0_f32; 4];
    let (Some((a, b)), Some(layout)) = (read_only_selection(node), node.text_layout.as_ref())
    else {
        return o;
    };
    let (ox, oy) = super::ifc_root_content_origin(node);
    let (w, h) = (node.layout.width, node.layout.height);
    for (left, top, width, height) in
        crate::text_query::selection_rects_for_layout(&layout.layout, a, b)
    {
        o[0] = o[0].max(-(ox + left));
        o[1] = o[1].max(-(oy + top));
        o[2] = o[2].max(ox + left + width - w);
        o[3] = o[3].max(oy + top + height - h);
    }
    o
}

/// Paint the translucent selection highlight for a **read-only** text selection
/// (`user-select: text`, [`read_only_selection`]) over IFC root `node`'s inline
/// layout, whose content-box origin is at device px `(text_x, text_y)`. The
/// layout is in CSS px, so its rects are scaled by `scale` (they were drawn at
/// 1/scale on a HiDPI display). (The editor and `<input>` render their own
/// selections elsewhere; this is only for selecting static, non-editable text.)
pub(super) fn paint_text_selection_highlight(
    node: &Node,
    painter: &mut dyn Painter,
    scale: f64,
    text_x: f64,
    text_y: f64,
    transform: Affine,
) {
    let (Some((a, b)), Some(layout)) = (read_only_selection(node), node.text_layout.as_ref())
    else {
        return;
    };
    paint_selection_rects(
        painter,
        transform,
        &layout.layout,
        a,
        b,
        text_x,
        text_y,
        scale,
    );
}

/// Get a CSS style property value from inline styles.
///
/// Parse an inline style property from a node's style attribute.
/// Most properties should be read from `node.computed_style` directly.
///
/// Through `rinch_core::dom::split_declarations`, the workspace's one
/// inline-style parser (#670), so a `;` or `:` inside a quoted or bracketed
/// value is part of that value here too.
///
/// `property` is matched ASCII case-insensitively — unless it is a custom
/// property, which CSS compares exactly — through the same
/// [`rinch_core::dom::normalize_property_name`] the parser applies to the names
/// it reads out (#711).
///
/// That also changed what a **duplicate** answers: this used to return the
/// first matching declaration and now returns the **last**, because the parser
/// collapses a repeated property there. The new answer is the browser's —
/// measured in Chrome 150, `getPropertyValue` on
/// `user-select: none; user-select: text` is `"text"` — and no caller was
/// affected, since this function has none.
#[allow(dead_code)]
pub(super) fn get_style_property(node: &Node, property: &str) -> Option<String> {
    // `computed_style_str` (used during style resolution) if there is one;
    // otherwise the inline style attribute. An empty `computed_style_str` is
    // "not resolved yet", not "no such property", which is why the first arm
    // does not fall through to the second.
    let source = if !node.computed_style_str.is_empty() {
        &node.computed_style_str
    } else {
        node.attributes.get("style")?
    };
    let property = rinch_core::dom::normalize_property_name(property);
    rinch_core::dom::split_declarations(source)
        .into_iter()
        .find(|(key, _)| key.as_str() == &*property)
        .map(|(_, value)| value)
}

/// Parse a pixel value like "10px" or "10" to f32.
#[allow(dead_code)]
pub(super) fn parse_px(value: &str) -> Option<f32> {
    let v = value.trim().strip_suffix("px").unwrap_or(value.trim());
    v.parse().ok()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn paint_input_value(
    node: &Node,
    value: &str,
    perf: &crate::perf::PerfCounters,
    painter: &mut dyn Painter,
    scale: f64,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<Brush>,
    transform: Affine,
    scroll: (f32, f32),
) {
    // `value` is `form_control::control_value`: the `value` attribute, else a
    // textarea's text children (#1159), which nothing else draws.
    let placeholder = node
        .attributes
        .get("placeholder")
        .map(|s| s.as_str())
        .unwrap_or("");

    // Check if this input is focused
    let is_focused = node
        .attributes
        .get("data-focused")
        .map(|s| s == "true")
        .unwrap_or(false);
    let cursor_pos = node
        .attributes
        .get("data-cursor-pos")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    let selection_start = node
        .attributes
        .get("data-selection-start")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(cursor_pos);
    let cursor_visible = node
        .attributes
        .get("data-cursor-visible")
        .map(|s| s == "true")
        .unwrap_or(true);

    let is_password = node.attributes.get("type").map(|s| s.as_str()) == Some("password");

    // IME composition (preedit): the in-progress composition string lives only in
    // `data-preedit` (written by the runtime during composition) — never in the
    // field's `value`. Splice it into the displayed text at the caret so it flows
    // inline, underline that run, and put the caret at its end. Composition takes
    // precedence over the placeholder (composing into an empty field shows the
    // composition, not the placeholder).
    let preedit = node
        .attributes
        .get("data-preedit")
        .map(|s| s.as_str())
        .unwrap_or("");
    let composing = is_focused && !preedit.is_empty() && !is_password;
    let composed_text;
    let (text, is_placeholder, cursor_pos, selection_start, preedit_range) = if composing {
        let c = cursor_pos.min(value.len());
        composed_text = format!("{}{}{}", &value[..c], preedit, &value[c..]);
        let end = c + preedit.len();
        (composed_text.as_str(), false, end, end, Some((c, end)))
    } else if value.is_empty() && !placeholder.is_empty() {
        (placeholder, true, cursor_pos, selection_start, None)
    } else {
        (value, false, cursor_pos, selection_start, None)
    };

    // Password masking: replace display text with bullets for type="password"
    let password_display;
    let (text, cursor_pos, selection_start) = if is_password && !is_placeholder && !text.is_empty()
    {
        let bullet = "\u{2022}";
        let bullet_len = bullet.len();
        let total_chars = value.chars().count();
        let cursor_chars = value[..cursor_pos.min(value.len())].chars().count();
        let sel_chars = value[..selection_start.min(value.len())].chars().count();
        password_display = bullet.repeat(total_chars);
        (
            password_display.as_str(),
            cursor_chars * bullet_len,
            sel_chars * bullet_len,
        )
    } else {
        (text, cursor_pos, selection_start)
    };

    let font_size = node.computed_style.font_size;

    // Get text color from computed style - dimmed for placeholder
    let base_color = node
        .computed_style
        .color
        .unwrap_or_else(|| AlphaColor::<Srgb>::from_rgba8(33, 37, 41, 255)); // #212529

    let color = if is_placeholder {
        // Placeholder uses dimmed color
        AlphaColor::<Srgb>::from_rgba8(134, 142, 150, 255) // #868e96
    } else {
        base_color
    };

    let is_textarea = node.tag() == Some("textarea");
    let scaled_font_size = font_size * scale as f32;

    // The one builder and the one origin the hit-test, caret-rect and IME
    // paths use (`form_control::field_text_layout` / `field_text_origin`,
    // #827): built at the scaled font size, so its geometry is device px.
    perf.bump(crate::perf::Counter::ShapePaint);
    let text_layout = crate::form_control::field_text_layout(
        node,
        text,
        scale as f32,
        Some(Brush::Solid(color)),
        font_cx,
        layout_cx,
    );
    let (origin_x, origin_y) =
        crate::form_control::field_text_origin(node, text_layout.height() / scale as f32, scroll);
    let text_x = x + origin_x as f64 * scale;
    let text_y = y + origin_y as f64 * scale;

    // A text control's text, selection, caret and composition underline are
    // clipped as Chrome's inner editor clips them, whenever they could reach
    // past it. A single-line field: to its content box horizontally (its
    // padding box vertically) once its line overflows the column or is
    // scrolled (#1721). A textarea, a scroll container: to its padding box
    // once its text is taller than the content box or scrolled (#827). Text
    // that fits needs no clip.
    let style = &node.computed_style;
    let (column_left, column_width) = crate::form_control::text_column(node);
    let column_left = column_left as f64 * scale;
    let column_width = column_width as f64 * scale;
    let top = style.border_top_width.to_px() as f64 * scale;
    let bottom = style.border_bottom_width.to_px() as f64 * scale;
    let clip = if is_textarea {
        let left = style.border_left_width.to_px() as f64 * scale;
        let right = style.border_right_width.to_px() as f64 * scale;
        let pad_top = style.padding_top.to_px() as f64 * scale;
        let pad_bottom = style.padding_bottom.to_px() as f64 * scale;
        let content_h = h - top - bottom - pad_top - pad_bottom;
        (scroll.1 > 0.0 || text_layout.height() as f64 > content_h).then(|| {
            Rect::new(
                x + left,
                y + top,
                (x + w - right).max(x + left),
                (y + h - bottom).max(y + top),
            )
        })
    } else {
        (scroll.0 > 0.0
            || text_layout.full_width() as f64 + crate::form_control::CARET_WIDTH as f64 * scale
                > column_width)
            .then(|| {
                Rect::new(
                    x + column_left,
                    y + top,
                    x + column_left + column_width,
                    (y + h - bottom).max(y + top),
                )
            })
    };
    if let Some(clip) = clip {
        painter.push_clip(Fill::NonZero, transform, &clip.into());
    }

    // Draw selection highlight if there's a selection
    if is_focused && cursor_pos != selection_start && !is_placeholder && !text.is_empty() {
        let sel_start_byte = cursor_pos.min(selection_start).min(text.len());
        let sel_end_byte = cursor_pos.max(selection_start).min(text.len());

        paint_selection_rects(
            painter,
            transform,
            &text_layout,
            sel_start_byte,
            sel_end_byte,
            text_x,
            text_y,
            // The control's layout is built at the scaled font size: already
            // device px.
            1.0,
        );
    }

    // Render text (layout already uses scaled_font_size, so pass scale=1.0)
    if !text.is_empty() {
        let text_shadows = node.computed_style.text_shadow.as_slice();
        render_text_with_shadow(
            painter,
            &text_layout,
            text_x,
            text_y,
            text_shadows,
            transform,
            1.0,
            None,
            None,
            None,
        );
    }

    // Draw cursor/caret if focused and visible
    if is_focused && cursor_visible {
        let caret_pos = cursor_pos.min(text.len());
        // An upstream caret at a soft wrap is drawn at the end of the upper
        // line (#941). It is the committed text's caret: a composition moves
        // the caret to the preedit's end, which nothing set an affinity for.
        let affinity = if composing || is_placeholder {
            rinch_core::dom::CaretAffinity::Downstream
        } else {
            crate::form_control::caret_affinity(node)
        };
        let (caret_offset_x, caret_offset_y) = if text.is_empty() {
            (0.0, 0.0)
        } else {
            crate::text_query::caret_position_for_offset_layout_with_affinity(
                &text_layout,
                caret_pos,
                affinity,
            )
        };
        let caret_x = text_x + caret_offset_x as f64;
        let caret_y = text_y + caret_offset_y as f64;

        let caret_height = scaled_font_size as f64 * 1.2;

        // Draw caret line
        let caret_color = base_color;
        let caret_rect = Rect::new(
            caret_x,
            caret_y,
            caret_x + 1.5 * scale,
            caret_y + caret_height,
        );
        painter.fill_color(Fill::NonZero, transform, caret_color, &caret_rect.into());
    }

    // Underline the IME composition run so it reads as an in-progress composition.
    if let Some((a, b)) = preedit_range {
        let a = a.min(text.len());
        let b = b.min(text.len());
        let (start_x, start_y) =
            crate::text_query::caret_position_for_offset_layout(&text_layout, a);
        let (end_x, _end_y) = crate::text_query::caret_position_for_offset_layout(&text_layout, b);
        let line_height = scaled_font_size as f64 * 1.2;
        let underline_y = text_y + start_y as f64 + line_height - scale;
        let ul_rect = Rect::new(
            text_x + start_x as f64,
            underline_y,
            text_x + end_x as f64,
            underline_y + scale,
        );
        painter.fill_color(Fill::NonZero, transform, base_color, &ul_rect.into());
    }

    if clip.is_some() {
        painter.pop_layer();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_px() {
        assert_eq!(parse_px("10px"), Some(10.0));
        assert_eq!(parse_px("10"), Some(10.0));
        assert_eq!(parse_px("0"), Some(0.0));
        assert_eq!(parse_px("abc"), None);
    }

    /// A `;` or `:` inside a quoted or bracketed value is part of that value,
    /// here as everywhere else (#670): `get_style_property` reads through
    /// `rinch_core::dom::split_declarations` rather than its own `split(';')`.
    ///
    /// The `padding` is what discriminates. Splitting on a bare `;` cuts the
    /// `url()` in two and leaves `base64,AAA=) no-repeat` as a "part" with no
    /// `:` — which is then skipped, so the *following* declaration is still
    /// found and a fixture that only asked for `padding` would pass against
    /// the naive reader. The `background` assertion is the one that fails.
    #[test]
    fn get_style_property_does_not_split_inside_a_url() {
        let guard = style::shared_lock::SharedRwLock::new();
        let mut node = Node::element(0, "div", guard);
        node.attributes.insert(
            "style".to_string(),
            "background: url(data:image/png;base64,AAA=) no-repeat; padding: 4px".to_string(),
        );

        assert_eq!(
            get_style_property(&node, "background").as_deref(),
            Some("url(data:image/png;base64,AAA=) no-repeat")
        );
        assert_eq!(get_style_property(&node, "padding").as_deref(), Some("4px"));
        assert_eq!(get_style_property(&node, "color"), None);
    }

    /// The name the caller asks for is matched ASCII case-insensitively, like
    /// CSSOM's `getPropertyValue` — but a custom property is matched exactly
    /// (#711). Both halves are needed: an unconditional lowercase passes the
    /// first and fails the second.
    ///
    /// Kills the mutant that drops `normalize_property_name` from
    /// `get_style_property`, and the one that applies it to every name.
    #[test]
    fn get_style_property_matches_a_name_the_way_css_compares_it() {
        let guard = style::shared_lock::SharedRwLock::new();
        let mut node = Node::element(0, "div", guard);
        node.attributes.insert(
            "style".to_string(),
            "USER-SELECT: none; --Foo: 1px; --foo: 2px".to_string(),
        );

        assert_eq!(
            get_style_property(&node, "user-select").as_deref(),
            Some("none")
        );
        assert_eq!(
            get_style_property(&node, "USER-select").as_deref(),
            Some("none")
        );
        assert_eq!(get_style_property(&node, "--Foo").as_deref(), Some("1px"));
        assert_eq!(get_style_property(&node, "--foo").as_deref(), Some("2px"));
    }

    /// A property declared twice answers its **last** value, which is CSSOM's
    /// rule — measured in Chrome 150, `getPropertyValue` on
    /// `user-select: none; user-select: text` is `"text"`. The pre-#670 reader
    /// returned the first match; this is what says which one it is now (#706
    /// review, F3).
    #[test]
    fn get_style_property_answers_the_last_of_a_duplicate() {
        let guard = style::shared_lock::SharedRwLock::new();
        let mut node = Node::element(0, "div", guard);
        node.attributes.insert(
            "style".to_string(),
            "user-select: none; gap: 4px; user-select: text".to_string(),
        );
        assert_eq!(
            get_style_property(&node, "user-select").as_deref(),
            Some("text")
        );
    }

    /// `computed_style_str` wins over the attribute when there is one, and an
    /// empty one means "not resolved yet" rather than "no such property" — so
    /// the attribute is read in that case and not before.
    #[test]
    fn get_style_property_prefers_the_resolved_string() {
        let guard = style::shared_lock::SharedRwLock::new();
        let mut node = Node::element(0, "div", guard);
        node.attributes
            .insert("style".to_string(), "color: red".to_string());
        assert_eq!(get_style_property(&node, "color").as_deref(), Some("red"));

        node.computed_style_str = "color: blue".to_string();
        assert_eq!(get_style_property(&node, "color").as_deref(), Some("blue"));
        assert_eq!(
            get_style_property(&node, "padding"),
            None,
            "a resolved string that lacks the property answers None rather \
             than falling back to the attribute"
        );
    }
}
