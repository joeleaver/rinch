//! The [`DomLayout`] supertrait of [`DomDocument`](super::DomDocument):
//! resolving layout, node boxes, sizes and insets, fonts and line boxes, the
//! flat text offsets of generated and substituted text, and the caret, glyph,
//! selection-rectangle and line-box queries the editor draws from.

use super::types::{CaretAffinity, GeneratedFlatRange, GlyphBounds, NodeFont};
use crate::dom::NodeId;

/// Layout results and text geometry.
///
/// One of the five supertraits [`DomDocument`](super::DomDocument) is made
/// of; see that trait for how they fit together.
pub trait DomLayout {
    /// Query the screen position of a text caret at the given byte offset.
    ///
    /// Returns the (x, y) coordinates where a text cursor would be rendered
    /// at the specified byte offset within the text node.
    ///
    /// **Relative to the node's border box** — the box
    /// [`query_node_layout`](Self::query_node_layout) reports — so a padded
    /// or bordered block's caret includes that padding and border (#1667).
    /// The browser backend measures from `getBoundingClientRect`; rinch-dom
    /// adds the offset at which it draws the Parley layout. The same frame
    /// for [`Self::query_glyph_bounds`], [`Self::query_selection_rects`],
    /// [`Self::query_inline_box_caret`] and
    /// [`Self::query_caret_position_with_affinity`].
    ///
    /// # Arguments
    /// * `node_id` - The ID of the text node or element containing text
    /// * `byte_offset` - The UTF-8 byte offset within the text content
    ///
    /// # Returns
    /// Some((x, y)) if the node has text layout and the offset is valid, None otherwise
    fn query_caret_position(&self, node_id: u64, byte_offset: usize) -> Option<(f32, f32)>;

    /// Query the bounding box of a glyph cluster at the given byte offset.
    ///
    /// Returns the bounding box of the glyph cluster containing the specified
    /// byte offset within the text node, relative to the node's border box
    /// (the frame of [`Self::query_caret_position`]).
    ///
    /// # Arguments
    /// * `node_id` - The ID of the text node or element containing text
    /// * `byte_offset` - The UTF-8 byte offset within the text content
    ///
    /// # Returns
    /// Some(GlyphBounds) if the node has text layout and the offset is valid, None otherwise
    fn query_glyph_bounds(&self, node_id: u64, byte_offset: usize) -> Option<GlyphBounds>;

    /// Resolve layout for the document at the given viewport size.
    ///
    /// This computes Taffy layout and builds text layouts (IFC) for all dirty nodes.
    /// Must be called before querying caret positions or glyph bounds.
    ///
    /// # Arguments
    /// * `width` - Viewport width in pixels
    /// * `height` - Viewport height in pixels
    fn resolve_layout(&mut self, width: f32, height: f32);

    /// Query the layout bounds of a node relative to its parent.
    ///
    /// Returns the (x, y, width, height) of the node's layout box.
    fn query_node_layout(&self, node_id: u64) -> Option<(f32, f32, f32, f32)>;

    /// The box element `node` is **painted** in, in viewport-relative logical
    /// pixels: the frame of `ClickContext::element_x` and of
    /// [`NodeHandle::bounds_signal`](crate::dom::NodeHandle::bounds_signal), so a transform on the element or an
    /// ancestor moves and scales it (issue #1779). The browser backend answers
    /// `getBoundingClientRect()`; rinch-dom answers the last layout's painted
    /// border box (it does not lay out on demand).
    ///
    /// A flowed inline element (`<span>`, `<a>`) answers the union of its line
    /// fragments, as `getBoundingClientRect()` does. `None` for a text node,
    /// an unknown node, or one not in the document. A `Some` zero rect for an
    /// element under `display: none` (itself or an ancestor) or one that is
    /// `display: contents` itself, as Chrome answers. `&mut` because rinch-dom
    /// may shape a font probe to measure an inline's fragments.
    /// Default `None`: a host with no geometry (the mock document).
    fn bounding_rect(&mut self, _node: NodeId) -> Option<crate::reactive::ElementBounds> {
        None
    }

    /// `node`'s border-box size as laid out, before any CSS transform on it
    /// or an ancestor scales it (the web's `offsetWidth`/`offsetHeight`), in
    /// logical pixels (#2154). `None` where the host has no layout (the
    /// default) or the node is gone.
    fn layout_size(&self, _node: NodeId) -> Option<(f32, f32)> {
        None
    }

    /// `node`'s border box as laid out, before any CSS transform on it or an
    /// ancestor, in the frame of `container`'s **padding box**: the `left`,
    /// `top`, `width` and `height` that put an absolutely positioned child of
    /// `container` (a positioned box, so its containing block) over it. In
    /// logical pixels, unscrolled: an absolute child of a box that scrolls is
    /// placed against its scrolled content, as `node` is. Desktop sums the
    /// layout; the web sums `offsetLeft`/`offsetTop` up the `offsetParent`
    /// chain to `container`.
    ///
    /// A sortable list places the slot of an open gap with it (#2154): the
    /// items around the gap carry a sliding `transform`, which a painted box
    /// ([`Self::bounding_rect`]) includes mid-slide and this does not.
    ///
    /// **`None` unless `container` is on `node`'s `offsetParent` chain**, on
    /// every host (CSSOM View's `offsetParent`, followed up): each step is
    /// the nearest ancestor that is a containing block for absolutely
    /// positioned boxes, the `<body>`, or, for a box that is not positioned
    /// itself, a `td`, `th` or `table`. Chrome 158 and Firefox 160 agree on
    /// what makes one: `position` other than `static`; a `transform`,
    /// `filter`, `backdrop-filter` or `perspective` other than `none` (an
    /// identity or `blur(0)` included); `contain: paint`, `layout`, `strict`
    /// or `content`; and `will-change` naming `transform`, `filter`,
    /// `backdrop-filter`, `perspective`, `contain` or `position`. Not
    /// `contain: size`/`style`, `container-type`, `opacity`, `overflow`,
    /// `isolation`, `clip-path`, `mix-blend-mode` or `z-index`. (The engines
    /// also count `translate`, `rotate`, `scale`, `offset-path`,
    /// `transform-style: preserve-3d` and `content-visibility`, which desktop
    /// does not implement.) So `None` when `container` is none of those (a
    /// list given `position: static`), when `node` or a box between is
    /// `position: fixed`, when `node` is not rendered (`display: none` on it
    /// or an ancestor, `display: contents` on it), when there is no layout,
    /// and for a host with no geometry (the default).
    ///
    /// **Whole pixels on both hosts**, rounded as the browser rounds
    /// `offsetLeft`: each step's offset from its `offsetParent`'s padding
    /// edge on its own, and the width and height their own lengths. Desktop
    /// rounds from its unrounded layout, half up (`floor(x + 0.5)`), which is
    /// Chrome's rounding; Firefox rounds a negative tie away from zero
    /// (-8.5 to -9 where Chrome and desktop answer -8), so there the hosts
    /// differ by a pixel. A box desktop places after its layout engine (an
    /// absolute whose containing block is not its parent, an atomic inline)
    /// or one with no layout node of its own (an inline element) is measured
    /// from its laid-out, already-rounded box. No browser API gives the
    /// untransformed box fractionally: `getBoundingClientRect` includes
    /// transforms.
    fn layout_box_in(
        &self,
        _node: NodeId,
        _container: NodeId,
    ) -> Option<crate::reactive::ElementBounds> {
        None
    }

    /// The viewport's size in logical pixels — the box a `position: fixed;
    /// inset: 0` element fills, and the frame [`Self::bounding_rect`] answers
    /// in (issue #1764). rinch-dom answers the size of its last layout
    /// (`None` before the first); the browser backend answers
    /// `documentElement.clientWidth/clientHeight` (the viewport less any
    /// classic scrollbar). Default `None`: a host with no geometry.
    fn viewport_size(&self) -> Option<(f32, f32)> {
        None
    }

    /// [`NodeHandle::bounds_signal`](crate::dom::NodeHandle::bounds_signal) was called for `node` (issue #1781) —
    /// on **every** call, so a backend must ignore a node it already watches.
    /// A backend that does not refresh bounds after its own layout passes — the
    /// web, where the browser lays out — starts watching the element here.
    /// Called with the document mutably borrowed: it must not run app code.
    /// Default: nothing (rinch-dom refreshes every registered node after each
    /// layout).
    fn watch_bounds(&mut self, _node: NodeId) {}

    /// The `(left, top)` inset from a node's box origin (the origin
    /// [`query_node_layout`](Self::query_node_layout) reports against) to the origin an
    /// absolutely-positioned child anchors to. On the web that is the node's border
    /// width (`clientLeft`/`clientTop`): CSS positions an `absolute` child against the
    /// padding box, while `getBoundingClientRect` differences report against the border
    /// box, so the editor's overlay container offset must subtract this once. The
    /// desktop renderer is the same (its layout origins are border-box-relative, an
    /// absolute child's insets padding-box-relative) and answers the border widths
    /// too. An absolute child of a box that scrolls sits against its *scrolled*
    /// content, so the web answers its border less its own `scrollLeft`/
    /// `scrollTop` (#1677), while the desktop's parent-relative layouts are
    /// unscrolled already. Default `(0, 0)`, for a backend with no borders to
    /// report.
    fn content_origin_inset(&self, _node_id: u64) -> (f32, f32) {
        (0.0, 0.0)
    }

    /// A node's padding plus border on each side, `(left, top, right,
    /// bottom)`: from its border box (the box
    /// [`query_node_layout`](Self::query_node_layout) reports) in to its
    /// content box, where its text starts. The editor places the caret in an
    /// empty textblock (no text to ask [`Self::query_caret_position`] about)
    /// at the block's content origin (#1667). Default zero, for a backend
    /// with no box model to report.
    fn content_box_inset(&self, _node_id: u64) -> (f32, f32, f32, f32) {
        (0.0, 0.0, 0.0, 0.0)
    }

    /// The height of one line box of `node_id`: its computed `line-height` in
    /// px. The editor draws the caret in an empty textblock this tall (#1667):
    /// its box need not be one line plus its padding (desktop sizes
    /// `min-height: 1lh` as a border box, #1278). Default `None` (the caller
    /// falls back to the block's content height).
    fn line_box_height(&self, _node_id: u64) -> Option<f32> {
        None
    }

    /// The resolved font of `node_id` ([`NodeFont`]), so the IME preedit overlay can
    /// match the text it composes into (e.g. a 32px heading vs. 16px body). Default
    /// `None` — the overlay keeps its inherited font. Backends that can read computed
    /// styles (the browser's `getComputedStyle`) override this.
    fn node_font(&self, _node_id: u64) -> Option<NodeFont> {
        None
    }

    /// How many bytes a `<br>` element occupies in the flat byte offsets this
    /// backend's text queries take and its hit tests answer
    /// ([`Self::query_caret_position`], [`Self::query_caret_rect`],
    /// [`Self::query_selection_rects`], and the byte a pointer hit resolves to).
    ///
    /// A flat byte offset counts the text of the text nodes under the element, in
    /// document order, as the backend lays that text out — a host may lay a
    /// character out as something of another length ([`Self::tab_flat_bytes`],
    /// [`Self::substituted_char_flat_bytes`]). A
    /// `<br>` has no text node, so a backend that counts only
    /// text gives it none — the browser's DOM ranges, and this default — while one
    /// that lays inline content out as a single string may give it the line break
    /// it lays out: rinch-dom's inline formatting context pushes `"\n"`, one byte.
    /// A caller mapping its own model onto flat offsets (the rich-text editor's
    /// view) asks this so a caret after a break lands after it (#1099).
    fn line_break_flat_bytes(&self) -> usize {
        0
    }

    /// How many bytes a tab (`'\t'`) in a text node occupies in the same flat
    /// byte offsets as [`Self::line_break_flat_bytes`]. Its UTF-8 length, `1`, by
    /// default — the browser's DOM ranges count the character, and rinch-dom
    /// lays a preserved tab out as one space widened to its tab stop (#1122).
    /// A host that laid a tab out as several characters would answer their
    /// bytes, so the rich-text editor's caret map counts each tab as that many
    /// (#1109: rinch-dom laid a tab out as four spaces until #1122).
    ///
    /// The answer is per host, not per node, and it holds only where
    /// `white-space` preserves the tab (`pre`, `pre-wrap`, `break-spaces`);
    /// under `normal`, `nowrap` and `pre-line` a tab collapses like a space.
    /// The editor's stylesheet sets `pre-wrap` on its container and its code
    /// blocks, which is why the editor may use it.
    fn tab_flat_bytes(&self) -> usize {
        1
    }

    /// The characters of a text node under preserved `white-space` that this
    /// backend lays out as something of another length, and the bytes each
    /// then occupies in the flat byte offsets of [`Self::tab_flat_bytes`]
    /// (#1181). None by default — the browser's DOM ranges count the
    /// characters themselves. rinch-dom hands parley a substitute for U+2028,
    /// U+2029, U+0085 and U+000C (its `ifc::PRESERVED_SUBSTITUTES`, the same
    /// table its layout reads), so the rich-text editor's caret map counts
    /// each as that many bytes. Like a tab's, the answer holds only where
    /// `white-space` preserves the text, which the editor's does.
    fn substituted_char_flat_bytes(&self) -> &'static [(char, usize)] {
        &[]
    }

    /// The stretches of `node_id`'s flat byte offsets (the offsets every text
    /// query here takes) that hold CSS generated content — the text of a
    /// `::before` / `::after` — sorted and disjoint (#891). Such text is laid
    /// out among the element's own but is not in its DOM, so a caller that
    /// counts the DOM's (or a model's) text steps over these bytes. None by
    /// default — the browser's DOM ranges never count generated content.
    /// rinch-dom lays it out in the inline formatting context, and answers
    /// each pseudo-element's text there, and the bidi control characters it
    /// puts around an element with a `unicode-bidi` other than `normal`
    /// (#2588): an opening one with `after` false, a closing one with `after`.
    fn generated_flat_ranges(&self, _node_id: u64) -> Vec<GeneratedFlatRange> {
        Vec::new()
    }

    /// The caret beside the atomic inline `box_id` (an `<img>`) that the
    /// text-bearing element `node_id` lays out in its inline layout, as
    /// `(x, y, height)`, border-box-local like [`Self::query_caret_position`]:
    /// on the box's leading edge, or its trailing edge when `after`, at the top
    /// of its line and one line high.
    ///
    /// A flat byte offset cannot name this side: an atomic inline occupies no
    /// bytes of the flat text, so the offsets on its two sides are one number,
    /// and so are those around two such boxes in a row. A caller that knows
    /// which box a caret is beside (the rich-text editor's view, for an image)
    /// asks this instead (#1104). `None` when the box is not in `node_id`'s
    /// inline layout; the caller then asks by byte. Default `None`, so a host
    /// that does not implement it (the mock) is asked by byte as before. The
    /// browser backend answers it for a `<br>` only, which has no bytes there
    /// either (the start of the line it ends, when text or another `<br>`
    /// follows, #1172), and asks an image by byte.
    fn query_inline_box_caret(
        &self,
        _node_id: u64,
        _box_id: u64,
        _after: bool,
    ) -> Option<(f32, f32, f32)> {
        None
    }

    /// The caret on one side of one edge of the inline element `element` (a
    /// `<code>`, a `<mark>`) in the text-bearing element `node_id`, as
    /// [`Self::query_inline_box_caret`] answers for an atomic inline: its
    /// inline-start margin, border and padding (`end == false`) or its
    /// inline-end ones (`end == true`), on their trailing side with `after`
    /// (inside the element at its start; outside it after its end) or their
    /// leading side (outside before its start; inside at its end), at the top
    /// of the line and one line high.
    ///
    /// Those take room on their lines (#1426) but hold no flat bytes, so the
    /// offsets on either side of an edge are one number, and a byte's caret
    /// is drawn before every edge at that byte. A caller that knows which
    /// side of an element the caret is on (the rich-text editor's view, from
    /// the marks the caret carries) asks this. `None` when the element takes
    /// no room at that edge or is not in `node_id`'s inline layout; the
    /// caller then asks by byte. Default `None`: the browser backend draws
    /// its own caret, and the mock has no geometry.
    ///
    /// rinch-dom counts the element's own generated text on that edge (its
    /// `::after` at the end, its `::before` at the start) as part of it
    /// (#891): the inner side is the caret on the element's side of that
    /// text. The outer side is past any padding; with none, the end edge's
    /// outer side is before the `::after` text too (Chrome's caret after a
    /// link with an arrow), and the start edge's is before the `::before`
    /// text.
    fn query_inline_edge_caret(
        &self,
        _node_id: u64,
        _element: u64,
        _end: bool,
        _after: bool,
    ) -> Option<(f32, f32, f32)> {
        None
    }

    /// [`Self::query_inline_edge_caret`] **on screen**, in the frame of
    /// [`Self::query_caret_rect`]. Default `None`.
    fn query_inline_edge_caret_rect(
        &self,
        _node_id: u64,
        _element: u64,
        _end: bool,
        _after: bool,
    ) -> Option<(f32, f32, f32)> {
        None
    }

    /// [`Self::query_inline_box_caret`] **on screen**, in the frame of
    /// [`Self::query_caret_rect`]. Default `None`.
    fn query_inline_box_caret_rect(
        &self,
        _node_id: u64,
        _box_id: u64,
        _after: bool,
    ) -> Option<(f32, f32, f32)> {
        None
    }

    /// Selection rectangles `(x, y, width, height)`, relative to the node's
    /// border box (the frame of [`Self::query_caret_position`]), covering the
    /// byte range `[a, b)`: on rinch-dom one per line, or one per visual piece
    /// of a line holding a right-to-left run (#1114); on the web the browser's
    /// `Range.getClientRects` boxes. Used to render a
    /// text selection's highlight. Default: empty (no inline layout / mock).
    fn query_selection_rects(
        &self,
        _node_id: u64,
        _byte_a: usize,
        _byte_b: usize,
    ) -> Vec<(f32, f32, f32, f32)> {
        Vec::new()
    }

    /// [`Self::query_selection_rects`] for a selection that names the
    /// zero-byte inline leaves it holds (images; on a host whose `<br>` has no
    /// bytes, hard breaks too): `selected_boxes` lists every selected one by
    /// its host element id (as for [`Self::query_inline_box_caret`]), and no
    /// other is selected.
    ///
    /// Such a leaf has no bytes, so the byte range cannot say whether one at
    /// `byte_a` or `byte_b` is inside the selection, nor whether one at
    /// `byte_a == byte_b` is selected at all (#1120). Default: the byte range
    /// alone ([`Self::query_selection_rects`]). rinch-dom and the browser
    /// backend answer it.
    fn query_selection_rects_with_boxes(
        &self,
        node_id: u64,
        byte_a: usize,
        byte_b: usize,
        selected_boxes: &[u64],
    ) -> Vec<(f32, f32, f32, f32)> {
        let _ = selected_boxes;
        self.query_selection_rects(node_id, byte_a, byte_b)
    }

    /// Where a text caret at `byte_offset` inside the text-bearing element
    /// `node_id` is **on screen**, as `(x, y, height)`: its top, in the frame an
    /// app positions a popup in. Logical window pixels on desktop (the frame of
    /// [`NodeHandle::bounds_signal`](crate::dom::NodeHandle::bounds_signal) and of a `position: fixed` element), viewport
    /// client pixels in the browser (`getBoundingClientRect`).
    ///
    /// An element with no text to measure (an empty paragraph) answers its own
    /// box's origin, one line high, which is where an editor paints the caret on
    /// a blank line. `None` when the node is unknown or has not been laid out.
    ///
    /// At a soft line wrap it answers the **downstream** caret, at the start of
    /// the lower line, on both backends; see
    /// [`Self::query_caret_rect_with_affinity`] for the other side.
    ///
    /// Backs `EditorHandle::caret_rect`. Default `None`: a host with no geometry
    /// (the mock document) has no screen to answer for.
    fn query_caret_rect(&self, _node_id: u64, _byte_offset: usize) -> Option<(f32, f32, f32)> {
        None
    }

    /// [`Self::query_caret_rect`] for a caret with `affinity` at a soft wrap
    /// (#301): an `Upstream` caret there is drawn at the end of the upper line,
    /// a `Downstream` one at the start of the lower. Anywhere else it is the same
    /// answer. Default: the affinity-blind [`Self::query_caret_rect`].
    fn query_caret_rect_with_affinity(
        &self,
        node_id: u64,
        byte_offset: usize,
        _affinity: CaretAffinity,
    ) -> Option<(f32, f32, f32)> {
        self.query_caret_rect(node_id, byte_offset)
    }

    /// [`Self::query_caret_position`] for a caret with `affinity` at a soft
    /// wrap (#301), border-box-local like it. Default: the affinity-blind
    /// [`Self::query_caret_position`].
    fn query_caret_position_with_affinity(
        &self,
        node_id: u64,
        byte_offset: usize,
        _affinity: CaretAffinity,
    ) -> Option<(f32, f32)> {
        self.query_caret_position(node_id, byte_offset)
    }

    /// The caret a text editor draws at `byte_offset` in text-bearing element
    /// `node_id` (with `affinity` at a soft wrap), as `(x, top, height)`,
    /// border-box-local like [`Self::query_caret_position`]: at that caret's
    /// x, over the content area of the text it stands beside (its font's
    /// ascent above the baseline to its descent below), as Chrome draws a
    /// caret, rather than over the whole line box. Under a `line-height`
    /// taller than the font the caret is shorter than the line and lower than
    /// its top. The rich-text editor's caret overlay is placed from it.
    ///
    /// Default: [`Self::query_caret_position_with_affinity`]'s point, as tall
    /// as [`Self::query_glyph_bounds`] there (18 without a glyph), for a host
    /// whose answers already are the browser's caret (rinch-web).
    fn query_caret_box_with_affinity(
        &self,
        node_id: u64,
        byte_offset: usize,
        affinity: CaretAffinity,
    ) -> Option<(f32, f32, f32)> {
        let (x, y) = self.query_caret_position_with_affinity(node_id, byte_offset, affinity)?;
        let height = self
            .query_glyph_bounds(node_id, byte_offset)
            .map_or(18.0, |g| g.height);
        Some((x, y, height))
    }

    /// The line box of the line a caret at `byte_offset` in text-bearing
    /// element `node_id` is drawn on (with `affinity` at a soft wrap), as
    /// `(top, height)`, border-box-local like
    /// [`Self::query_caret_position`]: the whole line, its leading included,
    /// not the glyphs of the character there. The editor's margin anchors
    /// (`EditorHandle::set_margin_anchors`) report it. Default `None`: the
    /// editor then derives the line from the caret rect and the block's
    /// [`line_box_height`](Self::line_box_height).
    fn query_line_box(
        &self,
        _node_id: u64,
        _byte_offset: usize,
        _affinity: CaretAffinity,
    ) -> Option<(f32, f32)> {
        None
    }

    /// [`Self::query_line_box`] for several carets in one text-bearing
    /// element, in order — for a backend that reads the element's lines once
    /// rather than once per caret (the browser). Default: one query each.
    fn query_line_boxes(
        &self,
        node_id: u64,
        carets: &[(usize, CaretAffinity)],
    ) -> Vec<Option<(f32, f32)>> {
        carets
            .iter()
            .map(|&(byte, affinity)| self.query_line_box(node_id, byte, affinity))
            .collect()
    }

    /// Whether the document has changes its geometry does not show yet: a
    /// mutation or a style change made since the last
    /// [`resolve_layout`](Self::resolve_layout). A backend that lays out on
    /// demand (the browser) answers `false`, as does the default. The editor's
    /// margin pass waits for a layout while it is `true` rather than report
    /// lines that are about to move.
    fn layout_pending(&self) -> bool {
        false
    }

    /// A stamp of the text layout `node_id`'s lines are read from (the one
    /// [`Self::query_line_box`] reads), which changes whenever that layout is
    /// rebuilt — the one way its lines move with no box moving. The editor's
    /// margin pass skips measuring a block whose stamp and boxes are what they
    /// were. Default `None`: unknown, so only the boxes are compared (the
    /// browser, which lays out on demand).
    fn text_layout_generation(&self, _node_id: u64) -> Option<u64> {
        None
    }
}
