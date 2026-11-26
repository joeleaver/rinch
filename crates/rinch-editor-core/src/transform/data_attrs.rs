//! Setting an app's `data-*` attributes on textblocks, and where they go when
//! an edit splits a textblock, joins two or pastes into one
//! ([`crate::schema::data_attrs`]).
//!
//! The attributes never decide the shape of a document: every edit fits,
//! splits and joins exactly as it would without them, and what is here runs
//! afterwards, setting them on the blocks the edit produced.

use crate::model::{Node, Slice};
use crate::pos::Pos;
use crate::schema::data_attrs::{node_app_data, same_element};
use crate::schema::{DataAttrSplit, MAX_DATA_ATTR_VALUE, MAX_DATA_ATTRS, is_app_data_attr};
use crate::transform::Transform;
use crate::transform::step::StepError;
use crate::transform::steps::SetNodeAttrStep;

/// Where a block stands among those a split makes of one.
#[derive(Clone, Copy)]
enum Place {
    First,
    Middle,
    Last,
}

/// What an edit that replaces a range puts there.
#[derive(Clone, Copy)]
pub(crate) enum Edit<'s> {
    /// Nothing, or typed text: whatever blocks the range's ends were in join.
    Join,
    /// Pasted content, whose blocks have attributes of their own.
    Paste(&'s Slice),
    /// Pasted lines of plain text: new text, as if Enter were pressed between
    /// them.
    Lines,
}

/// The textblocks the two ends of a range are in, as they were before the edit.
pub(crate) struct Ends {
    /// The position before the textblock the range starts in.
    head: usize,
    from_block: Node,
    to_block: Node,
    /// Whether both ends are in one textblock.
    same: bool,
    /// Whether the first textblock keeps text in front of the range.
    before: bool,
    /// Whether the last textblock keeps text after the range.
    after: bool,
    /// How far the range's end is from the document's end, which no step of
    /// the edit changes.
    from_end: usize,
}

type Pairs = Vec<(String, String)>;

fn pairs(node: &Node) -> Pairs {
    node_app_data(node)
        .map(|(n, v)| (n.to_string(), v.to_string()))
        .collect()
}

/// The first textblock in `slice` that its content is open into, if the slice
/// is open into one: what joins the textblock a paste starts in.
fn open_textblock(slice: &Slice) -> Option<&Node> {
    let mut node = slice.content.children().first()?;
    for _ in 1..slice.open_start {
        node = node.content().children().first()?;
    }
    (slice.open_start > 0 && node.is_textblock()).then_some(node)
}

/// Whether `node` or anything in it holds app data attributes.
fn holds_app_data(node: &Node) -> bool {
    node_app_data(node).next().is_some()
        || (!node.is_textblock() && node.content().children().iter().any(holds_app_data))
}

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
    /// [`slice_from_text`](crate::serialize::slice_from_text) builds it): as
    /// [`replace_range`](Self::replace_range), except that the lines are new
    /// text, so a block's app data attributes go to them as if Enter were
    /// pressed between them
    /// ([`DataAttrSplit`]). Returns the position
    /// right after the inserted content.
    pub fn replace_range_with_lines(
        &mut self,
        from: usize,
        to: usize,
        slice: Slice,
    ) -> Result<usize, StepError> {
        let ends = self.data_ends(from, to, Edit::Lines);
        let end = self.fit_range(from, to, slice)?;
        if let Some(ends) = ends {
            self.settle_data(ends, Edit::Lines)?;
        }
        Ok(end)
    }

    /// What [`settle_data`](Self::settle_data) needs to know of `from..to`
    /// before `edit` replaces it: `None` when no attribute can be at stake.
    pub(crate) fn data_ends(&self, from: usize, to: usize, edit: Edit<'_>) -> Option<Ends> {
        let r_from = self.doc.resolve(Pos(from)).ok()?;
        let r_to = self.doc.resolve(Pos(to)).ok()?;
        if r_from.depth() == 0 || r_to.depth() == 0 {
            return None;
        }
        let (from_block, to_block) = (r_from.parent(), r_to.parent());
        if !from_block.is_textblock() || !to_block.is_textblock() {
            return None;
        }
        let head = r_from.before(r_from.depth())?;
        let same = r_to.before(r_to.depth()) == Some(head);
        // A join inside one block changes no block's attributes.
        if same && matches!(edit, Edit::Join) {
            return None;
        }
        let pasted = match edit {
            Edit::Paste(slice) => slice.content.children().iter().any(holds_app_data),
            _ => false,
        };
        if !pasted && !holds_app_data(from_block) && !holds_app_data(to_block) {
            return None;
        }
        Some(Ends {
            head,
            same,
            before: r_from.parent_offset() > 0,
            after: r_to.parent_offset() < to_block.content().size(),
            from_end: self.doc.content_size().checked_sub(to)?,
            from_block: from_block.clone(),
            to_block: to_block.clone(),
        })
    }

    /// Set the app data attributes of the blocks `edit` made of the range
    /// `ends` was taken from, by the rules of
    /// [`crate::schema::data_attrs`].
    pub(crate) fn settle_data(&mut self, ends: Ends, edit: Edit<'_>) -> Result<(), StepError> {
        let head = self
            .doc
            .node_at(ends.head)
            .filter(Node::is_textblock)
            .map(|node| (ends.head, node));
        let tail = self
            .doc
            .content_size()
            .checked_sub(ends.from_end)
            .and_then(|at| {
                let r = self.doc.resolve(Pos(at)).ok()?;
                (r.depth() > 0 && r.parent().is_textblock())
                    .then(|| Some((r.before(r.depth())?, r.parent().clone())))?
            });
        let from_data = pairs(&ends.from_block);

        // One block holds both ends.
        if let (Some((h, block)), Some((t, _))) = (&head, &tail)
            && h == t
        {
            let want = if ends.before {
                // The first block's, which it still holds.
                return Ok(());
            } else if ends.after {
                // Nothing of the first block's text is left: the attributes
                // follow the text that is, where the block is still that text's
                // element. Where it is another (a heading left of a join with a
                // paragraph), a paste leaves the pasted block's and every other
                // edit none.
                match edit {
                    _ if same_element(&ends.to_block, block) => pairs(&ends.to_block),
                    Edit::Paste(slice) => open_textblock(slice).map(pairs).unwrap_or_default(),
                    _ => Vec::new(),
                }
            } else {
                match edit {
                    Edit::Paste(slice) => match open_textblock(slice).map(pairs) {
                        // A pasted block that brings its own arrives with them.
                        Some(own) if !own.is_empty() => own,
                        _ if same_element(&ends.from_block, block) => from_data,
                        _ => return Ok(()),
                    },
                    _ => from_data,
                }
            };
            return self.set_app_data(*h, &want);
        }
        // Two blocks that could not join keep their own.
        if matches!(edit, Edit::Join) {
            return Ok(());
        }
        // Which of the first block's attributes a block made of it holds.
        let after = ends.same && ends.after;
        let carried = |tf: &Self, place: Place| -> Pairs {
            from_data
                .iter()
                .filter(|(name, _)| tf.split_carries(name, place, ends.before, after))
                .cloned()
                .collect()
        };

        if let Some((pos, _)) = &head
            && !ends.before
        {
            match edit {
                Edit::Lines => {
                    let block = self.doc.node_at(*pos);
                    let want = match block {
                        Some(b) if same_element(&ends.from_block, &b) => {
                            carried(self, Place::First)
                        }
                        _ => Vec::new(),
                    };
                    self.set_app_data(*pos, &want)?;
                }
                // The block holds only pasted text: it has the pasted block's.
                Edit::Paste(slice) => {
                    if let Some(pasted) = open_textblock(slice) {
                        self.set_app_data(*pos, &pairs(pasted))?;
                    }
                }
                Edit::Join => {}
            }
        }

        if matches!(edit, Edit::Lines)
            && let (Some((h, first)), Some((t, _))) = (&head, &tail)
        {
            let mut middles = Vec::new();
            let (start, end) = (h + first.node_size(), *t);
            if start < end {
                self.doc.nodes_between(start, end, &mut |node, pos, _| {
                    if node.is_textblock()
                        && pos >= start
                        && pos < end
                        && same_element(&ends.from_block, node)
                    {
                        middles.push(pos);
                    }
                    true
                });
            }
            let want = carried(self, Place::Middle);
            for pos in middles {
                self.set_app_data(pos, &want)?;
            }
        }

        if let Some((pos, block)) = &tail {
            if ends.after {
                // The block the text after the range is in. Its attributes are
                // that text's, where it is still the same element.
                if same_element(&ends.to_block, block) {
                    let want = if ends.same {
                        carried(self, Place::Last)
                    } else {
                        pairs(&ends.to_block)
                    };
                    self.set_app_data(*pos, &want)?;
                }
            } else if matches!(edit, Edit::Lines) && same_element(&ends.from_block, block) {
                let want = carried(self, Place::Last);
                self.set_app_data(*pos, &want)?;
            }
        }
        Ok(())
    }

    /// Set the app data attributes of the two blocks a split made of
    /// `original`: `first` and `second` are the positions before them, and
    /// `before` / `after` whether each got any of its text. A second block that
    /// is another element (the paragraph after a heading) gets none.
    pub(crate) fn settle_split(
        &mut self,
        original: &Node,
        first: usize,
        second: usize,
        before: bool,
        after: bool,
    ) -> Result<(), StepError> {
        let data = pairs(original);
        if data.is_empty() {
            return Ok(());
        }
        for (pos, place) in [(first, Place::First), (second, Place::Last)] {
            let Some(block) = self.doc.node_at(pos) else {
                continue;
            };
            let want: Pairs = if same_element(original, &block) {
                data.iter()
                    .filter(|(name, _)| self.split_carries(name, place, before, after))
                    .cloned()
                    .collect()
            } else {
                Vec::new()
            };
            self.set_app_data(pos, &want)?;
        }
        Ok(())
    }

    /// Whether the block at `place`, of those made of one that had text
    /// `before` the cut and `after` it, holds that block's attribute `name`.
    fn split_carries(&self, name: &str, place: Place, before: bool, after: bool) -> bool {
        // The block that holds the original's text: the first, unless all of
        // it went to the last.
        let keeps = match place {
            Place::First => before || !after,
            Place::Last => after && !before,
            Place::Middle => false,
        };
        match self.data_attr_split(name) {
            DataAttrSplit::Copy => true,
            DataAttrSplit::CopyToText => match place {
                Place::First => before || !after,
                Place::Last => after,
                Place::Middle => before && after,
            },
            DataAttrSplit::Keep => keeps,
        }
    }

    /// Make the app data attributes of the block at `pos` exactly `want` (one
    /// that keeps none is left alone).
    fn set_app_data(&mut self, pos: usize, want: &[(String, String)]) -> Result<(), StepError> {
        let Some(node) = self.doc.node_at(pos) else {
            return Ok(());
        };
        let spec = node.node_type().spec();
        if !spec.data_attrs {
            return Ok(());
        }
        let have = pairs(&node);
        let mut changes: Vec<(String, Option<String>)> = have
            .iter()
            .filter(|(name, _)| !want.iter().any(|(n, _)| n == name))
            .map(|(name, _)| (name.clone(), None))
            .collect();
        changes.extend(
            want.iter()
                .filter(|(name, value)| {
                    !spec.attrs.contains_key(name.as_str())
                        && !have.iter().any(|(n, v)| n == name && v == value)
                })
                .map(|(name, value)| (name.clone(), Some(value.clone()))),
        );
        for (attr, value) in changes {
            self.step(Box::new(SetNodeAttrStep {
                pos,
                attr,
                value: value.map(Into::into),
            }))?;
        }
        Ok(())
    }
}
