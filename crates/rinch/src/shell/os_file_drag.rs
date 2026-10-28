//! An OS file drag over the window, translated from winit's data transfer API
//! into rinch's file-drag events (#1962).
//!
//! Since winit 0.31.0-beta.3 a drag is a *data transfer*: `DragEntered` names
//! a transfer id and maybe a position, `DragPosition` moves it, `DragDropped`
//! and `DragLeft` end it, and the dragged data is fetched asynchronously
//! (`fetch_data_transfer`, answered by `DataTransferReceived`). The drag is
//! rejected unless the app accepts an action (`set_valid_dnd_actions`). rinch
//! wants file paths, so a drag counts only when it offers a URI list; it is
//! accepted as a copy, or a move when the source offers only that
//! ([`ACCEPTED_ACTIONS`]), and its URI list is fetched once a position is
//! known (X11 names the position only with the first `XdndPosition`, and asks
//! for the data with that position's timestamp).
//!
//! What the app is handed is unchanged from winit beta.2's path-carrying
//! events: a [`PlatformEvent::FileDragMoved`] at every position, a
//! [`PlatformEvent::FileHoverEnter`] per path once the paths are known, a
//! [`PlatformEvent::FileDropped`] with every path at the last position, and a
//! [`PlatformEvent::FileHoverCancelled`] for a drag that leaves, or that a new
//! drag replaces before it ended. A drop whose paths have not arrived yet is
//! handed over when they do; one whose paths do not arrive within
//! [`DROP_TIMEOUT`] (X11 logs a failed selection read and sends nothing), or
//! that a new drag's entry finds still waiting, ends as cancelled: the app
//! holds one drag's state, so the two never overlap.
//!
//! [`OsFileDrag`] is the state machine with no winit event loop in it: the
//! runtime feeds it the events (positions already logical) and carries out the
//! [`DragStep`]s it answers through a [`DragHost`] ([`run_steps`]).

use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rinch_platform::PlatformEvent;
use winit::data_transfer::DataTransferId;
use winit::event_loop::{AsyncRequestSerial, DndAction};

/// The actions a file drag is accepted with, by preference: a copy, or a move
/// for a source that offers only that. The winit fork's Wayland drag and drop
/// accepted the same pair (`Copy | Move`, preferring `Copy`).
pub(crate) const ACCEPTED_ACTIONS: [DndAction; 2] = [DndAction::Copy, DndAction::Move];

/// How long a drop waits for its paths before it ends as cancelled.
pub(crate) const DROP_TIMEOUT: Duration = Duration::from_secs(3);

/// What the runtime does for an [`OsFileDrag`] answer, in order.
#[derive(Debug)]
pub(crate) enum DragStep {
    /// Accept the drag ([`ACCEPTED_ACTIONS`]).
    Accept(DataTransferId),
    /// Ask for the drag's URI list and report how it went with
    /// [`OsFileDrag::fetch_started`].
    Fetch(DataTransferId),
    /// Hand the app this event.
    Emit(PlatformEvent),
    /// Log this (once per drag at most).
    Warn(String),
}

/// The fetch of a drag's URI list.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Fetch {
    /// Not asked for yet: no position known.
    NotStarted,
    /// The request failed; not asked again until the drop.
    Failed,
    /// Asked for; the answer carries this serial.
    Pending(AsyncRequestSerial),
    /// Answered.
    Done,
}

/// One OS file drag: the one hovering, or a drop waiting for its paths.
#[derive(Debug)]
struct Drag {
    id: DataTransferId,
    /// The last position, logical px.
    position: Option<(f64, f64)>,
    fetch: Fetch,
    /// The paths, once the URI list has been read.
    paths: Option<Vec<PathBuf>>,
    /// When a dropped drag stops waiting for its paths.
    deadline: Option<Instant>,
    /// A warning has been logged for this drag.
    warned: bool,
}

impl Drag {
    fn new(id: DataTransferId) -> Self {
        Self {
            id,
            position: None,
            fetch: Fetch::NotStarted,
            paths: None,
            deadline: None,
            warned: false,
        }
    }

    fn position(&self) -> (f64, f64) {
        self.position.unwrap_or_default()
    }

    /// A warning, if this drag has had none yet.
    fn warn(&mut self, message: String) -> Option<DragStep> {
        (!std::mem::replace(&mut self.warned, true)).then_some(DragStep::Warn(message))
    }
}

/// The OS file drags over the window: at most one hovering, and the drops
/// still waiting for their paths.
#[derive(Debug, Default)]
pub(crate) struct OsFileDrag {
    hover: Option<Drag>,
    drops: Vec<Drag>,
}

impl OsFileDrag {
    /// `DragEntered`. `has_uri_list`: the transfer offers a URI list. A drag
    /// without one (text, an image's bytes) is not a file drag and is left
    /// rejected; its later events are ignored. Either way it ends, with one
    /// `FileHoverCancelled`, the drag the app still holds: the one hovering
    /// (whose end never came), and any drop still waiting for its paths,
    /// which is discarded with a warning. The app has one file-drag slot
    /// (its drop caret, hover target, picture claim): a drop handed over
    /// while the new drag hovers would land at the new drag's caret and end
    /// it, and the new drag would start with the old one's state.
    pub(crate) fn entered(
        &mut self,
        id: DataTransferId,
        position: Option<(f64, f64)>,
        has_uri_list: bool,
    ) -> Vec<DragStep> {
        let mut steps = Vec::new();
        let hovering = self.hover.take().is_some();
        if !self.drops.is_empty() {
            let n = self.drops.len();
            self.drops.clear();
            steps.push(DragStep::Warn(format!(
                "a new drag entered before {n} file drop(s) had their paths; the drop is \
                 cancelled"
            )));
        }
        if hovering || !steps.is_empty() {
            steps.push(DragStep::Emit(PlatformEvent::FileHoverCancelled));
        }
        if !has_uri_list {
            return steps;
        }
        self.hover = Some(Drag::new(id));
        steps.push(DragStep::Accept(id));
        if let Some(position) = position {
            steps.extend(self.moved(id, position));
        }
        steps
    }

    /// `DragPosition`.
    pub(crate) fn moved(&mut self, id: DataTransferId, position: (f64, f64)) -> Vec<DragStep> {
        let Some(drag) = self.hover.as_mut().filter(|d| d.id == id) else {
            return Vec::new();
        };
        drag.position = Some(position);
        let mut steps = Vec::new();
        if drag.fetch == Fetch::NotStarted {
            steps.push(DragStep::Fetch(id));
        }
        steps.push(DragStep::Emit(PlatformEvent::FileDragMoved { position }));
        steps
    }

    /// The runtime asked for the URI list ([`DragStep::Fetch`]): the request's
    /// serial, or why it failed. A hovering drag whose request failed hovers on
    /// path-less and asks again at its drop; a drop whose request failed ends
    /// with no paths.
    pub(crate) fn fetch_started(
        &mut self,
        id: DataTransferId,
        serial: Result<AsyncRequestSerial, String>,
    ) -> Vec<DragStep> {
        if let Some(drag) = self.hover.as_mut().filter(|d| d.id == id) {
            return match serial {
                Ok(serial) => {
                    drag.fetch = Fetch::Pending(serial);
                    Vec::new()
                }
                Err(e) => {
                    drag.fetch = Fetch::Failed;
                    drag.warn(format!("could not read a file drag's paths: {e}"))
                        .into_iter()
                        .collect()
                }
            };
        }
        let Some(at) = self.drops.iter().position(|d| d.id == id) else {
            return Vec::new();
        };
        match serial {
            Ok(serial) => {
                self.drops[at].fetch = Fetch::Pending(serial);
                Vec::new()
            }
            Err(e) => {
                let mut drag = self.drops.remove(at);
                let mut steps: Vec<DragStep> = drag
                    .warn(format!("could not read a file drop's paths: {e}"))
                    .into_iter()
                    .collect();
                steps.push(DragStep::Emit(PlatformEvent::FileDropped {
                    paths: Vec::new(),
                    position: drag.position(),
                }));
                steps
            }
        }
    }

    /// `DataTransferReceived` for request `serial` of transfer `id`: `uris` is
    /// the URI list read (`None` while it would block; it comes again with the
    /// next `DataTransferReceived`), or an empty list when it cannot be read
    /// ([`uris_from`]).
    pub(crate) fn received(
        &mut self,
        id: DataTransferId,
        serial: AsyncRequestSerial,
        uris: Option<Vec<String>>,
    ) -> Vec<DragStep> {
        let answers = |d: &Drag| d.id == id && d.fetch == Fetch::Pending(serial);
        let Some(uris) = uris else {
            return Vec::new();
        };
        let paths: Vec<PathBuf> = uris
            .iter()
            .filter_map(|uri| file_uri_to_path(uri))
            .collect();
        if let Some(drag) = self.hover.as_mut().filter(|d| answers(d)) {
            drag.fetch = Fetch::Done;
            let position = drag.position();
            let steps = paths
                .iter()
                .map(|path| {
                    DragStep::Emit(PlatformEvent::FileHoverEnter {
                        path: path.clone(),
                        position,
                    })
                })
                .collect();
            drag.paths = Some(paths);
            return steps;
        }
        let Some(at) = self.drops.iter().position(answers) else {
            return Vec::new();
        };
        let drag = self.drops.remove(at);
        vec![DragStep::Emit(PlatformEvent::FileDropped {
            paths,
            position: drag.position(),
        })]
    }

    /// `DragDropped` at `now`: the drop, at once if the paths are known, else
    /// when they arrive (asking for them now if nothing is asking), within
    /// [`DROP_TIMEOUT`].
    pub(crate) fn dropped(&mut self, id: DataTransferId, now: Instant) -> Vec<DragStep> {
        if self.hover.as_ref().is_none_or(|d| d.id != id) {
            return Vec::new();
        }
        let mut drag = self.hover.take().expect("checked above");
        if let Some(paths) = drag.paths.take() {
            return vec![DragStep::Emit(PlatformEvent::FileDropped {
                paths,
                position: drag.position(),
            })];
        }
        drag.deadline = Some(now + DROP_TIMEOUT);
        let ask = matches!(drag.fetch, Fetch::NotStarted | Fetch::Failed);
        self.drops.push(drag);
        if ask {
            vec![DragStep::Fetch(id)]
        } else {
            Vec::new()
        }
    }

    /// `DragLeft` for transfer `id`: the hovering drag left the window, or was
    /// cancelled. A leave naming another transfer, or a drop waiting for its
    /// paths, changes nothing.
    pub(crate) fn left(&mut self, id: DataTransferId) -> Vec<DragStep> {
        if self.hover.as_ref().is_some_and(|d| d.id == id) {
            self.hover = None;
            return vec![DragStep::Emit(PlatformEvent::FileHoverCancelled)];
        }
        Vec::new()
    }

    /// When the next waiting drop gives up ([`Self::expire`]).
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.drops.iter().filter_map(|d| d.deadline).min()
    }

    /// End every drop whose paths have not come by `now`: it is cancelled
    /// (the drop caret and the hover target go). No drag hovers while a drop
    /// waits: a new drag's entry ends the waiting drops ([`Self::entered`]).
    pub(crate) fn expire(&mut self, now: Instant) -> Vec<DragStep> {
        let mut steps = Vec::new();
        let mut i = 0;
        while i < self.drops.len() {
            if self.drops[i].deadline.is_some_and(|at| at <= now) {
                let mut drag = self.drops.remove(i);
                steps.extend(drag.warn(format!(
                    "a file drop's paths did not come within {DROP_TIMEOUT:?}; the drop is \
                     cancelled"
                )));
                steps.push(DragStep::Emit(PlatformEvent::FileHoverCancelled));
            } else {
                i += 1;
            }
        }
        steps
    }
}

/// The URI list a `DataTransferReceived` value read as (`TypedData::try_as_uris`):
/// `None` when the read would block or deadlock on the event loop (winit says
/// to retry on the next `DataTransferReceived`), an empty list when it cannot
/// be read at all.
pub(crate) fn uris_from(read: io::Result<Vec<String>>) -> Option<Vec<String>> {
    match read {
        Ok(uris) => Some(uris),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Deadlock
            ) =>
        {
            None
        }
        Err(_) => Some(Vec::new()),
    }
}

/// What the runtime does for a [`DragStep`]: winit's event loop, or a fake.
pub(crate) trait DragHost {
    /// `set_valid_dnd_actions(id, actions)`.
    fn accept(&mut self, id: DataTransferId, actions: &[DndAction]) -> Result<(), String>;
    /// `fetch_data_transfer(id, TypeHint::UriList)`.
    fn fetch_uri_list(&mut self, id: DataTransferId) -> Result<AsyncRequestSerial, String>;
    /// Hand the app an event.
    fn emit(&mut self, event: PlatformEvent);
    /// Log a warning.
    fn warn(&mut self, message: String);
}

/// Carry out `steps` through `host`, feeding back what a fetch answers.
pub(crate) fn run_steps(drag: &mut OsFileDrag, steps: Vec<DragStep>, host: &mut dyn DragHost) {
    let mut queue: std::collections::VecDeque<DragStep> = steps.into();
    while let Some(step) = queue.pop_front() {
        match step {
            DragStep::Accept(id) => {
                if let Err(e) = host.accept(id, &ACCEPTED_ACTIONS) {
                    host.warn(format!("could not accept a file drag: {e}"));
                }
            }
            DragStep::Fetch(id) => {
                let serial = host.fetch_uri_list(id);
                queue.extend(drag.fetch_started(id, serial));
            }
            DragStep::Emit(event) => host.emit(event),
            DragStep::Warn(message) => host.warn(message),
        }
    }
}

/// The local path a `file:` URI names, or `None` for any other URI (a web
/// link in the list) and a URI that is no path here.
pub(crate) fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let url = url::Url::parse(uri.trim()).ok()?;
    if url.scheme() != "file" {
        return None;
    }
    url.to_file_path().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `PlatformEvent` has no `PartialEq`: steps compare by their `Debug` form.
    #[track_caller]
    fn assert_steps(actual: Vec<DragStep>, expected: Vec<DragStep>) {
        assert_eq!(format!("{actual:#?}"), format!("{expected:#?}"));
    }

    fn id(n: i64) -> DataTransferId {
        DataTransferId::from_raw(n)
    }

    fn uris() -> Vec<String> {
        vec![
            "file:///home/me/a%20b.png".to_owned(),
            "https://example.com/x.png".to_owned(),
            "file:///home/me/c.txt".to_owned(),
        ]
    }

    #[cfg(unix)]
    fn paths() -> Vec<PathBuf> {
        vec![
            PathBuf::from("/home/me/a b.png"),
            PathBuf::from("/home/me/c.txt"),
        ]
    }

    fn now() -> Instant {
        Instant::now()
    }

    /// Wayland, Windows, macOS: the position comes with the enter, so the
    /// URI list is asked for at once; its paths hover, then drop.
    #[cfg(unix)]
    #[test]
    fn a_drag_with_a_position_on_enter_hovers_its_paths_then_drops_them() {
        let mut d = OsFileDrag::default();
        let steps = d.entered(id(7), Some((10.5, 20.25)), true);
        assert_steps(
            steps,
            vec![
                DragStep::Accept(id(7)),
                DragStep::Fetch(id(7)),
                DragStep::Emit(PlatformEvent::FileDragMoved {
                    position: (10.5, 20.25),
                }),
            ],
        );
        let serial = AsyncRequestSerial::get();
        assert!(d.fetch_started(id(7), Ok(serial)).is_empty());
        assert_steps(
            d.moved(id(7), (30.0, 40.0)),
            vec![DragStep::Emit(PlatformEvent::FileDragMoved {
                position: (30.0, 40.0),
            })],
        );
        assert_steps(
            d.received(id(7), serial, Some(uris())),
            paths()
                .into_iter()
                .map(|path| {
                    DragStep::Emit(PlatformEvent::FileHoverEnter {
                        path,
                        position: (30.0, 40.0),
                    })
                })
                .collect(),
        );
        let steps = d.moved(id(7), (31.0, 41.0));
        assert_eq!(steps.len(), 1, "no second fetch: {steps:?}");
        assert_steps(
            d.dropped(id(7), now()),
            vec![DragStep::Emit(PlatformEvent::FileDropped {
                paths: paths(),
                position: (31.0, 41.0),
            })],
        );
        // The drag is over: a late leave cancels nothing.
        assert!(d.left(id(7)).is_empty());
        assert_eq!(d.next_deadline(), None);
    }

    /// X11: no position on enter; the first position asks for the data.
    #[test]
    fn a_drag_without_a_position_on_enter_fetches_at_its_first_position() {
        let mut d = OsFileDrag::default();
        assert_steps(d.entered(id(1), None, true), vec![DragStep::Accept(id(1))]);
        assert_steps(
            d.moved(id(1), (5.0, 6.0)),
            vec![
                DragStep::Fetch(id(1)),
                DragStep::Emit(PlatformEvent::FileDragMoved {
                    position: (5.0, 6.0),
                }),
            ],
        );
    }

    /// A drop before the URI list has arrived is handed over when it does,
    /// at the drop's position; a read that would block waits for the next.
    #[cfg(unix)]
    #[test]
    fn a_drop_before_the_paths_waits_for_them() {
        let mut d = OsFileDrag::default();
        d.entered(id(2), Some((1.0, 2.0)), true);
        let serial = AsyncRequestSerial::get();
        d.fetch_started(id(2), Ok(serial));
        let t = now();
        assert!(d.dropped(id(2), t).is_empty());
        assert_eq!(d.next_deadline(), Some(t + DROP_TIMEOUT));
        // Neither a later move nor a leave touches a dropped drag.
        assert!(d.moved(id(2), (9.0, 9.0)).is_empty());
        assert!(d.left(id(2)).is_empty());
        assert!(d.received(id(2), serial, None).is_empty(), "would block");
        let other = AsyncRequestSerial::get();
        assert!(
            d.received(id(2), other, Some(uris())).is_empty(),
            "not its serial"
        );
        assert!(
            d.received(id(3), serial, Some(uris())).is_empty(),
            "not its transfer"
        );
        assert_steps(
            d.received(id(2), serial, Some(uris())),
            vec![DragStep::Emit(PlatformEvent::FileDropped {
                paths: paths(),
                position: (1.0, 2.0),
            })],
        );
        assert_eq!(d.next_deadline(), None);
    }

    /// A hover whose fetch fails warns once (not on every move) and asks
    /// again at the drop; a drop whose fetch fails too ends with no paths.
    #[test]
    fn a_failed_fetch_warns_once_and_is_asked_again_at_the_drop() {
        let mut d = OsFileDrag::default();
        d.entered(id(3), Some((4.0, 5.0)), true);
        assert_steps(
            d.fetch_started(id(3), Err("no".into())),
            vec![DragStep::Warn(
                "could not read a file drag's paths: no".into(),
            )],
        );
        // No second fetch on a move, so no second warning.
        assert_steps(
            d.moved(id(3), (4.0, 5.0)),
            vec![DragStep::Emit(PlatformEvent::FileDragMoved {
                position: (4.0, 5.0),
            })],
        );
        assert_steps(d.dropped(id(3), now()), vec![DragStep::Fetch(id(3))]);
        assert_steps(
            d.fetch_started(id(3), Err("no".into())),
            vec![DragStep::Emit(PlatformEvent::FileDropped {
                paths: Vec::new(),
                position: (4.0, 5.0),
            })],
        );
        assert_eq!(d.next_deadline(), None);
    }

    #[test]
    fn a_drag_that_leaves_is_cancelled() {
        let mut d = OsFileDrag::default();
        d.entered(id(4), Some((0.5, 0.5)), true);
        assert_steps(
            d.left(id(4)),
            vec![DragStep::Emit(PlatformEvent::FileHoverCancelled)],
        );
        assert!(d.moved(id(4), (1.0, 1.0)).is_empty());
    }

    /// A leave names its transfer: one naming another leaves the hover be.
    #[test]
    fn a_leave_for_another_transfer_cancels_nothing() {
        let mut d = OsFileDrag::default();
        d.entered(id(4), Some((0.5, 0.5)), true);
        assert!(d.left(id(40)).is_empty());
        assert_eq!(d.moved(id(4), (1.0, 1.0)).len(), 2, "still hovering");
    }

    /// Text or an image's bytes is no file drag: not accepted, no events.
    #[test]
    fn a_drag_without_a_uri_list_is_ignored() {
        let mut d = OsFileDrag::default();
        assert!(d.entered(id(5), Some((1.0, 1.0)), false).is_empty());
        assert!(d.moved(id(5), (2.0, 2.0)).is_empty());
        assert!(d.dropped(id(5), now()).is_empty());
        assert!(d.left(id(5)).is_empty());
    }

    /// A new drag ends one whose end never came, with a cancel, whether or
    /// not the new one is a file drag; the old one's events are ignored after.
    #[test]
    fn a_new_drag_cancels_a_stale_one() {
        let mut d = OsFileDrag::default();
        d.entered(id(8), Some((1.0, 1.0)), true);
        let steps = d.entered(id(9), Some((2.0, 2.0)), true);
        assert_eq!(
            format!("{:?}", steps[0]),
            format!("{:?}", DragStep::Emit(PlatformEvent::FileHoverCancelled))
        );
        assert!(d.moved(id(8), (3.0, 3.0)).is_empty());
        assert_eq!(d.moved(id(9), (3.0, 3.0)).len(), 2);

        assert_steps(
            d.entered(id(10), Some((2.0, 2.0)), false),
            vec![DragStep::Emit(PlatformEvent::FileHoverCancelled)],
        );
        assert!(
            d.moved(id(9), (3.0, 3.0)).is_empty(),
            "a text drag ended it"
        );
        assert!(d.left(id(9)).is_empty());
    }

    /// A new drag entering ends a drop still waiting for its paths: one
    /// warning, one `FileHoverCancelled` (which clears the app's drag state)
    /// before the new drag is accepted, and the old drop's late paths are
    /// ignored, so they cannot land at the new drag's caret or end it.
    #[test]
    fn a_new_drag_ends_a_pending_drop() {
        let mut d = OsFileDrag::default();
        d.entered(id(11), Some((1.0, 1.0)), true);
        let serial = AsyncRequestSerial::get();
        d.fetch_started(id(11), Ok(serial));
        assert!(d.dropped(id(11), now()).is_empty());
        let steps = d.entered(id(12), None, true);
        assert_eq!(steps.len(), 3, "{steps:?}");
        assert!(matches!(steps[0], DragStep::Warn(_)), "{steps:?}");
        assert_eq!(
            format!("{:?}", &steps[1..]),
            format!(
                "{:?}",
                [
                    DragStep::Emit(PlatformEvent::FileHoverCancelled),
                    DragStep::Accept(id(12)),
                ]
            )
        );
        assert_eq!(d.next_deadline(), None);
        assert!(d.received(id(11), serial, Some(uris())).is_empty());
        // A text drag ends it too.
        d.entered(id(13), Some((1.0, 1.0)), true);
        d.fetch_started(id(13), Ok(AsyncRequestSerial::get()));
        d.dropped(id(13), now());
        let steps = d.entered(id(14), None, false);
        assert_eq!(steps.len(), 2, "{steps:?}");
        assert!(matches!(
            steps[1],
            DragStep::Emit(PlatformEvent::FileHoverCancelled)
        ));
        assert_eq!(d.next_deadline(), None);
    }

    /// A drop whose paths never come (X11's failed selection read sends no
    /// answer) is cancelled at its deadline, not before, and warns once.
    #[test]
    fn a_drop_whose_paths_never_come_is_cancelled_at_its_deadline() {
        let mut d = OsFileDrag::default();
        d.entered(id(13), Some((1.0, 1.0)), true);
        let serial = AsyncRequestSerial::get();
        d.fetch_started(id(13), Ok(serial));
        let t = now();
        d.dropped(id(13), t);
        assert!(
            d.expire(t + DROP_TIMEOUT - Duration::from_millis(1))
                .is_empty()
        );
        let steps = d.expire(t + DROP_TIMEOUT);
        assert_eq!(steps.len(), 2, "{steps:?}");
        assert!(matches!(steps[0], DragStep::Warn(_)));
        assert_eq!(
            format!("{:?}", steps[1]),
            format!("{:?}", DragStep::Emit(PlatformEvent::FileHoverCancelled))
        );
        assert_eq!(d.next_deadline(), None);
        // A late answer finds nothing.
        assert!(d.received(id(13), serial, Some(uris())).is_empty());
    }

    #[test]
    fn a_read_that_would_block_or_deadlock_is_retried() {
        let err = |kind| Err(io::Error::from(kind));
        assert_eq!(
            uris_from(Ok(vec!["file:///a".into()])),
            Some(vec!["file:///a".into()])
        );
        assert_eq!(uris_from(err(io::ErrorKind::WouldBlock)), None);
        assert_eq!(uris_from(err(io::ErrorKind::Deadlock)), None);
        assert_eq!(uris_from(err(io::ErrorKind::InvalidData)), Some(Vec::new()));
    }

    /// A [`DragHost`] that records what it was asked.
    #[derive(Default)]
    struct FakeHost {
        log: Vec<String>,
        fetch_fails: bool,
        accept_fails: bool,
        serials: Vec<AsyncRequestSerial>,
    }

    impl DragHost for FakeHost {
        fn accept(&mut self, id: DataTransferId, actions: &[DndAction]) -> Result<(), String> {
            self.log
                .push(format!("accept {} {actions:?}", id.into_raw()));
            if self.accept_fails {
                Err("refused".into())
            } else {
                Ok(())
            }
        }
        fn fetch_uri_list(&mut self, id: DataTransferId) -> Result<AsyncRequestSerial, String> {
            self.log.push(format!("fetch {}", id.into_raw()));
            if self.fetch_fails {
                return Err("gone".into());
            }
            let serial = AsyncRequestSerial::get();
            self.serials.push(serial);
            Ok(serial)
        }
        fn emit(&mut self, event: PlatformEvent) {
            self.log.push(format!("emit {event:?}"));
        }
        fn warn(&mut self, message: String) {
            self.log.push(format!("warn {message}"));
        }
    }

    /// The steps reach the host in order: accepted as copy-then-move, the
    /// fetch's serial fed back (so the answer is recognised), events emitted.
    #[cfg(unix)]
    #[test]
    fn run_steps_accepts_fetches_and_emits_in_order() {
        let mut d = OsFileDrag::default();
        let mut host = FakeHost::default();
        let steps = d.entered(id(20), Some((3.0, 4.0)), true);
        run_steps(&mut d, steps, &mut host);
        assert_eq!(
            host.log,
            vec![
                "accept 20 [Copy, Move]".to_owned(),
                "fetch 20".to_owned(),
                format!(
                    "emit {:?}",
                    PlatformEvent::FileDragMoved {
                        position: (3.0, 4.0)
                    }
                ),
            ]
        );
        let serial = host.serials[0];
        let steps = d.received(id(20), serial, Some(vec!["file:///x/y.png".into()]));
        assert_eq!(
            steps.len(),
            1,
            "the fed-back serial is the drag's: {steps:?}"
        );
    }

    /// A refused accept and a failed fetch are logged; the failed fetch's
    /// warning comes once however often the drag moves.
    #[test]
    fn run_steps_logs_failures_once_per_drag() {
        let mut d = OsFileDrag::default();
        let mut host = FakeHost {
            fetch_fails: true,
            accept_fails: true,
            ..Default::default()
        };
        let steps = d.entered(id(21), Some((1.0, 1.0)), true);
        run_steps(&mut d, steps, &mut host);
        for _ in 0..5 {
            let steps = d.moved(id(21), (2.0, 2.0));
            run_steps(&mut d, steps, &mut host);
        }
        let warnings: Vec<_> = host.log.iter().filter(|l| l.starts_with("warn")).collect();
        assert_eq!(
            warnings,
            vec![
                "warn could not accept a file drag: refused",
                "warn could not read a file drag's paths: gone",
            ]
        );
        assert_eq!(
            host.log.iter().filter(|l| l.starts_with("fetch")).count(),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn file_uris_decode_to_paths() {
        assert_eq!(
            file_uri_to_path("file:///tmp/a%20b%25c.png"),
            Some(PathBuf::from("/tmp/a b%c.png"))
        );
        assert_eq!(
            file_uri_to_path("file://localhost/tmp/x"),
            Some(PathBuf::from("/tmp/x"))
        );
        assert_eq!(
            file_uri_to_path("file:///tmp/%C3%A9t%C3%A9\r"),
            Some(PathBuf::from("/tmp/été"))
        );
        assert_eq!(file_uri_to_path("https://example.com/a"), None);
        assert_eq!(file_uri_to_path("not a uri"), None);
    }
}
