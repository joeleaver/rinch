//! `float` and `clear`.
//!
//! Taffy 0.14's block layout places floats (its `float_layout` feature): a
//! floated child of a block container goes to the left or right of its block
//! formatting context, and a box that establishes a formatting context of its
//! own is narrowed to the room beside it. A paragraph's lines wrap around the
//! floats beside it line by line: its IFC root is a Taffy leaf, which rinch's
//! Taffy fork hands the block context it is laid out in (`float_wrap`).
//!
//! What is left here is what Taffy does not know about rinch's boxes. A
//! block-level replaced element or form control is a Taffy leaf in the same
//! formatting context, which Taffy lets overlap a float, where CSS 2.2 §9.5
//! has its border box avoid the float as a box that establishes a formatting
//! context does: it is given paint containment, which makes Taffy place it
//! in the room beside the float. And an anonymous box holding nothing but
//! the white space around a float, between two block boxes, is hidden, as a
//! browser's collapses to nothing.
//!
//! A float among inline content does not split the run around it (CSS 2.1
//! §9.2.1.1): it is out of flow (`InlineFlowRole::OutOfFlow`), stays a Taffy
//! child of its IFC root, and goes before the root's measure leaf, so Taffy
//! places it at the root's top and the lines wrap around it. A browser
//! places a float at the top of the line it stands in, which is the same
//! place for a float on a paragraph's first line; one on a later line goes
//! up to the first here.

use crate::RinchDocument;
use crate::computed_style::values::{DisplayValue, PositionValue};
use crate::node::RawNodeId;

impl RinchDocument {
    /// Record a box the cascade just computed as floated.
    pub(crate) fn note_float(&mut self, id: RawNodeId) {
        if !self.tree.floats.contains(&id) {
            self.tree.floats.push(id);
        }
    }

    /// Give every replaced box that must avoid a float its containment, take
    /// it from every one that no longer must, and hide the blank runs beside
    /// floats. Run before each layout compute.
    pub(crate) fn sync_float_avoidance(&mut self) {
        if self.tree.floats.is_empty()
            && self.tree.float_avoiders.is_empty()
            && self.tree.float_blank_runs.is_empty()
        {
            return;
        }
        let nodes = &self.tree.nodes;
        self.tree.floats.retain(|&id| {
            nodes
                .get_live(id)
                .is_some_and(|n| n.computed_style.float.is_floated())
        });

        let mut avoiders: Vec<RawNodeId> = Vec::new();
        let mut blank: Vec<RawNodeId> = Vec::new();
        for &id in &self.tree.floats {
            let Some(t) = self.tree.nodes[id].taffy_id else {
                continue;
            };
            let Some(parent) = self.tree.taffy.parent(t) else {
                continue;
            };
            let block_parent = self
                .tree
                .taffy
                .style(parent)
                .is_ok_and(|s| s.display == taffy::Display::Block);
            if !block_parent {
                continue;
            }
            let kids = self.tree.taffy.children(parent).unwrap_or_default();
            // A run of nothing but the white space around a float, between
            // two block boxes, takes no line: a browser's collapses to nothing.
            for &k in &kids {
                if let Some(&a) = self.tree.taffy_map.get(&k)
                    && !blank.contains(&a)
                    && self.is_blank_run(a)
                {
                    blank.push(a);
                }
            }
            let after = kids.iter().skip_while(|&&k| k != t).skip(1);
            for &k in after {
                if let Some(&a) = self.tree.taffy_map.get(&k)
                    && !avoiders.contains(&a)
                    && self.is_replaced_in_flow(k)
                {
                    avoiders.push(a);
                }
            }
        }

        let old = std::mem::take(&mut self.tree.float_avoiders);
        for &k in old.iter().filter(|k| !avoiders.contains(k)) {
            self.set_contain(k, taffy::Contain::NONE);
        }
        for &k in &avoiders {
            self.set_contain(k, taffy::Contain::PAINT);
        }
        self.tree.float_avoiders = avoiders;

        let old = std::mem::take(&mut self.tree.float_blank_runs);
        for &a in old.iter().filter(|a| !blank.contains(a)) {
            self.set_display(a, taffy::Display::Block);
        }
        for &a in &blank {
            self.set_display(a, taffy::Display::None);
        }
        self.tree.float_blank_runs = blank;
    }

    /// Whether `id` is an anonymous block box around nothing but collapsible
    /// white space.
    fn is_blank_run(&self, id: RawNodeId) -> bool {
        use crate::computed_style::values::WhiteSpaceValue;
        let n = &self.tree.nodes[id];
        n.is_anonymous_block_box
            && matches!(
                n.computed_style.white_space,
                WhiteSpaceValue::Normal | WhiteSpaceValue::NoWrap
            )
            && n.run_members.iter().all(|&m| {
                self.tree.nodes[m]
                    .text_content()
                    .is_some_and(|t| t.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\r')))
            })
    }

    fn set_display(&mut self, id: RawNodeId, display: taffy::Display) {
        let Some(t) = self.tree.nodes.get_live(id).and_then(|n| n.taffy_id) else {
            return;
        };
        if let Ok(old) = self.tree.taffy.style(t)
            && old.display != display
        {
            let mut st = old.clone();
            st.display = display;
            let _ = self.tree.taffy.set_style(t, st);
            self.tree.layout_dirty = true;
        }
    }

    /// Whether Taffy node `t` is a block-level replaced element or form
    /// control in flow: a leaf measured as a box, not as lines.
    fn is_replaced_in_flow(&self, t: taffy::NodeId) -> bool {
        use crate::node::NodeContext;
        let Some(&id) = self.tree.taffy_map.get(&t) else {
            return false;
        };
        let Some(n) = self.tree.nodes.get_live(id) else {
            return false;
        };
        let cs = &n.computed_style;
        if cs.float.is_floated()
            || matches!(cs.position, PositionValue::Absolute | PositionValue::Fixed)
            || !matches!(cs.display, DisplayValue::Block | DisplayValue::ListItem)
        {
            return false;
        }
        matches!(
            self.tree.taffy.get_node_context(t),
            Some(
                NodeContext::Image { .. }
                    | NodeContext::Replaced { .. }
                    | NodeContext::FormControl { .. }
            )
        )
    }

    /// Set `id`'s Taffy containment, if it still has a box.
    fn set_contain(&mut self, id: RawNodeId, contain: taffy::Contain) {
        let Some(t) = self.tree.nodes.get_live(id).and_then(|n| n.taffy_id) else {
            return;
        };
        if let Ok(old) = self.tree.taffy.style(t)
            && old.contain != contain
        {
            let mut st = old.clone();
            st.contain = contain;
            let _ = self.tree.taffy.set_style(t, st);
            self.tree.layout_dirty = true;
        }
    }
}
