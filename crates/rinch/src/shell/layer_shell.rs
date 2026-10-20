//! Wayland overlay windows as `wlr-layer-shell` surfaces (issue #1523).
//!
//! An xdg-toplevel cannot refuse keyboard focus, a taskbar entry, or ask to
//! stay above other windows, so on native Wayland a window that asks for that
//! — [`WindowProps::non_activating`], or [`WindowProps::wayland_layer_shell`]
//! `Some(true)` — is given a layer-shell role instead, through rinch's winit
//! fork (`WindowAttributesWayland::with_layer_shell`). Everything else about
//! the window (buffers, scale, input, resize, the renderers) is unchanged.
//!
//! What the window gets:
//!
//! - **Layer**: `top`, or `overlay` for a window raised above status bars
//!   ([`wants_overlay_layer`]: `WindowProps::level` above `Floating`).
//! - **Keyboard**: none for a non-activating window, on-demand otherwise.
//! - **Position**: anchored to the output's top-left corner, with the
//!   position as the top and left margins, in logical pixels relative to the
//!   output. A window with no position yet is not anchored: the compositor
//!   centres it, and the first move reads where that is
//!   ([`centred_position`]).
//! - **Move / resize**: the compositor has none for a layer surface, so a
//!   drag region and resize edges run a [`Gesture`] that follows the pointer
//!   and sets margins and size.
//!
//! When the compositor has no `zwlr_layer_shell_v1` (GNOME/Mutter) the fork
//! makes an ordinary xdg-toplevel, and the window behaves as before #1523.

use std::sync::atomic::{AtomicU8, Ordering};

pub use rinch_core::element::WindowSurfaceKind;
use rinch_core::element::{WindowLevel, WindowProps};
use rinch_platform::ResizeDirection;

use super::overlay_window::Backend;

/// Whether a window with `props` is a layer surface on `backend`.
pub(crate) fn uses_layer_shell(props: &WindowProps, backend: Backend) -> bool {
    backend == Backend::Wayland && props.wayland_layer_shell.unwrap_or(props.non_activating)
}

/// Whether the window goes in the `overlay` layer (above panels and
/// fullscreen windows) rather than `top`: a level above `Floating` (#1549).
/// `Normal` is in `top` too, since a layer surface is an overlay.
pub(crate) fn wants_overlay_layer(props: &WindowProps) -> bool {
    props.effective_level() > WindowLevel::Floating
}

/// What the layer surface is created with. Pure, so the choice is tested
/// without a compositor; turned into the fork's attributes by
/// [`LayerRequest::to_winit`].
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

    #[cfg(target_os = "linux")]
    pub(crate) fn to_winit(&self) -> winit::platform::wayland::LayerShellAttributes {
        use winit::platform::wayland::{
            LayerShellAnchor, LayerShellAttributes, LayerShellKeyboardInteractivity,
            LayerShellLayer,
        };
        LayerShellAttributes {
            layer: if self.overlay {
                LayerShellLayer::Overlay
            } else {
                LayerShellLayer::Top
            },
            keyboard_interactivity: if self.keyboard {
                LayerShellKeyboardInteractivity::OnDemand
            } else {
                LayerShellKeyboardInteractivity::None
            },
            anchor: if self.position.is_some() {
                LayerShellAnchor::TOP_LEFT
            } else {
                LayerShellAnchor::default()
            },
            margins: margins(self.position.unwrap_or((0.0, 0.0))),
            exclusive_zone: 0,
            output_name: self.output.clone(),
            namespace: self.namespace.clone(),
        }
    }
}

/// The top/left margins for a top-left position, logical.
#[cfg(target_os = "linux")]
pub(crate) fn margins(position: (f64, f64)) -> winit::platform::wayland::LayerShellMargins {
    winit::platform::wayland::LayerShellMargins {
        top: position.1.round() as i32,
        left: position.0.round() as i32,
        right: 0,
        bottom: 0,
    }
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

/// The main window's surface kind, published for
/// [`current_window_surface_kind`](crate::windows::current_window_surface_kind):
/// 0 none yet, 1 toplevel, 2 layer shell. Kept while the window is hidden.
static SURFACE_KIND: AtomicU8 = AtomicU8::new(0);

pub(crate) fn publish_surface_kind(kind: WindowSurfaceKind) {
    let v = match kind {
        WindowSurfaceKind::Toplevel => 1,
        WindowSurfaceKind::LayerShell => 2,
    };
    SURFACE_KIND.store(v, Ordering::Relaxed);
}

pub(crate) fn published_surface_kind() -> Option<WindowSurfaceKind> {
    match SURFACE_KIND.load(Ordering::Relaxed) {
        1 => Some(WindowSurfaceKind::Toplevel),
        2 => Some(WindowSurfaceKind::LayerShell),
        _ => None,
    }
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
/// window was off every screen.
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
}

impl LayerState {
    /// A surface created at `position` (`None`: centred by the compositor).
    pub(crate) fn new(position: Option<(f64, f64)>) -> Self {
        LayerState {
            position,
            applied: position,
            gesture: None,
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
            assert_eq!(worst, 0.0, "lag {lag}: every move goes where the pointer is");
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
}
