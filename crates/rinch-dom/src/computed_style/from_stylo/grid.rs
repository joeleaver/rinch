//! Grid-related Stylo conversion functions (from Stylo types to Taffy types).

use style::computed_values::grid_auto_flow::T as GridAutoFlow;
use style::values::computed::{GridLine, GridTemplateComponent};
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
        TrackBreadth::Fr(_) => taffy::MinTrackSizingFunction::auto(),
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
        TrackBreadth::Fr(val) => taffy::MaxTrackSizingFunction::fr(*val),
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
/// Taffy's own calc pointer resolves to `0.0` under `TaffyTree` (taffy-0.12.2,
/// `src/tree/taffy_tree.rs:391`). Until grid tracks grow a side channel, a
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
