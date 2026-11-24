//! Keyboard motion at a table cell's edge: `shiftArrow` and vertical entry
//! into a table.

use super::*;

/// Shift+Arrow at a table cell's edge: prosemirror-tables' `shiftArrow`, a
/// keyboard handler in front of [`Selection::normalized`].
///
/// `new_head` is where the plain extension (the text motion, or the vertical
/// step) would put the head; `None` when it goes nowhere. Returns the
/// selection to land, or `None` to let the plain extension stand:
///
/// - over a **cell selection**, the head cell steps to its neighbour along
///   `axis` (`dir` `-1`/`+1`), the anchor cell stays; at the table's edge the
///   selection stays as it is (the key is still consumed);
/// - over a **text selection** whose ends are both in one cell (or a caret
///   there), whose head's textblock is the cell's last (first) block at
///   every level (PM `atEndOfCell`), when the plain extension would take the
///   head out of that cell (landing in a nested table inside it is staying),
///   or (horizontally) the head is at its textblock's edge: the cell
///   selection from that cell to its neighbour. Out of a nested table into
///   another cell of a table around it, that cell alone. Past a row's end, where
///   prosemirror-tables lets the browser extend into the next row (and its
///   normalization then collapses that boundary-only range), the neighbour
///   is the cell the plain motion reached. Out of the top or bottom row, a
///   step that stayed in the table (a platform's fallback to the next
///   textblock in document order) becomes a text extension to the first text
///   past the table's edge.
///
/// Anything else (a head that stays in its cell or is in another block of
/// it, a range already reaching outside the cell, a head in no table, an
/// extension out of the table, a motion that goes nowhere at the document's
/// edge) is `None`.
pub fn shift_arrow(
    state: &EditorState,
    axis: tables::Axis,
    dir: i32,
    new_head: Option<Pos>,
) -> Option<Selection> {
    let doc = &state.doc;
    match &state.selection {
        Selection::Cell(c) => {
            let (map, _, _) = tables::map_around(doc, Pos(c.head_cell.0 + 1))?;
            let head = map
                .next_cell(c.head_cell.0, axis, dir)
                .unwrap_or(c.head_cell.0);
            Some(Selection::cell(c.anchor_cell, Pos(head)))
        }
        Selection::Text(t) => {
            let cell = tables::cell_at_pos(doc, t.head)?;
            if tables::cell_at_pos(doc, t.anchor) != Some(cell) {
                return None;
            }
            // PM `atEndOfCell`: the head's textblock is the cell's last
            // (first) block at every level, so a paragraph above a nested
            // table, or above another paragraph, extends as text.
            if !in_edge_block(doc, t.head, dir) {
                return None;
            }
            // Landing anywhere inside the head's cell — a nested table in it
            // included — is staying put.
            let end = cell + doc.node_at(cell)?.node_size();
            let inside = |p: Pos| p.0 > cell && p.0 < end;
            // A word motion stops at its textblock's end rather than leave
            // it, so the head being at that edge counts too; vertically only
            // the platform's step knows (PM asks `endOfTextblock`).
            let at_edge = axis == tables::Axis::Horiz && at_block_edge(doc, t.head, dir);
            if new_head.is_some_and(inside) && !at_edge {
                return None;
            }
            let landed = new_head
                .filter(|&h| !inside(h))
                .and_then(|h| tables::cell_at_pos(doc, h));
            let (map, table, start) = tables::map_around(doc, Pos(cell + 1))?;
            // At the document's start or end the motion goes nowhere: no
            // other cell, so nothing to select (not the head's own cell).
            let in_table = landed.filter(|&l| tables::same_table(doc, Pos(l + 1), Pos(cell + 1)));
            if let Some(next) = map.next_cell(cell, axis, dir) {
                return Some(Selection::cell(Pos(cell), Pos(next)));
            }
            // Out of a nested table into another cell of a table around it
            // (not the cell that holds this table): the plain extension
            // would land as a cell selection of the outer cells, losing this
            // cell as its anchor. prosemirror-tables' normalization turns that
            // range into the head's textblock; here the head's cell is
            // selected, and the anchor stays in it.
            let holds = |at: usize, node: Option<Node>| {
                node.is_some_and(|n| at < cell && cell < at + n.node_size())
            };
            let sibling = landed.filter(|&l| {
                let outer = doc
                    .resolve(Pos(l + 1))
                    .ok()
                    .and_then(|r| tables::table_around(&r))
                    .map(|(t, start)| (start - 1, t));
                in_table.is_none()
                    && !holds(l, doc.node_at(l))
                    && outer.is_some_and(|(at, t)| holds(at, Some(t)))
            });
            if sibling.is_some() && axis == tables::Axis::Horiz {
                return Some(Selection::cell(Pos(cell), Pos(cell)));
            }
            match axis {
                // Past a row's end, the cell the plain motion reached when it
                // is in this table; out of the table, the plain extension.
                tables::Axis::Horiz => in_table.map(|next| Selection::cell(Pos(cell), Pos(next))),
                // Out of the top or bottom row. A step that stayed in the
                // table got there by document order (the platform's fallback
                // to the next textblock), not by going up or down: extend to
                // the text past the table's edge instead.
                tables::Axis::Vert => {
                    in_table?;
                    let edge = if dir > 0 {
                        start - 1 + table.node_size()
                    } else {
                        start - 1
                    };
                    let head = Selection::near_text(doc, Pos(edge), dir)?.head();
                    Some(Selection::text(t.anchor, head))
                }
            }
        }
        Selection::Node(_) => None,
    }
}

/// Where an Up or Down step from `head` into a table lands, when the
/// platform's geometry found no line there and it fell back to the next
/// textblock in document order (`target`): that is the table's last cell
/// going up (its first going down), whatever column the caret was in. This
/// picks the cell of the table's edge row (its last row going up, first
/// going down) under the goal column instead, as a browser does: the
/// rightmost cell whose text starts at or left of `goal_x`, by `x_of` (a
/// position's caret x, in the platform's coordinates). Its last text going
/// up, its first going down. `None` when `target` is in no table that
/// `head` is outside of, or no cell can be placed (#1847, review of #1931:
/// Shift+Down out of the bottom row and back up made a cell selection of
/// the last cell, five cells under a rowspan).
pub fn vertical_entry(
    doc: &Node,
    head: Pos,
    target: Pos,
    down: bool,
    goal_x: f32,
    x_of: impl Fn(Pos) -> Option<f32>,
) -> Option<Pos> {
    let r = doc.resolve(target).ok()?;
    // The outermost table around `target` that does not hold `head`.
    let depth = (1..=r.depth()).find(|&d| {
        tables::is_table(r.node(d))
            && !(r.before(d).is_some_and(|b| b < head.0) && r.after(d).is_some_and(|a| a > head.0))
    })?;
    let (table, start) = (r.node(depth).clone(), r.start(depth));
    let map = tables::table_map(&table, start);
    let row = if down {
        0
    } else {
        map.height().checked_sub(1)?
    };
    let mut cells: Vec<usize> = Vec::new();
    for col in 0..map.width() {
        if let Some(c) = map.cell_at(row, col)
            && cells.last() != Some(&c)
            && !cells.contains(&c)
        {
            cells.push(c);
        }
    }
    let first_text = |c: usize| Selection::near_text(doc, Pos(c + 1), 1).map(|s| s.head());
    let mut pick = *cells.first()?;
    for &c in &cells {
        if let Some(x) = first_text(c).and_then(&x_of)
            && x <= goal_x + 0.5
        {
            pick = c;
        }
    }
    let end = pick + doc.node_at(pick)?.node_size();
    let at = if down {
        Selection::near_text(doc, Pos(pick + 1), 1)?.head()
    } else {
        Selection::near_text(doc, Pos(end - 1), -1)?.head()
    };
    (at.0 > pick && at.0 < end).then_some(at)
}

/// Whether `pos` is at its textblock's end (`dir > 0`) or start.
fn at_block_edge(doc: &Node, pos: Pos, dir: i32) -> bool {
    doc.resolve(pos).is_ok_and(|r| {
        if dir > 0 {
            r.parent_offset() == r.parent().content_size()
        } else {
            r.parent_offset() == 0
        }
    })
}

/// Whether the textblock at `pos` is its innermost cell's last (`dir > 0`)
/// or first block, at every level up to the cell: the structural half of PM
/// `atEndOfCell`.
fn in_edge_block(doc: &Node, pos: Pos, dir: i32) -> bool {
    let Ok(r) = doc.resolve(pos) else {
        return false;
    };
    for d in (0..r.depth()).rev() {
        if tables::is_cell(r.node(d + 1)) {
            return true;
        }
        let at = if dir > 0 {
            r.index(d) + 1 == r.node(d).child_count()
        } else {
            r.index(d) == 0
        };
        if !at {
            return false;
        }
    }
    false
}
