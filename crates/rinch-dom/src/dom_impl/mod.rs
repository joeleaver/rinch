//! DomDocument implementation for rinch-dom.

use peniko::Brush;
use servo_arc::Arc as ServoArc;

// Stylo CSS engine imports
use euclid::Scale;
use style::context::QuirksMode;
use style::font_metrics::FontMetrics;
use style::media_queries::{Device, MediaType};
use style::properties::style_structs::Font as StyloFont;
use style::properties::{ComputedValues, PropertyDeclarationBlock};
use style::queries::values::PrefersColorScheme;
use style::stylist::Stylist;
use style::values::computed::font::GenericFontFamily;
use style::values::computed::{CSSPixelLength, Length};
use style::values::specified::font::QueryFontMetricsFlags;
use stylo_config as style_config;
// CSSPixel and DevicePixel are used via euclid::Size2D type parameters

use crate::node::{DirtyFlags, NodeTree};

mod dom_document_impl;

/// A simple FontMetricsProvider that returns default/fixed values.
/// This is used by the Stylist's Device to resolve font-relative units.
#[derive(Debug)]
pub(crate) struct SimpleFontMetricsProvider;

impl style::servo::media_queries::FontMetricsProvider for SimpleFontMetricsProvider {
    fn query_font_metrics(
        &self,
        _vertical: bool,
        _font: &StyloFont,
        _base_size: CSSPixelLength,
        _flags: QueryFontMetricsFlags,
    ) -> FontMetrics {
        // Return sensible defaults - these will be used for font-relative units
        // like ex, ch, cap, ic when we don't have actual font metrics
        FontMetrics::default()
    }

    fn base_size_for_generic(&self, _generic: GenericFontFamily) -> Length {
        // Default base font size (16px for most generics)
        Length::new(16.0)
    }
}

/// Parameters baked into the Stylo [`Device`] that must survive its rebuild.
///
/// `set_stylist_viewport` reconstructs the whole `Device` on every viewport
/// change, so any value written directly onto a `Device` is silently reset to
/// its default by the first window resize. Anything that has to persist lives
/// here instead, and [`build_device`] re-applies it on every construction
/// (issues #279, #211).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DeviceParams {
    /// The computed font-size of the root (`<html>`) element in CSS px — the
    /// basis every `rem` length resolves against (#279). Kept in sync by
    /// `sync_root_font_size` whenever the root element is (re)cascaded.
    pub root_font_size: f32,
    /// Device pixel ratio. Drives the `resolution` media features,
    /// `image-set()` candidate selection, and border-width device-pixel
    /// snapping (#211). Does not affect layout geometry: 1 CSS px stays
    /// 1 layout unit regardless.
    pub device_pixel_ratio: f32,
}

impl Default for DeviceParams {
    fn default() -> Self {
        Self {
            root_font_size: 16.0,
            device_pixel_ratio: 1.0,
        }
    }
}

/// Build a Stylo [`Device`] from the surviving [`DeviceParams`].
///
/// Both construction sites — [`RinchDocument::new`] and
/// `set_stylist_viewport` — must go through this, so a viewport-driven
/// rebuild cannot silently reset the root font-size or the device pixel
/// ratio back to their defaults (#279, #211).
pub(crate) fn build_device(width: f32, height: f32, params: &DeviceParams) -> Device {
    let viewport_size = euclid::Size2D::new(width, height);
    let device_pixel_ratio = Scale::new(params.device_pixel_ratio);
    let font_metrics_provider = Box::new(SimpleFontMetricsProvider);
    let default_font = StyloFont::initial_values();
    let default_computed_values = ComputedValues::initial_values_with_font_override(default_font);

    let device = Device::new(
        MediaType::screen(),
        QuirksMode::NoQuirks,
        viewport_size,
        device_pixel_ratio,
        font_metrics_provider,
        default_computed_values,
        PrefersColorScheme::Light,
    );
    // A fresh Device starts at the 16px initial — restore the root element's
    // font-size so a rebuild doesn't erase the `rem` basis.
    device.set_root_font_size(params.root_font_size);
    device
}

/// The primary document type for rinch-dom.
///
/// Implements [`DomDocument`] using a slab-allocated node tree.
/// In later phases, this will integrate Taffy for layout,
/// Parley for text, and Vello for painting.
pub struct RinchDocument {
    /// Process-unique document identity (see [`DomDocument::doc_key`]) — scopes
    /// per-node-id state in thread-local registries so two documents on one
    /// thread never collide (issue #134).
    pub(crate) doc_key: u64,
    /// The node tree.
    pub tree: NodeTree,
    /// Parley font context for text shaping.
    pub font_cx: parley::FontContext,
    /// Parley layout context for text measurement.
    pub layout_cx: parley::LayoutContext<Brush>,
    /// Stylo CSS engine stylist for CSS cascade and selector matching.
    pub stylist: Stylist,
    /// Device parameters that must survive the `Device` rebuild in
    /// `set_stylist_viewport` — see [`DeviceParams`].
    pub(crate) device_params: DeviceParams,
    /// The theme stylesheet, held in a stable slot before every app sheet so app
    /// CSS always cascades over it. Managed by `set_theme_css`.
    pub(crate) theme_stylesheet: Option<style::stylesheets::DocumentStyleSheet>,
    /// App author stylesheets, in insertion (source) order.
    pub(crate) author_stylesheets: Vec<style::stylesheets::DocumentStyleSheet>,
    /// Whether any loaded stylesheet has a rule whose rightmost compound is a
    /// bare focus pseudo-class (`:focus` / `:focus-visible` / `:focus-within`
    /// with no tag/class/id/attribute anchor) — e.g. the theme's
    /// `:focus-visible { outline: ... }`. Stylo buckets such rules into a
    /// state-gated `rare_pseudo_classes` map that is only consulted while the
    /// element ALREADY has focus state, so `focus_sensitive` can never be set
    /// on an unfocused node by them; focus changes must then invalidate
    /// unconditionally (see `node_is_focus_sensitive`). Recomputed whenever a
    /// stylesheet is loaded or the theme sheet is replaced.
    pub(crate) has_bare_focus_rules: bool,
}

impl Default for RinchDocument {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for RinchDocument {
    fn drop(&mut self) {
        // A torn-down document can never drain its queued image decodes —
        // purge them so they don't strand in the process-global pending
        // queue forever (issue #137).
        crate::image_cache::purge_pending(self.doc_key);
    }
}

impl RinchDocument {
    /// Process-unique document identity (see
    /// [`DomDocument::doc_key`](rinch_core::dom::DomDocument::doc_key)) —
    /// inherent accessor so callers holding a concrete `RinchDocument` don't
    /// need the trait in scope.
    pub fn doc_key(&self) -> u64 {
        self.doc_key
    }

    /// Create a new document with root and body nodes.
    pub fn new() -> Self {
        // Enable CSS Grid and text-overflow support in Stylo
        // This must be called before any CSS parsing happens
        style_config::set_bool("layout.grid.enabled", true);
        style_config::set_bool("layout.unimplemented", true);

        // Create the Stylo Device with default viewport and settings
        let device_params = DeviceParams::default();
        let device = build_device(800.0, 600.0, &device_params);

        let stylist = Stylist::new(device, QuirksMode::NoQuirks);

        let mut doc = Self {
            doc_key: rinch_core::dom::next_doc_key(),
            tree: NodeTree::new(),
            font_cx: crate::fonts::new_font_context(),
            layout_cx: parley::LayoutContext::new(),
            stylist,
            device_params,
            theme_stylesheet: None,
            author_stylesheets: Vec::new(),
            has_bare_focus_rules: false,
        };

        // Set up default file-based image loader
        doc.tree.image_loader = Some(std::sync::Arc::new(crate::image_cache::FileImageLoader));

        // Load User-Agent stylesheet with default display values for HTML elements
        doc.load_ua_stylesheet();

        doc
    }

    /// Load the User-Agent stylesheet with default display values for HTML elements.
    /// Without this, all elements default to display: inline in Stylo.
    fn load_ua_stylesheet(&mut self) {
        use style::media_queries::MediaList;
        use style::stylesheets::{
            AllowImportRules, DocumentStyleSheet, Origin, Stylesheet, UrlExtraData,
        };

        // Basic UA stylesheet defining block-level elements
        // Note: Stylo's initial border-width is 'medium' (3px), so we reset it to 0
        let ua_css = r#"
            * {
                border-width: 0;
            }

            html, body, div, section, article, aside, header, footer, main, nav,
            h1, h2, h3, h4, h5, h6, p, blockquote, pre, figure, figcaption,
            ul, ol, menu, dir, li, dl, dt, dd, table, form, fieldset, legend, hr,
            address, details, summary {
                display: block;
            }

            head, style, script, link, meta, title, noscript {
                display: none;
            }

            span, a, em, strong, b, i, u, s, sub, sup, small, mark, abbr, cite,
            code, kbd, samp, var, q, dfn, time, label, br, wbr {
                display: inline;
            }

            strong, b {
                font-weight: bold;
            }

            em, i {
                font-style: italic;
            }

            /* The heading scale, verbatim from the HTML Standard's rendering
               section and measured against Chrome 150 (issue #627). rinch used
               to name h1–h6 only in the `display: block` rule above, so a bare
               `<h1>` computed 16px/400/no-margin — body text — while `rinch-web`
               ran on the browser's own UA sheet and rendered a heading. That
               divergence is the defect; these are the browser's numbers.

               `em`, not px, at both sites and deliberately: the scale is
               relative to the *inherited* size (an `<h1>` in a 32px container is
               64px), and an `em` in `margin` resolves against the element's
               **own** computed font-size, so an author `font-size` moves the
               margins with it — `<h1 style="font-size: 12px">` gets 8.04px of
               margin in Chrome, not 21.44px. Both facts are invisible at the
               default root size, where 2em is exactly 32px and 0.67em of either
               font-size is exactly 21.44px; `ua_heading_typography_tests` samples
               off that fixed point.

               Cascade rules, never a post-cascade patch — #616/#618 deleted the
               tag fixup that used to stamp `font-weight` after the cascade,
               precisely because an author `font-weight: normal` could not beat
               it. An author declaration beats these, which is the #616
               handshake `an_author_declaration_beats_the_new_ua_heading_rules`
               pins. */
            h1 { font-size: 2em;    margin-block: 0.67em; }
            h2 { font-size: 1.5em;  margin-block: 0.83em; }
            h3 { font-size: 1.17em; margin-block: 1em; }
            h4 { font-size: 1em;    margin-block: 1.33em; }
            h5 { font-size: 0.83em; margin-block: 1.67em; }
            h6 { font-size: 0.67em; margin-block: 2.33em; }

            h1, h2, h3, h4, h5, h6 {
                font-weight: bold;
            }

            /* A browser also gives `<th>` `display: table-cell`, and this rule
               deliberately does not: rinch has no table formatting context at
               all — `DisplayValue` carries no table variant, `table` is
               `display: block` above, and `tr`/`td`/`th`/`thead` keep Stylo's
               default `inline`. Declaring a display the layout engine cannot
               honour would buy nothing and mislead.
               `the_th_rule_does_not_claim_a_table_display` is the pin.

               `font-weight` is plainly real. `text-align` is real as a computed
               value everywhere, and takes visible effect wherever the cell is
               given a block display — which the rich-text editor does
               (`rinch-editor-view/src/styles.rs`: `td, th { display: block }`).
               On a *default* `<th>` it is inert, because rinch reads alignment
               from the IFC root and a `display: inline` cell establishes no
               inline formatting context.

               **`-moz-center-or-inherit`, not `center`, and the difference is
               observable.** The HTML Standard makes this rule conditional — it
               matches "th elements that have a parent node whose computed value
               for the 'text-align' property is its initial value" — so a `<th>`
               under an alignment its parent actually declares must *inherit*
               that, not be re-centred. Chrome implements the condition as its
               own private `-internal-center`; measured in Chrome 150, a `<th>`
               under a `text-align: right` ancestor computes `right`, under
               `justify` computes `justify`, and — the case that pins the wording
               — a `<th>` in a `<table style="text-align: start">` inside a
               `text-align: right` div computes `center` again, because the
               condition is on the **parent node**, not on any ancestor.

               Stylo 0.11 carries exactly this value (`MozCenterOrInherit`, whose
               own doc comment quotes the same spec paragraph). It is not
               gecko-gated, and it parses whenever `chrome_rules_enabled()` — i.e.
               for any non-author origin, which this sheet is
               (`Origin::UserAgent`, below). **An author stylesheet cannot use it**
               and will have the declaration dropped; that asymmetry is the whole
               reason the value exists.

               Spelling it `center` instead is a silent, *unconditional* centring
               that matches Chrome only where the parent declares no alignment —
               which is every fixture that does not go looking, and was the
               surviving mutant this rule shipped with. `a_th_inherits_an_alignment_its_parent_declares`
               samples off that fixed point. */
            th {
                font-weight: bold;
                text-align: -moz-center-or-inherit;
            }

            u, ins {
                text-decoration-line: underline;
            }

            s, strike, del {
                text-decoration-line: line-through;
            }

            /* `svg` is here for the same reason `img` is: in a browser it is an
               inline **replaced** element — an atomic inline — and `inline-block`
               is the model rinch already uses for one. Stylo gives an unknown
               element `display: inline`, and a `display: inline` element that is
               IFC content owns no box at all (`is_flowed_inline_element`, #635),
               so a bare `<svg>` measured 0x0 and painted nothing wherever it was
               flowed: `<div>Save<svg/></div>` was already blank before #592.
               #592 widened that to every `inline-block` — a `<button>` with an
               icon, an icon-only Tooltip/Popover/HoverCard/DropdownMenu target —
               because an inline-block's interior stopped being a Taffy flex
               container, which had been blockifying the svg into a flex item and
               sizing it from its declared width/height by accident. */
            img, input, button, select, textarea, svg {
                display: inline-block;
            }

            /* A closed <select> shows the selected option's label (painted by the
               backend) plus a dropdown arrow — its <option>/<optgroup> children are
               not laid out. Reserve room on the right for the arrow, and a
               min-width so an unstyled control doesn't collapse to its padding.
               The interactive popup is drawn by the app/shell layer (issue #121). */
            option, optgroup {
                display: none;
            }

            select {
                padding: 4px 24px 4px 8px;
                min-width: 60px;
                white-space: nowrap;
                overflow: hidden;
            }

            /* Default list indentation (matches browser default).

               `menu` and `dir` are here, and in every list rule below, because
               Chrome 150 gives them **exactly** `ul`'s treatment — measured:
               `display: block`, `margin-block: 1em`, `padding-left: 40px`, and
               the nested-list zero in both directions (a `<menu>` inside a
               `<ul>` and a `<ul>` inside a `<menu>` are both 0). `dir` is
               obsolete in HTML and `menu` is rare, but rinch named neither tag
               in any rule at all, so both were `display: inline` — a silent
               desktop/web divergence for one token per rule. */
            ul, ol, menu, dir {
                padding-left: 40px;
            }

            /* The block-level default margins, from the HTML Standard's
               rendering section and measured in Chrome 150 (issue #674).
               rinch had the list *indentation* above but none of the spacing,
               so runs of `<p>` butted together and lists sat flush against
               their neighbours — while `rinch-web`, running on the browser's
               own UA sheet, spaced them. Same divergence class as #627.

               `1em`, and an `em` in `margin` resolves against the element's
               **own** computed font-size: a `<p>` in a 20px container carries
               20px of margin, not 16. Invisible at the default root size,
               where 1em is exactly 16px — `ua_block_defaults_tests` samples
               off that fixed point with a 20px container throughout. */
            p, blockquote, figure, ul, ol, menu, dir, pre {
                margin-block: 1em;
            }

            blockquote, figure {
                margin-inline: 40px;
            }

            dd {
                margin-inline-start: 40px;
            }

            /* A list nested in a list carries **no** block margin — Chrome's
               own rule, spelled the same way, and a *descendant* combinator,
               not a child one: measured, a `<ul>` inside `<li>` inside `<ul>`
               (a grandchild) computes 0. Without it every nesting level of a
               bulleted list would add 2em of dead space.

               `:is()` rather than the sixteen pairs written out. Verified to
               match in this Stylo build before it was used here — rinch's
               selector surface has real gaps (`:has()` is silently dropped, see
               `docs/src/guide/theming.md`), so a selector functional
               pseudo-class is not something to assume. */
            :is(ul, ol, menu, dir) :is(ul, ol, menu, dir) {
                margin-block: 0;
            }

            /* Preserving whitespace is the entire point of `<pre>`, and rinch
               honours `white-space: pre` in the IFC already (`ifc.rs` maps it
               to parley's `WhiteSpaceCollapse::Preserve` and drops the wrap
               width) — the UA sheet simply never asked for it, so a bare
               `<pre>` collapsed its runs of spaces and its newlines onto one
               line. This is a real rule, not a rule that pretends: measured,
               `<pre>a  b\nc</pre>` lays out as **two** lines with it and one
               without. */
            pre {
                white-space: pre;
            }

            /* Monospace belongs in the UA sheet, not only in the theme.
               `rinch-theme` ships `code, pre, kbd, samp { font-family:
               var(--rinch-font-family-monospace) }`, so a themed build was
               already right and a **theme-less** one computed `serif` for a
               bare `<code>`. An author declaration still wins, so this changes
               nothing where the theme is loaded.

               rinch does **not** reproduce Chrome's monospace font-size quirk:
               Chrome renders a `medium`-sized monospace element at 13px rather
               than 16px (measured: a bare `<code>` is 13px, and 16px once an
               ancestor declares a size). That is a font-preference behaviour,
               not a stylesheet line, and nothing in rinch carries a per-family
               default size — so rinch's `<code>` stays at the inherited size.
               Deliberate, and the reason no fixture here pins 13px. */
            code, kbd, samp, pre {
                font-family: monospace;
            }

            /* `<hr>` rendered *nothing* before this (issue #674): the
               `* { border-width: 0 }` reset at the top of this sheet — which is
               here to undo Stylo's `medium` initial — applied to `<hr>` too,
               and nothing put it back, so it was a zero-height box with no
               border. The `hr` selector's specificity beats `*`, so this wins
               wherever it is placed in the sheet.

               Every value is Chrome 150's, measured. Two of them are easy to
               get subtly wrong:

               - **`color: gray`, not `border-color: gray`.** The border colour
                 is `currentcolor`; the UA sheet colours the *element*. Measured:
                 `<hr style="color: red">` computes a red border in Chrome, which
                 `an_authored_color_repaints_the_hr_border` pins. Writing
                 `border-color` instead would sit on the fixed point — identical
                 for every `<hr>` that does not declare a `color`.
               - **`margin-inline: auto`.** With `width: auto` it resolves to 0
                 and is invisible; give the rule a width and it centres
                 (`<hr style="width: 100px">` measures margin-left = margin-right
                 = 331px in a 762px body).

               `border-style: inset` is the spec's word and Stylo parses it, but
               rinch's `border_style_from_stylo` maps `groove`/`ridge`/`inset`/
               `outset` to `Solid` — a pre-existing mapping this change does not
               touch. So the *computed* box matches Chrome exactly (1px on each
               side, height 0, total 2px) while the paint is a flat grey rule
               rather than Chrome's shaded one. */
            hr {
                color: gray;
                border-style: inset;
                border-width: 1px;
                margin-block: 0.5em;
                margin-inline: auto;
                height: 0;
                overflow: hidden;
            }

            /* Chrome computes `smaller` as a 1.2 divisor in this range:
               13.3333px from a 16px parent, 16.6667px from a 20px one, and it
               compounds through nesting (a `<small>` in a `<small>` is
               11.1111px). Stylo implements the keyword — verified before it was
               used here, by declaring `font-size: smaller` as an author style
               and reading back 16.6667 from a 20px parent — so this is the
               keyword rather than the `0.83em` approximation the audit
               suggested, which would have given 13.28px and missed Chrome by
               0.05px at every level.

               `vertical-align: sub`/`super` is the other half of `<sub>`/`<sup>`
               and is **not** here: `ComputedStyle` carries no `vertical_align`
               field at all, so it needs property plumbing before a rule could
               mean anything. Tracked separately as issue #724 (#674 §5). */
            small, sub, sup {
                font-size: smaller;
            }

            /* Default body margin - set to 0 for GUI apps */
            /* overflow-y: auto enables viewport scrolling like browsers */
            body {
                margin: 0;
                overflow-y: auto;
            }

            /* A `data-viewport` node is a compositing hole: the game/video frame
               shows through it (find_viewport_rects punches the background,
               unless the node stamps `data-viewport-ready="false"` to say
               nothing will fill the hole yet — issue #186) and a
               pointer landing on it belongs to whatever renders into it, not to
               the rinch UI. That routing is decided by hit-testing the hole, so
               the hole must be HITTABLE — `pointer-events: none` on it makes the
               UI claim the mouse everywhere instead (issue #207). Declaring it
               here also beats a `pointer-events: none` inherited from a HUD root
               (issue #195) while still losing to any author declaration on the
               hole itself, so an app can still opt out. */
            [data-viewport] {
                pointer-events: auto;
                background: transparent;
            }
        "#;

        let url_data = UrlExtraData::from(url::Url::parse("about:ua-stylesheet").unwrap());
        let media = ServoArc::new(self.tree.guard.wrap(MediaList::empty()));

        let stylesheet = Stylesheet::from_str(
            ua_css,
            url_data,
            Origin::UserAgent, // Use UserAgent origin for lowest priority
            media,
            self.tree.guard.clone(),
            None, // stylesheet_loader
            None, // error_reporter
            QuirksMode::NoQuirks,
            AllowImportRules::No,
        );

        let doc_stylesheet = DocumentStyleSheet(ServoArc::new(stylesheet));
        let guard = self.tree.guard.read();
        self.stylist.append_stylesheet(doc_stylesheet, &guard);
        self.stylist
            .force_stylesheet_origins_dirty(Origin::UserAgent.into());
    }

    /// Mark a node and its ancestors as needing layout.
    fn mark_dirty_up(&mut self, node_id: usize, flags: DirtyFlags) {
        let mut current = Some(node_id);
        while let Some(id) = current {
            if let Some(node) = self.tree.nodes.get_mut(id) {
                node.dirty.insert(flags);
                current = node.parent;
            } else {
                break;
            }
        }
    }

    /// Push a node to the dirty list with layout+paint flags.
    fn push_dirty(&mut self, node_id: usize) {
        self.push_dirty_flags(node_id, DirtyFlags::LAYOUT | DirtyFlags::PAINT);
    }

    /// Push a node to the dirty list with specific flags.
    pub(crate) fn push_dirty_flags(&mut self, node_id: usize, flags: DirtyFlags) {
        if self.tree.contains(node_id) {
            self.tree.nodes[node_id].dirty.insert(flags);
            self.tree.push_dirty(node_id);
            if flags.contains(DirtyFlags::LAYOUT) {
                // Don't set layout_dirty here — it's set by:
                // 1. apply_stylo_styles_to_taffy() when Taffy style actually changes
                // 2. Structural changes (append_child, remove_child, etc.)
                // This avoids full Taffy recompute for paint-only changes like transform.
                self.mark_dirty_up(node_id, DirtyFlags::LAYOUT);
            }
        }
    }

    /// Cache a parsed inline `style` block on the node for Stylo's next
    /// cascade of it. Pair with [`parse_inline_style`].
    pub(crate) fn cache_inline_style(&mut self, node_id: usize, pdb: PropertyDeclarationBlock) {
        self.tree.nodes[node_id].style_attribute_cache =
            Some(ServoArc::new(self.tree.guard.wrap(pdb)));
    }

    /// Mark a node and its entire subtree as paint-dirty for removal.
    ///
    /// Used before removing nodes so the dirty region includes the old layout
    /// positions, ensuring borders, backgrounds, and other visuals are cleared.
    /// Saves the absolute rect of each node because the nodes will be deleted
    /// from the tree before `compute_dirty_region` runs.
    pub(crate) fn mark_subtree_paint_dirty(&mut self, node_id: usize) {
        if self.tree.contains(node_id) {
            // Save the rect the node's pixels are in before it leaves the
            // tree: compute_dirty_region won't be able to reach it after.
            //
            // That is the rect it was last *painted* in, with the ink it was
            // painted with (`previous_painted_rect`): not the box it is removed
            // from, which after a move since the last paint was never drawn,
            // and not its bare border box, which leaves an outset
            // `box-shadow` or `outline` behind. Placed from the painted state
            // of its whole chain, so a child removed after its parent moved
            // is cleared where it was drawn, and a transformed subtree leaves
            // no trail (#203).
            let painted = crate::paint::previous_painted_rect(&self.tree, node_id, 1.0);
            let r = match painted {
                Some(r) => Some(r),
                // Never painted since it was created (or removed): nothing of
                // it is on screen. Its current box is recorded anyway, as
                // before — over-repaint at worst, for a node a full repaint
                // drew without a paint ever consuming it.
                None if self.tree.nodes[node_id].painted.is_none() => {
                    let node = &self.tree.nodes[node_id];
                    (node.layout.width > 0.0 && node.layout.height > 0.0)
                        .then(|| crate::paint::painted_border_box(&self.tree, node_id, 1.0))
                }
                None => None,
            };
            if let Some(r) = r {
                self.tree
                    .paint_dirty_removed_rects
                    .push((r.x0, r.y0, r.width(), r.height()));
            }
            // Children first: their painted rects are summed through this
            // node's painted state, which is forgotten below.
            let children = self.tree.nodes[node_id].children.clone();
            for child_id in children {
                self.mark_subtree_paint_dirty(child_id);
            }
            // Its old pixels are accounted for, and after the next paint it
            // has none: a detached node re-inserted later must not carry a
            // painted box it no longer occupies.
            let node = &mut self.tree.nodes[node_id];
            node.painted = None;
            node.prev_layout = node.layout;
        }
    }

    /// Mark a node and its entire subtree as paint-dirty for insertion.
    ///
    /// Adds node IDs to `paint_dirty_nodes` so `compute_dirty_region` can
    /// read their rects after layout. Unlike `mark_subtree_paint_dirty`,
    /// this doesn't save absolute rects (the nodes haven't been laid out yet).
    pub(crate) fn mark_subtree_paint_dirty_ids(&mut self, node_id: usize) {
        if self.tree.contains(node_id) {
            self.tree.paint_dirty_nodes.push(node_id);
            let children = self.tree.nodes[node_id].children.clone();
            for child_id in children {
                self.mark_subtree_paint_dirty_ids(child_id);
            }
        }
    }

    /// Invalidate cached Stylo element data for all descendants of `node_id`.
    /// Used when a parent's class or interaction state changes, since descendant
    /// selectors (e.g. `.parent--active .child`) require descendants to be
    /// re-resolved against the updated ancestor.
    pub(crate) fn invalidate_descendant_styles(&mut self, node_id: usize) {
        // Collect children first to avoid borrow issues
        let children: Vec<usize> = self
            .tree
            .nodes
            .get(node_id)
            .map(|n| n.children.clone())
            .unwrap_or_default();

        for child_id in children {
            if let Some(node) = self.tree.nodes.get_mut(child_id) {
                // Drop the cached match result so the descendant re-resolves against
                // the new ancestor state (class / attribute).
                *node.stylo_element_data.borrow_mut() = None;
                // …and mark it paint-dirty: a re-resolved style that isn't repainted
                // leaves the software renderer's dirty-region cache showing the stale
                // pixels (e.g. a dark-mode toggle re-colored the tree but only the
                // toggled node repainted). Layout is driven by the ancestor's LAYOUT
                // flag (set by the caller), so STYLE | PAINT suffices here.
                node.dirty.insert(DirtyFlags::STYLE | DirtyFlags::PAINT);
            }
            self.tree.dirty_nodes.insert(child_id);
            // No text layout is dropped here. Text colour and every other input
            // a Parley layout is built from are compared per node when the
            // descendant is re-cascaded (`apply_stylo_styles_to_taffy`, through
            // `ComputedStyle::same_text_layout_inputs`), and only a node whose
            // inputs really changed invalidates its IFC; a `display` or
            // `position` change is structural and the IFC pass's content
            // signature catches it. Dropping every layout under the node here
            // re-shaped all of its text on every hover, class toggle and
            // attribute write, whatever had changed (audit F1.1).
            self.invalidate_descendant_styles(child_id);
        }
    }

    /// Advance all active CSS transitions by one frame.
    /// Returns true if any transitions are still active (caller should keep polling).
    pub fn tick_transitions(&mut self) -> bool {
        use web_time::SystemTime;
        let current_time_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
            * 1000.0;

        // Which nodes are having their *typography* interpolated, read **before**
        // the tick, which removes a transition the moment it completes.
        //
        // A transition writes `computed_style` directly, so none of the cascade's
        // invalidation runs for it — and a `font-size` frame changes no Taffy
        // field of its own, so the loop below cannot notice it either. Left
        // alone, each frame re-wraps the text and then serves the *previous*
        // frame's cached measure back for the box.
        //
        // This is issue #678 arriving by a second route, and #678's own repair
        // is what made it reachable — measured, not reasoned. Before that repair
        // no Taffy compute ran on a typography-only pass at all, so nothing had
        // cached a measure to serve and `transition_tests::a_finished_font_size_
        // transition_reaches_the_inline_layout` was green. With the repair and
        // without this pre-pass, the same fixture comes back 30px tall around
        // 40px text.
        let text_measure_nodes: Vec<usize> = self
            .tree
            .active_transitions
            .iter()
            .filter(|(_, props)| props.keys().any(|p| p.changes_text_measure()))
            .map(|(id, _)| *id)
            .collect();

        let any_active = crate::transition::tick_transitions(&mut self.tree, current_time_ms);

        for node_id in text_measure_nodes {
            self.invalidate_text_measure_for_node(node_id);
            // The box has to be measured again, and no Taffy style changed.
            self.tree.layout_dirty = true;
        }

        // For layout-affecting transitions, we need to re-sync Taffy styles
        // from the updated computed_style values.
        // Collect nodes that had LAYOUT dirty set by tick_transitions.
        let layout_dirty: Vec<usize> = self
            .tree
            .active_transitions
            .keys()
            .copied()
            .chain(
                // Also check nodes that just had transitions complete
                self.tree.dirty_nodes.iter().copied(),
            )
            .collect();

        for node_id in layout_dirty {
            // Skip root and html nodes — their Taffy styles are manually set
            // during NodeTree construction and must not be overwritten from
            // computed_style (which has default width:auto instead of 100%).
            if node_id == self.tree.root_id || node_id == self.tree.html_id {
                continue;
            }
            if !self.tree.contains(node_id) {
                continue;
            }
            let node = &self.tree.nodes[node_id];
            if !node.dirty.contains(DirtyFlags::LAYOUT) {
                continue;
            }
            if let Some(taffy_id) = node.taffy_id {
                let dd = self.default_display_for_node(node_id);
                let mut taffy_style = node.computed_style.to_taffy_style(dd);

                // HTML element must fill the viewport and clip horizontal overflow
                if node_id == self.tree.html_id {
                    if taffy_style.size.width == taffy::Dimension::auto() {
                        taffy_style.size.width = taffy::Dimension::percent(1.0);
                    }
                    if taffy_style.size.height == taffy::Dimension::auto() {
                        taffy_style.size.height = taffy::Dimension::percent(1.0);
                    }
                    if taffy_style.overflow.x == taffy::Overflow::Visible {
                        taffy_style.overflow.x = taffy::Overflow::Clip;
                    }
                }

                // Body node needs the same overrides as apply_stylo_styles_to_taffy
                if node_id == self.tree.body_id {
                    if taffy_style.flex_grow == 0.0 {
                        taffy_style.flex_grow = 1.0;
                    }
                    if taffy_style.size.width == taffy::Dimension::auto() {
                        taffy_style.size.width = taffy::Dimension::percent(1.0);
                    }
                }

                // Same as apply_stylo_styles_to_taffy: an out-of-flow box
                // whose containing block is not its Taffy parent is sized from
                // that containing block. Rebuilding the style from the computed
                // values drops the override, so a transition/animation frame on
                // a `position: fixed` (or ICB-absolute, #204) node used to
                // collapse it back onto its parent's box.
                if let Some(kind) = crate::out_of_flow::out_of_flow_kind(&self.tree, node_id) {
                    crate::out_of_flow::apply_out_of_flow_size_overrides(
                        node,
                        kind,
                        self.tree.viewport,
                        &mut taffy_style,
                    );
                }

                // Collapsed block (virtualized contenteditable): keep the
                // estimated height apply_stylo_styles_to_taffy would have set.
                if let Some(est_h) = node.estimated_height {
                    taffy_style.size.height = taffy::Dimension::length(est_h);
                }

                // This rebuilds the Taffy style from the computed values, so it
                // drops the childless-block line floor exactly the way
                // apply_stylo_styles_to_taffy used to — re-apply it here or a
                // transition frame collapses a blockified `<input>` to nothing.
                crate::ifc::apply_empty_block_line_floor(node, &mut taffy_style);

                // A `display: contents` wrapper's Taffy style belongs to
                // `sync_display_contents`, as in `apply_stylo_styles_to_taffy`;
                // rebuilding it from the computed values would give the wrapper
                // a `Display::Flex` box of its own.
                if node.taffy_style_owned_by_contents_splice() {
                    taffy_style = crate::node::display_contents_taffy_style();
                }

                // Only call set_style if the Taffy style actually changed.
                // set_style() internally calls mark_dirty() which propagates up
                // the entire ancestor chain — unconditional calls here were causing
                // 70%+ of Taffy nodes to lose their cache on every frame with
                // active transitions, even when only paint-only properties changed.
                let taffy_style_changed = match self.tree.taffy.style(taffy_id) {
                    Ok(old_taffy_style) => old_taffy_style != &taffy_style,
                    // No style to compare against: treat it as changed, which is
                    // what the `else` arm this replaced did.
                    Err(_) => true,
                };
                if taffy_style_changed {
                    let _ = self.tree.taffy.set_style(taffy_id, taffy_style);
                    self.tree.layout_dirty = true;
                    // The twin of the cascade's call in
                    // `apply_stylo_styles_to_taffy`, and it has to be here for
                    // the same reason: a Taffy style change reaches this node's
                    // own box through the compute, but not through an atomic
                    // inline sitting between it and the compute root — that box
                    // is detached from its parent's child list (#661).
                    //
                    // The tick pre-passes above do **not** cover this. They fire
                    // only for `changes_text_measure()` properties, so an
                    // ordinary `transition: width` on a box inside an
                    // `inline-block` used to set `layout_dirty`, run a compute,
                    // and leave the `inline-block` at `50x10` against a `300x10`
                    // oracle — #661's symptom, on the one path that reaches it
                    // without the cascade.
                    self.mark_atomic_inline_dirty(node_id);
                }
            }
        }

        any_active
    }

    /// Advance all active CSS animations by one frame.
    /// Returns true if any **running** animation is still active (caller should
    /// keep polling). A paused one, and a finished one that fills, is kept but
    /// not counted (#763, #782) — see [`crate::animation::tick_animations`].
    pub fn tick_animations(&mut self) -> bool {
        use web_time::SystemTime;
        let current_time_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
            * 1000.0;

        // The animation twin of the pre-pass in `tick_transitions` — same reason,
        // same issue (#678). An animation's property set lives in its keyframes
        // rather than in a map key, so the question is asked of those.
        //
        // Only of animations a tick can move (#763, #782). A paused animation's
        // sample and a settled fill are constant: the cascade that wrote them
        // invalidated the measure (see `animated_text_measure` in
        // `apply_stylo_styles_to_taffy` and `restart_animations_in_subtree`) —
        // which is the whole of the guarantee, so a hole in *that*
        // invalidation is a freeze nothing later repairs, as #784 was until the
        // `ifc.rs` repair beside it,
        // and re-measuring on every tick would set `layout_dirty` every tick —
        // which the desktop wake and the Android loop both read as a frame owed.
        // A filling animation that has *not* settled is still asked: the tick
        // below is the one that writes its fill.
        let text_measure_nodes: Vec<usize> = self
            .tree
            .active_animations
            .iter()
            .filter(|(_, anims)| {
                anims
                    .iter()
                    .any(|a| !a.is_paused() && !a.fill_settled && a.changes_text_measure())
            })
            .map(|(id, _)| *id)
            .collect();

        let any_active = crate::animation::tick_animations(&mut self.tree, current_time_ms);

        for node_id in text_measure_nodes {
            self.invalidate_text_measure_for_node(node_id);
            self.tree.layout_dirty = true;
        }

        // For layout-affecting animations, re-sync Taffy styles.
        let layout_dirty: Vec<usize> = self
            .tree
            .active_animations
            .keys()
            .copied()
            .chain(self.tree.dirty_nodes.iter().copied())
            .collect();

        for node_id in layout_dirty {
            if node_id == self.tree.root_id || node_id == self.tree.html_id {
                continue;
            }
            if !self.tree.contains(node_id) {
                continue;
            }
            let node = &self.tree.nodes[node_id];
            if !node.dirty.contains(DirtyFlags::LAYOUT) {
                continue;
            }
            if let Some(taffy_id) = node.taffy_id {
                let dd = self.default_display_for_node(node_id);
                let mut taffy_style = node.computed_style.to_taffy_style(dd);

                // Body node needs the same overrides as apply_stylo_styles_to_taffy
                if node_id == self.tree.body_id {
                    if taffy_style.flex_grow == 0.0 {
                        taffy_style.flex_grow = 1.0;
                    }
                    if taffy_style.size.width == taffy::Dimension::auto() {
                        taffy_style.size.width = taffy::Dimension::percent(1.0);
                    }
                }

                // Same as apply_stylo_styles_to_taffy: an out-of-flow box
                // whose containing block is not its Taffy parent is sized from
                // that containing block. Rebuilding the style from the computed
                // values drops the override, so a transition/animation frame on
                // a `position: fixed` (or ICB-absolute, #204) node used to
                // collapse it back onto its parent's box.
                if let Some(kind) = crate::out_of_flow::out_of_flow_kind(&self.tree, node_id) {
                    crate::out_of_flow::apply_out_of_flow_size_overrides(
                        node,
                        kind,
                        self.tree.viewport,
                        &mut taffy_style,
                    );
                }

                // Collapsed block (virtualized contenteditable): keep the
                // estimated height apply_stylo_styles_to_taffy would have set.
                if let Some(est_h) = node.estimated_height {
                    taffy_style.size.height = taffy::Dimension::length(est_h);
                }

                // Same rebuild-from-computed-values hazard as tick_transitions:
                // without this an animation frame drops the childless-block line
                // floor and the element collapses to zero height.
                crate::ifc::apply_empty_block_line_floor(node, &mut taffy_style);

                // Same ownership rule as `tick_transitions`.
                if node.taffy_style_owned_by_contents_splice() {
                    taffy_style = crate::node::display_contents_taffy_style();
                }

                let taffy_style_changed = match self.tree.taffy.style(taffy_id) {
                    Ok(old_taffy_style) => old_taffy_style != &taffy_style,
                    // No style to compare against: treat it as changed, which is
                    // what the `else` arm this replaced did.
                    Err(_) => true,
                };
                if taffy_style_changed {
                    let _ = self.tree.taffy.set_style(taffy_id, taffy_style);
                    self.tree.layout_dirty = true;
                    // Same atomic-inline hazard as `tick_transitions`, and the
                    // pre-pass above covers it no better here: an animated
                    // `width` on a box inside an `inline-block` leaves that box
                    // frozen without this (#661).
                    self.mark_atomic_inline_dirty(node_id);
                }
            }
        }

        any_active
    }

    /// Request an image load for a node's `src` attribute.
    ///
    /// If the image is already decoded in the cache, updates intrinsic dimensions
    /// immediately. Otherwise kicks off an async load on a background thread.
    pub(crate) fn request_image_load_for_node(&mut self, node_id: usize, src: &str) {
        if src.is_empty() {
            return;
        }

        // Data URIs are decoded synchronously and inserted directly into the
        // image cache (not the pending queue). This ensures that when a signal
        // triggers a re-render and creates a new img element, the cache lookup
        // in the "already in cache" path below finds the Decoded entry immediately.
        if src.starts_with("data:") {
            if !self.tree.image_cache.contains(src)
                && let Some(bytes) = crate::image_cache::decode_data_uri(src)
            {
                match image::load_from_memory(&bytes) {
                    Ok(img) => {
                        let rgba = img.to_rgba8();
                        let (w, h) = (rgba.width(), rgba.height());
                        self.tree.image_cache.insert_decoded(
                            src.to_string(),
                            crate::image_cache::DecodedImage {
                                data: rgba.into_raw(),
                                width: w,
                                height: h,
                            },
                        );
                    }
                    Err(e) => {
                        tracing::warn!("Failed to decode data URI image: {}", e);
                    }
                }
            }
            // Fall through to the "already in cache" path to set NodeContext
            // dimensions on this node (works for both first render and re-renders).
        }

        // Check if already in cache
        if let Some(img) = self.tree.image_cache.get(src) {
            // Already decoded — update intrinsic dimensions on the Taffy node
            let (iw, ih) = (img.width, img.height);
            if let Some(taffy_id) = self.tree.nodes[node_id].taffy_id {
                let _ = self.tree.taffy.set_node_context(
                    taffy_id,
                    Some(crate::node::NodeContext::Image {
                        src: src.to_string(),
                        width: iw,
                        height: ih,
                    }),
                );
                let _ = self.tree.taffy.mark_dirty(taffy_id);
            }
            self.push_dirty_flags(node_id, DirtyFlags::LAYOUT | DirtyFlags::PAINT);
            return;
        }

        // If already loading, don't re-request
        if self.tree.image_cache.contains(src) {
            return;
        }

        // Mark as loading and kick off background load
        let Some(loader) = self.tree.image_loader.clone() else {
            return;
        };

        self.tree.image_cache.mark_loading(src.to_string());

        // Update NodeContext with src (0x0 dims while loading)
        if let Some(taffy_id) = self.tree.nodes[node_id].taffy_id {
            let _ = self.tree.taffy.set_node_context(
                taffy_id,
                Some(crate::node::NodeContext::Image {
                    src: src.to_string(),
                    width: 0,
                    height: 0,
                }),
            );
        }

        // Kick off async load — result goes to PENDING_IMAGES static queue,
        // tagged with this document's identity (#137)
        crate::image_cache::request_image_load(self.doc_key, src.to_string(), loader);
    }

    /// Scan for background-image URLs that need loading and trigger async loads.
    pub fn request_background_image_loads(&mut self) {
        let Some(loader) = self.tree.image_loader.clone() else {
            return;
        };

        // Collect URLs that need loading
        let urls_to_load: Vec<String> = self
            .tree
            .nodes
            .iter()
            .filter_map(|(_, node)| {
                if let crate::computed_style::BackgroundValue::Image { url } =
                    &node.computed_style.background
                    && !self.tree.image_cache.contains(url)
                {
                    return Some(url.clone());
                }
                None
            })
            .collect();

        for url in urls_to_load {
            self.tree.image_cache.mark_loading(url.clone());
            crate::image_cache::request_image_load(self.doc_key, url, loader.clone());
        }
    }

    /// Drain completed image loads and update Taffy nodes with intrinsic dimensions.
    ///
    /// Called before layout to pick up newly decoded images.
    ///
    /// Returns true only if a decode changed an `<img>`'s intrinsic size, i.e.
    /// if the tree needs a Taffy recompute. A `background-image` decode changes
    /// no box, so it returns false and is published by marking its users
    /// paint-dirty instead — a full re-layout for a paint-only change would be
    /// wasted work, and leaving those users *out* of `paint_dirty_nodes` would
    /// let the software renderer's dirty-region path repaint some other node's
    /// rect and skip the freshly decoded background entirely.
    pub fn drain_pending_images(&mut self) -> bool {
        let newly_decoded = self.tree.image_cache.drain_pending(self.doc_key);
        if newly_decoded.is_empty() {
            return false;
        }
        let mut layout_changed = false;
        // For each newly decoded image, find <img> nodes referencing it
        // and update their intrinsic dimensions
        for src in &newly_decoded {
            // `background-image: url(src)` users: no intrinsic size feeds
            // layout, but their pixels changed, so they must be in the dirty
            // region the next paint clears and redraws.
            let bg_ids: Vec<usize> = self
                .tree
                .nodes
                .iter()
                .filter_map(|(id, node)| match &node.computed_style.background {
                    crate::computed_style::BackgroundValue::Image { url } if url == src => Some(id),
                    _ => None,
                })
                .collect();
            for node_id in bg_ids {
                self.push_dirty_flags(node_id, DirtyFlags::PAINT);
            }

            let img_dims = self.tree.image_cache.get(src).map(|i| (i.width, i.height));
            let Some((iw, ih)) = img_dims else {
                continue;
            };

            // Scan all nodes for img elements with matching src
            let node_ids: Vec<usize> = self
                .tree
                .nodes
                .iter()
                .filter_map(|(id, node)| {
                    if node.tag() == Some("img")
                        && node.attributes.get("src").map(|s| s.as_str()) == Some(src)
                    {
                        Some(id)
                    } else {
                        None
                    }
                })
                .collect();

            for node_id in node_ids {
                if let Some(taffy_id) = self.tree.nodes[node_id].taffy_id {
                    let _ = self.tree.taffy.set_node_context(
                        taffy_id,
                        Some(crate::node::NodeContext::Image {
                            src: src.clone(),
                            width: iw,
                            height: ih,
                        }),
                    );
                    let _ = self.tree.taffy.mark_dirty(taffy_id);
                }
                self.push_dirty_flags(node_id, DirtyFlags::LAYOUT | DirtyFlags::PAINT);

                // `<img>` is `inline-block` in the UA sheet, so a default-styled
                // one is *detached from the root Taffy tree* and pre-measured by
                // `compute_inline_block_layouts` — which `resolve_layout` runs
                // only under `ifc_dirty`. Re-contexting the detached Taffy node
                // therefore changes nothing on its own: the 0x0 box measured
                // while the image was still loading is what stays, however many
                // frames the wake produces. This is the invalidation that makes
                // the decoded size actually reach the box.
                self.tree.ifc_dirty = true;

                layout_changed = true;
            }
        }

        layout_changed
    }
}

impl RinchDocument {
    /// Simple recursive query selector.
    fn query_recursive(&self, node_id: usize, selector: &str) -> Option<usize> {
        let node = self.tree.nodes.get(node_id)?;

        // Match by #id
        if let Some(id) = selector.strip_prefix('#') {
            if node.attributes.get("id").map(|v| v.as_str()) == Some(id) {
                return Some(node_id);
            }
        }
        // Match by .class
        else if let Some(class) = selector.strip_prefix('.') {
            if let Some(classes) = node.attributes.get("class")
                && classes.split_whitespace().any(|c| c == class)
            {
                return Some(node_id);
            }
        }
        // Match by attribute selector [attr] or [attr=value]
        else if let Some(attr_sel) = selector.strip_prefix('[').and_then(|s| s.strip_suffix(']'))
        {
            if let Some((attr_name, attr_value)) = attr_sel.split_once('=') {
                // [attr=value]
                let value = attr_value.trim_matches('"').trim_matches('\'');
                if node.attributes.get(attr_name).map(|v| v.as_str()) == Some(value) {
                    return Some(node_id);
                }
            } else {
                // [attr]
                if node.attributes.contains_key(attr_sel) {
                    return Some(node_id);
                }
            }
        }
        // Match by tag name
        else if node.tag() == Some(selector) {
            return Some(node_id);
        }

        // Search children
        let children: Vec<_> = node.children.clone();
        for child in children {
            if let Some(found) = self.query_recursive(child, selector) {
                return Some(found);
            }
        }
        None
    }

    /// Query all nodes matching a selector.
    fn query_all_recursive(&self, node_id: usize, selector: &str, results: &mut Vec<usize>) {
        let Some(node) = self.tree.nodes.get(node_id) else {
            return;
        };

        let matches = if let Some(id) = selector.strip_prefix('#') {
            node.attributes.get("id").map(|v| v.as_str()) == Some(id)
        } else if let Some(class) = selector.strip_prefix('.') {
            node.attributes
                .get("class")
                .map(|classes| classes.split_whitespace().any(|c| c == class))
                .unwrap_or(false)
        } else if let Some(attr_sel) = selector.strip_prefix('[').and_then(|s| s.strip_suffix(']'))
        {
            if let Some((attr_name, attr_value)) = attr_sel.split_once('=') {
                let value = attr_value.trim_matches('"').trim_matches('\'');
                node.attributes.get(attr_name).map(|v| v.as_str()) == Some(value)
            } else {
                node.attributes.contains_key(attr_sel)
            }
        } else {
            node.tag() == Some(selector)
        };

        if matches {
            results.push(node_id);
        }

        // Search all children
        let children: Vec<_> = node.children.clone();
        for child in children {
            self.query_all_recursive(child, selector, results);
        }
    }

    /// Query all nodes matching a selector, returning a vector of NodeIds.
    pub fn query_selector_all(&self, selector: &str) -> Vec<rinch_core::dom::NodeId> {
        let mut results = Vec::new();
        self.query_all_recursive(self.tree.root_id, selector, &mut results);
        results.into_iter().map(rinch_core::dom::NodeId).collect()
    }

    /// Recursively collect text content from a node and its descendants.
    pub(crate) fn collect_text_content(&self, node_id: usize, result: &mut String) {
        let Some(node) = self.tree.nodes.get(node_id) else {
            return;
        };
        match &node.kind {
            crate::node::NodeKind::Text(data) => result.push_str(&data.content),
            crate::node::NodeKind::Element(_) => {
                for &child_id in &node.children {
                    self.collect_text_content(child_id, result);
                }
            }
            _ => {}
        }
    }
}

/// Parse an inline `style` attribute value into Stylo's declaration block the
/// way author content wants it: `about:blank`, no quirks, no error reporting.
/// One place, so `set_attribute("style")`, `set_styles` and the inset fast
/// path can't drift.
pub(super) fn parse_inline_style(css: &str) -> PropertyDeclarationBlock {
    style::properties::parse_style_attribute(
        css,
        &crate::layout::BLANK_URL_DATA,
        None,
        QuirksMode::NoQuirks,
        style::stylesheets::CssRuleType::Style,
    )
}
