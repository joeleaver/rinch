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
//!
//! A window's frames carry its own document's reactive work (#2798); the
//! work of no document is on the frames of the thread's only window, or,
//! with several, on one `[PERF] pass N` line per event-loop turn that had
//! some ([`end_pass`], read back by [`pass`]).
//!
//! A software window whose buffers the idle trim releases (#2843) prints
//! `[PERF] window 3 trimmed`, and the first present after it that allocates
//! them again `[PERF] window 3 untrimmed`, before that frame's own line
//! (#2852). The main window's are `[PERF] trimmed` and `[PERF] untrimmed`,
//! the form its frames take.
//!
//! Whether one will (#2917): a software window says, when its presentation is
//! made (before its first presented frame's line, and again when it is shown
//! again), `[PERF] window 3 idle_trim=2000ms`, the period after its last
//! present at which it trims, or `[PERF] window 3 idle_trim=never` (opened
//! with `IdleTrim::Never`, or on a surface that does not keep its contents,
//! which is not trimmed: X11, macOS); the main window's is
//! `[PERF] idle_trim=…`. A GPU-rendered window never trims and says nothing.

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

/// The work no window's frames carry, pass by pass (#2798): what ran under no
/// document while the thread has several. A window's frame carries its own
/// document's reactive work (its effects' runs, the signal writes made under
/// it, the `ReRender`s it queued); a write from `main`, a worker's queued
/// closure or a timer armed outside any document is nobody's, and is counted
/// here once, so the window lines and the pass lines together count every
/// unit of work exactly once. With one window, that window's frames carry
/// it, as they always have, and no pass is counted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PassPerf {
    /// Passes that had such work, since the last reset.
    pub passes: u64,
    /// The last of them.
    pub last_pass: FrameStats,
    /// All of them, since the last reset.
    pub total: FrameStats,
}

thread_local! {
    static PASS: std::cell::Cell<PassPerf> = std::cell::Cell::new(PassPerf::default());
}

/// The passes' counters (see [`PassPerf`]).
pub fn pass() -> PassPerf {
    PASS.with(|p| p.get())
}

/// End one pass of the event loop (the shell, at the end of each turn): the
/// work under no document since the last pass, when the thread has several
/// documents and there was some. Counted in [`pass`] and, under
/// `RINCH_PERF`, printed as `[PERF] pass N: …`, numbered in its own
/// sequence. `None`, and nothing printed, otherwise: a single window's
/// frames took that work themselves.
pub(crate) fn end_pass() -> Option<FrameStats> {
    if rinch_core::reactive::counted_doc_count() <= 1 {
        return None;
    }
    let taken = crate::app::take_unattributed_perf();
    if taken.iter().all(|&(_, n)| n == 0) {
        return None;
    }
    let counters = rinch_dom::perf::PerfCounters::default();
    for (counter, n) in taken {
        counters.add(counter, n);
    }
    let frame = counters.frame();
    let n = PASS.with(|p| {
        let mut pass = p.get();
        pass.passes += 1;
        pass.last_pass = frame;
        pass.total.accumulate(&frame);
        p.set(pass);
        pass.passes
    });
    if rinch_dom::perf::perf_env_enabled() {
        eprintln!("[PERF] pass {n}: {}", frame.summary_line());
    }
    Some(frame)
}

/// Zero every open window's counters, and the passes', as
/// `perf_stats { reset: true }` does.
pub fn reset_windows() {
    PASS.with(|p| p.set(PassPerf::default()));
    if rinch_core::reactive::counted_doc_count() > 1 {
        let _ = crate::app::take_unattributed_perf();
    }
    let docs: Vec<Rc<RefCell<RinchDocument>>> =
        WINDOWS.with(|w| w.borrow().iter().filter_map(|e| e.doc.upgrade()).collect());
    for doc in docs {
        if let Ok(mut d) = doc.try_borrow_mut() {
            d.tree.perf.reset();
        }
    }
}

/// [`pass`] as the `perf_stats` debug command reports it.
#[cfg(feature = "debug")]
pub(crate) fn pass_json() -> serde_json::Value {
    let p = pass();
    serde_json::json!({
        "passes": p.passes,
        "last_pass": p.last_pass.to_json(),
        "total": p.total.to_json(),
    })
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

/// Popup `id` opened (#2552): under `RINCH_PERF`, after its `opened` line,
/// `[PERF] window 5 popup: kind=menu parent=2`, so a popup's frames can be
/// told apart and its chain read.
pub(crate) fn popup_opened(id: u64, kind: rinch_core::element::PopupKind, parent: u64) {
    let line = popup_line(id, kind, parent);
    if rinch_dom::perf::perf_env_enabled() {
        eprintln!("{line}");
    }
    #[cfg(test)]
    capture::note(&line);
}

/// When a popup was asked for and when each step of its opening ended
/// (#2552), for its `first frame` line.
#[derive(Debug, Clone)]
pub(crate) struct PopupTimeline {
    asked: std::time::Instant,
    marks: Vec<(&'static str, std::time::Instant)>,
}

impl PopupTimeline {
    pub(crate) fn new(asked: std::time::Instant) -> Self {
        Self {
            asked,
            marks: Vec::new(),
        }
    }

    /// The step `what` ended now.
    pub(crate) fn mark(&mut self, what: &'static str) {
        self.marks.push((what, std::time::Instant::now()));
    }

    /// `12.3 ms after open_popup (queued 0.1, mount 3.0, ...)`: the total
    /// and each step's share, in the order they ended.
    pub(crate) fn describe(&self) -> String {
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
        let end = self.marks.last().map_or(self.asked, |m| m.1);
        let mut last = self.asked;
        let steps: Vec<String> = self
            .marks
            .iter()
            .map(|&(what, at)| {
                let step = format!("{what} {:.1}", ms(at.saturating_duration_since(last)));
                last = at;
                step
            })
            .collect();
        format!(
            "{:.1} ms after open_popup ({})",
            ms(end.saturating_duration_since(self.asked)),
            steps.join(", ")
        )
    }
}

/// Popup `id` presented its first frame: `[PERF] window 5 first frame: 12.3
/// ms after open_popup (queued 0.1, mount 3.0, measure 0.2, create 4.1,
/// renderer 0.0, size 0.3, to frame 1.2, paint 3.4)`, the input to first
/// presented frame of a popup and where it went: `queued` until the loop
/// made it (a keyboard wait included), `mount` its document built and laid
/// out, `measure` its content's size, `create` the OS window (the
/// compositor's first configure included), `renderer` its GPU renderer
/// (`0.0` on software), `size` its first size handled, `to frame` until its
/// first redraw, and `paint` the frame painted and presented.
pub(crate) fn popup_first_frame(id: u64, timeline: &PopupTimeline) {
    if rinch_dom::perf::perf_env_enabled() {
        eprintln!("[PERF] window {id} first frame: {}", timeline.describe());
    }
}

fn popup_line(id: u64, kind: rinch_core::element::PopupKind, parent: u64) -> String {
    let kind = match kind {
        rinch_core::element::PopupKind::Menu => "menu",
        rinch_core::element::PopupKind::Tooltip => "tooltip",
    };
    format!("[PERF] window {id} popup: kind={kind} parent={parent}")
}

/// `app`'s window gave its software buffers back to the idle trim (#2843):
/// under `RINCH_PERF`, `[PERF] window 3 trimmed`, labelled as its frames are
/// (`[PERF] trimmed` for the main window).
pub(crate) fn trimmed(app: &crate::app::RinchApp) {
    emit(app, "trimmed");
}

/// `app`'s window allocated its software buffers again, at the first present
/// after a trim: under `RINCH_PERF`, `[PERF] window 3 untrimmed`
/// (`[PERF] untrimmed`). Called before that frame ends, so the line comes
/// before the frame's own.
pub(crate) fn untrimmed(app: &crate::app::RinchApp) {
    emit(app, "untrimmed");
}

/// `app`'s window made its software presentation, whose buffers the idle
/// trim gives back after `period` without a frame (`None`: never; #2917):
/// under `RINCH_PERF`, `[PERF] window 3 idle_trim=2000ms` or
/// `[PERF] window 3 idle_trim=never` (`[PERF] idle_trim=…` for the main
/// window), before the line of the frame it is made for.
pub(crate) fn idle_trim(app: &crate::app::RinchApp, period: Option<std::time::Duration>) {
    emit(app, &idle_trim_word(period));
}

/// `idle_trim=2000ms` (fractions of a millisecond kept: `idle_trim=1.5ms`),
/// or `idle_trim=never`.
fn idle_trim_word(period: Option<std::time::Duration>) -> String {
    match period {
        Some(period) => format!("idle_trim={}ms", period.as_secs_f64() * 1000.0),
        None => "idle_trim=never".to_owned(),
    }
}

/// `[PERF] <label> <what>`, or `[PERF] <what>` with no label.
fn trim_line(label: Option<&str>, what: &str) -> String {
    match label {
        Some(label) => format!("[PERF] {label} {what}"),
        None => format!("[PERF] {what}"),
    }
}

/// Print `app`'s window's `what` line under `RINCH_PERF` (and, in this
/// crate's tests, hand it to `capture` while a test listens). Nothing is
/// built otherwise.
fn emit(app: &crate::app::RinchApp, what: &str) {
    let enabled = rinch_dom::perf::perf_env_enabled();
    #[cfg(test)]
    let enabled = enabled || capture::listening();
    if !enabled {
        return;
    }
    let line = trim_line(app.perf_label().as_deref(), what);
    #[cfg(test)]
    capture::note(&line);
    if rinch_dom::perf::perf_env_enabled() {
        eprintln!("{line}");
    }
}

/// The lines [`emit`] would print, kept for this thread's test while it
/// listens: `RINCH_PERF` is read once per process, so a test cannot set it.
#[cfg(test)]
pub(crate) mod capture {
    use std::cell::RefCell;

    thread_local! {
        static LINES: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
    }

    /// Start keeping this thread's lines.
    pub(crate) fn listen() {
        LINES.with(|l| *l.borrow_mut() = Some(Vec::new()));
    }

    /// The lines kept since [`listen`] or the last take.
    pub(crate) fn take() -> Vec<String> {
        LINES.with(|l| {
            l.borrow_mut()
                .as_mut()
                .map(std::mem::take)
                .unwrap_or_default()
        })
    }

    /// Whether this thread's test is listening.
    pub(crate) fn listening() -> bool {
        LINES.with(|l| l.borrow().is_some())
    }

    /// Keep `line`, while listening.
    pub(crate) fn note(line: &str) {
        LINES.with(|l| {
            if let Some(lines) = l.borrow_mut().as_mut() {
                lines.push(line.to_owned());
            }
        });
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

    /// A popup's `first frame` line (#2552) gives the total from
    /// `open_popup` to the last mark, and each step as the time since the
    /// step before, in the order they ended.
    #[test]
    fn a_popup_timeline_gives_the_total_and_each_step() {
        use std::time::{Duration, Instant};
        let asked = Instant::now();
        let mut t = PopupTimeline::new(asked);
        t.marks.push(("queued", asked + Duration::from_micros(100)));
        t.marks
            .push(("mount", asked + Duration::from_micros(3_100)));
        t.marks
            .push(("paint", asked + Duration::from_micros(12_320)));
        assert_eq!(
            t.describe(),
            "12.3 ms after open_popup (queued 0.1, mount 3.0, paint 9.2)"
        );
        assert_eq!(
            PopupTimeline::new(asked).describe(),
            "0.0 ms after open_popup ()"
        );
    }

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
