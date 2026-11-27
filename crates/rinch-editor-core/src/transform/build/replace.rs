//! The replace family: strict and fitted replacement, deletion, text.

use super::*;

impl Transform<'_> {
    // -- replace family --------------------------------------------------

    /// Replace `from..to` with `slice`.
    pub fn replace(
        &mut self,
        from: usize,
        to: usize,
        slice: Slice,
    ) -> Result<&mut Self, StepError> {
        self.step(Box::new(ReplaceStep::new(from, to, slice)))
    }

    /// Replace `from..to` with `slice`, **fitted** to the range: the paste
    /// primitive. Where [`replace`](Self::replace) refuses a slice whose open
    /// depths or content do not line up with the range, this closes or splits
    /// the nodes around the range as the content needs
    /// (ProseMirror's `replaceRange`; the rules are in
    /// [`transform::fit`](crate::transform)'s module doc):
    ///
    /// - content open at the slice's start continues the textblock the range
    ///   starts in, and the inline content after the range joins the
    ///   textblock the slice ends in;
    /// - code pasted in a textblock that is not code, over anything but
    ///   exactly that textblock's whole content, is a textblock for each of
    ///   its lines, not one holding line ends and not a code block that
    ///   takes the line's own text;
    /// - a block that is closed splits the textblock (leaving no empty block
    ///   when the range is at the textblock's edge);
    /// - over a whole textblock — a caret in an empty one included — a slice
    ///   that starts in a [defining](crate::schema::NodeSpec::defining) node
    ///   (a list item, a heading, a quote) replaces the textblock, keeping
    ///   that node.
    ///
    /// The fit is one step, followed by one for each app data attribute a
    /// block changes ([the rules](crate::schema::data_attrs); the attributes
    /// never change the fit). Returns the position right after the inserted
    /// content, in the new document. Errors, adding nothing, when no fit is
    /// valid, or when `slice` holds a node that is not valid content (the
    /// nodes open at its edges aside).
    ///
    /// A range whose ends are in different
    /// [isolating](crate::schema::NodeSpec::isolating) nodes (from text in a
    /// table cell to text outside its table) is fitted like any other, as
    /// ProseMirror fits it (#1930): the fit can take cells out of a row, which
    /// the selection gestures repair (`Transaction::with_table_repair`) and a
    /// transaction an app builds from positions does not.
    pub fn replace_range(
        &mut self,
        from: usize,
        to: usize,
        slice: Slice,
    ) -> Result<usize, StepError> {
        if slice.size() == 0 {
            self.delete(from, to)?;
            return Ok(from);
        }
        let pasted = slice.clone();
        let ends = self.data_ends(from, to, Edit::Paste(&pasted));
        let end = self.fit_range(from, to, slice)?;
        if let Some(ends) = ends {
            self.settle_data(ends, Edit::Paste(&pasted))?;
        }
        Ok(end)
    }

    /// The fit of [`replace_range`](Self::replace_range), data attributes
    /// left where the step put them.
    pub(crate) fn fit_range(
        &mut self,
        from: usize,
        to: usize,
        slice: Slice,
    ) -> Result<usize, StepError> {
        if slice.size() == 0 {
            self.fit_delete(from, to)?;
            return Ok(from);
        }
        if !fit::slice_is_sound(&slice) {
            return Err(StepError::new("the slice holds an invalid node"));
        }
        let slice =
            fit::code_lines_as_blocks(self.schema, &self.doc, from, to, &slice).unwrap_or(slice);
        for (a, b, candidate) in fit::range_candidates(&self.doc, from, to, &slice) {
            if let Some(found) = fit::fit_step(&self.doc, a, b, &candidate)
                && self.step(found.step).is_ok()
            {
                return Ok(found.end);
            }
        }
        let end = from + slice.size();
        self.step(Box::new(ReplaceStep::new(from, to, slice)))?;
        Ok(end)
    }

    /// Replace `from..to` with the (flat) `content`.
    pub fn replace_with(
        &mut self,
        from: usize,
        to: usize,
        content: Fragment,
    ) -> Result<&mut Self, StepError> {
        self.replace(from, to, Slice::from_fragment(content))
    }

    /// Delete the range `from..to`.
    ///
    /// A range whose ends are at the same depth is one plain
    /// [`replace`](Self::replace). One whose ends are not (from a list item
    /// into the item nested under it, from a paragraph into a list) has no
    /// plain replacement, so it is fitted as
    /// [`replace_range`](Self::replace_range) fits a slice: what is left of
    /// the node the range ends in joins the node it starts in, and what
    /// followed it stays where it can (ProseMirror's `Transform.delete`).
    /// Like `replace_range`, a range whose ends are in different isolating
    /// nodes (from text in a table cell to text outside its table, #1930) is
    /// fitted too, as ProseMirror fits it; one that holds only the tokens
    /// between two textblocks that cannot join (code, then marked text),
    /// which would delete nothing, is refused. One step is added, or none
    /// and the plain replacement's error. Blocks it joins then take their
    /// app data attributes by [the rules](crate::schema::data_attrs).
    pub fn delete(&mut self, from: usize, to: usize) -> Result<&mut Self, StepError> {
        let ends = self.data_ends(from, to, Edit::Join);
        self.fit_delete(from, to)?;
        if let Some(ends) = ends {
            self.settle_data(ends, Edit::Join)?;
        }
        Ok(self)
    }

    /// The step of [`delete`](Self::delete).
    fn fit_delete(&mut self, from: usize, to: usize) -> Result<&mut Self, StepError> {
        let Some(plain) = self
            .step(Box::new(ReplaceStep::new(from, to, Slice::empty())))
            .err()
        else {
            return Ok(self);
        };
        // A fit that only closes and reopens the nodes between the two ends
        // deletes nothing (the document is the same size): that is a refusal,
        // not an empty step in the history.
        if let Some(found) = fit::fit_step(&self.doc, from, to, &Slice::empty())
            && found
                .step
                .apply(&self.doc)
                .is_ok_and(|doc| doc.content_size() < self.doc.content_size())
            && self.step(found.step).is_ok()
        {
            return Ok(self);
        }
        Err(plain)
    }

    /// Insert `text` replacing `from..to` (collapsed when `from == to`).
    pub fn insert_text(
        &mut self,
        from: usize,
        to: usize,
        text: &str,
    ) -> Result<&mut Self, StepError> {
        let node = self
            .schema
            .text(text)
            .map_err(|e| StepError::new(e.to_string()))?;
        self.replace_inline(from, to, Fragment::from_node(node))
    }

    /// Replace `from..to` with inline `content` typed there: a
    /// [`replace`](Self::replace) after which blocks it joined take their app
    /// data attributes by [the rules](crate::schema::data_attrs).
    pub(crate) fn replace_inline(
        &mut self,
        from: usize,
        to: usize,
        content: Fragment,
    ) -> Result<&mut Self, StepError> {
        let ends = self.data_ends(from, to, Edit::Join);
        self.replace_with(from, to, content)?;
        if let Some(ends) = ends {
            self.settle_data(ends, Edit::Join)?;
        }
        Ok(self)
    }
}
