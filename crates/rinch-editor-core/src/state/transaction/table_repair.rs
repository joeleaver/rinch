//! The table repair a selection gesture (and an undo or redo) runs: which
//! tables an edit can have left ragged, and padding them to a rectangle.

use super::*;
use crate::pos::inside_one_textblock;
use crate::transform::data_attrs::Spot;

impl Transaction {
    /// Run the selection gesture `f`, which replaces `from..to`, then repair
    /// every table the gesture left ragged that it did not find ragged
    /// (#1847): ProseMirror's `fixTables`, scoped to this edit.
    ///
    /// A range whose ends are in different tables, or with one end outside
    /// every table, has no cell selection to stand for it
    /// ([`Selection::normalized`] makes one only for two cells of one table),
    /// so the fitted replace can take cells out of a row or rows out from
    /// under a rowspan; and a pasted table may itself be ragged. The repair
    /// puts the grid back: a rowspan past the last row or a span into another
    /// cell's slot is cut, then each short row gets empty cells at its end
    /// ([`Self::repair_table`]).
    ///
    /// Which tables are looked at, so that an edit inside one cell costs
    /// nothing in the size of the table around it:
    ///
    /// - **a caret, or a range inside one textblock or inside the innermost
    ///   cell around its start, and no table inserted** (`inserts_tables`
    ///   false — typing, a delete, a paste of text): none, and nothing is
    ///   resolved but the range's start (`r_from`, the caller's own resolve
    ///   when it has one: `insert_text` shares it with the inherited marks);
    /// - **both ends in one cell** (the innermost cell around each is the
    ///   same), or both outside every table: the edit cannot change the grid
    ///   of any table around it, so only the tables inside the edited range
    ///   afterwards — pasted ones — are checked;
    /// - **ends in different cells**: also the tables around the two ends (and
    ///   one either side of the range, which a fitted replace may join), each
    ///   checked whole.
    ///
    /// A table counts as this edit's when it is new (pasted) or was not
    /// ragged before (its old self is found through the inverted mapping): a
    /// table that was already ragged, loaded from HTML and drawn as a browser
    /// draws it, stays as it is.
    pub(super) fn with_table_repair(
        &mut self,
        from: usize,
        to: usize,
        inserts_tables: bool,
        r_from: Option<ResolvedPos>,
        f: impl FnOnce(&mut Self, Spot<'_>) -> Result<(), StepError>,
    ) -> Result<(), StepError> {
        if from == to && !inserts_tables {
            return f(self, Spot::Unknown);
        }
        // Everything below is read off `from`'s resolved position (the
        // caller's, when it has one) without resolving `to`: a resolve walks
        // every row before the position in a table, so an edit inside one
        // cell would cost in the table's size.
        let Some(r) = r_from.or_else(|| self.doc.resolve(Pos(from)).ok()) else {
            return f(self, Spot::Unknown);
        };
        let cell_depth = (1..=r.depth()).rev().find(|&d| tables::is_cell(r.node(d)));
        // `to` in the same innermost cell: inside its end, and not inside a
        // cell of a table nested in it (#1930: since the fitted delete takes
        // a range from a cell's text into a nested table's cell, that range
        // can take cells out of the nested table's rows). Resolved inside
        // the cell only, so the cost is the cell's content, not the table's.
        // A range inside the textblock is inside the cell (no resolve).
        let in_block = r.parent().is_textblock() && to <= r.end(r.depth());
        // `to` resolved inside that cell, when it is in it and in no cell
        // below it: not needed for a range inside the textblock.
        let inner = cell_depth
            .filter(|&d| !in_block && to <= r.end(d))
            .and_then(|d| r.node(d).resolve(Pos(to - r.start(d))).ok())
            .filter(|inner| !(1..=inner.depth()).any(|k| tables::is_cell(inner.node(k))));
        let in_from_cell = cell_depth.is_some() && (in_block || inner.is_some());
        // What the edit's data attribute check would otherwise resolve again.
        let spot = match &inner {
            _ if in_block => Spot::OneBlock,
            Some(inner) => Spot::Ends {
                from: &r,
                to: inner,
            },
            None => Spot::Unknown,
        };
        // Nothing to check when the edit takes nothing out of a table's grid
        // and puts no table in: a range inside one textblock or one cell (a
        // nested table wholly inside the range goes whole; one an end is in
        // makes that end's innermost cell another), with no table inserted.
        // This is every keystroke over a range.
        if !inserts_tables && (in_block || in_from_cell) {
            return f(self, spot);
        }
        let from_cell = cell_depth.and_then(|d| r.before(d));
        let to_cell = if in_from_cell {
            from_cell
        } else {
            self.doc
                .resolve(Pos(to))
                .ok()
                .and_then(|r| tables::cell_around(&r))
        };
        let confined = from_cell == to_cell;
        let old_doc = self.doc.clone();
        let first = self.mapping.maps().len();
        f(self, spot)?;
        let total = self.mapping.maps().len();
        if total == first {
            return Ok(());
        }
        // The steps' own maps, read in place: slicing the mapping would clone
        // them on every keystroke.
        let maps = &self.mapping.maps()[first..total];
        let through = |maps: &[StepMap], pos: usize, assoc: i32| {
            maps.iter().fold(pos, |p, m| m.map(p, assoc))
        };
        let end = self.doc.content_size();
        let (lo, hi) = if confined {
            (from, to)
        } else {
            (from.saturating_sub(1), (to + 1).min(old_doc.content_size()))
        };
        let (mut lo, mut hi) = (through(maps, lo, -1), through(maps, hi, 1));
        // Everything a step put in, wherever the fit placed it (a pasted
        // table may go in beside the textblock the caret was in).
        for (k, m) in maps.iter().enumerate() {
            for t in m.ranges().as_chunks::<3>().0 {
                lo = lo.min(through(&maps[k + 1..], m.map(t[0], -1), -1));
                hi = hi.max(through(&maps[k + 1..], m.map(t[0] + t[1], 1), 1));
            }
        }
        let (lo, hi) = (lo.min(end), hi.min(end));
        // In a confined edit, the cell's own position: the tables around it
        // are not this edit's to check.
        let floor = from_cell.map(|c| through(maps, c, -1));
        let mut ragged = Vec::new();
        self.doc.nodes_between(lo, hi, &mut |node, pos, _| {
            let around = confined && floor.is_some_and(|c| pos <= c);
            if tables::is_table(node) && !around && !tables::table_problems(node).is_empty() {
                ragged.push(pos);
            }
            !node.is_textblock()
        });
        if ragged.is_empty() {
            return Ok(());
        }
        let mut back = Mapping::new();
        for m in maps.iter().rev() {
            back.append_map(m.invert());
        }
        let was_ragged = |pos: usize| {
            // Inside the table, so a table pasted right at an old one is not
            // taken for it.
            let r = back.map_result(pos + 1, 1);
            !r.deleted()
                && r.pos
                    .checked_sub(1)
                    .and_then(|p| old_doc.node_at(p))
                    .filter(tables::is_table)
                    .is_some_and(|t| !tables::table_problems(&t).is_empty())
        };
        ragged.retain(|&p| !was_ragged(p));
        // Innermost (later) tables first: a repair only adds or changes
        // things inside its own table, so the positions before it hold.
        for pos in ragged.into_iter().rev() {
            self.repair_table(pos)?;
        }
        Ok(())
    }

    /// After an undo or redo (`plugins::history`): repair each table its steps
    /// left ragged, in this transaction (#1937). The steps an undo reverts are
    /// mapped through the changes made since — a peer's, which the history did
    /// not record — and a mapped table command can apply in part: a
    /// `mergeCells` undone after a peer's merge restored cells into the peer's
    /// table and kept the spans. #1847 decided to repair, not skip, what an edit
    /// leaves ragged; this is that repair for the history's transactions, which
    /// are built step by step rather than through [`Self::with_table_repair`].
    ///
    /// Only the tables a step can have changed the grid of are looked at, so
    /// that undoing a keystroke costs nothing in the size of the table around
    /// it: an inline insertion changes no grid and is not looked up; a range
    /// inside one textblock that puts in no table, row or cell (the undo of
    /// typing, while it still is) is found so by one walk down the document it
    /// applied to, without a resolve; any other step inside one cell that puts
    /// in no table part changes no grid (one resolve of its start); a step whose
    /// ends
    /// are in different cells, a `BatchStep` and a span attribute set on a cell
    /// look at the tables around them; a step that puts in a table, row or cell
    /// inside one cell looks at the tables inside its range. A table that was
    /// ragged before the undo (its old self found through the inverted
    /// mapping) stays as it is.
    pub(crate) fn repair_tables_after_history(&mut self) -> Result<(), StepError> {
        use crate::transform::steps::{BatchStep, ReplaceAroundStep, ReplaceStep, SetNodeAttrStep};
        let n = self.steps.len();
        if n == 0 {
            return Ok(());
        }
        let full = full_repair_scan();
        let maps = self.mapping.maps();
        // A position of step `k`'s document, in the document now.
        let through =
            |k: usize, pos: usize, assoc: i32| maps[k..].iter().fold(pos, |p, m| m.map(p, assoc));
        // Ranges of the document now to look for tables in, each with the cell
        // around it whose own tables are not this step's to check.
        let mut scans: Vec<(usize, usize, Option<usize>)> = Vec::new();
        if full {
            scans.push((0, self.doc.content_size(), None));
        }
        for (k, (step, doc)) in self.steps.iter().zip(&self.docs).enumerate() {
            if full {
                break;
            }
            // Inline content put in at one position, which only a textblock
            // takes: no grid changes, and nothing is looked up.
            if inline_insertion(step.as_ref()) {
                continue;
            }
            let step = step.as_any();
            let size = doc.content_size();
            let around = |scans: &mut Vec<(usize, usize, Option<usize>)>, f: usize, t: usize| {
                scans.push((
                    through(k, f.saturating_sub(1), -1),
                    through(k, (t + 1).min(size), 1),
                    None,
                ));
            };
            let (from, to, slice) = if let Some(s) = step.downcast_ref::<ReplaceStep>() {
                (s.from, s.to, &s.slice)
            } else if let Some(s) = step.downcast_ref::<ReplaceAroundStep>() {
                (s.from, s.to, &s.slice)
            } else if let Some(s) = step.downcast_ref::<SetNodeAttrStep>() {
                if matches!(s.attr.as_str(), "colspan" | "rowspan") {
                    around(&mut scans, s.pos, s.pos + 1);
                }
                continue;
            } else if let Some(s) = step.downcast_ref::<BatchStep>() {
                let map = &maps[k];
                for t in map.ranges().as_chunks::<3>().0 {
                    around(&mut scans, t[0], t[0] + t[1]);
                }
                for pos in s.attr_positions() {
                    around(&mut scans, pos, pos + 1);
                }
                continue;
            } else {
                // Marks and document attributes change no grid.
                continue;
            };
            let structural = holds_table_parts(&slice.content);
            // A range inside one textblock that puts in no table part: the undo
            // of typing, wherever it was mapped, when it still is (review of
            // #1950: a peer who moved half of my pasted text into a new cell
            // made its undo a delete across two cells). One walk down the
            // document, no resolve.
            if !structural && inside_one_textblock(doc, from, to) {
                continue;
            }
            let Ok(r) = doc.resolve(Pos(from)) else {
                around(&mut scans, from, to);
                continue;
            };
            let cell = (1..=r.depth()).rev().find(|&d| tables::is_cell(r.node(d)));
            // A slice open at either end puts its open nodes' closing and opening
            // tokens in at its ends: with a cell or a row among them it splits the
            // table around it (review of #1950: the inverse of a delete from one
            // row's cell into the next row's is such a slice, put in at one point
            // inside a cell), so the tables around it are looked at.
            let open = slice.open_start > 0 || slice.open_end > 0;
            match cell {
                Some(d) if to <= r.end(d) && !(structural && open) => {
                    if structural {
                        let floor = r.before(d).map(|c| through(k, c, -1));
                        scans.push((through(k, from, -1), through(k, to, 1), floor));
                    }
                }
                _ => around(&mut scans, from, to),
            }
        }
        if scans.is_empty() {
            return Ok(());
        }
        let end = self.doc.content_size();
        let mut ragged: Vec<usize> = Vec::new();
        for (lo, hi, floor) in scans {
            let (lo, hi) = (lo.min(end), hi.min(end));
            self.doc.nodes_between(lo, hi, &mut |node, pos, _| {
                let skip = floor.is_some_and(|c| pos <= c);
                if tables::is_table(node)
                    && !skip
                    && !ragged.contains(&pos)
                    && !tables::table_problems(node).is_empty()
                {
                    ragged.push(pos);
                }
                !node.is_textblock()
            });
        }
        if ragged.is_empty() {
            return Ok(());
        }
        let old_doc = &self.docs[0];
        let mut back = Mapping::new();
        for m in maps.iter().rev() {
            back.append_map(m.invert());
        }
        let was_ragged = |pos: usize| {
            let r = back.map_result(pos + 1, 1);
            !r.deleted()
                && r.pos
                    .checked_sub(1)
                    .and_then(|p| old_doc.node_at(p))
                    .filter(tables::is_table)
                    .is_some_and(|t| !tables::table_problems(&t).is_empty())
        };
        ragged.retain(|&p| !was_ragged(p));
        // Innermost (later) tables first, so the positions before each hold.
        ragged.sort_unstable();
        for pos in ragged.into_iter().rev() {
            self.repair_table(pos)?;
        }
        Ok(())
    }

    /// Make the table at `table_pos` tile its grid ([`tables::table_problems`]):
    /// cut each colliding or overlong span to what fits, then give each short
    /// row empty cells at its end, until the table tiles. Port of
    /// prosemirror-tables' `fixTable`, which cuts a colliding cell's colspan
    /// (a collision below a cell's first row cuts its rowspan here) and pads
    /// a cell per missing column — which this does too, up to the HTML
    /// reader's padding bound; past it a short row gets one cell per run of
    /// missing slots, spanning the run, so a wide span cannot make the
    /// padding explode.
    fn repair_table(&mut self, table_pos: usize) -> Result<(), StepError> {
        // Each round cuts every colliding or overlong span it finds (so the
        // spans' sum falls), or pads; a cut can move the cells after it and
        // uncover or collide again, so it runs until the table tiles. The
        // bound — twice the cells, plus the pad — is only a backstop. A fixed
        // three rounds left one random pasted table in 3,000 ragged (the
        // second review of #1931; `table_paste_repair_1847.rs`).
        let cells = |t: &Node| {
            (0..t.child_count())
                .map(|r| t.child(r).child_count())
                .sum::<usize>()
        };
        let rounds = self
            .doc
            .node_at(table_pos)
            .filter(tables::is_table)
            .map_or(0, |t| 2 * cells(&t) + 3);
        for _ in 0..rounds {
            let Some(table) = self.doc.node_at(table_pos).filter(tables::is_table) else {
                return Ok(());
            };
            let problems = tables::table_problems(&table);
            if problems.is_empty() {
                return Ok(());
            }
            let start = table_pos + 1;
            let mut cut = false;
            for problem in &problems {
                let (pos, attr, value) = match problem {
                    TableProblem::OverlongRowspan { pos, rows_fit } => (*pos, "rowspan", *rows_fit),
                    TableProblem::Collision { pos, rows_fit, .. } if *rows_fit > 0 => {
                        (*pos, "rowspan", *rows_fit)
                    }
                    TableProblem::Collision { pos, cols_fit, .. } if *cols_fit > 0 => {
                        (*pos, "colspan", *cols_fit)
                    }
                    _ => continue,
                };
                self.set_node_attr(start + pos, attr, AttrValue::Int(value as i64))?;
                cut = true;
            }
            if cut {
                continue;
            }
            // Pad the short rows, the last first so the earlier rows'
            // positions hold: a cell per missing column, as prosemirror-tables
            // pads, while that is at most the HTML reader's padding bound (8
            // empty cells per cell in the table, 16 whatever); past it one
            // cell per run of missing slots, spanning the run — a pasted
            // `<td colspan=1000>` over 65 bare rows asks for 64,935 cells.
            let missing: usize = problems
                .iter()
                .map(|p| match p {
                    TableProblem::Missing { runs, .. } => runs.iter().sum(),
                    _ => 0,
                })
                .sum();
            let per_column = missing <= 8 * cells(&table) + 16;
            let mut row_ends = Vec::with_capacity(table.child_count());
            let mut pos = start;
            for r in 0..table.child_count() {
                pos += table.child(r).node_size();
                row_ends.push(pos - 1);
            }
            for problem in problems.iter().rev() {
                let TableProblem::Missing { row, runs } = problem else {
                    continue;
                };
                let header = table
                    .child(*row)
                    .content()
                    .children()
                    .first()
                    .is_some_and(|c| c.type_name() == "table_header_cell");
                let widths: Vec<usize> = if per_column {
                    runs.iter()
                        .flat_map(|&n| std::iter::repeat_n(1, n))
                        .collect()
                } else {
                    runs.clone()
                };
                let pad = widths
                    .into_iter()
                    .map(|n| self.empty_cell(header, n))
                    .collect::<Result<Vec<_>, _>>()?;
                let at = row_ends[*row];
                self.replace_with(at, at, Fragment::from_children(pad))?;
            }
        }
        Ok(())
    }

    /// An empty cell (one empty paragraph) `colspan` columns wide, a header
    /// cell when `header`.
    fn empty_cell(&self, header: bool, colspan: usize) -> Result<Node, StepError> {
        let err = |e: crate::error::EditorError| StepError::new(e.to_string());
        let para = self
            .schema
            .create_node("paragraph", Attrs::new(), Fragment::empty())
            .map_err(err)?;
        let name = if header {
            "table_header_cell"
        } else {
            "table_cell"
        };
        let attrs = if colspan > 1 {
            Attrs::from_iter([("colspan", AttrValue::Int(colspan as i64))])
        } else {
            Attrs::new()
        };
        self.schema
            .create_node(name, attrs, Fragment::from_node(para))
            .map_err(err)
    }
}

/// Whether `node` is or holds a table: what a slice or node inserted by a
/// selection gesture must be checked for (a pasted table may be ragged).
pub(super) fn holds_table(node: &Node) -> bool {
    if tables::is_table(node) {
        return true;
    }
    !node.is_textblock() && node.content().children().iter().any(holds_table)
}

thread_local! {
    /// How the history's repair looks for tables an undo or redo left ragged
    /// ([`__set_full_repair_scan`]): `0` where its steps can have changed a grid,
    /// `1` at every table, `2` at every table after every undo and redo, local
    /// ones included.
    static FULL_REPAIR_SCAN: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

/// Whether the repair looks at every table ([`__set_full_repair_scan`]).
pub(crate) fn full_repair_scan() -> bool {
    FULL_REPAIR_SCAN.with(|f| f.get()) >= 1
}

/// Whether every undo and redo runs the repair, local ones included.
pub(crate) fn repair_every_undo() -> bool {
    FULL_REPAIR_SCAN.with(|f| f.get()) >= 2
}

/// **Test seam** (#1937): on this thread, make the history's repair look at
/// every table of the document for one an undo or redo left ragged (`1`), and
/// also after a local undo or redo, which needs none (`2`), instead of where its
/// steps can have changed a grid (`0`, the default). A differential test runs one
/// history both ways and requires the same documents.
#[doc(hidden)]
pub fn __set_full_repair_scan(level: u8) {
    FULL_REPAIR_SCAN.with(|f| f.set(level));
}

/// Whether `content` holds a table, a row or a cell anywhere: what a replace must
/// put in to change a table's grid from inside one cell.
fn holds_table_parts(content: &Fragment) -> bool {
    content.children().iter().any(|n| {
        tables::is_table(n)
            || tables::is_cell(n)
            || n.type_name() == "table_row"
            || (!n.is_textblock() && !n.is_leaf() && holds_table_parts(n.content()))
    })
}

/// Whether `step` puts in inline content alone at one position (typing, an
/// inline paste): a `ReplaceStep` that replaces nothing with a closed slice of
/// inline nodes. Only a textblock takes inline content, so it changes no table's
/// grid, and its inverse deletes inline content of one textblock (#1937).
pub(crate) fn inline_insertion(step: &dyn Step) -> bool {
    step.as_any()
        .downcast_ref::<crate::transform::steps::ReplaceStep>()
        .is_some_and(|s| {
            s.from == s.to
                && s.slice.open_start == 0
                && s.slice.open_end == 0
                && s.slice.content.children().iter().all(Node::is_inline)
        })
}
