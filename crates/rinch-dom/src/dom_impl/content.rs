//! [`DomContent`] for [`RinchDocument`]: text content, attributes, inline
//! styles, style isolation and highlight overlays.

use rinch_core::dom::{DomContent, NodeId};

use peniko::color::{AlphaColor, Srgb};

use crate::ifc_scope::IfcSeed;
use crate::node::RawNodeId;
use crate::node::{DirtyFlags, Node, NodeContext, NodeKind, TextMeasure};

use super::{RinchDocument, parse_inline_style};

impl DomContent for RinchDocument {
    fn set_text_content(&mut self, node: NodeId, text: &str) {
        // #723: an id that names no live node is a no-op, as on the web.
        if !self.tree.nodes.contains(node.0) {
            return;
        }
        let n = node.0;

        // Fast path: skip if content is already identical.
        match &self.tree.nodes[n].kind {
            NodeKind::Text(t) if t.content == text => return,
            NodeKind::Element(_) => {
                // The element's own children: its generated boxes are not
                // replaced by a text write (#1630).
                let (lo, hi) = self.own_children_range(n);
                if hi - lo == 1
                    && let NodeKind::Text(t) =
                        &self.tree.nodes[self.tree.nodes[n].children[lo]].kind
                    && t.content == text
                {
                    return;
                }
            }
            _ => {}
        }
        // An element whose only child is a text node keeps that node: the new
        // text is written into it, by the text-node path below — the one every
        // reactive `{|| text}` takes, so it re-shapes and repaints as that does.
        // Orphaning it and minting another changed the element's child list on
        // every text rewrite (a removal observer above was told a child left
        // each time, #1440) and left one orphaned slab entry and Taffy leaf
        // behind per write.
        //
        // "Only" among its own children: a generated `::before` / `::after`
        // box beside the text does not count (#1630).
        if self.tree.nodes[n].is_element() {
            let (lo, hi) = self.own_children_range(n);
            if hi - lo == 1 {
                let only = self.tree.nodes[n].children[lo];
                if matches!(self.tree.nodes[only].kind, NodeKind::Text(_)) {
                    return self.set_text_content(NodeId(only), text);
                }
            }
        }
        // Past the no-op return: an identical write moves nothing, and must
        // not leave the next pointer move with a cold hit cache.
        self.tree.hit_cache.invalidate();
        // New text can change the first strong letter a `dir=auto` element
        // above takes its direction from (#2272).
        if self.tree.nodes[n].is_element() {
            self.note_dir_auto_content(n);
        } else if let Some(parent) = self.tree.nodes[n].parent {
            self.note_dir_auto_content(parent);
        }

        // Invalidate IFC if this node belongs to one
        self.invalidate_ifc_for_node(n);
        // Also invalidate parent's IFC
        if let Some(parent_id) = self.tree.nodes[n].parent {
            self.invalidate_parent_ifc(parent_id);
            // Text gained or lost beside rows decides whether the block
            // stands for a table (`note_table_host`).
            if matches!(self.tree.nodes[n].kind, NodeKind::Text(_)) {
                self.note_table_children_changed(parent_id);
            }
        }
        // New text is new content to shrink-wrap: every atomic inline above
        // this node has to be measured again, and the root compute cannot do it
        // — they are detached from its child list (issue #661).
        self.mark_atomic_inline_dirty(n);
        // Track which IFC root has dirty text content so the measure callback
        // can skip expensive Parley rebuilds for unchanged roots.
        {
            let ifc_root = self.tree.nodes[n]
                .ifc_root
                .or_else(|| self.tree.nodes[n].parent);
            if let Some(root_id) = ifc_root {
                self.tree.dirty_ifc_text_roots.insert(root_id);
                // Invalidate cached measure results for this root
                self.tree.forget_ifc_measures(root_id);
            }
        }
        // Text can flip its parent's `:empty` (an empty text node does not
        // stop an element being empty); an element's children are replaced
        // outright below.
        if let Some(parent_id) = self.tree.nodes[n].parent
            && let NodeKind::Text(t) = &self.tree.nodes[n].kind
            && t.content.is_empty() != text.is_empty()
        {
            self.note_text_emptiness_changed(parent_id, n);
        }
        // Text that becomes, or stops being, nothing but white space changes
        // whether a grid container's run of it is rendered at all (#2443): the
        // run's anonymous box is minted hidden by the structural pass, which
        // an edit of the text does not otherwise re-run.
        if let Some(parent_id) = self.tree.nodes[n].parent
            && matches!(self.tree.nodes[n].kind, NodeKind::Text(_))
            && crate::ifc::is_white_space_text(&self.tree.nodes[n])
                != crate::ifc::is_white_space_only(text)
        {
            self.tree.seed_ifc(parent_id, IfcSeed::Children);
        }
        let replaces_children = self.tree.nodes[n].is_element();
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
                // For elements: remove the element's own children and add a
                // text child in their place. Its generated `::before` /
                // `::after` boxes stay (#1630): they are not DOM children, and
                // nothing would make them again until the element is next
                // cascaded.
                let (lo, hi) = self.own_children_range(n);
                let old_children: Vec<_> = self.tree.nodes[n].children[lo..hi].to_vec();
                for child in old_children {
                    // Its pixels are recorded and its painted state forgotten
                    // while it is still under the parent it was painted under,
                    // as `remove_node` does (#966).
                    self.mark_subtree_paint_dirty(child);
                    self.tree.nodes[child].parent = None;
                    // The fourth detach route, and the one that does not look
                    // like one (#699). These children are orphaned but **not
                    // freed** — unlike `set_inner_html`, which reaches
                    // `NodeTree::remove_subtree` and drops the slab entry along
                    // with both animation maps — so a handle app code still
                    // holds stays alive, styled, and transitioning.
                    self.detach_subtree_styles(child);
                    // And it leaves its inline-formatting state behind, as
                    // `remove_child` and the moves do (#1073). The orphans'
                    // `ifc_root` marks are what every IFC consumer reads: left
                    // in place, the orphaned roots stayed roots, the next
                    // layout shaped them (#1069) and sized their atomic
                    // inlines (#1040), for a subtree nothing paints. Attaching
                    // the subtree again seeds a structural pass that marks it.
                    // The two dirty sets are not scrubbed, for the same reason
                    // `remove_child` does not: both are drained by the next
                    // layout, and every consumer that measures or shapes asks
                    // for a mark first — an atomic inline its own `ifc_root`,
                    // a text root a marked child.
                    self.clear_ifc_root_recursive(child);
                    // Remove each child's contribution from taffy — for a
                    // spliced `display: contents` child that is its
                    // children's slots, not its own id (#517)
                    if let Some(parent_taffy) = self.tree.nodes[n].taffy_id {
                        self.taffy_detach_contribution(parent_taffy, child);
                    }
                }
                self.tree.nodes[n].children.drain(lo..hi);
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
                    white_space: crate::computed_style::WhiteSpaceValue::Normal,
                    letter_spacing: 0.0,
                    word_spacing: 0.0,
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
                text_node.taffy_id = Some(taffy_id);
                self.tree.taffy_map.insert(taffy_id, text_id);
                self.tree.nodes.insert(text_node);
                // Between the generated boxes, at `lo`.
                let len = self.tree.nodes[n].children.len();
                self.tree.nodes[n].children.insert(lo, text_id);
                // Add to taffy parent
                if let Some(parent_taffy) = self.tree.nodes[n].taffy_id {
                    if lo == len {
                        self.taffy_add_child_checked(parent_taffy, taffy_id);
                    } else {
                        let taffy_idx = self.compute_taffy_child_index(n, lo);
                        self.attach_taffy_child_at(parent_taffy, taffy_idx, taffy_id, n, text_id);
                    }
                }
                // Structural change (children replaced).
                self.tree.seed_ifc(n, IfcSeed::Children);
            }
        }
        // After the replacement, not before: whether `:empty` can have
        // flipped is judged on the children the element now has. Before it,
        // two old non-empty children read as "cannot flip" and
        // `set_text_content(el, "")` left `el:empty` stale.
        if replaces_children {
            self.note_child_list_changed(n, 0);
        }
        self.tree.layout_dirty = true; // Text content change affects layout
        self.push_dirty(n);
        self.note_select_content_changed(n);

        // If this node is a <style> element, reload its CSS
        self.maybe_load_style_css(n);
        // If the parent is a <style> element (text node content changed), reload
        if let Some(parent_id) = self.tree.nodes[n].parent {
            self.maybe_load_style_css(parent_id);
        }
    }

    fn set_attribute(&mut self, node: NodeId, name: &str, value: &str) {
        // #723: an id that names no live node is a no-op, as on the web.
        if !self.tree.nodes.contains(node.0) {
            return;
        }
        // #688: in HTML content the name is ASCII case-insensitive, so it is
        // stored folded — every reader below and in `stylo_impl` looks an
        // attribute up by a lowercase literal (`"style"`, `"class"`, `"id"`,
        // `"src"`), and Stylo hands `attr_matches` an already-lowercased
        // selector name. In SVG content it is case-sensitive and kept verbatim.
        let folded = crate::attr_name::fold_attribute_name(self.tree.nodes[node.0].tag(), name);
        let name: &str = &folded;

        // Writing the value an attribute already has changes nothing, so it
        // restyles nothing: no selector can match differently and no inline
        // style differs. Without this, every identical write re-resolved the
        // node's whole subtree, and a reactive attribute that re-runs with the
        // same answer is the common case (a toolbar's fifteen button styles,
        // re-written on every keystroke, restyled about a hundred nodes).
        // `set_text_content` has always skipped identical text the same way.
        // `src` keeps its old behaviour: an identical write re-applies a cached
        // image's intrinsic size and requests a load the cache has not started
        // (no loader was installed at the first write). It does not retry a
        // failed load — a `Failed` entry stays cached.
        if name != "src"
            && self.tree.nodes[node.0]
                .attributes
                .get(name)
                .map(String::as_str)
                == Some(value)
        {
            return;
        }
        // Past the no-op return, like `set_text_content`'s.
        self.tree.hit_cache.invalidate();

        // An `<option>`'s selectedness moves on the *transition* into the
        // `selected` attribute, never on a re-write of one already there — the
        // browser's own gate, and what makes the selected option the last one
        // **set** rather than the last one carrying the attribute (#692).
        let selects_this_option = name == "selected"
            && self.tree.nodes[node.0].tag() == Some("option")
            && !self.tree.nodes[node.0].attributes.contains_key(name);

        if name == "data-onmousemove" && !self.tree.nodes[node.0].attributes.contains_key(name) {
            self.tree.mousemove_handlers += 1;
        }
        if name == "title" && !self.tree.nodes[node.0].attributes.contains_key(name) {
            self.tree.title_sources += 1;
        }
        // A `value` write that changes an unfocused text control's value puts
        // its caret at the end, as a browser's value setter does (issue
        // #1678; rinch-web writes through that setter). The focused field's
        // own selection is the runtime's, synced with each edit.
        let value_before = (name == "value"
            && crate::form_control::has_text_selection(&self.tree.nodes[node.0])
            && self.tree.nodes[node.0]
                .attributes
                .get("data-focused")
                .map(String::as_str)
                != Some("true"))
        .then(|| {
            crate::form_control::control_value(&self.tree.nodes, node.0)
                .unwrap_or_default()
                .into_owned()
        });
        // Before the write: the invalidation snapshot holds the old value.
        self.note_attribute_change(node.0, name);
        self.tree.nodes[node.0].write_attribute(name, value);
        if let Some(before) = value_before {
            let after =
                crate::form_control::control_value(&self.tree.nodes, node.0).unwrap_or_default();
            if after != before {
                // It overrides a selection set before it, posted or not.
                rinch_core::cancel_pending_text_selection_request(self.doc_key, node.0);
                let len = after.encode_utf16().count();
                self.tree.text_selections.insert(
                    node.0,
                    (len, len, rinch_core::dom::SelectionDirection::Forward),
                );
            }
        }
        if selects_this_option {
            crate::select::set_option_selectedness(&mut self.tree, node.0, true);
        }
        // An option's `label` attribute is its label, which the closed
        // select is sized from (#1128).
        if name == "label" && self.tree.nodes[node.0].tag() == Some("option") {
            self.note_select_content_changed(node.0);
        }
        // A `<select>`'s own `value` write is a live selection write too
        // (#757): it is the freshest of the two mechanisms until a later
        // `selected` write on one of its options clears it — see
        // `NodeTree::select_value_fresh`'s doc.
        if name == "value" && self.tree.nodes[node.0].tag() == Some("select") {
            self.tree.select_value_fresh.insert(node.0);
        }

        // Parse inline style into Stylo PropertyDeclarationBlock
        if name == "style" {
            if value.trim().is_empty() {
                // Empty/blank style: clear the cache entirely so Stylo sees None
                // (not Some(empty_pdb)) and falls back to class-based styles.
                self.tree.nodes[node.0].style_attribute_cache = None;
            } else {
                self.cache_inline_style(node.0, parse_inline_style(value));
            }
        }
        self.sync_presentational_hints(node.0, name);
        // No IFC is invalidated here for a `style`/`class` write: the restyle
        // below re-cascades this node and its subtree, and the cascade drops
        // the text layout of exactly the nodes whose text inputs changed
        // (`ComputedStyle::same_text_layout_inputs`). See
        // `invalidate_descendant_styles`.
        // Handle <img src="..."> — trigger async image load
        if name == "src" && self.tree.nodes[node.0].tag() == Some("img") {
            self.request_image_load_for_node(node.0, value);
        }
        // What an `<img>` with no picture shows rests on its `src` and its
        // `alt` (#2309); neither need restyle it.
        if matches!(name, "src" | "alt") {
            self.sync_img_placeholder(node.0);
        }

        // Which elements the change restyles is decided at the next
        // `resolve_styles`, by Stylo's invalidator against the snapshot
        // `note_attribute_change` took above: this element, its descendants or
        // its later siblings — exactly the ones some selector's answer
        // changed for (`style_resolution::invalidation`). It used to be this
        // element and its whole subtree, for every attribute, whether or not
        // any selector named it, and never a sibling.
        //
        // The node is still marked dirty for layout and paint whatever the
        // selectors say: an attribute can reach paint without reaching style
        // (an `<input>`'s `value` is painted from the attribute).
        self.push_dirty_flags(
            node.0,
            DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT,
        );
    }

    fn remove_attribute(&mut self, node: NodeId, name: &str) {
        // #723: an id that names no live node is a no-op, as on the web.
        if !self.tree.nodes.contains(node.0) {
            return;
        }
        // Folded exactly as the write was (#688) — `NodeHandle::write_attribute`
        // turns a falsey boolean attribute into a `remove_attribute` with the
        // caller's own spelling, so a fold on one side and not the other could
        // never turn one off again.
        let folded = crate::attr_name::fold_attribute_name(self.tree.nodes[node.0].tag(), name);
        let name: &str = &folded;

        // Removing an attribute the node does not carry changes nothing, so it
        // costs nothing: everything below invalidates this node's style and its
        // whole subtree's, and paying that for a write that erases nothing is
        // exactly what `NodeHandle::write_attribute`'s removal guard used to
        // avoid by not calling in at all. Since #687 that guard lets `checked` /
        // `selected` through unconditionally — on the web their live state sits
        // in an IDL property the content attribute stopped describing the moment
        // the user toggled the control, so only the backend can see it — and the
        // saving moves here, where it covers every other caller too. A browser
        // agrees: `removeAttribute` for an absent attribute mutates nothing and
        // schedules no invalidation.
        //
        // Nothing below has an effect on a node without the attribute. The
        // `style` arm is the one worth stating: `style_attribute_cache` is
        // written only by `set_attribute("style", …)` and `set_styles`, and both
        // put the `style` attribute in the map first, so a cache with no
        // attribute cannot exist.
        //
        // The one removal that is a write whether or not the attribute is
        // there: a `<textarea>`'s `value`. rinch-web removes it by writing
        // `.value = ""`, which sets the browser's dirty value flag for good, so
        // the field empties and stops following its text children — a pristine
        // field with no attribute included (#1222).
        if name == "value"
            && !self.tree.nodes[node.0].value_dirty
            && self.tree.nodes[node.0].tag() == Some("textarea")
        {
            self.tree.nodes[node.0].value_dirty = true;
            if !self.tree.nodes[node.0].attributes.contains_key(name) {
                // What the field shows went from its children to `""`; no
                // attribute changed, so no selector can see it.
                self.tree.hit_cache.invalidate();
                self.push_dirty_flags(node.0, DirtyFlags::LAYOUT | DirtyFlags::PAINT);
                return;
            }
        }
        if !self.tree.nodes[node.0].attributes.contains_key(name) {
            return;
        }
        // Past the no-op return, like `set_attribute`'s.
        self.tree.hit_cache.invalidate();
        // The option is losing a `selected` it really carries (#692). The early
        // return above already guarantees the attribute is present, so this
        // clause only has to decide that the node is an `<option>`.
        let deselects_this_option =
            name == "selected" && self.tree.nodes[node.0].tag() == Some("option");

        if name == "data-onmousemove" {
            self.tree.mousemove_handlers = self.tree.mousemove_handlers.saturating_sub(1);
        }
        if name == "title" {
            self.tree.title_sources = self.tree.title_sources.saturating_sub(1);
        }
        // Before the erase, as in `set_attribute`.
        self.note_attribute_change(node.0, name);
        self.tree.nodes[node.0].erase_attribute(name);
        // Losing the attribute deselects the option (#692). Nothing takes its
        // place: with no option selected, `resolve_select_model` falls to its
        // first non-disabled option, which is HTML's "ask for a reset".
        if deselects_this_option {
            crate::select::set_option_selectedness(&mut self.tree, node.0, false);
        }
        // Without its `label` the option's label is its text again (#1128).
        if name == "label" && self.tree.nodes[node.0].tag() == Some("option") {
            self.note_select_content_changed(node.0);
        }
        // Removing the select's own `value` attribute retires its claim to
        // freshness too (#757) — there's nothing left for step 1 to answer
        // from, and a later re-write starts the race over cleanly.
        if name == "value" && self.tree.nodes[node.0].tag() == Some("select") {
            self.tree.select_value_fresh.remove(&node.0);
        }
        if name == "style" {
            self.tree.nodes[node.0].style_attribute_cache = None;
        }
        self.sync_presentational_hints(node.0, name);
        // As in `set_attribute` (#2309): an `<img>` without its `src` has no
        // picture, and one without its `alt` no alt text.
        if matches!(name, "src" | "alt") {
            self.sync_img_placeholder(node.0);
        }
        // Symmetric with `set_attribute`: removing an attribute a selector
        // named (`[data-highlighted]`, `[aria-selected]`) restyles what
        // Stylo's invalidator says it reaches.
        self.push_dirty_flags(
            node.0,
            DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT,
        );
    }

    fn attribute_list(&self, node: NodeId) -> Vec<(String, String)> {
        self.tree.nodes.get(node.0).map_or_else(Vec::new, |n| {
            n.attributes
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        })
    }

    fn get_attribute(&self, node: NodeId, name: &str) -> Option<String> {
        let n = self.tree.nodes.get(node.0)?;
        // Folded like the write (#688), so `getAttribute("ID")` answers for an
        // element written with `id=` and vice versa, as in a browser.
        let folded = crate::attr_name::fold_attribute_name(n.tag(), name);
        n.attributes.get(folded.as_ref()).cloned()
    }

    /// The `value` attribute, as the trait's default answers — except that a
    /// `<textarea>` with none answers its text children, its default value
    /// (#1159), and an `<input>` answers it sanitized for its type (line
    /// breaks stripped, #1554), as the web's `.value` property does.
    fn live_value(&self, node: NodeId) -> Option<String> {
        if matches!(
            self.tree.nodes.get(node.0)?.tag(),
            Some("textarea" | "input")
        ) {
            return crate::form_control::control_value(&self.tree.nodes, node.0)
                .map(|v| v.into_owned());
        }
        self.get_attribute(node, "value")
    }

    fn set_highlight_owner(&mut self, overlay: NodeId, owner: Option<NodeId>) {
        let owner = owner.map(|o| o.0).filter(|&o| o != overlay.0);
        let Some(node) = self.tree.nodes.get_mut(overlay.0) else {
            return;
        };
        if node.highlight_owner == owner {
            return;
        }
        node.highlight_owner = owner;
        // Its pixels are where they were, drawn at another point of the order:
        // repaint its box.
        self.tree.paint_dirty_nodes.push(overlay.0);
    }

    fn set_style(&mut self, node: NodeId, property: &str, value: &str) {
        // Anything this can move invalidates the hit tester's memo.
        self.tree.hit_cache.invalidate();
        self.set_styles(node, &[(property, value)]);
    }

    fn set_styles(&mut self, node: NodeId, properties: &[(&str, &str)]) {
        // #723: an id that names no live node is a no-op, as on the web.
        if !self.tree.nodes.contains(node.0) {
            return;
        }
        // Anything this can move invalidates the hit tester's memo.
        self.tree.hit_cache.invalidate();
        // Merge into the inline style attribute and parse it once; both paths
        // below store exactly this string and this declaration block.
        let style_str = self.merged_inline_style(node.0, properties);
        // Nothing to do when the merge leaves the inline style as it was (see
        // the identical-write skip in `set_attribute`).
        if self.tree.nodes[node.0].attributes.get("style") == Some(&style_str) {
            return;
        }
        let pdb = parse_inline_style(&style_str);
        let insets = self.inset_fast_path_values(node.0, properties, &pdb);
        // Before the write, as `set_attribute` does: a `[style*=…]` selector
        // sees the change. Off the fast path the element is restyled too; on
        // it, the inset is written directly and a snapshot is taken only if
        // some rule names the `style` attribute.
        if insets.is_none() {
            self.note_attribute_change(node.0, "style");
        } else {
            self.note_inset_style_write(node.0);
        }

        self.tree.nodes[node.0]
            .attributes
            .insert("style".to_string(), style_str);
        self.cache_inline_style(node.0, pdb);

        match insets {
            Some(insets) => self.apply_inset_fast_path(node.0, &insets),
            None => self.invalidate_inline_style(node.0),
        }
    }

    /// Flags the node ([`crate::Node::style_isolated`]) and marks it with
    /// the `rinch-style-isolated` attribute, which the UA sheet's link rules
    /// for isolated content key on. Any children it already has restyle.
    fn isolate_styles(&mut self, node: NodeId) {
        let Some(n) = self.tree.nodes.get_mut(node.0) else {
            return;
        };
        if n.style_isolated {
            return;
        }
        n.style_isolated = true;
        self.tree.isolated_nodes += 1;
        self.set_attribute(node, "rinch-style-isolated", "");
        self.mark_restyle(node.0, true);
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

impl RinchDocument {
    /// Called when the children or text under `node` changed, or an
    /// `<option>`'s `label` attribute did: when `node` is, or sits anywhere
    /// inside the options of, a `<select>`, queue that select for a Taffy
    /// style re-sync. An unstyled closed select is as wide as its widest
    /// option label (`apply_stylo_styles_to_taffy`), which is an input no
    /// selector and no style of the select's own carries.
    ///
    /// An option's label is the text of its whole subtree (`select::element_text`),
    /// so the walk passes any element on its way up to the `<option>` — a
    /// reactive `option { {|| name} }` puts its text in a `display: contents`
    /// span (#1128). Past the option, only `<optgroup>` may stand between it
    /// and the select. In a document that has never created an `<option>`
    /// this returns at once; otherwise it walks the node's ancestors.
    pub(super) fn note_select_content_changed(&mut self, node: RawNodeId) {
        if self.tree.option_elements == 0 {
            return;
        }
        let mut current = Some(node);
        let mut in_option = false;
        while let Some(id) = current {
            let Some(n) = self.tree.nodes.get(id) else {
                return;
            };
            match n.tag() {
                Some("select") => {
                    self.tree.style_dirty_nodes.push(id);
                    self.tree.styles_dirty = true;
                    return;
                }
                Some("option") if !in_option => in_option = true,
                Some("optgroup") => {}
                // Above the option nothing else may stand before the select.
                Some(_) if in_option => return,
                _ => {}
            }
            current = n.parent;
        }
    }
}
