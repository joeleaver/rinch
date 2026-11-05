//! Pseudo-element resolution (::before, ::after).

use servo_arc::Arc as ServoArc;

use style::properties::ComputedValues;

use crate::RinchDocument;
use crate::computed_style::ComputedStyle;
use crate::node::RawNodeId;

use super::resolve::style_is_flex_or_grid_container;

impl RinchDocument {
    /// Resolve a pseudo-element (::before or ::after) for a given parent node.
    ///
    /// Queries Stylo for the pseudo-element's computed styles, extracts the `content`
    /// property text, and creates synthetic DOM nodes (a wrapper span + text child)
    /// inserted as first child (::before) or last child (::after).
    ///
    /// `existing` is the box this pseudo-element generated on the element's
    /// previous cascade. It is **kept** while the pseudo still generates
    /// (#1023): its pseudo style is replaced in place and pushed for
    /// `apply_stylo_styles_to_taffy`, which compares it with the box's last
    /// `computed_style` as for any restyled element — so a running animation
    /// keeps its clock and a declared `transition` runs. As in Chrome, the
    /// animation restarts when its name changes, when the pseudo stops being
    /// rendered (`content: none`, `display: none`) and comes back, or when the
    /// originator leaves the document and is inserted again (#699). Its text child is replaced
    /// only when the text changed. When the pseudo no longer generates the
    /// box is removed.
    ///
    /// Answers whether the element's child list changed: a box generated or
    /// removed, or a kept box's text child replaced.
    pub(crate) fn resolve_pseudo_element(
        &mut self,
        parent_id: RawNodeId,
        parent_style: &ServoArc<ComputedValues>,
        pseudo: style::selector_parser::PseudoElement,
        existing: Option<RawNodeId>,
    ) -> bool {
        use selectors::matching::{
            MatchingContext, MatchingForInvalidation, MatchingMode, NeedsSelectorFlags,
            SelectorCaches, VisitedHandlingMode,
        };
        use style::applicable_declarations::ApplicableDeclarationList;
        use style::context::CascadeInputs;
        use style::properties::FirstLineReparenting;
        use style::rule_cache::RuleCacheConditions;
        use style::selector_parser::PseudoElement;
        use style::shared_lock::StylesheetGuards;
        use style::stylist::RuleInclusion;

        use crate::stylo_impl::RinchNode;

        let is_before = matches!(pseudo, PseudoElement::Before);

        // #542: the generated box's flex/grid-item-ness, from the same layout
        // parent `layout_parent` below blockifies against — computed ahead of
        // that (narrower-scoped) local so it survives to where the box is
        // created.
        let layout_parent_is_flex_or_grid = if parent_style.clone_display().is_contents() {
            let start = self.tree.nodes[parent_id].parent;
            self.nearest_non_contents_style(start)
                .map(|s| style_is_flex_or_grid_container(&s))
                .unwrap_or_else(|| style_is_flex_or_grid_container(parent_style))
        } else {
            style_is_flex_or_grid_container(parent_style)
        };

        // Query Stylo for pseudo-element declarations
        let pseudo_computed = {
            let rinch_node = RinchNode::new(parent_id, &self.tree);
            let guard = self.tree.guard.read();
            let guards = StylesheetGuards::same(&guard);

            let mut selector_caches = SelectorCaches::default();
            let mut matching_context =
                MatchingContext::<'_, style::selector_parser::SelectorImpl>::new_for_visited(
                    MatchingMode::ForStatelessPseudoElement,
                    None,
                    &mut selector_caches,
                    VisitedHandlingMode::AllLinksUnvisited,
                    self.stylist.quirks_mode(),
                    // Recorded, as for the element's own match: a structural
                    // selector that only a pseudo rule uses
                    // (`li:last-child::after`, Breadcrumbs'
                    // `:not(:last-child)::after`, `:empty::before`) must flag
                    // the parent too, or an insertion or removal never
                    // restyles the sibling it moves (`invalidation.rs`).
                    NeedsSelectorFlags::Yes,
                    MatchingForInvalidation::No,
                );
            matching_context.extra_data.originating_element_style = Some(parent_style);

            let mut applicable_declarations = ApplicableDeclarationList::new();

            // No style attribute: an inline `style` applies to the element,
            // never to its pseudo-elements. Stylo's rule collector pushes
            // whatever block it is handed at the style-attribute level whether
            // or not a pseudo is being matched, so passing the element's own
            // block here made the declaration list non-empty for every
            // inline-styled element — the early return below never fired, and
            // each restyle of such an element paid two full pseudo cascades
            // only for `ineffective_content_property` to throw them away. (The
            // generated box's own style did not change: measured, a
            // `style="width: 100px"` element's `::before` computes `width:
            // auto` either way.)
            self.stylist.push_applicable_declarations(
                rinch_node,
                Some(&pseudo),
                None,
                None,
                Default::default(),
                RuleInclusion::All,
                &mut applicable_declarations,
                &mut matching_context,
            );

            // If no declarations matched, no pseudo-element defined
            if applicable_declarations.is_empty() {
                None
            } else {
                let rule_node = self
                    .stylist
                    .rule_tree()
                    .compute_rule_node(&mut applicable_declarations, &guards);

                let mut rule_cache_conditions = RuleCacheConditions::default();

                // A `display: contents` element's generated boxes are children of
                // its layout parent's box, so that is what blockifies them (Stylo's
                // `layout_parent_style_for_pseudo`; #998).
                let layout_parent = if parent_style.clone_display().is_contents() {
                    let start = self.tree.nodes[parent_id].parent;
                    self.nearest_non_contents_style(start)
                        .unwrap_or_else(|| parent_style.clone())
                } else {
                    parent_style.clone()
                };

                Some(self.stylist.cascade_style_and_visited(
                    Some(RinchNode::new(parent_id, &self.tree)),
                    Some(&pseudo),
                    &CascadeInputs {
                        rules: Some(rule_node),
                        visited_rules: None,
                        flags: matching_context.extra_data.cascade_input_flags,
                        // Not `STARTING_STYLE`: `@starting-style` rules stay
                        // out, as `IncludeStartingStyle::No` kept them before.
                        included_cascade_flags: style::rule_tree::RuleCascadeFlags::empty(),
                    },
                    &guards,
                    Some(parent_style),
                    Some(&layout_parent),
                    FirstLineReparenting::No,
                    &Default::default(),
                    None,
                    &mut rule_cache_conditions,
                    &mut Default::default(), // tree_counting_caches
                ))
            }
        };
        let Some(pseudo_computed) = pseudo_computed else {
            return existing.is_some_and(|b| self.remove_generated_box(parent_id, b));
        };

        // A generated box sized in viewport units makes its element a
        // viewport-unit user too (`restyle_for_viewport_change`).
        if pseudo_computed
            .flags
            .intersects(style::computed_value_flags::ComputedValueFlags::USES_VIEWPORT_UNITS)
        {
            self.tree.nodes[parent_id].uses_viewport_units.set(true);
        }

        // `content: attr(x)` reads an attribute no selector need name; the
        // cascade above asked `TElement::get_attr` for it, which flagged the
        // element (`content_reads_attrs`) for `note_attribute_change`.

        // Check content property - if none/normal/empty, skip
        if pseudo_computed.ineffective_content_property() {
            return existing.is_some_and(|b| self.remove_generated_box(parent_id, b));
        }

        // Compute counter values for this element (needed for content: counter(...))
        let counter_values = self.compute_list_item_counters(parent_id);

        // Extract text content from the content property. `ineffective_content_property()`
        // above is the "no pseudo at all" check (`none` / `normal`); an empty
        // string here is NOT that — css-content-3 makes `content: ''` a valid,
        // present box with no text in it (#773), which is the only spelling a
        // purely decorative `::before`/`::after` (an arrow, a divider stroke, a
        // dot) ever wants. So this does not early-return on an empty string: it
        // only decides whether the box gets a text child below.
        let text = Self::extract_content_text_with_counters(&pseudo_computed, &counter_values);

        // Convert pseudo computed style to our ComputedStyle
        let pseudo_style = ComputedStyle::from_stylo(&pseudo_computed);
        self.tree.note_background_image(&pseudo_style);

        if let Some(span_raw) = existing {
            return self.update_generated_box(
                span_raw,
                pseudo_computed,
                &text,
                layout_parent_is_flex_or_grid,
            );
        }

        // Create a wrapper span element for the pseudo-element
        use rinch_core::dom::DomDocument;
        let span_id = self.create_element("span");

        // Append a text child only when there is text to show it — an empty
        // `content: ''` box is a shape (sized and painted from its own style),
        // not an empty text node for the IFC to lay out.
        if !text.is_empty() {
            let text_node_id = self.create_text(&text);
            // Append the text node to the span (using raw tree manipulation
            // to avoid triggering style recomputation via DomDocument::append_child)
            let span_raw = span_id.0;
            let text_raw = text_node_id.0;
            self.tree.nodes[text_raw].parent = Some(span_raw);
            self.tree.nodes[span_raw].children.push(text_raw);
            // Sync taffy
            if let (Some(parent_taffy), Some(child_taffy)) = (
                self.tree.nodes[span_raw].taffy_id,
                self.tree.nodes[text_raw].taffy_id,
            ) {
                let _ = self.tree.taffy.add_child(parent_taffy, child_taffy);
            }
        }

        // Set computed style and mark as pseudo-element. The pseudo cascade's
        // result is also the box's *Stylo* style: `apply_stylo_styles_to_taffy`
        // reads a node's `computed_style` back from there, and
        // `resolve_styles_recursive` recognises a box that carries a pseudo
        // style (`ComputedValues::is_pseudo_style`) and does not re-cascade it
        // as the plain `<span>` it is in the tree (#1004). Each cascade of its
        // originator gives it a fresh pseudo cascade, in place (#1023).
        {
            let span_raw = span_id.0;
            // #542 (perf review): read before `pseudo_style` moves into the
            // node below — the cached filter check, same reasoning as the
            // main cascade in `style_resolution/mod.rs`.
            let filter_creates_stacking_context = pseudo_style.has_non_identity_filter();
            self.tree.nodes[span_raw].computed_style = pseudo_style;
            self.tree.nodes[span_raw].is_pseudo_element = true;
            self.tree.nodes[span_raw].is_generated_after = !is_before;
            // #542: the same layout parent that blockified this generated box
            // decides whether it is a flex/grid item for
            // `Node::creates_stacking_context`'s flex/grid z-index creator.
            self.tree.nodes[span_raw]
                .is_flex_or_grid_item
                .set(layout_parent_is_flex_or_grid);
            self.tree.nodes[span_raw]
                .filter_creates_stacking_context
                .set(filter_creates_stacking_context);
            let mut data = style::data::ElementData::default();
            data.styles.primary = Some(pseudo_computed);
            self.tree.nodes[span_raw].stylo_element_data.set(data);
            self.tree.style_dirty_nodes.push(span_raw);
        }

        // Insert as first child (::before) or last child (::after) of parent
        {
            let span_raw = span_id.0;
            self.tree.nodes[span_raw].parent = Some(parent_id);
            if is_before {
                // Insert at beginning of parent's children
                self.tree.nodes[parent_id].children.insert(0, span_raw);
                // Sync taffy: insert at index 0
                if let (Some(parent_taffy), Some(child_taffy)) = (
                    self.tree.nodes[parent_id].taffy_id,
                    self.tree.nodes[span_raw].taffy_id,
                ) {
                    let _ = self
                        .tree
                        .taffy
                        .insert_child_at_index(parent_taffy, 0, child_taffy);
                }
            } else {
                // Append at end of parent's children
                self.tree.nodes[parent_id].children.push(span_raw);
                if let (Some(parent_taffy), Some(child_taffy)) = (
                    self.tree.nodes[parent_id].taffy_id,
                    self.tree.nodes[span_raw].taffy_id,
                ) {
                    let _ = self.tree.taffy.add_child(parent_taffy, child_taffy);
                }
            }
        }
        true
    }

    /// Give a kept generated box (`existing` in [`Self::resolve_pseudo_element`])
    /// its new pseudo cascade and content, in place (#1023). Its
    /// `computed_style` is left for `apply_stylo_styles_to_taffy`, which reads
    /// the old one as the before-change style. Answers whether its text
    /// child was replaced.
    fn update_generated_box(
        &mut self,
        span_raw: RawNodeId,
        pseudo_computed: ServoArc<ComputedValues>,
        text: &str,
        layout_parent_is_flex_or_grid: bool,
    ) -> bool {
        use rinch_core::dom::DomDocument;

        self.tree.nodes[span_raw]
            .is_flex_or_grid_item
            .set(layout_parent_is_flex_or_grid);
        if let Some(data) = self.tree.nodes[span_raw]
            .stylo_element_data
            .borrow_mut()
            .as_mut()
        {
            data.styles.primary = Some(pseudo_computed);
        }
        self.tree.style_dirty_nodes.push(span_raw);
        self.mark_restyled_for_paint(span_raw);

        // The text child, when there is one, is the box's only child.
        let old_text = self.tree.nodes[span_raw].children.first().copied();
        let same = match old_text {
            Some(t) => self.tree.nodes[t].text_content() == Some(text),
            None => text.is_empty(),
        };
        if same {
            return false;
        }
        if let Some(t) = old_text {
            if let Some(span_taffy) = self.tree.nodes[span_raw].taffy_id {
                self.taffy_detach_contribution(span_taffy, t);
            }
            self.mark_subtree_paint_dirty(t);
            self.tree.remove_subtree(t);
            self.tree.nodes[span_raw].children.retain(|&c| c != t);
        }
        if !text.is_empty() {
            let text_raw = self.create_text(text).0;
            self.tree.nodes[text_raw].parent = Some(span_raw);
            self.tree.nodes[span_raw].children.push(text_raw);
            if let (Some(parent_taffy), Some(child_taffy)) = (
                self.tree.nodes[span_raw].taffy_id,
                self.tree.nodes[text_raw].taffy_id,
            ) {
                let _ = self.tree.taffy.add_child(parent_taffy, child_taffy);
            }
        }
        // The box's own child list changed: an `inline-block` box is a
        // formatting container of its own, which the originator's seed does
        // not reach, and its inline layout named the freed text node.
        self.tree
            .seed_ifc(span_raw, crate::ifc_scope::IfcSeed::Children);
        self.invalidate_ifc_root(span_raw);
        true
    }

    /// Take the generated box (or list marker) `cid` out of `parent_id` and
    /// free it. Answers `true` (the child list changed).
    pub(crate) fn remove_generated_box(&mut self, parent_id: RawNodeId, cid: RawNodeId) -> bool {
        // The IFC the generated box was laid out in names it; the caller's
        // structural seed re-sets the region up.
        if let Some(root) = self.tree.nodes[cid].ifc_root {
            self.invalidate_ifc_root(root);
        }
        // Detach the pseudo-element's whole Taffy contribution, not just its
        // own id (#521): a `display: contents` pseudo has already had its own
        // id spliced out of `parent_id`'s Taffy child list by
        // `sync_display_contents`, with its *effective* children (any that
        // are themselves genuine Taffy members) spliced in directly — the
        // same shape `taffy_detach_contribution` (#515/#517) exists to detach
        // everywhere else.
        if let Some(parent_taffy) = self.tree.nodes[parent_id].taffy_id {
            self.taffy_detach_contribution(parent_taffy, cid);
        }
        // Its pixels are recorded where it was painted before it is freed,
        // as `remove_node` does (#966): a generated box that overflows its
        // originator, or casts a shadow past it, left them on screen when its
        // rule stopped matching.
        self.mark_subtree_paint_dirty(cid);
        self.tree.remove_subtree(cid);
        self.tree.nodes[parent_id].children.retain(|&c| c != cid);
        true
    }

    /// Extract text content from a Stylo ComputedValues `content` property.
    /// Handles string content items and counter() references.
    /// For counter() values, uses the provided counter_values map.
    pub(crate) fn extract_content_text_with_counters(
        computed: &ComputedValues,
        counter_values: &std::collections::HashMap<String, i32>,
    ) -> String {
        use style::values::generics::counters::{Content, ContentItem};

        let content = &computed.get_counters().content;
        match content {
            Content::Normal | Content::None => String::new(),
            Content::Items(items) => {
                let mut result = String::new();
                for item in items.items.iter() {
                    match item {
                        ContentItem::String(s) => {
                            result.push_str(s);
                        }
                        ContentItem::Counter(name, _style) => {
                            let name_str = name.0.to_string();
                            if let Some(&value) = counter_values.get(&name_str) {
                                result.push_str(&value.to_string());
                            } else {
                                result.push('0');
                            }
                        }
                        ContentItem::Counters(name, separator, _style) => {
                            let name_str = name.0.to_string();
                            if let Some(&value) = counter_values.get(&name_str) {
                                result.push_str(&value.to_string());
                            } else {
                                result.push('0');
                            }
                            let _ = separator; // TODO: nested counter separator support
                        }
                        // `attr(name)` is no content item since stylo 0.21: it
                        // is substituted in the cascade (`TElement::get_attr`)
                        // and arrives here as the `String` above.
                        _ => {
                            // Skip other content items (image, quotes, etc.)
                        }
                    }
                }
                result
            }
        }
    }

    /// Compute the CSS counter value for `list-item` on an `<li>` element.
    ///
    /// Walks siblings to determine the ordinal position of this `<li>`
    /// within its parent `<ol>` or `<ul>`. Returns a map with the
    /// `list-item` counter set to the appropriate value.
    pub(crate) fn compute_list_item_counters(
        &self,
        node_id: RawNodeId,
    ) -> std::collections::HashMap<String, i32> {
        let mut counters = std::collections::HashMap::new();

        let node = match self.tree.nodes.get(node_id) {
            Some(n) => n,
            None => return counters,
        };

        // Only compute for <li> elements
        if node.tag() != Some("li") {
            return counters;
        }

        let parent_id = match node.parent {
            Some(p) => p,
            None => return counters,
        };

        let parent = match self.tree.nodes.get_live(parent_id) {
            Some(n) => n,
            None => return counters,
        };

        // Check start attribute on parent <ol>
        let start = if parent.tag() == Some("ol") {
            parent
                .attributes
                .get("start")
                .and_then(|s| rinch_core::dom::parse_html_integer(s))
                .unwrap_or(1)
        } else {
            1
        };

        // Count this <li>'s position among <li> siblings
        let mut count = start;
        for &sibling_id in &parent.children {
            if sibling_id == node_id {
                break;
            }
            if let Some(sibling) = self.tree.nodes.get_live(sibling_id) {
                if sibling.tag() == Some("li") {
                    // Check for value attribute override
                    if let Some(val) = sibling.attributes.get("value") {
                        if let Some(v) = rinch_core::dom::parse_html_integer(val) {
                            count = v.saturating_add(1);
                            continue;
                        }
                    }
                    count = count.saturating_add(1);
                }
            }
        }

        // Check if this <li> has a value attribute
        if let Some(val) = node.attributes.get("value") {
            if let Some(v) = rinch_core::dom::parse_html_integer(val) {
                count = v;
            }
        }

        counters.insert("list-item".to_string(), count);
        counters
    }

    /// Generate a list marker pseudo-element for `<li>` elements.
    ///
    /// If the `<li>` doesn't already have a `::before` pseudo-element from CSS,
    /// this creates a marker span whose text depends on the item's computed
    /// `list-style-type` (#725):
    /// - `disc` → "•", `circle` → "◦", `square` → "▪"
    /// - `decimal` (or any `<ol>`/`<li>` combination not in that set, kept for
    ///   back-compat) → "1.", "2.", … via [`Self::compute_list_item_counters`]
    /// - anything else not covered above falls back to the old tag-based
    ///   heuristic (a bullet under `<ul>`, a number under `<ol>`) rather than
    ///   drawing nothing, so an unsupported keyword (`lower-alpha`,
    ///   `upper-roman`, …) still shows *a* marker — tracked for a real
    ///   rendering as a follow-up (alphabetic/roman types aren't even in
    ///   Stylo's servo-build `list-style-type` keyword set for roman numerals).
    /// - `none` → no marker, as before.
    ///
    /// Each gets a trailing en-space (`\u{2002}`) — wider than a normal space
    /// and not subject to whitespace collapsing in the IFC.
    ///
    /// This is still the #725 "inside"-shaped marker: the span is an ordinary
    /// first inline child of the `<li>`, so it participates in the item's own
    /// line rather than being painted in the parent `<ul>`/`<ol>`'s padding
    /// as a real `outside` marker box would be. For a single-line item that
    /// is visually indistinguishable from `outside`; it diverges on a
    /// wrapped multi-line item, where a real `outside` marker leaves every
    /// continuation line flush with the first line's text and this one
    /// leaves it indented by the marker's own width instead. Filed as a
    /// follow-up (paint-side marker box, `list-style-position: inside`
    /// support) rather than landed here.
    ///
    /// Answers whether a marker was generated.
    pub(crate) fn resolve_list_marker(&mut self, node_id: RawNodeId) -> bool {
        let node = match self.tree.nodes.get(node_id) {
            Some(n) => n,
            None => return false,
        };

        // Only process <li> elements.
        if node.tag() != Some("li") {
            return false;
        }

        // Check Stylo's own freshly-cascaded data for `display: list-item`
        // and `list-style-type` together, in one borrow. `node.computed_style`
        // (rinch's own `ComputedStyle`) is NOT yet synced with this cascade —
        // that happens later, in `apply_stylo_styles_to_taffy` — so reading
        // it here would see the *previous* pass's value (`Flex`, rinch's
        // `DisplayValue::default()`, on this node's very first cascade) and
        // suppress every marker's first paint. An author who overrides
        // `display` off `list-item` gets no marker, matching Chrome; one
        // who sets `list-style-type: none` (inherited, so checking the
        // `<li>` covers both an inline override and inheriting it from the
        // parent `<ul>`/`<ol>`) gets none either.
        use style::properties::longhands::list_style_type::computed_value::T as ListStyleType;
        #[derive(Clone, Copy, PartialEq)]
        enum Marker {
            Disc,
            Circle,
            Square,
            Decimal,
            Other,
        }
        let keyword = {
            let stylo_data = node.stylo_element_data.borrow();
            match &stylo_data {
                Some(data) => match &data.styles.primary {
                    Some(primary) => {
                        if !primary.get_box().display.is_list_item() {
                            return false;
                        }
                        let kw = &primary.get_list().list_style_type;
                        if *kw == ListStyleType::none() {
                            return false;
                        }
                        // A `<counter-style>` since stylo 0.21 (it was a
                        // keyword enum): the four names this picks a glyph
                        // for, anything else `Other`.
                        let is = |name: &str| kw.is_name(&style::Atom::from(name));
                        if is("disc") {
                            Marker::Disc
                        } else if is("circle") {
                            Marker::Circle
                        } else if is("square") {
                            Marker::Square
                        } else if is("decimal") {
                            Marker::Decimal
                        } else {
                            Marker::Other
                        }
                    }
                    None => return false,
                },
                None => return false,
            }
        };

        // Skip if already has a pseudo-element child (from CSS ::before)
        let has_pseudo = node.children.iter().any(|&cid| {
            self.tree
                .nodes
                .get(cid)
                .is_some_and(|n| n.is_pseudo_element)
        });
        if has_pseudo {
            return false;
        }

        let parent_id = match node.parent {
            Some(p) => p,
            None => return false,
        };

        let parent_tag = self
            .tree
            .nodes
            .get(parent_id)
            .and_then(|n| n.tag())
            .unwrap_or("");

        if parent_tag != "ol" && parent_tag != "ul" {
            return false; // Not inside a list
        }

        // Pick the marker glyph from the keyword, not just the parent tag
        // (#725's core complaint: `list-style-type` was parsed and ignored).
        // `Decimal`, and any keyword this match doesn't know yet, fall back
        // to the pre-#725 tag-based heuristic.
        let marker_text = match keyword {
            Marker::Disc => "\u{2022}\u{2002}".to_string(),   // •
            Marker::Circle => "\u{25E6}\u{2002}".to_string(), // ◦
            Marker::Square => "\u{25AA}\u{2002}".to_string(), // ▪
            _ if parent_tag == "ol" || keyword == Marker::Decimal => {
                let counters = self.compute_list_item_counters(node_id);
                let num = counters.get("list-item").copied().unwrap_or(1);
                format!("{}.\u{2002}", num)
            }
            _ => "\u{2022}\u{2002}".to_string(),
        };

        // Create the marker pseudo-element (span + text node)
        use rinch_core::dom::DomDocument;
        let span_id = self.create_element("span");
        let text_node_id = self.create_text(&marker_text);

        // Append text node to span
        {
            let span_raw = span_id.0;
            let text_raw = text_node_id.0;
            self.tree.nodes[text_raw].parent = Some(span_raw);
            self.tree.nodes[span_raw].children.push(text_raw);
            if let (Some(parent_taffy), Some(child_taffy)) = (
                self.tree.nodes[span_raw].taffy_id,
                self.tree.nodes[text_raw].taffy_id,
            ) {
                let _ = self.tree.taffy.add_child(parent_taffy, child_taffy);
            }
        }

        // Mark as pseudo-element, inherit parent's computed style
        {
            let span_raw = span_id.0;
            self.tree.nodes[span_raw].computed_style =
                self.tree.nodes[node_id].computed_style.clone();
            self.tree.nodes[span_raw].is_pseudo_element = true;
            self.tree.style_dirty_nodes.push(span_raw);
        }

        // Insert as first child of the <li>
        {
            let span_raw = span_id.0;
            self.tree.nodes[span_raw].parent = Some(node_id);
            self.tree.nodes[node_id].children.insert(0, span_raw);
            if let (Some(parent_taffy), Some(child_taffy)) = (
                self.tree.nodes[node_id].taffy_id,
                self.tree.nodes[span_raw].taffy_id,
            ) {
                let _ = self
                    .tree
                    .taffy
                    .insert_child_at_index(parent_taffy, 0, child_taffy);
            }
        }
        true
    }
}
