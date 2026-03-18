//! `DomDocument` trait implementation for `RinchDocument`.

use std::collections::HashMap;

use rinch_core::dom::{DomDocument, NodeId};

use peniko::color::{AlphaColor, Srgb};
use servo_arc::Arc as ServoArc;

use style::context::QuirksMode;

use crate::node::{DirtyFlags, DisplayMode, Node, NodeContext, NodeKind, TextMeasure};

use super::{RinchDocument, parse_style_string};

impl DomDocument for RinchDocument {
    fn create_element(&mut self, tag: &str) -> NodeId {
        let id = self.tree.nodes.vacant_key();
        let mut node = Node::element(id, tag, self.tree.guard.clone());
        // Use CSS-standard defaults based on element type:
        // Block elements (div, p, h1, etc.): flex-column (emulates block stacking)
        // Inline elements (span, a, etc.): flex-row
        let is_block = matches!(node.display_mode, DisplayMode::Block);
        let default_style = taffy::Style {
            display: taffy::Display::Flex,
            flex_direction: if is_block {
                taffy::FlexDirection::Column
            } else {
                taffy::FlexDirection::Row
            },
            flex_wrap: taffy::FlexWrap::NoWrap,
            ..Default::default()
        };
        let taffy_id = if tag == "img" {
            // Image elements use NodeContext::Image for intrinsic sizing
            let context = NodeContext::Image {
                src: String::new(),
                width: 0,
                height: 0,
            };
            self.tree
                .taffy
                .new_leaf_with_context(default_style, context)
                .unwrap()
        } else {
            self.tree.taffy.new_leaf(default_style).unwrap()
        };
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
            text_overflow: crate::computed_style::TextOverflowValue::default(),
            parent_overflow_hidden: false,
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
        self.tree.layout_dirty = true; // Structural change needs full layout
        self.tree.ifc_dirty = true; // Tree structure changed
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
        self.tree.layout_dirty = true; // Structural change needs full layout
        self.tree.ifc_dirty = true; // Tree structure changed
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
        self.tree.layout_dirty = true; // Structural change needs full layout
        self.tree.ifc_dirty = true; // Tree structure changed
        self.push_dirty_flags(p, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);

        // Recompute styles for the inserted subtree to pick up ancestor-based selectors
        self.recompute_node_styles_recursive(c);
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
            self.tree.layout_dirty = true; // Structural change needs full layout
            self.tree.ifc_dirty = true; // Tree structure changed
            self.push_dirty_flags(parent_id, DirtyFlags::LAYOUT | DirtyFlags::CHILDREN);

            // Recompute styles for the new subtree to pick up ancestor-based selectors
            self.recompute_node_styles_recursive(new.0);
        }
    }

    fn remove_node(&mut self, node: NodeId) {
        // Mark the removed subtree as paint-dirty so the dirty region
        // includes the old layout positions (e.g., borders, backgrounds).
        // Without this, the old pixels aren't cleared on repaint.
        self.mark_subtree_paint_dirty(node.0);

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
            self.tree.layout_dirty = true; // Structural change needs full layout
            self.tree.ifc_dirty = true; // Tree structure changed
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
        // Track which IFC root has dirty text content so the measure callback
        // can skip expensive Parley rebuilds for unchanged roots.
        {
            let ifc_root = self.tree.nodes[n]
                .ifc_root
                .or_else(|| self.tree.nodes[n].parent);
            if let Some(root_id) = ifc_root {
                self.tree.dirty_ifc_text_roots.insert(root_id);
                // Invalidate cached measure results for this root
                self.tree
                    .ifc_measure_cache
                    .retain(|&(rid, _), _| rid != root_id);
            }
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
                    text_overflow: crate::computed_style::TextOverflowValue::default(),
                    parent_overflow_hidden: false,
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
                self.tree.ifc_dirty = true; // Structural change (children replaced)
            }
        }
        self.tree.layout_dirty = true; // Text content change affects layout
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
            if value.trim().is_empty() {
                // Empty/blank style: clear the cache entirely so Stylo sees None
                // (not Some(empty_pdb)) and falls back to class-based styles.
                self.tree.nodes[node.0].style_attribute_cache = None;
            } else {
                use style::properties::parse_style_attribute;
                use style::stylesheets::CssRuleType;
                use url::Url;

                let url = Url::parse("about:blank").unwrap();
                let extra_data = style::stylesheets::UrlExtraData::from(url);
                let pdb = parse_style_attribute(
                    value,
                    &extra_data,
                    None, // error_reporter
                    QuirksMode::NoQuirks,
                    CssRuleType::Style,
                );
                self.tree.nodes[node.0].style_attribute_cache =
                    Some(ServoArc::new(self.tree.guard.wrap(pdb)));
            }
        }
        // Invalidate IFC if this node belongs to one (style/class changes affect inline layout).
        // Only for nodes that actually participate in inline formatting — block elements
        // don't need IFC invalidation and the mark_dirty propagation would defeat Taffy's cache.
        if (name == "style" || name == "class")
            && (self.tree.nodes[node.0].ifc_root.is_some()
                || self.tree.nodes[node.0].is_inline()
                || self.tree.nodes[node.0].text_layout.is_some())
        {
            self.invalidate_ifc_for_node(node.0);
            // Also invalidate parent's IFC in case this is an inline child
            if let Some(parent_id) = self.tree.nodes[node.0].parent {
                self.invalidate_parent_ifc(parent_id);
            }
        }
        // Handle <img src="..."> — trigger async image load
        if name == "src" && self.tree.nodes[node.0].tag() == Some("img") {
            self.request_image_load_for_node(node.0, value);
        }

        // SVG elements: width/height HTML attributes affect layout sizing
        let needs_style_recompute = name == "class"
            || name == "style"
            || ((name == "width" || name == "height" || name == "viewBox")
                && self.tree.nodes[node.0].tag() == Some("svg"));

        if needs_style_recompute {
            // Invalidate cached Stylo data — deferred to resolve_layout()
            *self.tree.nodes[node.0].stylo_element_data.borrow_mut() = None;
            self.tree.style_roots.push(node.0);
            self.tree.styles_dirty = true;
            self.push_dirty_flags(
                node.0,
                DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT,
            );

            // Class changes can affect descendant selectors (e.g. `.parent--active .child`),
            // so invalidate all descendants' cached Stylo data too. The style_root above
            // ensures resolve_styles_recursive walks from this node down, and clearing
            // descendants' caches forces Stylo to re-match selectors against the new class.
            if name == "class" {
                self.invalidate_descendant_styles(node.0);
            }
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
            self.tree.style_roots.push(node.0);
            self.tree.styles_dirty = true;
            // Class removal affects descendant selectors
            if name == "class" {
                self.invalidate_descendant_styles(node.0);
            }
        } else {
            self.push_dirty(node.0);
        }
    }

    fn get_attribute(&self, node: NodeId, name: &str) -> Option<String> {
        self.tree.nodes.get(node.0)?.attributes.get(name).cloned()
    }

    fn set_style(&mut self, node: NodeId, property: &str, value: &str) {
        // ── Fast path: inset-only changes on absolute/fixed elements ─────
        // Changing left/top/right/bottom on an out-of-flow element only moves
        // it within its containing block — children and siblings are unaffected.
        // Skip Stylo re-parse and style resolution entirely. Update
        // ComputedStyle and Taffy inset directly.
        //
        // Taffy's set_style calls mark_dirty which clears this node's cache,
        // but children's caches are preserved. On next compute_layout, Taffy
        // recomputes this node's position (cheap) and children hit their cache
        // (free). The expensive part we skip is Stylo parse_style_attribute +
        // style resolution + IFC invalidation.
        if matches!(property, "left" | "top" | "right" | "bottom")
            && matches!(
                self.tree.nodes[node.0].computed_style.position,
                crate::computed_style::PositionValue::Absolute
                    | crate::computed_style::PositionValue::Fixed
            )
        {
            // Update the style attribute string (for consistency / serialization)
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

            // Re-parse PDB so Stylo stays consistent for future full re-resolutions.
            // This is cheap — it's the cascade/resolution we're skipping.
            {
                use style::properties::parse_style_attribute;
                use style::stylesheets::CssRuleType;
                use url::Url;
                let url = Url::parse("about:blank").unwrap();
                let extra_data = style::stylesheets::UrlExtraData::from(url);
                let pdb = parse_style_attribute(
                    &style_str,
                    &extra_data,
                    None,
                    QuirksMode::NoQuirks,
                    CssRuleType::Style,
                );
                self.tree.nodes[node.0].style_attribute_cache =
                    Some(ServoArc::new(self.tree.guard.wrap(pdb)));
            }

            // Update ComputedStyle directly (skip Stylo)
            let vp = &crate::layout::Viewport::default();
            let new_val = crate::computed_style::LengthPercentageAutoValue::parse(value, vp);
            match property {
                "left" => self.tree.nodes[node.0].computed_style.left = new_val,
                "top" => self.tree.nodes[node.0].computed_style.top = new_val,
                "right" => self.tree.nodes[node.0].computed_style.right = new_val,
                "bottom" => self.tree.nodes[node.0].computed_style.bottom = new_val,
                _ => unreachable!(),
            }

            // Update Taffy inset directly (skip full style resolution).
            // mark_dirty clears this node's cache but NOT children's caches,
            // so children won't be re-measured.
            if let Some(taffy_id) = self.tree.nodes[node.0].taffy_id {
                if let Ok(mut taffy_style) = self.tree.taffy.style(taffy_id).cloned() {
                    let taffy_val = new_val.to_taffy();
                    match property {
                        "left" => taffy_style.inset.left = taffy_val,
                        "top" => taffy_style.inset.top = taffy_val,
                        "right" => taffy_style.inset.right = taffy_val,
                        "bottom" => taffy_style.inset.bottom = taffy_val,
                        _ => unreachable!(),
                    }
                    let _ = self.tree.taffy.set_style(taffy_id, taffy_style);
                    self.tree.layout_dirty = true;
                }
            }

            // Mark dirty and request full repaint so the old position gets cleared.
            self.tree.nodes[node.0]
                .dirty
                .insert(DirtyFlags::LAYOUT | DirtyFlags::PAINT);
            self.tree.dirty_nodes.insert(node.0);
            self.tree.full_repaint_needed = true;
            return;
        }

        // ── Normal path: full Stylo re-parse ─────────────────────────────
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
            QuirksMode::NoQuirks,
            CssRuleType::Style,
        );
        self.tree.nodes[node.0].style_attribute_cache =
            Some(ServoArc::new(self.tree.guard.wrap(pdb)));

        // Invalidate cached Stylo data — deferred to resolve_layout()
        *self.tree.nodes[node.0].stylo_element_data.borrow_mut() = None;
        self.tree.style_roots.push(node.0);
        self.tree.styles_dirty = true;
        // Only invalidate IFC state for nodes that participate in inline formatting.
        // Block elements (like slider track) don't affect IFC layout, and the
        // mark_dirty() in invalidate_parent_ifc would propagate to root, defeating
        // Taffy's cache for ALL InlineRoot measure callbacks.
        if self.tree.nodes[node.0].ifc_root.is_some()
            || self.tree.nodes[node.0].is_inline()
            || self.tree.nodes[node.0].text_layout.is_some()
        {
            self.invalidate_ifc_for_node(node.0);
            if let Some(parent_id) = self.tree.nodes[node.0].parent {
                self.invalidate_parent_ifc(parent_id);
            }
        }
        self.push_dirty_flags(
            node.0,
            DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT,
        );
    }

    fn set_styles(&mut self, node: NodeId, properties: &[(&str, &str)]) {
        // ── Fast path: inset-only batch on absolute/fixed elements ────────
        let all_inset = !properties.is_empty()
            && properties
                .iter()
                .all(|(p, _)| matches!(*p, "left" | "top" | "right" | "bottom"));
        let is_out_of_flow = matches!(
            self.tree.nodes[node.0].computed_style.position,
            crate::computed_style::PositionValue::Absolute
                | crate::computed_style::PositionValue::Fixed
        );

        if all_inset && is_out_of_flow {
            // Update style attribute string
            let mut styles: HashMap<String, String> = self.tree.nodes[node.0]
                .attributes
                .get("style")
                .map(|s| parse_style_string(s))
                .unwrap_or_default();
            for &(property, value) in properties {
                styles.insert(property.to_string(), value.to_string());
            }
            let style_str = styles
                .iter()
                .map(|(k, v)| format!("{}: {}", k, v))
                .collect::<Vec<_>>()
                .join("; ");
            self.tree.nodes[node.0]
                .attributes
                .insert("style".to_string(), style_str.clone());

            // Re-parse PDB for Stylo consistency
            {
                use style::properties::parse_style_attribute;
                use style::stylesheets::CssRuleType;
                use url::Url;
                let url = Url::parse("about:blank").unwrap();
                let extra_data = style::stylesheets::UrlExtraData::from(url);
                let pdb = parse_style_attribute(
                    &style_str,
                    &extra_data,
                    None,
                    QuirksMode::NoQuirks,
                    CssRuleType::Style,
                );
                self.tree.nodes[node.0].style_attribute_cache =
                    Some(ServoArc::new(self.tree.guard.wrap(pdb)));
            }

            // Update ComputedStyle + Taffy inset directly (skip style resolution)
            let vp = &crate::layout::Viewport::default();

            if let Some(taffy_id) = self.tree.nodes[node.0].taffy_id {
                if let Ok(mut ts) = self.tree.taffy.style(taffy_id).cloned() {
                    for &(property, value) in properties {
                        let new_val =
                            crate::computed_style::LengthPercentageAutoValue::parse(value, vp);
                        match property {
                            "left" => {
                                self.tree.nodes[node.0].computed_style.left = new_val;
                                ts.inset.left = new_val.to_taffy();
                            }
                            "top" => {
                                self.tree.nodes[node.0].computed_style.top = new_val;
                                ts.inset.top = new_val.to_taffy();
                            }
                            "right" => {
                                self.tree.nodes[node.0].computed_style.right = new_val;
                                ts.inset.right = new_val.to_taffy();
                            }
                            "bottom" => {
                                self.tree.nodes[node.0].computed_style.bottom = new_val;
                                ts.inset.bottom = new_val.to_taffy();
                            }
                            _ => unreachable!(),
                        }
                    }
                    let _ = self.tree.taffy.set_style(taffy_id, ts);
                    self.tree.layout_dirty = true;
                }
            }

            self.tree.nodes[node.0]
                .dirty
                .insert(DirtyFlags::LAYOUT | DirtyFlags::PAINT);
            self.tree.dirty_nodes.insert(node.0);
            self.tree.full_repaint_needed = true;
            return;
        }

        // ── Normal path ──────────────────────────────────────────────────
        let mut styles: HashMap<String, String> = self.tree.nodes[node.0]
            .attributes
            .get("style")
            .map(|s| parse_style_string(s))
            .unwrap_or_default();
        for &(property, value) in properties {
            styles.insert(property.to_string(), value.to_string());
        }
        let style_str = styles
            .iter()
            .map(|(k, v)| format!("{}: {}", k, v))
            .collect::<Vec<_>>()
            .join("; ");
        self.tree.nodes[node.0]
            .attributes
            .insert("style".to_string(), style_str.clone());

        use style::properties::parse_style_attribute;
        use style::stylesheets::CssRuleType;
        use url::Url;

        let url = Url::parse("about:blank").unwrap();
        let extra_data = style::stylesheets::UrlExtraData::from(url);
        let pdb = parse_style_attribute(
            &style_str,
            &extra_data,
            None,
            QuirksMode::NoQuirks,
            CssRuleType::Style,
        );
        self.tree.nodes[node.0].style_attribute_cache =
            Some(ServoArc::new(self.tree.guard.wrap(pdb)));

        *self.tree.nodes[node.0].stylo_element_data.borrow_mut() = None;
        self.tree.style_roots.push(node.0);
        self.tree.styles_dirty = true;
        if self.tree.nodes[node.0].ifc_root.is_some()
            || self.tree.nodes[node.0].is_inline()
            || self.tree.nodes[node.0].text_layout.is_some()
        {
            self.invalidate_ifc_for_node(node.0);
            if let Some(parent_id) = self.tree.nodes[node.0].parent {
                self.invalidate_parent_ifc(parent_id);
            }
        }
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
        self.tree.layout_dirty = true; // Structural change needs full layout
        self.tree.ifc_dirty = true; // Tree structure changed
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

    fn tag_name(&self, node: NodeId) -> Option<String> {
        let n = self.tree.nodes.get(node.0)?;
        match &n.kind {
            NodeKind::Element(data) => Some(data.tag.clone()),
            _ => None,
        }
    }

    fn node_type(&self, node: NodeId) -> Option<u16> {
        let n = self.tree.nodes.get(node.0)?;
        Some(match &n.kind {
            NodeKind::Element(_) => 1,
            NodeKind::Text(_) => 3,
            NodeKind::Comment(_) => 8,
            NodeKind::Document => 9,
        })
    }

    fn text_content(&self, node: NodeId) -> Option<String> {
        let n = self.tree.nodes.get(node.0)?;
        match &n.kind {
            NodeKind::Text(data) => Some(data.content.clone()),
            NodeKind::Element(_) => {
                let mut result = String::new();
                self.collect_text_content(node.0, &mut result);
                Some(result)
            }
            _ => None,
        }
    }
}
