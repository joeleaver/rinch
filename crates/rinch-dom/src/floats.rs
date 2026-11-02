//! `float` and `clear`.
//!
//! Taffy 0.14's block layout places floats (its `float_layout` feature): a
//! floated child of a block container goes to the left or right of its block
//! formatting context, and a box that establishes a formatting context of its
//! own is narrowed to the room beside it. What Taffy cannot do is wrap *lines*
//! around a float, because a rinch IFC is a Taffy leaf whose measure gets no
//! float context, and parley 0.11 has no per-line horizontal offset to start a
//! line after a left float.
//!
//! So a box that holds inline content and follows a float in the same
//! container is made to avoid the float as a whole: it is given paint
//! containment, which makes Taffy treat it as its own formatting context and
//! place it in the slot beside the float. Its lines fit that slot, which is
//! what a browser draws beside the float; where the text is longer than the
//! float, a browser flows the remaining lines back under it at full width and
//! rinch keeps them in the narrower column. Containment is set only on those
//! boxes, so nowhere else loses its baseline or scrollable overflow.

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

    /// Give every box that must avoid a float its containment, and take it
    /// from every box that no longer must. Run before each layout compute.
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
            // The white space around a float is a line of its own once the
            // float splits the run, where a browser's collapses to nothing.
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
                    && self.holds_lines(k)
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

    /// Whether the box behind Taffy node `t` is in flow, not floated, and
    /// lays out lines itself: an anonymous block box, a block whose own
    /// children are inline content, or a leaf such as an image.
    fn holds_lines(&self, t: taffy::NodeId) -> bool {
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
        if n.is_anonymous_block_box || self.tree.taffy.children(t).is_ok_and(|c| c.is_empty()) {
            return true;
        }
        n.children.iter().any(|&c| {
            let c = &self.tree.nodes[c];
            match c.text_content() {
                Some(text) => !text.trim().is_empty(),
                None => c.display_mode.is_inline_level(),
            }
        })
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
