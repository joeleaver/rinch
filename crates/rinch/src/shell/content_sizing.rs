//! A window sized by its content ([`WindowProps::size_to_content`]): the
//! shell's decisions, pure so they are tested without a compositor. The
//! sizing itself is the app's content-sized mode (`app/content_size.rs`).
//!
//! - **Which axes the content sizes** ([`content_axes`]): both, except on a
//!   layer surface an axis [`LayerSurface::compositor_size`] assigns (anchored
//!   to both its edges), which is the compositor's: the content fills it.
//!   With both axes assigned nothing is left to size, and the window is not
//!   content-sized (a warning is logged).
//! - **The maximum** ([`content_max`]): per axis, the app's
//!   ([`ContentSizing`]), else the size of the output the window is on less
//!   a layer surface's margins on the edges it is anchored to on that axis,
//!   else the props' `width`/`height` when no output is listed.
//! - **The output** ([`sizing_output`]): a layer surface's output by id,
//!   else by name, else the window's `monitor`, else the first output listed (the
//!   compositor chooses an unnamed one; its grant corrects a wrong guess).

use rinch_core::element::{ContentSizing, LayerSurface, WindowProps};

use crate::outputs::Output;

/// How the content sizes one axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AxisSizing {
    /// The window's size on this axis is the content's.
    Content,
    /// The compositor assigns this axis; the content fills it.
    Fill,
}

/// How the content sizes each axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContentAxes {
    pub width: AxisSizing,
    pub height: AxisSizing,
}

/// The axes a content-sized window's content sizes, given the layer surface
/// it is made as (`None`: not a layer surface). `None` when the compositor
/// assigns both: nothing is left to size.
pub(crate) fn content_axes(surface: Option<&LayerSurface>) -> Option<ContentAxes> {
    let assigned = surface.map(LayerSurface::assigned_axes).unwrap_or_default();
    let axis = |assigned: bool| {
        if assigned {
            AxisSizing::Fill
        } else {
            AxisSizing::Content
        }
    };
    let axes = ContentAxes {
        width: axis(assigned.width),
        height: axis(assigned.height),
    };
    (axes.width == AxisSizing::Content || axes.height == AxisSizing::Content).then_some(axes)
}

/// Say so when a window asked to be sized by its content cannot be: its
/// layer surface's compositor assigns both axes.
pub(crate) fn warn_nothing_to_size() {
    tracing::warn!(
        "WindowProps::size_to_content on a layer surface whose compositor_size assigns both \
         axes: the compositor sizes it, so its content does not"
    );
}

/// Whether `props` sets `width`/`height` beside `size_to_content`, which
/// wins: they are not used. Read against their defaults, the only way a
/// value set to them is told from one left alone.
pub(crate) fn sets_ignored_size(props: &WindowProps) -> bool {
    let default = WindowProps::default();
    props.size_to_content.is_some()
        && (props.width, props.height) != (default.width, default.height)
}

/// A debug build's warning that `width`/`height` are not used
/// ([`sets_ignored_size`]).
pub(crate) fn warn_ignored_size(props: &WindowProps) {
    if cfg!(debug_assertions) && sets_ignored_size(props) {
        tracing::warn!(
            "WindowProps::width/height ({}x{}) are not used: size_to_content sizes the window by \
             its content",
            props.width,
            props.height
        );
    }
}

/// The output a content-sized window is measured against: its layer
/// surface's output by id, else by name, else its `monitor`, else the first
/// listed. An unknown name falls back to the first too.
pub(crate) fn sizing_output<'a>(
    surface: Option<&LayerSurface>,
    props: &WindowProps,
    outputs: &'a [Output],
) -> Option<&'a Output> {
    let by_id = surface
        .and_then(|s| s.output_id)
        .and_then(|id| outputs.iter().find(|o| o.id == id));
    let name = match surface {
        Some(surface) => surface.output.as_deref(),
        None => props.monitor.as_deref(),
    };
    by_id
        .or_else(|| name.and_then(|name| outputs.iter().find(|o| o.name.as_deref() == Some(name))))
        .or_else(|| outputs.first())
}

/// The largest size the content may take, logical px (see the module docs).
pub(crate) fn content_max(
    sizing: &ContentSizing,
    surface: Option<&LayerSurface>,
    output: Option<&Output>,
    fallback: (u32, u32),
) -> (f32, f32) {
    let (left, right, top, bottom) = surface.map_or((0, 0, 0, 0), |s| {
        let (a, m) = (s.anchor, s.margins);
        let on = |anchored: bool, margin: i32| if anchored { margin.max(0) } else { 0 };
        (
            on(a.left, m.left),
            on(a.right, m.right),
            on(a.top, m.top),
            on(a.bottom, m.bottom),
        )
    });
    let axis = |given: Option<u32>, output: Option<f64>, margins: i32, fallback: u32| {
        let size = match (given, output) {
            (Some(given), _) => given as f64,
            (None, Some(output)) => output - margins as f64,
            (None, None) => fallback as f64,
        };
        size.max(1.0) as f32
    };
    (
        axis(
            sizing.max_width,
            output.map(|o| o.size.0),
            left + right,
            fallback.0,
        ),
        axis(
            sizing.max_height,
            output.map(|o| o.size.1),
            top + bottom,
            fallback.1,
        ),
    )
}

#[cfg(test)]
#[path = "content_sizing_tests.rs"]
mod tests;
