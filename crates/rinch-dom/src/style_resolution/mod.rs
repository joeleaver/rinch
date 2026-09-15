//! Style resolution: Stylo CSS cascade, Taffy sync, hover, and theme operations.

mod pseudo;
mod resolve;
mod state_tracking;

use servo_arc::Arc as ServoArc;

use rinch_core::dom::NodeId;
use style::context::QuirksMode;

use crate::RinchDocument;
use crate::computed_style::ComputedStyle;
use crate::layout;
use crate::node::{DirtyFlags, DisplayMode, NodeTree};

impl RinchDocument {
    /// Load CSS into the document's stylesheet.
    ///
    /// Parses the CSS string and merges rules/variables into the existing stylesheet.
    /// Call this at startup to load theme and component CSS.
    pub fn load_css(&mut self, css: &str) {
        self.load_stylo_css(css);
    }

    /// Create a DOM node from a parsed HTML node (recursive).
    pub(crate) fn create_node_from_parsed(
        &mut self,
        parsed: &crate::html_parser::ParsedNode,
    ) -> NodeId {
        use crate::html_parser::ParsedNode;
        use rinch_core::dom::DomDocument;

        match parsed {
            ParsedNode::Element {
                tag,
                attrs,
                children,
            } => {
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
            ParsedNode::Text(text) => self.create_text(text),
        }
    }

    /// If the given node is a `<style>` element, extract its text children's content
    /// and load it into the stylesheet.
    pub(crate) fn maybe_load_style_css(&mut self, node_id: usize) {
        let is_style = self
            .tree
            .nodes
            .get(node_id)
            .and_then(|n| n.tag())
            .map(|t| t == "style")
            .unwrap_or(false);
        if !is_style {
            return;
        }

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
            // New CSS rules may affect any existing node — invalidate all caches
            // and clear style_roots to force a full tree walk.
            for (nid, _) in self.tree.nodes.iter() {
                *self.tree.nodes[nid].stylo_element_data.borrow_mut() = None;
            }
            self.tree.style_roots.clear();
            self.tree.styles_dirty = true;
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

        // Create a new Device with the updated viewport. Everything else the
        // Device carries is rebuilt from `device_params`, so a resize cannot
        // silently reset the root font-size (#279) or the device pixel ratio
        // (#211) back to their defaults.
        let device = crate::dom_impl::build_device(width, height, &self.device_params);

        // Update the stylist's device using StylesheetGuards
        let guard = self.tree.guard.read();
        let guards = StylesheetGuards::same(&guard);
        self.stylist.set_device(device, &guards);

        // Mark all stylesheet origins as dirty to force style recomputation with new viewport
        self.stylist
            .force_stylesheet_origins_dirty(Origin::UserAgent.into());
        self.stylist
            .force_stylesheet_origins_dirty(Origin::Author.into());
    }

    /// Set the device pixel ratio (DPI scale factor) for the Stylo Device
    /// (issue #211).
    ///
    /// This drives the `resolution` media features (`@media (min-resolution:
    /// 2dppx)`), `image-set()` candidate selection, and border-width
    /// device-pixel snapping. It does **not** change layout geometry — 1 CSS
    /// px remains 1 layout unit. The value survives viewport-driven `Device`
    /// rebuilds.
    ///
    /// Every cached style is invalidated, since any rule gated on a
    /// resolution media query may now match differently.
    pub fn set_device_pixel_ratio(&mut self, dpr: f32) {
        if !dpr.is_finite() || dpr <= 0.0 || dpr == self.device_params.device_pixel_ratio {
            return;
        }
        self.device_params.device_pixel_ratio = dpr;

        // Rebuild the Device from the updated params (this also forces all
        // stylesheet origins dirty, which media-feature changes require).
        let viewport = self.tree.viewport;
        self.set_stylist_viewport(viewport.width, viewport.height);

        // Invalidate all cached styles and force a full re-resolve +
        // relayout, mirroring resolve_layout's viewport-change branch.
        for (node_id, _) in self.tree.nodes.iter() {
            *self.tree.nodes[node_id].stylo_element_data.borrow_mut() = None;
        }
        self.tree.style_roots.clear();
        self.tree.styles_dirty = true;
        self.tree.layout_dirty = true;
    }

    /// Parse a CSS string into an author-origin Stylo stylesheet.
    fn parse_author_stylesheet(&self, css: &str) -> style::stylesheets::DocumentStyleSheet {
        use style::media_queries::MediaList;
        use style::stylesheets::{AllowImportRules, DocumentStyleSheet, Origin, Stylesheet};

        let url_data = crate::layout::BLANK_URL_DATA.clone();

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

        DocumentStyleSheet(ServoArc::new(stylesheet))
    }

    /// Load CSS into Stylo's stylesheet system.
    ///
    /// Parses the CSS string and appends it to the Stylist for CSS cascade.
    /// Appended sheets cascade after everything loaded before them, matching
    /// document source order.
    pub fn load_stylo_css(&mut self, css: &str) {
        use style::stylesheets::Origin;

        let doc_stylesheet = self.parse_author_stylesheet(css);

        // Add the stylesheet to the stylist
        let guard = self.tree.guard.read();
        self.stylist
            .append_stylesheet(doc_stylesheet.clone(), &guard);
        drop(guard);

        // Track app author sheets in insertion order so the theme sheet can be
        // re-inserted ahead of them (see `set_theme_css`).
        self.author_stylesheets.push(doc_stylesheet);

        // Mark stylesheets as changed so they'll be flushed on next style computation
        self.stylist
            .force_stylesheet_origins_dirty(Origin::Author.into());

        self.recompute_bare_focus_rules();
    }

    /// Recompute [`Self::has_bare_focus_rules`] over the theme sheet and every
    /// app author sheet. See the field's doc for why bare (unanchored) focus
    /// selectors defeat the `focus_sensitive` invalidation scheme: stylo's
    /// `SelectorMap` puts them in a bucket that is only consulted while the
    /// element already has focus state, so matching never reaches them on an
    /// unfocused node. (The UA sheet is not scanned — it carries no focus
    /// rules.)
    fn recompute_bare_focus_rules(&mut self) {
        let sheets: Vec<style::stylesheets::DocumentStyleSheet> = self
            .theme_stylesheet
            .iter()
            .chain(self.author_stylesheets.iter())
            .cloned()
            .collect();
        let guard = self.tree.guard.read();
        let found = sheets.iter().any(|sheet| {
            let contents = sheet.0.contents.read_with(&guard);
            Self::rules_have_bare_focus(contents.rules(&guard), &guard)
        });
        drop(guard);
        self.has_bare_focus_rules = found;
    }

    /// Whether any style rule in `rules` (recursing into `@media` / `@supports`)
    /// has a selector whose rightmost compound contains a focus pseudo-class and
    /// no tag/class/id/attribute anchor — the shape stylo buckets into the
    /// state-gated `rare_pseudo_classes` map.
    fn rules_have_bare_focus(
        rules: &[style::stylesheets::CssRule],
        guard: &style::shared_lock::SharedRwLockReadGuard,
    ) -> bool {
        use style::stylesheets::CssRule;
        rules.iter().any(|rule| match rule {
            CssRule::Style(locked) => {
                let style_rule = locked.read_with(guard);
                style_rule
                    .selectors
                    .slice()
                    .iter()
                    .any(Self::selector_is_bare_focus)
            }
            CssRule::Media(media) => {
                Self::rules_have_bare_focus(&media.rules.read_with(guard).0, guard)
            }
            CssRule::Supports(supports) => {
                Self::rules_have_bare_focus(&supports.rules.read_with(guard).0, guard)
            }
            _ => false,
        })
    }

    /// Whether a selector's rightmost compound contains a focus pseudo-class
    /// (`:focus`, `:focus-visible`, `:focus-within` — directly or inside
    /// `:not()`/`:is()`/`:where()`) without any tag/class/id/attribute/`:root`
    /// anchor. Over-matching here is safe (it only costs an extra invalidation
    /// on focus change); under-matching re-introduces the never-restyled ring.
    fn selector_is_bare_focus(
        selector: &selectors::parser::Selector<style::selector_parser::SelectorImpl>,
    ) -> bool {
        use selectors::parser::Component;
        use style::selector_parser::NonTSPseudoClass;

        fn component_has_focus(c: &Component<style::selector_parser::SelectorImpl>) -> bool {
            match c {
                Component::NonTSPseudoClass(pc) => matches!(
                    pc,
                    NonTSPseudoClass::Focus
                        | NonTSPseudoClass::FocusVisible
                        | NonTSPseudoClass::FocusWithin
                ),
                Component::Negation(list) | Component::Is(list) | Component::Where(list) => list
                    .slice()
                    .iter()
                    .any(|s| s.iter().any(component_has_focus)),
                _ => false,
            }
        }

        let mut anchored = false;
        let mut has_focus = false;
        // `iter()` walks the rightmost compound only (stops at the first
        // combinator), which is exactly the compound stylo buckets by.
        for component in selector.iter() {
            match component {
                Component::LocalName(_)
                | Component::ID(_)
                | Component::Class(_)
                | Component::AttributeInNoNamespaceExists { .. }
                | Component::AttributeInNoNamespace { .. }
                | Component::AttributeOther(_)
                | Component::Root => anchored = true,
                other => {
                    if component_has_focus(other) {
                        has_focus = true;
                    }
                }
            }
        }
        has_focus && !anchored
    }

    /// Install (or replace) the theme stylesheet.
    ///
    /// The theme sheet occupies a stable slot *before* every app stylesheet, so
    /// app CSS always cascades over it. This mirrors rinch-web, where the theme
    /// lives in a single `<style data-rinch-theme>` in `<head>` that is updated
    /// in place — its document position never changes.
    ///
    /// Appending a regenerated theme sheet instead would move it *after* the
    /// app's `<style>` rules, letting theme `:root` custom properties silently
    /// win over an app's `:root` overrides of the same variable.
    pub fn set_theme_css(&mut self, css: &str) {
        use style::stylesheets::Origin;

        let new_sheet = self.parse_author_stylesheet(css);

        let guard = self.tree.guard.read();

        // Drop the previous theme sheet, if any.
        if let Some(old) = self.theme_stylesheet.take() {
            self.stylist.remove_stylesheet(old, &guard);
        }

        // Re-insert ahead of the first app sheet so app CSS keeps overriding the
        // theme. With no app sheets yet, appending puts it first anyway.
        match self.author_stylesheets.first() {
            Some(first_app) => {
                self.stylist
                    .insert_stylesheet_before(new_sheet.clone(), first_app.clone(), &guard);
            }
            None => {
                self.stylist.append_stylesheet(new_sheet.clone(), &guard);
            }
        }
        drop(guard);

        self.theme_stylesheet = Some(new_sheet);

        self.stylist
            .force_stylesheet_origins_dirty(Origin::Author.into());

        self.recompute_bare_focus_rules();
    }

    /// Get the default display type for a node based on its tag.
    pub(crate) fn default_display_for_node(&self, node_id: usize) -> layout::DefaultDisplay {
        if self.tree.nodes[node_id].display_mode.is_inline_level() {
            layout::DefaultDisplay::Inline
        } else {
            layout::DefaultDisplay::Block
        }
    }

    /// Update theme CSS variables without duplicating non-`:root` rules.
    /// After calling this, call `recompute_all_styles_full()` to apply the new variables.
    ///
    /// Replaces the theme sheet in its stable pre-app slot; see [`Self::set_theme_css`].
    pub fn update_theme_variables(&mut self, css: &str) {
        self.set_theme_css(css);
    }

    /// Recompute taffy styles for all element nodes, clearing cached style props
    /// so that CSS variables are re-resolved. Use this after `update_theme_variables()`.
    pub fn recompute_all_styles_full(&mut self) {
        // Clear cached Stylo element data so styles are recomputed.
        // Also clear text_layout so build_ifc_layouts() doesn't skip
        // IFC roots whose text content hasn't changed but whose style
        // (e.g. text color) has — the old Parley layout has stale brushes.
        let node_ids: Vec<usize> = self.tree.nodes.iter().map(|(id, _)| id).collect();
        for &nid in &node_ids {
            *self.tree.nodes[nid].stylo_element_data.borrow_mut() = None;
            self.tree.nodes[nid].text_layout = None;
        }
        // Disable transitions during full restyle — theme changes should apply
        // instantly. Without this, elements with `transition: color` would start
        // animating from old values, baking stale colors into Parley text layouts.
        let transitions_were_enabled = self.tree.transitions_enabled;
        self.tree.transitions_enabled = false;
        self.tree.active_transitions.clear();
        self.tree.active_animations.clear();
        // Clear roots to force full tree walk
        self.tree.style_roots.clear();
        // Resolve styles using Stylo
        self.tree.styles_dirty = true;
        self.resolve_styles();
        self.apply_stylo_styles_to_taffy();
        self.tree.transitions_enabled = transitions_were_enabled;
        // Force IFC rebuild so text layouts pick up new colors/fonts from
        // the updated computed styles (text brush is baked into Parley layout).
        // Also set layout_dirty so resolve_layout doesn't early-return before
        // reaching the ifc_dirty branch.
        self.tree.ifc_dirty = true;
        self.tree.layout_dirty = true;
    }

    /// Recompute taffy styles for all element nodes.
    /// Called when viewport dimensions change to update vh/vw-dependent styles.
    #[allow(dead_code)]
    pub(crate) fn recompute_all_styles(&mut self) {
        // When viewport changes, Stylo needs to know about it to recalculate vh/vw units
        // For now, just resolve styles and apply to Taffy
        self.tree.styles_dirty = true;
        self.resolve_styles();
        self.apply_stylo_styles_to_taffy();
    }

    /// Recompute styles recursively for a node and all its descendants.
    /// This is needed when a node is inserted into a new parent, as ancestor-based
    /// CSS selectors (like `.parent .child`) need to be re-evaluated with the new ancestor chain.
    /// Recompute Stylo styles for a node and all its descendants, then apply
    /// to Taffy. Called after bulk DOM operations to batch style resolution
    /// into a single pass.
    pub fn recompute_node_styles_recursive(&mut self, node_id: usize) {
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
        self.tree.style_roots.push(node_id);
        invalidate_recursive(&mut self.tree, node_id);

        // Resolve styles using Stylo
        self.tree.styles_dirty = true;
        self.resolve_styles();
        self.apply_stylo_styles_to_taffy();
        self.push_dirty_flags(
            node_id,
            DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT,
        );
    }

    /// Where in `parent_id`'s **Taffy** child list a DOM child that now sits at
    /// DOM index `dom_index` belongs.
    ///
    /// This used to count preceding DOM siblings that merely *have* a
    /// `taffy_id`, which rests on a premise that is false three separate ways
    /// (#477): *a node's Taffy child list ≡ its DOM children that have a
    /// `taffy_id`, in DOM order*.
    ///
    /// 1. **The IFC detach.** `mark_inline_descendants` removes every inline
    ///    child of an IFC root from the root's Taffy list — and every
    ///    `display: none` child (#487) — but leaves their `taffy_id` set. An
    ///    all-inline container has DOM children with `taffy_id`s and an
    ///    **empty** Taffy list, so the old count overshot and Taffy refused the
    ///    insert outright.
    /// 2. **`display: contents` flattening.** `sync_display_contents` splices a
    ///    wrapper's grandchildren into the parent's list in the wrapper's own
    ///    place, so **one** DOM sibling contributes **zero or N** slots. The
    ///    old count was wrong in *both* directions here, and in the "N" one it
    ///    stayed in range — a silently misplaced box, not a refused insert, so
    ///    clamping alone would not have caught it.
    /// 3. **The #466 measure leaf.** A Taffy child of an IFC root with no DOM
    ///    identity at all, deliberately at index 0. Any DOM-derived count is
    ///    off by one against such a parent.
    ///
    /// So ask the truth instead. Walk the preceding DOM siblings backwards;
    /// the first one that has **anything attached** decides — the answer is one
    /// past the last slot that sibling occupies. A sibling's occupancy is its
    /// *contribution* ([`Self::collect_taffy_contribution`]), not its own id,
    /// because a spliced wrapper's slots are its descendants'. This is correct
    /// under all three gap sources without enumerating them: a detached sibling
    /// is simply absent from the list, a wrapper answers with its real ids, and
    /// the measure leaf — matching no DOM node's contribution — is skipped
    /// rather than miscounted.
    ///
    /// The result is **always in range**: it is either `0` or `pos + 1` for a
    /// `pos` that indexes the live list. Callers clamp anyway
    /// ([`RinchDocument::attach_taffy_child_at`]) so that the property is local
    /// rather than remote.
    ///
    /// One imprecision is deliberate, and it is about the measure leaf. When no
    /// preceding sibling is attached the answer is `0`, which puts the new box
    /// *ahead* of a leaf sitting in slot 0. The leaf stands in for inline
    /// content scattered through DOM order, so no single slot for it is right;
    /// the IFC canonicalization rebuilds the whole list leaf-first on the next
    /// `ifc_dirty` pass regardless (`ifc.rs`), and every mutation entry point
    /// sets that flag. Ordering against the leaf is therefore transient in a
    /// way that attachment is not.
    ///
    /// The **nearest** attached preceding sibling decides, not the furthest —
    /// i.e. this stops at the first hit rather than taking the maximum over
    /// every preceding sibling. The two agree whenever the Taffy list is in
    /// (flattened) DOM order, which every rebuild pass puts it in. They differ
    /// only on a list already out of order, where the maximum would land after
    /// *all* preceding siblings and this can land between two of them —
    /// neither is right, and the next rebuild overwrites both. Taking the
    /// maximum would cost a full scan of the preceding siblings on every
    /// insert, including the common append-to-a-long-list, so the cheap rule
    /// wins a choice between two guesses.
    pub(crate) fn compute_taffy_child_index(&self, parent_id: usize, dom_index: usize) -> usize {
        let Some(parent_taffy) = self.tree.nodes[parent_id].taffy_id else {
            return 0;
        };
        let Ok(attached) = self.tree.taffy.children(parent_taffy) else {
            return 0;
        };
        // An empty list has exactly one valid index, and this is the common
        // case — every all-inline IFC root is here after the detach pass.
        if attached.is_empty() {
            return 0;
        }
        let children = &self.tree.nodes[parent_id].children;
        let mut contribution: Vec<taffy::NodeId> = Vec::new();
        for i in (0..dom_index.min(children.len())).rev() {
            contribution.clear();
            Self::collect_taffy_contribution(&self.tree.nodes, children[i], &mut contribution);
            // The *last* slot this sibling occupies. `max` rather than "the
            // position of the last contributed id" is **defensive, not
            // demonstrated**: the contribution is collected in DOM pre-order,
            // so the two agree unless the parent's attached list is itself out
            // of contribution order, and every rebuild pass writes flattened
            // DOM order. Swapping this for `.last()` survives the suite. Kept
            // because it costs nothing and is right under a list this function
            // did not produce.
            let last = contribution
                .iter()
                .filter_map(|id| attached.iter().position(|a| a == id))
                .max();
            if let Some(pos) = last {
                return pos + 1;
            }
        }
        0
    }

    /// Apply Stylo computed styles to Taffy layout nodes.
    ///
    /// This reads from each element's `stylo_element_data` and sets the corresponding
    /// Taffy style. It also updates our `ComputedStyle` for paint operations.
    ///
    /// When transitions are enabled, property changes are intercepted and animated
    /// instead of applied immediately.
    ///
    /// PERFORMANCE: Only processes nodes in `style_dirty_nodes` (set by resolve_styles).
    pub fn apply_stylo_styles_to_taffy(&mut self) {
        use crate::transition::{
            TransitionSpec, apply_value_to_style, diff_animatable, start_transitions,
        };

        // Take the dirty nodes list - only these need Taffy sync
        let dirty_node_ids = std::mem::take(&mut self.tree.style_dirty_nodes);

        // If no dirty nodes, nothing to do
        if dirty_node_ids.is_empty() {
            return;
        }
        let perf = std::env::var("RINCH_PERF").is_ok();
        let style_dirty_count = dirty_node_ids.len();
        let taffy_style_changed_count = std::cell::Cell::new(0u32);

        // Get current time for transition start timestamps
        let current_time_ms = self.current_time_ms();

        // Nodes whose `display` was `none` when this cascade started. See the
        // push site below and [`Self::is_rendered_for_transition`] (#703).
        let mut was_hidden: Vec<usize> = Vec::new();

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
            if matches!(
                node.tag(),
                Some("style" | "script" | "head" | "meta" | "link" | "title")
            ) {
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
            let mut new_style = ComputedStyle::from_stylo(&computed_values);

            // `user-select` has no UA rule and this Stylo build does not parse
            // the property at all (which is also why the inline `style`
            // attribute is re-read for it further down), so the presentational
            // default for the code-ish elements has to be applied by tag here.
            //
            // The other HTML presentational defaults — `font-weight: bold` on
            // `<b>`/`<strong>`, `font-style: italic` on `<em>`/`<i>`, the
            // `text-decoration-line` on `<u>`/`<ins>`/`<s>`/`<strike>`/`<del>`
            // — are UA *rules* in `load_ua_stylesheet` and must stay there.
            // They used to be patched onto `new_style` here as well, and a
            // post-cascade patch gets two things wrong (issue #616). It cannot
            // tell "the author declared nothing" from "the author declared the
            // initial value", so `font-weight: normal` on a `<b>` lost to the
            // patch and computed 700 while `300` and `bold` worked. And it
            // cannot inherit, because inheritance happens inside the cascade —
            // so a descendant block of that `<b>` read the cascaded 400 while
            // the `<b>` itself read the patched 700, one cause presenting as
            // two consumers disagreeing.
            if matches!(node.tag(), Some("code" | "pre" | "kbd" | "samp")) {
                new_style.user_select = crate::computed_style::UserSelectValue::Text;
            }

            // `<textarea rows=N>` maps to an intrinsic height of N lines, as
            // browsers do. A textarea holds its value in an attribute rather
            // than as a text child, so nothing else gives it a content height —
            // without this it collapses to a single line regardless of `rows`.
            // The HTML default is 2 rows.
            if node.tag() == Some("textarea") && new_style.height.lays_out_as_auto() {
                let rows = node
                    .attributes
                    .get("rows")
                    .and_then(|r| r.trim().parse::<f32>().ok())
                    .filter(|r| *r >= 1.0)
                    .unwrap_or(2.0);
                let line_h = new_style.line_height_px();
                // min-height is a border-box value (rinch sets a global
                // `box-sizing: border-box`, and Taffy defaults to it), so the
                // padding and border have to be added on top of the line boxes.
                let intrinsic = rows * line_h
                    + new_style.padding_top.to_px()
                    + new_style.padding_bottom.to_px()
                    + new_style.border_top_width.to_px()
                    + new_style.border_bottom_width.to_px();
                // An author `min-height` still wins when it is the larger of
                // the two, matching `max(rows, min-height)` in browsers.
                let author_min = match new_style.min_height {
                    crate::computed_style::DimensionValue::Length(px) => px,
                    _ => 0.0,
                };
                new_style.min_height =
                    crate::computed_style::DimensionValue::Length(intrinsic.max(author_min));
            }

            // A closed `<select>` shows one option's label, so browsers size it to
            // fit the *widest* option — the width stays stable when the selection
            // changes. Its `<option>` children are `display:none` and give it no
            // content, so without this the control collapses to its padding and
            // clips the label. Only applied when the author left `width: auto`; an
            // explicit width is respected and the label clips (the painter clips
            // too). The text width is estimated from the label length rather than
            // measured with Parley (not available at style-resolution time) —
            // erring wide is harmless since the painter clips to the content box.
            if node.tag() == Some("select") && new_style.width.lays_out_as_auto() {
                let model = crate::select::resolve_select_model(&self.tree, node_id);
                let widest = model
                    .options
                    .iter()
                    .map(|o| o.label.chars().count())
                    .max()
                    .unwrap_or(0);
                if widest > 0 {
                    let text_w = widest as f32 * new_style.font_size * 0.62;
                    // border-box, and padding_right already reserves the arrow box.
                    let intrinsic = text_w
                        + new_style.padding_left.to_px()
                        + new_style.padding_right.to_px()
                        + new_style.border_left_width.to_px()
                        + new_style.border_right_width.to_px();
                    let author_min = match new_style.min_width {
                        crate::computed_style::DimensionValue::Length(px) => px,
                        _ => 0.0,
                    };
                    new_style.min_width =
                        crate::computed_style::DimensionValue::Length(intrinsic.max(author_min));
                }
            }

            // Check inline style for user-select override (Stylo servo build
            // doesn't handle this property).
            //
            // Through the one inline-style parser (#670), not a bare
            // `split(';')`: a `content: "…;…"` or a `url(data:…;base64,…)`
            // earlier in the attribute is one declaration, and splitting on the
            // `;` inside it fabricates a fragment that can carry a `:` and be
            // read as a declaration of its own.
            if let Some(style_str) = node.attributes.get("style")
                && let Some((_, value)) = rinch_core::dom::split_declarations(style_str)
                    .iter()
                    .find(|(key, _)| key == "user-select")
            {
                new_style.user_select = crate::computed_style::UserSelectValue::parse(value);
            }

            // Capture old display before transitions overwrite computed_style
            let old_display = self.tree.nodes[node_id].computed_style.display;

            // A node whose display was `none` *before* this cascade is recorded
            // for the ancestor walk below (#703). The cascade pushes parents
            // before children, so by the time a descendant is reached an
            // ancestor restyled on this same pass already carries its **new**
            // display — and "was this element being rendered before the change"
            // is exactly the question §3 asks. Only hidden nodes go in, so this
            // stays empty (and unallocated) on every pass with nothing hidden.
            if matches!(old_display, crate::computed_style::DisplayValue::None) {
                was_hidden.push(node_id);
            }

            // …and whether the shaped text this node's style produced is still
            // the text this style would produce (issue #654). A `text_layout`
            // is derived from the typography below it, but `build_ifc_layouts`
            // re-serves one to any IFC root whose `max_width` is unchanged, so
            // a recascade that changes the font, size, colour or line height
            // and does not say so leaves the old glyphs on screen. Read here,
            // acted on below — the assignment in between is what destroys the
            // evidence. See `ComputedStyle::same_text_layout_inputs`.
            //
            // There is deliberately **no `!has_been_styled` special case**. A
            // node being styled for the first time compares against
            // `ComputedStyle::default()`, and if that somehow matched, skipping
            // would cost nothing: a node that has never been styled has no
            // `text_layout` to drop and a Taffy node that was created moments
            // ago and is already dirty. The one thing the invalidation would
            // still do for it is `invalidate_ifc_for_node`'s ancestor-walk
            // fallback, reaching up to whatever IFC root now contains it — and
            // the `append_child` that put it there has already invalidated that
            // root. An earlier revision carried the clause; it survived every
            // one of the 1014 rinch-dom tests, and it costs an O(depth) walk per
            // node on first build, so it is gone rather than pinned.
            let text_layout_stale = !self.tree.nodes[node_id]
                .computed_style
                .same_text_layout_inputs(&new_style);

            // …and, narrower, whether the *measured size* that layout took from
            // it is still the size this style would measure (issue #678). The
            // two questions are different and the gap between them is the cheap
            // path: see `ComputedStyle::same_measured_text_inputs`.
            let measured_size_stale = !self.tree.nodes[node_id]
                .computed_style
                .same_measured_text_inputs(&new_style);

            // Extract transition specs from Stylo
            let transition_specs = TransitionSpec::extract_from_stylo(&computed_values);
            self.tree.nodes[node_id].transition_specs = transition_specs;

            // --- Transition logic ---
            let specs = &self.tree.nodes[node_id].transition_specs;
            let node_has_been_styled = self.tree.nodes[node_id].has_been_styled;
            if self.tree.transitions_enabled && node_has_been_styled && !specs.is_empty() {
                let old_style = &self.tree.nodes[node_id].computed_style;
                let diffs = diff_animatable(old_style, &new_style);

                // css-transitions-1 §3 defines the before-change style only for
                // an element that is **being rendered**, so no transition
                // starts for one that is not — and a `display: none` element,
                // or any descendant of one, is not (issue #703). The answer is
                // not in this node's own `ComputedStyle`: `display` does not
                // inherit, so a box under a hidden wrapper computes
                // `display: block` and says nothing about it. Hence the walk,
                // placed **here** rather than beside `has_been_styled` above:
                // it is the last thing asked before a transition is started, so
                // a node that declares no `transition`, or whose cascade found
                // no animatable change, never pays for it.
                if !diffs.is_empty()
                    && Self::is_rendered_for_transition(
                        &self.tree,
                        node_id,
                        old_display,
                        new_style.display,
                        &was_hidden,
                    )
                {
                    // Clone specs for borrow-checker (specs borrows from tree.nodes)
                    let specs_clone: Vec<TransitionSpec> = specs.clone();

                    let transitions_map = self.tree.active_transitions.entry(node_id).or_default();

                    let transitioning =
                        start_transitions(transitions_map, &specs_clone, &diffs, current_time_ms);

                    // Apply new_style to computed_style, but for transitioning
                    // properties, keep the current interpolated value
                    self.tree.nodes[node_id].computed_style = new_style.clone();

                    // Overwrite transitioning properties with their current interpolated values
                    for prop in &transitioning {
                        if let Some(trans_map) = self.tree.active_transitions.get(&node_id)
                            && let Some(transition) = trans_map.get(prop)
                            && let Some(value) = transition.value_at(current_time_ms)
                        {
                            apply_value_to_style(
                                &mut self.tree.nodes[node_id].computed_style,
                                *prop,
                                &value,
                            );
                        }
                    }
                } else {
                    // No animatable diffs, or nothing rendered to animate —
                    // apply directly. This is the branch that makes a change
                    // made while hidden *land*: the hidden element takes the
                    // new value outright, so it is already there when it is
                    // shown, which is what a browser does.
                    self.tree.nodes[node_id].computed_style = new_style.clone();
                }
            } else {
                // No transitions — apply directly (current behavior)
                self.tree.nodes[node_id].computed_style = new_style.clone();
            }

            // --- Animation logic ---
            // Extract animation specs from Stylo
            let animation_specs =
                crate::animation::AnimationSpec::extract_from_stylo(&computed_values);
            self.tree.nodes[node_id].animation_specs = animation_specs;

            if self.tree.transitions_enabled {
                let anim_specs = self.tree.nodes[node_id].animation_specs.clone();
                let base_style = self.tree.nodes[node_id].computed_style.clone();
                let guard = self.tree.guard.read();

                // Extract active_animations temporarily to avoid borrow conflict
                let mut active_animations = std::mem::take(&mut self.tree.active_animations);
                crate::animation::start_animations(
                    &mut active_animations,
                    node_id,
                    &anim_specs,
                    &base_style,
                    &self.stylist,
                    &guard,
                    &self.tree,
                    current_time_ms,
                );
                self.tree.active_animations = active_animations;
                drop(guard);

                // Apply current animation values on top of computed_style
                if let Some(animations) = self.tree.active_animations.get(&node_id) {
                    for anim in animations {
                        if let crate::animation::AnimationResult::Values(values) =
                            anim.values_at(current_time_ms)
                        {
                            for (prop, value) in &values {
                                apply_value_to_style(
                                    &mut self.tree.nodes[node_id].computed_style,
                                    *prop,
                                    value,
                                );
                            }
                        }
                    }
                }
            }

            // An element that **stops** being rendered has its transitions
            // cancelled, and so does everything under it (css-transitions-1 §3,
            // issue #703). It has to descend: a descendant's own cascade need
            // not run at all when an ancestor is hidden — nothing about the
            // descendant's own style changed — so this cannot be a per-node
            // check in the gate above. Guarded on the map being non-empty,
            // which is the usual state, so an app with nothing transitioning
            // pays nothing for the walk.
            if !matches!(old_display, crate::computed_style::DisplayValue::None)
                && matches!(new_style.display, crate::computed_style::DisplayValue::None)
                && !self.tree.active_transitions.is_empty()
            {
                self.cancel_transitions_in_subtree(node_id);
            }

            // Reset scroll offset when an element transitions from display:none
            // to visible. This prevents stale scroll positions from a previous
            // display cycle (e.g., a menu flyout that was scrolled while the
            // window was small, then the window grows and the flyout reopens).
            if matches!(old_display, crate::computed_style::DisplayValue::None)
                && !matches!(new_style.display, crate::computed_style::DisplayValue::None)
            {
                self.tree.nodes[node_id].scroll_offset = (0.0, 0.0);
            }

            // Mark node as styled so future changes can trigger transitions
            self.tree.nodes[node_id].has_been_styled = true;

            // Drop the Parley layout the old typography was baked into, and
            // every measurement taken from it (#654, #661, #678) —
            // `invalidate_text_measure_for_node` is the one place that knows
            // what those are. Its first step, `invalidate_ifc_for_node`, is the
            // one place that knows which node actually *holds* the layout: this
            // node when it is the IFC root, its `ifc_root` when it is an
            // inline inside one, and an ancestor walk for anything else.
            //
            // **It marks Taffy twice, and both marks are load-bearing on
            // different shapes** — measured, because the obvious attribution is
            // wrong. The plain `taffy.mark_dirty(root)` is what re-measures an
            // atomic inline that shrink-wraps to its text
            // (`a_moved_inline_block_is_remeasured_in_its_new_font`); deleting
            // it alone kills that fixture and nothing else. The extra
            // `mark_ifc_measure_dirty` reaches the #466 measure leaf, which a
            // mark on the root does not, and matters only for an IFC root that
            // *has* one — a root with an out-of-flow child — on a pass that
            // recomputes Taffy without rebuilding the IFC structure, since a
            // structural pass mints the leaf fresh and dirty anyway. Deleting
            // it alone leaves every other test in the crate green and kills
            // `an_ifc_root_with_a_measure_leaf_is_remeasured_in_its_new_font`.
            //
            // Gated on a real change. **The gate is not backed by a
            // measurement**, and the honest reason it is here is mechanical
            // rather than empirical: invalidating unconditionally would re-shape
            // every restyled node's text whether or not any typography changed,
            // and a scalar comparison in place of a guaranteed invalidation
            // cannot be the slower of the two. An earlier revision of #654
            // claimed a 2-9x gap on a 500-row keyed reversal. That was measured
            // without interleaving the variants, on a host running at four times
            // its core count, and it is withdrawn: neither this PR's reviewer
            // nor a re-run with the binaries built once and alternated could
            // reproduce it. `tests/restyle_invalidation_bench.rs` is that
            // harness, `#[ignore]`d, so the next person measures rather than
            // inherits a number. Both variants are correct, so nothing
            // behavioural can tell them apart — `the_staleness_gate_lists_what_
            // an_inline_layout_is_built_from` is the only pin on the predicate's
            // contents.
            if text_layout_stale {
                self.invalidate_text_measure_for_node(node_id);
            }

            // A typography change that re-wraps the text has to re-run Taffy,
            // or the box keeps the size it was measured at in the old font
            // (issue #678). `font-family`, `font-weight`, `font-style`,
            // `line-height` (as a number), `letter-spacing`, `word-spacing`,
            // `text-transform`, `white-space` and `overflow-wrap` are not Taffy
            // properties, so the comparison further down never fires for them
            // and `layout_dirty` stayed false: `resolve_layout` took its
            // text-only branch, rebuilt the Parley layout in the new font and
            // returned without a compute, leaving a block that should have gone
            // 40 → 80 tall at 40.
            //
            // **`font-size` is on that list too, and that is not obvious.** It
            // reaches the Taffy style only through a value that *uses* it — an
            // `em` length, a `line-height` multiplier baked to px — so a box
            // sized in `px` around text sized in `px` changes no Taffy field at
            // all. The measured-inputs predicate is what catches it, and it is
            // why #661's `font-size: 16px → 32px` repro froze before this line
            // as well as after it.
            //
            // Narrower than `text_layout_stale` on purpose: a `:hover { color }`
            // must still take the cheap path, which is what the early return in
            // `resolve_layout` exists for.
            if measured_size_stale {
                self.tree.layout_dirty = true;
            }

            // Sync display_mode from computed style (always from new_style target)
            let display_mode = match new_style.display {
                crate::computed_style::DisplayValue::Inline => DisplayMode::Inline,
                crate::computed_style::DisplayValue::InlineBlock => DisplayMode::InlineBlock,
                crate::computed_style::DisplayValue::InlineFlex => DisplayMode::InlineFlex,
                crate::computed_style::DisplayValue::Block => DisplayMode::Block,
                crate::computed_style::DisplayValue::Flex => DisplayMode::Flex,
                crate::computed_style::DisplayValue::Grid => DisplayMode::Block,
                crate::computed_style::DisplayValue::InlineGrid => DisplayMode::InlineGrid,
                crate::computed_style::DisplayValue::None => DisplayMode::Block,
                crate::computed_style::DisplayValue::Contents => DisplayMode::Block,
            };
            let old_display_mode =
                std::mem::replace(&mut self.tree.nodes[node_id].display_mode, display_mode);

            // **Any** change of [`DisplayMode`] re-runs the IFC pass, because
            // that enum is exactly "how the inline formatting machinery
            // classifies this box" — and the Taffy comparison below cannot be
            // trusted to notice one, for exactly the reason the `contents`
            // crossing below it cannot (#597).
            // `DisplayValue::to_taffy` is not injective: `inline`, `block` and
            // — since #592 — `inline-block` all map to `taffy::Display::Block`,
            // `flex`, `inline-flex` and `contents` all map to
            // `taffy::Display::Flex`, and `grid` and `inline-grid` both map to
            // `taffy::Display::Grid` (#607). So `inline-block → flex` — a bare
            // `<button>` handed `display: flex` by a reactive `style:` closure,
            // which is #597's own repro — compared **equal** on every Taffy
            // field and never re-ran the IFC pass at all: the button stayed
            // detached, kept its inline-block box, and kept a stale `ifc_root`
            // that made `taffy_tree_violations`' invariant `C` exempt it.
            //
            // This used to ask only whether [`DisplayMode::is_inline_level`]
            // **crossed**, which was enough while every same-side pair differed
            // in its Taffy display. #592 broke that: `inline-block` maps to
            // `taffy::Display::Block` now, so `inline ↔ inline-block` is
            // inline-level on both sides *and* equal on every Taffy field —
            // two passes' worth of work skipped on a crossing that changes
            // whether the element is split (#513) and whether it is an IFC root
            // of its own. Measured, both directions: `inline → inline-block`
            // left the element with the split's parentless Taffy node and no
            // `ifc_root` (`C orphan`), and `inline-block → inline` left its
            // block child held by a node that is itself detached
            // (`D detached`). Both are clean under the declared twin, which is
            // what says the fault was the trigger and not the passes.
            //
            // Asking about the mode rather than re-spelling a predicate keeps
            // this and the marking pass on one authority — `Node::is_inline`
            // reads the same enum — and makes a mode added later trigger by
            // default rather than by remembering to widen a `matches!`.
            if old_display_mode != display_mode {
                self.tree.ifc_dirty = true;
                self.tree.layout_dirty = true;
            }

            // A node crossing into or out of `display: contents` changes the
            // *tree* — `sync_display_contents` must splice or heal — which the
            // Taffy-style comparison below cannot be trusted to notice (#520):
            // `Contents` maps to `taffy::Display::Flex`, so `flex → contents`
            // compares equal, and a spliced wrapper's Taffy style was stamped
            // `Display::None` by the sync pass, so `contents → none` compares
            // equal on the display field. Set the flags on the computed-display
            // crossing itself, independent of that comparison.
            if (old_display == crate::computed_style::DisplayValue::Contents)
                != (new_style.display == crate::computed_style::DisplayValue::Contents)
            {
                self.tree.ifc_dirty = true;
                self.tree.layout_dirty = true;
            }

            // Convert to Taffy style (from current computed_style which may have transition values)
            let dd = self.default_display_for_node(node_id);
            let mut taffy_style = self.tree.nodes[node_id].computed_style.to_taffy_style(dd);

            // HTML element must fill the viewport and clip horizontal overflow
            // (mirrors browser behavior where the viewport constrains content width).
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

            // An out-of-flow box whose containing block is not the Taffy parent
            // — `position: fixed` (the viewport), or a `position: absolute`
            // with no positioned ancestor (the initial containing block, #204)
            // — is sized here, before layout, so its own children lay out
            // inside the right box.
            if let Some(kind) = crate::out_of_flow::out_of_flow_kind(&self.tree, node_id) {
                crate::out_of_flow::apply_out_of_flow_size_overrides(
                    &self.tree.nodes[node_id],
                    kind,
                    self.tree.viewport,
                    &mut taffy_style,
                );
            }

            // Collapsed block (virtualized contenteditable): override height
            // to the estimated value so Taffy doesn't need a measure callback.
            if let Some(est_h) = self.tree.nodes[node_id].estimated_height {
                taffy_style.size.height = taffy::Dimension::length(est_h);
            }

            // A childless block container is one line box tall. The IFC pass
            // writes that floor straight onto the Taffy style, and this function
            // rebuilds the style from the computed values — so the floor has to
            // be re-applied here or the next restyle of the node drops it — it
            // would come back only on a structural change, the one thing that
            // re-runs the IFC pass. See `apply_empty_block_line_floor`.
            crate::ifc::apply_empty_block_line_floor(&self.tree.nodes[node_id], &mut taffy_style);

            // Only call set_style if the Taffy style actually changed.
            // This avoids marking the Taffy tree dirty for paint-only changes
            // (e.g. background-color on hover) which don't affect layout.
            if let Ok(old_taffy_style) = self.tree.taffy.style(taffy_id) {
                if old_taffy_style != &taffy_style {
                    // Only set ifc_dirty when display or position changes —
                    // that's what affects IFC structure. Display covers
                    // inline/block mixing, display:contents and display:none;
                    // position covers in-flow ↔ out-of-flow flips (Taffy's
                    // `position` is Absolute exactly for CSS absolute/fixed),
                    // which change whether a sibling run needs an anonymous
                    // box and whether this node's parent needs a measure leaf
                    // (#466) — a runtime `static → absolute` toggle would
                    // otherwise never re-run IFC setup and the leaf decision
                    // would go stale. Other layout changes (width, padding,
                    // margin) don't need IFC rebuild.
                    if old_taffy_style.display != taffy_style.display
                        || old_taffy_style.position != taffy_style.position
                    {
                        self.tree.ifc_dirty = true;
                    }
                    let _ = self.tree.taffy.set_style(taffy_id, taffy_style);
                    self.tree.layout_dirty = true;
                    // A Taffy style change reaches this node's own box through
                    // the compute — unless an atomic inline sits between it and
                    // the compute root, which is every node inside one (#661).
                    self.mark_atomic_inline_dirty(node_id);
                    taffy_style_changed_count.set(taffy_style_changed_count.get() + 1);
                }
            } else {
                let _ = self.tree.taffy.set_style(taffy_id, taffy_style);
                self.tree.layout_dirty = true;
                self.tree.ifc_dirty = true;
                taffy_style_changed_count.set(taffy_style_changed_count.get() + 1);
            }
        }
        if perf {
            eprintln!(
                "  [PERF] apply_to_taffy: style_dirty_nodes={} taffy_changed={}",
                style_dirty_count,
                taffy_style_changed_count.get()
            );
        }
    }

    /// Whether a transition may be **started** on `node_id` by this cascade.
    ///
    /// css-transitions-1 §3 defines a before-change style only for an element
    /// that is *being rendered*, so an element that is not being rendered has
    /// none and no transition starts for it — and its before-change style, were
    /// one needed, would be its after-change style. A `display: none` element is
    /// not being rendered, and neither is anything inside one (issue #703).
    ///
    /// Four things are asked, and each answers a case the others do not:
    ///
    /// - `old_display` — the node's own display **before** this cascade. This is
    ///   the one that says "it was hidden, and is being shown now": a single
    ///   class write can un-hide a box and retarget it at once, and at that
    ///   cascade the new display already reads `block`.
    /// - `new_display` — its display **after**. An element that is about to stop
    ///   being rendered starts nothing either; the cancel in the caller then
    ///   takes away whatever was already running.
    /// - every ancestor's current `display`, because `display` does not inherit:
    ///   a box under a hidden wrapper computes `display: block` and its own
    ///   style says nothing about whether it is rendered. rinch caches no
    ///   rendered bit — `read_layout_results` learns this by recursion, in
    ///   `zero_subtree_layout` — so the chain is walked.
    /// - `was_hidden`, the nodes this cascade has already found hidden. The
    ///   cascade pushes parents before children, so an ancestor restyled on this
    ///   same pass is carrying its *new* display by the time a descendant is
    ///   reached; without this list an ancestor shown and a descendant
    ///   retargeted by one class write would look rendered-all-along.
    ///
    /// # Cost
    ///
    /// O(depth), and only at the one site where the answer can change an
    /// outcome: after `diff_animatable` has found an animatable difference on a
    /// node that declares a `transition`. A node with neither never walks.
    ///
    /// `was_hidden` is a linear `Vec::contains` per ancestor step, so the pass
    /// as a whole is O(transitioning dirty nodes x depth x hidden dirty nodes).
    /// That product is the one shape worth naming, since "almost always none"
    /// says nothing about what happens when it is not: measured at **depth 50
    /// with 2000 hidden nodes restyled in the same pass** — 54,027,000
    /// comparisons over 27,000 ancestor steps — 6.560ms against 6.549ms with the
    /// walk stubbed out, i.e. still inside the noise. A `HashSet` would trade
    /// that for an allocation on every cascade that hides anything, which is the
    /// common case; the `Vec` is the right shape until a profile says otherwise.
    ///
    /// Two notes for anyone re-measuring. The hidden nodes must come **first in
    /// document order**, or DFS leaves `was_hidden` empty while the visible
    /// boxes are processed and the scan is never exercised at all — the numbers
    /// then look flat for the wrong reason. And the comparison has to be against
    /// the walk *stubbed*, not against `main`: deleting the gate changes which
    /// transitions start, which changes the work downstream of it.
    fn is_rendered_for_transition(
        tree: &NodeTree,
        node_id: usize,
        old_display: crate::computed_style::DisplayValue,
        new_display: crate::computed_style::DisplayValue,
        was_hidden: &[usize],
    ) -> bool {
        use crate::computed_style::DisplayValue;
        if matches!(old_display, DisplayValue::None) || matches!(new_display, DisplayValue::None) {
            return false;
        }
        let mut current = tree.nodes.get(node_id).and_then(|n| n.parent);
        while let Some(id) = current {
            let Some(node) = tree.nodes.get(id) else {
                break;
            };
            if matches!(node.computed_style.display, DisplayValue::None) || was_hidden.contains(&id)
            {
                return false;
            }
            current = node.parent;
        }
        true
    }

    /// Cancel every transition running on `node_id` or anything below it.
    ///
    /// Iterative, like [`RinchDocument::detach_subtree_styles`] — a deep subtree
    /// must not overflow the stack. Unlike that helper this clears **only**
    /// `active_transitions`: `has_been_styled` is left alone because
    /// [`Self::is_rendered_for_transition`] is what refuses a hidden node's
    /// transitions, and `active_animations` is left alone because a hidden
    /// element's `@keyframes` animations are a separate deviation with a
    /// separate blast radius — it decides restart semantics this does not have
    /// to (issue #747).
    fn cancel_transitions_in_subtree(&mut self, node_id: usize) {
        let mut stack = vec![node_id];
        while let Some(id) = stack.pop() {
            self.tree.active_transitions.remove(&id);
            if let Some(node) = self.tree.nodes.get(id) {
                stack.extend(node.children.iter().copied());
            }
        }
    }

    /// Get a monotonic timestamp in milliseconds for transition timing.
    fn current_time_ms(&self) -> f64 {
        use web_time::SystemTime;
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
            * 1000.0
    }
}
