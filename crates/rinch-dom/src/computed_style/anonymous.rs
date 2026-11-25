//! The style of an anonymous block box ([`ComputedStyle::for_anonymous_box`]):
//! what crosses over from its parent, and what deliberately does not.

use super::*;

impl ComputedStyle {
    /// The style of an anonymous block box: only what CSS **inherits** crosses
    /// over from the parent (CSS 2.1 §9.2.1.1), plus `display: block`.
    ///
    /// The box model deliberately does **not** cross (#319). An anonymous box
    /// used to take the parent's entire computed style while its Taffy style
    /// stayed `Default::default()`, so layout saw zero padding and paint saw
    /// the parent's — every consumer of the clone's padding/border then
    /// double-counted the parent's box model: paint's IFC arm displaced the
    /// whole inline layout, `ifc_content_box_offset` displaced hit testing and
    /// caret placement to match, `build_inline_layout`'s `max_width`
    /// re-subtracted a padding Taffy had already taken out, and the cloned
    /// `background`/`border` decorations were painted a second time at the
    /// anonymous box's own rect. Starting from the default style and copying
    /// only the inherited set makes all of them read `(0, 0)` box model — a
    /// field added to `ComputedStyle` later is *not* carried over unless
    /// someone decides it should be, which fails toward the spec rather than
    /// back toward the phantom box model.
    ///
    /// Three entries are worth their own note:
    /// - `display` is `block` — definitionally, not the struct default
    ///   (`Flex`) — **except** for a box minted under a `display: contents`
    ///   wrapper, which keeps `contents`. The wrapper generates no box for an
    ///   anonymous box to live inside; the box stands in the wrapper's
    ///   flattened position, and every display-first scan
    ///   (`scan_contents_children`, `mark_inline_descendants`,
    ///   `sync_display_contents`) must keep treating that position as
    ///   transparent to the surrounding IFC. The full clone supplied this by
    ///   accident; `ifc_leaf_invariant_tests`' display-none fixtures pin it.
    /// - `text_decoration` is **not** inherited, but decorations propagate to
    ///   all in-flow descendants, anonymous boxes included (CSS Text
    ///   Decoration §2), and `build_inline_layout` seeds the root text style
    ///   from the IFC root's value — dropping it would strip the underline
    ///   from text the parent decorates.
    /// - `user_select` is not inherited either; its initial `auto` defers to
    ///   the parent, which this engine has no `auto`-chasing for, so copying
    ///   the parent's value is what `auto` would have computed to.
    pub(crate) fn for_anonymous_box(parent: &ComputedStyle) -> Self {
        Self {
            display: if parent.display == DisplayValue::Contents {
                DisplayValue::Contents
            } else {
                DisplayValue::Block
            },
            has_explicit_display: true,

            // The inherited set, in struct order.
            scrollbar_color: parent.scrollbar_color,
            color: parent.color,
            visibility: parent.visibility,
            text_shadow: parent.text_shadow.clone(),
            text_paint: parent.text_paint,
            cursor: parent.cursor,
            pointer_events: parent.pointer_events,
            user_select: parent.user_select,
            font_size: parent.font_size,
            font_weight: parent.font_weight,
            font_family: parent.font_family.clone(),
            font_style: parent.font_style,
            font_synthesis_weight: parent.font_synthesis_weight,
            line_height: parent.line_height,
            letter_spacing: parent.letter_spacing,
            word_spacing: parent.word_spacing,
            text_align: parent.text_align,
            html_align: parent.html_align,
            text_decoration: parent.text_decoration.clone(),
            text_transform: parent.text_transform,
            text_underline_offset: parent.text_underline_offset,
            white_space: parent.white_space,
            overflow_wrap: parent.overflow_wrap,
            tab_size: parent.tab_size,
            word_break: parent.word_break,
            word_break_break_word: parent.word_break_break_word,
            hyphens_none: parent.hyphens_none,
            line_break: parent.line_break,
            text_indent: parent.text_indent,

            ..Self::default()
        }
    }
}
