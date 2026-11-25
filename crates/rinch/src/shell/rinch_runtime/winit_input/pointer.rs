//! winit's pointers in rinch's terms: pointer sources and ids, pressure,
//! buttons held, wheels, the cursor icon, and the OS file drag's host.

use super::*;

// ── Pointer sources ──────────────────────────────────────────────────────────

/// Touch pointers are numbered from here, so a finger never shares an id
/// with the mouse (1) or a tablet tool (2).
pub(in crate::shell::rinch_runtime) const TOUCH_POINTER_BASE: u64 = 1 << 32;

/// The [`PlatformEvent`] a winit wheel delta becomes, with the pointer at the
/// logical `cursor` on a window of `scale`: what both windows of the desktop
/// shell send (#2287).
///
/// - A `LineDelta` — a mouse wheel's notches — is a
///   [`PlatformEvent::MouseWheelLines`], whose default scroll glides as
///   Chrome's does.
/// - A `PixelDelta` — a trackpad, a high-resolution wheel — is a
///   [`PlatformEvent::MouseWheel`], which scrolls directly. The delta is a
///   *physical* distance compared against logical scroll extents, so it
///   divides like a position does, or a HiDPI trackpad scrolls `scale` times
///   too far (#299).
/// - Any other kind (`MouseScrollDelta` is non-exhaustive) is `None`: it
///   scrolls nothing.
pub fn wheel_platform_event(
    delta: winit::event::MouseScrollDelta,
    cursor: (f32, f32),
    scale: f64,
) -> Option<PlatformEvent> {
    let (x, y) = cursor;
    match delta {
        winit::event::MouseScrollDelta::LineDelta(lines_x, lines_y) => {
            Some(PlatformEvent::MouseWheelLines {
                x,
                y,
                lines_x: lines_x as f64,
                lines_y: lines_y as f64,
            })
        }
        winit::event::MouseScrollDelta::PixelDelta(pos) => {
            let (delta_x, delta_y) = to_logical_point((pos.x, pos.y), scale);
            Some(PlatformEvent::MouseWheel {
                x,
                y,
                delta_x,
                delta_y,
            })
        }
        _ => None,
    }
}

/// A winit force as a pressure in `0.0..=1.0`; with none measured, the
/// Pointer Events default (`0.5` while down, else `0`).
fn force_pressure(force: Option<&winit::event::Force>, down: bool) -> f32 {
    match force {
        Some(f) => (f.normalized(None) as f32).clamp(0.0, 1.0),
        None if down => 0.5,
        None => 0.0,
    }
}

/// A tablet tool's kind as a surface pointer's: an eraser is an eraser, a
/// puck or lens a mouse, a finger on the tablet a touch, every stylus a pen.
fn tool_kind(kind: winit::event::TabletToolKind) -> SurfacePointerKind {
    match kind {
        winit::event::TabletToolKind::Eraser => SurfacePointerKind::Eraser,
        winit::event::TabletToolKind::Mouse | winit::event::TabletToolKind::Lens => {
            SurfacePointerKind::Mouse
        }
        winit::event::TabletToolKind::Finger => SurfacePointerKind::Touch,
        _ => SurfacePointerKind::Pen,
    }
}

/// The one id a tablet tool has: winit numbers neither tools nor tablets.
pub(super) const TABLET_POINTER_ID: u64 = 2;

/// [`SurfacePointer::id`] of the pointer a winit source describes.
pub(in crate::shell::rinch_runtime) fn pointer_id_of(source: &winit::event::PointerSource) -> u64 {
    match source {
        winit::event::PointerSource::Touch { finger_id, .. } => {
            TOUCH_POINTER_BASE + finger_id.into_raw() as u64
        }
        winit::event::PointerSource::TabletTool { .. } => TABLET_POINTER_ID,
        _ => SurfacePointer::mouse(false).id,
    }
}

/// `os_file_drag`'s steps carried out on winit's event loop (#1962); the
/// events are collected and handed to the app after, in order.
pub(in crate::shell::rinch_runtime) struct WinitDragHost<'a> {
    pub(in crate::shell::rinch_runtime) event_loop: &'a dyn ActiveEventLoop,
    pub(in crate::shell::rinch_runtime) events: Vec<PlatformEvent>,
}

impl crate::shell::rinch_runtime::os_file_drag::DragHost for WinitDragHost<'_> {
    fn accept(
        &mut self,
        id: winit::data_transfer::DataTransferId,
        actions: &[winit::event_loop::DndAction],
    ) -> Result<(), String> {
        self.event_loop
            .set_valid_dnd_actions(id, actions)
            .map_err(|e| e.to_string())
    }

    fn fetch_uri_list(
        &mut self,
        id: winit::data_transfer::DataTransferId,
    ) -> Result<winit::event_loop::AsyncRequestSerial, String> {
        self.event_loop
            .fetch_data_transfer(id, &winit::data_transfer::TypeHint::UriList)
            .map_err(|e| e.to_string())
    }

    fn emit(&mut self, event: PlatformEvent) {
        self.events.push(event);
    }

    fn warn(&mut self, message: String) {
        tracing::warn!("{message}");
    }
}

/// The pointer a winit move came from, as a render surface hears it. `down`:
/// that pointer has a button or contact down, which decides the pressure a
/// device that cannot measure it reports.
pub(in crate::shell::rinch_runtime) fn surface_pointer_of(
    source: &winit::event::PointerSource,
    primary: bool,
    down: bool,
) -> SurfacePointer {
    use winit::event::PointerSource;
    match source {
        PointerSource::Mouse | PointerSource::Unknown => SurfacePointer {
            primary,
            ..SurfacePointer::mouse(down)
        },
        PointerSource::Touch { finger_id, force } => SurfacePointer {
            id: TOUCH_POINTER_BASE + finger_id.into_raw() as u64,
            kind: SurfacePointerKind::Touch,
            pressure: force_pressure(force.as_ref(), down),
            primary,
        },
        PointerSource::TabletTool { kind, data } => SurfacePointer {
            id: TABLET_POINTER_ID,
            kind: tool_kind(*kind),
            pressure: force_pressure(data.force.as_ref(), down),
            primary,
        },
        // `PointerSource` is non-exhaustive (winit 0.31.0-beta.3): a source
        // this shell does not know is a mouse, as `Unknown` is.
        _ => SurfacePointer {
            primary,
            ..SurfacePointer::mouse(down)
        },
    }
}

/// [`surface_pointer_of`] for a winit button event. `pressed`: the button
/// went down (a release reports no pressure).
pub(in crate::shell::rinch_runtime) fn surface_pointer_of_button(
    button: &winit::event::ButtonSource,
    primary: bool,
    pressed: bool,
) -> SurfacePointer {
    use winit::event::{ButtonSource, PointerSource};
    let source = match button {
        ButtonSource::Mouse(_) | ButtonSource::Unknown(_) => PointerSource::Mouse,
        ButtonSource::Touch { finger_id, force } => PointerSource::Touch {
            finger_id: *finger_id,
            force: *force,
        },
        ButtonSource::TabletTool { kind, data, .. } => PointerSource::TabletTool {
            kind: *kind,
            data: data.clone(),
        },
        // Non-exhaustive (winit 0.31.0-beta.3): an unknown source is a mouse.
        _ => PointerSource::Mouse,
    };
    let mut pointer = surface_pointer_of(&source, primary, pressed);
    if !pressed {
        pointer.pressure = 0.0;
    }
    pointer
}

/// The buttons or contacts down, as `(SurfacePointer::id, button)`: what
/// decides the pressure a move reports for a device that cannot measure it.
/// Per button, so a chord's release (right up while left is held) leaves the
/// pointer down.
#[derive(Debug, Default)]
pub(in crate::shell::rinch_runtime) struct PressedButtons(Vec<(u64, Option<MouseButton>)>);

impl PressedButtons {
    /// `button` of pointer `id` went down (`pressed`) or up.
    pub(in crate::shell::rinch_runtime) fn note(
        &mut self,
        id: u64,
        button: Option<MouseButton>,
        pressed: bool,
    ) {
        self.0.retain(|k| *k != (id, button));
        if pressed {
            self.0.push((id, button));
        }
    }

    /// Whether pointer `id` has any button or contact down.
    pub(in crate::shell::rinch_runtime) fn is_down(&self, id: u64) -> bool {
        self.0.iter().any(|(p, _)| *p == id)
    }

    /// Whether the primary button (a mouse's left, a finger, a pen's tip) of
    /// any pointer is down.
    pub(in crate::shell::rinch_runtime) fn primary_down(&self) -> bool {
        self.0.iter().any(|(_, b)| *b == Some(MouseButton::Left))
    }

    /// Forget every button of pointer `id`; answers whether it had any down.
    pub(in crate::shell::rinch_runtime) fn lift(&mut self, id: u64) -> bool {
        let was = self.is_down(id);
        self.0.retain(|(p, _)| *p != id);
        was
    }

    /// Forget every press (the window lost focus).
    pub(in crate::shell::rinch_runtime) fn clear(&mut self) {
        self.0.clear();
    }
}

/// Resets the pointer `set_current_pointer` named when dropped.
pub(in crate::shell::rinch_runtime) struct ForgetCurrentPointer;

impl Drop for ForgetCurrentPointer {
    fn drop(&mut self) {
        crate::render_surface::set_current_pointer(None);
    }
}

impl RinchRuntime {
    /// Convert a platform CursorStyle to a winit CursorIcon.
    pub(in crate::shell::rinch_runtime) fn cursor_style_to_winit(
        style: rinch_platform::CursorStyle,
    ) -> winit::cursor::CursorIcon {
        use rinch_platform::CursorStyle as CS;
        use winit::cursor::CursorIcon;
        match style {
            CS::Auto | CS::Default => CursorIcon::Default,
            CS::Pointer => CursorIcon::Pointer,
            CS::Text => CursorIcon::Text,
            CS::Move => CursorIcon::Move,
            CS::NotAllowed => CursorIcon::NotAllowed,
            CS::Crosshair => CursorIcon::Crosshair,
            CS::Grab => CursorIcon::Grab,
            CS::Grabbing => CursorIcon::Grabbing,
            CS::ColResize => CursorIcon::ColResize,
            CS::RowResize => CursorIcon::RowResize,
            CS::NResize => CursorIcon::NResize,
            CS::SResize => CursorIcon::SResize,
            CS::EResize => CursorIcon::EResize,
            CS::WResize => CursorIcon::WResize,
            CS::NeResize => CursorIcon::NeResize,
            CS::NwResize => CursorIcon::NwResize,
            CS::SeResize => CursorIcon::SeResize,
            CS::SwResize => CursorIcon::SwResize,
            CS::EwResize => CursorIcon::EwResize,
            CS::NsResize => CursorIcon::NsResize,
            CS::NeswResize => CursorIcon::NeswResize,
            CS::NwseResize => CursorIcon::NwseResize,
            CS::ZoomIn => CursorIcon::ZoomIn,
            CS::ZoomOut => CursorIcon::ZoomOut,
            CS::Wait => CursorIcon::Wait,
            CS::Progress => CursorIcon::Progress,
            CS::Help => CursorIcon::Help,
            CS::ContextMenu => CursorIcon::ContextMenu,
            CS::Cell => CursorIcon::Cell,
            CS::Copy => CursorIcon::Copy,
            CS::Alias => CursorIcon::Alias,
            CS::NoDrop => CursorIcon::NoDrop,
            CS::AllScroll => CursorIcon::AllScroll,
            CS::None => CursorIcon::Default, // winit doesn't have "none"
        }
    }
}
