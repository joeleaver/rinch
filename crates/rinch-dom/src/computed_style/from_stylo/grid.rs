//! Grid-related Stylo conversion functions (from Stylo types to Taffy types).

use style::computed_values::grid_auto_flow::T as GridAutoFlow;
use style::values::computed::GridTemplateComponent;
use style::values::generics::grid::{RepeatCount, TrackBreadth, TrackListValue, TrackSize};
use style::values::specified::GenericGridTemplateComponent;

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
fn length_percentage_from_stylo_lp(
    lp: &style::values::computed::LengthPercentage,
) -> taffy::LengthPercentage {
    if let Some(len) = lp.to_length() {
        taffy::LengthPercentage::length(len.px())
    } else if let Some(pct) = lp.to_percentage() {
        taffy::LengthPercentage::percent(pct.0)
    } else {
        taffy::LengthPercentage::length(0.0)
    }
}
