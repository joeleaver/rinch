//! The outputs (monitors) connected to the desktop, and their changes
//! (issue #2414).
//!
//! [`outputs`] lists them, each with the platform's name for it, its place
//! and size on the desktop in logical pixels, and its scale. It is
//! **reactive**: it reads a signal, so a `{|| …}` expression, a [`Memo`] or an
//! [`Effect`] that calls it runs again when an output is connected or
//! disconnected, or one's position, size or scale changes. That is how an app
//! keeps one surface per monitor, or a window on a monitor, with no polling of
//! its own. `output_changes` says what changed between two lists, and
//! `output_tracking` whether this platform follows changes at all.
//!
//! ```ignore
//! use rinch::prelude::*;
//!
//! #[component]
//! fn monitors() -> NodeHandle {
//!     rsx! {
//!         ul {
//!             for output in outputs() {
//!                 // Keyed by every field, so a changed output's row is rebuilt.
//!                 li {
//!                     key: format!("{output:?}"),
//!                     {format!("{:?}: {:?} at {:?}, scale {}",
//!                         output.name, output.size, output.position, output.scale)}
//!                 }
//!             }
//!         }
//!     }
//! }
//! ```
//!
//! To act on each change (open a panel on a new monitor, close the one whose
//! monitor went away), keep the last list and compare:
//!
//! ```ignore
//! let mut last: Vec<Output> = Vec::new();
//! Effect::new(move || {
//!     let now = outputs();
//!     for change in output_changes(&last, &now) {
//!         match change {
//!             OutputChange::Added(o) => { /* … */ }
//!             OutputChange::Removed(o) => { /* … */ }
//!             OutputChange::Changed { before, after } => { /* … */ }
//!             _ => {}
//!         }
//!     }
//!     last = now;
//! });
//! ```
//!
//! # Units
//!
//! The units rinch positions windows in ([`WindowProps::x`]/`y`,
//! [`current_window_position`]): logical pixels, where a monitor's logical
//! rectangle is its desktop rectangle over its own scale.
//!
//! - **X11, Windows**: the monitor's desktop rectangle in physical pixels
//!   (RandR's CRTC, `rcMonitor`), divided by its scale. On a desktop whose
//!   monitors have different scales, neighbours' logical rectangles can
//!   overlap or leave a gap; that is the space a logical window position is
//!   read in, monitor by monitor.
//! - **macOS**: points (`CGDisplayBounds`), the unit AppKit positions windows
//!   in; the scale is the backing scale (`2.0` on a Retina display).
//! - **Wayland**: the compositor's logical layout (`xdg_output`'s logical
//!   position and size). Outputs in it need not be contiguous: under KWin,
//!   giving one output scale 1.5 shrank its logical size and left its
//!   neighbour where it was, with a gap between them. So read each output's
//!   own position rather than assume where it is from the others. The scale is the
//!   output's mode over its logical size, to the nearest 1/120 (the unit of the
//!   fractional-scale protocol), so `1.25` and `1.5` come out as they are set.
//!   A layer-shell window's position ([`WindowProps::output`]) is relative to
//!   its output's top-left.
//!
//! # Where the list comes from
//!
//! The desktop shell lists the outputs when its event loop starts (before
//! the window is created, so the first render sees them), and again at every
//! change it hears of. `output_tracking` says which:
//!
//! - **Wayland, X11**: `OutputTracking::Live`. Each `wl_output` advertised,
//!   withdrawn or changed (`wl_output.done`); each RandR screen change and
//!   each `Xft.dpi` change (X11 has one scale for all its monitors).
//! - **Windows, macOS**: `OutputTracking::Snapshot`. Listed at start-up;
//!   changes are not followed yet (winit reports none there; issue #2414).
//! - Anywhere no desktop event loop runs (before `App::run`, a native embed
//!   host, the web, Android): `OutputTracking::NotListed`, and the list is
//!   empty.
//!
//! The getters are for the UI thread (they read signals, which belong to the
//! thread): read on another thread they answer that thread's empty list and
//! `NotListed`, and say so once per thread in a `tracing` warning.
//!
//! [`Memo`]: rinch_core::Memo
//! [`Effect`]: rinch_core::Effect
//! [`WindowProps::x`]: rinch_core::element::WindowProps::x
//! [`WindowProps::output`]: rinch_core::element::WindowProps::output
//! [`current_window_position`]: crate::windows::current_window_position

use std::cell::Cell;

use rinch_core::Signal;

/// An output (monitor) connected to the desktop. See the [module
/// docs](self) for the units.
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    /// The platform's name for it: the connector on Wayland and X11 (`DP-1`,
    /// `eDP-1`, `HDMI-A-1`), the device on Windows (`\\.\DISPLAY1`),
    /// `Monitor #<model number>` on macOS (not unique for two of one model).
    /// Exactly the string [`WindowProps::monitor`] and, on Wayland,
    /// [`WindowProps::output`] and `set_current_window_output` accept, so it
    /// is how an app puts a window on this output. `None` where the platform
    /// names none (a Wayland compositor with `wl_output` older than version 4).
    ///
    /// [`WindowProps::monitor`]: rinch_core::element::WindowProps::monitor
    /// [`WindowProps::output`]: rinch_core::element::WindowProps::output
    pub name: Option<String>,
    /// Top-left on the desktop, logical pixels.
    pub position: (f64, f64),
    /// Width and height, logical pixels (a rotated output's are rotated).
    pub size: (f64, f64),
    /// Physical pixels per logical pixel on this output: `1.0`, `2.0`, or a
    /// fractional scale such as `1.5`.
    pub scale: f64,
}

/// Whether [`outputs`] follows the outputs' changes here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OutputTracking {
    /// Nothing lists outputs here: no desktop event loop has started on this
    /// thread (before `App::run`, a native embed host, the web, Android).
    /// [`outputs`] is empty.
    NotListed,
    /// Listed when the event loop started; later changes are not followed
    /// (Windows and macOS: winit reports no monitor change there, issue
    /// #2414). The list stays as it was at start-up.
    Snapshot,
    /// Every change is published as it happens (Wayland, X11).
    Live,
}

/// One difference between two output lists ([`output_changes`]).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum OutputChange {
    /// An output in the new list only: connected (or enabled).
    Added(Output),
    /// An output in the old list only: disconnected (or disabled).
    Removed(Output),
    /// The same output in both, with another position, size or scale.
    Changed {
        /// As it was.
        before: Output,
        /// As it is now.
        after: Output,
    },
}

/// The connected outputs, in the platform's order. Reactive: a reader re-runs
/// when the list changes (see the [module docs](self)). Empty until the
/// desktop shell lists them.
pub fn outputs() -> Vec<Output> {
    signals().list.get()
}

/// Whether [`outputs`] follows changes on this platform. Reactive: it turns
/// from [`OutputTracking::NotListed`] to what the platform does when the
/// event loop starts.
pub fn output_tracking() -> OutputTracking {
    signals().tracking.get()
}

/// What changed from `before` to `after`: first each output removed (in
/// `before`'s order), then each one added or changed (in `after`'s order).
/// Equal lists give none.
///
/// Outputs are the same output when they have the same name; two of one
/// name (two macOS monitors of one model) are paired in order. An output with
/// no name is recognised only while nothing about it changes, so a change to
/// one reads as a removal and an addition.
pub fn output_changes(before: &[Output], after: &[Output]) -> Vec<OutputChange> {
    // For each output of `after`, the index of its partner in `before`.
    let mut taken = vec![false; before.len()];
    let partners: Vec<Option<usize>> = after
        .iter()
        .map(|now| {
            let found = (0..before.len()).find(|&i| {
                !taken[i]
                    && match (&before[i].name, &now.name) {
                        (Some(a), Some(b)) => a == b,
                        (None, None) => before[i] == *now,
                        _ => false,
                    }
            });
            if let Some(i) = found {
                taken[i] = true;
            }
            found
        })
        .collect();
    let mut changes: Vec<OutputChange> = before
        .iter()
        .zip(&taken)
        .filter(|(_, taken)| !**taken)
        .map(|(o, _)| OutputChange::Removed(o.clone()))
        .collect();
    for (now, partner) in after.iter().zip(partners) {
        match partner {
            None => changes.push(OutputChange::Added(now.clone())),
            Some(i) if before[i] != *now => changes.push(OutputChange::Changed {
                before: before[i].clone(),
                after: now.clone(),
            }),
            Some(_) => {}
        }
    }
    changes
}

#[derive(Clone, Copy)]
struct OutputSignals {
    list: Signal<Vec<Output>>,
    tracking: Signal<OutputTracking>,
}

thread_local! {
    static SIGNALS: Cell<Option<OutputSignals>> = const { Cell::new(None) };
}

/// The thread's output signals, made on first use. They belong to no scope
/// (`unowned`): they live as long as the thread, whichever component first
/// asked.
fn signals() -> OutputSignals {
    if let Some(s) = SIGNALS.with(Cell::get) {
        return s;
    }
    if !rinch_core::is_main_thread() {
        tracing::warn!(
            "rinch::outputs was read off the UI thread: the output list belongs to the UI \
             thread, so this thread sees an empty one"
        );
    }
    let s = rinch_core::unowned(|| OutputSignals {
        list: Signal::new(Vec::new()),
        tracking: Signal::new(OutputTracking::NotListed),
    });
    SIGNALS.with(|c| c.set(Some(s)));
    s
}

/// Publish the outputs the shell read, and whether it follows them, as one
/// batch: a reader of both runs once. Equal values wake nothing.
pub(crate) fn publish(list: Vec<Output>, tracking: OutputTracking) {
    let s = signals();
    rinch_core::batch(|| {
        s.list.set_if_changed(list);
        s.tracking.set_if_changed(tracking);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(name: Option<&str>, x: f64, w: f64, scale: f64) -> Output {
        Output {
            name: name.map(str::to_owned),
            position: (x, 0.0),
            size: (w, 1080.0),
            scale,
        }
    }

    #[test]
    fn equal_lists_have_no_changes() {
        let list = vec![
            output(Some("DP-1"), 0.0, 1920.0, 1.0),
            output(None, 1920.0, 1280.0, 1.5),
        ];
        assert_eq!(output_changes(&list, &list), vec![]);
        assert_eq!(output_changes(&[], &[]), vec![]);
    }

    #[test]
    fn an_output_plugged_in_is_added_and_one_unplugged_is_removed() {
        let dp1 = output(Some("DP-1"), 0.0, 1920.0, 1.0);
        let hdmi = output(Some("HDMI-A-1"), 1920.0, 1920.0, 1.0);
        assert_eq!(
            output_changes(std::slice::from_ref(&dp1), &[dp1.clone(), hdmi.clone()]),
            vec![OutputChange::Added(hdmi.clone())]
        );
        assert_eq!(
            output_changes(&[dp1.clone(), hdmi.clone()], std::slice::from_ref(&dp1)),
            vec![OutputChange::Removed(hdmi.clone())]
        );
        // Everything unplugged, and the first output of a session.
        assert_eq!(
            output_changes(std::slice::from_ref(&dp1), &[]),
            vec![OutputChange::Removed(dp1.clone())]
        );
        assert_eq!(
            output_changes(&[], std::slice::from_ref(&dp1)),
            vec![OutputChange::Added(dp1)]
        );
    }

    #[test]
    fn a_rescaled_or_moved_output_is_changed_not_replaced() {
        let before = output(Some("DP-1"), 0.0, 3840.0, 1.0);
        let rescaled = output(Some("DP-1"), 0.0, 2560.0, 1.5);
        assert_eq!(
            output_changes(
                std::slice::from_ref(&before),
                std::slice::from_ref(&rescaled)
            ),
            vec![OutputChange::Changed {
                before: before.clone(),
                after: rescaled,
            }]
        );
        let moved = output(Some("DP-1"), 100.0, 3840.0, 1.0);
        assert_eq!(
            output_changes(std::slice::from_ref(&before), std::slice::from_ref(&moved)),
            vec![OutputChange::Changed {
                before,
                after: moved,
            }]
        );
    }

    #[test]
    fn removals_come_first_then_additions_and_changes_in_the_new_order() {
        let a = output(Some("A"), 0.0, 100.0, 1.0);
        let b = output(Some("B"), 100.0, 100.0, 1.0);
        let b2 = output(Some("B"), 100.0, 200.0, 1.0);
        let c = output(Some("C"), 300.0, 100.0, 1.0);
        assert_eq!(
            output_changes(&[a.clone(), b.clone()], &[c.clone(), b2.clone()]),
            vec![
                OutputChange::Removed(a),
                OutputChange::Added(c),
                OutputChange::Changed {
                    before: b,
                    after: b2
                },
            ]
        );
    }

    #[test]
    fn two_outputs_of_one_name_pair_in_order() {
        let first = output(Some("Monitor #1234"), 0.0, 100.0, 1.0);
        let second = output(Some("Monitor #1234"), 100.0, 100.0, 1.0);
        let list = [first.clone(), second.clone()];
        assert_eq!(output_changes(&list, &list), vec![]);
        // The second one unplugged: the first keeps its partner.
        assert_eq!(
            output_changes(&list, std::slice::from_ref(&first)),
            vec![OutputChange::Removed(second)]
        );
    }

    #[test]
    fn an_unnamed_output_is_recognised_only_while_unchanged() {
        let before = output(None, 0.0, 100.0, 1.0);
        let after = output(None, 0.0, 100.0, 2.0);
        assert_eq!(
            output_changes(std::slice::from_ref(&before), std::slice::from_ref(&before)),
            vec![]
        );
        assert_eq!(
            output_changes(std::slice::from_ref(&before), std::slice::from_ref(&after)),
            vec![OutputChange::Removed(before), OutputChange::Added(after)]
        );
        // A name never pairs with no name.
        let named = output(Some("DP-1"), 0.0, 100.0, 1.0);
        let unnamed = output(None, 0.0, 100.0, 1.0);
        assert_eq!(
            output_changes(std::slice::from_ref(&named), std::slice::from_ref(&unnamed)),
            vec![OutputChange::Removed(named), OutputChange::Added(unnamed)]
        );
    }

    #[test]
    fn publishing_wakes_readers_once_and_only_for_a_change() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let seen: Rc<RefCell<Vec<(usize, OutputTracking)>>> = Rc::default();
        let effect = rinch_core::Effect::new({
            let seen = seen.clone();
            move || {
                let n = outputs().len();
                let tracking = output_tracking();
                seen.borrow_mut().push((n, tracking));
            }
        });
        let one = vec![output(Some("DP-1"), 0.0, 1920.0, 1.0)];
        publish(one.clone(), OutputTracking::Live);
        // The same list again wakes nothing.
        publish(one.clone(), OutputTracking::Live);
        let two = vec![one[0].clone(), output(Some("DP-2"), 1920.0, 1920.0, 1.0)];
        publish(two.clone(), OutputTracking::Live);
        assert_eq!(
            *seen.borrow(),
            vec![
                (0, OutputTracking::NotListed),
                // The list and the tracking together: one run.
                (1, OutputTracking::Live),
                (2, OutputTracking::Live),
            ]
        );
        assert_eq!(outputs(), two);
        drop(effect);
    }
}
