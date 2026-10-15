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
///
/// A server id means something only to the server that issued it: a
/// notification server that restarts (plasmashell, dunst, mako) numbers from
/// the start again, and its ids then name other notifications. So every
/// mapping records the **unique bus name** of the server that answered its
/// `Notify`, a signal is matched on (sender, id), and when the well-known name
/// changes owner every mapping of another owner is ended
/// ([`Ids::owner_changed`]): those notifications went with their server.
#[derive(Default)]
struct Ids {
    by_server: HashMap<(String, u32), u64>,
    by_local: HashMap<u64, (String, u32)>,
    /// Signals for a server id not (yet) mapped: a server may close a
    /// notification before its `Notify` reply has been read (one that drops
    /// everything while "do not disturb" is on). Signals for other apps'
    /// notifications land here too, so it is bounded.
    early: Vec<((String, u32), Event)>,
}

const EARLY_CAP: usize = 32;

impl Ids {
    /// The rinch id a signal from `sender` about `server` is for, forgetting
    /// the pair (a click or a close ends a notification). An unknown pair
    /// keeps `ev` for [`Ids::insert`].
    fn take(&mut self, sender: &str, server: u32, ev: Event) -> Option<u64> {
        let key = (sender.to_string(), server);
        let Some(local) = self.by_server.remove(&key) else {
            if self.early.len() == EARLY_CAP {
                self.early.remove(0);
            }
            self.early.push((key, ev));
            return None;
        };
        self.by_local.remove(&local);
        Some(local)
    }

    /// Map `(sender, server)` to `local`, or answer the event that arrived
    /// first. A pair still mapped to an older notification means that one is
    /// gone (its id was reused); it is answered in `stale`.
    fn insert(
        &mut self,
        sender: &str,
        server: u32,
        local: u64,
        stale: &mut Vec<u64>,
    ) -> Option<Event> {
        let key = (sender.to_string(), server);
        if let Some(i) = self.early.iter().position(|(k, _)| *k == key) {
            return Some(self.early.remove(i).1);
        }
        if let Some(old) = self.by_server.insert(key.clone(), local) {
            self.by_local.remove(&old);
            stale.push(old);
        }
        self.by_local.insert(local, key);
        None
    }

    /// The well-known name now belongs to `owner` (`None`: nobody). Every
    /// notification mapped under another owner ended with its server:
    /// forget it and answer it.
    fn owner_changed(&mut self, owner: Option<&str>) -> Vec<u64> {
        let gone: Vec<u64> = self
            .by_local
            .iter()
            .filter(|(_, (o, _))| Some(o.as_str()) != owner)
            .map(|(l, _)| *l)
            .collect();
        for l in &gone {
            if let Some(key) = self.by_local.remove(l) {
                self.by_server.remove(&key);
            }
        }
        self.early.retain(|((o, _), _)| Some(o.as_str()) == owner);
        gone
    }
}

struct Live {
    proxy: Proxy<'static>,
}

async fn connect(address: Option<&str>) -> zbus::Result<Connection> {
    match address {
        Some(a) => zbus::connection::Builder::address(a)?.build().await,
        None => Connection::session().await,
    }
}

/// Connect, and start the tasks that turn the server's signals — and its
/// going away — into [`Event`]s.
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
                let Some(sender) = sender_of(&msg) else {
                    continue;
                };
                if key != DEFAULT_ACTION {
                    continue;
                }
                let local = ids.lock().unwrap_or_else(|p| p.into_inner()).take(
                    &sender,
                    server,
                    Event::Clicked,
                );
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
                let Some(sender) = sender_of(&msg) else {
                    continue;
                };
                let ev = Event::Dismissed(dismiss_reason(reason));
                let local =
                    ids.lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .take(&sender, server, ev.clone());
                if let Some(local) = local {
                    sink(local, ev);
                }
            }
        });
    }
    let mut owners = proxy
        .receive_owner_changed()
        .await
        .map_err(|e| NotificationError::Platform(e.to_string()))?;
    {
        let ids = ids.clone();
        let sink = sink.clone();
        tokio::spawn(async move {
            while let Some(owner) = owners.next().await {
                let gone = ids
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .owner_changed(owner.as_ref().map(|o| o.as_str()));
                for local in gone {
                    sink(local, Event::Dismissed(DismissReason::Unknown));
                }
            }
        });
    }
    Ok(Live { proxy })
}

/// The unique name that owns the notification service now.
async fn current_owner(live: &Live) -> Option<String> {
    let dbus = zbus::fdo::DBusProxy::new(live.proxy.connection())
        .await
        .ok()?;
    let name = zbus::names::BusName::try_from(DEST).ok()?;
    dbus.get_name_owner(name).await.ok().map(|o| o.to_string())
}

fn sender_of(msg: &zbus::Message) -> Option<String> {
    msg.header().sender().map(|s| s.to_string())
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
        zbus::Error::InputOutput(_) => {
            NotificationError::NoService(format!("the D-Bus session bus went away: {e}"))
        }
        _ => NotificationError::Platform(e.to_string()),
    }
}

/// `Notify`: the server's id, and the unique name of the server that issued it.
async fn notify(live: &Live, req: &Request) -> zbus::Result<(String, u32)> {
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
    let reply = live
        .proxy
        .call_method(
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
        .await?;
    let id: u32 = reply.body().deserialize()?;
    let sender = sender_of(&reply).unwrap_or_default();
    Ok((sender, id))
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
                    Ok((sender, server)) => {
                        let mut stale = Vec::new();
                        let early = ids
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .insert(&sender, server, req.id, &mut stale);
                        for old in stale {
                            sink(old, Event::Dismissed(DismissReason::Unknown));
                        }
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
                let mapped = ids
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .by_local
                    .get(&local)
                    .cloned();
                let (Some((owner, server)), Some(l)) = (mapped, &live) else {
                    continue;
                };
                // Only to the server that issued the id: after a restart the
                // same number names someone else's notification.
                let now = current_owner(l).await;
                if now.as_deref() != Some(owner.as_str()) {
                    let gone = ids
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .owner_changed(now.as_deref());
                    for g in gone {
                        sink(g, Event::Dismissed(DismissReason::Unknown));
                    }
                    continue;
                }
                // The server answers with NotificationClosed(reason 3).
                let _ = l
                    .proxy
                    .call::<_, _, ()>("CloseNotification", &(server,))
                    .await;
            }
        }
    }
}

#[cfg(test)]
mod ids_tests {
    use super::*;

    #[test]
    fn a_reused_id_from_the_same_server_ends_the_old_mapping() {
        let mut ids = Ids::default();
        let mut stale = Vec::new();
        assert_eq!(ids.insert(":1.5", 41, 7, &mut stale), None);
        assert_eq!(ids.insert(":1.5", 41, 8, &mut stale), None);
        assert_eq!(stale, vec![7]);
        assert_eq!(ids.by_local.get(&7), None, "close(7) must not reach 8's id");
        assert_eq!(ids.take(":1.5", 41, Event::Clicked), Some(8));
    }

    #[test]
    fn a_signal_from_another_owner_is_not_ours() {
        let mut ids = Ids::default();
        let mut stale = Vec::new();
        ids.insert(":1.5", 41, 7, &mut stale);
        assert_eq!(ids.take(":1.9", 41, Event::Clicked), None);
        assert_eq!(ids.take(":1.5", 41, Event::Clicked), Some(7));
    }

    #[test]
    fn an_owner_change_ends_every_other_owners_notifications() {
        let mut ids = Ids::default();
        let mut stale = Vec::new();
        ids.insert(":1.5", 41, 7, &mut stale);
        ids.insert(":1.5", 42, 8, &mut stale);
        ids.insert(":1.9", 41, 9, &mut stale);
        let mut gone = ids.owner_changed(Some(":1.9"));
        gone.sort();
        assert_eq!(gone, vec![7, 8]);
        assert_eq!(ids.owner_changed(None), vec![9]);
        assert!(ids.by_local.is_empty() && ids.by_server.is_empty());
    }
}
