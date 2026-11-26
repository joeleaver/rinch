//! Setting and removing an app's `data-*` attribute on textblocks
//! ([`crate::schema::data_attrs`]).

use crate::model::Slice;
use crate::pos::Pos;
use crate::schema::data_attrs::node_app_data;
use crate::schema::{MAX_DATA_ATTR_VALUE, MAX_DATA_ATTRS, is_app_data_attr};
use crate::transform::Transform;
use crate::transform::step::StepError;
use crate::transform::steps::SetNodeAttrStep;

impl Transform<'_> {
    /// Set (`Some`) or remove (`None`) the app data attribute `name` on every
    /// textblock overlapping `from..to` (a caret's own block when they are
    /// equal) whose type keeps data attributes
    /// ([`NodeSpec::data_attrs`](crate::schema::NodeSpec::data_attrs)); other
    /// blocks are passed over, as is one whose spec declares an attribute of
    /// that name. One attribute step per block that changes, none when no block
    /// does. Positions do not move.
    ///
    /// Errors, adding nothing, when `name` is not an attribute an app may keep
    /// ([`is_app_data_attr`]), when `value` is longer than
    /// [`MAX_DATA_ATTR_VALUE`] bytes, or when a block already holds
    /// [`MAX_DATA_ATTRS`] attributes and `name` would be one more: the model
    /// would drop an attribute to stay inside the bound, so it is refused here.
    pub fn set_block_data_attr(
        &mut self,
        from: usize,
        to: usize,
        name: &str,
        value: Option<&str>,
    ) -> Result<&mut Self, StepError> {
        if !is_app_data_attr(name) {
            return Err(StepError::new(format!(
                "`{name}` is not an app data attribute"
            )));
        }
        if value.is_some_and(|v| v.len() > MAX_DATA_ATTR_VALUE) {
            return Err(StepError::new(format!(
                "the value of `{name}` is longer than {MAX_DATA_ATTR_VALUE} bytes"
            )));
        }
        let size = self.doc.content_size();
        if from > to || to > size || self.doc.resolve(Pos(from)).is_err() {
            return Err(StepError::new("data attribute range out of bounds"));
        }
        let mut targets: Vec<usize> = Vec::new();
        let mut full = false;
        self.doc.nodes_between(from, to, &mut |node, pos, _parent| {
            let spec = node.node_type().spec();
            if node.is_textblock() && spec.data_attrs && !spec.attrs.contains_key(name) {
                let current = node.attrs().get_str(name);
                if current != value {
                    full |= current.is_none() && node_app_data(node).count() >= MAX_DATA_ATTRS;
                    targets.push(pos);
                }
            }
            true
        });
        if full {
            return Err(StepError::new(format!(
                "a block already holds {MAX_DATA_ATTRS} data attributes"
            )));
        }
        for pos in targets {
            self.step(Box::new(SetNodeAttrStep {
                pos,
                attr: name.into(),
                value: value.map(Into::into),
            }))?;
        }
        Ok(self)
    }

    /// Replace `[from, to)` with pasted **lines of plain text** (`slice`, as
    /// [`slice_from_text`](crate::serialize::slice_from_text) builds it),
    /// placing a block's app data attributes as Enter would: the lines are new
    /// text, so where they split a textblock the text after the range keeps
    /// the block's attributes, and where the range is at the block's start the
    /// first line's block has none. Anything else is
    /// [`replace_range`](Self::replace_range). Returns what that does, the
    /// position right after the inserted content.
    pub fn replace_range_with_lines(
        &mut self,
        from: usize,
        to: usize,
        slice: Slice,
    ) -> Result<usize, StepError> {
        // The textblock split, where it carries data attributes to place.
        let split = (slice.content.child_count() > 1)
            .then(|| {
                let r_from = self.doc.resolve(Pos(from)).ok()?;
                let r_to = self.doc.resolve(Pos(to)).ok()?;
                let block = r_from.parent();
                let splits = r_from.depth() > 0
                    && r_from.depth() == r_to.depth()
                    && r_from.start(r_from.depth()) == r_to.start(r_to.depth())
                    && block.is_textblock()
                    && node_app_data(block).next().is_some();
                let text_after = r_to.parent_offset() < block.content().size();
                let after = self.doc.content().size() - to;
                let at_start = r_from.parent_offset() == 0;
                splits.then(|| {
                    (
                        r_from.before(r_from.depth()),
                        block.clone(),
                        after,
                        at_start,
                        text_after,
                    )
                })
            })
            .flatten();
        let end = self.replace_range(from, to, slice)?;
        let Some((Some(head), block, after, at_start, text_after)) = split else {
            return Ok(end);
        };
        if !text_after {
            // Enter at the end: the block split keeps its attributes, also
            // when an empty first line took its place.
            if at_start && self.doc.resolve(Pos(head + 1)).is_ok() {
                for (name, value) in node_app_data(&block) {
                    self.set_block_data_attr(head + 1, head + 1, name, Some(value))?;
                }
            }
            return Ok(end);
        }
        // What was after the range is as far from the document's end as it was.
        let tail = self.doc.content().size().saturating_sub(after);
        let Ok(r) = self.doc.resolve(Pos(tail)) else {
            return Ok(end);
        };
        if r.depth() == 0 || r.before(r.depth()) == Some(head) {
            return Ok(end);
        }
        if let Some(tail) = r.before(r.depth()) {
            for (name, value) in node_app_data(&block) {
                self.set_block_data_attr(tail + 1, tail + 1, name, Some(value))?;
            }
        }
        if at_start {
            for (name, _) in node_app_data(&block) {
                self.set_block_data_attr(head + 1, head + 1, name, None)?;
            }
        }
        Ok(end)
    }
}
