//! Wayland: the `org.freedesktop.portal.GlobalShortcuts` portal (KDE Plasma,
//! recent GNOME), spoken over `zbus` on a private thread.
//!
//! A Wayland compositor gives no client the keyboard of another, so the
//! portal is the only route. Its model differs from every other backend:
//!
//! - The app names shortcuts by **id** with a description and a *preferred*
//!   trigger; the **user** approves or changes the trigger in a system dialog,
//!   and the desktop remembers that choice per app and id. So a binding here is
//!   [`Binding::SystemChosen`], with the desktop's own description of it, and
//!   it arrives later through `Report::Binding` — `register` answers
//!   [`Binding::Pending`] at once rather than block the main thread on a dialog.
//! - Activated/Deactivated signals are press and release, so hold-to-talk works.
//! - A modifier-only shortcut has no portal spelling; it is registered with no
//!   preferred trigger and the user picks one.
//! - Every change re-binds the whole set in a fresh session (portal v1 has no
//!   way to add to a bound session). The desktop shows its dialog only for ids
//!   it has no choice for yet.
//!
//! **Why raw zbus and a tokio runtime, not `ashpd`.** `ashpd` refuses to build
//! with both of its runtime features, and the workspace already has one crate
//! on each (`rfd` on async-std, `rinch-screen-capture` on tokio), so any choice
//! breaks a combination of rinch features. `zbus` builds with both and then
//! runs in tokio mode, where it spawns its tasks with `tokio::spawn`. So this
//! thread asks zbus for `async-io` and still runs everything inside a
//! current-thread tokio runtime: right in either mode, whatever else in the
//! app (the `system-tray` feature's `ksni` turns tokio mode on) unifies in.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use futures_lite::stream::{self, StreamExt};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use winit::keyboard::KeyCode;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, Proxy};

use super::BackendImpl;
use crate::hotkey::matcher::BindingKey;
use crate::hotkey::{
    Backend, Binding, HotkeyError, HotkeyState, Meta, Modifier, Report, Shortcut, Trigger, report,
};

const DEST: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const IFACE: &str = "org.freedesktop.portal.GlobalShortcuts";

enum Cmd {
    Set(BindingKey, Meta, Option<String>),
    Remove(BindingKey),
}

enum Ev {
    Cmd(Cmd),
    Activated(String),
    Deactivated(String),
    Changed(Vec<(String, String)>),
}

pub(crate) struct PortalBackend {
    tx: UnboundedSender<Cmd>,
}

impl PortalBackend {
    pub(crate) fn start() -> Result<Self, HotkeyError> {
        let (tx, rx) = unbounded_channel::<Cmd>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        std::thread::Builder::new()
            .name("rinch-hotkey-portal".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e.to_string()));
                        return;
                    }
                };
                rt.block_on(run(rx, ready_tx));
            })
            .map_err(|e| HotkeyError::Platform(e.to_string()))?;
        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(PortalBackend { tx }),
            Ok(Err(e)) => Err(HotkeyError::Unsupported(format!(
                "the GlobalShortcuts portal is not available: {e}"
            ))),
            Err(_) => Err(HotkeyError::Unsupported(
                "the GlobalShortcuts portal did not answer within 5 s".into(),
            )),
        }
    }
}

static TOKEN: AtomicU32 = AtomicU32::new(0);

fn token() -> String {
    format!("rinch{}", TOKEN.fetch_add(1, Ordering::Relaxed))
}

/// Call a portal method that answers through a `Request` object, and wait
/// for its `Response`: `(code, results)`, code 0 meaning success.
async fn request<B>(
    conn: &Connection,
    portal: &Proxy<'_>,
    method: &str,
    token: &str,
    body: &B,
) -> zbus::Result<(u32, HashMap<String, OwnedValue>)>
where
    B: zbus::export::serde::ser::Serialize + zbus::zvariant::DynamicType,
{
    // Subscribe before calling: the request path is predictable, and a fast
    // portal could answer before a later subscription.
    let sender = conn
        .unique_name()
        .map(|n| n.trim_start_matches(':').replace('.', "_"))
        .unwrap_or_default();
    let path = format!("{PATH}/request/{sender}/{token}");
    let req = Proxy::new(conn, DEST, path, "org.freedesktop.portal.Request").await?;
    let mut responses = req.receive_signal("Response").await?;
    let _: OwnedObjectPath = portal.call(method, body).await?;
    let msg = responses
        .next()
        .await
        .ok_or_else(|| zbus::Error::Failure("the portal sent no Response".into()))?;
    msg.body().deserialize()
}

type ShortcutList = Vec<(String, HashMap<String, OwnedValue>)>;

fn trigger_descriptions(list: ShortcutList) -> Vec<(String, String)> {
    list.into_iter()
        .map(|(id, props)| {
            let trigger = props
                .get("trigger_description")
                .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
                .unwrap_or_default();
            (id, trigger)
        })
        .collect()
}

async fn create_session(conn: &Connection, portal: &Proxy<'_>) -> zbus::Result<OwnedObjectPath> {
    let t = token();
    let mut opts: HashMap<&str, Value<'_>> = HashMap::new();
    opts.insert("handle_token", Value::from(t.as_str()));
    let st = token();
    opts.insert("session_handle_token", Value::from(st.as_str()));
    let (code, results) = request(conn, portal, "CreateSession", &t, &(opts,)).await?;
    if code != 0 {
        return Err(zbus::Error::Failure(format!(
            "CreateSession answered {code}"
        )));
    }
    let handle = results
        .get("session_handle")
        .ok_or_else(|| zbus::Error::Failure("no session_handle".into()))?
        .try_clone()?;
    // The specification says `s`; some portals send `o`.
    let path = match String::try_from(handle.try_clone()?) {
        Ok(s) => s,
        Err(_) => OwnedObjectPath::try_from(handle)?.to_string(),
    };
    Ok(OwnedObjectPath::try_from(path)?)
}

async fn close_session(conn: &Connection, session: &OwnedObjectPath) {
    if let Ok(p) = Proxy::new(
        conn,
        DEST,
        session.as_ref(),
        "org.freedesktop.portal.Session",
    )
    .await
    {
        let _ = p.call_method("Close", &()).await;
    }
}

/// Bind the whole set in `session`: per binding, what the desktop answered.
async fn bind(
    conn: &Connection,
    portal: &Proxy<'_>,
    session: &OwnedObjectPath,
    bound: &BTreeMap<BindingKey, (Meta, Option<String>)>,
) -> zbus::Result<Vec<(String, String)>> {
    let shortcuts: Vec<(String, HashMap<&str, Value<'_>>)> = bound
        .values()
        .map(|(m, pref)| {
            let mut props: HashMap<&str, Value<'_>> = HashMap::new();
            props.insert("description", Value::from(m.description.as_str()));
            if let Some(p) = pref {
                props.insert("preferred_trigger", Value::from(p.as_str()));
            }
            (m.id.clone(), props)
        })
        .collect();
    let t = token();
    let mut opts: HashMap<&str, Value<'_>> = HashMap::new();
    opts.insert("handle_token", Value::from(t.as_str()));
    let (code, results) = request(
        conn,
        portal,
        "BindShortcuts",
        &t,
        &(session, shortcuts, "", opts),
    )
    .await?;
    if code != 0 {
        return Err(zbus::Error::Failure(format!(
            "BindShortcuts answered {code} (dismissed)"
        )));
    }
    let list: ShortcutList = match results.get("shortcuts") {
        Some(v) => ShortcutList::try_from(v.try_clone()?)?,
        None => Vec::new(),
    };
    Ok(trigger_descriptions(list))
}

async fn run(mut rx: UnboundedReceiver<Cmd>, ready: mpsc::Sender<Result<(), String>>) {
    let setup = async {
        let conn = Connection::session().await?;
        // An unsandboxed app has no app id the portal can see, and KDE's
        // GlobalShortcuts refuses it ("An app id is required", measured on
        // Plasma 6). xdg-desktop-portal 1.19+ lets a host app say which one it
        // is — the id its window carries, so the desktop files the shortcuts
        // under the same application — with `Registry.Register`, which must be
        // the connection's first portal call. An older portal has no registry;
        // that error is ignored and the next call says whether it matters.
        let app_id = crate::hotkey::app_id();
        let mut registered = Err(String::from("the portal has no host Registry"));
        if let Ok(registry) =
            Proxy::new(&conn, DEST, PATH, "org.freedesktop.host.portal.Registry").await
        {
            let register = || async {
                let opts: HashMap<&str, Value<'_>> = HashMap::new();
                registry
                    .call_method("Register", &(app_id.as_str(), opts))
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            };
            registered = register().await;
            // No installed entry: write a NoDisplay stub (unless the app said
            // not to) and ask again while the desktop's app index picks it up.
            if registered
                .as_ref()
                .is_err_and(|e| e.contains("App info not found"))
                && crate::hotkey::may_write_desktop_entry()
            {
                match crate::shell::rinch_runtime::ensure_desktop_entry(&app_id) {
                    Ok(Some(path)) => {
                        tracing::info!("global hotkeys: wrote {} for the portal", path.display());
                        for _ in 0..20 {
                            tokio::time::sleep(Duration::from_millis(100)).await;
                            registered = register().await;
                            if registered.is_ok() {
                                break;
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::warn!("global hotkeys: could not write a desktop entry: {e}")
                    }
                }
            }
        }
        // The portal accepts only an app id with an installed desktop entry
        // ("App info not found"): say so, since the refusal that follows
        // ("An app id is required") does not.
        let explain = |e: zbus::Error| match &registered {
            Err(why) => zbus::Error::Failure(format!(
                "{e}; registering app id {app_id:?} failed: {why} (the desktop needs \
                 {app_id}.desktop in an applications directory; see hotkey::set_app_id)"
            )),
            Ok(()) => e,
        };
        let portal = Proxy::new(&conn, DEST, PATH, IFACE).await?;
        let activated = portal.receive_signal("Activated").await?;
        let deactivated = portal.receive_signal("Deactivated").await?;
        let changed = portal.receive_signal("ShortcutsChanged").await?;
        let session = create_session(&conn, &portal).await.map_err(explain)?;
        Ok::<_, zbus::Error>((conn, portal, activated, deactivated, changed, session))
    };
    let (conn, portal, activated, deactivated, changed, mut session) = match setup.await {
        Ok(v) => {
            let _ = ready.send(Ok(()));
            v
        }
        Err(e) => {
            let _ = ready.send(Err(e.to_string()));
            return;
        }
    };
    // Signal bodies: (session_handle, shortcut_id, timestamp, options) and
    // (session_handle, shortcuts). Only this session's are ours.
    type Act = (OwnedObjectPath, String, u64, HashMap<String, OwnedValue>);
    let current = std::rc::Rc::new(std::cell::RefCell::new(session.clone()));
    let (c1, c2, c3) = (current.clone(), current.clone(), current.clone());
    // `or` polls left to right, so when a press and its release are both
    // waiting (they are two signal streams) the press is taken first.
    let mut events = activated
        .filter_map(move |m| {
            let (s, id, _, _): Act = m.body().deserialize().ok()?;
            (s == *c1.borrow()).then_some(Ev::Activated(id))
        })
        .or(deactivated.filter_map(move |m| {
            let (s, id, _, _): Act = m.body().deserialize().ok()?;
            (s == *c2.borrow()).then_some(Ev::Deactivated(id))
        }))
        .or(changed.filter_map(move |m| {
            let (s, list): (OwnedObjectPath, ShortcutList) = m.body().deserialize().ok()?;
            (s == *c3.borrow()).then(|| Ev::Changed(trigger_descriptions(list)))
        }))
        .or(stream::poll_fn(move |cx| rx.poll_recv(cx)).map(Ev::Cmd))
        .boxed_local();
    let mut bound: BTreeMap<BindingKey, (Meta, Option<String>)> = BTreeMap::new();
    let key_of = |bound: &BTreeMap<BindingKey, (Meta, Option<String>)>, id: &str| {
        bound.iter().find(|(_, (m, _))| m.id == id).map(|(k, _)| *k)
    };
    while let Some(ev) = events.next().await {
        match ev {
            Ev::Activated(id) => {
                if let Some(k) = key_of(&bound, &id) {
                    report(k, Report::State(HotkeyState::Pressed));
                }
            }
            Ev::Deactivated(id) => {
                if let Some(k) = key_of(&bound, &id) {
                    report(k, Report::State(HotkeyState::Released));
                }
            }
            Ev::Changed(list) => {
                for (id, trigger) in list {
                    if let Some(k) = key_of(&bound, &id) {
                        report(k, Report::Binding(system_chosen(&trigger)));
                    }
                }
            }
            Ev::Cmd(cmd) => {
                match cmd {
                    Cmd::Set(k, meta, preferred) => {
                        bound.insert(k, (meta, preferred));
                    }
                    Cmd::Remove(k) => {
                        bound.remove(&k);
                    }
                }
                // Each change re-binds the whole set. A command that arrives
                // while the desktop's dialog is open waits for the next pass.
                close_session(&conn, &session).await;
                session = match create_session(&conn, &portal).await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("global hotkeys: the portal refused a session: {e}");
                        for k in bound.keys() {
                            report(*k, Report::Binding(Binding::Declined));
                        }
                        continue;
                    }
                };
                *current.borrow_mut() = session.clone();
                if bound.is_empty() {
                    continue;
                }
                match bind(&conn, &portal, &session, &bound).await {
                    Ok(bound_now) => {
                        for (k, (m, _)) in &bound {
                            let b = bound_now
                                .iter()
                                .find(|(id, _)| *id == m.id)
                                .map(|(_, t)| system_chosen(t))
                                .unwrap_or(Binding::Declined);
                            report(*k, Report::Binding(b));
                        }
                    }
                    Err(e) => {
                        tracing::info!("global hotkeys: the portal bound nothing: {e}");
                        for k in bound.keys() {
                            report(*k, Report::Binding(Binding::Declined));
                        }
                    }
                }
            }
        }
    }
}

fn system_chosen(trigger: &str) -> Binding {
    Binding::SystemChosen {
        description: (!trigger.is_empty()).then(|| trigger.to_string()),
    }
}

impl BackendImpl for PortalBackend {
    fn kind(&self) -> Backend {
        Backend::WaylandPortal
    }

    fn register(
        &mut self,
        key: BindingKey,
        shortcut: Option<&Shortcut>,
        meta: &Meta,
    ) -> Result<Binding, HotkeyError> {
        let preferred = shortcut.and_then(xdg_trigger);
        self.tx
            .send(Cmd::Set(key, meta.clone(), preferred))
            .map_err(|_| HotkeyError::Platform("the portal thread has stopped".into()))?;
        Ok(Binding::Pending)
    }

    fn unregister(&mut self, key: BindingKey) {
        let _ = self.tx.send(Cmd::Remove(key));
    }
}

/// A shortcut in the XDG shortcuts specification's spelling
/// (`"CTRL+ALT+space"`), for the portal's `preferred_trigger`. `None` for a
/// modifier-only shortcut, which the specification cannot spell, and for a
/// key it has no name for: the user picks the trigger instead. Sides are
/// dropped: the specification has none.
pub(crate) fn xdg_trigger(sc: &Shortcut) -> Option<String> {
    let Trigger::Key(code) = sc.trigger() else {
        return None;
    };
    let mut s = String::new();
    for (m, _) in sc.held_modifiers() {
        s.push_str(match m {
            Modifier::Ctrl => "CTRL+",
            Modifier::Alt => "ALT+",
            Modifier::Shift => "SHIFT+",
            Modifier::Super => "LOGO+",
        });
    }
    s.push_str(&keysym_name(code)?);
    Some(s)
}

fn keysym_name(code: KeyCode) -> Option<String> {
    let name = code.to_string();
    if let Some(l) = name.strip_prefix("Key") {
        return Some(l.to_ascii_lowercase());
    }
    if let Some(d) = name.strip_prefix("Digit") {
        return Some(d.to_string());
    }
    if name.len() <= 3 && name.starts_with('F') && name[1..].parse::<u8>().is_ok() {
        return Some(name);
    }
    Some(
        match code {
            KeyCode::Space => "space",
            KeyCode::Enter => "Return",
            KeyCode::Escape => "Escape",
            KeyCode::Tab => "Tab",
            KeyCode::Backspace => "BackSpace",
            KeyCode::Delete => "Delete",
            KeyCode::Insert => "Insert",
            KeyCode::Home => "Home",
            KeyCode::End => "End",
            KeyCode::PageUp => "Prior",
            KeyCode::PageDown => "Next",
            KeyCode::ArrowUp => "Up",
            KeyCode::ArrowDown => "Down",
            KeyCode::ArrowLeft => "Left",
            KeyCode::ArrowRight => "Right",
            KeyCode::Pause => "Pause",
            KeyCode::PrintScreen => "Print",
            KeyCode::ScrollLock => "Scroll_Lock",
            KeyCode::Minus => "minus",
            KeyCode::Equal => "equal",
            KeyCode::Backquote => "grave",
            KeyCode::Backslash => "backslash",
            KeyCode::BracketLeft => "bracketleft",
            KeyCode::BracketRight => "bracketright",
            KeyCode::Comma => "comma",
            KeyCode::Period => "period",
            KeyCode::Quote => "apostrophe",
            KeyCode::Semicolon => "semicolon",
            KeyCode::Slash => "slash",
            _ => return None,
        }
        .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Option<String> {
        xdg_trigger(&Shortcut::parse(s).unwrap())
    }

    #[test]
    fn shortcuts_are_spelled_the_xdg_way() {
        assert_eq!(t("Ctrl+Alt+Space").as_deref(), Some("CTRL+ALT+space"));
        assert_eq!(t("Super+Shift+K").as_deref(), Some("SHIFT+LOGO+k"));
        assert_eq!(t("F13").as_deref(), Some("F13"));
        assert_eq!(t("Ctrl+PageDown").as_deref(), Some("CTRL+Next"));
        assert_eq!(t("RightCtrl+Slash").as_deref(), Some("CTRL+slash"));
        assert_eq!(t("RightCtrl"), None, "modifier-only: the user picks");
        assert_eq!(t("Numpad1"), None);
    }

    /// Live: talks to the session's portal and creates a session. Creating a
    /// session shows no dialog (binding does, and this does not bind).
    #[test]
    #[ignore = "needs a desktop session with the GlobalShortcuts portal"]
    fn the_portal_starts() {
        // An id with no desktop entry: the backend writes the NoDisplay stub,
        // which the portal then accepts. The stub is removed afterwards.
        let id = "org.rinch.HotkeyProbe";
        crate::hotkey::set_app_id(id);
        let stub = std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(format!(".local/share/applications/{id}.desktop"));
        let started = PortalBackend::start();
        let wrote = std::fs::read_to_string(&stub).is_ok_and(|t| t.contains("X-Rinch-Generated"));
        if wrote {
            let _ = std::fs::remove_file(&stub);
        }
        started.expect("portal");
        assert!(wrote, "the stub was written");
    }
}
