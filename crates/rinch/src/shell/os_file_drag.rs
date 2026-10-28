//! An OS file drag over the window, translated from winit's data transfer API
//! into rinch's file-drag events (#1962).
//!
//! Since winit 0.31.0-beta.3 a drag is a *data transfer*: `DragEntered` names
//! a transfer id and maybe a position, `DragPosition` moves it, `DragDropped`
//! and `DragLeft` end it, and the dragged data is fetched asynchronously
//! (`fetch_data_transfer`, answered by `DataTransferReceived`). The drag is
//! rejected unless the app accepts an action (`set_valid_dnd_actions`). rinch
//! wants file paths, so a drag counts only when it offers a URI list; it is
//! accepted as a copy and its URI list is fetched once a position is known
//! (X11 names the position only with the first `XdndPosition`, and asks for
//! the data with that position's timestamp).
//!
//! What the app is handed is unchanged from winit beta.2's path-carrying
//! events: a [`PlatformEvent::FileDragMoved`] at every position, a
//! [`PlatformEvent::FileHoverEnter`] per path once the paths are known, a
//! [`PlatformEvent::FileDropped`] with every path at the last position, and a
//! [`PlatformEvent::FileHoverCancelled`] for a drag that leaves. A drop whose
//! paths have not arrived yet is handed over when they do.
//!
//! [`OsFileDrag`] is the state machine with no winit event loop in it: the
//! runtime feeds it the events (positions already logical) and carries out the
//! [`DragStep`]s it answers.

use std::path::PathBuf;

use rinch_platform::PlatformEvent;
use winit::data_transfer::DataTransferId;
use winit::event_loop::AsyncRequestSerial;

/// What the runtime does for an [`OsFileDrag`] answer, in order.
#[derive(Debug)]
pub(crate) enum DragStep {
    /// Accept the drag as a copy (`set_valid_dnd_actions(id, [Copy])`).
    Accept(DataTransferId),
    /// Ask for the drag's URI list (`fetch_data_transfer(id, UriList)`) and
    /// report the serial with [`OsFileDrag::fetch_started`].
    Fetch(DataTransferId),
    /// Hand the app this event.
    Emit(PlatformEvent),
}

/// The fetch of a drag's URI list.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Fetch {
    /// Not asked for yet (no position known), or the request failed.
    NotStarted,
    /// Asked for; the answer carries this serial.
    Pending(AsyncRequestSerial),
    /// Answered.
    Done,
}

/// The one OS file drag in flight.
#[derive(Debug)]
struct Drag {
    id: DataTransferId,
    /// The last position, logical px.
    position: Option<(f64, f64)>,
    fetch: Fetch,
    /// The paths, once the URI list has been read.
    paths: Option<Vec<PathBuf>>,
    /// Dropped; the drop waits for the paths.
    dropped: bool,
}

/// The OS file drag over the window, if any.
#[derive(Debug, Default)]
pub(crate) struct OsFileDrag {
    drag: Option<Drag>,
}

impl OsFileDrag {
    /// `DragEntered`. `has_uri_list`: the transfer offers a URI list. A drag
    /// without one (text, an image's bytes) is not a file drag and is left
    /// rejected; its later events are ignored.
    pub(crate) fn entered(
        &mut self,
        id: DataTransferId,
        position: Option<(f64, f64)>,
        has_uri_list: bool,
    ) -> Vec<DragStep> {
        self.drag = None;
        if !has_uri_list {
            return Vec::new();
        }
        self.drag = Some(Drag {
            id,
            position: None,
            fetch: Fetch::NotStarted,
            paths: None,
            dropped: false,
        });
        let mut steps = vec![DragStep::Accept(id)];
        if let Some(position) = position {
            steps.extend(self.moved(id, position));
        }
        steps
    }

    /// `DragPosition`.
    pub(crate) fn moved(&mut self, id: DataTransferId, position: (f64, f64)) -> Vec<DragStep> {
        let Some(drag) = self.drag.as_mut().filter(|d| d.id == id && !d.dropped) else {
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

    /// The runtime asked for the URI list ([`DragStep::Fetch`]): `Some` with
    /// the request's serial, `None` when the request failed. A drop waiting
    /// on a request that failed ends with no paths.
    pub(crate) fn fetch_started(
        &mut self,
        id: DataTransferId,
        serial: Option<AsyncRequestSerial>,
    ) -> Vec<DragStep> {
        let Some(drag) = self.drag.as_mut().filter(|d| d.id == id) else {
            return Vec::new();
        };
        match serial {
            Some(serial) => {
                drag.fetch = Fetch::Pending(serial);
                Vec::new()
            }
            None => {
                drag.fetch = Fetch::NotStarted;
                if drag.dropped {
                    self.paths_read(id, Vec::new())
                } else {
                    Vec::new()
                }
            }
        }
    }

    /// `DataTransferReceived` for this drag's request `serial`: `uris` is the
    /// URI list read (`None` while it would block; it comes again with the
    /// next `DataTransferReceived`), or an empty list when it cannot be read.
    pub(crate) fn received(
        &mut self,
        id: DataTransferId,
        serial: AsyncRequestSerial,
        uris: Option<Vec<String>>,
    ) -> Vec<DragStep> {
        let Some(drag) = self.drag.as_mut() else {
            return Vec::new();
        };
        if drag.id != id || drag.fetch != Fetch::Pending(serial) {
            return Vec::new();
        }
        let Some(uris) = uris else {
            return Vec::new();
        };
        let paths = uris
            .iter()
            .filter_map(|uri| file_uri_to_path(uri))
            .collect();
        self.paths_read(id, paths)
    }

    /// The paths are known: a hover per path, or the drop it was waiting for.
    fn paths_read(&mut self, id: DataTransferId, paths: Vec<PathBuf>) -> Vec<DragStep> {
        let Some(drag) = self.drag.as_mut().filter(|d| d.id == id) else {
            return Vec::new();
        };
        drag.fetch = Fetch::Done;
        let position = drag.position.unwrap_or_default();
        if drag.dropped {
            self.drag = None;
            return vec![DragStep::Emit(PlatformEvent::FileDropped {
                paths,
                position,
            })];
        }
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
        steps
    }

    /// `DragDropped`: the drop, now if the paths are known, else when they
    /// arrive (asking for them now if nothing has yet).
    pub(crate) fn dropped(&mut self, id: DataTransferId) -> Vec<DragStep> {
        let Some(drag) = self.drag.as_mut().filter(|d| d.id == id && !d.dropped) else {
            return Vec::new();
        };
        drag.dropped = true;
        if let Some(paths) = drag.paths.take() {
            let position = drag.position.unwrap_or_default();
            self.drag = None;
            return vec![DragStep::Emit(PlatformEvent::FileDropped {
                paths,
                position,
            })];
        }
        match drag.fetch {
            Fetch::NotStarted => vec![DragStep::Fetch(id)],
            Fetch::Pending(_) | Fetch::Done => Vec::new(),
        }
    }

    /// `DragLeft`: the drag left the window, or was cancelled. A dropped drag
    /// waiting for its paths is not cancelled by it.
    pub(crate) fn left(&mut self, id: DataTransferId) -> Vec<DragStep> {
        match &self.drag {
            Some(drag) if drag.id == id && !drag.dropped => {
                self.drag = None;
                vec![DragStep::Emit(PlatformEvent::FileHoverCancelled)]
            }
            _ => Vec::new(),
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
        assert!(d.fetch_started(id(7), Some(serial)).is_empty());
        let steps = d.moved(id(7), (30.0, 40.0));
        assert_steps(
            steps,
            vec![DragStep::Emit(PlatformEvent::FileDragMoved {
                position: (30.0, 40.0),
            })],
        );
        let steps = d.received(id(7), serial, Some(uris()));
        assert_steps(
            steps,
            paths()
                .into_iter()
                .map(|path| {
                    DragStep::Emit(PlatformEvent::FileHoverEnter {
                        path,
                        position: (30.0, 40.0),
                    })
                })
                .collect::<Vec<_>>(),
        );
        let steps = d.moved(id(7), (31.0, 41.0));
        assert_eq!(steps.len(), 1, "no second fetch: {steps:?}");
        let steps = d.dropped(id(7));
        assert_steps(
            steps,
            vec![DragStep::Emit(PlatformEvent::FileDropped {
                paths: paths(),
                position: (31.0, 41.0),
            })],
        );
        // The drag is over: a late leave cancels nothing.
        assert!(d.left(id(7)).is_empty());
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
        d.fetch_started(id(2), Some(serial));
        assert!(d.dropped(id(2)).is_empty());
        // Neither a later move nor a leave touches a dropped drag.
        assert!(d.moved(id(2), (9.0, 9.0)).is_empty());
        assert!(d.left(id(2)).is_empty());
        assert!(d.received(id(2), serial, None).is_empty(), "would block");
        let other = AsyncRequestSerial::get();
        assert!(
            d.received(id(2), other, Some(uris())).is_empty(),
            "not its serial"
        );
        assert_steps(
            d.received(id(2), serial, Some(uris())),
            vec![DragStep::Emit(PlatformEvent::FileDropped {
                paths: paths(),
                position: (1.0, 2.0),
            })],
        );
    }

    /// A drop whose fetch was never started (it failed while hovering) asks
    /// again; a drop whose fetch fails ends with no paths.
    #[test]
    fn a_drop_with_no_fetch_asks_and_a_failed_fetch_drops_nothing() {
        let mut d = OsFileDrag::default();
        d.entered(id(3), Some((4.0, 5.0)), true);
        assert!(d.fetch_started(id(3), None).is_empty());
        assert_steps(d.dropped(id(3)), vec![DragStep::Fetch(id(3))]);
        assert_steps(
            d.fetch_started(id(3), None),
            vec![DragStep::Emit(PlatformEvent::FileDropped {
                paths: Vec::new(),
                position: (4.0, 5.0),
            })],
        );
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

    /// Text or an image's bytes is no file drag: not accepted, no events.
    #[test]
    fn a_drag_without_a_uri_list_is_ignored() {
        let mut d = OsFileDrag::default();
        assert!(d.entered(id(5), Some((1.0, 1.0)), false).is_empty());
        assert!(d.moved(id(5), (2.0, 2.0)).is_empty());
        assert!(d.dropped(id(5)).is_empty());
        assert!(d.left(id(5)).is_empty());
    }

    /// A new drag replaces one whose end never came.
    #[test]
    fn a_new_drag_replaces_a_stale_one() {
        let mut d = OsFileDrag::default();
        d.entered(id(8), Some((1.0, 1.0)), true);
        d.entered(id(9), Some((2.0, 2.0)), true);
        assert!(d.moved(id(8), (3.0, 3.0)).is_empty());
        assert_eq!(d.moved(id(9), (3.0, 3.0)).len(), 2);
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
