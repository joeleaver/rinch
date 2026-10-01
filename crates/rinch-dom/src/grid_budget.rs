//! Keeping a CSS grid inside the grid lines Taffy can number (#1210).
//!
//! Taffy 0.12 numbers grid lines as `i16` (`OriginZeroLine(i16)`, and its
//! track counts as `u16`). A grid whose placement reaches past line 32767 —
//! 40000 auto-placed rows, 33 items of `grid-row: span 1000`, a template of
//! `repeat(10000, 1px 1px 1px 1px)` — **panics** inside `compute_layout`
//! (an overflow in debug; an index or `unwrap` panic in release), and a panic
//! in layout is a crash reachable from ordinary markup: the rich-text editor's
//! tables are grids, one grid row per table row. Measured on a raw
//! `taffy::TaffyTree`: a one-column grid of 32767 auto rows lays out and one of
//! 32768 panics, in both profiles, and stacked spans panic exactly when their
//! sum passes 32767.
//!
//! Taffy 0.13 clamps a grid to 10000 tracks per axis (its PR #986, css-grid-1
//! §11 "overlarge grids", which is Gecko's limit too); Chrome 153 has no limit
//! at these sizes (40000 rows lay out 400000px tall, 120 items of
//! `span 10000` make 1.2M rows). Neither is available without leaving Taffy
//! 0.12, so rinch keeps every grid Taffy is handed provably inside the limit:
//!
//! - [`over_budget`] bounds, from the computed styles alone, the grid lines
//!   the placement algorithm can reach in each axis. The bound is **sound**
//!   (a grid it passes cannot reach past the limit — pinned against raw
//!   Taffy by `grid_line_limit_tests`) and **tight for the grids people build**:
//!   `n` auto-placed 1x1 items in `C` explicit columns count `ceil(n / C)`
//!   rows, so a 3-column table of 12000 rows (36000 cells) is a grid, as it
//!   always was.
//! - A grid over the bound is **not handed to Taffy as a grid**
//!   ([`apply_degrade`]). With one explicit column (or none) it becomes a
//!   flex column: each item stacks and fills the width — the track's, when
//!   that is one fixed length — which is exactly the grid's geometry for a
//!   long list. With `C` columns it becomes a row-wrapping flex container and
//!   each in-flow item a `flex: 0 0 <span/C of the width>` item with no
//!   automatic minimum width (rinch lays every box out border-box, so that is
//!   the item's outer width). For equal columns whose items are auto-placed
//!   and span no rows — every editor table without a rowspan — that is the
//!   grid's own geometry: rows of `C`, each as tall as its tallest item. What
//!   it gives up: rows spanned by an item, definite line placement, unequal
//!   or fixed track sizes across several columns, explicit row sizes, the
//!   column gap (a gap would push the `C`th item onto the next line), and
//!   alignment other than stretch. Nothing is dropped — every item is laid
//!   out and painted.
//!
//! What this does not cover: `repeat(auto-fill | auto-fit, …)`, whose count
//! Taffy derives from the container's inner size during the compute, is
//! counted as one repetition here, and still panics past the limit (#1231).
use crate::computed_style::{ComputedStyle, DisplayValue, PositionValue};
use crate::node::NodeTree;

/// The highest origin-zero grid line Taffy 0.12 can number, and the most lines
/// an axis may span from its lowest line to its highest: a line is an `i16`,
/// and `into_track_vec_index` adds the negative implicit track count to it.
pub const GRID_LINE_LIMIT: i64 = i16::MAX as i64;

/// Whether `display` makes a box a grid container.
pub(crate) fn is_grid(display: DisplayValue) -> bool {
    matches!(display, DisplayValue::Grid | DisplayValue::InlineGrid)
}

/// The explicit track count of one axis' template. An auto repetition counts
/// once (see the module doc); a fixed one its count times its track list.
fn explicit_tracks(template: &[taffy::GridTemplateComponent<String>]) -> i64 {
    template
        .iter()
        .map(|c| match c {
            taffy::GridTemplateComponent::Single(_) => 1,
            taffy::GridTemplateComponent::Repeat(r) => {
                let per = r.tracks.len() as i64;
                match r.count {
                    taffy::RepetitionCount::Count(n) => i64::from(n) * per,
                    taffy::RepetitionCount::AutoFill | taffy::RepetitionCount::AutoFit => per,
                }
            }
        })
        .sum()
}

/// One item's placement in one axis, in origin-zero lines.
enum Axis {
    /// Some line is definite: the item occupies `min..max`.
    Definite { min: i64, max: i64 },
    /// Placed by the auto-placement cursor, spanning `span` tracks.
    Auto { span: i64 },
}

/// Resolve `line` against an axis with `explicit` tracks, conservatively for
/// named lines: rinch declares no line names, so Taffy resolves a named line or
/// span against the implicit lines past the explicit grid, and the bound takes
/// the farthest such line in both directions.
fn axis(line: &taffy::Line<taffy::GridPlacement<String>>, explicit: i64) -> Axis {
    use taffy::GridPlacement as P;
    // A definite line as an origin-zero range it can occupy (a named line may
    // land anywhere within it).
    let oz = |p: &P<String>| -> Option<(i64, i64)> {
        match p {
            P::Line(l) => {
                let n = i64::from(l.as_i16());
                Some(if n > 0 {
                    (n - 1, n - 1)
                } else if n < 0 {
                    (n + explicit + 1, n + explicit + 1)
                } else {
                    return None;
                })
            }
            P::NamedLine(_, n) => {
                let n = i64::from(n.unsigned_abs()) + 1;
                Some((-n, explicit + n))
            }
            _ => None,
        }
    };
    let span = |p: &P<String>| -> Option<i64> {
        match p {
            P::Span(s) => Some(i64::from(*s).max(1)),
            P::NamedSpan(_, s) => Some(explicit + i64::from(*s) + 1),
            _ => None,
        }
    };
    match (oz(&line.start), oz(&line.end)) {
        (Some((a0, a1)), Some((b0, b1))) => Axis::Definite {
            min: a0.min(b0),
            max: a1.max(b1) + 1,
        },
        (Some((a0, a1)), None) => Axis::Definite {
            min: a0,
            max: a1 + span(&line.end).unwrap_or(1),
        },
        (None, Some((b0, b1))) => Axis::Definite {
            min: b0 - span(&line.start).unwrap_or(1),
            max: b1,
        },
        (None, None) => Axis::Auto {
            span: span(&line.start).or_else(|| span(&line.end)).unwrap_or(1),
        },
    }
}

/// Visit every in-flow grid item of `container` — its children, and through
/// any `display: contents` child that child's — as a computed style, or
/// `None` for a text run (an anonymous item placed like an auto 1x1 one).
fn for_each_item(tree: &NodeTree, container: usize, f: &mut impl FnMut(Option<&ComputedStyle>)) {
    let Some(node) = tree.nodes.get(container) else {
        return;
    };
    for &c in &node.children {
        let Some(child) = tree.nodes.get(c) else {
            continue;
        };
        if child.is_text() {
            if child
                .text_content()
                .is_some_and(|t| !t.chars().all(char::is_whitespace))
            {
                f(None);
            }
            continue;
        }
        if !child.is_element() {
            continue;
        }
        let style = &child.computed_style;
        match style.display {
            DisplayValue::None => {}
            DisplayValue::Contents => for_each_item(tree, c, f),
            _ if matches!(
                style.position,
                PositionValue::Absolute | PositionValue::Fixed
            ) => {}
            _ => f(Some(style)),
        }
    }
}

/// The column count a grid over the line budget is laid out with, or `None`
/// when `node_id` is not a grid container or its placement provably fits.
///
/// The bound, per axis: the lines definite placements and the explicit
/// template reach, plus — in the flow axis — one line per `C` auto-placed 1x1
/// items and `span + 1` per other auto-placed item, and — in the cross axis —
/// the span of every item locked to a flow line (step 2 of the placement
/// algorithm grows cross tracks for those), plus `N` more per locked item
/// and `N` below the lowest line, `N` the larger negative implicit track
/// count of the two axes. That `N` is Taffy 0.12's own arithmetic, not CSS:
/// step 2's sparse cursor (`CellOccupancyMatrix::last_of_type`) turns the
/// cross-axis index of the last auto-placed cell in the item's flow track
/// back into a line with the *flow* axis' negative count, so the search
/// starts up to `N` tracks past (or before) where it should — and each
/// locked item can push the grid that far further. A grid with no negative
/// lines pays nothing for it. A 1x1 item that opens a fresh
/// flow track does so at its first cross track, and the ones after it fill
/// that track until another kind of item interrupts them, which is what the
/// `+ 1` charges.
pub(crate) fn over_budget(tree: &NodeTree, node_id: usize) -> Option<u32> {
    let (flow_lines, cross_lines) = line_bounds(tree, node_id)?;
    if flow_lines <= GRID_LINE_LIMIT && cross_lines <= GRID_LINE_LIMIT {
        return None;
    }
    let columns = explicit_tracks(&tree.nodes[node_id].computed_style.grid_template_columns);
    Some(columns.clamp(1, i64::from(u32::MAX)) as u32)
}

/// The bound [`over_budget`] compares with [`GRID_LINE_LIMIT`]: the most
/// lines, lowest to highest, the placement of grid container `node_id` can
/// span in its flow axis and its cross axis. `None` for any other box.
pub fn line_bounds(tree: &NodeTree, node_id: usize) -> Option<(i64, i64)> {
    let style = &tree.nodes.get(node_id)?.computed_style;
    if !is_grid(style.display) {
        return None;
    }
    let rows_flow = matches!(
        style.grid_auto_flow,
        taffy::GridAutoFlow::Row | taffy::GridAutoFlow::RowDense
    );
    let e_col = explicit_tracks(&style.grid_template_columns);
    let e_row = explicit_tracks(&style.grid_template_rows);
    let (e_flow, e_cross) = if rows_flow {
        (e_row, e_col)
    } else {
        (e_col, e_row)
    };
    // (min, max) lines reached by definite placement and the template.
    let (mut f_min, mut f_max) = (0i64, e_flow);
    let (mut x_min, mut x_max) = (0i64, e_cross);
    // Cross growth from items locked to a flow line, 1x1 auto items, and the
    // flow lines every other auto item can add.
    let (mut x_locked, mut locked, mut ones, mut others) = (0i64, 0i64, 0i64, 0i64);
    for_each_item(tree, node_id, &mut |item| {
        let (flow, cross) = match item {
            None => (Axis::Auto { span: 1 }, Axis::Auto { span: 1 }),
            Some(s) => {
                let (f, x) = if rows_flow {
                    (&s.grid_row, &s.grid_column)
                } else {
                    (&s.grid_column, &s.grid_row)
                };
                (axis(f, e_flow), axis(x, e_cross))
            }
        };
        let cross_span = match cross {
            Axis::Definite { min, max } => {
                x_min = x_min.min(min);
                x_max = x_max.max(max);
                None
            }
            Axis::Auto { span } => {
                x_max = x_max.max(span);
                Some(span)
            }
        };
        match flow {
            Axis::Definite { min, max } => {
                f_min = f_min.min(min);
                f_max = f_max.max(max);
                if let Some(span) = cross_span {
                    x_locked += span;
                    locked += 1;
                }
            }
            Axis::Auto { span: 1 } if cross_span == Some(1) => ones += 1,
            Axis::Auto { span } => others += span + 1,
        }
    });
    let columns = e_cross.max(1);
    let flow_lines = f_max + (ones + columns - 1) / columns + others - f_min;
    // Taffy's mis-converted step-2 cursor (see `over_budget`): the cross
    // axis' negative count never passes `n`, so neither does the shift.
    let n = (-x_min).max(-f_min);
    let cross_lines = x_max + x_locked + locked * n + n;
    Some((flow_lines, cross_lines))
}

/// The nearest ancestor-or-self of `node_id` that is not `display: contents`
/// — the box whose layout children include `node_id`'s own.
pub(crate) fn layout_container_of(tree: &NodeTree, node_id: usize) -> Option<usize> {
    let mut id = node_id;
    loop {
        let node = tree.nodes.get(id)?;
        if node.computed_style.display != DisplayValue::Contents {
            return Some(id);
        }
        id = node.parent?;
    }
}

/// Write a degraded grid's layout into `style`, the Taffy style `node_id`'s
/// computed values produce (see the module doc): a container carrying
/// [`crate::node::Node::grid_degrade`] becomes a row-wrapping flex container,
/// and an in-flow item of one a fixed-basis flex item. Every other box is left
/// alone — and costs one load while no grid anywhere is degraded.
pub(crate) fn apply_degrade(tree: &NodeTree, node_id: usize, style: &mut taffy::Style) {
    if tree.degraded_grids == 0 {
        return;
    }
    let Some(node) = tree.nodes.get(node_id) else {
        return;
    };
    if let Some(columns) = node.grid_degrade {
        style.display = taffy::Display::Flex;
        if columns == 1 {
            style.flex_direction = taffy::FlexDirection::Column;
            style.flex_wrap = taffy::FlexWrap::NoWrap;
        } else {
            style.flex_direction = taffy::FlexDirection::Row;
            style.flex_wrap = taffy::FlexWrap::Wrap;
            style.gap.width = taffy::LengthPercentage::length(0.0);
        }
        return;
    }
    let cs = &node.computed_style;
    if matches!(cs.display, DisplayValue::None | DisplayValue::Contents)
        || matches!(cs.position, PositionValue::Absolute | PositionValue::Fixed)
    {
        return;
    }
    let Some((container, columns)) = node
        .parent
        .and_then(|p| layout_container_of(tree, p))
        .and_then(|c| Some((c, tree.nodes[c].grid_degrade?)))
    else {
        return;
    };
    // A grid item neither grows nor shrinks with its siblings.
    style.flex_grow = 0.0;
    style.flex_shrink = 0.0;
    if columns == 1 {
        // One column: a flex column, whose cross axis stretches the item to
        // the container's width as the grid stretches it to the track's —
        // and to the track's own size, when that is one fixed length.
        if let Some(length) =
            uniform_fixed_track(&tree.nodes[container].computed_style.grid_template_columns)
            && style.size.width == taffy::Dimension::auto()
        {
            style.size.width = taffy::Dimension::length(length);
        }
        return;
    }
    let span = match axis(&cs.grid_column, i64::from(columns)) {
        Axis::Auto { span } => span,
        Axis::Definite { min, max } => max - min,
    }
    .clamp(1, i64::from(columns));
    // `span / C` of the line — rinch lays every box out border-box (Taffy's
    // default; it has no `box-sizing`), so that is the item's outer width and
    // exactly `C` one-column items fill a line — with no automatic minimum
    // to push it wider.
    style.flex_basis = taffy::Dimension::percent(span as f32 / columns as f32);
    style.min_size.width = taffy::Dimension::length(0.0);
}

/// The size every column of `template` has, when all of them are one fixed
/// length (`100px`, `repeat(3, 40px)`): a degraded one-column grid's items
/// take that width, as the grid's track gives them.
fn uniform_fixed_track(template: &[taffy::GridTemplateComponent<String>]) -> Option<f32> {
    let mut tracks = template.iter().flat_map(|c| match c {
        taffy::GridTemplateComponent::Single(t) => std::slice::from_ref(t).iter(),
        taffy::GridTemplateComponent::Repeat(r) => r.tracks.iter(),
    });
    let first = *tracks.next()?;
    if !tracks.all(|t| *t == first) {
        return None;
    }
    let none = |_: *const (), _: f32| 0.0;
    let min = first.min.definite_value(None, none)?;
    let max = first.max.definite_value(None, none)?;
    (min == max).then_some(min)
}

/// Copy the fields [`apply_degrade`] writes from `from` into `to`.
pub(crate) fn copy_degrade_fields(from: &taffy::Style, to: &mut taffy::Style) {
    to.display = from.display;
    to.flex_direction = from.flex_direction;
    to.flex_wrap = from.flex_wrap;
    to.gap.width = from.gap.width;
    to.flex_basis = from.flex_basis;
    to.flex_grow = from.flex_grow;
    to.flex_shrink = from.flex_shrink;
    to.min_size.width = from.min_size.width;
    to.size.width = from.size.width;
}

/// Every element whose Taffy style [`apply_degrade`] writes as an item of
/// `container`: its in-flow layout children that are elements.
pub(crate) fn item_elements(tree: &NodeTree, container: usize, out: &mut Vec<usize>) {
    let Some(node) = tree.nodes.get(container) else {
        return;
    };
    for &c in &node.children {
        let Some(child) = tree.nodes.get(c) else {
            continue;
        };
        if !child.is_element() {
            continue;
        }
        match child.computed_style.display {
            DisplayValue::None => {}
            DisplayValue::Contents => item_elements(tree, c, out),
            _ => out.push(c),
        }
    }
}
