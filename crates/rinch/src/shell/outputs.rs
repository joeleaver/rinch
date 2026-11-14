//! Reading the connected monitors from winit: for placing and restoring the
//! window (`window_geometry::MonitorRect`, #1884) and for the public output
//! list (`crate::outputs`, #2414). Both come from one reading of each
//! monitor ([`MonitorReading`]), so the name an app reads in an
//! [`Output`] is the one `WindowProps::monitor` is matched against.

use winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::monitor::MonitorHandle;

#[cfg(target_os = "linux")]
use super::overlay_window;
use super::overlay_window::Backend;
use super::window_geometry::MonitorRect;
use crate::outputs::{Output, OutputId, OutputTracking};

/// The unit a platform positions windows in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Units {
    /// Desktop (physical) pixels: X11, Windows, Wayland.
    DesktopPixels,
    /// Points: macOS.
    Points,
}

impl Units {
    /// This target's.
    fn of_target() -> Units {
        if cfg!(target_os = "macos") {
            Units::Points
        } else {
            Units::DesktopPixels
        }
    }
}

/// What winit, and the platform beside it, say of one monitor.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MonitorReading {
    /// `MonitorHandle::native_id`: the `wl_output` global's name on Wayland,
    /// the RandR CRTC on X11, the `HMONITOR` on Windows, the
    /// `CGDirectDisplayID` on macOS. [`Output::id`](crate::outputs::Output::id).
    pub id: u64,
    /// `MonitorHandle::name`.
    pub name: Option<String>,
    /// `MonitorHandle::position`: desktop pixels; on Wayland the logical
    /// position times the integer `wl_output.scale`.
    pub position: Option<PhysicalPosition<i32>>,
    /// The current video mode's size (unrotated on X11 and Wayland).
    pub mode: Option<PhysicalSize<u32>>,
    /// `MonitorHandle::scale_factor`: on Wayland the integer `wl_output.scale`.
    pub scale: f64,
    /// The platform's own rectangle ([`native_monitor_rect`]).
    pub native: Option<(PhysicalPosition<i32>, PhysicalSize<u32>)>,
    /// Wayland: `xdg_output`'s logical position and size.
    pub logical: Option<(LogicalPosition<i32>, LogicalSize<i32>)>,
}

impl MonitorReading {
    fn of(m: &MonitorHandle, backend: Backend) -> MonitorReading {
        MonitorReading {
            id: m.native_id(),
            name: m.name().map(|n| n.into_owned()),
            position: m.position(),
            mode: m.current_video_mode().map(|v| v.size()),
            scale: m.scale_factor(),
            native: native_monitor_rect(m, backend),
            logical: wayland_logical_rect(m, backend),
        }
    }

    /// The monitor in the unit windows are positioned in: desktop pixels, or
    /// points on macOS (given scale 1, so a logical position is compared as
    /// the points it is). `None` when winit gives no position or mode and
    /// the platform no rectangle.
    pub(crate) fn rect(&self, units: Units) -> Option<MonitorRect> {
        let name = self.name.clone();
        if units == Units::Points {
            // Points, the unit AppKit positions windows in.
            let (p, s) = match self.native {
                Some(rect) => rect,
                None => {
                    let p = self.position?.to_logical::<f64>(self.scale);
                    let s = self.mode?.to_logical::<f64>(self.scale);
                    (
                        PhysicalPosition::new(p.x.round() as i32, p.y.round() as i32),
                        PhysicalSize::new(s.width.round() as u32, s.height.round() as u32),
                    )
                }
            };
            return Some(MonitorRect::of(p, s, 1.0, name, None));
        }
        if self.native.is_none() && (self.position.is_none() || self.mode.is_none()) {
            return None;
        }
        Some(MonitorRect::of(
            self.position.unwrap_or_default(),
            self.mode.unwrap_or_default(),
            self.scale,
            name,
            self.native,
        ))
    }

    /// The monitor as the public list gives it: logical pixels (see
    /// `crate::outputs`, "Units"). The name is [`rect`](Self::rect)'s.
    pub(crate) fn output(&self, units: Units) -> Option<Output> {
        if let Some((position, size)) = self.logical {
            return Some(self.wayland_output(position, size));
        }
        let rect = self.rect(units)?;
        let scale = match units {
            Units::DesktopPixels => rect.scale,
            Units::Points => 1.0,
        };
        Some(Output {
            id: OutputId::from_raw(self.id),
            name: rect.name,
            position: (
                f64::from(rect.position.x) / scale,
                f64::from(rect.position.y) / scale,
            ),
            size: (
                f64::from(rect.size.width) / scale,
                f64::from(rect.size.height) / scale,
            ),
            // macOS's rectangle is in points at scale 1; the output's scale
            // is the backing scale.
            scale: self.scale,
        })
    }

    /// A Wayland output: `xdg_output`'s logical rectangle, and the scale the
    /// compositor renders it at, its mode over its logical size. The long
    /// sides are compared, so a rotated output (whose mode winit gives
    /// unrotated, and whose logical size is rotated) has the same scale.
    /// Rounded to 1/120, the fractional-scale protocol's unit; winit's
    /// integer `wl_output.scale` (2 for 1.5) when there is no mode.
    fn wayland_output(&self, position: LogicalPosition<i32>, size: LogicalSize<i32>) -> Output {
        let long_side = size.width.max(size.height);
        let scale = match self.mode {
            Some(mode) if long_side > 0 => {
                let ratio = f64::from(mode.width.max(mode.height)) / f64::from(long_side);
                (ratio * 120.0).round() / 120.0
            }
            _ => self.scale,
        };
        Output {
            id: OutputId::from_raw(self.id),
            name: self.name.clone(),
            position: (f64::from(position.x), f64::from(position.y)),
            size: (f64::from(size.width), f64::from(size.height)),
            scale,
        }
    }
}

/// Each connected monitor, read once.
fn readings(event_loop: &dyn ActiveEventLoop) -> Vec<MonitorReading> {
    let backend = Backend::of(event_loop);
    event_loop
        .available_monitors()
        .map(|m| MonitorReading::of(&m, backend))
        .collect()
}

/// The monitors connected now, in the unit windows are positioned in:
/// desktop pixels, or points on macOS (given scale 1). Each monitor's
/// rectangle is the platform's own where rinch can read it
/// ([`native_monitor_rect`]): winit gives only a position and the current
/// video mode, whose size on X11 ignores rotation.
pub(crate) fn connected_monitors(event_loop: &dyn ActiveEventLoop) -> Vec<MonitorRect> {
    let units = Units::of_target();
    readings(event_loop)
        .iter()
        .filter_map(|r| r.rect(units))
        .collect()
}

/// Whether the shell hears of every monitor change on `backend`: winit calls
/// `ApplicationHandler::monitors_changed` on Wayland and X11 only.
pub(crate) fn tracking_of(backend: Backend) -> OutputTracking {
    match backend {
        Backend::Wayland | Backend::X11 => OutputTracking::Live,
        Backend::Windows | Backend::MacOs | Backend::Other => OutputTracking::Snapshot,
    }
}

/// Read the outputs and publish them to `crate::outputs`: at the event
/// loop's start and on each `monitors_changed`.
pub(crate) fn publish(event_loop: &dyn ActiveEventLoop) {
    let units = Units::of_target();
    let list = readings(event_loop)
        .iter()
        .filter_map(|r| r.output(units))
        .collect();
    crate::outputs::publish(list, tracking_of(Backend::of(event_loop)));
}

/// Wayland: `xdg_output`'s logical position and size (the winit fork's
/// `MonitorHandleExtWayland`). `None` elsewhere, and on a compositor with no
/// `zxdg_output_manager_v1`.
#[allow(unused_variables)]
fn wayland_logical_rect(
    m: &MonitorHandle,
    backend: Backend,
) -> Option<(LogicalPosition<i32>, LogicalSize<i32>)> {
    #[cfg(target_os = "linux")]
    if backend == Backend::Wayland {
        use winit::platform::wayland::MonitorHandleExtWayland;
        return Some((m.logical_position()?, m.logical_size()?));
    }
    None
}

/// A monitor's rectangle on the desktop as the platform keeps it: the RandR
/// CRTC on X11 (rotation included), `rcMonitor` on Windows (physical
/// desktop pixels), `CGDisplayBounds` on macOS (points). `None` where it
/// cannot be read, and on Wayland, which gives no desktop coordinates.
#[allow(unused_variables)]
fn native_monitor_rect(
    m: &MonitorHandle,
    backend: Backend,
) -> Option<(PhysicalPosition<i32>, PhysicalSize<u32>)> {
    #[cfg(target_os = "linux")]
    if backend == Backend::X11 {
        let (x, y, w, h) = overlay_window::x11_crtc_rect(m.native_id() as u32)?;
        return Some((PhysicalPosition::new(x, y), PhysicalSize::new(w, h)));
    }
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO};
        // SAFETY: a zeroed MONITORINFO with its size set is what
        // GetMonitorInfoW fills; the handle is winit's HMONITOR.
        let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        let ok = unsafe { GetMonitorInfoW(m.native_id() as usize as HMONITOR, &mut info) };
        if ok == 0 {
            return None;
        }
        let r = info.rcMonitor;
        return Some((
            PhysicalPosition::new(r.left, r.top),
            PhysicalSize::new(
                (r.right - r.left).max(0) as u32,
                (r.bottom - r.top).max(0) as u32,
            ),
        ));
    }
    #[cfg(target_os = "macos")]
    {
        #[repr(C)]
        struct CGPoint {
            x: f64,
            y: f64,
        }
        #[repr(C)]
        struct CGSize {
            width: f64,
            height: f64,
        }
        #[repr(C)]
        struct CGRect {
            origin: CGPoint,
            size: CGSize,
        }
        #[link(name = "CoreGraphics", kind = "framework")]
        unsafe extern "C" {
            fn CGDisplayBounds(display: u32) -> CGRect;
        }
        // SAFETY: CGDisplayBounds takes any display id (an unknown one gives
        // an empty rect) and returns a plain struct.
        let r = unsafe { CGDisplayBounds(m.native_id() as u32) };
        if r.size.width <= 0.0 || r.size.height <= 0.0 {
            return None;
        }
        return Some((
            PhysicalPosition::new(r.origin.x.round() as i32, r.origin.y.round() as i32),
            PhysicalSize::new(r.size.width.round() as u32, r.size.height.round() as u32),
        ));
    }
    #[allow(unreachable_code)]
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::window_geometry::place_requested;
    use winit::dpi::{LogicalSize as Logical, Size};

    fn reading(name: &str) -> MonitorReading {
        MonitorReading {
            id: 7,
            name: Some(name.to_owned()),
            position: Some(PhysicalPosition::new(0, 0)),
            mode: Some(PhysicalSize::new(1920, 1080)),
            scale: 1.0,
            native: None,
            logical: None,
        }
    }

    fn wayland(
        mode: (u32, u32),
        int_scale: f64,
        at: (i32, i32),
        size: (i32, i32),
    ) -> MonitorReading {
        MonitorReading {
            // winit-wayland's position: the logical one times the integer scale.
            position: Some(PhysicalPosition::new(
                (f64::from(at.0) * int_scale) as i32,
                (f64::from(at.1) * int_scale) as i32,
            )),
            mode: Some(PhysicalSize::new(mode.0, mode.1)),
            scale: int_scale,
            logical: Some((
                LogicalPosition::new(at.0, at.1),
                LogicalSize::new(size.0, size.1),
            )),
            ..reading("DP-1")
        }
    }

    #[test]
    fn a_desktop_pixel_monitor_is_its_rectangle_over_its_scale() {
        // X11 or Windows: a 3840x2160 monitor right of a 1920-wide one, at
        // scale 2; the platform's own rectangle wins over winit's.
        let r = MonitorReading {
            position: Some(PhysicalPosition::new(1920, 0)),
            mode: Some(PhysicalSize::new(2160, 3840)),
            scale: 2.0,
            native: Some((
                PhysicalPosition::new(1920, 0),
                PhysicalSize::new(3840, 2160),
            )),
            ..reading("DP-2")
        };
        let rect = r.rect(Units::DesktopPixels).unwrap();
        assert_eq!(rect.size, PhysicalSize::new(3840, 2160));
        assert_eq!(
            r.output(Units::DesktopPixels),
            Some(Output {
                id: OutputId::from_raw(7),
                name: Some("DP-2".into()),
                position: (960.0, 0.0),
                size: (1920.0, 1080.0),
                scale: 2.0,
            })
        );
    }

    #[test]
    fn a_macos_display_is_in_points_with_its_backing_scale() {
        let r = MonitorReading {
            id: 69_733_382,
            name: Some("Monitor #41001".into()),
            position: Some(PhysicalPosition::new(0, 0)),
            mode: Some(PhysicalSize::new(3024, 1964)),
            scale: 2.0,
            native: Some((PhysicalPosition::new(0, 0), PhysicalSize::new(1512, 982))),
            logical: None,
        };
        // The placement list keeps points at scale 1.
        assert_eq!(r.rect(Units::Points).unwrap().scale, 1.0);
        assert_eq!(
            r.output(Units::Points),
            Some(Output {
                id: OutputId::from_raw(69_733_382),
                name: Some("Monitor #41001".into()),
                position: (0.0, 0.0),
                size: (1512.0, 982.0),
                scale: 2.0,
            })
        );
        // No CGDisplayBounds: winit's position and mode, over the scale.
        let r = MonitorReading { native: None, ..r };
        assert_eq!(r.output(Units::Points).unwrap().size, (1512.0, 982.0));
    }

    #[test]
    fn a_wayland_output_is_xdg_outputs_logical_rectangle_at_its_fractional_scale() {
        // 3840x2160 at 1.5: wl_output.scale says 2, winit's position is the
        // logical one doubled, and the mode over 2 would be 1920x1080.
        let r = wayland((3840, 2160), 2.0, (1920, 0), (2560, 1440));
        assert_eq!(
            r.output(Units::DesktopPixels),
            Some(Output {
                id: OutputId::from_raw(7),
                name: Some("DP-1".into()),
                position: (1920.0, 0.0),
                size: (2560.0, 1440.0),
                scale: 1.5,
            })
        );
        // Rotated: the mode is unrotated, the logical size is not.
        let rotated = wayland((3840, 2160), 2.0, (0, 0), (1440, 2560));
        let o = rotated.output(Units::DesktopPixels).unwrap();
        assert_eq!((o.size, o.scale), ((1440.0, 2560.0), 1.5));
        // A logical size the compositor rounded (1366 / 1.25 = 1092.8) still
        // reads as the scale that was set, to 1/120.
        let rounded = wayland((1366, 768), 2.0, (0, 0), (1093, 614));
        assert_eq!(rounded.output(Units::DesktopPixels).unwrap().scale, 1.25);
        // Integer scales are exact.
        let one = wayland((1920, 1080), 1.0, (0, 0), (1920, 1080));
        assert_eq!(one.output(Units::DesktopPixels).unwrap().scale, 1.0);
        // No mode: winit's integer scale.
        let no_mode = MonitorReading {
            mode: None,
            ..wayland((1, 1), 2.0, (0, 0), (2560, 1440))
        };
        assert_eq!(no_mode.output(Units::DesktopPixels).unwrap().scale, 2.0);
    }

    #[test]
    fn a_wayland_output_without_xdg_output_falls_back_to_winits_reading() {
        let r = MonitorReading {
            position: Some(PhysicalPosition::new(3840, 0)),
            mode: Some(PhysicalSize::new(3840, 2160)),
            scale: 2.0,
            ..reading("DP-1")
        };
        assert_eq!(
            r.output(Units::DesktopPixels),
            Some(Output {
                id: OutputId::from_raw(7),
                name: Some("DP-1".into()),
                position: (1920.0, 0.0),
                size: (1920.0, 1080.0),
                scale: 2.0,
            })
        );
    }

    #[test]
    fn a_monitor_winit_cannot_place_is_left_out_of_both_lists() {
        let r = MonitorReading {
            mode: None,
            ..reading("DP-1")
        };
        assert_eq!(r.rect(Units::DesktopPixels), None);
        assert_eq!(r.output(Units::DesktopPixels), None);
    }

    /// The name an app reads in an `Output` is the string `WindowProps::monitor`
    /// and the layer-shell `WindowProps::output` take: one reading gives both
    /// lists their name, `place_requested` finds the monitor by it, and the
    /// layer request hands it to the winit fork unchanged (which matches it
    /// against sctk's `OutputInfo::name`, the field winit-wayland's
    /// `MonitorHandle::name` answers).
    #[test]
    fn an_outputs_name_is_what_window_props_monitor_and_output_accept() {
        let left = reading("DP-1");
        // A scale-2 monitor right of it: (1000, 100) is a point on both.
        let right = MonitorReading {
            position: Some(PhysicalPosition::new(1920, 0)),
            mode: Some(PhysicalSize::new(3840, 2160)),
            scale: 2.0,
            ..reading("HDMI-A-1")
        };
        let units = Units::DesktopPixels;
        let rects: Vec<MonitorRect> = [&left, &right]
            .iter()
            .filter_map(|r| r.rect(units))
            .collect();
        let outputs: Vec<Output> = [&left, &right]
            .iter()
            .filter_map(|r| r.output(units))
            .collect();
        for (rect, output) in rects.iter().zip(&outputs) {
            assert_eq!(rect.name, output.name);
        }
        let size = Size::Logical(Logical::new(400.0, 300.0));
        for output in &outputs {
            let name = output.name.as_deref();
            let placed = place_requested((1000.0, 100.0), size, name, &rects).unwrap();
            assert_eq!(placed.monitor.as_deref(), name);
        }

        #[cfg(target_os = "linux")]
        {
            use crate::shell::layer_shell::LayerSpec;
            use rinch_core::element::{LayerSurface, WindowProps};
            let name = outputs[1].name.clone();
            // The overlay shorthand (`WindowProps::output`), a full layer
            // surface (`WindowProps::wayland_layer`), and an output chosen at
            // run time (`set_current_window_output`) all hand it on as is.
            let shorthand = WindowProps {
                output: name.clone(),
                ..Default::default()
            };
            let layer = WindowProps {
                wayland_layer: Some(LayerSurface {
                    output: name.clone(),
                    ..Default::default()
                }),
                ..Default::default()
            };
            for spec in [
                LayerSpec::for_window(&shorthand, None, None, "app"),
                LayerSpec::for_window(&layer, None, None, "app"),
                LayerSpec::for_window(&WindowProps::default(), None, Some(name.clone()), "app"),
            ] {
                assert_eq!(spec.to_winit().output_name.as_deref(), Some("HDMI-A-1"));
            }
        }
    }

    /// An output with no name still has its id in the public list, and a
    /// layer surface given that id reaches the winit fork as the `wl_output`
    /// global's name, which the fork matches against sctk's `OutputInfo::id`
    /// (what winit-wayland's `MonitorHandle::native_id` answers; #2658).
    #[test]
    fn an_unnamed_output_has_an_id_a_layer_surface_is_placed_by() {
        let unnamed = MonitorReading {
            id: 42,
            name: None,
            ..wayland((1920, 1080), 1.0, (1920, 0), (1920, 1080))
        };
        let output = unnamed.output(Units::DesktopPixels).unwrap();
        assert_eq!(
            (output.id, output.name.clone()),
            (OutputId::from_raw(42), None)
        );

        #[cfg(target_os = "linux")]
        {
            use crate::shell::layer_shell::LayerSpec;
            use rinch_core::element::{LayerSurface, WindowProps};
            let props = WindowProps {
                wayland_layer: Some(LayerSurface {
                    output_id: Some(output.id),
                    ..Default::default()
                }),
                ..Default::default()
            };
            let attrs = LayerSpec::for_window(&props, None, None, "app").to_winit();
            assert_eq!((attrs.output_id, attrs.output_name), (Some(42), None));
        }
    }

    #[test]
    fn wayland_and_x11_are_followed_and_the_others_listed_once() {
        assert_eq!(tracking_of(Backend::Wayland), OutputTracking::Live);
        assert_eq!(tracking_of(Backend::X11), OutputTracking::Live);
        assert_eq!(tracking_of(Backend::Windows), OutputTracking::Snapshot);
        assert_eq!(tracking_of(Backend::MacOs), OutputTracking::Snapshot);
        assert_eq!(tracking_of(Backend::Other), OutputTracking::Snapshot);
    }
}
