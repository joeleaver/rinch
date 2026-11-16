//! Grid-related Stylo conversion functions (from Stylo types to Taffy types).

use style::computed_values::grid_auto_flow::T as GridAutoFlow;
use style::values::computed::{GridLine, GridTemplateComponent};
use style::values::computed::{GridTemplateAreas, ImplicitGridTracks};
use style::values::generics::grid::{RepeatCount, TrackBreadth, TrackListValue, TrackSize};
use style::values::specified::GenericGridTemplateComponent;

/// Convert a single Stylo grid line (`grid-column-start` etc.) to a Taffy grid
/// placement. Honors `span N` and explicit line numbers (named lines are carried
/// through as their ident string). Mirrors `stylo-taffy`'s `grid_line`.
fn grid_line_from_stylo(input: &GridLine) -> taffy::GridPlacement<String> {
    if input.is_auto() {
        return taffy::GridPlacement::Auto;
    }
    let ident = input.ident.0.to_string();
    if input.is_span {
        let n: u16 = input.line_num.try_into().unwrap_or(1);
        if ident.is_empty() {
            taffy::GridPlacement::Span(n.max(1))
        } else {
            taffy::GridPlacement::NamedSpan(ident, n.max(1))
        }
    } else if !ident.is_empty() {
        taffy::GridPlacement::NamedLine(ident, input.line_num as i16)
    } else if input.line_num != 0 {
        taffy::style_helpers::line(input.line_num as i16)
    } else {
        taffy::GridPlacement::Auto
    }
}

/// Convert a Stylo `(start, end)` grid-line pair to a Taffy placement line
/// (`grid-column` / `grid-row`).
pub(super) fn grid_placement_from_stylo(
    start: &GridLine,
    end: &GridLine,
) -> taffy::Line<taffy::GridPlacement<String>> {
    taffy::Line {
        start: grid_line_from_stylo(start),
        end: grid_line_from_stylo(end),
    }
}

pub(super) fn grid_auto_flow_from_stylo(input: &GridAutoFlow) -> taffy::GridAutoFlow {
    let is_row = input.contains(GridAutoFlow::ROW);
    let is_dense = input.contains(GridAutoFlow::DENSE);

    match (is_row, is_dense) {
        (true, false) => taffy::GridAutoFlow::Row,
        (true, true) => taffy::GridAutoFlow::RowDense,
        (false, false) => taffy::GridAutoFlow::Column,
        (false, true) => taffy::GridAutoFlow::ColumnDense,
    }
}

pub(super) fn grid_template_tracks_from_stylo(
    input: &GridTemplateComponent,
) -> Vec<taffy::GridTemplateComponent<String>> {
    match input {
        GenericGridTemplateComponent::None => Vec::new(),
        GenericGridTemplateComponent::TrackList(list) => list
            .values
            .iter()
            .map(|track| match track {
                TrackListValue::TrackSize(size) => {
                    taffy::GridTemplateComponent::Single(track_size_from_stylo(size))
                }
                TrackListValue::TrackRepeat(repeat) => {
                    taffy::GridTemplateComponent::Repeat(taffy::GridTemplateRepetition {
                        count: track_repeat_from_stylo(repeat.count),
                        tracks: repeat
                            .track_sizes
                            .iter()
                            .map(track_size_from_stylo)
                            .collect(),
                        line_names: repeat
                            .line_names
                            .iter()
                            .map(|line_name_set| {
                                line_name_set
                                    .iter()
                                    .map(|ident| ident.0.to_string())
                                    .collect::<Vec<_>>()
                            })
                            .collect::<Vec<_>>(),
                    })
                }
            })
            .collect(),

        // TODO: Implement subgrid and masonry
        GenericGridTemplateComponent::Subgrid(_) => Vec::new(),
        GenericGridTemplateComponent::Masonry => Vec::new(),
    }
}

/// The `[names]` of a track list (#2704) as Taffy reads them: one set per
/// line between the list's top-level components, `values.len() + 1` sets,
/// where a `repeat()` counts as one component and keeps its own names in
/// [`grid_template_tracks_from_stylo`]'s `line_names`.
///
/// Taffy walks a `repeat()`'s names only while it walks these sets (it takes
/// the next component after each set), so the list is empty only when no
/// line is named anywhere, the top level and every `repeat()` alike.
pub(super) fn grid_template_line_names_from_stylo(
    input: &GridTemplateComponent,
) -> Vec<Vec<String>> {
    let GenericGridTemplateComponent::TrackList(list) = input else {
        return Vec::new();
    };
    let named_in_repeat = list.values.iter().any(|value| match value {
        TrackListValue::TrackRepeat(repeat) => repeat.line_names.iter().any(|set| !set.is_empty()),
        TrackListValue::TrackSize(_) => false,
    });
    if !named_in_repeat && list.line_names.iter().all(|set| set.is_empty()) {
        return Vec::new();
    }
    (0..=list.values.len())
        .map(|line| {
            list.line_names
                .get(line)
                .map(|set| set.iter().map(|ident| ident.0.to_string()).collect())
                .unwrap_or_default()
        })
        .collect()
}

/// `grid-template-areas` (#2704) as Taffy's areas. Stylo numbers an area's
/// rows and columns by their lines from 1, end exclusive, as Taffy does; the
/// template's size (its strings, and the columns in each) is the explicit
/// grid's size where the track lists are shorter.
pub(super) fn grid_template_areas_from_stylo(
    input: &GridTemplateAreas,
) -> Option<taffy::GridTemplateAreas<String>> {
    let GridTemplateAreas::Areas(areas) = input else {
        return None;
    };
    let areas = &areas.0;
    let clamp = |n: u32| u16::try_from(n).unwrap_or(u16::MAX);
    Some(taffy::GridTemplateAreas {
        areas: areas
            .areas
            .iter()
            .map(|area| taffy::GridTemplateArea {
                name: area.name.to_string(),
                row_start: clamp(area.rows.start),
                row_end: clamp(area.rows.end),
                column_start: clamp(area.columns.start),
                column_end: clamp(area.columns.end),
            })
            .collect(),
        row_count: clamp(areas.strings.len() as u32),
        column_count: clamp(areas.width),
    })
}

/// `grid-auto-columns` / `grid-auto-rows` (#2716) as Taffy's auto track
/// list: each value converted as a track list's single size. Stylo keeps the
/// initial `auto` as an empty list, which Taffy reads as `auto` too.
pub(super) fn grid_auto_tracks_from_stylo(
    input: &ImplicitGridTracks,
) -> Vec<taffy::TrackSizingFunction> {
    input.0.iter().map(track_size_from_stylo).collect()
}

fn track_repeat_from_stylo(input: RepeatCount<i32>) -> taffy::RepetitionCount {
    match input {
        RepeatCount::Number(val) => taffy::RepetitionCount::Count(val.try_into().unwrap_or(1)),
        RepeatCount::AutoFill => taffy::RepetitionCount::AutoFill,
        RepeatCount::AutoFit => taffy::RepetitionCount::AutoFit,
    }
}

fn track_size_from_stylo(
    input: &TrackSize<style::values::computed::LengthPercentage>,
) -> taffy::TrackSizingFunction {
    match input {
        TrackSize::Breadth(breadth) => taffy::MinMax {
            min: min_track_from_stylo(breadth),
            max: max_track_from_stylo(breadth),
        },
        TrackSize::Minmax(min, max) => taffy::MinMax {
            min: min_track_from_stylo(min),
            max: max_track_from_stylo(max),
        },
        TrackSize::FitContent(limit) => taffy::MinMax {
            min: taffy::MinTrackSizingFunction::auto(),
            max: match limit {
                TrackBreadth::Breadth(lp) => {
                    use taffy::style_helpers::TaffyFitContent;
                    taffy::MaxTrackSizingFunction::fit_content(length_percentage_from_stylo_lp(lp))
                }
                // Fr, Auto, MinContent, MaxContent shouldn't appear in fit-content
                _ => taffy::MaxTrackSizingFunction::auto(),
            },
        },
    }
}

fn min_track_from_stylo(
    input: &TrackBreadth<style::values::computed::LengthPercentage>,
) -> taffy::MinTrackSizingFunction {
    match input {
        TrackBreadth::Breadth(lp) => {
            taffy::MinTrackSizingFunction::from(length_percentage_from_stylo_lp(lp))
        }
        TrackBreadth::Flex(_) => taffy::MinTrackSizingFunction::auto(),
        TrackBreadth::Auto => taffy::MinTrackSizingFunction::auto(),
        TrackBreadth::MinContent => taffy::MinTrackSizingFunction::min_content(),
        TrackBreadth::MaxContent => taffy::MinTrackSizingFunction::max_content(),
    }
}

fn max_track_from_stylo(
    input: &TrackBreadth<style::values::computed::LengthPercentage>,
) -> taffy::MaxTrackSizingFunction {
    match input {
        TrackBreadth::Breadth(lp) => {
            taffy::MaxTrackSizingFunction::from(length_percentage_from_stylo_lp(lp))
        }
        TrackBreadth::Flex(val) => taffy::MaxTrackSizingFunction::fr(val.0),
        TrackBreadth::Auto => taffy::MaxTrackSizingFunction::auto(),
        TrackBreadth::MinContent => taffy::MaxTrackSizingFunction::min_content(),
        TrackBreadth::MaxContent => taffy::MaxTrackSizingFunction::max_content(),
    }
}

/// Convert Stylo LengthPercentage to Taffy LengthPercentage.
/// This is similar to length_percentage_from_stylo in box_model but works with plain
/// LengthPercentage instead of NonNegativeLengthPercentage, and returns a Taffy type.
///
/// This converter does NOT route a mixed `calc()` through `Calc { px, pct }`
/// like the rest of the #278 family, because its output is a bare Taffy value
/// stored inside `grid_template_*`/`grid_auto_*` on the Taffy style — there is
/// no `ComputedStyle` slot to carry the pair to `resolve_layout_calcs`, and
/// Taffy's own calc pointer resolves to `0.0` under `TaffyTree` (taffy-0.14.0,
/// `src/tree/taffy_tree.rs:387`). Until grid tracks grow a side channel, a
/// mixed calc keeps its percentage component — `calc(50% - 10px)` sizes the
/// track at `50%`, off by the length part, where it used to collapse to `0`.
fn length_percentage_from_stylo_lp(
    lp: &style::values::computed::LengthPercentage,
) -> taffy::LengthPercentage {
    if let Some(len) = lp.to_length() {
        taffy::LengthPercentage::length(len.px())
    } else if let Some(pct) = lp.to_percentage() {
        taffy::LengthPercentage::percent(pct.0)
    } else {
        let (px, pct) = super::calc::split_length_percentage(lp);
        if pct != 0.0 {
            taffy::LengthPercentage::percent(pct)
        } else {
            taffy::LengthPercentage::length(px)
        }
    }
}
