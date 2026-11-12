//! Wayland windows as `wlr-layer-shell` surfaces (issues #1523, #2413).
//!
//! An xdg-toplevel cannot refuse keyboard focus, a taskbar entry, or ask to
//! stay above other windows, so on native Wayland a window that asks for that
//! — [`WindowProps::non_activating`], or [`WindowProps::wayland_layer_shell`]
//! `Some(true)` — is given a layer-shell role instead, through rinch's winit
//! fork (`WindowAttributesWayland::with_layer_shell`). Everything else about
//! the window (buffers, scale, input, resize, the renderers) is unchanged.
//!
//! Two configurations, one role:
//!
//! - **An overlay** (the shorthands, #1523; [`LayerRequest`]): rinch picks
//!   the configuration and places the surface.
//!   - **Layer**: `top`, or `overlay` for a window raised above status bars
//!     ([`wants_overlay_layer`]: `WindowProps::level` above `Floating`).
//!   - **Keyboard**: none for a non-activating window, on-demand otherwise.
//!   - **Position**: anchored to the output's top-left corner, with the
//!     position as the top and left margins, in logical pixels relative to
//!     the output. A window with no position yet is not anchored: the
//!     compositor centres it, and the first move reads where that is
//!     ([`centred_position`]).
//!   - **Move / resize**: the compositor has none for a layer surface, so a
//!     drag region and resize edges run a [`Gesture`] that follows the
//!     pointer and sets margins and size.
//! - **An app's own surface** ([`WindowProps::wayland_layer`], #2413): the
//!   app's [`LayerSurface`] exactly, which wins over the shorthands
//!   ([`LayerSpec::for_window`]). Its anchors and margins are the app's, so
//!   rinch runs no gesture on it and never re-anchors it
//!   ([`LayerState::app_placed`]); a change at run time goes through the
//!   fork's setters ([`layer_update`]), and a change of output makes the
//!   surface again.
//!
//! When the compositor has no `zwlr_layer_shell_v1` (GNOME/Mutter) the fork
//! makes an ordinary xdg-toplevel, and the window behaves as before #1523.

pub use rinch_core::element::WindowSurfaceKind;
use rinch_core::element::{
    LayerAnchor, LayerAxes, LayerExclusiveZone, LayerKeyboard, LayerMargins, LayerShellLayer,
    LayerSurface, WindowLevel, WindowProps,
};
use rinch_platform::ResizeDirection;

use super::overlay_window::Backend;
use super::window_geometry::WindowOf;

/// Whether a window with `props` is a layer surface on `backend`: an app's
/// own [`LayerSurface`] wins over the shorthands.
pub(crate) fn uses_layer_shell(props: &WindowProps, backend: Backend) -> bool {
    backend == Backend::Wayland
        && (props.wayland_layer.is_some()
            || props.wayland_layer_shell.unwrap_or(props.non_activating))
}

/// Whether the window goes in the `overlay` layer (above panels and
/// fullscreen windows) rather than `top`: a level above `Floating` (#1549).
/// `Normal` is in `top` too, since a layer surface is an overlay.
pub(crate) fn wants_overlay_layer(props: &WindowProps) -> bool {
    props.effective_level() > WindowLevel::Floating
}

/// What the layer surface is created with. Pure, so the choice is tested
/// without a compositor; turned into the fork's attributes by
/// `LayerRequest::to_winit`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LayerRequest {
    /// `overlay` layer rather than `top`.
    pub overlay: bool,
    /// On-demand keyboard rather than none.
    pub keyboard: bool,
    /// Top-left position on the output (the margins), logical; `None`:
    /// centred by the compositor.
    pub position: Option<(f64, f64)>,
    /// The output's connector name.
    pub output: Option<String>,
    /// The layer surface's namespace: the app id.
    pub namespace: String,
}

impl LayerRequest {
    /// The request for `props`, at `position`, on `output`.
    pub(crate) fn new(
        props: &WindowProps,
        position: Option<(f64, f64)>,
        output: Option<String>,
        namespace: &str,
    ) -> Self {
        LayerRequest {
            overlay: wants_overlay_layer(props),
            keyboard: !props.non_activating,
            position,
            output,
            namespace: namespace.to_string(),
        }
    }

    /// The overlay as a [`LayerSurface`]: what the shorthands mean in the
    /// protocol's terms.
    pub(crate) fn surface(&self) -> LayerSurface {
        LayerSurface {
            layer: if self.overlay {
                LayerShellLayer::Overlay
            } else {
                LayerShellLayer::Top
            },
            anchor: if self.position.is_some() {
                LayerAnchor::TOP | LayerAnchor::LEFT
            } else {
                LayerAnchor::NONE
            },
            margins: position_margins(self.position.unwrap_or((0.0, 0.0))),
            exclusive_zone: LayerExclusiveZone::KeepClear,
            keyboard: if self.keyboard {
                LayerKeyboard::OnDemand
            } else {
                LayerKeyboard::None
            },
            output: self.output.clone(),
            compositor_size: LayerAxes::NONE,
        }
    }

    /// The overlay in the fork's terms, as [`LayerSpec::to_winit`] sends it
    /// (the shell goes through [`LayerSpec`]; the tests read this).
    #[cfg(all(target_os = "linux", test))]
    pub(crate) fn to_winit(&self) -> winit::platform::wayland::LayerShellAttributes {
        surface_to_winit(&self.surface(), &self.namespace)
    }
}

/// The layer surface a window is created as: an overlay's configuration or
/// the app's own (#2413), and its namespace.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LayerSpec {
    pub surface: LayerSurface,
    pub namespace: String,
}

impl LayerSpec {
    /// The layer surface for a window with `props`: its
    /// [`WindowProps::wayland_layer`] when it has one (which wins over the
    /// shorthands), else the overlay at `position`. `output` is an output
    /// chosen since (`set_current_window_output`, or `Some(None)` after the
    /// compositor closed the surface); `None` keeps the configured one.
    pub(crate) fn for_window(
        props: &WindowProps,
        position: Option<(f64, f64)>,
        output: Option<Option<String>>,
        namespace: &str,
    ) -> Self {
        match &props.wayland_layer {
            Some(surface) => {
                let mut surface = surface.clone();
                if let Some(output) = output {
                    surface.output = output;
                }
                LayerSpec {
                    surface,
                    namespace: namespace.to_string(),
                }
            }
            None => {
                let output = output.unwrap_or_else(|| props.output.clone());
                LayerSpec {
                    surface: LayerRequest::new(props, position, output, namespace).surface(),
                    namespace: namespace.to_string(),
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn to_winit(&self) -> winit::platform::wayland::LayerShellAttributes {
        warn_unassignable(&self.surface);
        surface_to_winit(&self.surface, &self.namespace)
    }
}

/// Say so when [`LayerSurface::compositor_size`] names an axis the
/// compositor cannot assign ([`unassignable_axes`]).
pub(crate) fn warn_unassignable(surface: &LayerSurface) {
    let unassignable = unassignable_axes(surface);
    if unassignable != LayerAxes::NONE {
        tracing::warn!(
            "LayerSurface::compositor_size {unassignable:?} on an axis not anchored to both of \
             its edges: the protocol allows the compositor's size only there, so the window's \
             own size is used"
        );
    }
}

/// The axes [`LayerSurface::compositor_size`] asks for that the compositor
/// cannot assign: not anchored to both edges.
pub(crate) fn unassignable_axes(surface: &LayerSurface) -> LayerAxes {
    let assigned = surface.assigned_axes();
    LayerAxes {
        width: surface.compositor_size.width && !assigned.width,
        height: surface.compositor_size.height && !assigned.height,
    }
}

/// The top/left margins for a top-left position, logical.
pub(crate) fn position_margins(position: (f64, f64)) -> LayerMargins {
    LayerMargins {
        top: position.1.round() as i32,
        left: position.0.round() as i32,
        right: 0,
        bottom: 0,
    }
}

/// What places a layer surface's top-left at `position` (logical, on its
/// output): anchored top-left, with the position as its margins. The same
/// change [`margins`] and a top-left anchor make one setter at a time, as
/// sets for one commit ([`apply_layer_sets`]).
pub(crate) fn placement_sets(position: (f64, f64)) -> [LayerSet; 2] {
    [
        LayerSet::Anchor(LayerAnchor {
            top: true,
            left: true,
            bottom: false,
            right: false,
        }),
        LayerSet::Margins(position_margins(position)),
    ]
}

/// The top/left margins for a top-left position, in the fork's type.
#[cfg(target_os = "linux")]
pub(crate) fn margins(position: (f64, f64)) -> winit::platform::wayland::LayerShellMargins {
    winit_margins(position_margins(position))
}

#[cfg(target_os = "linux")]
fn winit_margins(m: LayerMargins) -> winit::platform::wayland::LayerShellMargins {
    winit::platform::wayland::LayerShellMargins {
        top: m.top,
        right: m.right,
        bottom: m.bottom,
        left: m.left,
    }
}

#[cfg(target_os = "linux")]
fn winit_anchor(a: LayerAnchor) -> winit::platform::wayland::LayerShellAnchor {
    winit::platform::wayland::LayerShellAnchor {
        top: a.top,
        bottom: a.bottom,
        left: a.left,
        right: a.right,
    }
}

#[cfg(target_os = "linux")]
fn winit_layer(layer: LayerShellLayer) -> winit::platform::wayland::LayerShellLayer {
    use winit::platform::wayland::LayerShellLayer as L;
    match layer {
        LayerShellLayer::Background => L::Background,
        LayerShellLayer::Bottom => L::Bottom,
        LayerShellLayer::Top => L::Top,
        LayerShellLayer::Overlay => L::Overlay,
    }
}

#[cfg(target_os = "linux")]
fn winit_keyboard(k: LayerKeyboard) -> winit::platform::wayland::LayerShellKeyboardInteractivity {
    use winit::platform::wayland::LayerShellKeyboardInteractivity as K;
    match k {
        LayerKeyboard::None => K::None,
        LayerKeyboard::OnDemand => K::OnDemand,
        LayerKeyboard::Exclusive => K::Exclusive,
    }
}

#[cfg(target_os = "linux")]
fn winit_axes(a: LayerAxes) -> winit::platform::wayland::LayerShellCompositorSize {
    winit::platform::wayland::LayerShellCompositorSize {
        width: a.width,
        height: a.height,
    }
}

/// A [`LayerSurface`] in the fork's terms.
#[cfg(target_os = "linux")]
pub(crate) fn surface_to_winit(
    surface: &LayerSurface,
    namespace: &str,
) -> winit::platform::wayland::LayerShellAttributes {
    winit::platform::wayland::LayerShellAttributes {
        layer: winit_layer(surface.layer),
        keyboard_interactivity: winit_keyboard(surface.keyboard),
        anchor: winit_anchor(surface.anchor),
        margins: winit_margins(surface.margins),
        exclusive_zone: surface.exclusive_zone.protocol_value(),
        output_name: surface.output.clone(),
        namespace: namespace.to_string(),
        compositor_size: winit_axes(surface.compositor_size),
    }
}

/// One change to a live layer surface, through the fork's setter for it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LayerSet {
    Layer(LayerShellLayer),
    Anchor(LayerAnchor),
    Margins(LayerMargins),
    ExclusiveZone(LayerExclusiveZone),
    Keyboard(LayerKeyboard),
    CompositorSize(LayerAxes),
}

/// What `set_current_window_layer` does to a live layer surface (#2413).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LayerUpdate {
    /// The changes, all in one commit ([`apply_layer_sets`]).
    Apply(Vec<LayerSet>),
    /// Make the surface again: the output changed, which the protocol fixes
    /// when the surface is made (`get_layer_surface`).
    Recreate,
}

/// The update from the live surface's configuration `was` (with the output
/// it is on) to `new`. Pure, so the rule is tested without a compositor.
/// Every change but the output goes to the fork together, in one commit
/// ([`apply_layer_sets`]); it cannot be a protocol error, since the fork
/// sends the size 0 only on an axis anchored to both edges in that commit.
pub(crate) fn layer_update(was: &LayerSurface, new: &LayerSurface) -> LayerUpdate {
    if was.output != new.output {
        return LayerUpdate::Recreate;
    }
    let mut sets = Vec::new();
    if was.layer != new.layer {
        sets.push(LayerSet::Layer(new.layer));
    }
    if was.anchor != new.anchor {
        sets.push(LayerSet::Anchor(new.anchor));
    }
    if was.margins != new.margins {
        sets.push(LayerSet::Margins(new.margins));
    }
    if was.exclusive_zone != new.exclusive_zone {
        sets.push(LayerSet::ExclusiveZone(new.exclusive_zone));
    }
    if was.keyboard != new.keyboard {
        sets.push(LayerSet::Keyboard(new.keyboard));
    }
    if was.compositor_size != new.compositor_size {
        sets.push(LayerSet::CompositorSize(new.compositor_size));
    }
    LayerUpdate::Apply(sets)
}

/// The changes in the fork's terms: one [`LayerShellChanges`] carrying every
/// one of them, so they reach the compositor in one commit.
///
/// [`LayerShellChanges`]: winit::platform::wayland::LayerShellChanges
#[cfg(target_os = "linux")]
pub(crate) fn winit_changes(sets: &[LayerSet]) -> winit::platform::wayland::LayerShellChanges {
    let mut changes = winit::platform::wayland::LayerShellChanges::default();
    for set in sets {
        match *set {
            LayerSet::Layer(l) => changes.layer = Some(winit_layer(l)),
            LayerSet::Anchor(a) => changes.anchor = Some(winit_anchor(a)),
            LayerSet::Margins(m) => changes.margins = Some(winit_margins(m)),
            LayerSet::ExclusiveZone(z) => changes.exclusive_zone = Some(z.protocol_value()),
            LayerSet::Keyboard(k) => changes.keyboard_interactivity = Some(winit_keyboard(k)),
            LayerSet::CompositorSize(c) => changes.compositor_size = Some(winit_axes(c)),
        }
    }
    changes
}

/// Make the changes on the live window, in **one** commit
/// (`set_layer_shell_changes`): the layer-shell state is double-buffered, so
/// the compositor goes from the old configuration to the new one with no
/// state in between. Nothing to change sends nothing.
pub(crate) fn apply_layer_sets(window: &dyn winit::window::Window, sets: &[LayerSet]) {
    #[cfg(target_os = "linux")]
    if !sets.is_empty() {
        use winit::platform::wayland::WindowExtWayland;
        window.set_layer_shell_changes(winit_changes(sets));
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (window, sets);
}

/// Where a compositor-centred surface of `size` sits on an output of
/// `output` size (both logical): the start of a move from that state.
pub(crate) fn centred_position(output: (f64, f64), size: (f64, f64)) -> (f64, f64) {
    (
        ((output.0 - size.0) / 2.0).max(0.0).round(),
        ((output.1 - size.1) / 2.0).max(0.0).round(),
    )
}

/// The smallest size a resize gesture leaves, logical.
pub(crate) const MIN_SIZE: f64 = 16.0;

/// What a gesture changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GestureKind {
    Move,
    Resize(ResizeDirection),
}

/// A move or resize of a layer surface that follows the pointer from the
/// press that started it to the release (issue #1523). Positions are the
/// surface's top-left on its output, sizes its surface size, both logical.
///
/// The pointer is reported relative to the surface, and the surface moves
/// under it, so the pointer's place on the output is `applied + local`,
/// where `applied` is the position the compositor last applied (the
/// position committed with the last frame): a position set but not yet
/// committed has not moved the surface the pointer is reported against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Gesture {
    pub kind: GestureKind,
    /// The pointer on the output at the start.
    start_pointer: (f64, f64),
    start_position: (f64, f64),
    start_size: (f64, f64),
}

impl Gesture {
    /// Start at the pointer's `local` position over a surface at `position`
    /// of `size`.
    pub(crate) fn start(
        kind: GestureKind,
        local: (f64, f64),
        position: (f64, f64),
        size: (f64, f64),
    ) -> Self {
        Gesture {
            kind,
            start_pointer: (position.0 + local.0, position.1 + local.1),
            start_position: position,
            start_size: size,
        }
    }

    /// The position and size for the pointer at `local` over the surface
    /// as last applied at `applied`.
    pub(crate) fn update(
        &self,
        local: (f64, f64),
        applied: (f64, f64),
    ) -> ((f64, f64), (f64, f64)) {
        let dx = (applied.0 + local.0 - self.start_pointer.0).round();
        let dy = (applied.1 + local.1 - self.start_pointer.1).round();
        let (x0, y0) = self.start_position;
        let (w0, h0) = self.start_size;
        let dir = match self.kind {
            GestureKind::Move => return ((x0 + dx, y0 + dy), (w0, h0)),
            GestureKind::Resize(dir) => dir,
        };
        use ResizeDirection as D;
        let west = matches!(dir, D::West | D::NorthWest | D::SouthWest);
        let east = matches!(dir, D::East | D::NorthEast | D::SouthEast);
        let north = matches!(dir, D::North | D::NorthEast | D::NorthWest);
        let south = matches!(dir, D::South | D::SouthEast | D::SouthWest);
        // An edge that moves takes the position with it; the opposite edge
        // stays where it was, also once the size reaches its minimum.
        let (x, w) = if west {
            let w = (w0 - dx).max(MIN_SIZE);
            (x0 + w0 - w, w)
        } else if east {
            (x0, (w0 + dx).max(MIN_SIZE))
        } else {
            (x0, w0)
        };
        let (y, h) = if north {
            let h = (h0 - dy).max(MIN_SIZE);
            (y0 + h0 - h, h)
        } else if south {
            (y0, (h0 + dy).max(MIN_SIZE))
        } else {
            (y0, h0)
        };
        ((x, y), (w, h))
    }
}

/// Each window's surface kind, published for
/// [`current_window_surface_kind`](crate::windows::current_window_surface_kind),
/// by window (`WindowOf`, #2416). Kept while the window is hidden; a window
/// has none until it first exists.
static SURFACE_KIND: std::sync::Mutex<Vec<(WindowOf, WindowSurfaceKind)>> =
    std::sync::Mutex::new(Vec::new());

pub(crate) fn publish_surface_kind(of: WindowOf, kind: WindowSurfaceKind) {
    let mut kinds = SURFACE_KIND.lock().unwrap_or_else(|p| p.into_inner());
    match kinds.iter_mut().find(|(w, _)| *w == of) {
        Some((_, old)) => *old = kind,
        None => kinds.push((of, kind)),
    }
}

/// Window `of`'s surface kind.
pub(crate) fn published_surface_kind_of(of: WindowOf) -> Option<WindowSurfaceKind> {
    SURFACE_KIND
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .find(|(w, _)| *w == of)
        .map(|(_, k)| *k)
}

/// The current window's surface kind (`window_geometry::current_window`).
pub(crate) fn published_surface_kind() -> Option<WindowSurfaceKind> {
    published_surface_kind_of(crate::shell::window_geometry::current_window(|of| {
        published_surface_kind_of(of).is_some()
    }))
}

/// Forget window `of`'s surface kind: the window is gone.
pub(crate) fn forget_surface_kind(of: WindowOf) {
    SURFACE_KIND
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .retain(|(w, _)| *w != of);
}

/// The surface kind `window` got.
pub(crate) fn surface_kind_of(window: &dyn winit::window::Window) -> WindowSurfaceKind {
    #[cfg(target_os = "linux")]
    {
        use winit::platform::wayland::{WaylandSurfaceRole, WindowExtWayland};
        if window.wayland_surface_role() == Some(WaylandSurfaceRole::LayerShell) {
            return WindowSurfaceKind::LayerShell;
        }
    }
    let _ = window;
    WindowSurfaceKind::Toplevel
}

/// A layer surface's position state (#1523): where rinch has put it, and
/// where the compositor has it.
///
/// A move sets the margins and commits at once, but the compositor keeps
/// reporting the pointer over the surface's old place until it has processed
/// that commit. The winit fork follows each margin commit with a
/// `wl_display.sync` and delivers a `WindowEvent::Moved` when its callback
/// is done, in order with the pointer's events: a pointer position before it
/// is over the old place, one after it over the new one. So every position
/// is read against [`LayerState::applied`], the origin the last `Moved`
/// reported, and none is dropped or counted twice. Reading them against the
/// position last *set* instead added each move again for every position
/// already on its way, and a fast drag swung further each way until the
/// window was off every screen. That holds for a **move**. A west or north
/// resize also changes the size, which the compositor applies with a later
/// buffer than the margin commit the sync follows, so at high pointer rates
/// (about 8 ms between events) it can still land off (#1604).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct LayerState {
    /// Top-left on its output, logical: its margins, as last set. `None`
    /// while the compositor centres it (no position was given yet).
    pub position: Option<(f64, f64)>,
    /// Where the compositor has the surface: the origin pointer positions
    /// are relative to. Follows [`Self::position`] once a `Moved` confirms it.
    applied: Option<(f64, f64)>,
    /// A move or resize following the pointer.
    pub gesture: Option<Gesture>,
    /// The app's own [`LayerSurface`] (#2413): it places the surface by its
    /// anchors and margins, so rinch runs no gesture, never re-anchors it,
    /// and has no position for it ([`Self::position`] stays `None`).
    pub app_placed: bool,
}

impl LayerState {
    /// A surface created at `position` (`None`: centred by the compositor).
    pub(crate) fn new(position: Option<(f64, f64)>) -> Self {
        LayerState {
            position,
            applied: position,
            gesture: None,
            app_placed: false,
        }
    }

    /// A surface the app places itself ([`Self::app_placed`]).
    pub(crate) fn app_placed() -> Self {
        LayerState {
            app_placed: true,
            ..Default::default()
        }
    }

    /// The surface is moved to `position`; the compositor has it once its
    /// `Moved` arrives.
    pub(crate) fn place(&mut self, position: (f64, f64)) {
        self.position = Some(position);
    }

    /// A surface the compositor centred, found at `position` and anchored
    /// there: it is where the compositor already has it.
    pub(crate) fn place_centred(&mut self, position: (f64, f64)) {
        self.position = Some(position);
        self.applied = Some(position);
    }

    /// The compositor has processed a move: pointer positions from here on
    /// are over the surface at `origin` (logical).
    pub(crate) fn moved(&mut self, origin: (f64, f64)) {
        self.applied = Some(origin);
    }

    /// Where the compositor has the surface.
    pub(crate) fn applied(&self) -> Option<(f64, f64)> {
        self.applied.or(self.position)
    }

    /// The position and size the gesture gives for the pointer at `local`
    /// (logical, over the surface as the compositor has it); `None` when no
    /// gesture is running.
    pub(crate) fn follow(&self, local: (f64, f64)) -> Option<((f64, f64), (f64, f64))> {
        Some(self.gesture?.update(local, self.applied()?))
    }
}

/// The origin a `WindowEvent::Moved` reports for a layer surface, logical:
/// the fork sends the margins times the scale, and margins are whole logical
/// pixels.
pub(crate) fn moved_origin(physical: (i32, i32), scale: f64) -> (f64, f64) {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    (
        (physical.0 as f64 / scale).round(),
        (physical.1 as f64 / scale).round(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #2416: each window's surface kind is its own, read by its document's
    /// code; everything else reads the main window's.
    #[test]
    fn each_window_publishes_and_reads_its_own_surface_kind() {
        // Keys of this test's own: the main window's kind is process-wide,
        // and other tests publish it.
        publish_surface_kind(Some(9), WindowSurfaceKind::Toplevel);
        publish_surface_kind(Some(11), WindowSurfaceKind::LayerShell);
        {
            let _doc = rinch_core::push_dispatching_doc(9);
            assert_eq!(published_surface_kind(), Some(WindowSurfaceKind::Toplevel));
        }
        {
            let _doc = rinch_core::push_dispatching_doc(11);
            assert_eq!(
                published_surface_kind(),
                Some(WindowSurfaceKind::LayerShell)
            );
        }
        forget_surface_kind(Some(9));
        forget_surface_kind(Some(11));
        assert_eq!(published_surface_kind_of(Some(9)), None);
        let _doc = rinch_core::push_dispatching_doc(9);
        assert_eq!(
            published_surface_kind(),
            published_surface_kind_of(None),
            "gone: the main window's"
        );
    }

    /// A compositor model for [`LayerState`]: it holds the surface at an
    /// origin, reports the pointer relative to it, and applies the client's
    /// margin commits in order, each followed by its `Moved`.
    struct Compositor {
        origin: (f64, f64),
        /// Margin commits not processed yet.
        commits: std::collections::VecDeque<(f64, f64)>,
    }

    impl Compositor {
        fn local(&self, pointer: (f64, f64)) -> (f64, f64) {
            (pointer.0 - self.origin.0, pointer.1 - self.origin.1)
        }
        /// Process one commit; the client reads its `Moved`.
        fn apply_one(&mut self, state: &mut LayerState) -> bool {
            let Some(o) = self.commits.pop_front() else {
                return false;
            };
            self.origin = o;
            state.moved(moved_origin(
                ((o.0 * 1.5).round() as i32, (o.1 * 1.5).round() as i32),
                1.5,
            ));
            true
        }
    }

    /// The client's side of one pointer position: follow it, commit a move.
    fn pointer_at(state: &mut LayerState, comp: &mut Compositor, pointer: (f64, f64)) {
        let (p, _) = state.follow(comp.local(pointer)).expect("a gesture");
        if Some(p) != state.position {
            state.place(p);
            comp.commits.push_back(p);
        }
    }

    /// Drag the grip at `grab` (over the surface at `start`) through `steps`
    /// of `step`, the compositor processing `lag` commits behind; then the
    /// rest. Returns where the surface ends and the largest distance it was
    /// put from where the pointer wanted it.
    fn drag(steps: usize, step: (f64, f64), lag: usize) -> ((f64, f64), f64) {
        let start = (80.0, 60.0);
        let grab = (16.0, 22.0);
        let mut state = LayerState::new(Some(start));
        state.gesture = Some(Gesture::start(GestureKind::Move, grab, start, (84.0, 84.0)));
        let mut comp = Compositor {
            origin: start,
            commits: Default::default(),
        };
        let mut pointer = (start.0 + grab.0, start.1 + grab.1);
        let mut worst: f64 = 0.0;
        for _ in 0..steps {
            pointer = (pointer.0 + step.0, pointer.1 + step.1);
            pointer_at(&mut state, &mut comp, pointer);
            let want = (pointer.0 - grab.0, pointer.1 - grab.1);
            let p = state.position.unwrap();
            worst = worst.max((p.0 - want.0).abs().max((p.1 - want.1).abs()));
            while comp.commits.len() > lag {
                comp.apply_one(&mut state);
            }
        }
        while comp.apply_one(&mut state) {}
        (comp.origin, worst)
    }

    /// Every position is read against where the compositor had the surface
    /// when it sent it, so a drag lands where the pointer ends at every
    /// lag between commit and `Moved`: none (a slow mouse), a few, and all
    /// of them (one burst read in one batch). Reading against the position
    /// last set put the burst at 18380,11040 (the runaway); dropping the
    /// positions read before a `Moved` left it 590 px short.
    #[test]
    fn a_drag_lands_under_the_pointer_at_every_lag() {
        for lag in [0, 1, 2, 3, 7, 60] {
            let (end, worst) = drag(60, (10.0, 6.0), lag);
            assert_eq!(end, (680.0, 420.0), "lag {lag}");
            assert_eq!(
                worst, 0.0,
                "lag {lag}: every move goes where the pointer is"
            );
        }
    }

    #[test]
    fn a_moved_origin_is_the_logical_margins() {
        assert_eq!(moved_origin((150, 90), 1.5), (100.0, 60.0));
        assert_eq!(moved_origin((101, 61), 1.0), (101.0, 61.0));
        assert_eq!(moved_origin((250, 175), 1.25), (200.0, 140.0));
        assert_eq!(moved_origin((7, 9), 0.0), (7.0, 9.0), "a zero scale is 1x");
    }

    #[test]
    fn a_centred_surface_is_applied_where_it_is_found() {
        let mut state = LayerState::new(None);
        assert_eq!(state.applied(), None);
        state.place_centred((598.0, 358.0));
        assert_eq!(state.applied(), Some((598.0, 358.0)));
        state.place((600.0, 360.0));
        assert_eq!(
            state.applied(),
            Some((598.0, 358.0)),
            "a move is not where the compositor has the surface until its Moved"
        );
        state.moved((600.0, 360.0));
        assert_eq!(state.applied(), Some((600.0, 360.0)));
    }

    fn props(non_activating: bool, layer: Option<bool>) -> WindowProps {
        WindowProps {
            non_activating,
            wayland_layer_shell: layer,
            ..Default::default()
        }
    }

    /// The choice: a non-activating window on Wayland, unless told
    /// otherwise; never anywhere else. `skip_taskbar` alone keeps the
    /// decorated toplevel.
    #[test]
    fn a_layer_surface_is_chosen_for_a_non_activating_wayland_window_only() {
        let w = Backend::Wayland;
        assert!(uses_layer_shell(&props(true, None), w));
        assert!(!uses_layer_shell(&props(false, None), w));
        assert!(uses_layer_shell(&props(false, Some(true)), w));
        assert!(!uses_layer_shell(&props(true, Some(false)), w));
        let skip = WindowProps {
            skip_taskbar: true,
            ..Default::default()
        };
        assert!(!uses_layer_shell(&skip, w));
        for b in [
            Backend::X11,
            Backend::Windows,
            Backend::MacOs,
            Backend::Other,
        ] {
            assert!(!uses_layer_shell(&props(true, Some(true)), b), "{b:?}");
        }
    }

    /// Keyboard: none for a non-activating window, on-demand for a forced
    /// focusable one.
    #[test]
    fn the_keyboard_follows_non_activating() {
        assert!(!LayerRequest::new(&props(true, None), None, None, "a").keyboard);
        assert!(LayerRequest::new(&props(false, Some(true)), None, None, "a").keyboard);
        // Forced and non-activating: still no keyboard.
        assert!(!LayerRequest::new(&props(true, Some(true)), None, None, "a").keyboard);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_position_anchors_top_left_with_margins_and_none_centres() {
        use winit::platform::wayland::{LayerShellAnchor, LayerShellKeyboardInteractivity};
        let r = LayerRequest::new(
            &props(true, None),
            Some((130.4, 57.6)),
            Some("DP-1".into()),
            "app",
        );
        let a = r.to_winit();
        assert_eq!(a.anchor, LayerShellAnchor::TOP_LEFT);
        assert_eq!((a.margins.left, a.margins.top), (130, 58));
        assert_eq!(a.output_name.as_deref(), Some("DP-1"));
        assert_eq!(a.namespace, "app");
        assert_eq!(
            a.keyboard_interactivity,
            LayerShellKeyboardInteractivity::None
        );
        let c = LayerRequest::new(&props(true, None), None, None, "app").to_winit();
        assert_eq!(c.anchor, LayerShellAnchor::default());
    }

    /// A move keeps the grabbed point under the pointer: off the identity,
    /// the surface at (100, 50), grabbed at (10, 7), pointer moved by
    /// (+33, -12) — reported against the surface as last applied.
    #[test]
    fn a_move_follows_the_pointer_against_the_applied_position() {
        let g = Gesture::start(GestureKind::Move, (10.0, 7.0), (100.0, 50.0), (200.0, 80.0));
        // Not applied yet: the surface is still at (100, 50).
        assert_eq!(
            g.update((43.0, -5.0), (100.0, 50.0)),
            ((133.0, 38.0), (200.0, 80.0))
        );
        // Applied: the same pointer is reported at the grab point again.
        assert_eq!(
            g.update((10.0, 7.0), (133.0, 38.0)),
            ((133.0, 38.0), (200.0, 80.0))
        );
    }

    /// Each edge: the opposite edge stays put, also at the minimum size.
    #[test]
    fn a_resize_moves_only_the_dragged_edges() {
        use ResizeDirection as D;
        let start = |d| {
            Gesture::start(
                GestureKind::Resize(d),
                (0.0, 0.0),
                (100.0, 50.0),
                (200.0, 80.0),
            )
        };
        let at = (100.0, 50.0);
        // East/south grow from the top-left.
        assert_eq!(
            start(D::SouthEast).update((25.0, 9.0), at),
            ((100.0, 50.0), (225.0, 89.0))
        );
        // West: the right edge (300) stays.
        assert_eq!(
            start(D::West).update((-30.0, 4.0), at),
            ((70.0, 50.0), (230.0, 80.0))
        );
        // North at the minimum: the bottom edge (130) stays.
        assert_eq!(
            start(D::North).update((0.0, 500.0), at),
            ((100.0, 130.0 - MIN_SIZE), (200.0, MIN_SIZE))
        );
        // East and south at the minimum: the top-left stays.
        assert_eq!(
            start(D::SouthEast).update((-500.0, -500.0), at),
            ((100.0, 50.0), (MIN_SIZE, MIN_SIZE))
        );
        assert_eq!(
            start(D::NorthWest).update((10.0, 10.0), at),
            ((110.0, 60.0), (190.0, 70.0))
        );
    }

    /// The layer follows the level: `top` up to `Floating` (`always_on_top`
    /// included), `overlay` above.
    #[test]
    fn the_layer_follows_the_window_level() {
        let at = |level| WindowProps {
            level,
            ..props(true, None)
        };
        assert!(!wants_overlay_layer(&at(WindowLevel::Normal)));
        assert!(!wants_overlay_layer(&WindowProps {
            always_on_top: true,
            ..at(WindowLevel::Normal)
        }));
        assert!(!wants_overlay_layer(&at(WindowLevel::Floating)));
        assert!(wants_overlay_layer(&at(WindowLevel::StatusBar)));
        assert!(wants_overlay_layer(&at(WindowLevel::ScreenSaver)));
    }

    #[test]
    fn a_centred_surface_starts_its_move_from_the_centre() {
        assert_eq!(
            centred_position((1920.0, 1080.0), (300.0, 41.0)),
            (810.0, 520.0)
        );
        assert_eq!(centred_position((100.0, 100.0), (300.0, 41.0)), (0.0, 30.0));
    }

    // ---- The app's own surface (#2413) ----

    /// A full-width bottom bar: what `examples/wayland-bar` asks for.
    fn bar() -> LayerSurface {
        LayerSurface {
            anchor: LayerAnchor::BOTTOM | LayerAnchor::LEFT | LayerAnchor::RIGHT,
            exclusive_zone: LayerExclusiveZone::Reserve(32),
            compositor_size: LayerAxes::WIDTH,
            output: Some("DP-1".into()),
            ..Default::default()
        }
    }

    fn with_layer(surface: LayerSurface, base: WindowProps) -> WindowProps {
        WindowProps {
            wayland_layer: Some(surface),
            ..base
        }
    }

    /// `wayland_layer` wins over the shorthands: a layer surface whatever
    /// `wayland_layer_shell` says (`Some(false)` included), on Wayland only.
    #[test]
    fn an_explicit_layer_surface_wins_over_the_shorthands() {
        let w = Backend::Wayland;
        for (non_activating, shell) in [
            (false, None),
            (true, None),
            (false, Some(false)),
            (true, Some(false)),
            (false, Some(true)),
        ] {
            let p = with_layer(bar(), props(non_activating, shell));
            assert!(uses_layer_shell(&p, w), "{non_activating} {shell:?}");
            for b in [
                Backend::X11,
                Backend::Windows,
                Backend::MacOs,
                Backend::Other,
            ] {
                assert!(!uses_layer_shell(&p, b), "{b:?}");
            }
        }
    }

    /// The surface is the app's exactly: `non_activating` does not take its
    /// keyboard, `level` does not move its layer, `x`/`y` are not its
    /// margins and `WindowProps::output` is not its output.
    #[test]
    fn an_explicit_layer_surface_is_sent_as_given() {
        let surface = LayerSurface {
            keyboard: LayerKeyboard::Exclusive,
            layer: LayerShellLayer::Bottom,
            margins: LayerMargins {
                top: 0,
                right: 4,
                bottom: 6,
                left: 4,
            },
            ..bar()
        };
        let p = with_layer(
            surface.clone(),
            WindowProps {
                non_activating: true,
                level: WindowLevel::ScreenSaver,
                x: Some(300),
                y: Some(200),
                output: Some("HDMI-A-1".into()),
                ..Default::default()
            },
        );
        let spec = LayerSpec::for_window(&p, Some((300.0, 200.0)), None, "deskrinch-bar");
        assert_eq!(spec.surface, surface);
        assert_eq!(spec.namespace, "deskrinch-bar");
    }

    /// An output chosen since (`set_current_window_output`, or none after
    /// the compositor closed the surface) replaces the surface's own.
    #[test]
    fn a_chosen_output_replaces_the_surfaces_own() {
        let p = with_layer(bar(), WindowProps::default());
        let o = |output| LayerSpec::for_window(&p, None, output, "a").surface.output;
        assert_eq!(o(None), Some("DP-1".into()));
        assert_eq!(o(Some(Some("eDP-1".into()))), Some("eDP-1".into()));
        assert_eq!(o(Some(None)), None);
    }

    /// The shorthands are exactly the overlay configuration they always
    /// were, now spelled as a `LayerSurface`.
    #[test]
    fn the_shorthands_are_todays_overlay_configuration() {
        let placed = LayerSpec::for_window(&props(true, None), Some((130.4, 57.6)), None, "app");
        assert_eq!(
            placed.surface,
            LayerSurface {
                layer: LayerShellLayer::Top,
                anchor: LayerAnchor::TOP | LayerAnchor::LEFT,
                margins: LayerMargins {
                    top: 58,
                    left: 130,
                    right: 0,
                    bottom: 0
                },
                exclusive_zone: LayerExclusiveZone::KeepClear,
                keyboard: LayerKeyboard::None,
                output: None,
                compositor_size: LayerAxes::NONE,
            }
        );
        let raised = WindowProps {
            level: WindowLevel::StatusBar,
            output: Some("DP-2".into()),
            ..props(false, Some(true))
        };
        let centred = LayerSpec::for_window(&raised, None, None, "app").surface;
        assert_eq!(centred.layer, LayerShellLayer::Overlay);
        assert_eq!(centred.anchor, LayerAnchor::NONE);
        assert_eq!(centred.keyboard, LayerKeyboard::OnDemand);
        assert_eq!(centred.output, Some("DP-2".into()));
        let moved = LayerSpec::for_window(&raised, None, Some(Some("DP-3".into())), "app");
        assert_eq!(moved.surface.output, Some("DP-3".into()));
    }

    /// The overlay reaches the fork exactly as before: through `LayerSpec`
    /// and through `LayerRequest` the same attributes.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_shorthands_reach_the_fork_as_before() {
        use winit::platform::wayland::{
            LayerShellAnchor, LayerShellCompositorSize, LayerShellKeyboardInteractivity,
            LayerShellLayer as L,
        };
        let p = props(true, None);
        let r = LayerRequest::new(&p, Some((10.0, 20.0)), Some("DP-1".into()), "app");
        let spec = LayerSpec::for_window(
            &WindowProps {
                output: Some("DP-1".into()),
                ..p
            },
            Some((10.0, 20.0)),
            None,
            "app",
        );
        let a = spec.to_winit();
        assert_eq!(a, r.to_winit());
        assert_eq!(a.layer, L::Top);
        assert_eq!(a.anchor, LayerShellAnchor::TOP_LEFT);
        assert_eq!(a.exclusive_zone, 0);
        assert_eq!(
            a.keyboard_interactivity,
            LayerShellKeyboardInteractivity::None
        );
        assert_eq!(a.compositor_size, LayerShellCompositorSize::default());
    }

    /// An explicit surface in the fork's terms: the bar reserves 32 px at
    /// the bottom and leaves its width to the compositor.
    #[cfg(target_os = "linux")]
    #[test]
    fn an_explicit_surface_reaches_the_fork_field_for_field() {
        use winit::platform::wayland::{LayerShellCompositorSize, LayerShellKeyboardInteractivity};
        let a = surface_to_winit(
            &LayerSurface {
                keyboard: LayerKeyboard::OnDemand,
                layer: LayerShellLayer::Background,
                ..bar()
            },
            "bar",
        );
        assert!(a.anchor.bottom && a.anchor.left && a.anchor.right && !a.anchor.top);
        assert_eq!(a.exclusive_zone, 32);
        assert_eq!(
            a.compositor_size,
            LayerShellCompositorSize {
                width: true,
                height: false
            }
        );
        assert_eq!(a.output_name.as_deref(), Some("DP-1"));
        assert_eq!(a.namespace, "bar");
        assert_eq!(
            a.layer,
            winit::platform::wayland::LayerShellLayer::Background
        );
        assert_eq!(
            a.keyboard_interactivity,
            LayerShellKeyboardInteractivity::OnDemand
        );
    }

    #[test]
    fn exclusive_zones_are_the_protocols_values() {
        assert_eq!(LayerExclusiveZone::KeepClear.protocol_value(), 0);
        assert_eq!(LayerExclusiveZone::Ignore.protocol_value(), -1);
        assert_eq!(LayerExclusiveZone::Reserve(48).protocol_value(), 48);
        assert_eq!(
            LayerExclusiveZone::Reserve(u32::MAX).protocol_value(),
            i32::MAX
        );
    }

    /// The compositor assigns an axis only where it is anchored to both
    /// edges; asked anywhere else, the axis is named for the warning.
    #[test]
    fn only_an_axis_anchored_to_both_edges_is_the_compositors() {
        let b = bar();
        assert_eq!(b.assigned_axes(), LayerAxes::WIDTH);
        assert_eq!(unassignable_axes(&b), LayerAxes::NONE);
        let both = LayerSurface {
            compositor_size: LayerAxes::BOTH,
            ..bar()
        };
        assert_eq!(both.assigned_axes(), LayerAxes::WIDTH);
        assert_eq!(unassignable_axes(&both), LayerAxes::HEIGHT);
        let wallpaper = LayerSurface {
            anchor: LayerAnchor::ALL,
            compositor_size: LayerAxes::BOTH,
            ..Default::default()
        };
        assert_eq!(wallpaper.assigned_axes(), LayerAxes::BOTH);
        let corner = LayerSurface {
            anchor: LayerAnchor::TOP | LayerAnchor::RIGHT,
            compositor_size: LayerAxes::BOTH,
            ..Default::default()
        };
        assert_eq!(corner.assigned_axes(), LayerAxes::NONE);
        assert_eq!(unassignable_axes(&corner), LayerAxes::BOTH);
    }

    /// At run time every field but the output goes through its setter, in
    /// a fixed order; nothing changed is nothing sent.
    #[test]
    fn a_runtime_change_is_one_setter_per_changed_field() {
        assert_eq!(layer_update(&bar(), &bar()), LayerUpdate::Apply(vec![]));
        let taller = LayerSurface {
            exclusive_zone: LayerExclusiveZone::Reserve(48),
            ..bar()
        };
        assert_eq!(
            layer_update(&bar(), &taller),
            LayerUpdate::Apply(vec![LayerSet::ExclusiveZone(LayerExclusiveZone::Reserve(
                48
            ))])
        );
        let top = LayerSurface {
            layer: LayerShellLayer::Overlay,
            anchor: LayerAnchor::TOP | LayerAnchor::LEFT | LayerAnchor::RIGHT,
            margins: LayerMargins {
                top: 8,
                ..Default::default()
            },
            keyboard: LayerKeyboard::Exclusive,
            compositor_size: LayerAxes::NONE,
            ..taller.clone()
        };
        assert_eq!(
            layer_update(&taller, &top),
            LayerUpdate::Apply(vec![
                LayerSet::Layer(LayerShellLayer::Overlay),
                LayerSet::Anchor(top.anchor),
                LayerSet::Margins(top.margins),
                LayerSet::Keyboard(LayerKeyboard::Exclusive),
                LayerSet::CompositorSize(LayerAxes::NONE),
            ])
        );
    }

    /// A new output makes the surface again: the protocol fixes it when the
    /// surface is made. Any other change with it rides along.
    #[test]
    fn a_new_output_makes_the_surface_again() {
        for output in [Some("HDMI-A-1".to_string()), None] {
            let moved = LayerSurface {
                output,
                exclusive_zone: LayerExclusiveZone::Ignore,
                ..bar()
            };
            assert_eq!(layer_update(&bar(), &moved), LayerUpdate::Recreate);
        }
    }

    /// The app places its own surface: a fresh state has no position and no
    /// gesture, and is marked so the shell starts none.
    #[test]
    fn an_app_placed_surface_has_no_position() {
        let s = LayerState::app_placed();
        assert!(s.app_placed);
        assert_eq!((s.position, s.applied(), s.gesture), (None, None, None));
        assert_eq!(s.follow((5.0, 5.0)), None);
        assert!(!LayerState::new(Some((1.0, 2.0))).app_placed);
    }

    /// A move to the other edge with a new margin and zone is one set of
    /// changes for the fork: one commit, every property in it.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_change_of_several_properties_is_one_set_of_changes() {
        let moved = LayerSurface {
            anchor: LayerAnchor::TOP | LayerAnchor::LEFT | LayerAnchor::RIGHT,
            margins: LayerMargins {
                top: 6,
                ..Default::default()
            },
            exclusive_zone: LayerExclusiveZone::Reserve(48),
            ..bar()
        };
        let LayerUpdate::Apply(sets) = layer_update(&bar(), &moved) else {
            panic!("no output change");
        };
        assert_eq!(sets.len(), 3);
        let c = winit_changes(&sets);
        let a = c.anchor.expect("the anchors");
        assert!(a.top && a.left && a.right && !a.bottom);
        assert_eq!(c.margins.map(|m| m.top), Some(6));
        assert_eq!(c.exclusive_zone, Some(48));
        assert_eq!(
            (c.layer, c.keyboard_interactivity, c.compositor_size),
            (None, None, None),
            "what did not change is not sent"
        );
    }
}
