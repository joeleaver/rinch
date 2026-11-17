//! The performance counters of each of the app's windows (#2657).
//!
//! Every window's document keeps its own counters (`rinch_dom::perf`), and
//! every window that paints ends one performance frame per paint (and one per
//! preparation, `prepare_current_window`). [`windows`] reads them: one
//! [`WindowPerf`] per open window, the main one first. Under `RINCH_PERF` the
//! main window's frames are printed as they always were, `[PERF] frame N:`,
//! and every other window's as `[PERF] window <id> frame N:` (and
//! `[PERF] window <id> prepared N:`), numbered in that window's own sequence;
//! `<id>` is its [`WindowHandle::id`]. A window opened while `RINCH_PERF` is
//! set prints one line naming it first:
//!
//! ```text
//! [PERF] window 3 opened: title="Panel" app_id="org.example.panel" output="DP-2"
//! ```
//!
//! (`app_id` and `output` only when its props name them; `output` is
//! `WindowProps::output`, else its layer surface's). A layer surface placed by
//! [`LayerSurface::output_id`](rinch_core::element::LayerSurface::output_id)
//! (#2658) adds `output_id=<n>`, the id as a bare number, after `output`,
//! which is then that output's name from `outputs()` and left out when it has
//! none: `output="DP-2" output_id=52`, or `output_id=52` alone. Then
//! `[PERF] window 3 closed` when it closes. The `perf_stats` debug command reports the same
//! list under `windows`. Desktop only: an embedded `RinchContext` keeps its
//! document's counters itself, and the web has none.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use rinch_dom::RinchDocument;
use rinch_dom::perf::FrameStats;

use crate::WindowHandle;

/// One window's counters, as [`windows`] reads them.
#[derive(Debug, Clone)]
pub struct WindowPerf {
    /// The window.
    pub window: WindowHandle,
    /// Whether it is the app's main window (whose `RINCH_PERF` lines carry no
    /// window id).
    pub main: bool,
    /// Its title when it was opened.
    pub title: String,
    /// Its `WindowProps::app_id`, if it has one.
    pub app_id: Option<String>,
    /// The output its props name (`WindowProps::output`, else its layer
    /// surface's), if they name one. For a layer surface placed by
    /// `output_id`, that output's name, when it has one.
    pub output: Option<String>,
    /// The output id its layer surface is placed by
    /// ([`LayerSurface::output_id`](rinch_core::element::LayerSurface::output_id)),
    /// which tells an unnamed output apart (#2658).
    pub output_id: Option<rinch_core::element::OutputId>,
    /// Its frames ended since the last reset, prepared frames aside.
    pub frames: u64,
    /// Its prepared frames (painted while hidden, not presented).
    pub prepared_frames: u64,
    /// Its last ended frame.
    pub last_frame: FrameStats,
    /// Every counter since the last reset, the current frame included.
    pub total: FrameStats,
}

struct Entry {
    id: u64,
    main: bool,
    title: String,
    app_id: Option<String>,
    output: Option<String>,
    output_id: Option<rinch_core::element::OutputId>,
    doc: Weak<RefCell<RinchDocument>>,
}

thread_local! {
    static WINDOWS: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
}

/// Every open window's counters, the main window first, then the others in
/// the order they were opened. A window whose document is borrowed right now
/// (only possible from inside its own event handling) is left out.
pub fn windows() -> Vec<WindowPerf> {
    WINDOWS.with(|w| {
        let mut w = w.borrow_mut();
        w.retain(|e| e.doc.strong_count() > 0);
        w.iter()
            .filter_map(|e| {
                let doc = e.doc.upgrade()?;
                let d = doc.try_borrow().ok()?;
                let perf = &d.tree.perf;
                Some(WindowPerf {
                    window: WindowHandle::from_id(e.id),
                    main: e.main,
                    title: e.title.clone(),
                    app_id: e.app_id.clone(),
                    output: e.output.clone(),
                    output_id: e.output_id,
                    frames: perf.frames(),
                    prepared_frames: perf.prepared(),
                    last_frame: perf.last_frame(),
                    total: perf.total(),
                })
            })
            .collect()
    })
}

/// Zero every open window's counters, as `perf_stats { reset: true }` does.
pub fn reset_windows() {
    let docs: Vec<Rc<RefCell<RinchDocument>>> =
        WINDOWS.with(|w| w.borrow().iter().filter_map(|e| e.doc.upgrade()).collect());
    for doc in docs {
        if let Ok(mut d) = doc.try_borrow_mut() {
            d.tree.perf.reset();
        }
    }
}

/// [`windows`] as the `perf_stats` debug command reports it.
#[cfg(feature = "debug")]
pub(crate) fn windows_json() -> serde_json::Value {
    serde_json::Value::Array(
        windows()
            .into_iter()
            .map(|w| {
                serde_json::json!({
                    "window": w.window.id(),
                    "main": w.main,
                    "title": w.title,
                    "app_id": w.app_id,
                    "output": w.output,
                    "output_id": w.output_id.map(|id| id.raw()),
                    "frames": w.frames,
                    "prepared_frames": w.prepared_frames,
                    "last_frame": w.last_frame.to_json(),
                    "total": w.total.to_json(),
                })
            })
            .collect(),
    )
}

/// Register window `id`'s document (the shell, once it is mounted): it is
/// listed by [`windows`] while the document lives. Another window's document
/// is labelled `window <id>` for its `RINCH_PERF` lines, and its identity line
/// printed under `RINCH_PERF`.
pub(crate) fn register(
    id: u64,
    main: bool,
    props: Option<&rinch_core::element::WindowProps>,
    doc: &Rc<RefCell<RinchDocument>>,
) {
    let title = props.map(|p| p.title.clone()).unwrap_or_default();
    let app_id = props.and_then(|p| p.app_id.clone());
    let (output, output_id) = match props {
        Some(p) => output_of(p, &rinch_core::reactive::untracked(crate::outputs::outputs)),
        None => (None, None),
    };
    if !main {
        doc.borrow_mut()
            .tree
            .perf
            .set_label(Some(format!("window {id}")));
        if rinch_dom::perf::perf_env_enabled() {
            eprintln!(
                "{}",
                opened_line(id, &title, app_id.as_deref(), output.as_deref(), output_id)
            );
        }
    }
    WINDOWS.with(|w| {
        let mut w = w.borrow_mut();
        w.retain(|e| e.id != id && e.doc.strong_count() > 0);
        let entry = Entry {
            id,
            main,
            title,
            app_id,
            output,
            output_id,
            doc: Rc::downgrade(doc),
        };
        if main {
            w.insert(0, entry);
        } else {
            w.push(entry);
        }
    });
}

/// Window `id` closed: it leaves [`windows`], and under `RINCH_PERF` says so.
pub(crate) fn closed(id: u64) {
    let was = WINDOWS.with(|w| {
        let mut w = w.borrow_mut();
        let before = w.len();
        w.retain(|e| e.id != id);
        before != w.len()
    });
    if was && rinch_dom::perf::perf_env_enabled() {
        eprintln!("[PERF] window {id} closed");
    }
}

/// The output a window with `props` names, and the id it is placed by:
/// `WindowProps::output`, else its layer surface's; a layer surface placed by
/// `output_id` (which wins over its own name) is named by that output's name
/// in `outputs`, none when it has none or is gone.
fn output_of(
    props: &rinch_core::element::WindowProps,
    outputs: &[crate::outputs::Output],
) -> (Option<String>, Option<rinch_core::element::OutputId>) {
    let layer = props.wayland_layer.as_ref();
    let output_id = layer.and_then(|l| l.output_id);
    let layer_name = match output_id {
        Some(id) => outputs
            .iter()
            .find(|o| o.id == id)
            .and_then(|o| o.name.clone()),
        None => layer.and_then(|l| l.output.clone()),
    };
    (props.output.clone().or(layer_name), output_id)
}

/// The line naming a window when it opens (see the module docs).
fn opened_line(
    id: u64,
    title: &str,
    app_id: Option<&str>,
    output: Option<&str>,
    output_id: Option<rinch_core::element::OutputId>,
) -> String {
    let mut line = format!("[PERF] window {id} opened: title={title:?}");
    if let Some(app_id) = app_id {
        line.push_str(&format!(" app_id={app_id:?}"));
    }
    if let Some(output) = output {
        line.push_str(&format!(" output={output:?}"));
    }
    if let Some(output_id) = output_id {
        line.push_str(&format!(" output_id={output_id}"));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_opened_line_names_the_window() {
        assert_eq!(
            opened_line(3, "Panel", Some("org.example.panel"), Some("DP-2"), None),
            r#"[PERF] window 3 opened: title="Panel" app_id="org.example.panel" output="DP-2""#
        );
        assert_eq!(
            opened_line(4, "a \"b\"", None, None, None),
            r#"[PERF] window 4 opened: title="a \"b\"""#
        );
    }

    /// A panel placed by `output_id` (#2658) is named by its output's name
    /// and its id, so "the panel on DP-2" is found, and an unnamed output
    /// is still told apart by its id.
    #[test]
    fn a_window_placed_by_output_id_names_the_output_and_its_id() {
        use crate::outputs::Output;
        use rinch_core::element::{LayerSurface, OutputId, WindowProps};
        let output = |id, name: Option<&str>| Output {
            id: OutputId::from_raw(id),
            name: name.map(str::to_owned),
            position: (0.0, 0.0),
            size: (1920.0, 1080.0),
            scale: 1.0,
        };
        let outputs = [output(51, Some("DP-2")), output(52, None)];
        let placed = |id, name: Option<&str>| WindowProps {
            wayland_layer: Some(LayerSurface {
                output_id: Some(OutputId::from_raw(id)),
                output: name.map(str::to_owned),
                ..Default::default()
            }),
            ..Default::default()
        };
        let line = |props: &WindowProps| {
            let (output, id) = output_of(props, &outputs);
            opened_line(3, "Panel", None, output.as_deref(), id)
        };
        assert_eq!(
            line(&placed(51, None)),
            r#"[PERF] window 3 opened: title="Panel" output="DP-2" output_id=51"#
        );
        assert_eq!(
            line(&placed(52, None)),
            r#"[PERF] window 3 opened: title="Panel" output_id=52"#
        );
        // The id wins over the surface's own name, as it does in placement.
        assert_eq!(
            line(&placed(51, Some("HDMI-A-1"))),
            r#"[PERF] window 3 opened: title="Panel" output="DP-2" output_id=51"#
        );
        // By name alone: as before, no id.
        let by_name = WindowProps {
            wayland_layer: Some(LayerSurface {
                output: Some("HDMI-A-1".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            line(&by_name),
            r#"[PERF] window 3 opened: title="Panel" output="HDMI-A-1""#
        );
    }
}
