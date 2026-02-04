//! DomDocument implementation for rinch-dom.

use std::collections::HashMap;

use rinch_core::dom::{DomDocument, NodeId};

use peniko::Brush;
use peniko::color::{AlphaColor, Srgb};
use servo_arc::Arc as ServoArc;

// Stylo CSS engine imports
use style::context::QuirksMode;
use style::media_queries::{Device, MediaType};
use style::properties::style_structs::Font as StyloFont;
use style::properties::ComputedValues;
use style::queries::values::PrefersColorScheme;
use style::stylist::Stylist;
use style::values::computed::{CSSPixelLength, Length};
use style::values::specified::font::QueryFontMetricsFlags;
use style::font_metrics::FontMetrics;
use style::values::computed::font::GenericFontFamily;
use euclid::Scale;
use stylo_config as style_config;
// CSSPixel and DevicePixel are used via euclid::Size2D type parameters

use crate::layout;
use crate::node::{DirtyFlags, Node, NodeKind, NodeTree, NodeContext, TextMeasure, LayoutResult, DisplayMode, InlineLayout};
use crate::computed_style::ComputedStyle;

/// A simple FontMetricsProvider that returns default/fixed values.
/// This is used by the Stylist's Device to resolve font-relative units.
#[derive(Debug)]
struct SimpleFontMetricsProvider;

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
        use style::stylesheets::{Stylesheet, Origin, UrlExtraData, AllowImportRules, DocumentStyleSheet};
        use style::media_queries::MediaList;

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
        self.stylist.force_stylesheet_origins_dirty(Origin::UserAgent.into());
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
    fn push_dirty_flags(&mut self, node_id: usize, flags: DirtyFlags) {
        if self.tree.contains(node_id) {
            self.tree.nodes[node_id].dirty.insert(flags);
            self.tree.push_dirty(node_id);
            if flags.contains(DirtyFlags::LAYOUT) {
                self.mark_dirty_up(node_id, DirtyFlags::LAYOUT);
            }
        }
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
        let taffy_id = self.tree.taffy.new_leaf(taffy::Style {
            display: taffy::Display::Flex,
            flex_direction: if is_block { taffy::FlexDirection::Column } else { taffy::FlexDirection::Row },
            flex_wrap: taffy::FlexWrap::NoWrap,
            ..Default::default()
        }).unwrap();
        node.taffy_id = Some(taffy_id);
        self.tree.taffy_map.insert(taffy_id, id);
        self.tree.nodes.insert(node);

        // Hidden elements should not participate in layout
        if matches!(tag, "style" | "script" | "head" | "meta" | "link" | "title") {
            let _ = self.tree.taffy.set_style(taffy_id, taffy::Style {
                display: taffy::Display::None,
                ..Default::default()
            });
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
            no_wrap: false, // default, updated from parent
        });
        let taffy_id = self.tree.taffy.new_leaf_with_context(taffy::Style::default(), context).unwrap();
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
        if let (Some(parent_taffy), Some(child_taffy)) = (
            self.tree.nodes[p].taffy_id,
            self.tree.nodes[c].taffy_id,
        ) {
            let _ = self.tree.taffy.add_child(parent_taffy, child_taffy);
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
        if let (Some(parent_taffy), Some(child_taffy)) = (
            self.tree.nodes[p].taffy_id,
            self.tree.nodes[c].taffy_id,
        ) {
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
        let insert_pos = if let Some(pos) = self.tree.nodes[p].children.iter().position(|&x| x == r) {
            self.tree.nodes[p].children.insert(pos, c);
            Some(pos)
        } else {
            self.tree.nodes[p].children.push(c);
            None
        };
        // Sync taffy
        if let (Some(parent_taffy), Some(child_taffy)) = (
            self.tree.nodes[p].taffy_id,
            self.tree.nodes[c].taffy_id,
        ) {
            if let Some(pos) = insert_pos {
                // Count taffy children before this position to find taffy index
                let taffy_idx = self.compute_taffy_child_index(p, pos);
                let _ = self.tree.taffy.insert_child_at_index(parent_taffy, taffy_idx, child_taffy);
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
            if let Some(pos) = self.tree.nodes[parent_id].children.iter().position(|&x| x == old.0) {
                self.tree.nodes[parent_id].children[pos] = new.0;
                // Sync taffy: remove old, insert new at same position
                if let Some(parent_taffy) = self.tree.nodes[parent_id].taffy_id {
                    if let Some(old_taffy) = self.tree.nodes[old.0].taffy_id {
                        self.taffy_remove_child_safe(parent_taffy, old_taffy);
                    }
                    if let Some(new_taffy) = self.tree.nodes[new.0].taffy_id {
                        let taffy_idx = self.compute_taffy_child_index(parent_id, pos);
                        let _ = self.tree.taffy.insert_child_at_index(parent_taffy, taffy_idx, new_taffy);
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
                    if let Some(ctx) = self.tree.taffy.get_node_context_mut(taffy_id) {
                        if let NodeContext::Text(tm) = ctx {
                            tm.content = text.to_string();
                        }
                    }
                    let _ = self.tree.taffy.mark_dirty(taffy_id);
                }
            }
            _ => {
                // For elements: remove all children and add a text child
                let old_children: Vec<_> = self.tree.nodes[n].children.clone();
                for child in old_children {
                    self.tree.nodes[child].parent = None;
                    // Remove from taffy parent
                    if let (Some(parent_taffy), Some(child_taffy)) = (
                        self.tree.nodes[n].taffy_id,
                        self.tree.nodes[child].taffy_id,
                    ) {
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
                });
                let taffy_id = self.tree.taffy.new_leaf_with_context(taffy::Style::default(), context).unwrap();
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
        self.tree.nodes[node.0].attributes.insert(name.to_string(), value.to_string());

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
        let needs_style_recompute = name == "class" || name == "style"
            || ((name == "width" || name == "height" || name == "viewBox")
                && self.tree.nodes[node.0].tag() == Some("svg"));

        if needs_style_recompute {
            // Invalidate cached Stylo data and resolve styles for this subtree
            *self.tree.nodes[node.0].stylo_element_data.borrow_mut() = None;
            self.resolve_styles();
            self.apply_stylo_styles_to_taffy();
            self.push_dirty_flags(node.0, DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT);
        } else {
            self.push_dirty(node.0);
        }
    }

    fn remove_attribute(&mut self, node: NodeId, name: &str) {
        self.tree.nodes[node.0].attributes.remove(name);
        if name == "class" || name == "style" {
            self.push_dirty_flags(node.0, DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT);
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
        let style_str = styles.iter()
            .map(|(k, v)| format!("{}: {}", k, v))
            .collect::<Vec<_>>()
            .join("; ");
        self.tree.nodes[node.0].attributes.insert("style".to_string(), style_str.clone());

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
        self.resolve_styles();
        self.apply_stylo_styles_to_taffy();
        self.push_dirty_flags(node.0, DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT);
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
        self.query_recursive(self.tree.root_id, selector).map(NodeId)
    }

    fn query_selector_all(&self, selector: &str) -> Vec<NodeId> {
        // Query all matching nodes
        let mut results = Vec::new();
        self.query_all_recursive(self.tree.root_id, selector, &mut results);
        results.into_iter().map(NodeId).collect()
    }

    fn get_children(&self, node: NodeId) -> Vec<NodeId> {
        self.tree.nodes.get(node.0)
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
        if let (Some(parent_taffy), Some(child_taffy)) = (
            self.tree.nodes[p].taffy_id,
            self.tree.nodes[c].taffy_id,
        ) {
            let taffy_idx = self.compute_taffy_child_index(p, actual_index);
            let _ = self.tree.taffy.insert_child_at_index(parent_taffy, taffy_idx, child_taffy);
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

    fn query_glyph_bounds(&self, node_id: u64, byte_offset: usize) -> Option<rinch_core::dom::GlyphBounds> {
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
}

impl RinchDocument {
    /// Load CSS into the document's stylesheet.
    ///
    /// Parses the CSS string and merges rules/variables into the existing stylesheet.
    /// Call this at startup to load theme and widget CSS.
    pub fn load_css(&mut self, css: &str) {
        self.load_stylo_css(css);
    }

    /// Create a DOM node from a parsed HTML node (recursive).
    fn create_node_from_parsed(&mut self, parsed: &crate::html_parser::ParsedNode) -> NodeId {
        use crate::html_parser::ParsedNode;
        use rinch_core::dom::DomDocument;

        match parsed {
            ParsedNode::Element { tag, attrs, children } => {
                let node_id = self.create_element(tag);

                // Set attributes
                for (name, value) in attrs {
                    self.set_attribute(node_id, name, value);
                }

                // Recursively create and append children
                for child in children {
                    let child_id = self.create_node_from_parsed(child);
                    self.append_child(node_id, child_id);
                }

                node_id
            }
            ParsedNode::Text(text) => {
                self.create_text(text)
            }
        }
    }

    /// If the given node is a `<style>` element, extract its text children's content
    /// and load it into the stylesheet.
    fn maybe_load_style_css(&mut self, node_id: usize) {
        let is_style = self.tree.nodes.get(node_id)
            .and_then(|n| n.tag())
            .map(|t| t == "style")
            .unwrap_or(false);
        if !is_style { return; }

        // Collect text content from children
        let children: Vec<usize> = self.tree.nodes[node_id].children.clone();
        let mut css = String::new();
        for child_id in children {
            if let Some(text) = self.tree.nodes.get(child_id).and_then(|n| n.text_content()) {
                css.push_str(text);
            }
        }
        if !css.is_empty() {
            self.load_stylo_css(&css);
            // Recompute all styles since new CSS rules may affect existing nodes
            self.resolve_styles();
            self.apply_stylo_styles_to_taffy();
        }
    }

    /// Set viewport dimensions for resolving vh/vw CSS units.
    /// This also updates the Stylo Device so that vh/vw units resolve correctly.
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        self.set_stylist_viewport(width, height);
    }

    /// Set viewport dimensions for the Stylo Device.
    /// Call this when the window is resized to update media queries and viewport units.
    pub fn set_stylist_viewport(&mut self, width: f32, height: f32) {
        use style::shared_lock::StylesheetGuards;
        use style::stylesheets::Origin;

        // Update our internal viewport tracking
        self.tree.viewport = crate::layout::Viewport { width, height };

        // Create a new Device with the updated viewport
        let viewport_size = euclid::Size2D::new(width, height);
        let device_pixel_ratio = Scale::new(1.0);
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

        // Update the stylist's device using StylesheetGuards
        let guard = self.tree.guard.read();
        let guards = StylesheetGuards::same(&guard);
        self.stylist.set_device(device, &guards);

        // Mark all stylesheet origins as dirty to force style recomputation with new viewport
        self.stylist.force_stylesheet_origins_dirty(Origin::UserAgent.into());
        self.stylist.force_stylesheet_origins_dirty(Origin::Author.into());
    }

    /// Load CSS into Stylo's stylesheet system.
    ///
    /// Parses the CSS string and adds it to the Stylist for CSS cascade.
    /// This is the Stylo-based replacement for the old stylesheet system.
    pub fn load_stylo_css(&mut self, css: &str) {
        use style::stylesheets::{Stylesheet, Origin, UrlExtraData, AllowImportRules, DocumentStyleSheet};
        use style::media_queries::MediaList;

        // Create a dummy URL for the stylesheet
        let url_data = UrlExtraData::from(
            ::url::Url::parse("about:blank").expect("about:blank is a valid URL")
        );

        // Parse the CSS into a stylesheet
        let media = ServoArc::new(self.tree.guard.wrap(MediaList::empty()));
        let stylesheet = Stylesheet::from_str(
            css,
            url_data,
            Origin::Author,
            media,
            self.tree.guard.clone(),
            None, // stylesheet_loader
            None, // error_reporter
            QuirksMode::NoQuirks,
            AllowImportRules::Yes,
        );

        // Wrap in DocumentStyleSheet for the Stylist
        let doc_stylesheet = DocumentStyleSheet(ServoArc::new(stylesheet));

        // Add the stylesheet to the stylist
        let guard = self.tree.guard.read();
        self.stylist.append_stylesheet(doc_stylesheet, &guard);

        // Mark stylesheets as changed so they'll be flushed on next style computation
        self.stylist.force_stylesheet_origins_dirty(Origin::Author.into());
    }

    /// Resolve styles for all elements using Stylo's CSS cascade.
    ///
    /// This walks the DOM tree and computes styles for each element using:
    /// 1. Selector matching via `push_applicable_declarations()`
    /// 2. Rule tree construction via `compute_rule_node()`
    /// 3. Cascade via `cascade_style_and_visited()`
    ///
    /// The computed styles are stored in each node's `stylo_element_data` field.
    pub fn resolve_styles(&mut self) {
        use style::shared_lock::StylesheetGuards;
        use crate::stylo_impl::RinchNode;

        // Flush any pending stylesheet changes
        {
            let guard = self.tree.guard.read();
            let guards = StylesheetGuards::same(&guard);
            self.stylist.flush::<RinchNode>(&guards, None, None);
        }

        // Start from the html element and traverse down
        let html_id = self.tree.html_id;
        self.resolve_styles_recursive(html_id, None);
    }

    /// Recursively resolve styles for a node and its descendants.
    fn resolve_styles_recursive(
        &mut self,
        node_id: usize,
        parent_style: Option<ServoArc<ComputedValues>>,
    ) {
        use style::applicable_declarations::ApplicableDeclarationList;
        use style::context::CascadeInputs;
        use style::data::ElementData;
        use style::properties::FirstLineReparenting;
        use style::rule_cache::RuleCacheConditions;
        use style::shared_lock::StylesheetGuards;
        use style::stylist::RuleInclusion;
        use selectors::matching::{MatchingContext, MatchingMode, NeedsSelectorFlags, MatchingForInvalidation, VisitedHandlingMode, IncludeStartingStyle, SelectorCaches};

        use crate::stylo_impl::RinchNode;

        // Extract node info in a block to release borrows before recursion
        let (is_element, children, cached_style) = {
            let node = match self.tree.nodes.get(node_id) {
                Some(n) => n,
                None => return,
            };

            let is_element = node.is_element();
            let children: Vec<usize> = node.children.clone();

            // Check for cached style
            let cached_style = {
                let stylo_data = node.stylo_element_data.borrow();
                stylo_data.as_ref().and_then(|d| d.styles.primary.clone())
            };

            (is_element, children, cached_style)
        };

        // Skip non-element nodes
        if !is_element {
            // For text nodes, just recurse to children (shouldn't have any)
            for child_id in children {
                self.resolve_styles_recursive(child_id, parent_style.clone());
            }
            return;
        }

        // PERFORMANCE: Skip nodes that already have computed styles (cache hit)
        // When a node's style changes, its stylo_element_data is set to None,
        // causing it to be recomputed. Nodes with valid cached styles are skipped.
        if let Some(computed) = cached_style {
            for child_id in children {
                self.resolve_styles_recursive(child_id, Some(computed.clone()));
            }
            return;
        }

        // Compute styles in a block so borrows are dropped before recursion
        let (computed, children) = {
            // Create the RinchNode wrapper for Stylo
            let rinch_node = RinchNode::new(node_id, &self.tree);

            // Set up matching context
            let guard = self.tree.guard.read();
            let guards = StylesheetGuards::same(&guard);

            let mut selector_caches = SelectorCaches::default();
            let mut matching_context = MatchingContext::new_for_visited(
                MatchingMode::Normal,
                None, // bloom filter - could add for performance
                &mut selector_caches,
                VisitedHandlingMode::AllLinksUnvisited,
                IncludeStartingStyle::No,
                self.stylist.quirks_mode(),
                NeedsSelectorFlags::No,
                MatchingForInvalidation::No,
            );

            // Collect applicable declarations
            let mut applicable_declarations = ApplicableDeclarationList::new();

            // Get the style attribute (already parsed and cached)
            let style_attribute = rinch_node.node().style_attribute_cache
                .as_ref()
                .map(|arc| arc.borrow_arc());

            self.stylist.push_applicable_declarations(
                rinch_node,
                None, // pseudo_element
                style_attribute,
                None, // smil_override
                Default::default(), // animation_declarations
                RuleInclusion::All,
                &mut applicable_declarations,
                &mut matching_context,
            );

            // Build rule node from applicable declarations
            let rule_node = self.stylist
                .rule_tree()
                .compute_rule_node(&mut applicable_declarations, &guards);

            // Cascade to compute final styles
            let parent_style_ref = parent_style.as_ref().map(|s| &**s);
            let mut rule_cache_conditions = RuleCacheConditions::default();

            let computed = self.stylist.cascade_style_and_visited(
                Some(rinch_node),
                None, // pseudo
                CascadeInputs {
                    rules: Some(rule_node),
                    visited_rules: None,
                    flags: matching_context.extra_data.cascade_input_flags,
                },
                &guards,
                parent_style_ref, // parent_style
                parent_style_ref, // layout_parent_style
                FirstLineReparenting::No,
                &Default::default(), // try_tactic (PositionTryFallbacksTryTactic)
                None, // rule_cache
                &mut rule_cache_conditions,
            );

            // Store the computed style in the node's ElementData
            {
                let mut stylo_data = self.tree.nodes[node_id].stylo_element_data.borrow_mut();
                if stylo_data.is_none() {
                    *stylo_data = Some(ElementData::default());
                }
                if let Some(ref mut data) = *stylo_data {
                    data.styles.primary = Some(computed.clone());
                }
            }

            // Mark this node as needing Taffy sync (style was recomputed)
            self.tree.style_dirty_nodes.push(node_id);

            // Clone children list before returning
            let children: Vec<usize> = self.tree.nodes[node_id].children.clone();

            (computed.clone(), children)
        };

        // Now we can recurse without holding borrows
        for child_id in children {
            self.resolve_styles_recursive(child_id, Some(computed.clone()));
        }
    }

    /// Update hover state: set the hovered node and its ancestors as hovered,
    /// clear previous hover, and recompute styles for affected nodes.
    /// Returns true if the hovered node changed (caller should repaint).
    pub fn update_hover(&mut self, new_hovered: Option<usize>) -> bool {
        let old_hovered = self.tree.hovered_node;
        if old_hovered == new_hovered {
            return false;
        }

        // Collect old hovered chain (node + ancestors)
        let mut old_chain = Vec::new();
        if let Some(old_id) = old_hovered {
            let mut current = Some(old_id);
            while let Some(id) = current {
                old_chain.push(id);
                current = self.tree.nodes.get(id).and_then(|n| n.parent);
            }
        }

        // Collect new hovered chain (node + ancestors)
        let mut new_chain = Vec::new();
        if let Some(new_id) = new_hovered {
            let mut current = Some(new_id);
            while let Some(id) = current {
                new_chain.push(id);
                current = self.tree.nodes.get(id).and_then(|n| n.parent);
            }
        }

        // Clear old hover state
        for &id in &old_chain {
            if let Some(node) = self.tree.nodes.get_mut(id) {
                node.is_hovered = false;
            }
        }

        // Set new hover state
        for &id in &new_chain {
            if let Some(node) = self.tree.nodes.get_mut(id) {
                node.is_hovered = true;
            }
        }

        self.tree.hovered_node = new_hovered;

        // Recompute styles for nodes that changed hover state
        // (nodes in old chain but not new, and vice versa)
        let mut dirty_nodes: Vec<usize> = Vec::new();
        for &id in &old_chain {
            if !new_chain.contains(&id) {
                dirty_nodes.push(id);
            }
        }
        for &id in &new_chain {
            if !old_chain.contains(&id) {
                dirty_nodes.push(id);
            }
        }

        // Recompute styles using Stylo for affected nodes
        // For simplicity, we recompute styles for the entire tree
        // (a more optimized approach would only restyle the affected subtrees)
        self.resolve_styles();
        self.apply_stylo_styles_to_taffy();

        // Mark dirty nodes for repaint
        for id in dirty_nodes {
            self.push_dirty_flags(id, DirtyFlags::STYLE | DirtyFlags::PAINT);
        }

        true
    }

    /// Get the default display type for a node based on its tag.
    fn default_display_for_node(&self, node_id: usize) -> layout::DefaultDisplay {
        match self.tree.nodes[node_id].display_mode {
            crate::node::DisplayMode::Inline | crate::node::DisplayMode::InlineBlock => layout::DefaultDisplay::Inline,
            _ => layout::DefaultDisplay::Block,
        }
    }

    /// Update theme CSS variables without duplicating non-`:root` rules.
    /// After calling this, call `recompute_all_styles_full()` to apply the new variables.
    pub fn update_theme_variables(&mut self, css: &str) {
        // Load CSS variables into Stylo
        self.load_stylo_css(css);
    }

    /// Recompute taffy styles for all element nodes, clearing cached style props
    /// so that CSS variables are re-resolved. Use this after `update_theme_variables()`.
    pub fn recompute_all_styles_full(&mut self) {
        // Clear cached Stylo element data so styles are recomputed
        let node_ids: Vec<usize> = self.tree.nodes.iter().map(|(id, _)| id).collect();
        for &nid in &node_ids {
            *self.tree.nodes[nid].stylo_element_data.borrow_mut() = None;
        }
        // Resolve styles using Stylo
        self.resolve_styles();
        self.apply_stylo_styles_to_taffy();
    }

    /// Recompute taffy styles for all element nodes.
    /// Called when viewport dimensions change to update vh/vw-dependent styles.
    fn recompute_all_styles(&mut self) {
        // When viewport changes, Stylo needs to know about it to recalculate vh/vw units
        // For now, just resolve styles and apply to Taffy
        self.resolve_styles();
        self.apply_stylo_styles_to_taffy();
    }


    /// Recompute styles recursively for a node and all its descendants.
    /// This is needed when a node is inserted into a new parent, as ancestor-based
    /// CSS selectors (like `.parent .child`) need to be re-evaluated with the new ancestor chain.
    fn recompute_node_styles_recursive(&mut self, node_id: usize) {
        if !self.tree.nodes[node_id].is_element() {
            return;
        }

        // Invalidate cached Stylo data for this node and its descendants
        fn invalidate_recursive(tree: &mut NodeTree, node_id: usize) {
            *tree.nodes[node_id].stylo_element_data.borrow_mut() = None;
            let children = tree.nodes[node_id].children.clone();
            for &child_id in &children {
                invalidate_recursive(tree, child_id);
            }
        }
        invalidate_recursive(&mut self.tree, node_id);

        // Resolve styles using Stylo
        self.resolve_styles();
        self.apply_stylo_styles_to_taffy();
        self.push_dirty_flags(node_id, DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT);
    }


    /// Compute the taffy child index for a DOM child at the given position.
    /// This counts only children that have taffy IDs (skipping comments).
    fn compute_taffy_child_index(&self, parent_id: usize, dom_index: usize) -> usize {
        let children = &self.tree.nodes[parent_id].children;
        let mut taffy_idx = 0;
        for i in 0..dom_index {
            if i < children.len() {
                if self.tree.nodes[children[i]].taffy_id.is_some() {
                    taffy_idx += 1;
                }
            }
        }
        taffy_idx
    }

    /// Apply Stylo computed styles to Taffy layout nodes.
    ///
    /// This reads from each element's `stylo_element_data` and sets the corresponding
    /// Taffy style. It also updates our `ComputedStyle` for paint operations.
    ///
    /// PERFORMANCE: Only processes nodes in `style_dirty_nodes` (set by resolve_styles).
    pub fn apply_stylo_styles_to_taffy(&mut self) {
        // Take the dirty nodes list - only these need Taffy sync
        let dirty_node_ids = std::mem::take(&mut self.tree.style_dirty_nodes);

        // If no dirty nodes, nothing to do
        if dirty_node_ids.is_empty() {
            return;
        }

        for node_id in dirty_node_ids {
            // Skip root and html nodes - their Taffy styles are manually set
            if node_id == self.tree.root_id || node_id == self.tree.html_id {
                continue;
            }

            let node = match self.tree.nodes.get(node_id) {
                Some(n) => n,
                None => continue,
            };

            if !node.is_element() {
                continue;
            }

            // Skip elements that should never participate in layout
            if matches!(node.tag(), Some("style" | "script" | "head" | "meta" | "link" | "title")) {
                continue;
            }

            // Get Taffy node ID
            let taffy_id = match node.taffy_id {
                Some(id) => id,
                None => continue,
            };

            // Get computed values from Stylo
            let stylo_data = node.stylo_element_data.borrow();
            let computed_values = match stylo_data.as_ref().and_then(|d| d.styles.get_primary()) {
                Some(cv) => cv.clone(),
                None => continue,
            };
            drop(stylo_data);

            // Convert Stylo ComputedValues to our ComputedStyle
            let computed_style = ComputedStyle::from_stylo(&computed_values);

            // Update node's computed_style for paint operations
            self.tree.nodes[node_id].computed_style = computed_style.clone();

            // Sync display_mode from computed style
            let display_mode = match computed_style.display {
                crate::computed_style::DisplayValue::Inline => DisplayMode::Inline,
                crate::computed_style::DisplayValue::InlineBlock => DisplayMode::InlineBlock,
                crate::computed_style::DisplayValue::InlineFlex => DisplayMode::Flex,
                crate::computed_style::DisplayValue::Block => DisplayMode::Block,
                crate::computed_style::DisplayValue::Flex => DisplayMode::Flex,
                crate::computed_style::DisplayValue::Grid => DisplayMode::Block, // Grid treated as block for IFC
                crate::computed_style::DisplayValue::None => DisplayMode::Block,
                crate::computed_style::DisplayValue::Contents => DisplayMode::Block,
            };
            self.tree.nodes[node_id].display_mode = display_mode;

            // Convert to Taffy style
            let dd = self.default_display_for_node(node_id);
            let mut taffy_style = computed_style.to_taffy_style(dd);

            // Body node needs flex_grow: 1 and height: auto to fill the viewport
            if node_id == self.tree.body_id {
                if taffy_style.flex_grow == 0.0 {
                    taffy_style.flex_grow = 1.0;
                }
                if taffy_style.size.height == taffy::Dimension::auto() {
                    taffy_style.size.height = taffy::Dimension::auto();
                }
                if taffy_style.size.width == taffy::Dimension::auto() {
                    taffy_style.size.width = taffy::Dimension::percent(1.0);
                }
            }

            // Apply to Taffy node
            let _ = self.tree.taffy.set_style(taffy_id, taffy_style);
        }
    }

    /// Resolve layout using Taffy.
    ///
    /// Computes layout for the entire tree given a viewport size,
    /// then reads layout results back into each node's `layout` field.
    /// Text nodes are measured using Parley for accurate text layout.
    pub fn resolve_layout(&mut self, width: f32, height: f32) {
        let old_viewport = self.tree.viewport;
        self.tree.viewport = crate::layout::Viewport { width, height };

        // When viewport changes, update Stylo's Device and invalidate all cached styles
        // so that vh/vw units are recomputed with the new viewport dimensions
        if (old_viewport.width - width).abs() > 0.5 || (old_viewport.height - height).abs() > 0.5 {
            self.set_stylist_viewport(width, height);

            // Invalidate all cached stylo_element_data so styles are recomputed with new viewport
            for (node_id, _) in self.tree.nodes.iter() {
                *self.tree.nodes[node_id].stylo_element_data.borrow_mut() = None;
            }
        }

        // Resolve Stylo styles and apply to Taffy nodes
        self.resolve_styles();
        self.apply_stylo_styles_to_taffy();

        let root_taffy = match self.tree.nodes[self.tree.root_id].taffy_id {
            Some(id) => id,
            None => return,
        };

        // Handle display:contents by rebuilding taffy children for affected nodes
        self.sync_display_contents();

        // Detect and set up inline formatting contexts
        self.setup_inline_formatting_contexts();

        // Pre-compute layout for inline-block children that were detached from Taffy.
        // They need their own subtree measured so walk_inline_children can read dimensions.
        self.compute_inline_block_layouts();

        // Sync font-size from parent elements to text node contexts
        self.sync_text_contexts();

        let available_space = taffy::Size {
            width: taffy::AvailableSpace::Definite(width),
            height: taffy::AvailableSpace::Definite(height),
        };

        let font_cx = &mut self.font_cx;
        let layout_cx = &mut self.layout_cx;
        let mut paint_layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
        let nodes = &self.tree.nodes;

        // Cache for text layouts built during measurement.
        // Key: (node_id, wrap_width as bits) - wrap width is part of key since layout depends on it
        // Value: Parley layout
        use std::cell::RefCell;
        let text_layout_cache: RefCell<HashMap<(usize, u32), parley::layout::Layout<Brush>>> = RefCell::new(HashMap::new());

        self.tree.taffy.compute_layout_with_measure(
            root_taffy,
            available_space,
            |known_dims, avail_space, _node_id, context, _style| {
                let max_width = match avail_space.width {
                    taffy::AvailableSpace::Definite(w) => Some(w),
                    taffy::AvailableSpace::MaxContent => None,
                    taffy::AvailableSpace::MinContent => Some(0.0),
                };

                match context {
                    Some(NodeContext::Text(text)) => {
                        if text.content.is_empty() {
                            return taffy::Size { width: 0.0, height: 0.0 };
                        }

                        let mut builder = layout_cx.ranged_builder(font_cx, &text.content, 1.0, true);
                        builder.push_default(parley::style::StyleProperty::FontSize(text.font_size));
                        if (text.font_weight - 400.0).abs() > 1.0 {
                            builder.push_default(parley::style::StyleProperty::FontWeight(
                                parley::style::FontWeight::new(text.font_weight),
                            ));
                        }
                        if let Some(lh) = layout::css_line_height_to_parley(&text.line_height_css) {
                            builder.push_default(parley::style::StyleProperty::LineHeight(lh));
                        }
                        let font_stack = if !text.font_family.is_empty() {
                            std::borrow::Cow::Owned(text.font_family.clone())
                        } else {
                            std::borrow::Cow::Borrowed("sans-serif")
                        };
                        builder.push_default(parley::style::StyleProperty::FontStack(
                            parley::style::FontStack::Source(font_stack),
                        ));
                        // Add brush so the cached layout can be rendered with color
                        builder.push_default(parley::style::StyleProperty::Brush(Brush::Solid(text.color)));
                        let mut layout = builder.build(&text.content);
                        // If no_wrap is set (white-space: nowrap), don't constrain width
                        let wrap_width = if text.no_wrap {
                            None
                        } else {
                            known_dims.width.or(max_width)
                        };
                        layout.break_all_lines(wrap_width);

                        // Cache the layout for use during paint
                        // Use wrap_width bits as part of the key since layout depends on it
                        let wrap_bits = wrap_width.map(|w| w.to_bits()).unwrap_or(u32::MAX);
                        text_layout_cache.borrow_mut().insert((text.node_id, wrap_bits), layout);

                        taffy::Size {
                            width: known_dims.width.unwrap_or_else(|| {
                                text_layout_cache.borrow().get(&(text.node_id, wrap_bits)).map(|l| l.width()).unwrap_or(0.0)
                            }),
                            height: known_dims.height.unwrap_or_else(|| {
                                text_layout_cache.borrow().get(&(text.node_id, wrap_bits)).map(|l| l.height()).unwrap_or(0.0)
                            }),
                        }
                    }
                    Some(NodeContext::InlineRoot(root_id)) => {
                        // Build Parley inline layout for this IFC root
                        let root_id = *root_id;
                        let inline_layout = Self::build_inline_layout(
                            nodes,
                            root_id,
                            max_width,
                            1.0,
                            font_cx,
                            &mut paint_layout_cx,
                        );
                        let w = inline_layout.layout.width();
                        let h = inline_layout.layout.height();
                        // Measure callback for IFC root
                        taffy::Size {
                            width: known_dims.width.unwrap_or(w),
                            height: known_dims.height.unwrap_or(h),
                        }
                    }
                    _ => taffy::Size::ZERO,
                }
            },
        ).unwrap();

        // Read layout results back into nodes
        self.read_layout_results(self.tree.root_id);

        // Build inline layouts for IFC roots (rebuild with final widths and store)
        self.build_ifc_layouts(&mut paint_layout_cx);

        // Copy cached text layouts to nodes (use the exact layouts from measurement)
        self.copy_cached_text_layouts(text_layout_cache.into_inner());
    }

    /// Sync font-size from parent elements into text node contexts.
    ///
    /// Walks all text nodes and updates their `TextMeasure.font_size`
    /// from the parent element's computed style.
    fn sync_text_contexts(&mut self) {
        use crate::computed_style::WhiteSpaceValue;
        let mut updates: Vec<(taffy::NodeId, usize, f32, f32, String, String, AlphaColor<Srgb>, bool)> = Vec::new();

        for (id, node) in &self.tree.nodes {
            if let NodeKind::Text(_) = &node.kind {
                let taffy_id = match node.taffy_id {
                    Some(t) => t,
                    None => continue,
                };

                // Read from parent's parsed computed_style instead of parsing CSS strings
                let (font_size, font_weight, font_family, line_height_css, color, no_wrap) = node.parent
                    .and_then(|p| self.tree.nodes.get(p))
                    .map(|parent| {
                        let font_size = parent.computed_style.font_size;
                        let font_weight = parent.computed_style.font_weight;
                        let font_family = if parent.computed_style.font_family.is_empty() {
                            "sans-serif".to_string()
                        } else {
                            parent.computed_style.font_family.clone()
                        };
                        let line_height_css = match &parent.computed_style.line_height {
                            crate::computed_style::LineHeightValue::Normal => String::new(),
                            crate::computed_style::LineHeightValue::Absolute(v) => format!("{}px", v),
                            crate::computed_style::LineHeightValue::Relative(v) => v.to_string(),
                        };
                        let color = parent.computed_style.color
                            .unwrap_or_else(|| AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255));
                        // Check if white-space prevents wrapping
                        let no_wrap = matches!(
                            parent.computed_style.white_space,
                            WhiteSpaceValue::NoWrap | WhiteSpaceValue::Pre
                        );
                        (font_size, font_weight, font_family, line_height_css, color, no_wrap)
                    })
                    .unwrap_or((16.0, 400.0, "sans-serif".to_string(), String::new(), AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255), false));

                updates.push((taffy_id, id, font_size, font_weight, font_family, line_height_css, color, no_wrap));
            }
        }

        for (taffy_id, node_id, font_size, font_weight, font_family, line_height_css, color, no_wrap) in updates {
            if let Some(ctx) = self.tree.taffy.get_node_context_mut(taffy_id) {
                if let NodeContext::Text(tm) = ctx {
                    tm.font_size = font_size;
                    tm.font_weight = font_weight;
                    tm.font_family = font_family;
                    tm.line_height_css = line_height_css;
                    tm.node_id = node_id;
                    tm.color = color;
                    tm.no_wrap = no_wrap;
                }
            }
        }
    }

    /// Recursively read Taffy layout results into node LayoutResult fields.
    fn read_layout_results(&mut self, node_id: usize) {
        let children: Vec<usize> = self.tree.nodes[node_id].children.clone();

        if let Some(taffy_id) = self.tree.nodes[node_id].taffy_id {
            if let Ok(taffy_layout) = self.tree.taffy.layout(taffy_id) {
                let node = &mut self.tree.nodes[node_id];
                node.layout = LayoutResult {
                    x: taffy_layout.location.x,
                    y: taffy_layout.location.y,
                    width: taffy_layout.size.width,
                    height: taffy_layout.size.height,
                };
            }
        }

        for child_id in children {
            self.read_layout_results(child_id);
        }
    }

    /// Handle display:contents nodes by reparenting their taffy children
    /// to the taffy parent of the display:contents node.
    fn sync_display_contents(&mut self) {
        // Collect display:contents nodes
        let mut contents_nodes = Vec::new();
        for (id, node) in &self.tree.nodes {
            if let Some(style_str) = node.attributes.get("style") {
                if layout::is_display_contents(style_str) {
                    contents_nodes.push(id);
                }
            }
        }

        for node_id in contents_nodes {
            let parent_id = match self.tree.nodes[node_id].parent {
                Some(p) => p,
                None => continue,
            };
            let parent_taffy = match self.tree.nodes[parent_id].taffy_id {
                Some(t) => t,
                None => continue,
            };
            let node_taffy = match self.tree.nodes[node_id].taffy_id {
                Some(t) => t,
                None => continue,
            };

            // Remove the contents node from taffy parent
            self.taffy_remove_child_safe(parent_taffy, node_taffy);

            // Find the position of this node among parent's DOM children to know where
            // to insert its children in the taffy tree
            let parent_children: Vec<usize> = self.tree.nodes[parent_id].children.clone();
            let dom_pos = parent_children.iter().position(|&c| c == node_id).unwrap_or(0);

            // Compute taffy insert index (count taffy-having siblings before this position,
            // excluding the contents node itself)
            let mut taffy_insert_idx = 0;
            for i in 0..dom_pos {
                let sibling_id = parent_children[i];
                if sibling_id != node_id && self.tree.nodes[sibling_id].taffy_id.is_some() {
                    // Check if sibling is NOT also display:contents (already removed)
                    let is_contents = self.tree.nodes[sibling_id].attributes.get("style")
                        .map(|s| layout::is_display_contents(s))
                        .unwrap_or(false);
                    if !is_contents {
                        taffy_insert_idx += 1;
                    }
                }
            }

            // Add children of contents node directly to taffy parent
            let grandchildren: Vec<usize> = self.tree.nodes[node_id].children.clone();
            for (i, &grandchild_id) in grandchildren.iter().enumerate() {
                if let Some(gc_taffy) = self.tree.nodes[grandchild_id].taffy_id {
                    // Remove from contents node's taffy
                    self.taffy_remove_child_safe(node_taffy, gc_taffy);
                    let _ = self.tree.taffy.insert_child_at_index(parent_taffy, taffy_insert_idx + i, gc_taffy);
                }
            }

            // Set the contents node's taffy to display:none with zero size
            let _ = self.tree.taffy.set_style(node_taffy, taffy::Style {
                display: taffy::Display::None,
                ..Default::default()
            });
        }
    }

    /// Invalidate the IFC that owns a node (if any).
    ///
    /// Clears the IFC root's cached text_layout so it rebuilds on next layout pass.
    /// Also checks the parent's text_layout as a fallback when ifc_root hasn't been
    /// set yet (before the first layout pass).
    fn invalidate_ifc_for_node(&mut self, node_id: usize) {
        if let Some(ifc_root_id) = self.tree.nodes.get(node_id).and_then(|n| n.ifc_root) {
            if let Some(root) = self.tree.nodes.get_mut(ifc_root_id) {
                root.text_layout = None;
            }
        } else {
            // Fallback: walk ancestors to find one with text_layout (the IFC root)
            let mut cur = self.tree.nodes.get(node_id).and_then(|n| n.parent);
            while let Some(pid) = cur {
                if self.tree.nodes.get(pid).map(|p| p.text_layout.is_some()).unwrap_or(false) {
                    self.tree.nodes[pid].text_layout = None;
                    break;
                }
                cur = self.tree.nodes.get(pid).and_then(|n| n.parent);
            }
        }
    }

    /// Safely remove a child from a Taffy parent, checking membership first.
    /// Taffy's `remove_child` panics if the child isn't actually a child of the parent,
    /// which can happen when inline children were detached by `setup_inline_formatting_contexts`.
    fn taffy_remove_child_safe(&mut self, parent_taffy: taffy::NodeId, child_taffy: taffy::NodeId) {
        if let Ok(children) = self.tree.taffy.children(parent_taffy) {
            if children.contains(&child_taffy) {
                let _ = self.tree.taffy.remove_child(parent_taffy, child_taffy);
            }
        }
    }

    /// Clear ifc_root on a node and all its descendants.
    fn clear_ifc_root_recursive(&mut self, node_id: usize) {
        // Use iterative approach to avoid stack overflow
        let mut stack = vec![node_id];
        while let Some(id) = stack.pop() {
            if let Some(node) = self.tree.nodes.get_mut(id) {
                node.ifc_root = None;
                stack.extend(node.children.iter().copied());
            }
        }
    }

    /// Invalidate IFC state for a parent element.
    /// Clears text_layout on the parent and ifc_root on all its inline children.
    /// Also marks the Taffy node dirty so the measure callback re-fires.
    fn invalidate_parent_ifc(&mut self, parent_id: usize) {
        if let Some(parent) = self.tree.nodes.get_mut(parent_id) {
            parent.text_layout = None;
        }
        if let Some(taffy_id) = self.tree.nodes.get(parent_id).and_then(|n| n.taffy_id) {
            let _ = self.tree.taffy.mark_dirty(taffy_id);
        }
        let children: Vec<usize> = self.tree.nodes.get(parent_id)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        for child_id in children {
            if let Some(child) = self.tree.nodes.get_mut(child_id) {
                child.ifc_root = None;
            }
        }
    }

    /// Build inline layouts for all IFC roots after Taffy layout.
    ///
    /// Uses the computed width from Taffy as the available width for Parley line breaking.
    fn build_ifc_layouts(&mut self, paint_layout_cx: &mut parley::LayoutContext<Brush>) {
        // Collect IFC roots (elements that have inline children with ifc_root set)
        let mut ifc_roots: Vec<usize> = Vec::new();
        for (id, node) in &self.tree.nodes {
            if !node.is_element() { continue; }
            if matches!(node.display_mode, DisplayMode::Inline | DisplayMode::InlineBlock | DisplayMode::Flex) { continue; }
            // Check if any child has ifc_root pointing to this node
            let is_ifc = node.children.iter().any(|&child_id| {
                self.tree.nodes.get(child_id)
                    .map(|c| c.ifc_root == Some(id))
                    .unwrap_or(false)
            });
            if is_ifc {
                ifc_roots.push(id);
            }
        }

        for root_id in ifc_roots {
            let node = &self.tree.nodes[root_id];
            // Only constrain width if the element has an explicit width set.
            // For auto-width elements, don't re-constrain text to measured width
            // as floating-point precision can cause unwanted line breaks.
            let max_width = match node.computed_style.width {
                crate::computed_style::DimensionValue::Auto => None,
                _ => {
                    let available_width = node.layout.width;
                    if available_width > 0.0 { Some(available_width) } else { None }
                }
            };

            let inline_layout = Self::build_inline_layout(
                &self.tree.nodes,
                root_id,
                max_width,
                1.0, // scale=1.0 for layout; paint scales later
                &mut self.font_cx,
                paint_layout_cx,
            );

            // Write positions from Parley layout back to child nodes
            // Walk the layout lines to find positioned inline boxes and text runs
            self.write_inline_positions(root_id, &inline_layout);

            self.tree.nodes[root_id].text_layout = Some(Box::new(inline_layout));
        }
    }

    /// Copy cached text layouts from measurement to nodes.
    ///
    /// Uses the exact layouts built during Taffy measurement to ensure
    /// paint uses identical text shaping results.
    fn copy_cached_text_layouts(&mut self, cache: HashMap<(usize, u32), parley::layout::Layout<Brush>>) {
        // First collect node IDs and their layouts to apply
        let updates: Vec<(usize, parley::layout::Layout<Brush>)> = self.tree.nodes.iter()
            .filter_map(|(id, node)| {
                if node.ifc_root.is_some() { return None; } // Skip IFC-managed nodes
                if !matches!(&node.kind, NodeKind::Text(_)) { return None; }

                let width = node.layout.width;
                let wrap_bits = if width > 0.0 { width.to_bits() } else { u32::MAX };

                // Try to find a cached layout for this node with the final width
                if let Some(layout) = cache.get(&(id, wrap_bits)) {
                    return Some((id, layout.clone()));
                }

                // Fallback: prefer the MaxContent layout (wrap_bits = u32::MAX) which has natural width.
                // This avoids picking a MinContent layout that was wrapped to 0 width.
                if let Some(layout) = cache.get(&(id, u32::MAX)) {
                    return Some((id, layout.clone()));
                }
                // Last resort: find any cached layout for this node (prefer widest/tallest ratio)
                let mut best: Option<&parley::layout::Layout<Brush>> = None;
                for ((nid, _), layout) in &cache {
                    if *nid == id {
                        // Prefer single-line layouts (lower height) over wrapped ones
                        if best.is_none() || layout.height() < best.unwrap().height() {
                            best = Some(layout);
                        }
                    }
                }
                best.map(|layout| (id, layout.clone()))
            })
            .collect();

        // Apply the updates
        for (id, layout) in updates {
            self.tree.nodes[id].cached_text_parley = Some(Box::new(layout));
        }
    }

    /// Write computed positions from an InlineLayout back into child node layout fields.
    fn write_inline_positions(&mut self, root_id: usize, inline_layout: &InlineLayout) {
        let root_layout = self.tree.nodes[root_id].layout;

        // Walk Parley layout lines to find positioned items
        for line in inline_layout.layout.lines() {
            for item in line.items() {
                match item {
                    parley::layout::PositionedLayoutItem::GlyphRun(_) => {
                        // Text runs don't map to individual child nodes
                    }
                    parley::layout::PositionedLayoutItem::InlineBox(positioned_box) => {
                        let child_id = positioned_box.id as usize;
                        if let Some(child) = self.tree.nodes.get_mut(child_id) {
                            child.layout.x = positioned_box.x;
                            child.layout.y = positioned_box.y;
                        }
                    }
                }
            }
        }

        // For text nodes that are direct children, set their layout to cover the
        // IFC root's content area (they're painted via Parley, not individually)
        let children: Vec<usize> = self.tree.nodes[root_id].children.clone();
        for child_id in children {
            if let Some(child) = self.tree.nodes.get(child_id) {
                if child.is_text() && child.ifc_root == Some(root_id) {
                    // Text nodes in IFC get zero layout — they're painted by the IFC root
                    // But we set their position relative to root for hit testing
                    if let Some(child) = self.tree.nodes.get_mut(child_id) {
                        child.layout.x = 0.0;
                        child.layout.y = 0.0;
                        child.layout.width = root_layout.width;
                        child.layout.height = root_layout.height;
                    }
                }
            }
        }
    }

    /// Detect IFC roots and mark inline children.
    ///
    /// An element is an IFC root if it's a block/flex container that has
    /// inline content that benefits from unified Parley layout — specifically:
    /// - Multiple inline children (text + text, text + inline element, etc.)
    /// - At least one inline element (span, em, etc.)
    ///
    /// Single text children under block parents continue using the existing
    /// Taffy text measurement path (no IFC needed).
    fn setup_inline_formatting_contexts(&mut self) {
        let mut ifc_roots: Vec<usize> = Vec::new();
        for (id, node) in &self.tree.nodes {
            if !node.is_element() { continue; }
            // Only block containers can be IFC roots — skip inline, inline-block, and flex
            if matches!(node.display_mode, DisplayMode::Inline | DisplayMode::InlineBlock | DisplayMode::Flex) { continue; }

            let inline_children: Vec<usize> = node.children.iter()
                .filter(|&&child_id| {
                    self.tree.nodes.get(child_id)
                        .map(|c| c.is_inline())
                        .unwrap_or(false)
                })
                .copied()
                .collect();

            // Only activate IFC when there's actual inline formatting context complexity
            let has_inline_elements = inline_children.iter().any(|&child_id| {
                self.tree.nodes.get(child_id)
                    .map(|c| matches!(c.kind, NodeKind::Element(_)) && matches!(c.display_mode, DisplayMode::Inline | DisplayMode::InlineBlock))
                    .unwrap_or(false)
            });
            let needs_ifc = has_inline_elements || inline_children.len() > 1;

            if needs_ifc {
                ifc_roots.push(id);
            }
        }

        for root_id in ifc_roots {
            let children: Vec<usize> = self.tree.nodes[root_id].children.clone();
            let root_taffy = match self.tree.nodes[root_id].taffy_id {
                Some(t) => t,
                None => continue,
            };

            // Remove inline children from Taffy (Parley will handle their layout)
            for &child_id in &children {
                let child = match self.tree.nodes.get(child_id) {
                    Some(c) => c,
                    None => continue,
                };
                if child.is_inline() {
                    if let Some(child_taffy) = child.taffy_id {
                        if let Ok(taffy_children) = self.tree.taffy.children(root_taffy) {
                            if taffy_children.contains(&child_taffy) {
                                let _ = self.tree.taffy.remove_child(root_taffy, child_taffy);
                            }
                        }
                    }
                    if let Some(c) = self.tree.nodes.get_mut(child_id) {
                        c.ifc_root = Some(root_id);
                    }
                }
            }

            // Set NodeContext::InlineRoot on the IFC root's Taffy node
            // so the measure function fires for it
            if let Some(ctx) = self.tree.taffy.get_node_context_mut(root_taffy) {
                *ctx = NodeContext::InlineRoot(root_id);
            } else {
                // Element nodes don't have context by default — we need to set one.
                // Taffy only calls measure for nodes with context, so we must ensure it has one.
                let _ = self.tree.taffy.set_node_context(root_taffy, Some(NodeContext::InlineRoot(root_id)));
            }
        }
    }

    /// Pre-compute layout for inline-block children that were detached from Taffy.
    ///
    /// Inline-block children are removed from their parent's Taffy tree (so the parent
    /// uses InlineRoot measurement), but they still need their own subtree computed
    /// so `walk_inline_children` can read their width/height for Parley InlineBox.
    fn compute_inline_block_layouts(&mut self) {
        // Collect inline-block children that belong to an IFC
        let mut ib_taffy_ids: Vec<taffy::NodeId> = Vec::new();
        for (_id, node) in &self.tree.nodes {
            if node.ifc_root.is_some() && node.display_mode == DisplayMode::InlineBlock {
                if let Some(taffy_id) = node.taffy_id {
                    ib_taffy_ids.push(taffy_id);
                }
            }
        }

        let font_cx = &mut self.font_cx;
        let layout_cx = &mut self.layout_cx;

        for taffy_id in ib_taffy_ids {
            let avail = taffy::Size {
                width: taffy::AvailableSpace::MaxContent,
                height: taffy::AvailableSpace::MaxContent,
            };
            let _ = self.tree.taffy.compute_layout_with_measure(
                taffy_id,
                avail,
                |known_dims, avail_space, _node_id, context, _style| {
                    let max_width = match avail_space.width {
                        taffy::AvailableSpace::Definite(w) => Some(w),
                        taffy::AvailableSpace::MaxContent => None,
                        taffy::AvailableSpace::MinContent => Some(0.0),
                    };
                    match context {
                        Some(NodeContext::Text(text)) => {
                            if text.content.is_empty() {
                                return taffy::Size::ZERO;
                            }
                            let mut builder = layout_cx.ranged_builder(font_cx, &text.content, 1.0, true);
                            builder.push_default(parley::style::StyleProperty::FontSize(text.font_size));
                            if (text.font_weight - 400.0).abs() > 1.0 {
                                builder.push_default(parley::style::StyleProperty::FontWeight(
                                    parley::style::FontWeight::new(text.font_weight),
                                ));
                            }
                            if let Some(lh) = layout::css_line_height_to_parley(&text.line_height_css) {
                                builder.push_default(parley::style::StyleProperty::LineHeight(lh));
                            }
                            let font_stack = if !text.font_family.is_empty() {
                                std::borrow::Cow::Owned(text.font_family.clone())
                            } else {
                                std::borrow::Cow::Borrowed("sans-serif")
                            };
                            builder.push_default(parley::style::StyleProperty::FontStack(
                                parley::style::FontStack::Source(font_stack),
                            ));
                            let mut layout = builder.build(&text.content);
                            // If no_wrap is set (white-space: nowrap), don't constrain width
                            let wrap_width = if text.no_wrap {
                                None
                            } else {
                                known_dims.width.or(max_width)
                            };
                            layout.break_all_lines(wrap_width);
                            taffy::Size {
                                width: known_dims.width.unwrap_or(layout.width()),
                                height: known_dims.height.unwrap_or(layout.height()),
                            }
                        }
                        _ => taffy::Size::ZERO,
                    }
                },
            );

            // Read the computed layout back into the node
            if let Ok(taffy_layout) = self.tree.taffy.layout(taffy_id) {
                let node_id = self.tree.taffy_map.get(&taffy_id).copied();
                if let Some(nid) = node_id {
                    if let Some(node) = self.tree.nodes.get_mut(nid) {
                        node.layout.width = taffy_layout.size.width;
                        node.layout.height = taffy_layout.size.height;
                    }
                }
            }
        }
    }

    /// Build a Parley inline layout for an IFC root node.
    ///
    /// Walks the IFC root's children, collecting text nodes and inline elements
    /// into a single Parley TreeBuilder layout. Returns the InlineLayout.
    fn build_inline_layout(
        nodes: &slab::Slab<Node>,
        root_id: usize,
        max_width: Option<f32>,
        scale: f32,
        font_cx: &mut parley::FontContext,
        layout_cx: &mut parley::LayoutContext<Brush>,
    ) -> InlineLayout {
        // Get root style properties from typed ComputedStyle
        let root_computed = &nodes[root_id].computed_style;
        let root_font_size = root_computed.font_size * scale;
        let root_color = root_computed.color.unwrap_or_else(|| {
            peniko::color::AlphaColor::<peniko::color::Srgb>::from_rgba8(0, 0, 0, 255)
        });

        let font_family: std::borrow::Cow<'static, str> = if root_computed.font_family.is_empty() {
            "sans-serif".into()
        } else {
            root_computed.font_family.clone().into()
        };

        let mut root_text_style = parley::style::TextStyle {
            font_size: root_font_size,
            brush: Brush::Solid(root_color),
            font_stack: parley::style::FontStack::Source(font_family),
            ..Default::default()
        };

        // Apply font-weight from computed style
        root_text_style.font_weight = parley::style::FontWeight::new(root_computed.font_weight);

        // Apply font-style from computed style
        root_text_style.font_style = root_computed.font_style.to_parley();

        // Apply line-height from computed style
        if let Some(lh) = root_computed.line_height.to_parley() {
            root_text_style.line_height = lh;
        }

        // Apply text-decoration from computed style
        root_text_style.has_underline = root_computed.text_decoration.underline;
        root_text_style.has_strikethrough = root_computed.text_decoration.strikethrough;

        let mut builder = layout_cx.tree_builder(font_cx, scale, true, &root_text_style);

        // Apply white-space mode from computed style
        use crate::computed_style::WhiteSpaceValue;
        let collapse = match root_computed.white_space {
            WhiteSpaceValue::Pre | WhiteSpaceValue::PreWrap | WhiteSpaceValue::PreLine => {
                parley::style::WhiteSpaceCollapse::Preserve
            }
            _ => parley::style::WhiteSpaceCollapse::Collapse,
        };
        builder.set_white_space_mode(collapse);

        let mut child_positions = Vec::new();

        // Walk children and build the Parley tree
        Self::walk_inline_children(nodes, root_id, &mut builder, &mut child_positions, scale);

        let (text_layout, text_content) = builder.build();
        let mut text_layout = text_layout;
        text_layout.break_all_lines(max_width);

        // Apply text-align from computed style
        let alignment = root_computed.text_align.to_parley();
        text_layout.align(alignment, parley::layout::AlignmentOptions::default());

        InlineLayout {
            layout: text_layout,
            text_content,
            child_positions,
        }
    }

    /// Recursively walk inline children, pushing text and style spans into the TreeBuilder.
    fn walk_inline_children(
        nodes: &slab::Slab<Node>,
        parent_id: usize,
        builder: &mut parley::TreeBuilder<'_, Brush>,
        child_positions: &mut Vec<(usize, LayoutResult)>,
        scale: f32,
    ) {
        let children: Vec<usize> = nodes[parent_id].children.clone();
        for child_id in children {
            let child = match nodes.get(child_id) {
                Some(c) => c,
                None => continue,
            };
            match &child.kind {
                NodeKind::Text(text_data) => {
                    if !text_data.content.is_empty() {
                        builder.push_text(&text_data.content);
                        // Record position placeholder — actual position comes from layout
                        child_positions.push((child_id, LayoutResult::default()));
                    }
                }
                NodeKind::Element(_) if child.display_mode == DisplayMode::Inline => {
                    // Push style span for inline element using typed ComputedStyle
                    let child_computed = &child.computed_style;
                    let mut props: Vec<parley::style::StyleProperty<'_, Brush>> = Vec::new();

                    // Font size (always apply scaled)
                    props.push(parley::style::StyleProperty::FontSize(
                        child_computed.font_size * scale,
                    ));

                    // Font weight
                    props.push(parley::style::StyleProperty::FontWeight(
                        parley::style::FontWeight::new(child_computed.font_weight),
                    ));

                    // Font style
                    props.push(parley::style::StyleProperty::FontStyle(
                        child_computed.font_style.to_parley(),
                    ));

                    // Color
                    if let Some(color) = child_computed.color {
                        props.push(parley::style::StyleProperty::Brush(Brush::Solid(color)));
                    }

                    // Text decoration
                    if child_computed.text_decoration.underline {
                        props.push(parley::style::StyleProperty::Underline(true));
                    }
                    if child_computed.text_decoration.strikethrough {
                        props.push(parley::style::StyleProperty::Strikethrough(true));
                    }

                    // Line height
                    if let Some(lh) = child_computed.line_height.to_parley() {
                        // Scale absolute line heights
                        let scaled_lh = match lh {
                            parley::style::LineHeight::Absolute(v) => {
                                parley::style::LineHeight::Absolute(v * scale)
                            }
                            other => other,
                        };
                        props.push(parley::style::StyleProperty::LineHeight(scaled_lh));
                    }

                    builder.push_style_modification_span(props.iter());
                    child_positions.push((child_id, LayoutResult::default()));

                    // Recurse into inline element's children
                    Self::walk_inline_children(nodes, child_id, builder, child_positions, scale);

                    builder.pop_style_span();
                }
                NodeKind::Element(_) if child.display_mode == DisplayMode::InlineBlock => {
                    // Inline-block: measure via Taffy first, then embed as InlineBox
                    let child_layout = &child.layout;
                    builder.push_inline_box(parley::InlineBox {
                        id: child_id as u64,
                        index: 0, // will be set by builder
                        width: child_layout.width * scale,
                        height: child_layout.height * scale,
                        kind: parley::InlineBoxKind::InFlow,
                    });
                    child_positions.push((child_id, LayoutResult::default()));
                }
                NodeKind::Comment(_) => {
                    // Skip comments in inline layout
                }
                _ => {
                    // Block children break inline flow — stop here
                    break;
                }
            }
        }
    }

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
            if let Some(classes) = node.attributes.get("class") {
                if classes.split_whitespace().any(|c| c == class) {
                    return Some(node_id);
                }
            }
        }
        // Match by attribute selector [attr] or [attr=value]
        else if let Some(attr_sel) = selector.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
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
        let Some(node) = self.tree.nodes.get(node_id) else { return };

        let matches = if let Some(id) = selector.strip_prefix('#') {
            node.attributes.get("id").map(|v| v.as_str()) == Some(id)
        } else if let Some(class) = selector.strip_prefix('.') {
            node.attributes.get("class")
                .map(|classes| classes.split_whitespace().any(|c| c == class))
                .unwrap_or(false)
        } else if let Some(attr_sel) = selector.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
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
        if part.is_empty() { continue; }
        if let Some((key, value)) = part.split_once(':') {
            result.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    result
}
