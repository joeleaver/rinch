//! CSS table layout, on Taffy's grid.
//!
//! Taffy has no table formatting context, and the box a table needs is close
//! to the one a grid gives: tracks sized from their cells, spanning items,
//! rows as tall as their tallest cell. So a table box **is** a Taffy grid.
//! Row groups and rows compute [`DisplayValue::Contents`] and generate no box,
//! which puts every cell in the table's own Taffy child list, and
//! `RinchDocument::resolve_tables` does what is specifically a table's:
//!
//! - **Slots.** The HTML "forming a table" algorithm assigns each cell its
//!   row and column, honouring `rowspan`, `colspan`, and a `thead`/`tfoot` in
//!   any source position. A cell is placed on explicit grid lines.
//! - **Columns.** A cell's `width` is a statement about its column, not a
//!   size for the cell, so it is read here and never reaches Taffy. See
//!   *Column widths*.
//! - **Spacing.** `border-spacing` is the grid's gap, and the same amount of
//!   padding on the table, which is where the outer spacing lives. A caption
//!   is outside both (*Captions*).
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
//! An automatic-layout table shares its width between its columns as CSS
//! 2.1 §17.5.2.2 and Blink's `DistributeInlineSizeToComputedInlineSizeAuto`
//! do, through the Taffy fork's `proportional_column_sizing` on its grid
//! (items 10 and 12 in `docs/internals/dependencies.md`). Blink sizes the
//! table between four "guesses": every column at its min-content (the *min
//! guess*), then the percentage columns at their percentage, then the
//! declared columns at their width (the *specified guess*), then every
//! column at its max-content (the *max guess*). A width between two guesses
//! is reached by growing the columns that differ between them, each by the
//! same fraction of its own difference; Taffy's Maximise Tracks step does it
//! in those stages, each track in proportion to its room to grow. So a
//! table narrower than its declared columns gives each its min-content plus
//! the same fraction of its declared width minus min-content (#2306; a
//! cell's declared width counts its padding and border), and one narrower
//! than its max-content does the same from min-content to max-content
//! (#2281); both used to share the width in proportion to the declared or
//! max-content widths. Past the max guess the excess goes to the `auto`
//! columns in proportion to their max-content (when they are all empty, in
//! equal shares: an email's filler cell beside its logo), or with none, to
//! the declared ones in proportion to their widths, then to the percentage
//! ones (the fork's stretch step). A percentage resolves against the
//! table's width less every spacing, as in Chrome (#2333: the fork scales a
//! table's percentage tracks, which a grid resolves against its content box,
//! gaps included, by `(width - gaps) / width`), and percentages past 100% are
//! capped in column order, each to what the earlier ones left (#2334,
//! `cap_percent_tracks`; a fixed table's are left as they are).
//!
//! The tracks: an `auto` column is `minmax(min-content, auto)` in a table
//! with a width of its own (its growth limit is its cells' max-content) and
//! `auto` in one without; a declared column is `minmax(min-content, Npx)`;
//! a percentage column `minmax(min-content, P%)` in a table with a width,
//! an `auto` track in one without (#2284). A declared column's limit is
//! its width, floored at its cells' min-content (Taffy floors a growth
//! limit at the base size): a `width=50` cell holding a 100px box asks 100
//! (#2288), and a picture that scales into its cell (min-content 0) only
//! the cell's width. Another cell's max-content does not raise it: Chrome
//! 156 shares a table's excess between declared columns by their widths
//! (`<td width=50>` over a cell of max-content 206, beside `<td width=60>`,
//! comes out 56:66). `RinchDocument::measure_table_columns` still asks
//! each cell of a table with a width its max-content (size-only, and only
//! again once it changed, #2141; with a spanning cell or a caption, by a
//! detached layout), which weighed the columns before #2306 and which
//! nothing reads now.
//!
//! A `colspan` cell's min-content past its columns' goes to them in
//! proportion to their room to grow up to their max-content, then in
//! proportion to their max-content (#2281; Blink's
//! `DistributeColspanCellToColumnsAuto`), and its max-content past theirs in
//! proportion to their max-content (#2280). A table with a width of its own
//! is never narrower than its columns' minimum (#2266): its Taffy style sets
//! `item_is_table`, so its block parent leaves it to resolve its own width,
//! and the fork's grid widens to its tracks. A `table-layout: fixed` table
//! keeps its width, as in Chrome. A table with no width of its own shrinks
//! to fit on these tracks and needs no measure. Not modelled (Chrome 156
//! widens both to the columns): a table with a width as a flex item, which
//! is handed its width and does not widen; and a table's min-content
//! contribution to a shrink-to-fit parent, which is its `width`.
//!
//! A cell's `max-width` caps what it asks of its column (its min-content,
//! max-content and `width`), and the cell is still laid out at its column's
//! width (#2264): its Taffy `max_size` caps the contributions, and
//! `stretch_past_max_width` lets the grid stretch it past that.
//!
//! ## Row heights
//!
//! A row has no box, so its `height` is a statement about its track: a
//! length one (`0` included) is the track's minimum, `minmax(<height>,
//! auto)`. The Taffy fork's `table_rows` sizes the rows as Blink does
//! (item 15 in `docs/internals/dependencies.md`), not as a grid's tracks:
//! each row is as tall as its tallest cell of span 1 and at least its
//! `height`, and a row with a `height`, or holding a cell with one, is
//! constrained. A rowspan cell taller than its rows grows the rows where
//! another spanning cell starts, else its unconstrained rows that hold
//! something in proportion to their heights, else (all empty) the last, else
//! every row by its height. A taller table gives its extra height to its
//! body sections by their heights, and each section to its rows the same
//! way, empty rows growing when every other is constrained (a section's
//! share is by its rows' heights before any rowspan grew them); a caption
//! takes none (the table is its `height` plus its captions), and a table
//! shorter than its rows grows to them (#2312), also when its parent hands
//! it its height: a flex item, a grid item in a shorter track (which it
//! overflows) and an absolutely positioned table grow too (#2358; the fork's
//! `resolves_own_height` on each kind of child style). `rebuild_table`
//! hands the fork each row's section (`TableBox::sections`) and which rows
//! are captions. A block standing for an anonymous table keeps its own
//! height. A percentage and a row group's `height` size nothing. A
//! collapsing table has no padding (CSS 2.1 §17.6.2).
//!
//! ## Captions
//!
//! A caption is a row of the grid that spans every column, above the rows or
//! below them. CSS puts it outside the table box (CSS 2.2 §17.4), so outside
//! the table's border and the padding that carries the outer spacing, and
//! with no spacing between it and the rows or another caption (#2350, #2394).
//! The fork leaves the gutters beside a caption row empty, and the caption's
//! Taffy margins take the table's border and padding back
//! (`TableCell::outside`, written by `rebuild_table`): a top caption gets
//! `-(border-top + padding-top)` above and as much below, so it sits at the
//! table's top edge and the border and padding (spacing included) lie between
//! it and the first row; a bottom caption the mirror; and the same across,
//! so it is as wide as the table's border box, and a wider caption widens the
//! table to its own width. The table box is then the grid less its captions'
//! bands, which is where paint draws the table's background, border, shadows
//! and outline (`caption_bands`; they were drawn round the captions too). A
//! block standing for an anonymous table keeps its border round its
//! captions. Its own margins are added on top, and every one is
//! written as a length. A caption's containing block is the table wrapper, as
//! wide as the table, so a percentage or `calc()` margin resolves against the
//! table's border-box width, and `auto` side margins beside a `width` share
//! what the table's width leaves (`auto` is zero otherwise). These need the
//! width, so the calc pass resolves them after each compute
//! (`patch_caption_margins`, from `calc_layout.rs`). A percentage side margin
//! in a table as wide as its content is taken as zero, because it would feed
//! its own basis (Chrome resolves it against the final width). A top or bottom
//! one resolves, since it sizes no width.
//!
//! ## Vertical alignment
//!
//! Taffy's block layout has no `align-content`, and a table cell is a block
//! container, so `vertical-align` is applied after the compute: the cell's
//! leftover height becomes extra top padding on the computed style (what
//! positions its text, and the atomic inlines in its lines) and an offset on
//! the children Taffy places. Blink calls the same
//! thing intrinsic padding. It reaches no Taffy style, so it cannot feed back
//! into the row's height.
//!
//! `baseline` (and every value but `top`, `middle` and `bottom`) aligns a
//! row's cells on one baseline (#2359, `align_row_baselines`): each cell's
//! first in-flow line's, or with none the bottom of its content, the lowest
//! of them from the row's top the row's, and each cell's content moved down
//! to it. A block in a row (a `<td style="display: block">`) stands in an
//! anonymous cell whose `vertical-align` is `baseline`, so it takes part,
//! counting its top margin, and is moved itself. A spanning cell aligns in
//! its first row. Not modelled: the row does not grow for it (the fork's
//! `table_rows` sizes rows from their cells' heights), so a cell moved below
//! its row's bottom overflows it (Chrome grows the row).
//!
//! A cell standing in an anonymous inline-table's place
//! (`wrap_cell_in_inline_table`) is no table's cell, and is aligned the
//! same way by its own `vertical-align` (#2514), its shift kept on
//! [`crate::node::Node::anon_cell`].

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
    /// The cell's max-content width, as `RinchDocument::measure_table_columns`
    /// last asked it; asked again once Taffy has dropped the cell's sizes
    /// (a change inside it, or to its style).
    pub max_content: Option<f32>,
    /// The same for its min-content width, asked only of a cell with a
    /// `width` of its own, which weighs its column by the larger of the two
    /// (#2288). The width itself is read at each weighing: a cell's `width`
    /// never reaches its Taffy style, so a change to it leaves this valid.
    pub min_content: Option<f32>,
    /// A caption's: what its Taffy margins (top, right, bottom, left) add to
    /// its own, to take it out of the table's padding and outer spacing
    /// (#2350; see *Captions* in the module docs). Zero for a cell.
    pub outside: [f32; 4],
}

/// What `vertical-align` moved in a cell standing in an anonymous
/// inline-table's place, on [`crate::node::Node::anon_cell`]: the same two
/// halves as [`TableCell::shift`] and [`TableCell::kids_shift`] (#2514).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CellShift {
    pub shift: f32,
    pub kids_shift: f32,
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
    /// What each column's cells ask of it ([`RinchDocument::measure_table_columns`]).
    weights: Vec<f32>,
    /// Whether the table has a width of its own.
    definite: bool,
    /// `table-layout: fixed` in force.
    fixed: bool,
    spacing: (f32, f32),
    /// `border-collapse: collapse`: the table has no padding (CSS 2.1
    /// §17.6.2, Chrome), whatever it declares.
    collapse: bool,
    /// The grid line of the first row, after the top captions.
    first_row: usize,
    /// Each row's declared `height` in px, when it has a length one: the
    /// least its track is (see *Row heights*).
    row_heights: Vec<Option<f32>>,
    /// Each row's section (a run of rows of one row group, or of bare rows)
    /// and whether it is body-like (not a `thead` or `tfoot`), which takes a
    /// taller table's extra height first.
    sections: Vec<(u16, bool)>,
    /// How many captions follow the rows.
    bottom_captions: usize,
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

/// Whether `id` is a row group, a row or a cell: a part that needs a table.
fn is_row_part(tree: &NodeTree, id: RawNodeId) -> bool {
    tree.nodes[id].computed_style.table_part.is_row_like()
        || tree.nodes[id].computed_style.table_part == TablePart::Cell
}

/// Whether `node` is a block (or inline-block) standing for a table
/// ([`RinchDocument::note_table_host`]).
pub(crate) fn is_host(node: &crate::node::Node) -> bool {
    node.computed_style.table_part == TablePart::Table
        && matches!(
            node.computed_style.display,
            DisplayValue::Block | DisplayValue::ListItem | DisplayValue::InlineBlock
        )
}

/// The box `id`'s box sits in: its nearest ancestor that is not `display:
/// contents` (row groups and rows of a table included, which generate no
/// box in rinch).
fn box_parent(tree: &NodeTree, id: RawNodeId) -> Option<RawNodeId> {
    let mut p = tree.nodes.get(id)?.parent?;
    while tree.nodes.get(p)?.computed_style.display == DisplayValue::Contents {
        p = tree.nodes[p].parent?;
    }
    Some(p)
}

/// Lay a table cell whose parent box is an inline out as the anonymous
/// `inline-table` CSS wraps it in (CSS 2.1 §17.2.1, CSS Tables 3 §3.4.1:
/// the anonymous table is an `inline-table` when its parent is an inline
/// box), so it sits on the line: mail puts `<span style="display:
/// table-cell">` counts inside a link beside an icon. rinch has no anonymous
/// table box, so the cell is an `inline-block` itself, as wide as its
/// content, as a lone cell's table is. The table's `vertical-align` is
/// `baseline`, and the cell's own says where the table's baseline is
/// ([`ComputedStyle::anonymous_inline_table`],
/// `ifc::atomic_inline_baseline`). Rows and row groups generate no box, so a
/// cell in a row in an inline is one too.
///
/// Run on `style`, `id`'s new computed style, as it is cascaded; its parent
/// is cascaded first. A parent whose display crosses `inline` has its cells
/// cascaded again ([`inline_table_cells_under`]).
///
/// Not modelled: consecutive cells share one anonymous table and row in
/// CSS, so they are as tall as the tallest; here each is its own box, on the
/// line's baseline, spaced as one table's cells are (the white space between them is dropped, as CSS drops it:
/// `ifc`'s `is_blank_between_inline_cells`). A row's children that are not
/// cells flow as inline content, where CSS wraps them in an anonymous cell.
///
/// [`ComputedStyle::anonymous_inline_table`]: crate::computed_style::ComputedStyle::anonymous_inline_table
pub(crate) fn wrap_cell_in_inline_table(
    tree: &NodeTree,
    id: RawNodeId,
    style: &mut crate::computed_style::ComputedStyle,
) {
    if style.table_part != TablePart::Cell || style.display != DisplayValue::Block {
        return;
    }
    let Some(parent) = box_parent(tree, id) else {
        return;
    };
    let p = &tree.nodes[parent];
    if p.computed_style.display != DisplayValue::Inline
        || p.tag()
            .is_some_and(crate::node::is_atomic_at_display_inline)
    {
        return;
    }
    style.anonymous_inline_table = Some(style.vertical_align);
    style.vertical_align = VerticalAlignValue::Baseline;
    style.display = DisplayValue::InlineBlock;
    // The table's `border-spacing`, which it inherits (2px under a `<table>`
    // with no `cellspacing`), is its padding in the separated model; a cell
    // takes no margin of its own. Above and below the cell it is a margin,
    // which its line lines up (`ifc::atomic_vertical_margins`). Across, the
    // IFC puts it beside the cell (`inline_table_spacing`), because an
    // atomic inline's side margins take no room on its line (#1405).
    // Chrome 157: `border-spacing: 10px` puts the cell 10px in, the line 40.
    let v = inline_table_spacing(style).1;
    use crate::computed_style::LengthPercentageAutoValue::Length;
    style.margin_top = Length(v);
    style.margin_bottom = Length(v);
    style.margin_left = Length(0.0);
    style.margin_right = Length(0.0);
}

/// The `border-spacing` of the anonymous inline-table cell `style` stands
/// in ([`wrap_cell_in_inline_table`]), across and down: none under
/// `border-collapse: collapse`.
pub(crate) fn inline_table_spacing(style: &crate::computed_style::ComputedStyle) -> (f32, f32) {
    if style.border_collapse {
        (0.0, 0.0)
    } else {
        style.border_spacing
    }
}

/// Whether the sibling before `id`, past comments and white space, is a cell
/// standing in an anonymous inline-table: then both are in one table, and
/// `id` has no spacing before it, only the one between them.
pub(crate) fn follows_inline_cell_in(nodes: &crate::node_slab::NodeSlab, id: RawNodeId) -> bool {
    let Some(siblings) = nodes
        .get(id)
        .and_then(|n| n.parent)
        .and_then(|p| nodes.get(p))
        .map(|p| p.children.as_slice())
    else {
        return false;
    };
    let Some(at) = siblings.iter().position(|&c| c == id) else {
        return false;
    };
    siblings[..at]
        .iter()
        .rev()
        .filter_map(|&c| nodes.get(c))
        .find(|n| !n.is_comment() && !crate::ifc::is_white_space_text(n))
        .is_some_and(|n| n.is_element() && n.computed_style.anonymous_inline_table.is_some())
}

/// The cells whose box parent is `id` ([`wrap_cell_in_inline_table`]):
/// cascaded again when `id`'s display crosses `inline`, which decides what
/// they are.
pub(crate) fn inline_table_cells_under(tree: &NodeTree, id: RawNodeId, out: &mut Vec<RawNodeId>) {
    for &c in &tree.nodes[id].children {
        let Some(n) = tree.nodes.get(c) else { continue };
        if !n.is_element() {
            continue;
        }
        if n.computed_style.display == DisplayValue::Contents {
            inline_table_cells_under(tree, c, out);
        } else if n.computed_style.table_part == TablePart::Cell {
            out.push(c);
        }
    }
}

/// Whether `id` is a block or inline-block whose in-flow children, seen through
/// `display: contents` wrappers, are all rows, row groups or cells, at least
/// one, with no text that shows. Stops at the first child that says no.
fn qualifies_as_host(tree: &NodeTree, id: RawNodeId) -> bool {
    let cs = &tree.nodes[id].computed_style;
    if !matches!(
        cs.display,
        DisplayValue::Block | DisplayValue::ListItem | DisplayValue::InlineBlock
    ) || !matches!(cs.table_part, TablePart::None | TablePart::Table)
    {
        return false;
    }
    let mut any = false;
    all_row_parts(tree, id, &mut any) && any
}

fn all_row_parts(tree: &NodeTree, id: RawNodeId, any: &mut bool) -> bool {
    for &c in &tree.nodes[id].children {
        let n = &tree.nodes[c];
        if !n.is_element() {
            if n.text_content()
                .is_some_and(|t| !t.chars().all(char::is_whitespace))
            {
                return false;
            }
            continue;
        }
        let cs = &n.computed_style;
        if cs.display == DisplayValue::None
            || matches!(cs.position, PositionValue::Absolute | PositionValue::Fixed)
            || cs.table_part == TablePart::Other
        {
            continue;
        }
        if cs.display == DisplayValue::Contents && cs.table_part == TablePart::None {
            if !all_row_parts(tree, c, any) {
                return false;
            }
            continue;
        }
        if !is_row_part(tree, c) {
            return false;
        }
        *any = true;
    }
    true
}

/// A row's rows: itself, or, when none of its children is a table cell, one
/// row for each child, in the first column. CSS wraps a row's consecutive
/// children that are not cells in one anonymous cell, in the first column, so
/// they stack there; HTML email's phone layouts rely on it (`<th
/// style="display: block !important; width: 100% !important">` columns that
/// stack under 600px, in a table of one row, so the first column is all of
/// it). rinch has no anonymous cell box; a row of its own per child, in that
/// column, draws the same. (A row mixing cells and blocks keeps each in a
/// column of its own.)
fn row_of(tree: &NodeTree, row: RawNodeId, group: Option<RawNodeId>) -> Vec<Row> {
    let mut kids = Vec::new();
    flat_children(tree, row, &mut kids);
    kids.retain(|&c| {
        let cs = &tree.nodes[c].computed_style;
        cs.position != PositionValue::Absolute
            && cs.position != PositionValue::Fixed
            && cs.table_part != TablePart::Other
    });
    let blocks = !kids.is_empty()
        && kids
            .iter()
            .all(|&c| tree.nodes[c].computed_style.table_part != TablePart::Cell);
    if blocks && kids.len() > 1 {
        return kids
            .into_iter()
            .map(|c| Row {
                id: Some(row),
                group,
                cells: vec![c],
            })
            .collect();
    }
    vec![Row {
        id: Some(row),
        group,
        cells: kids,
    }]
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
            TablePart::Row => s.body.extend(row_of(tree, c, None)),
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
                        list.extend(row_of(tree, r, Some(c)));
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
/// border box when it declares a content box. A cell's is capped at its
/// `max-width` (#2264; Blink's `CreateCellInlineConstraint`): `width: 300px;
/// max-width: 120px` asks 120 of the column, in either table layout, and
/// with `min-width: 200px` as well, 200.
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
            // `min-width` wins over `max-width`, as everywhere in CSS.
            let w = match (cs.max_width, cs.min_width) {
                (DimensionValue::Length(m), min) if cs.table_part == TablePart::Cell => {
                    let w = w.min(m);
                    match min {
                        DimensionValue::Length(n) => w.max(n),
                        _ => w,
                    }
                }
                _ => w,
            };
            Track::Px(w + extra)
        }
        DimensionValue::Percent(p) => Track::Percent(p),
        _ => Track::Auto,
    }
}

/// The horizontal margins of a block standing in an anonymous cell (a `<td
/// style="display: block">`), which are inside that cell and part of what it
/// asks of its column (#2288). A real cell has none.
fn anon_cell_margins(tree: &NodeTree, id: RawNodeId) -> f32 {
    let cs = &tree.nodes[id].computed_style;
    if cs.table_part == TablePart::Cell {
        return 0.0;
    }
    let px = |m: &crate::computed_style::values::LengthPercentageAutoValue| match m {
        crate::computed_style::values::LengthPercentageAutoValue::Length(v) => *v,
        _ => 0.0,
    };
    px(&cs.margin_left) + px(&cs.margin_right)
}

/// A real cell's declared length width, as its column reads it.
fn declared_px(tree: &NodeTree, id: RawNodeId) -> Option<f32> {
    if tree.nodes[id].computed_style.table_part != TablePart::Cell {
        return None;
    }
    match declared_track(tree, id) {
        Track::Px(p) => Some(p),
        _ => None,
    }
}

/// Percentage columns adding up past 100% in an automatic-layout table
/// (#2334): each, in column order, gets at most what the earlier ones left of
/// 100%, so a later one can be left with `0%` (Chrome 156: `70%` and `60%`
/// resolve as 70 and 30). A fixed table is left as it is: Blink shares one's
/// width otherwise, which rinch does not model.
fn cap_percent_tracks(tracks: &mut [Track]) {
    let mut left = 1.0f32;
    for t in tracks {
        if let Track::Percent(p) = t {
            let q = p.max(0.0).min(left);
            left -= q;
            *p = q;
        }
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

    /// A block or inline-block whose in-flow children are all rows, row
    /// groups or cells (and no text) is laid out as the table CSS wraps them
    /// in (CSS 2.1 §17.2.1, "anonymous table objects"): email sets
    /// `display: block` on a `<table>` for phones, and `display:
    /// inline-block` on a `<table>` or a `<tr>` to centre a row of buttons
    /// or icons, and its cells stay side by side in Chrome. rinch has no
    /// anonymous box, so the block takes the table's part itself
    /// ([`is_host`]): it keeps its own width, as Chrome's block does, and its
    /// columns pack at its start as the anonymous table's would
    /// (`apply_table_style`).
    ///
    /// Not modelled: a block that mixes table parts with other content,
    /// which CSS wraps run by run; it lays out as before (each part where
    /// its own display puts it).
    ///
    /// Called as each node is cascaded. A block is marked when it is styled
    /// with its rows already styled, or when a row is styled under it; it
    /// gives the part up ([`Self::unhost`]) when it is restyled and no longer
    /// qualifies, and a rebuild checks again ([`Self::resolve_tables`]), so a
    /// row removed, added or restyled is seen too.
    pub(crate) fn note_table_host(&mut self, node_id: RawNodeId) {
        let cs = &self.tree.nodes[node_id].computed_style;
        if cs.table_part == TablePart::None {
            if qualifies_as_host(&self.tree, node_id) {
                self.tree.nodes[node_id].computed_style.table_part = TablePart::Table;
                self.owe_table(node_id);
                // The table it stands in, if any, now holds it as a cell.
                if let Some(p) = self.tree.nodes[node_id].parent {
                    self.note_table_children_changed(p);
                }
            } else if self.tree.nodes[node_id].table_box.is_some() {
                self.unhost(node_id);
            }
            return;
        }
        if !is_row_part(&self.tree, node_id) {
            return;
        }
        let mut id = node_id;
        let block = loop {
            let Some(p) = self.tree.nodes[id].parent else {
                return;
            };
            if self.tree.nodes[p].computed_style.display != DisplayValue::Contents {
                break p;
            }
            id = p;
        };
        let part = self.tree.nodes[block].computed_style.table_part;
        if part == TablePart::None && qualifies_as_host(&self.tree, block) {
            self.tree.nodes[block].computed_style.table_part = TablePart::Table;
            self.owe_table(block);
        } else if is_host(&self.tree.nodes[block]) {
            self.owe_table(block);
        }
    }

    fn owe_table(&mut self, table: RawNodeId) {
        if self.tree.tables_owed.last() != Some(&table) {
            self.tree.tables_owed.push(table);
        }
        self.tree.layout_dirty = true;
    }

    /// A block that stood for a table ([`Self::note_table_host`]) and no
    /// longer does: its part, its table box and its cells' places go, and
    /// each takes the Taffy style of its own computed values again.
    fn unhost(&mut self, host: RawNodeId) {
        let node = &mut self.tree.nodes[host];
        if node.computed_style.table_part == TablePart::Table
            && matches!(
                node.computed_style.display,
                DisplayValue::Block | DisplayValue::ListItem | DisplayValue::InlineBlock
            )
        {
            node.computed_style.table_part = TablePart::None;
        }
        let placed = node.table_box.take().map(|b| b.placed).unwrap_or_default();
        self.tree.tables.retain(|&t| t != host);
        self.show_blank_runs(host);
        for id in placed.into_iter().chain([host]) {
            let Some(n) = self.tree.nodes.get_mut(id) else {
                continue;
            };
            n.table_cell = None;
            self.rewrite_taffy_style(id);
        }
        self.tree.layout_dirty = true;
        if let Some(p) = self.tree.nodes[host].parent {
            self.note_table_children_changed(p);
        }
    }

    /// Owe `parent`'s table a rebuild after its child list changed: neither a
    /// removal nor a move of an already-styled row restyles anything that
    /// would say so. Consecutive changes under one table are pushed once.
    ///
    /// A plain block is owed a look too: one that lost its last child that is
    /// not a row, or whose text went, may stand for a table now
    /// ([`Self::note_table_host`]); [`Self::resolve_tables`] checks.
    pub(crate) fn note_table_children_changed(&mut self, parent: RawNodeId) {
        let owed = Self::table_of(&self.tree, parent).or_else(|| {
            let cs = &self.tree.nodes.get_live(parent)?.computed_style;
            (cs.table_part == TablePart::None
                && matches!(
                    cs.display,
                    DisplayValue::Block | DisplayValue::ListItem | DisplayValue::InlineBlock
                ))
            .then_some(parent)
        });
        if let Some(table) = owed
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
            if !self.tree.contains(id) {
                continue;
            }
            if is_host(&self.tree.nodes[id]) && !qualifies_as_host(&self.tree, id) {
                self.unhost(id);
                continue;
            }
            // A plain block owed a look (`note_table_children_changed`).
            if self.tree.nodes[id].computed_style.table_part == TablePart::None {
                if !qualifies_as_host(&self.tree, id) {
                    continue;
                }
                self.tree.nodes[id].computed_style.table_part = TablePart::Table;
                if let Some(p) = self.tree.nodes[id].parent {
                    self.note_table_children_changed(p);
                }
            }
            if self.tree.nodes[id].computed_style.table_part == TablePart::Table {
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
        // A row's `height` is the least its track is (*Row heights*). A
        // percentage sizes nothing. (A row of blocks is a row of anonymous
        // cells since #2288, so its height applies like any other row's.)
        let row_heights: Vec<Option<f32>> = rows
            .iter()
            .map(|r| match r.id {
                Some(id) => match self.tree.nodes[id].computed_style.height {
                    DimensionValue::Length(h) if h >= 0.0 => Some(h),
                    _ => None,
                },
                None => None,
            })
            .collect();

        // Sections: a new one at each change of row group, and between the
        // header, body and footer.
        let mut sections = Vec::with_capacity(rows.len());
        let mut section = 0u16;
        for (y, row) in rows.iter().enumerate() {
            let part = |y: usize| match y {
                y if y < s.head.len() => 0,
                y if y < s.head.len() + s.body.len() => 1,
                _ => 2,
            };
            if y > 0 && (row.group != rows[y - 1].group || part(y) != part(y - 1)) {
                section = section.saturating_add(1);
            }
            sections.push((section, part(y) == 1));
        }

        let cs = &self.tree.nodes[table].computed_style;
        // A host's own width is its block's: the anonymous table in it is
        // as wide as its content.
        let definite = !is_host(&self.tree.nodes[table])
            && !matches!(
                cs.width,
                DimensionValue::Auto | DimensionValue::Intrinsic(_)
            );
        let fixed = cs.table_layout_fixed && definite;
        let collapse = cs.border_collapse;
        let spacing = if collapse {
            (0.0, 0.0)
        } else {
            let scale = self.tree.device_scale;
            (
                whole_device_spacing(cs.border_spacing.0, scale),
                whole_device_spacing(cs.border_spacing.1, scale),
            )
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
        // A block in a row (a `<td style="display: block">`, stacked or
        // beside real cells) stands in an anonymous cell, whose width is
        // auto: the block's `width` is its own, inside that cell, and does
        // not declare the column (#2288; Chrome gives a wide table's free
        // width to that auto column, not to the declared ones).
        for (cell, slot, y) in &placed {
            let real = self.tree.nodes[*cell].computed_style.table_part == TablePart::Cell;
            if slot.colspan == 1 && (!fixed || *y == 0) && real {
                let i = slot.col as usize - 1;
                tracks[i] = merge_track(tracks[i], declared_track(&self.tree, *cell));
            }
        }
        if !fixed {
            cap_percent_tracks(&mut tracks);
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
        // A caption is outside the table box (CSS 2.2 §17.4), and so outside
        // its border and the padding that carries the outer spacing: its
        // margins take those back, on its outer side and across, and give
        // them to its inner side, where they separate the caption from the
        // rows (the fork leaves the gutters beside a caption row empty). The
        // same padding as [`Self::apply_table_style`] writes, and the border
        // the table has (none when it collapses its borders onto its cells),
        // which paint leaves off the captions ([`caption_bands`], #2394). A
        // block standing for an anonymous table keeps its border round its
        // captions: the table is inside it.
        let pads = {
            let cs = &self.tree.nodes[table].computed_style;
            let host = is_host(&self.tree.nodes[table]);
            let pad = |p: LengthPercentageValue, b: LengthPercentageValue, spacing: f32| {
                if host {
                    p.to_px() + spacing
                } else if collapse {
                    b.to_px()
                } else {
                    p.to_px() + b.to_px() + spacing
                }
            };
            [
                pad(cs.padding_top, cs.border_top_width, spacing.1),
                pad(cs.padding_right, cs.border_right_width, spacing.0),
                pad(cs.padding_bottom, cs.border_bottom_width, spacing.1),
                pad(cs.padding_left, cs.border_left_width, spacing.0),
            ]
        };
        let top_count = s.top_captions.len();
        for (i, (c, y)) in captions.into_iter().enumerate() {
            let [top, right, bottom, left] = pads;
            let outside = if i < top_count {
                [-top, -right, top, -left]
            } else {
                [bottom, -right, -bottom, -left]
            };
            let t = TableCell {
                row: y as i16,
                col: 0,
                rowspan: 1,
                colspan: (cols.max(1) + 1) as u16,
                outside,
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
        // A declared column's weight reads its width (#2288), which never
        // reaches a cell's Taffy style: tracks that moved owe a weighing.
        if prev.as_ref().is_some_and(|p| p.tracks != tracks) {
            self.tree.tables_owed_columns.push(table);
        }
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
            collapse,
            first_row: first_line,
            row_heights,
            sections,
            bottom_captions: s.bottom_captions.len(),
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
            slot.min_content = old.min_content;
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
        // A cell in an anonymous inline-table's place: its `vertical-align`
        // padding is the computed style's, never its box's (#2514).
        if let Some(a) = &node.anon_cell
            && a.shift != 0.0
        {
            style.padding.top =
                length((node.computed_style.padding_top.to_px() - a.shift).max(0.0));
        }
        if let Some(b) = &node.table_box {
            // A block that hosts table parts ([`Self::note_table_host`]):
            // a grid of its own width, its tracks packed at its start as the
            // anonymous table's columns and rows would be.
            let host = is_host(node);
            if host {
                style.display = taffy::Display::Grid;
                style.justify_content = Some(taffy::JustifyContent::START);
                style.align_content = Some(taffy::AlignContent::START);
            }
            // An automatic-layout table shares its room between its tracks
            // as a table shares it between columns, not in a grid's equal
            // shares (#2280, #2306, *Column widths*), and one with a width
            // of its own widens to its columns' minimum (#2266), which needs
            // it to resolve its own width rather than be handed one by its
            // block parent (Taffy's `item_is_table`).
            style.proportional_column_sizing = !b.fixed;
            let auto_with_width = b.definite && !b.fixed;
            style.item_is_table = auto_with_width && !host;
            // A fixed table whose width is a percentage asks for all the
            // room it is offered: its max-content width is Blink's cap on a
            // table's width (the Taffy fork's `fill_max_content`), so the
            // auto table, float or shrink-to-fit box around it fills its
            // room, and the percentage then resolves against that (#2322).
            // (A mixed `calc()` reaches Taffy as a resolved length, so it
            // does not fill: Chrome fills for one too.)
            style.fill_max_content =
                b.fixed && matches!(node.computed_style.width, DimensionValue::Percent(_));
            style.grid_template_columns = b
                .tracks
                .iter()
                .map(|t| {
                    taffy::GridTemplateComponent::Single(match *t {
                        Track::Auto if b.fixed => fr(1.0),
                        // Its cells' minimum, and their max-content as the
                        // limit it grows to.
                        Track::Auto if auto_with_width => {
                            minmax(min_content(), taffy::style_helpers::auto())
                        }
                        Track::Auto => taffy::style_helpers::auto(),
                        Track::Px(p) if b.fixed => length(p),
                        // From its minimum up to its width, or past it to
                        // a cell's min-content (Taffy floors the limit at the
                        // base), never by another cell's max-content: Chrome
                        // 156 shares a table's excess by declared widths.
                        Track::Px(p) => minmax(min_content(), length(p)),
                        Track::Percent(p) if b.definite => minmax(min_content(), percent(p)),
                        Track::Percent(_) => taffy::style_helpers::auto(),
                    })
                })
                .collect();
            // The fork sizes the rows as Blink sizes a table's (*Row
            // heights*, Taffy's `table_rows`): it reads a row's declared
            // height as its track's minimum, `minmax(<height>, auto)`, and
            // which rows are captions and which section each row is in.
            style.grid_template_rows = match b.row_heights.iter().rposition(Option::is_some) {
                None => Vec::new(),
                Some(last) => (1..b.first_row)
                    .map(|_| None)
                    .chain(b.row_heights[..=last].iter().copied())
                    .map(|h| {
                        taffy::GridTemplateComponent::Single(match h {
                            Some(h) => minmax(length(h), taffy::style_helpers::auto()),
                            None => taffy::style_helpers::auto(),
                        })
                    })
                    .collect(),
            };
            let caption = taffy::TableRow {
                caption: true,
                section: 0,
                body: false,
            };
            style.table_rows = Some(taffy::TableRows {
                rows: (1..b.first_row)
                    .map(|_| caption)
                    .chain(b.sections.iter().map(|&(section, body)| taffy::TableRow {
                        caption: false,
                        section,
                        body,
                    }))
                    .chain((0..b.bottom_captions).map(|_| caption))
                    .collect(),
                // A block standing for an anonymous table keeps its own
                // height: the table in it is as tall as its rows.
                sized: !host,
            });
            style.gap = taffy::Size {
                width: length(b.spacing.0),
                height: length(b.spacing.1),
            };
            // A collapsing table has no padding, and no spacing either. (A
            // block standing for one keeps its own: the table is inside it.)
            let cs = &node.computed_style;
            let pad = |p: crate::computed_style::LengthPercentageValue, spacing: f32| {
                if b.collapse && !host {
                    0.0
                } else {
                    p.to_px() + spacing
                }
            };
            style.padding = taffy::Rect {
                top: length(pad(cs.padding_top, b.spacing.1)),
                bottom: length(pad(cs.padding_bottom, b.spacing.1)),
                left: length(pad(cs.padding_left, b.spacing.0)),
                right: length(pad(cs.padding_right, b.spacing.0)),
            };
            if !host && !b.definite && matches!(node.computed_style.width, DimensionValue::Auto) {
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
            // A cell's `max-width` caps what it asks of its column (its
            // min-content, max-content and `width`, [`declared_track`]), and
            // the cell is laid out at its column's width, past it (#2264;
            // Blink: `CreateCellInlineConstraint`,
            // `SetupTableCellConstraintSpaceBuilder`). Taffy keeps `max_size`
            // for the contributions and lets the stretch pass it. Only a
            // `display: table-cell` box: a `<td style="display: block">` (or
            // `flex`, …) is a block in an anonymous cell, which its
            // `max-width` clamps as any block's (Chrome 156: 90 in a 400px
            // table; email stacks its columns this way). Written on every
            // sync from the part alone, so no restyle strands it.
            let cell = node.computed_style.table_part != TablePart::Caption;
            let real = node.computed_style.table_part == TablePart::Cell;
            style.stretch_past_max_width = real;
            if cell {
                // A cell's `height` is a minimum, and its `width` is its
                // column's. It fills its slot and has no margins. A block
                // standing in an anonymous cell keeps its own size and
                // margins, inside the cell (#2288): its `width` is what it
                // asks of the column, which is `auto` ([`declared_track`] is
                // not read for it).
                if real {
                    if let DimensionValue::Length(h) = node.computed_style.height {
                        style.min_size.height = length(h);
                    }
                    style.size = taffy::Size::auto();
                    style.margin = taffy::Rect::length(0.0);
                }
                // A cell is as tall as its row. A block in an anonymous
                // cell is as tall as its content (Chrome: a 10px block in a
                // `height: 40px` row is 10 tall, #2312), at the row's top;
                // the anonymous cell's `vertical-align` is `baseline`, which
                // moves it down to the row's baseline after the compute
                // (`align_row_baselines`, #2359).
                style.align_self = Some(if real {
                    taffy::AlignSelf::STRETCH
                } else {
                    taffy::AlignSelf::START
                });
                style.justify_self = Some(taffy::JustifySelf::STRETCH);
                if c.shift != 0.0 {
                    style.padding.top =
                        length((node.computed_style.padding_top.to_px() - c.shift).max(0.0));
                }
            } else if c.outside != [0.0; 4] {
                // A caption, out of the table's padding (#2350). A margin
                // that needs the table's width (a percentage, a `calc()`, an
                // `auto` beside a `width`) is seeded here and resolved after
                // each compute (`patch_caption_margins`).
                let [top, right, bottom, left] =
                    caption_margins(&node.computed_style, c.outside, None);
                style.margin = taffy::Rect {
                    top: taffy::LengthPercentageAuto::length(top),
                    right: taffy::LengthPercentageAuto::length(right),
                    bottom: taffy::LengthPercentageAuto::length(bottom),
                    left: taffy::LengthPercentageAuto::length(left),
                };
            }
        }
    }

    /// Whether caption `node`'s margins wait on a compute: one is a
    /// percentage or a `calc()`, which resolve against the table's width, or
    /// a side margin is `auto` beside a `width`, which shares the room the
    /// table leaves it. [`Self::patch_caption_margins`] resolves them.
    pub(crate) fn caption_margins_need_layout(node: &crate::node::Node) -> bool {
        use crate::computed_style::values::LengthPercentageAutoValue as M;
        let Some(c) = &node.table_cell else {
            return false;
        };
        if c.outside == [0.0; 4] || node.computed_style.table_part != TablePart::Caption {
            return false;
        }
        let cs = &node.computed_style;
        let relative = |m: M| matches!(m, M::Percent(_) | M::Calc { .. });
        relative(cs.margin_top)
            || relative(cs.margin_right)
            || relative(cs.margin_bottom)
            || relative(cs.margin_left)
            || (caption_is_sized(cs)
                && (matches!(cs.margin_left, M::Auto) || matches!(cs.margin_right, M::Auto)))
    }

    /// Write caption `id`'s margins into its Taffy style `ts`, resolved
    /// against the table's border-box width from the last compute (CSS: a
    /// caption's containing block is the table wrapper, as wide as the
    /// table). A percentage side margin resolves only when the table has a
    /// width of its own; in a table as wide as its content it would feed its
    /// own basis, and it is taken as zero (Chrome resolves it against the
    /// final width). A top or bottom one always resolves: it sizes no width.
    pub(crate) fn patch_caption_margins(
        &self,
        id: RawNodeId,
        taffy_id: taffy::NodeId,
        ts: &mut taffy::Style,
    ) {
        let node = &self.tree.nodes[id];
        let Some(c) = &node.table_cell else {
            return;
        };
        let Some(table) = self.tree.taffy.parent(taffy_id) else {
            return;
        };
        let (Ok(t), Ok(own)) = (
            self.tree.taffy.layout(table),
            self.tree.taffy.layout(taffy_id),
        ) else {
            return;
        };
        let definite = node
            .parent
            .and_then(|p| Self::table_of(&self.tree, p))
            .and_then(|t| self.tree.nodes[t].table_box.as_ref())
            .is_some_and(|b| b.definite);
        let basis = CaptionBasis {
            full: t.size.width,
            percent: definite.then_some(t.size.width),
            // A caption is outside the table's border too (#2394).
            table: t.size.width,
            caption: own.size.width,
        };
        let [top, right, bottom, left] =
            caption_margins(&node.computed_style, c.outside, Some(basis));
        ts.margin = taffy::Rect {
            top: taffy::LengthPercentageAuto::length(top),
            right: taffy::LengthPercentageAuto::length(right),
            bottom: taffy::LengthPercentageAuto::length(bottom),
            left: taffy::LengthPercentageAuto::length(left),
        };
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

    /// Note, before any compute of a layout, which tables changed and which of
    /// their cells: a changed cell's remembered max-content width is dropped
    /// (`TableCell::max_content`), and a changed table is owed a column
    /// measure (`NodeTree::tables_owed_columns`). Both used to be read off
    /// Taffy's dirty flags where the measure runs, after an atomic inline's own
    /// compute had laid out a table inside it (#2156), and after the computes
    /// of the layouts in which the table had no width of its own (review of
    /// #2153) — each clears them.
    pub(crate) fn note_changed_tables(&mut self) {
        for i in 0..self.tree.tables.len() {
            let id = self.tree.tables[i];
            let Some(n) = self.tree.nodes.get_live(id) else {
                continue;
            };
            let (Some(t), Some(b)) = (n.taffy_id, n.table_box.as_ref()) else {
                continue;
            };
            if !self.tree.taffy.dirty(t).unwrap_or(false) {
                continue;
            }
            for c in b.placed.clone() {
                let Some(tid) = self.tree.nodes.get_live(c).and_then(|n| n.taffy_id) else {
                    continue;
                };
                if self.tree.taffy.dirty(tid).unwrap_or(true)
                    && let Some(cell) = self.tree.nodes[c].table_cell.as_mut()
                {
                    cell.max_content = None;
                    cell.min_content = None;
                }
            }
            self.tree.tables_owed_columns.push(id);
        }
    }

    /// Measure what the cells of every table with a width of its own ask of
    /// their columns, whose subtree changed since. Its columns were weighed
    /// by these before #2306; nothing reads them now (*Column widths*).
    /// Innermost first, because an outer table's max-content reads an inner
    /// one's.
    pub(crate) fn measure_table_columns(&mut self) {
        let owed = std::mem::take(&mut self.tree.tables_owed_columns);
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
                    // Nothing reads these since #2306 (*Column widths*).
                    .is_some_and(|b| {
                        b.definite
                            && !b.fixed
                            && b.tracks.iter().any(|t| !matches!(t, Track::Percent(_)))
                    })
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
        // Tables whose columns moved.
        let mut moved = Vec::new();
        for (_, id) in tables {
            let Some(t) = self.tree.nodes[id].taffy_id else {
                continue;
            };
            if !owed.contains(&id) && !self.tree.taffy.dirty(t).unwrap_or(false) {
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
                        moved.push(id);
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
            moved.push(id);
        }
        // A table inside an atomic inline was laid out by that box's own
        // compute, which ran before this and which the root compute does not
        // reach: lay it out again at the widths just measured (#2156).
        for &id in &moved {
            self.mark_atomic_inline_dirty(id);
        }
        self.remeasure_dirty_atomic_inlines();
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
                let c = self.tree.nodes[id].table_cell.as_ref();
                c.is_none_or(|c| c.max_content.is_none())
                    || (c.is_some_and(|c| c.min_content.is_none())
                        && declared_px(&self.tree, id).is_some())
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
            // A cell with a `width` of its own is asked its min-content too
            // (see the weighing below).
            let declared: Vec<(RawNodeId, taffy::NodeId)> = ask
                .iter()
                .copied()
                .filter(|&(id, _)| declared_px(&self.tree, id).is_some())
                .collect();
            let mins = if declared.is_empty() {
                Vec::new()
            } else {
                let tids: Vec<taffy::NodeId> = declared.iter().map(|&(_, tid)| tid).collect();
                self.measure_taffy_sizes(
                    &tids,
                    taffy::Size {
                        width: taffy::AvailableSpace::MinContent,
                        height: taffy::AvailableSpace::MaxContent,
                    },
                )
            };
            for ((id, _), size) in ask.into_iter().zip(sizes) {
                if let Some(c) = self.tree.nodes[id].table_cell.as_mut() {
                    // Rounded, as the detached layout's boxes are.
                    c.max_content = Some(size.width.round());
                }
            }
            for ((id, _), size) in declared.into_iter().zip(mins) {
                if let Some(c) = self.tree.nodes[id].table_cell.as_mut() {
                    c.min_content = Some(size.width.round());
                }
            }
        }
        // What each cell asks of its column's weight (#2288; Blink's
        // `ComputeColumnInlineConstraints`): a cell with a `width` of its own
        // the larger of that width and its min-content, so a declared column
        // whose content cannot fit is weighed by the content and a picture
        // that scales into it (min-content 0) is not; any other cell its
        // max-content, a block in an anonymous cell with its side margins.
        let mut weights = vec![0.0f32; cols];
        for (id, _, col) in cells {
            let c = self.tree.nodes[id].table_cell.as_ref();
            let w = match declared_px(&self.tree, id) {
                Some(p) => p.max(c.and_then(|c| c.min_content).unwrap_or(0.0)),
                None => {
                    c.and_then(|c| c.max_content).unwrap_or(0.0) + anon_cell_margins(&self.tree, id)
                }
            };
            weights[col] = weights[col].max(w);
        }
        Some(weights)
    }

    /// The white space between a table's rows and cells generates no box
    /// (css-tables-3 §3.9). The IFC pass mints an anonymous block box around
    /// it, as around any run in a grid, which would be a stray grid item.
    /// Undo [`Self::hide_blank_runs`] for a block that stops standing for a
    /// table: its text runs are anonymous blocks again, shown whatever they
    /// hold (one blank then is no longer blank).
    fn show_blank_runs(&mut self, table: RawNodeId) {
        let boxes: Vec<RawNodeId> = self.tree.nodes[table].run_boxes.clone();
        for b in boxes {
            let Some(t) = self.tree.nodes.get_live(b).and_then(|n| n.taffy_id) else {
                continue;
            };
            if let Ok(old) = self.tree.taffy.style(t).cloned()
                && old.display == taffy::Display::None
            {
                let mut st = old;
                st.display = taffy::Display::Block;
                let _ = self.tree.taffy.set_style(t, st);
                self.tree.layout_dirty = true;
            }
        }
    }

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
                // A cell with a `width` of its own asks for that width here:
                // this layout has its max-content, not its min-content.
                let w = match declared_px(&self.tree, c) {
                    Some(p) => p,
                    None => {
                        self.tree.taffy.layout(n.taffy_id?).ok()?.size.width
                            + anon_cell_margins(&self.tree, c)
                    }
                };
                Some((slot.col, slot.colspan, w))
            })
            .collect()
    }

    /// Take back a cell's `vertical-align` as its box is read back: the read
    /// reset its children, and its height may have moved. The shift is applied
    /// again by [`Self::align_table_cells`], once the cell's text is laid out.
    pub(crate) fn note_table_cell_read(&mut self, id: RawNodeId) {
        let node = &mut self.tree.nodes[id];
        if node.table_cell.is_none()
            && node.anon_cell.is_none()
            && node.computed_style.anonymous_inline_table.is_some()
        {
            node.anon_cell = Some(Box::default());
        }
        let (shift, kids_shift) = match (node.table_cell.as_mut(), node.anon_cell.as_mut()) {
            (Some(c), _) => (&mut c.shift, &mut c.kids_shift),
            (None, Some(a)) => (&mut a.shift, &mut a.kids_shift),
            (None, None) => return,
        };
        if node.computed_style.table_part == TablePart::Caption {
            return;
        }
        *kids_shift = 0.0;
        let shift = std::mem::take(shift);
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
        let mut baseline_tables: Vec<RawNodeId> = Vec::new();
        for id in std::mem::take(&mut self.tree.cells_to_align) {
            if self.tree.contains(id)
                && let Some(table) = self.align_table_cell(id)
                && !baseline_tables.contains(&table)
            {
                baseline_tables.push(table);
            }
        }
        for table in baseline_tables {
            self.align_row_baselines(table);
        }
    }

    /// Whether `id`, a box placed in a table, is aligned by its row's
    /// baseline: a cell whose `vertical-align` is `baseline` (or any value
    /// but `top`, `middle` and `bottom`, which CSS 2.1 §17.5.4 applies as
    /// `baseline`), or a block standing in an anonymous cell (a `<td
    /// style="display: block">`), whose `vertical-align` is the initial
    /// `baseline`.
    fn aligns_by_row_baseline(node: &crate::node::Node) -> bool {
        if node.table_cell.is_none() || node.table_box.is_some() {
            return false;
        }
        match node.computed_style.table_part {
            TablePart::Caption => false,
            TablePart::Cell => !matches!(
                node.computed_style.vertical_align,
                VerticalAlignValue::Top | VerticalAlignValue::Middle | VerticalAlignValue::Bottom
            ),
            _ => true,
        }
    }

    /// Align the baseline-aligned cells of each row of `table` on the row's
    /// baseline (#2359; CSS 2.1 §17.5.3, Blink's `RowBaselineTabulator`): the
    /// lowest of their baselines from the row's top, each cell's content
    /// moved down to it. A cell's baseline is its first in-flow line's, or
    /// with none the bottom of its content; a block in an anonymous cell
    /// counts its top margin. A row spanned from above is aligned in the row
    /// its cell starts in.
    ///
    /// Not modelled: a row grows for the alignment (the Taffy fork's
    /// `table_rows` sizes rows from their cells' heights, not their
    /// baselines), so a cell moved down past its row's bottom overflows it.
    fn align_row_baselines(&mut self, table: RawNodeId) {
        let Some(placed) = self
            .tree
            .nodes
            .get(table)
            .and_then(|n| n.table_box.as_ref().map(|b| b.placed.clone()))
        else {
            return;
        };
        // (row, cell, its baseline from the row's top)
        let mut rows: Vec<(i16, RawNodeId, f32)> = Vec::new();
        for id in placed {
            let Some(node) = self.tree.nodes.get(id) else {
                continue;
            };
            if !Self::aligns_by_row_baseline(node) {
                continue;
            }
            let row = node.table_cell.as_ref().map_or(0, |c| c.row);
            if let Some(b) = self.row_baseline_of(id) {
                rows.push((row, id, b));
            }
        }
        rows.sort_by_key(|r| r.0);
        for group in rows.chunk_by(|a, b| a.0 == b.0) {
            let line = group.iter().map(|r| r.2).fold(f32::MIN, f32::max);
            for &(_, id, b) in group {
                let shift = (line - b).max(0.0);
                if self.tree.nodes[id].computed_style.table_part == TablePart::Cell {
                    self.set_cell_shift(id, shift);
                } else {
                    // A block in an anonymous cell is moved itself, as the
                    // anonymous cell's content (`TableCell::kids_shift`
                    // holds what its own `y` carries).
                    let node = &mut self.tree.nodes[id];
                    let cell = node.table_cell.as_mut().unwrap();
                    let delta = shift - cell.kids_shift;
                    if delta != 0.0 {
                        cell.kids_shift = shift;
                        node.layout.y += delta;
                        self.tree.paint_dirty_nodes.push(id);
                    }
                }
            }
        }
    }

    /// The baseline of `id`, a box [`Self::aligns_by_row_baseline`] says
    /// takes part, from its row's top, as if it were not moved yet.
    fn row_baseline_of(&self, id: RawNodeId) -> Option<f32> {
        let node = &self.tree.nodes[id];
        let cell = node.table_cell.as_ref()?;
        let cs = &node.computed_style;
        if cs.table_part == TablePart::Cell {
            // A cell starts at its row's top.
            let inset_top = cs.padding_top.to_px() - cell.shift + cs.border_top_width.to_px();
            let first = match &node.text_layout {
                Some(l) => Self::first_line_of(l).map(|b| inset_top + b),
                None => Self::first_baseline_below(&self.tree.nodes, id, cell.kids_shift, 0),
            };
            Some(first.unwrap_or_else(|| inset_top + self.cell_content_height(id)))
        } else {
            // A block in an anonymous cell: the cell starts at the row's top,
            // the block below its own top margin.
            let margin_top = match cs.margin_top {
                crate::computed_style::LengthPercentageAutoValue::Length(v) => v,
                _ => 0.0,
            };
            let own = Self::box_first_baseline(&self.tree.nodes, id, 0).unwrap_or(
                node.layout.height
                    + match cs.margin_bottom {
                        crate::computed_style::LengthPercentageAutoValue::Length(v) => v,
                        _ => 0.0,
                    },
            );
            Some(margin_top + own)
        }
    }

    /// The first line's baseline of an inline layout, from its top; `None`
    /// when it has no line box (CSS 2.1 §9.4.2: an IFC with no content).
    fn first_line_of(l: &crate::node::InlineLayout) -> Option<f32> {
        if l.layout
            .inline_boxes()
            .any(|b| b.id == crate::ifc::EMPTY_IFC_BOX)
        {
            return None;
        }
        crate::ifc::first_line_baseline(&l.layout)
    }

    /// The first in-flow baseline among the boxes Taffy places in `id`, from
    /// `id`'s border-box top, their `y` less the `offset` they carry.
    fn first_baseline_below(
        nodes: &crate::node_slab::NodeSlab,
        id: RawNodeId,
        offset: f32,
        depth: usize,
    ) -> Option<f32> {
        if depth > 64 {
            return None;
        }
        for &c in Self::box_tree_children(nodes, id).iter() {
            let Some(n) = nodes.get(c) else {
                continue;
            };
            let cs = &n.computed_style;
            if cs.float.is_floated()
                || matches!(cs.position, PositionValue::Absolute | PositionValue::Fixed)
                || cs.display == DisplayValue::None
            {
                continue;
            }
            if let Some(b) = Self::box_first_baseline(nodes, c, depth + 1) {
                return Some(n.layout.y - offset + b);
            }
        }
        None
    }

    /// A block box's first in-flow line's baseline, from its border-box top.
    fn box_first_baseline(
        nodes: &crate::node_slab::NodeSlab,
        id: RawNodeId,
        depth: usize,
    ) -> Option<f32> {
        let n = nodes.get(id)?;
        let cs = &n.computed_style;
        let top = cs.padding_top.to_px() + cs.border_top_width.to_px();
        match &n.text_layout {
            Some(l) => Self::first_line_of(l).map(|b| top + b),
            None => Self::first_baseline_below(nodes, id, 0.0, depth),
        }
    }

    /// The height of a cell's content as it lays out, before alignment: its
    /// lines (and the floats among them), or the boxes Taffy places in it.
    fn cell_content_height(&self, id: RawNodeId) -> f32 {
        let node = &self.tree.nodes[id];
        let (old, kids_old) = node
            .table_cell
            .as_ref()
            .map_or((0.0, 0.0), |c| (c.shift, c.kids_shift));
        let cs = &node.computed_style;
        let inset_top = cs.padding_top.to_px() - old + cs.border_top_width.to_px();
        match &node.text_layout {
            Some(l) => l.content_height(),
            None => {
                Self::box_tree_children(&self.tree.nodes, id)
                    .iter()
                    .map(|&c| {
                        let l = &self.tree.nodes[c].layout;
                        l.y - kids_old + l.height
                    })
                    .fold(0.0, f32::max)
                    - inset_top
            }
        }
        .max(0.0)
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
    ///
    /// A cell aligned by its row's baseline is left to
    /// [`Self::align_row_baselines`]: its table is returned.
    fn align_table_cell(&mut self, id: RawNodeId) -> Option<RawNodeId> {
        let node = &self.tree.nodes[id];
        // A table's cell, or one standing in an anonymous inline-table's
        // place (`wrap_cell_in_inline_table`), whose own `vertical-align`
        // aligns its content in its box as a table's cell's does (#2514).
        let (old, kids_old) = match (&node.table_cell, &node.anon_cell) {
            (Some(c), _) => (c.shift, c.kids_shift),
            (None, Some(a)) => (a.shift, a.kids_shift),
            (None, None) => return None,
        };
        if node.table_cell.is_none()
            && node.computed_style.anonymous_inline_table.is_none()
            && old == 0.0
            && kids_old == 0.0
        {
            self.tree.nodes[id].anon_cell = None;
            return None;
        }
        // A cell aligned by its row's baseline, or a block standing in an
        // anonymous cell (a `<td style="display: block">`), whose
        // `vertical-align` is the initial `baseline` whatever the `<td>`'s UA
        // `middle` says to the block (#2288, #2359).
        if Self::aligns_by_row_baseline(node) {
            return node.parent.and_then(|p| Self::table_of(&self.tree, p));
        }
        // A block that hosts a table in a cell's place ([`Self::note_table_host`])
        // is that table, which its cell is exactly as tall as. A caption is
        // not aligned. A box that stopped standing in an anonymous
        // inline-table aligns as `top`, which takes its shift back.
        if node.table_box.is_some()
            || (node.computed_style.table_part != TablePart::Cell && node.table_cell.is_some())
        {
            return None;
        }
        let align = match (&node.table_cell, node.computed_style.anonymous_inline_table) {
            (Some(_), _) => node.computed_style.vertical_align,
            (None, Some(a)) if node.computed_style.table_part == TablePart::Cell => a,
            (None, _) => VerticalAlignValue::Top,
        };
        let cs = &node.computed_style;
        let base_top = cs.padding_top.to_px() - old;
        let inset_top = base_top + cs.border_top_width.to_px();
        let inset_bottom = cs.padding_bottom.to_px() + cs.border_bottom_width.to_px();
        let avail = node.layout.height - inset_top - inset_bottom;
        let content = match &node.text_layout {
            // Its lines, and the floats among them: a cell is a block
            // formatting context root, so its content reaches the bottom of
            // its floats' margin boxes (CSS 2.2 §10.6.7). With the lines
            // alone, a cell whose text sits beside or below a float taller
            // than the text was aligned as if the float were not there, and
            // the float moved down by the shift (#2326: Chrome puts it at
            // the cell's top).
            // Its floats are those in its inline content, inside inline
            // elements too (`<span><div style="float:left">`): the IFC root's
            // own, laid out relative to it.
            Some(l) => {
                let nodes = &self.tree.nodes;
                let mut floats = 0.0f32;
                let mut stack: Vec<RawNodeId> = nodes[id].children.clone();
                while let Some(c) = stack.pop() {
                    let Some(n) = nodes.get(c) else {
                        continue;
                    };
                    if n.computed_style.float != taffy::Float::None {
                        floats = floats.max(
                            n.layout.y - kids_old
                                + n.layout.height
                                + n.computed_style.margin_bottom.to_px()
                                - inset_top,
                        );
                    } else if n.display_mode == crate::node::DisplayMode::Inline {
                        stack.extend(n.children.iter().copied());
                    }
                }
                l.content_height().max(floats)
            }
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
        self.set_cell_shift(id, shift);
        None
    }

    /// Move cell `id`'s content down by `shift` from where it lays out: as
    /// extra top padding (`TableCell::shift`), which moves everything its own
    /// inline formatting context positions, and an offset on the `y` of each
    /// child Taffy places (`TableCell::kids_shift`), each applied as the
    /// difference from what it carries. A percentage top padding takes none.
    fn set_cell_shift(&mut self, id: RawNodeId, shift: f32) {
        let node = &self.tree.nodes[id];
        let (old, kids_old) = match (&node.table_cell, &node.anon_cell) {
            (Some(c), _) => (c.shift, c.kids_shift),
            (None, Some(a)) => (a.shift, a.kids_shift),
            (None, None) => return,
        };
        let cs = &node.computed_style;
        let base_top = cs.padding_top.to_px() - old;
        let plain = matches!(
            cs.padding_top,
            LengthPercentageValue::Zero | LengthPercentageValue::Length(_)
        );
        let shift = if plain { shift } else { 0.0 };
        if shift == old && shift == kids_old {
            return;
        }
        let children = Self::box_tree_children(&self.tree.nodes, id).into_owned();
        let node = &mut self.tree.nodes[id];
        match (node.table_cell.as_mut(), node.anon_cell.as_mut()) {
            (Some(c), _) => (c.shift, c.kids_shift) = (shift, shift),
            (None, Some(a)) => (a.shift, a.kids_shift) = (shift, shift),
            (None, None) => unreachable!("checked above"),
        }
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
            // the padding above has moved it already. So is an inline
            // element, and moving it moved the boxes in it a second time (a
            // float inside a `<span>`, #2339 review).
            if child.ifc_root == Some(id)
                && (child.is_text()
                    || child.display_mode.is_atomic_inline()
                    || child.display_mode == crate::node::DisplayMode::Inline)
            {
                continue;
            }
            child.layout.y += delta;
        }
    }
}

/// How far table `node`'s captions reach into its box, above its rows and
/// below them, in CSS px: the bands its own background, border, shadows and
/// outline leave out, since a caption is outside the table box (CSS 2.2
/// §17.4, #2394). `None` for a box with no caption, and for a block standing
/// for an anonymous table, which keeps its captions inside its own border.
pub(crate) fn caption_bands(tree: &NodeTree, node: &crate::node::Node) -> Option<(f32, f32)> {
    let b = node.table_box.as_ref()?;
    if is_host(node) || (b.first_row <= 1 && b.bottom_captions == 0) {
        return None;
    }
    let (mut top, mut bottom) = (0.0f32, node.layout.height);
    for &id in &b.placed {
        let Some(n) = tree.nodes.get(id) else {
            continue;
        };
        let (Some(c), Some(t)) = (&n.table_cell, n.taffy_id) else {
            continue;
        };
        if n.computed_style.table_part != TablePart::Caption
            || n.computed_style.display == DisplayValue::None
        {
            continue;
        }
        let Ok(l) = tree.taffy.layout(t) else {
            continue;
        };
        // Its margin box, less what its margins take back from the table.
        if (c.row as usize) < b.first_row {
            top = top.max(n.layout.y + n.layout.height + l.margin.bottom - c.outside[2]);
        } else {
            bottom = bottom.min(n.layout.y - (l.margin.top - c.outside[0]));
        }
    }
    let bottom = (node.layout.height - bottom).max(0.0);
    (top > 0.0 || bottom > 0.0).then_some((top, bottom))
}

/// What a caption's margins resolve against, from the last compute.
#[derive(Debug, Clone, Copy)]
struct CaptionBasis {
    /// The table's border-box width, for a percentage top or bottom margin.
    full: f32,
    /// The same for a percentage side margin, when the table has a width of
    /// its own: in a table as wide as its content, a side margin would feed
    /// its own basis.
    percent: Option<f32>,
    /// The table's width inside its border, where the caption is.
    table: f32,
    /// The caption's border-box width.
    caption: f32,
}

/// Whether a caption has a `width` of its own, which `auto` side margins
/// share the rest of the table's width around.
fn caption_is_sized(cs: &crate::computed_style::ComputedStyle) -> bool {
    matches!(
        cs.width,
        DimensionValue::Length(_) | DimensionValue::Percent(_) | DimensionValue::Calc { .. }
    )
}

/// A caption's Taffy margins (top, right, bottom, left): its own, resolved
/// against the table's width, plus `outside`, which takes it out of the
/// table's padding (#2350; module docs, *Captions*). With no `basis` (before
/// the first compute) a margin that needs one is seeded: a percentage as
/// zero, a `calc()` as its length, an `auto` as zero. `auto` is zero
/// vertically, and beside an `auto` width (the caption fills the table); beside
/// a `width`, the side margins share what the table's width leaves.
fn caption_margins(
    cs: &crate::computed_style::ComputedStyle,
    outside: [f32; 4],
    basis: Option<CaptionBasis>,
) -> [f32; 4] {
    use crate::computed_style::values::LengthPercentageAutoValue as M;
    let resolve_in = |m: M, width: Option<f32>| -> Option<f32> {
        match m {
            M::Auto => None,
            M::Length(v) => Some(v),
            M::Percent(p) => Some(width.map_or(0.0, |w| p * w)),
            M::Calc { px, pct } => Some(px + width.map_or(0.0, |w| pct * w)),
        }
    };
    let (full, side) = (basis.map(|b| b.full), basis.and_then(|b| b.percent));
    let top = resolve_in(cs.margin_top, full).unwrap_or(0.0);
    let bottom = resolve_in(cs.margin_bottom, full).unwrap_or(0.0);
    let (l, r) = (
        resolve_in(cs.margin_left, side),
        resolve_in(cs.margin_right, side),
    );
    let (left, right) = match (l, r, basis) {
        (Some(l), Some(r), _) => (l, r),
        (l, r, Some(b)) if caption_is_sized(cs) => {
            let free = b.table - b.caption;
            match (l, r) {
                (Some(l), None) => (l, free - l),
                (None, Some(r)) => (free - r, r),
                _ => {
                    let half = (free / 2.0).max(0.0);
                    (half, half)
                }
            }
        }
        (l, r, _) => (l.unwrap_or(0.0), r.unwrap_or(0.0)),
    };
    [
        top + outside[0],
        right + outside[1],
        bottom + outside[2],
        left + outside[3],
    ]
}

/// A `border-spacing` as Chrome keeps it: a `short` of device pixels,
/// converted by Blink's `RoundForImpreciseConversion` (0.01 added, the
/// fraction cut off, 0 past 32767). So `10.9px` is 10px at 1x and 10.5px at
/// 2x, `9.99px` is 10px at 1x, and `40000px` is no spacing.
pub(crate) fn whole_device_spacing(css_px: f32, scale: f64) -> f32 {
    let scale = if scale > 0.0 { scale } else { 1.0 };
    // Blink holds the length as a double parsed from the text; ours went
    // through an `f32`, which can sit just under it (9.99 is 9.98999977).
    // Two steps of the `f32` lift it back to the text's value and no
    // further, so a value really below `k - 0.01` (9.98999) stays below.
    let lifted = css_px.next_up().next_up();
    let device = f64::from(lifted) * scale + 0.01;
    if device > f64::from(i16::MAX) {
        0.0
    } else {
        (device.trunc() / scale) as f32
    }
}
