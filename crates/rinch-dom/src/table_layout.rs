//! CSS table layout, on Taffy's grid.
//!
//! Taffy has no table formatting context, and the box a table needs is close
//! to the one a grid gives: tracks sized from their cells, spanning items,
//! rows as tall as their tallest cell. So a table box **is** a Taffy grid.
//! Row groups and rows compute [`DisplayValue::Contents`] and generate no box,
//! which puts every cell in the table's own Taffy child list, and
//! [`RinchDocument::resolve_tables`] does what is specifically a table's:
//!
//! - **Slots.** The HTML "forming a table" algorithm assigns each cell its
//!   row and column, honouring `rowspan`, `colspan`, and a `thead`/`tfoot` in
//!   any source position. A cell is placed on explicit grid lines.
//! - **Columns.** A cell's `width` is a statement about its column, not a
//!   size for the cell, so it is read here and never reaches Taffy. See
//!   *Column widths*.
//! - **Spacing.** `border-spacing` is the grid's gap, and the same amount of
//!   padding on the table, which is where the outer spacing lives.
//! - **Row backgrounds.** A row has no box to paint. css-tables-3 paints a
//!   row's background per cell anyway, which is why the spacing between two
//!   cells of a coloured row shows the table: [`TableCell::under`] carries the
//!   colours to the cell's paint. (An image on a row is not painted.)
//! - **Collapsed borders.** Each shared edge has one winner (CSS 2.1 §17.6.2.1)
//!   and it is drawn by the cell to its left or above it, which writes it into
//!   its `computed_style`: every painter, hit test and Taffy sync reads those.
//!   The base values are kept in [`TableCell::base`] and restored by a
//!   restyle. So a collapsed edge sits whole on one side of the grid line,
//!   where a browser centres it: up to half its width of difference.
//!
//! ## Column widths
//!
//! A column with no declared width is `minmax(min-content, <max-content>fr)`
//! when the table has a width of its own, which spreads the room as the
//! automatic table layout does: never under a cell's minimum, and in
//! proportion to what each column would like. Its max-content is measured by
//! [`RinchDocument::measure_table_columns`], laying the table out detached
//! with `auto` tracks. A table with no width of its own shrinks to fit on
//! plain `auto` tracks and needs no measure.
//!
//! ## Vertical alignment
//!
//! Taffy's block layout has no `align-content`, and a table cell is a block
//! container, so `vertical-align` is applied after the compute: the cell's
//! leftover height becomes extra top padding on the computed style (what
//! positions its text) and an offset on its children. Blink calls the same
//! thing intrinsic padding. It reaches no Taffy style, so it cannot feed back
//! into the row's height. `baseline` aligns as `top`.

use crate::RinchDocument;
use crate::computed_style::values::{
    BorderStyleValue, DimensionValue, DisplayValue, LengthPercentageValue, PositionValue,
    TablePart, VerticalAlignValue,
};
use crate::node::{NodeTree, RawNodeId};
use peniko::Color;
use taffy::style_helpers::{fr, length, line, min_content, minmax, percent, span};

/// HTML's limits on the span attributes.
const MAX_COLSPAN: usize = 1000;
const MAX_ROWSPAN: usize = 65534;

/// One side of a box's border, as the cascade computed it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BorderSide {
    pub width: LengthPercentageValue,
    pub style: BorderStyleValue,
    pub color: Option<Color>,
}

/// A box's four border sides: top, right, bottom, left.
pub type BorderSides = [BorderSide; 4];

/// A cell's place in its table, on [`crate::node::Node::table_cell`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TableCell {
    /// Grid lines, 1-based.
    pub row: i16,
    pub col: i16,
    pub rowspan: u16,
    pub colspan: u16,
    /// Solid backgrounds painted under this cell: its row group's, then its
    /// row's.
    pub under: [Option<Color>; 2],
    /// The borders the cascade gave it, while a collapsing table has
    /// overwritten them.
    pub base: Option<BorderSides>,
    /// Extra top padding `vertical-align` wrote on the computed style.
    pub shift: f32,
    /// The offset `vertical-align` added to the `y` of the children Taffy
    /// places (block boxes, anonymous boxes, floats, out-of-flow boxes), which
    /// they still carry. A read-back resets them, so it is zeroed there; a
    /// restyle replaces the computed style but not the children's boxes, so
    /// there it is kept apart from [`Self::shift`] (#2144).
    pub kids_shift: f32,
    /// The cell's max-content width, as [`RinchDocument::measure_table_columns`]
    /// last asked it; asked again once Taffy has dropped the cell's sizes
    /// (a change inside it, or to its style).
    pub max_content: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Track {
    Auto,
    Px(f32),
    Percent(f32),
}

/// What a table's Taffy style carries, on [`crate::node::Node::table_box`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TableBox {
    tracks: Vec<Track>,
    /// Max-content of each column ([`RinchDocument::measure_table_columns`]).
    weights: Vec<f32>,
    /// Whether the table has a width of its own.
    definite: bool,
    /// `table-layout: fixed` in force.
    fixed: bool,
    spacing: (f32, f32),
    /// The borders the cascade gave it, while it collapses them onto cells.
    base: Option<BorderSides>,
    /// The boxes the last rebuild placed.
    placed: Vec<RawNodeId>,
}

fn sides_of(cs: &crate::computed_style::ComputedStyle) -> BorderSides {
    [
        BorderSide {
            width: cs.border_top_width,
            style: cs.border_top_style,
            color: cs.border_top_color,
        },
        BorderSide {
            width: cs.border_right_width,
            style: cs.border_right_style,
            color: cs.border_right_color,
        },
        BorderSide {
            width: cs.border_bottom_width,
            style: cs.border_bottom_style,
            color: cs.border_bottom_color,
        },
        BorderSide {
            width: cs.border_left_width,
            style: cs.border_left_style,
            color: cs.border_left_color,
        },
    ]
}

fn write_sides(cs: &mut crate::computed_style::ComputedStyle, s: &BorderSides) {
    (
        cs.border_top_width,
        cs.border_top_style,
        cs.border_top_color,
    ) = (s[0].width, s[0].style, s[0].color);
    (
        cs.border_right_width,
        cs.border_right_style,
        cs.border_right_color,
    ) = (s[1].width, s[1].style, s[1].color);
    (
        cs.border_bottom_width,
        cs.border_bottom_style,
        cs.border_bottom_color,
    ) = (s[2].width, s[2].style, s[2].color);
    (
        cs.border_left_width,
        cs.border_left_style,
        cs.border_left_color,
    ) = (s[3].width, s[3].style, s[3].color);
}

const NO_SIDE: BorderSide = BorderSide {
    width: LengthPercentageValue::Zero,
    style: BorderStyleValue::None,
    color: None,
};

/// CSS 2.1 §17.6.2.1's order of styles: the higher wins a tie in width.
fn style_rank(s: BorderStyleValue) -> u8 {
    use BorderStyleValue as B;
    match s {
        B::Double => 8,
        B::Solid => 7,
        B::Dashed => 6,
        B::Dotted => 5,
        B::Ridge => 4,
        B::Outset => 3,
        B::Groove => 2,
        B::Inset => 1,
        _ => 0,
    }
}

/// The side that wins an edge. `b` is the later candidate, which comes from a
/// box lower in the cell > row > row group > table order, so it wins only by
/// beating `a` outright.
fn winner(a: BorderSide, b: BorderSide) -> BorderSide {
    use BorderStyleValue as B;
    if a.style == B::Hidden {
        return a;
    }
    if b.style == B::Hidden {
        return b;
    }
    let drawn = |s: BorderSide| {
        if s.style == B::None {
            0.0
        } else {
            s.width.to_px()
        }
    };
    let (wa, wb) = (drawn(a), drawn(b));
    if wb > wa || (wb == wa && style_rank(b.style) > style_rank(a.style)) {
        b
    } else {
        a
    }
}

/// One row, and the row group it closes over for `rowspan`.
struct Row {
    /// The row element, when there is one (a run of bare cells has none).
    id: Option<RawNodeId>,
    group: Option<RawNodeId>,
    cells: Vec<RawNodeId>,
}

#[derive(Default)]
struct Structure {
    head: Vec<Row>,
    body: Vec<Row>,
    foot: Vec<Row>,
    top_captions: Vec<RawNodeId>,
    bottom_captions: Vec<RawNodeId>,
    columns: Vec<RawNodeId>,
}

/// Every element child of `id`, seeing through `display: contents` wrappers
/// that are not table parts.
fn flat_children(tree: &NodeTree, id: RawNodeId, out: &mut Vec<RawNodeId>) {
    for &c in &tree.nodes[id].children {
        let n = &tree.nodes[c];
        if !n.is_element() {
            continue;
        }
        let cs = &n.computed_style;
        if cs.display == DisplayValue::Contents && cs.table_part == TablePart::None {
            flat_children(tree, c, out);
        } else if cs.table_part == TablePart::Other {
            // `display: none` on the tag is how a column box generates
            // nothing, so this precedes the check below.
            out.push(c);
        } else if cs.display != DisplayValue::None {
            out.push(c);
        }
    }
}

fn row_of(tree: &NodeTree, row: RawNodeId, group: Option<RawNodeId>) -> Row {
    let mut kids = Vec::new();
    flat_children(tree, row, &mut kids);
    kids.retain(|&c| {
        let cs = &tree.nodes[c].computed_style;
        cs.position != PositionValue::Absolute
            && cs.position != PositionValue::Fixed
            && cs.table_part != TablePart::Other
    });
    Row {
        id: Some(row),
        group,
        cells: kids,
    }
}

fn structure_of(tree: &NodeTree, table: RawNodeId) -> Structure {
    let mut s = Structure::default();
    let mut kids = Vec::new();
    flat_children(tree, table, &mut kids);
    let mut bare: Vec<RawNodeId> = Vec::new();
    for c in kids {
        let cs = &tree.nodes[c].computed_style;
        let part = cs.table_part;
        if !part.is_row_like() && part != TablePart::Caption && part != TablePart::Other {
            let out = matches!(cs.position, PositionValue::Absolute | PositionValue::Fixed);
            if !out {
                bare.push(c);
            }
            continue;
        }
        if !bare.is_empty() {
            s.body.push(Row {
                id: None,
                group: None,
                cells: std::mem::take(&mut bare),
            });
        }
        match part {
            TablePart::Row => s.body.push(row_of(tree, c, None)),
            TablePart::RowGroup | TablePart::HeaderGroup | TablePart::FooterGroup => {
                let mut rows = Vec::new();
                flat_children(tree, c, &mut rows);
                let list = match part {
                    TablePart::HeaderGroup => &mut s.head,
                    TablePart::FooterGroup => &mut s.foot,
                    _ => &mut s.body,
                };
                // Bare cells in a group share one anonymous row.
                let mut cells = Vec::new();
                for r in rows {
                    if tree.nodes[r].computed_style.table_part == TablePart::Row {
                        if !cells.is_empty() {
                            list.push(Row {
                                id: None,
                                group: Some(c),
                                cells: std::mem::take(&mut cells),
                            });
                        }
                        list.push(row_of(tree, r, Some(c)));
                    } else {
                        cells.push(r);
                    }
                }
                if !cells.is_empty() {
                    list.push(Row {
                        id: None,
                        group: Some(c),
                        cells,
                    });
                }
            }
            TablePart::Caption => {
                if cs.caption_bottom {
                    s.bottom_captions.push(c)
                } else {
                    s.top_captions.push(c)
                }
            }
            _ => s.columns.push(c),
        }
    }
    if !bare.is_empty() {
        s.body.push(Row {
            id: None,
            group: None,
            cells: bare,
        });
    }
    s
}

fn span_attr(tree: &NodeTree, id: RawNodeId, name: &str) -> Option<usize> {
    let n = &tree.nodes[id];
    if !matches!(n.tag(), Some("td" | "th")) {
        return None;
    }
    n.attributes.get(name)?.trim().parse().ok()
}

/// A cell's or column's contribution to its column: `width`, grown to the
/// border box when it declares a content box.
fn declared_track(tree: &NodeTree, id: RawNodeId) -> Track {
    let cs = &tree.nodes[id].computed_style;
    match cs.width {
        DimensionValue::Length(w) => {
            let extra = if cs.box_sizing_border_box {
                0.0
            } else {
                cs.padding_left.to_px()
                    + cs.padding_right.to_px()
                    + cs.border_left_width.to_px()
                    + cs.border_right_width.to_px()
            };
            Track::Px(w + extra)
        }
        DimensionValue::Percent(p) => Track::Percent(p),
        _ => Track::Auto,
    }
}

fn merge_track(a: Track, b: Track) -> Track {
    match (a, b) {
        (Track::Percent(x), Track::Percent(y)) => Track::Percent(x.max(y)),
        (Track::Percent(x), _) | (_, Track::Percent(x)) => Track::Percent(x),
        (Track::Px(x), Track::Px(y)) => Track::Px(x.max(y)),
        (Track::Px(x), Track::Auto) | (Track::Auto, Track::Px(x)) => Track::Px(x),
        (Track::Auto, Track::Auto) => Track::Auto,
    }
}

impl RinchDocument {
    /// The table `node_id` lays out in: itself, or its nearest ancestor-or-self
    /// that is not `display: contents`, when that box is a table.
    pub(crate) fn table_of(tree: &NodeTree, node_id: RawNodeId) -> Option<RawNodeId> {
        let mut id = node_id;
        loop {
            let node = tree.nodes.get(id)?;
            if node.computed_style.display != DisplayValue::Contents {
                return (node.computed_style.table_part == TablePart::Table).then_some(id);
            }
            id = node.parent?;
        }
    }

    /// Owe `parent`'s table a rebuild after its child list changed: neither a
    /// removal nor a move of an already-styled row restyles anything that
    /// would say so. Consecutive changes under one table are pushed once.
    pub(crate) fn note_table_children_changed(&mut self, parent: RawNodeId) {
        if let Some(table) = Self::table_of(&self.tree, parent)
            && self.tree.tables_owed.last() != Some(&table)
        {
            self.tree.tables_owed.push(table);
            self.tree.layout_dirty = true;
        }
    }

    /// Rebuild every table owed one. Run after the style sync and before
    /// every layout compute. A rebuild writes only the Taffy styles that
    /// moved, so a rebuild that changes nothing dirties nothing.
    pub(crate) fn resolve_tables(&mut self) {
        if self.tree.tables_owed.is_empty() {
            return;
        }
        let mut owed = std::mem::take(&mut self.tree.tables_owed);
        owed.sort_unstable();
        owed.dedup();
        for id in owed {
            if self.tree.contains(id)
                && self.tree.nodes[id].computed_style.table_part == TablePart::Table
            {
                self.rebuild_table(id);
            }
        }
    }

    fn rebuild_table(&mut self, table: RawNodeId) {
        if !self.tree.tables.contains(&table) {
            self.tree.tables.push(table);
        }
        let s = structure_of(&self.tree, table);
        let rows: Vec<&Row> = s.head.iter().chain(&s.body).chain(&s.foot).collect();
        let first_line = 1 + s.top_captions.len();

        // Where each row's group ends, for a `rowspan` that closes with it.
        let mut group_end = vec![rows.len(); rows.len()];
        let mut i = rows.len();
        while i > 0 {
            i -= 1;
            let same = i + 1 < rows.len() && rows[i].group == rows[i + 1].group;
            group_end[i] = if same { group_end[i + 1] } else { i + 1 };
        }

        // The slots: `open[x]` is how many more rows column x is taken for.
        let mut open: Vec<usize> = Vec::new();
        let mut placed: Vec<(RawNodeId, TableCell, usize)> = Vec::new();
        for (y, row) in rows.iter().enumerate() {
            let mut x = 0;
            for &cell in &row.cells {
                while open.get(x).is_some_and(|&n| n > 0) {
                    x += 1;
                }
                let left = group_end[y] - y;
                let cs = span_attr(&self.tree, cell, "colspan")
                    .unwrap_or(1)
                    .clamp(1, MAX_COLSPAN);
                let rs = match span_attr(&self.tree, cell, "rowspan") {
                    Some(0) => left,
                    Some(n) => n.min(MAX_ROWSPAN).min(left),
                    None => 1,
                };
                if open.len() < x + cs {
                    open.resize(x + cs, 0);
                }
                for o in &mut open[x..x + cs] {
                    *o = rs;
                }
                let bg = |id: Option<RawNodeId>| {
                    id.and_then(|i| self.tree.nodes[i].computed_style.background_color())
                };
                placed.push((
                    cell,
                    TableCell {
                        row: (y + first_line) as i16,
                        col: (x + 1) as i16,
                        rowspan: rs as u16,
                        colspan: cs as u16,
                        under: [bg(row.group), bg(row.id)],
                        ..Default::default()
                    },
                    y,
                ));
                x += cs;
            }
            for o in &mut open {
                *o = o.saturating_sub(1);
            }
        }
        let cols = open.len();

        let cs = &self.tree.nodes[table].computed_style;
        let definite = !matches!(
            cs.width,
            DimensionValue::Auto | DimensionValue::Intrinsic(_)
        );
        let fixed = cs.table_layout_fixed && definite;
        let collapse = cs.border_collapse;
        let spacing = if collapse {
            (0.0, 0.0)
        } else {
            (cs.border_spacing.0, cs.border_spacing.1)
        };

        // Declared widths. Columns first, then cells of span 1; a fixed table
        // reads no row past its first.
        let mut tracks = vec![Track::Auto; cols];
        let mut x = 0;
        for &c in &s.columns {
            let mut kids = Vec::new();
            flat_children(&self.tree, c, &mut kids);
            let elements: Vec<RawNodeId> = if kids.is_empty() { vec![c] } else { kids };
            for e in elements {
                let n = self.tree.nodes[e]
                    .attributes
                    .get("span")
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(1usize)
                    .max(1);
                let t = declared_track(&self.tree, e);
                for slot in tracks.iter_mut().skip(x).take(n) {
                    *slot = merge_track(*slot, t);
                }
                x += n;
            }
        }
        for (cell, slot, y) in &placed {
            if slot.colspan == 1 && (!fixed || *y == 0) {
                let i = slot.col as usize - 1;
                tracks[i] = merge_track(tracks[i], declared_track(&self.tree, *cell));
            }
        }

        if collapse {
            self.collapse_borders(table, &s, &rows, &mut placed, cols, first_line);
        } else if let Some(base) = self.tree.nodes[table]
            .table_box
            .as_mut()
            .and_then(|b| b.base.take())
        {
            write_sides(&mut self.tree.nodes[table].computed_style, &base);
        }

        // Captions take the whole width, above or below every row.
        let last = first_line + rows.len();
        let captions = s
            .top_captions
            .iter()
            .enumerate()
            .map(|(i, &c)| (c, i + 1))
            .chain(
                s.bottom_captions
                    .iter()
                    .enumerate()
                    .map(|(i, &c)| (c, last + i)),
            )
            .collect::<Vec<_>>();
        for (c, y) in captions {
            let t = TableCell {
                row: y as i16,
                col: 0,
                rowspan: 1,
                colspan: (cols.max(1) + 1) as u16,
                ..Default::default()
            };
            self.place(c, t);
        }

        let ids: Vec<RawNodeId> = placed
            .iter()
            .map(|p| p.0)
            .chain(s.top_captions.iter().copied())
            .chain(s.bottom_captions.iter().copied())
            .collect();
        for (cell, slot, _) in placed {
            self.place(cell, slot);
        }

        let prev = self.tree.nodes[table].table_box.take();
        let (weights, base, was) = prev
            .map(|p| (p.weights, p.base, p.placed))
            .unwrap_or_default();
        // A box that stopped being a cell of this table gets its own style back.
        let kept: std::collections::HashSet<_> = ids.iter().copied().collect();
        for id in was {
            if !kept.contains(&id)
                && let Some(n) = self.tree.nodes.get_mut(id)
                && n.table_cell.take().is_some()
            {
                let style = self.taffy_style_from_computed(id);
                if let Some(t) = self.tree.nodes[id].taffy_id {
                    let _ = self.tree.taffy.set_style(t, style);
                    self.tree.layout_dirty = true;
                }
            }
        }
        self.tree.nodes[table].table_box = Some(Box::new(TableBox {
            tracks,
            weights,
            definite,
            fixed,
            spacing,
            base,
            placed: ids,
        }));
        self.sync_table_style(table);
    }

    /// Record a cell's place and write it to its Taffy style.
    fn place(&mut self, id: RawNodeId, mut slot: TableCell) {
        let node = &mut self.tree.nodes[id];
        if let Some(old) = &node.table_cell {
            slot.base = slot.base.or(old.base);
            slot.shift = old.shift;
            slot.kids_shift = old.kids_shift;
            slot.max_content = old.max_content;
            if old.under != slot.under {
                self.tree.paint_dirty_nodes.push(id);
            }
        }
        self.tree.nodes[id].table_cell = Some(Box::new(slot));
        self.sync_table_style(id);
    }

    fn sync_table_style(&mut self, id: RawNodeId) {
        let Some(t) = self.tree.nodes[id].taffy_id else {
            return;
        };
        let Ok(old) = self.tree.taffy.style(t).cloned() else {
            return;
        };
        let mut style = old.clone();
        self.apply_table_style(id, &mut style);
        if style != old {
            let _ = self.tree.taffy.set_style(t, style);
            self.tree.layout_dirty = true;
            self.mark_atomic_inline_dirty(id);
            self.tree.perf.bump(crate::perf::Counter::TaffyStyleChanges);
        }
    }

    /// Write what a table part's Taffy style carries beyond its computed
    /// values, from the node's last rebuild. Every site that rebuilds a Taffy
    /// style from `computed_style` goes through this.
    pub(crate) fn apply_table_style(&self, id: RawNodeId, style: &mut taffy::Style) {
        let node = &self.tree.nodes[id];
        if let Some(b) = &node.table_box {
            let can_weigh = b.definite && !b.fixed;
            let all_declared = b.tracks.iter().all(|t| *t != Track::Auto);
            style.grid_template_columns = b
                .tracks
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let w = b.weights.get(i).copied().filter(|w| *w > 0.0);
                    taffy::GridTemplateComponent::Single(match *t {
                        Track::Auto if b.fixed => fr(1.0),
                        Track::Auto => match w {
                            Some(w) if can_weigh => minmax(min_content(), fr(w)),
                            _ => taffy::style_helpers::auto(),
                        },
                        Track::Px(p) if b.fixed => length(p),
                        Track::Px(p) if can_weigh && all_declared => {
                            minmax(min_content(), fr(p.max(1.0)))
                        }
                        Track::Px(p) => minmax(min_content(), length(p)),
                        Track::Percent(p) if b.definite => minmax(min_content(), percent(p)),
                        Track::Percent(_) => taffy::style_helpers::auto(),
                    })
                })
                .collect();
            style.gap = taffy::Size {
                width: length(b.spacing.0),
                height: length(b.spacing.1),
            };
            let cs = &node.computed_style;
            style.padding = taffy::Rect {
                top: length(cs.padding_top.to_px() + b.spacing.1),
                bottom: length(cs.padding_bottom.to_px() + b.spacing.1),
                left: length(cs.padding_left.to_px() + b.spacing.0),
                right: length(cs.padding_right.to_px() + b.spacing.0),
            };
            if !b.definite && matches!(node.computed_style.width, DimensionValue::Auto) {
                style.size.width = taffy::Dimension::fit_content();
            }
        }
        if let Some(c) = &node.table_cell {
            style.grid_row = taffy::Line {
                start: line(c.row),
                end: span(c.rowspan.max(1)),
            };
            style.grid_column = if c.col == 0 {
                taffy::Line {
                    start: line(1),
                    end: line(c.colspan as i16),
                }
            } else {
                taffy::Line {
                    start: line(c.col),
                    end: span(c.colspan.max(1)),
                }
            };
            if node.computed_style.table_part != TablePart::Caption {
                // A cell's `height` is a minimum, and its `width` is its
                // column's. It fills its slot and has no margins.
                if let DimensionValue::Length(h) = node.computed_style.height {
                    style.min_size.height = length(h);
                }
                style.size = taffy::Size::auto();
                style.margin = taffy::Rect::length(0.0);
                style.align_self = Some(taffy::AlignSelf::STRETCH);
                style.justify_self = Some(taffy::JustifySelf::STRETCH);
                if c.shift != 0.0 {
                    style.padding.top =
                        length((node.computed_style.padding_top.to_px() - c.shift).max(0.0));
                }
            }
        }
    }

    /// Resolve every shared edge of a collapsing table and write the winners
    /// to its cells.
    #[allow(clippy::needless_range_loop)] // two rows of `h` at one `x`
    fn collapse_borders(
        &mut self,
        table: RawNodeId,
        s: &Structure,
        rows: &[&Row],
        placed: &mut [(RawNodeId, TableCell, usize)],
        cols: usize,
        first_line: usize,
    ) {
        let (nr, nc) = (rows.len(), cols);
        if nr == 0 || nc == 0 {
            return;
        }
        // The table's own border is an outer candidate and paints nothing.
        let tb = {
            let n = &mut self.tree.nodes[table];
            let cur = sides_of(&n.computed_style);
            let bx = n.table_box.get_or_insert_with(Default::default);
            *bx.base.get_or_insert(cur)
        };
        write_sides(&mut self.tree.nodes[table].computed_style, &[NO_SIDE; 4]);

        // `h[y][x]`: the edge above row y of column x. `v[y][x]`: left of it.
        let mut h = vec![vec![NO_SIDE; nc]; nr + 1];
        let mut v = vec![vec![NO_SIDE; nc + 1]; nr];
        let mut cell_base = Vec::with_capacity(placed.len());
        for (id, slot, _) in placed.iter_mut() {
            let n = &self.tree.nodes[*id];
            let base = n
                .table_cell
                .as_ref()
                .and_then(|c| c.base)
                .unwrap_or_else(|| sides_of(&n.computed_style));
            slot.base = Some(base);
            let (y0, x0) = (slot.row as usize - first_line, slot.col as usize - 1);
            let (y1, x1) = (y0 + slot.rowspan as usize, x0 + slot.colspan as usize);
            for x in x0..x1 {
                h[y0][x] = winner(h[y0][x], base[0]);
                h[y1][x] = winner(h[y1][x], base[2]);
            }
            for row in v.iter_mut().take(y1).skip(y0) {
                row[x0] = winner(row[x0], base[3]);
                row[x1] = winner(row[x1], base[1]);
            }
            cell_base.push((y0, x0, y1, x1));
        }
        // Rows, then row groups, then the table: each loses a tie to the last.
        for (y, row) in rows.iter().enumerate() {
            if let Some(id) = row.id {
                let b = sides_of(&self.tree.nodes[id].computed_style);
                for x in 0..nc {
                    h[y][x] = winner(h[y][x], b[0]);
                    h[y + 1][x] = winner(h[y + 1][x], b[2]);
                }
                v[y][0] = winner(v[y][0], b[3]);
                v[y][nc] = winner(v[y][nc], b[1]);
            }
        }
        let mut y = 0;
        for list in [&s.head, &s.body, &s.foot] {
            for r in list.iter() {
                let group = r.group;
                if let Some(id) = group {
                    let b = sides_of(&self.tree.nodes[id].computed_style);
                    let first = y == 0 || rows[y - 1].group != group;
                    let lastr = y + 1 == nr || rows[y + 1].group != group;
                    for x in 0..nc {
                        if first {
                            h[y][x] = winner(h[y][x], b[0]);
                        }
                        if lastr {
                            h[y + 1][x] = winner(h[y + 1][x], b[2]);
                        }
                    }
                    v[y][0] = winner(v[y][0], b[3]);
                    v[y][nc] = winner(v[y][nc], b[1]);
                }
                y += 1;
            }
        }
        for x in 0..nc {
            h[0][x] = winner(h[0][x], tb[0]);
            h[nr][x] = winner(h[nr][x], tb[2]);
        }
        for row in v.iter_mut() {
            row[0] = winner(row[0], tb[3]);
            row[nc] = winner(row[nc], tb[1]);
        }

        let hidden = |s: BorderSide| {
            if s.style == BorderStyleValue::Hidden {
                NO_SIDE
            } else {
                s
            }
        };
        for ((id, _, _), (y0, x0, y1, x1)) in placed.iter().zip(cell_base) {
            let right = hidden(v[y0][x1]);
            let bottom = hidden(h[y1][x0]);
            let top = if y0 == 0 { hidden(h[0][x0]) } else { NO_SIDE };
            let left = if x0 == 0 { hidden(v[y0][0]) } else { NO_SIDE };
            let sides = [top, right, bottom, left];
            let cs = &mut self.tree.nodes[*id].computed_style;
            if sides_of(cs) != sides {
                write_sides(cs, &sides);
                let px = |s: BorderSide| length(s.width.to_px());
                if let Some(t) = self.tree.nodes[*id].taffy_id
                    && let Ok(old) = self.tree.taffy.style(t).cloned()
                {
                    let mut st = old;
                    st.border = taffy::Rect {
                        top: px(top),
                        right: px(right),
                        bottom: px(bottom),
                        left: px(left),
                    };
                    let _ = self.tree.taffy.set_style(t, st);
                    self.tree.layout_dirty = true;
                }
                self.tree.paint_dirty_nodes.push(*id);
            }
        }
        if let Some(t) = self.tree.nodes[table].taffy_id
            && let Ok(old) = self.tree.taffy.style(t).cloned()
            && old.border != taffy::Rect::length(0.0)
        {
            let mut st = old;
            st.border = taffy::Rect::length(0.0);
            let _ = self.tree.taffy.set_style(t, st);
            self.tree.layout_dirty = true;
        }
    }

    /// Measure the columns of every table whose room is shared out by weight:
    /// a table with a width of its own, with `auto` columns, whose subtree
    /// changed since. Innermost first, because an outer table's max-content
    /// reads an inner one's.
    pub(crate) fn measure_table_columns(&mut self) {
        if self.tree.tables.is_empty() {
            return;
        }
        let nodes = &self.tree.nodes;
        self.tree.tables.retain(|&id| {
            nodes
                .get_live(id)
                .is_some_and(|n| n.computed_style.table_part == TablePart::Table)
        });
        let live = self.tree.tables.clone();
        for &id in &live {
            self.hide_blank_runs(id);
        }
        let mut tables: Vec<(usize, RawNodeId)> = live
            .into_iter()
            .filter(|&id| {
                self.tree.nodes[id]
                    .table_box
                    .as_ref()
                    .is_some_and(|b| b.definite && !b.fixed && b.tracks.contains(&Track::Auto))
            })
            .map(|id| {
                let mut depth = 0;
                let mut p = self.tree.nodes[id].parent;
                while let Some(i) = p {
                    depth += 1;
                    p = self.tree.nodes[i].parent;
                }
                (depth, id)
            })
            .collect();
        tables.sort_unstable_by_key(|&(d, id)| (std::cmp::Reverse(d), id));
        for (_, id) in tables {
            let Some(t) = self.tree.nodes[id].taffy_id else {
                continue;
            };
            if !self.tree.taffy.dirty(t).unwrap_or(false) {
                continue;
            }
            let Ok(style) = self.tree.taffy.style(t).cloned() else {
                continue;
            };
            let cols = self.tree.nodes[id].table_box.as_ref().unwrap().tracks.len();
            let from_cells = if self.tree.table_columns_by_detached_layout {
                None
            } else {
                self.column_weights_from_cells(id, t, cols)
            };
            if let Some(weights) = from_cells {
                let b = self.tree.nodes[id].table_box.as_mut().unwrap();
                if b.weights != weights {
                    b.weights = weights;
                    let mut restyled = style.clone();
                    self.apply_table_style(id, &mut restyled);
                    if restyled != style {
                        let _ = self.tree.taffy.set_style(t, restyled);
                    }
                }
                continue;
            }
            let mut probe = style.clone();
            probe.size.width = taffy::Dimension::auto();
            probe.grid_template_columns =
                vec![taffy::GridTemplateComponent::Single(taffy::style_helpers::auto()); cols];
            let _ = self.tree.taffy.set_style(t, probe);
            self.run_taffy_compute(
                t,
                taffy::Size {
                    width: taffy::AvailableSpace::MaxContent,
                    height: taffy::AvailableSpace::MaxContent,
                },
            );
            let mut weights = vec![0.0f32; cols];
            for (col, span, w) in self.table_cells_of(id) {
                if span == 1 && col > 0 {
                    let i = col as usize - 1;
                    if i < cols {
                        weights[i] = weights[i].max(w);
                    }
                }
            }
            if let Some(b) = self.tree.nodes[id].table_box.as_mut() {
                b.weights = weights;
            }
            let mut restored = style;
            self.apply_table_style(id, &mut restored);
            let _ = self.tree.taffy.set_style(t, restored);
        }
    }

    /// Each column's max-content width, from its cells' own, when every box
    /// the table's grid lays out is a cell one column wide: an `auto` track
    /// under max-content is then as wide as its widest cell, which is what
    /// the detached layout below reads. A cell is asked only when Taffy has
    /// dropped its sizes since it was last asked, and asked size-only, which
    /// leaves the layout every cell keeps for the root compute in place
    /// (#2141: the detached layout asked every row twice per edit, once
    /// itself and once more in the root compute it had evicted).
    ///
    /// `None` — a spanning cell or a caption shares its width between
    /// columns, or the grid holds a box that is no cell — leaves it to the
    /// detached layout.
    fn column_weights_from_cells(
        &mut self,
        table: RawNodeId,
        t: taffy::NodeId,
        cols: usize,
    ) -> Option<Vec<f32>> {
        let placed = self.tree.nodes[table].table_box.as_ref()?.placed.clone();
        let mut cells = Vec::with_capacity(placed.len());
        for id in placed {
            let n = self.tree.nodes.get_live(id)?;
            let cell = n.table_cell.as_ref()?;
            if cell.colspan != 1 || cell.col < 1 || cell.col as usize > cols {
                return None;
            }
            cells.push((id, n.taffy_id?, cell.col as usize - 1));
        }
        let ids: std::collections::HashSet<taffy::NodeId> =
            cells.iter().map(|&(_, tid, _)| tid).collect();
        for child in self.tree.taffy.children(t).ok()? {
            if ids.contains(&child) {
                continue;
            }
            let st = self.tree.taffy.style(child).ok()?;
            if st.display != taffy::Display::None && st.position != taffy::Position::Absolute {
                return None;
            }
        }
        let ask: Vec<(RawNodeId, taffy::NodeId)> = cells
            .iter()
            .filter(|&&(id, tid, _)| {
                self.tree.nodes[id]
                    .table_cell
                    .as_ref()
                    .is_none_or(|c| c.max_content.is_none())
                    || self.tree.taffy.dirty(tid).unwrap_or(true)
            })
            .map(|&(id, tid, _)| (id, tid))
            .collect();
        if !ask.is_empty() {
            let tids: Vec<taffy::NodeId> = ask.iter().map(|&(_, tid)| tid).collect();
            let sizes = self.measure_taffy_sizes(
                &tids,
                taffy::Size {
                    width: taffy::AvailableSpace::MaxContent,
                    height: taffy::AvailableSpace::MaxContent,
                },
            );
            self.tree
                .perf
                .add(crate::perf::Counter::TableColumnMeasures, ask.len() as u64);
            for ((id, _), size) in ask.into_iter().zip(sizes) {
                if let Some(c) = self.tree.nodes[id].table_cell.as_mut() {
                    // Rounded, as the detached layout's boxes are.
                    c.max_content = Some(size.width.round());
                }
            }
        }
        let mut weights = vec![0.0f32; cols];
        for (id, _, col) in cells {
            let w = self.tree.nodes[id]
                .table_cell
                .as_ref()
                .and_then(|c| c.max_content)
                .unwrap_or(0.0);
            weights[col] = weights[col].max(w);
        }
        Some(weights)
    }

    /// The white space between a table's rows and cells generates no box
    /// (css-tables-3 §3.9). The IFC pass mints an anonymous block box around
    /// it, as around any run in a grid, which would be a stray grid item.
    fn hide_blank_runs(&mut self, table: RawNodeId) {
        let mut parts = vec![table];
        let mut boxes = Vec::new();
        while let Some(p) = parts.pop() {
            let n = &self.tree.nodes[p];
            boxes.extend(n.run_boxes.iter().copied());
            parts.extend(n.children.iter().copied().filter(|&c| {
                let cs = &self.tree.nodes[c].computed_style;
                cs.table_part.is_row_like()
                    || (cs.display == DisplayValue::Contents && cs.table_part == TablePart::None)
            }));
        }
        for b in boxes {
            let Some(n) = self.tree.nodes.get_live(b) else {
                continue;
            };
            let blank = n.run_members.iter().all(|&m| {
                let m = &self.tree.nodes[m];
                m.text_content()
                    .is_some_and(|t| t.chars().all(char::is_whitespace))
            });
            if let (true, Some(t)) = (blank, n.taffy_id)
                && let Ok(old) = self.tree.taffy.style(t).cloned()
                && old.display != taffy::Display::None
            {
                let mut st = old;
                st.display = taffy::Display::None;
                let _ = self.tree.taffy.set_style(t, st);
            }
        }
    }

    /// `(column, colspan, laid-out width)` of each cell of `table`, read from
    /// Taffy.
    fn table_cells_of(&self, table: RawNodeId) -> Vec<(i16, u16, f32)> {
        let s = structure_of(&self.tree, table);
        s.head
            .iter()
            .chain(&s.body)
            .chain(&s.foot)
            .flat_map(|r| r.cells.iter())
            .filter_map(|&c| {
                let n = &self.tree.nodes[c];
                let slot = n.table_cell.as_ref()?;
                let w = self.tree.taffy.layout(n.taffy_id?).ok()?.size.width;
                Some((slot.col, slot.colspan, w))
            })
            .collect()
    }

    /// Take back a cell's `vertical-align` as its box is read back: the read
    /// reset its children, and its height may have moved. The shift is applied
    /// again by [`Self::align_table_cells`], once the cell's text is laid out.
    pub(crate) fn note_table_cell_read(&mut self, id: RawNodeId) {
        let node = &mut self.tree.nodes[id];
        let Some(cell) = node.table_cell.as_mut() else {
            return;
        };
        if node.computed_style.table_part == TablePart::Caption {
            return;
        }
        cell.kids_shift = 0.0;
        let shift = std::mem::take(&mut cell.shift);
        if shift != 0.0 {
            let top = node.computed_style.padding_top.to_px() - shift;
            node.computed_style.padding_top = if top == 0.0 {
                LengthPercentageValue::Zero
            } else {
                LengthPercentageValue::Length(top)
            };
        }
        self.tree.cells_to_align.push(id);
    }

    /// Apply `vertical-align` to every cell read back this layout. See the
    /// module docs.
    pub(crate) fn align_table_cells(&mut self) {
        for id in std::mem::take(&mut self.tree.cells_to_align) {
            if self.tree.contains(id) {
                self.align_table_cell(id);
            }
        }
    }

    /// Apply a cell's `vertical-align`: its leftover height becomes extra top
    /// padding (`TableCell::shift`), which moves everything its own inline
    /// formatting context positions — text, and the atomic inlines in it, whose
    /// boxes are relative to its content box — and an offset on the `y` of
    /// each child Taffy places (`TableCell::kids_shift`).
    ///
    /// Idempotent, whatever a layout re-read: each half is applied as the
    /// difference from what it already carries. It used to add the whole shift
    /// to every child, on the assumption that the read-back had just reset
    /// them all. An atomic inline is not reset by a read (its place is its
    /// line's, `write_inline_positions`), so it took the shift once more per
    /// layout its line was not rebuilt in, on top of the padding that already
    /// moved it (#2144).
    fn align_table_cell(&mut self, id: RawNodeId) {
        let node = &self.tree.nodes[id];
        let Some(cell) = &node.table_cell else {
            return;
        };
        if node.computed_style.table_part == TablePart::Caption {
            return;
        }
        let (old, kids_old) = (cell.shift, cell.kids_shift);
        let align = node.computed_style.vertical_align;
        let cs = &node.computed_style;
        let base_top = cs.padding_top.to_px() - old;
        let inset_top = base_top + cs.border_top_width.to_px();
        let inset_bottom = cs.padding_bottom.to_px() + cs.border_bottom_width.to_px();
        let avail = node.layout.height - inset_top - inset_bottom;
        let content = match &node.text_layout {
            Some(l) => l.layout.height(),
            None => {
                let kids = Self::box_tree_children(&self.tree.nodes, id);
                kids.iter()
                    .map(|&c| {
                        let l = &self.tree.nodes[c].layout;
                        l.y - kids_old + l.height
                    })
                    .fold(0.0, f32::max)
                    - inset_top
            }
        };
        let room = (avail - content).max(0.0);
        let plain = matches!(
            cs.padding_top,
            LengthPercentageValue::Zero | LengthPercentageValue::Length(_)
        );
        let shift = match align {
            _ if !plain => 0.0,
            VerticalAlignValue::Middle => room / 2.0,
            VerticalAlignValue::Bottom => room,
            _ => 0.0,
        };
        if shift == old && shift == kids_old {
            return;
        }
        let children = Self::box_tree_children(&self.tree.nodes, id).into_owned();
        let node = &mut self.tree.nodes[id];
        let cell = node.table_cell.as_mut().unwrap();
        cell.shift = shift;
        cell.kids_shift = shift;
        if plain {
            let top = base_top + shift;
            node.computed_style.padding_top = if top == 0.0 {
                LengthPercentageValue::Zero
            } else {
                LengthPercentageValue::Length(top)
            };
        }
        let delta = shift - kids_old;
        if delta == 0.0 {
            return;
        }
        for c in children {
            let child = &mut self.tree.nodes[c];
            // Placed by the cell's own lines, relative to its content box:
            // the padding above has moved it already.
            if child.ifc_root == Some(id)
                && (child.is_text() || child.display_mode.is_atomic_inline())
            {
                continue;
            }
            child.layout.y += delta;
        }
    }
}
