//! Setting and removing an app's `data-*` attribute on textblocks
//! ([`crate::schema::data_attrs`]).

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
}
