//! Linux: `org.freedesktop.Notifications` over D-Bus (the Desktop
//! Notifications specification, 1.2), spoken over `zbus` on a thread of its
//! own.
//!
//! One connection for the life of the process, made on the first `show` and
//! made again after it fails. `Notify` answers the server's id; the
//! `ActionInvoked` and `NotificationClosed` signals name that id, and are
//! translated back to rinch's.
//!
//! **Why a current-thread tokio runtime** (the reasoning #1521's portal
//! backend gives): `zbus` runs its internal tasks on its own executor in
//! `async-io` mode and on `tokio::spawn` in tokio mode, and another crate in
//! the app (the `system-tray` feature's `ksni`) can switch it to tokio mode.
//! Running everything inside a tokio runtime is right in both.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use futures_lite::StreamExt;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use zbus::zvariant::Value;
use zbus::{Connection, Proxy};

use crate::notification::{Backend, DismissReason, Event, NotificationError, Request, Sink};

const DEST: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const IFACE: &str = "org.freedesktop.Notifications";
/// The action key the specification reserves for a click on the body.
const DEFAULT_ACTION: &str = "default";

enum Cmd {
    Show(Request),
    Close(u64),
}

pub(crate) struct FreedesktopBackend {
    tx: UnboundedSender<Cmd>,
}

impl FreedesktopBackend {
    /// Start the worker. `address` is a D-Bus address to use instead of the
    /// session bus (tests run a private one).
    pub(crate) fn start(address: Option<String>, sink: Sink) -> Result<Self, NotificationError> {
        let (tx, rx) = unbounded_channel::<Cmd>();
        std::thread::Builder::new()
            .name("rinch-notifications".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        tracing::warn!("notifications: no runtime: {e}");
                        return;
                    }
                };
                rt.block_on(run(address, rx, sink));
            })
            .map_err(|e| NotificationError::Platform(e.to_string()))?;
        Ok(FreedesktopBackend { tx })
    }
}

impl Backend for FreedesktopBackend {
    fn show(&self, req: Request) -> Result<(), NotificationError> {
        self.tx
            .send(Cmd::Show(req))
            .map_err(|_| NotificationError::Platform("the notification thread has stopped".into()))
    }

    fn close(&self, id: u64) {
        let _ = self.tx.send(Cmd::Close(id));
    }
}

/// Server id ↔ rinch id, shared with the signal tasks.
#[derive(Default)]
struct Ids {
    by_server: HashMap<u32, u64>,
    by_local: HashMap<u64, u32>,
    /// Signals for a server id not (yet) mapped: a server may close a
    /// notification before its `Notify` reply has been read (one that drops
    /// everything while "do not disturb" is on). Signals for other apps'
    /// notifications land here too, so it is bounded.
    early: Vec<(u32, Event)>,
}

const EARLY_CAP: usize = 32;

struct Live {
    proxy: Proxy<'static>,
}

async fn connect(address: Option<&str>) -> zbus::Result<Connection> {
    match address {
        Some(a) => zbus::connection::Builder::address(a)?.build().await,
        None => Connection::session().await,
    }
}

/// Connect, and start the two tasks that turn the server's signals into
/// [`Event`]s.
async fn open(
    address: Option<&str>,
    ids: &Arc<Mutex<Ids>>,
    sink: &Sink,
) -> Result<Live, NotificationError> {
    let conn = connect(address)
        .await
        .map_err(|e| NotificationError::NoService(format!("no D-Bus session bus: {e}")))?;
    let proxy = Proxy::new(&conn, DEST, PATH, IFACE)
        .await
        .map_err(|e| NotificationError::Platform(e.to_string()))?;
    let mut actions = proxy
        .receive_signal("ActionInvoked")
        .await
        .map_err(|e| NotificationError::Platform(e.to_string()))?;
    let mut closed = proxy
        .receive_signal("NotificationClosed")
        .await
        .map_err(|e| NotificationError::Platform(e.to_string()))?;
    {
        let ids = ids.clone();
        let sink = sink.clone();
        tokio::spawn(async move {
            while let Some(msg) = actions.next().await {
                let Ok((server, key)) = msg.body().deserialize::<(u32, String)>() else {
                    continue;
                };
                if key != DEFAULT_ACTION {
                    continue;
                }
                let local = ids
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .take(server, Event::Clicked);
                if let Some(local) = local {
                    sink(local, Event::Clicked);
                }
            }
        });
    }
    {
        let ids = ids.clone();
        let sink = sink.clone();
        tokio::spawn(async move {
            while let Some(msg) = closed.next().await {
                let Ok((server, reason)) = msg.body().deserialize::<(u32, u32)>() else {
                    continue;
                };
                let ev = Event::Dismissed(dismiss_reason(reason));
                let local = ids
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .take(server, ev.clone());
                if let Some(local) = local {
                    sink(local, ev);
                }
            }
        });
    }
    Ok(Live { proxy })
}

impl Ids {
    /// The rinch id a signal about `server` is for, forgetting the pair (a
    /// click or a close ends a notification). An unknown id keeps `ev` for
    /// [`Ids::insert`].
    fn take(&mut self, server: u32, ev: Event) -> Option<u64> {
        let Some(local) = self.by_server.remove(&server) else {
            if self.early.len() == EARLY_CAP {
                self.early.remove(0);
            }
            self.early.push((server, ev));
            return None;
        };
        self.by_local.remove(&local);
        Some(local)
    }

    /// Map `server` to `local`, or answer the event that arrived first.
    fn insert(&mut self, server: u32, local: u64) -> Option<Event> {
        if let Some(i) = self.early.iter().position(|(s, _)| *s == server) {
            return Some(self.early.remove(i).1);
        }
        self.by_server.insert(server, local);
        self.by_local.insert(local, server);
        None
    }
}

/// The specification's `NotificationClosed` reasons.
pub(crate) fn dismiss_reason(reason: u32) -> DismissReason {
    match reason {
        1 => DismissReason::Expired,
        2 => DismissReason::Dismissed,
        3 => DismissReason::Closed,
        _ => DismissReason::Unknown,
    }
}

fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for b in path.to_string_lossy().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                uri.push(b as char)
            }
            _ => uri.push_str(&format!("%{b:02X}")),
        }
    }
    uri
}

fn method_error(e: zbus::Error) -> NotificationError {
    match &e {
        zbus::Error::MethodError(name, _, _)
            if name.as_str() == "org.freedesktop.DBus.Error.ServiceUnknown"
                || name.as_str() == "org.freedesktop.DBus.Error.NameHasNoOwner" =>
        {
            NotificationError::NoService(format!(
                "no notification server ({DEST}) on the session bus"
            ))
        }
        _ => NotificationError::Platform(e.to_string()),
    }
}

async fn notify(live: &Live, req: &Request) -> zbus::Result<u32> {
    let actions: Vec<&str> = if req.wants_click {
        // The label is what a server that draws buttons shows; the default
        // action is usually a click on the body and its label is not drawn.
        vec![DEFAULT_ACTION, "Open"]
    } else {
        Vec::new()
    };
    let mut hints: HashMap<&str, Value<'_>> = HashMap::new();
    hints.insert("desktop-entry", Value::from(req.app_id.as_str()));
    let image = req.icon.as_deref().map(file_uri);
    if let Some(image) = &image {
        hints.insert("image-path", Value::from(image.as_str()));
    }
    live.proxy
        .call(
            "Notify",
            &(
                req.app_name.as_str(),
                0u32,
                "",
                req.title.as_str(),
                req.body.as_str(),
                actions,
                hints,
                -1i32,
            ),
        )
        .await
}

async fn run(address: Option<String>, mut rx: UnboundedReceiver<Cmd>, sink: Sink) {
    let ids = Arc::new(Mutex::new(Ids::default()));
    let mut live: Option<Live> = None;
    while let Some(cmd) = rx.recv().await {
        match cmd {
            Cmd::Show(req) => {
                if live.is_none() {
                    match open(address.as_deref(), &ids, &sink).await {
                        Ok(l) => live = Some(l),
                        Err(e) => {
                            sink(req.id, Event::Failed(e));
                            continue;
                        }
                    }
                }
                let Some(l) = &live else { continue };
                match notify(l, &req).await {
                    Ok(server) => {
                        let early = ids
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .insert(server, req.id);
                        if let Some(ev) = early {
                            sink(req.id, ev);
                        }
                    }
                    Err(e) => {
                        // A dead connection is made again on the next show.
                        if matches!(e, zbus::Error::InputOutput(_)) {
                            live = None;
                        }
                        sink(req.id, Event::Failed(method_error(e)));
                    }
                }
            }
            Cmd::Close(local) => {
                let server = ids
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .by_local
                    .get(&local)
                    .copied();
                if let (Some(server), Some(l)) = (server, &live) {
                    // The server answers with NotificationClosed(reason 3).
                    let _ = l
                        .proxy
                        .call::<_, _, ()>("CloseNotification", &(server,))
                        .await;
                }
            }
        }
    }
}
