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
//! `WindowProps::output`, else its layer surface's), and `[PERF] window 3
//! closed` when it closes. The `perf_stats` debug command reports the same
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
    /// surface's), if they name one.
    pub output: Option<String>,
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
    let output = props.and_then(|p| {
        p.output
            .clone()
            .or_else(|| p.wayland_layer.as_ref().and_then(|l| l.output.clone()))
    });
    if !main {
        doc.borrow_mut()
            .tree
            .perf
            .set_label(Some(format!("window {id}")));
        if rinch_dom::perf::perf_env_enabled() {
            eprintln!(
                "{}",
                opened_line(id, &title, app_id.as_deref(), output.as_deref())
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

/// The line naming a window when it opens (see the module docs).
fn opened_line(id: u64, title: &str, app_id: Option<&str>, output: Option<&str>) -> String {
    let mut line = format!("[PERF] window {id} opened: title={title:?}");
    if let Some(app_id) = app_id {
        line.push_str(&format!(" app_id={app_id:?}"));
    }
    if let Some(output) = output {
        line.push_str(&format!(" output={output:?}"));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_opened_line_names_the_window() {
        assert_eq!(
            opened_line(3, "Panel", Some("org.example.panel"), Some("DP-2")),
            r#"[PERF] window 3 opened: title="Panel" app_id="org.example.panel" output="DP-2""#
        );
        assert_eq!(
            opened_line(4, "a \"b\"", None, None),
            r#"[PERF] window 4 opened: title="a \"b\"""#
        );
    }
}
