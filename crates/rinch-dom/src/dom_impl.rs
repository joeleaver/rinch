//! DomDocument implementation for rinch-dom.

use std::collections::HashMap;

use rinch_core::dom::{DomDocument, NodeId};

use peniko::Brush;
use peniko::color::{AlphaColor, Srgb};
use servo_arc::Arc as ServoArc;

// Stylo CSS engine imports
use euclid::Scale;
use style::context::QuirksMode;
use style::font_metrics::FontMetrics;
use style::media_queries::{Device, MediaType};
use style::properties::ComputedValues;
use style::properties::style_structs::Font as StyloFont;
use style::queries::values::PrefersColorScheme;
use style::stylist::Stylist;
use style::values::computed::font::GenericFontFamily;
use style::values::computed::{CSSPixelLength, Length};
use style::values::specified::font::QueryFontMetricsFlags;
use stylo_config as style_config;
// CSSPixel and DevicePixel are used via euclid::Size2D type parameters

use crate::node::{DirtyFlags, DisplayMode, Node, NodeContext, NodeKind, NodeTree, TextMeasure};

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

/// The primary document type for rinch-dom.
///
/// Implements [`DomDocument`] using a slab-allocated node tree.
/// In later phases, this will integrate Taffy for layout,
/// Parley for text, and Vello for painting.
pub struct RinchDocument {
    /// The node tree.
    pub tree: NodeTree,
    /// Parley font context for text shaping.
    pub font_cx: parley::FontContext,
    /// Parley layout context for text measurement.
    pub layout_cx: parley::LayoutContext<Brush>,
    /// Stylo CSS engine stylist for CSS cascade and selector matching.
    pub stylist: Stylist,
}

impl Default for RinchDocument {
    fn default() -> Self {
        Self::new()
    }
}

impl RinchDocument {
    /// Create a new document with root and body nodes.
    pub fn new() -> Self {
        // Enable CSS Grid support in Stylo
        // This must be called before any CSS parsing happens
        style_config::set_bool("layout.grid.enabled", true);

        // Create the Stylo Device with default viewport and settings
        let viewport_size = euclid::Size2D::new(800.0, 600.0);
        let device_pixel_ratio = Scale::new(1.0);
        let font_metrics_provider = Box::new(SimpleFontMetricsProvider);
        let default_font = StyloFont::initial_values();
        let default_computed_values =
            ComputedValues::initial_values_with_font_override(default_font);

        let device = Device::new(
            MediaType::screen(),
            QuirksMode::NoQuirks,
            viewport_size,
            device_pixel_ratio,
            font_metrics_provider,
            default_computed_values,
            PrefersColorScheme::Light,
        );

        let stylist = Stylist::new(device, QuirksMode::NoQuirks);

        let mut doc = Self {
            tree: NodeTree::new(),
            font_cx: parley::FontContext::new(),
            layout_cx: parley::LayoutContext::new(),
            stylist,
        };

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
            ul, ol, li, dl, dt, dd, table, form, fieldset, legend, hr,
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

            img, input, button, select, textarea {
                display: inline-block;
            }

            /* Default list indentation (matches browser default) */
            ul, ol {
                padding-left: 40px;
            }

            /* Default body margin - set to 0 for GUI apps */
            body {
                margin: 0;
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
                self.mark_dirty_up(node_id, DirtyFlags::LAYOUT);
            }
        }
    }

    /// Advance all active CSS transitions by one frame.
    /// Returns true if any transitions are still active (caller should keep polling).
    pub fn tick_transitions(&mut self) -> bool {
        use std::time::SystemTime;
        let current_time_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
            * 1000.0;

        let any_active = crate::transition::tick_transitions(&mut self.tree, current_time_ms);

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
            if !self.tree.contains(node_id) {
                continue;
            }
            let node = &self.tree.nodes[node_id];
            if !node.dirty.contains(DirtyFlags::LAYOUT) {
                continue;
            }
            if let Some(taffy_id) = node.taffy_id {
                let dd = self.default_display_for_node(node_id);
                let taffy_style = node.computed_style.to_taffy_style(dd);
                let _ = self.tree.taffy.set_style(taffy_id, taffy_style);
            }
        }

        any_active
    }
}

impl DomDocument for RinchDocument {
    fn create_element(&mut self, tag: &str) -> NodeId {
        let id = self.tree.nodes.vacant_key();
        let mut node = Node::element(id, tag, self.tree.guard.clone());
        // Use CSS-standard defaults based on element type:
        // Block elements (div, p, h1, etc.): flex-column (emulates block stacking)
        // Inline elements (span, a, etc.): flex-row
        let is_block = matches!(node.display_mode, DisplayMode::Block);
        let taffy_id = self
            .tree
            .taffy
            .new_leaf(taffy::Style {
                display: taffy::Display::Flex,
                flex_direction: if is_block {
                    taffy::FlexDirection::Column
                } else {
                    taffy::FlexDirection::Row
                },
                flex_wrap: taffy::FlexWrap::NoWrap,
                ..Default::default()
            })
            .unwrap();
        node.taffy_id = Some(taffy_id);
        self.tree.taffy_map.insert(taffy_id, id);
        self.tree.nodes.insert(node);

        // Hidden elements should not participate in layout
        if matches!(tag, "style" | "script" | "head" | "meta" | "link" | "title") {
            let _ = self.tree.taffy.set_style(
                taffy_id,
                taffy::Style {
                    display: taffy::Display::None,
                    ..Default::default()
                },
            );
        }

        NodeId(id)
    }

    fn create_text(&mut self, text: &str) -> NodeId {
        let id = self.tree.nodes.vacant_key();
        let mut node = Node::text(id, text, self.tree.guard.clone());
        let context = NodeContext::Text(TextMeasure {
            content: text.to_string(),
            font_size: 16.0, // default, will be updated from parent before layout
            font_weight: 400.0,
            font_family: String::new(),
            line_height_css: String::new(),
            node_id: id,
            color: AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255), // default black, updated from parent
            no_wrap: false,                                      // default, updated from parent
            overflow_wrap: crate::computed_style::OverflowWrapValue::default(),
        });
        let taffy_id = self
            .tree
            .taffy
            .new_leaf_with_context(taffy::Style::default(), context)
            .unwrap();
        node.taffy_id = Some(taffy_id);
        self.tree.taffy_map.insert(taffy_id, id);
        self.tree.nodes.insert(node);
        NodeId(id)
    }

    fn create_comment(&mut self, text: &str) -> NodeId {
        let id = self.tree.nodes.vacant_key();
        let node = Node::comment(id, text, self.tree.guard.clone());
        // Comments do NOT get Taffy nodes
        self.tree.nodes.insert(node);
        NodeId(id)
    }

    fn append_child(&mut self, parent: NodeId, child: NodeId) {
        let p = parent.0;
        let c = child.0;
        // Invalidate old IFC if child was in one
        self.invalidate_ifc_for_node(c);
        self.clear_ifc_root_recursive(c);
        // Remove from old parent if any (both DOM and Taffy)
        if let Some(old_parent) = self.tree.nodes[c].parent {
            self.tree.nodes[old_parent].children.retain(|&x| x != c);
            // Remove from old taffy parent
            if let (Some(old_taffy_parent), Some(child_taffy)) = (
                self.tree.nodes[old_parent].taffy_id,
                self.tree.nodes[c].taffy_id,
            ) {
                self.taffy_remove_child_safe(old_taffy_parent, child_taffy);
            }
        }
        self.tree.nodes[c].parent = Some(p);
        self.tree.nodes[p].children.push(c);
        // Sync taffy
        if let (Some(parent_taffy), Some(child_taffy)) =
            (self.tree.nodes[p].taffy_id, self.tree.nodes[c].taffy_id)
        {
            let _ = self.tree.taffy.add_child(parent_taffy, child_taffy);
            // Mark child as dirty so it gets measured during next layout pass
            let _ = self.tree.taffy.mark_dirty(child_taffy);
        }
        // Invalidate parent's IFC (structure changed)
        self.invalidate_parent_ifc(p);
        self.push_dirty_flags(p, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);

        // Recompute styles for the inserted subtree to pick up ancestor-based selectors
        self.recompute_node_styles_recursive(c);

        // If a text node is appended to a <style> element, load its content as CSS
        self.maybe_load_style_css(p);
    }

    fn remove_child(&mut self, parent: NodeId, child: NodeId) {
        let p = parent.0;
        let c = child.0;
        // Clear IFC state on removed child
        self.clear_ifc_root_recursive(c);
        self.tree.nodes[p].children.retain(|&x| x != c);
        self.tree.nodes[c].parent = None;
        // Sync taffy
        if let (Some(parent_taffy), Some(child_taffy)) =
            (self.tree.nodes[p].taffy_id, self.tree.nodes[c].taffy_id)
        {
            self.taffy_remove_child_safe(parent_taffy, child_taffy);
        }
        // Invalidate parent's IFC
        self.invalidate_parent_ifc(p);
        self.push_dirty_flags(p, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);
    }

    fn insert_before(&mut self, parent: NodeId, child: NodeId, reference: NodeId) {
        let p = parent.0;
        let c = child.0;
        let r = reference.0;
        // Invalidate old IFC
        self.invalidate_ifc_for_node(c);
        self.clear_ifc_root_recursive(c);
        // Remove from old parent if any
        if let Some(old_parent) = self.tree.nodes[c].parent {
            self.tree.nodes[old_parent].children.retain(|&x| x != c);
            if let (Some(old_taffy_parent), Some(child_taffy)) = (
                self.tree.nodes[old_parent].taffy_id,
                self.tree.nodes[c].taffy_id,
            ) {
                self.taffy_remove_child_safe(old_taffy_parent, child_taffy);
            }
        }
        self.tree.nodes[c].parent = Some(p);
        let insert_pos = if let Some(pos) = self.tree.nodes[p].children.iter().position(|&x| x == r)
        {
            self.tree.nodes[p].children.insert(pos, c);
            Some(pos)
        } else {
            self.tree.nodes[p].children.push(c);
            None
        };
        // Sync taffy
        if let (Some(parent_taffy), Some(child_taffy)) =
            (self.tree.nodes[p].taffy_id, self.tree.nodes[c].taffy_id)
        {
            if let Some(pos) = insert_pos {
                // Count taffy children before this position to find taffy index
                let taffy_idx = self.compute_taffy_child_index(p, pos);
                let _ = self
                    .tree
                    .taffy
                    .insert_child_at_index(parent_taffy, taffy_idx, child_taffy);
            } else {
                let _ = self.tree.taffy.add_child(parent_taffy, child_taffy);
            }
        }
        self.invalidate_parent_ifc(p);
        self.push_dirty_flags(p, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);
    }

    fn replace_node(&mut self, old: NodeId, new: NodeId) {
        self.invalidate_ifc_for_node(old.0);
        self.clear_ifc_root_recursive(old.0);
        self.invalidate_ifc_for_node(new.0);
        self.clear_ifc_root_recursive(new.0);
        if let Some(parent_id) = self.tree.nodes[old.0].parent {
            // Remove new from its old parent if any
            if let Some(old_parent) = self.tree.nodes[new.0].parent {
                self.tree.nodes[old_parent].children.retain(|&x| x != new.0);
                if let (Some(old_taffy_parent), Some(new_taffy)) = (
                    self.tree.nodes[old_parent].taffy_id,
                    self.tree.nodes[new.0].taffy_id,
                ) {
                    self.taffy_remove_child_safe(old_taffy_parent, new_taffy);
                }
            }
            // Replace old with new in parent's children
            if let Some(pos) = self.tree.nodes[parent_id]
                .children
                .iter()
                .position(|&x| x == old.0)
            {
                self.tree.nodes[parent_id].children[pos] = new.0;
                // Sync taffy: remove old, insert new at same position
                if let Some(parent_taffy) = self.tree.nodes[parent_id].taffy_id {
                    if let Some(old_taffy) = self.tree.nodes[old.0].taffy_id {
                        self.taffy_remove_child_safe(parent_taffy, old_taffy);
                    }
                    if let Some(new_taffy) = self.tree.nodes[new.0].taffy_id {
                        let taffy_idx = self.compute_taffy_child_index(parent_id, pos);
                        let _ = self.tree.taffy.insert_child_at_index(
                            parent_taffy,
                            taffy_idx,
                            new_taffy,
                        );
                    }
                }
            }
            self.tree.nodes[new.0].parent = Some(parent_id);
            self.tree.nodes[old.0].parent = None;
            self.invalidate_parent_ifc(parent_id);
            self.push_dirty_flags(parent_id, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);
        }
    }

    fn remove_node(&mut self, node: NodeId) {
        self.clear_ifc_root_recursive(node.0);
        if let Some(parent_id) = self.tree.nodes[node.0].parent {
            self.tree.nodes[parent_id].children.retain(|&x| x != node.0);
            // Sync taffy
            if let (Some(parent_taffy), Some(node_taffy)) = (
                self.tree.nodes[parent_id].taffy_id,
                self.tree.nodes[node.0].taffy_id,
            ) {
                self.taffy_remove_child_safe(parent_taffy, node_taffy);
            }
            self.invalidate_parent_ifc(parent_id);
            self.push_dirty_flags(parent_id, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);
        }
        self.tree.nodes[node.0].parent = None;
        // Don't remove from slab yet — caller may still reference it
    }

    fn set_text_content(&mut self, node: NodeId, text: &str) {
        let n = node.0;
        // Invalidate IFC if this node belongs to one
        self.invalidate_ifc_for_node(n);
        // Also invalidate parent's IFC
        if let Some(parent_id) = self.tree.nodes[n].parent {
            self.invalidate_parent_ifc(parent_id);
        }
        match &mut self.tree.nodes[n].kind {
            NodeKind::Text(t) => {
                t.content = text.to_string();
                // Update the Taffy NodeContext too
                if let Some(taffy_id) = self.tree.nodes[n].taffy_id {
                    if let Some(ctx) = self.tree.taffy.get_node_context_mut(taffy_id)
                        && let NodeContext::Text(tm) = ctx
                    {
                        tm.content = text.to_string();
                    }
                    let _ = self.tree.taffy.mark_dirty(taffy_id);
                }
            }
            _ => {
                // Clear any IFC text_layout on this element (it is the IFC root
                // for its inline children, but ifc_root points to it from children,
                // not from itself, so invalidate_ifc_for_node misses it).
                self.tree.nodes[n].text_layout = None;
                // For elements: remove all children and add a text child
                let old_children: Vec<_> = self.tree.nodes[n].children.clone();
                for child in old_children {
                    self.tree.nodes[child].parent = None;
                    // Remove from taffy parent
                    if let (Some(parent_taffy), Some(child_taffy)) =
                        (self.tree.nodes[n].taffy_id, self.tree.nodes[child].taffy_id)
                    {
                        self.taffy_remove_child_safe(parent_taffy, child_taffy);
                    }
                }
                self.tree.nodes[n].children.clear();
                // Create text child with taffy node and context
                let text_id = self.tree.nodes.vacant_key();
                let mut text_node = Node::text(text_id, text, self.tree.guard.clone());
                text_node.parent = Some(n);
                let context = NodeContext::Text(TextMeasure {
                    content: text.to_string(),
                    font_size: 16.0,
                    font_weight: 400.0,
                    font_family: String::new(),
                    line_height_css: String::new(),
                    node_id: text_id,
                    color: AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255),
                    no_wrap: false,
                    overflow_wrap: crate::computed_style::OverflowWrapValue::default(),
                });
                let taffy_id = self
                    .tree
                    .taffy
                    .new_leaf_with_context(taffy::Style::default(), context)
                    .unwrap();
                text_node.taffy_id = Some(taffy_id);
                self.tree.taffy_map.insert(taffy_id, text_id);
                self.tree.nodes.insert(text_node);
                self.tree.nodes[n].children.push(text_id);
                // Add to taffy parent
                if let Some(parent_taffy) = self.tree.nodes[n].taffy_id {
                    let _ = self.tree.taffy.add_child(parent_taffy, taffy_id);
                }
            }
        }
        self.push_dirty(n);

        // If this node is a <style> element, reload its CSS
        self.maybe_load_style_css(n);
        // If the parent is a <style> element (text node content changed), reload
        if let Some(parent_id) = self.tree.nodes[n].parent {
            self.maybe_load_style_css(parent_id);
        }
    }

    fn set_attribute(&mut self, node: NodeId, name: &str, value: &str) {
        self.tree.nodes[node.0]
            .attributes
            .insert(name.to_string(), value.to_string());

        // Parse inline style into Stylo PropertyDeclarationBlock
        if name == "style" {
            use style::properties::parse_style_attribute;
            use style::stylesheets::CssRuleType;
            use url::Url;

            let url = Url::parse("about:blank").unwrap();
            let extra_data = style::stylesheets::UrlExtraData::from(url);
            let pdb = parse_style_attribute(
                value,
                &extra_data,
                None, // error_reporter
                style::context::QuirksMode::NoQuirks,
                CssRuleType::Style,
            );
            self.tree.nodes[node.0].style_attribute_cache =
                Some(ServoArc::new(self.tree.guard.wrap(pdb)));
        }
        // Invalidate IFC if this node belongs to one (style/class changes affect inline layout)
        if name == "style" || name == "class" {
            self.invalidate_ifc_for_node(node.0);
            // Also invalidate parent's IFC in case this is an inline child
            if let Some(parent_id) = self.tree.nodes[node.0].parent {
                self.invalidate_parent_ifc(parent_id);
            }
        }
        // SVG elements: width/height HTML attributes affect layout sizing
        let needs_style_recompute = name == "class"
            || name == "style"
            || ((name == "width" || name == "height" || name == "viewBox")
                && self.tree.nodes[node.0].tag() == Some("svg"));

        if needs_style_recompute {
            // Invalidate cached Stylo data and resolve styles for this subtree
            *self.tree.nodes[node.0].stylo_element_data.borrow_mut() = None;
            self.tree.styles_dirty = true;
            self.resolve_styles();
            self.apply_stylo_styles_to_taffy();
            self.push_dirty_flags(
                node.0,
                DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT,
            );
        } else {
            self.push_dirty(node.0);
        }
    }

    fn remove_attribute(&mut self, node: NodeId, name: &str) {
        self.tree.nodes[node.0].attributes.remove(name);
        if name == "class" || name == "style" {
            self.push_dirty_flags(
                node.0,
                DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT,
            );
            // Invalidate Stylo element data so styles are recomputed
            *self.tree.nodes[node.0].stylo_element_data.borrow_mut() = None;
            self.tree.nodes[node.0].style_attribute_cache = None;
            self.tree.styles_dirty = true;
        } else {
            self.push_dirty(node.0);
        }
    }

    fn get_attribute(&self, node: NodeId, name: &str) -> Option<String> {
        self.tree.nodes.get(node.0)?.attributes.get(name).cloned()
    }

    fn set_style(&mut self, node: NodeId, property: &str, value: &str) {
        let mut styles: HashMap<String, String> = self.tree.nodes[node.0]
            .attributes
            .get("style")
            .map(|s| parse_style_string(s))
            .unwrap_or_default();
        styles.insert(property.to_string(), value.to_string());
        let style_str = styles
            .iter()
            .map(|(k, v)| format!("{}: {}", k, v))
            .collect::<Vec<_>>()
            .join("; ");
        self.tree.nodes[node.0]
            .attributes
            .insert("style".to_string(), style_str.clone());

        // Parse inline style into Stylo PropertyDeclarationBlock (same as set_attribute)
        use style::properties::parse_style_attribute;
        use style::stylesheets::CssRuleType;
        use url::Url;

        let url = Url::parse("about:blank").unwrap();
        let extra_data = style::stylesheets::UrlExtraData::from(url);
        let pdb = parse_style_attribute(
            &style_str,
            &extra_data,
            None, // error_reporter
            style::context::QuirksMode::NoQuirks,
            CssRuleType::Style,
        );
        self.tree.nodes[node.0].style_attribute_cache =
            Some(ServoArc::new(self.tree.guard.wrap(pdb)));

        // Invalidate cached Stylo data and resolve styles
        *self.tree.nodes[node.0].stylo_element_data.borrow_mut() = None;
        self.tree.styles_dirty = true;
        self.resolve_styles();
        self.apply_stylo_styles_to_taffy();
        self.push_dirty_flags(
            node.0,
            DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT,
        );
    }

    fn mark_dirty(&mut self, node: NodeId) {
        self.push_dirty(node.0);
    }

    fn take_dirty_nodes(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.tree.dirty_nodes)
            .into_iter()
            .map(NodeId)
            .collect()
    }

    fn root(&self) -> NodeId {
        NodeId(self.tree.root_id)
    }

    fn body(&self) -> NodeId {
        NodeId(self.tree.body_id)
    }

    fn query_selector(&self, selector: &str) -> Option<NodeId> {
        // Simple selector matching: supports #id, .class, tag
        self.query_recursive(self.tree.root_id, selector)
            .map(NodeId)
    }

    fn query_selector_all(&self, selector: &str) -> Vec<NodeId> {
        // Query all matching nodes
        let mut results = Vec::new();
        self.query_all_recursive(self.tree.root_id, selector, &mut results);
        results.into_iter().map(NodeId).collect()
    }

    fn get_children(&self, node: NodeId) -> Vec<NodeId> {
        self.tree
            .nodes
            .get(node.0)
            .map(|n| n.children.iter().map(|&c| NodeId(c)).collect())
            .unwrap_or_default()
    }

    fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize) {
        let p = parent.0;
        let c = child.0;
        // Invalidate old IFC
        self.invalidate_ifc_for_node(c);
        self.clear_ifc_root_recursive(c);
        // Remove from old parent if any
        if let Some(old_parent) = self.tree.nodes[c].parent {
            self.tree.nodes[old_parent].children.retain(|&x| x != c);
            if let (Some(old_taffy_parent), Some(child_taffy)) = (
                self.tree.nodes[old_parent].taffy_id,
                self.tree.nodes[c].taffy_id,
            ) {
                self.taffy_remove_child_safe(old_taffy_parent, child_taffy);
            }
        }
        self.tree.nodes[c].parent = Some(p);
        let len = self.tree.nodes[p].children.len();
        let actual_index = if index >= len {
            self.tree.nodes[p].children.push(c);
            len
        } else {
            self.tree.nodes[p].children.insert(index, c);
            index
        };
        // Sync taffy
        if let (Some(parent_taffy), Some(child_taffy)) =
            (self.tree.nodes[p].taffy_id, self.tree.nodes[c].taffy_id)
        {
            let taffy_idx = self.compute_taffy_child_index(p, actual_index);
            let _ = self
                .tree
                .taffy
                .insert_child_at_index(parent_taffy, taffy_idx, child_taffy);
        }
        self.invalidate_parent_ifc(p);
        self.push_dirty_flags(p, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);

        // Recompute styles for the inserted subtree to pick up ancestor-based selectors
        self.recompute_node_styles_recursive(c);
    }

    fn parent_node(&self, node: NodeId) -> Option<NodeId> {
        self.tree.nodes.get(node.0)?.parent.map(NodeId)
    }

    fn next_sibling(&self, node: NodeId) -> Option<NodeId> {
        let parent_id = self.tree.nodes.get(node.0)?.parent?;
        let siblings = &self.tree.nodes[parent_id].children;
        let pos = siblings.iter().position(|&c| c == node.0)?;
        siblings.get(pos + 1).map(|&c| NodeId(c))
    }

    fn parse_html(&mut self, _html: &str) -> Option<NodeId> {
        // Phase 9: HTML parser integration
        // For now, return None
        None
    }

    fn set_scroll_top(&mut self, node: NodeId, scroll_top: f64) {
        if let Some(n) = self.tree.nodes.get_mut(node.0) {
            n.scroll_offset.1 = scroll_top;
        }
        self.push_dirty_flags(node.0, DirtyFlags::PAINT);
    }

    fn set_inner_html(&mut self, node: NodeId, html: &str) {
        use crate::html_parser::parse_html_string;

        // Clear existing children (including taffy sync)
        let old_children: Vec<_> = self.tree.nodes[node.0].children.clone();
        for child in old_children {
            self.clear_ifc_root_recursive(child);
            // Remove from taffy parent
            if let (Some(parent_taffy), Some(child_taffy)) = (
                self.tree.nodes[node.0].taffy_id,
                self.tree.nodes[child].taffy_id,
            ) {
                self.taffy_remove_child_safe(parent_taffy, child_taffy);
            }
            self.tree.nodes[child].parent = None;
            self.tree.remove_subtree(child);
        }
        self.tree.nodes[node.0].children.clear();

        // Parse HTML and create nodes
        if let Some(parsed_nodes) = parse_html_string(html) {
            for parsed in parsed_nodes {
                let child_id = self.create_node_from_parsed(&parsed);
                self.append_child(node, child_id);
            }
        }

        self.invalidate_parent_ifc(node.0);
        self.push_dirty_flags(node.0, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);
    }

    fn query_caret_position(&self, node_id: u64, byte_offset: usize) -> Option<(f32, f32)> {
        use crate::text_query::caret_position_for_offset;
        caret_position_for_offset(self, node_id, byte_offset)
    }

    fn query_glyph_bounds(
        &self,
        node_id: u64,
        byte_offset: usize,
    ) -> Option<rinch_core::dom::GlyphBounds> {
        use crate::text_query::glyph_bounds_for_offset;
        let bounds = glyph_bounds_for_offset(self, node_id, byte_offset)?;
        Some(rinch_core::dom::GlyphBounds {
            x: bounds.x,
            y: bounds.y,
            width: bounds.width,
            height: bounds.height,
        })
    }

    fn focus_element(&mut self, node_id: NodeId) {
        // Request focus via the event system - the runtime will apply it
        rinch_core::request_focus(node_id.0);
    }

    fn resolve_layout(&mut self, width: f32, height: f32) {
        // Delegate to the existing implementation method
        RinchDocument::resolve_layout(self, width, height);
    }

    fn query_node_layout(&self, node_id: u64) -> Option<(f32, f32, f32, f32)> {
        let node = self.tree.nodes.get(node_id as usize)?;
        Some((
            node.layout.x,
            node.layout.y,
            node.layout.width,
            node.layout.height,
        ))
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
    pub fn query_selector_all(&self, selector: &str) -> Vec<NodeId> {
        let mut results = Vec::new();
        self.query_all_recursive(self.tree.root_id, selector, &mut results);
        results.into_iter().map(NodeId).collect()
    }
}

/// Parse a CSS style string like "display: flex; gap: 8px" into key-value pairs.
fn parse_style_string(style: &str) -> HashMap<String, String> {
    let mut result = HashMap::new();
    for part in style.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((key, value)) = part.split_once(':') {
            result.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    result
}
