//! Node construction for [`RinchDocument`]: the bodies of
//! [`DomTree`](rinch_core::dom::DomTree)'s `create_element`, `create_text` and `create_comment`.

use rinch_core::dom::NodeId;

use peniko::color::{AlphaColor, Srgb};

use crate::node::{DisplayMode, Node, NodeContext, TextMeasure};

use super::RinchDocument;

impl RinchDocument {
    /// [`DomTree::create_element`](rinch_core::dom::DomTree::create_element): a new, detached element with its Taffy
    /// leaf and the per-tag bookkeeping (style sheets, option and textarea
    /// counts, SVG titles).
    pub(super) fn build_element(&mut self, tag: &str) -> NodeId {
        // #739: fold the tag to its canonical spelling before anything else
        // reads it — `default_display_for_tag`, the `tag == "img"` /
        // `matches!(tag, "style" | …)` checks below, and every later
        // `fold_attribute_name` call all key on the tag the node ends up
        // carrying, not the one the author wrote.
        let tag = crate::attr_name::fold_tag_name(tag);
        let tag = tag.as_ref();
        let id = self.tree.nodes.vacant_key();
        if tag == "title" {
            // An SVG element's tooltip text (#1646).
            self.tree.title_sources += 1;
        }
        if tag == "option" {
            self.tree.option_elements += 1;
        }
        if tag == "style" {
            // Its sheet follows it in and out of the document (#884).
            self.style_elements.push(id);
        }
        if tag == "textarea" {
            self.tree.textarea_elements += 1;
        }
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
        let replaced = crate::replaced::replaced_context(&node);
        let taffy_id = if let Some(context) = replaced {
            // `<canvas>`, `<video>`, `<iframe>`: sized from their natural or
            // default object size (#1173). The attributes are not set yet,
            // so a canvas starts at its 300x150 default; a later
            // `width`/`height` write re-syncs it through the cascade.
            self.tree
                .taffy
                .new_leaf_with_context(default_style, context)
                .unwrap()
        } else if tag == "img" {
            // Image elements use NodeContext::Image for intrinsic sizing
            let context = NodeContext::Image {
                src: String::new(),
                natural: None,
                aspect: Default::default(),
                alt: None,
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

    /// [`DomTree::create_text`](rinch_core::dom::DomTree::create_text): a new, detached text node with its measured
    /// Taffy leaf, styled from its parent before layout.
    pub(super) fn build_text(&mut self, text: &str) -> NodeId {
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
            letter_spacing: 0.0,                                 // default, updated from parent
            word_spacing: 0.0,                                   // default, updated from parent
            white_space: crate::computed_style::WhiteSpaceValue::Normal,
            overflow_wrap: crate::computed_style::OverflowWrapValue::default(),
            word_break: crate::computed_style::WordBreakValue::default(),
            line_break: crate::computed_style::LineBreakValue::default(),
            hyphens_none: false,
            base_direction: parley::BaseDirection::Ltr,
            text_indent: crate::computed_style::TextIndentValue::default(),
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

    /// [`DomTree::create_comment`](rinch_core::dom::DomTree::create_comment): a new, detached comment node, which gets
    /// no Taffy node.
    pub(super) fn build_comment(&mut self, text: &str) -> NodeId {
        let id = self.tree.nodes.vacant_key();
        let node = Node::comment(id, text, self.tree.guard.clone());
        // Comments do NOT get Taffy nodes
        self.tree.nodes.insert(node);
        NodeId(id)
    }
}
