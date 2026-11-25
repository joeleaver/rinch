//! What paint and layout ask of a [`ComputedStyle`] beyond its fields: the
//! background layers' geometry and image URLs, resolved colours and line
//! height, and whether a filter is in force.

use super::*;

impl ComputedStyle {
    /// The first background layer's geometry (#1142): `background_size`,
    /// the two positions, the two repeats and `background_origin`.
    pub fn first_background_geometry(&self) -> BackgroundLayerGeometry {
        BackgroundLayerGeometry {
            size: self.background_size,
            position_x: self.background_position_x,
            position_y: self.background_position_y,
            repeat_x: self.background_repeat_x,
            repeat_y: self.background_repeat_y,
            origin: self.background_origin,
        }
    }

    /// Every `url()` among the background layers, first layer first (#1142).
    pub fn background_image_urls(&self) -> impl Iterator<Item = &str> {
        std::iter::once(&self.background)
            .chain(self.background_layers.iter().map(|l| &l.image))
            .filter_map(|b| match b {
                BackgroundValue::Image { url } => Some(url.as_str()),
                _ => None,
            })
    }

    /// Every `url()` paint loads for this style: the background layers'
    /// ([`Self::background_image_urls`]) and then the `mask-image` layers'
    /// (#2099).
    pub fn paint_image_urls(&self) -> impl Iterator<Item = &str> {
        self.background_image_urls()
            .chain(self.mask_image.iter().filter_map(|l| match &l.image {
                BackgroundValue::Image { url } => Some(url.as_str()),
                _ => None,
            }))
    }

    /// Get the background color (convenience accessor for BackgroundValue::Color).
    pub fn background_color(&self) -> Option<peniko::Color> {
        match &self.background {
            BackgroundValue::Color(c) => Some(*c),
            _ => None,
        }
    }

    /// Get the effective border color for a side (for backwards compat).
    pub fn border_color(&self) -> Option<peniko::Color> {
        self.border_top_color
    }

    /// Resolve `line-height` to pixels against this style's `font-size`.
    ///
    /// `normal` is the 1.2 factor rinch uses everywhere a line box has to be
    /// sized without Parley metrics (a text-entry form control's content
    /// height, `form_control.rs`).
    pub fn line_height_px(&self) -> f32 {
        match self.line_height {
            LineHeightValue::Normal => self.font_size * 1.2,
            LineHeightValue::Relative(r) => self.font_size * r,
            LineHeightValue::Absolute(px) => px,
        }
    }

    /// Whether `filter` or `backdrop-filter` is not `none` (#542, #2719): a
    /// box with any filter function, an identity one such as `brightness(1)`
    /// and a `blur()` included, is a stacking context and the containing
    /// block of its absolute and fixed descendants (Filter Effects 1 §5 and 2
    /// §3, measured in Chrome 157). The real computation behind
    /// [`crate::node::Node::has_filter`], which this cascade is the **only**
    /// caller of.
    ///
    /// Cascade-time only (#729 perf review): `Node::creates_stacking_context`
    /// is on the hit-test and paint hot paths — walked once per node per
    /// pointer move and per frame — so asking `ComputedStyle` there measured
    /// as a real regression (CI Perf: `shell::pointer_move_warm.warm_x50`
    /// +11.95%, `pointer_move_cold.cold` +9.76%, `hover_frame.partial_repaint`
    /// +3.56%, when it compared four filter scalars). Call this once, at the
    /// point a node's `ComputedStyle` is finalized (`style_resolution/mod.rs`'s
    /// `apply_stylo_styles_to_taffy`, and the pseudo-element cascade in
    /// `style_resolution/pseudo.rs`), and cache the answer in
    /// `Node::filter_creates_stacking_context`; nothing else should call this
    /// per-frame.
    pub fn has_filter(&self) -> bool {
        !self.filter.is_empty() || !self.backdrop_filter.is_empty()
    }

    /// Whether `backdrop-filter` changes the backdrop at all (#2719): a list
    /// of identity functions only — the component library's
    /// `blur(var(--rinch-modal-overlay-blur, 0))` above all — is still a
    /// stacking context ([`Self::has_filter`]) but draws nothing, so paint
    /// skips its backdrop entirely.
    pub fn has_backdrop_effect(&self) -> bool {
        self.backdrop_filter.iter().any(|f| !f.is_identity())
    }

    /// Whether the `filter` list has a function that moves pixels
    /// ([`FilterOp::moves_pixels`]: a `blur()` or a `drop-shadow()`), so paint
    /// draws the box through an offscreen raster (`paint::filter`, #2719)
    /// rather than the colour overlays.
    pub fn has_raster_filter(&self) -> bool {
        self.filter.iter().any(FilterOp::moves_pixels)
    }
}
