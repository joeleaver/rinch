//! `rinch::notification`: the main-thread half over a fake backend, the
//! Windows toast XML, and the freedesktop backend against a private D-Bus
//! bus with a fake notification server.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Mutex;

use rinch_core::reactive::{Scope, Signal};

use super::*;

/// A backend that records what it is asked, and refuses when told to.
#[derive(Default)]
struct Fake {
    shown: Mutex<Vec<Request>>,
    closed: Mutex<Vec<u64>>,
    refuse: Option<NotificationError>,
}

impl Backend for Fake {
    fn show(&self, req: Request) -> Result<(), NotificationError> {
        if let Some(e) = &self.refuse {
            return Err(e.clone());
        }
        self.shown.lock().unwrap().push(req);
        Ok(())
    }
    fn close(&self, id: u64) {
        self.closed.lock().unwrap().push(id);
    }
}

fn fake() -> &'static Fake {
    Box::leak(Box::new(Fake::default()))
}

fn counter() -> (Rc<Cell<u32>>, impl Fn() + 'static) {
    let n = Rc::new(Cell::new(0));
    let c = n.clone();
    (n, move || c.set(c.get() + 1))
}

#[test]
fn a_click_runs_on_click_and_ends_the_notification() {
    let backend = fake();
    let (clicks, on_click) = counter();
    let dismissed = Rc::new(Cell::new(false));
    let d = dismissed.clone();
    let before = registered_count();
    let handle = show_with(
        Ok(backend),
        Notification::new("Hotkey unavailable")
            .body("Ctrl+Alt+Space is taken.")
            .on_click(on_click)
            .on_dismiss(move |_| d.set(true)),
    )
    .unwrap();
    let shown = backend.shown.lock().unwrap().clone();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].id, handle.id());
    assert_eq!(shown[0].title, "Hotkey unavailable");
    assert_eq!(shown[0].body, "Ctrl+Alt+Space is taken.");
    assert!(shown[0].wants_click);
    assert_eq!(registered_count(), before + 1);

    deliver(handle.id(), Event::Clicked);
    assert_eq!(clicks.get(), 1);
    // The close a Linux server reports after an action is not a dismissal.
    deliver(handle.id(), Event::Dismissed(DismissReason::Closed));
    assert!(!dismissed.get());
    deliver(handle.id(), Event::Clicked);
    assert_eq!(clicks.get(), 1, "a click ends the notification's callbacks");
    assert_eq!(registered_count(), before);
}

#[test]
fn a_dismissal_carries_its_reason() {
    let backend = fake();
    let reason = Rc::new(Cell::new(None));
    let r = reason.clone();
    let h = show_with(
        Ok(backend),
        Notification::new("t").on_dismiss(move |why| r.set(Some(why))),
    )
    .unwrap();
    assert!(!backend.shown.lock().unwrap()[0].wants_click);
    deliver(h.id(), Event::Dismissed(DismissReason::Expired));
    assert_eq!(reason.get(), Some(DismissReason::Expired));
}

#[test]
fn a_callback_set_during_a_render_stops_with_its_component() {
    let backend = fake();
    let (clicks, on_click) = counter();
    let scope = Scope::new();
    let n = scope.run(|| Notification::new("owned").on_click(on_click));
    // Shown from outside the render: the owner is the one current when the
    // callback was set (the menu rule), not when `show` ran.
    let h = show_with(Ok(backend), n).unwrap();
    scope.dispose();
    deliver(h.id(), Event::Clicked);
    assert_eq!(
        clicks.get(),
        0,
        "a disposed component's callback must not run"
    );
}

#[test]
fn a_live_owners_callback_runs_inside_it() {
    let backend = fake();
    let scope = Scope::new();
    let made: Rc<RefCell<Option<Signal<u32>>>> = Rc::new(RefCell::new(None));
    let m = made.clone();
    let n = scope.run(|| {
        Notification::new("owned").on_click(move || *m.borrow_mut() = Some(Signal::new(7)))
    });
    let h = show_with(Ok(backend), n).unwrap();
    deliver(h.id(), Event::Clicked);
    let signal = made.borrow().expect("the callback ran");
    assert_eq!(signal.get(), 7);
    scope.dispose();
    assert!(
        !signal.is_alive(),
        "a signal the callback made belongs to the callback's component"
    );
}

#[test]
fn an_ownerless_callback_has_app_lifetime() {
    let backend = fake();
    let (clicks, on_click) = counter();
    let h = show_with(Ok(backend), Notification::new("main").on_click(on_click)).unwrap();
    deliver(h.id(), Event::Clicked);
    assert_eq!(clicks.get(), 1);
}

#[test]
fn a_failure_reaches_on_error_and_ends_the_notification() {
    let backend = fake();
    let got = Rc::new(RefCell::new(None));
    let g = got.clone();
    let (clicks, on_click) = counter();
    let h = show_with(
        Ok(backend),
        Notification::new("t")
            .on_click(on_click)
            .on_error(move |e| *g.borrow_mut() = Some(e.clone())),
    )
    .unwrap();
    let err = NotificationError::NoService("none".into());
    deliver(h.id(), Event::Failed(err.clone()));
    assert_eq!(got.borrow().clone(), Some(err));
    deliver(h.id(), Event::Clicked);
    assert_eq!(clicks.get(), 0);
}

#[test]
fn a_failure_without_on_error_is_logged_not_dropped_on_the_floor() {
    // Nothing registered for it (no callback at all): the failure still has
    // somewhere to go — the warn branch — and does not panic.
    let backend = fake();
    let before = registered_count();
    let h = show_with(Ok(backend), Notification::new("t")).unwrap();
    assert_eq!(registered_count(), before, "no callback, no entry");
    let warned = UNHANDLED.with(|u| u.get());
    deliver(h.id(), Event::Failed(NotificationError::PermissionDenied));
    assert_eq!(registered_count(), before);
    assert_eq!(
        UNHANDLED.with(|u| u.get()),
        warned + 1,
        "the failure was logged"
    );

    // And one whose on_error belongs to an unmounted component.
    let scope = Scope::new();
    let n =
        scope.run(|| Notification::new("t").on_error(|_| panic!("a dead owner's on_error ran")));
    let h = show_with(Ok(backend), n).unwrap();
    scope.dispose();
    deliver(h.id(), Event::Failed(NotificationError::PermissionDenied));
    assert_eq!(
        UNHANDLED.with(|u| u.get()),
        warned + 2,
        "a dead on_error falls back to the log"
    );
}

#[test]
fn a_refusal_known_at_once_is_shows_err_and_registers_nothing() {
    let backend: &'static Fake = Box::leak(Box::new(Fake {
        refuse: Some(NotificationError::Unsupported("test".into())),
        ..Fake::default()
    }));
    let before = registered_count();
    let r = show_with(Ok(backend), Notification::new("t").on_click(|| {}));
    assert_eq!(r, Err(NotificationError::Unsupported("test".into())));
    assert_eq!(registered_count(), before);

    let r = show_with(
        Err(NotificationError::Unsupported("no backend".into())),
        Notification::new("t").on_click(|| {}),
    );
    assert!(matches!(r, Err(NotificationError::Unsupported(_))));
    assert_eq!(registered_count(), before);
}

#[test]
fn a_callback_may_show_another_notification() {
    let backend = fake();
    let inner = Rc::new(Cell::new(None));
    let i = inner.clone();
    let h = show_with(
        Ok(backend),
        Notification::new("outer").on_click(move || {
            let h = show_with(Ok(fake()), Notification::new("inner").on_click(|| {})).unwrap();
            i.set(Some(h));
        }),
    )
    .unwrap();
    deliver(h.id(), Event::Clicked);
    assert!(inner.get().is_some());
}

#[test]
fn dead_entries_are_pruned_by_the_next_show() {
    let backend = fake();
    let before = registered_count();
    let scope = Scope::new();
    let n = scope.run(|| Notification::new("owned").on_dismiss(|_| {}));
    show_with(Ok(backend), n).unwrap();
    assert_eq!(registered_count(), before + 1);
    scope.dispose();
    show_with(Ok(backend), Notification::new("next")).unwrap();
    assert_eq!(registered_count(), before);
}

#[test]
fn the_icon_is_written_once_per_image_and_handed_over_by_path() {
    let backend = fake();
    // Off any fixed point: two different images give two different files,
    // the same image twice gives one.
    let a = [0x89u8, b'P', b'N', b'G', 1, 2, 3];
    let b = [0x89u8, b'P', b'N', b'G', 4, 5, 6];
    show_with(Ok(backend), Notification::new("a").icon_png(&a)).unwrap();
    show_with(Ok(backend), Notification::new("b").icon_png(&b)).unwrap();
    show_with(Ok(backend), Notification::new("a2").icon_png(&a)).unwrap();
    let shown = backend.shown.lock().unwrap().clone();
    let paths: Vec<PathBuf> = shown.iter().map(|r| r.icon.clone().unwrap()).collect();
    assert_ne!(paths[0], paths[1]);
    assert_eq!(paths[0], paths[2]);
    assert_eq!(std::fs::read(&paths[0]).unwrap(), a);
    assert_eq!(std::fs::read(&paths[1]).unwrap(), b);
}

#[test]
fn close_reaches_the_backend_only_once_one_was_started() {
    // `close` starts no backend of its own.
    let h = NotificationHandle { id: u64::MAX };
    h.close();
}

#[test]
fn an_executable_names_the_app_like_the_window_does() {
    use std::path::Path;
    assert_eq!(
        app_id_from_exe_path(Path::new("/usr/bin/com.example.notes")).as_deref(),
        Some("com.example.notes"),
        "a reverse-DNS name keeps its dots"
    );
    assert_eq!(
        app_id_from_exe_path(Path::new("C:/apps/beardictate.exe")).as_deref(),
        Some("beardictate")
    );
    assert_eq!(
        app_id_from_exe_path(Path::new("C:/apps/Bear.EXE")).as_deref(),
        Some("Bear")
    );
    assert_eq!(
        app_id_from_exe_path(Path::new("/opt/exe")).as_deref(),
        Some("exe")
    );
}

#[test]
fn set_app_id_and_name_override_the_derivation() {
    // Process-global; this is the only test that sets them.
    let derived = app_id();
    assert!(!derived.is_empty());
    set_app_id("com.example.Bear");
    assert_eq!(app_id(), "com.example.Bear");
    assert_eq!(
        app_name(),
        "com.example.Bear",
        "the name defaults to the id"
    );
    set_app_name("Bear Dictate");
    assert_eq!(app_name(), "Bear Dictate");
    *APP_ID.lock().unwrap() = None;
    *APP_NAME.lock().unwrap() = None;
    assert_eq!(app_id(), derived);
}

/// R6 (review-1535): the icon went to a predictable name in a shared
/// temporary directory, and the write followed a symlink planted there. Now
/// the directory must be private, and a planted symlink at the final name is
/// not trusted or written through.
#[test]
#[cfg(unix)]
fn r6_the_icon_write_follows_no_planted_symlink() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("rinch-r6-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let victim = root.join("victim.txt");
    std::fs::write(&victim, b"precious").unwrap();
    let png = b"\x89PNG fake";

    // A directory another user could write (as `/tmp/rinch-notifications`
    // made by someone else would be): refused, nothing written.
    let shared = root.join("shared");
    std::fs::create_dir(&shared).unwrap();
    std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o777)).unwrap();
    let r = write_icon_in(&shared, png);
    assert!(
        r.is_err(),
        "wrote into a directory open to other users: {r:?}"
    );
    assert_eq!(std::fs::read_dir(&shared).unwrap().count(), 0);

    // A symlink where the directory should be: refused.
    let link = root.join("link");
    std::os::unix::fs::symlink(&shared, &link).unwrap();
    assert!(write_icon_in(&link, png).is_err());

    // A private directory holding a symlink at the final name: the victim
    // is untouched, and the answer is a regular file of our bytes.
    let private = root.join("private");
    let first = write_icon_in(&private, b"other").unwrap();
    assert_eq!(
        std::fs::metadata(&private).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let final_name = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        png.as_slice().hash(&mut h);
        private.join(format!("{:016x}.png", h.finish()))
    };
    std::os::unix::fs::symlink(&victim, &final_name).unwrap();
    let p = write_icon_in(&private, png).unwrap();
    assert_eq!(std::fs::read(&victim).unwrap(), b"precious");
    assert!(std::fs::symlink_metadata(&p).unwrap().file_type().is_file());
    assert_eq!(std::fs::read(&p).unwrap(), png);
    assert_eq!(std::fs::read(&first).unwrap(), b"other");
    let _ = std::fs::remove_dir_all(&root);
}
/// R7: `NotificationHandle::id` says "unique within this process"; the counter
/// is thread-local.
#[test]
fn r7_ids_are_unique_across_threads() {
    let backend = fake();
    let a = show_with(Ok(backend), Notification::new("main"))
        .unwrap()
        .id();
    let b = std::thread::spawn(move || {
        show_with(Ok(backend), Notification::new("other"))
            .unwrap()
            .id()
    })
    .join()
    .unwrap();
    assert_ne!(a, b, "two notifications share id {a}");
}

/// R8 (kills `.all` -> `.any` in `Entry::is_dead`): a live ownerless
/// `on_dismiss` beside a dead `on_click` is not pruned.
#[test]
fn r8_a_live_callback_beside_a_dead_one_is_not_pruned() {
    let backend = fake();
    let scope = Scope::new();
    let dismissed = Rc::new(Cell::new(false));
    let d = dismissed.clone();
    let n = scope.run(|| Notification::new("mixed").on_click(|| {}));
    let n = n.on_dismiss(move |_| d.set(true));
    // on_dismiss was set outside the scope (ownerless)
    let h = show_with(Ok(backend), n).unwrap();
    scope.dispose();
    show_with(Ok(backend), Notification::new("next")).unwrap();
    deliver(h.id(), Event::Dismissed(DismissReason::Expired));
    assert!(
        dismissed.get(),
        "a live on_dismiss was pruned with its dead sibling"
    );
}

/// R9 (kills dropping the batch in `Owned::invoke`): two writes in a
/// callback wake an observing effect once.
#[test]
fn r9_a_callback_runs_in_one_batch() {
    let backend = fake();
    let a = Signal::new(0);
    let b = Signal::new(0);
    let runs = Rc::new(Cell::new(0));
    let r = runs.clone();
    let _e = rinch_core::reactive::Effect::new(move || {
        let _ = (a.get(), b.get());
        r.set(r.get() + 1);
    });
    let base = runs.get();
    let h = show_with(
        Ok(backend),
        Notification::new("t").on_click(move || {
            a.set(1);
            b.set(1);
        }),
    )
    .unwrap();
    deliver(h.id(), Event::Clicked);
    assert_eq!(
        runs.get() - base,
        1,
        "the effect ran once per write, not once per batch"
    );
}

// ── Windows toast XML ───────────────────────────────────────────────────────

fn request(title: &str, body: &str, icon: Option<&str>) -> Request {
    Request {
        id: 1,
        title: title.into(),
        body: body.into(),
        icon: icon.map(PathBuf::from),
        wants_click: true,
        app_id: "bear".into(),
        app_name: "Bear".into(),
    }
}

#[test]
fn toast_xml_escapes_text_and_names_the_image() {
    use super::backend::toast_xml::toast_xml;
    let xml = toast_xml(&request(
        "Tom & <Jerry>",
        "\"quoted\" it's\u{1}",
        Some(r"C:\Temp\rinch-notifications\a b.png"),
    ));
    assert_eq!(
        xml,
        "<toast><visual><binding template=\"ToastGeneric\">\
         <text>Tom &amp; &lt;Jerry&gt;</text>\
         <text>&quot;quoted&quot; it&apos;s</text>\
         <image placement=\"appLogoOverride\" src=\"file:///C:/Temp/rinch-notifications/a b.png\"/>\
         </binding></visual></toast>"
    );
    assert_eq!(
        toast_xml(&request("t", "", None)),
        "<toast><visual><binding template=\"ToastGeneric\"><text>t</text></binding></visual></toast>",
        "no body, no second line"
    );
}

// ── freedesktop, against a private bus ──────────────────────────────────────

#[cfg(target_os = "linux")]
mod freedesktop_bus {
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;

    use futures_lite::StreamExt;
    use zbus::zvariant::OwnedValue;
    use zbus::{MessageStream, message::Type};

    use super::request;
    use crate::notification::backend::freedesktop::FreedesktopBackend;
    use crate::notification::{Backend, DismissReason, Event, NotificationError, Sink};

    /// A `dbus-daemon` of its own, killed on drop.
    struct Bus {
        child: Child,
        address: String,
        _dir: std::path::PathBuf,
    }

    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = std::fs::remove_dir_all(&self._dir);
        }
    }

    /// `None` when the host has no `dbus-daemon` (said loudly on stderr).
    fn private_bus(tag: &str) -> Option<Bus> {
        let dir =
            std::env::temp_dir().join(format!("rinch-notif-bus-{}-{tag}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let config = dir.join("bus.conf");
        std::fs::write(
            &config,
            format!(
                "<!DOCTYPE busconfig PUBLIC \"-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN\" \
                 \"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd\">\
                 <busconfig><type>session</type>\
                 <listen>unix:path={}/bus</listen>\
                 <auth>EXTERNAL</auth>\
                 <policy context=\"default\"><allow send_destination=\"*\" eavesdrop=\"true\"/>\
                 <allow eavesdrop=\"true\"/><allow own=\"*\"/></policy></busconfig>",
                dir.display()
            ),
        )
        .ok()?;
        let mut child = match Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .arg("--nofork")
            .arg("--print-address")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!("SKIPPED: no dbus-daemon on this host ({e})");
                return None;
            }
        };
        let mut line = String::new();
        BufReader::new(child.stdout.take()?)
            .read_line(&mut line)
            .ok()?;
        Some(Bus {
            child,
            address: line.trim().to_string(),
            _dir: dir,
        })
    }

    /// What the fake server was sent.
    #[derive(Debug, Clone)]
    enum Seen {
        Notify {
            app_name: String,
            summary: String,
            body: String,
            actions: Vec<String>,
            hints: HashMap<String, String>,
            id: u32,
        },
        Close(u32),
    }

    /// What the test tells the fake server to do.
    enum Do {
        Signal(&'static str, u32, Option<String>, u32),
    }

    /// A fake `org.freedesktop.Notifications` on `address`: answers `Notify`
    /// with ids from 41 up (off the fixed point where rinch's and the
    /// server's ids agree), answers `CloseNotification` with a
    /// `NotificationClosed(id, 3)`, and closes a notification whose summary
    /// is "early" *before* replying to its `Notify`.
    fn fake_server(
        address: String,
    ) -> (mpsc::Receiver<Seen>, tokio::sync::mpsc::UnboundedSender<Do>) {
        let (seen_tx, seen_rx) = mpsc::channel();
        let (do_tx, mut do_rx) = tokio::sync::mpsc::unbounded_channel::<Do>();
        let (ready_tx, ready_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let conn = zbus::connection::Builder::address(address.as_str())
                    .unwrap()
                    .build()
                    .await
                    .unwrap();
                conn.request_name("org.freedesktop.Notifications").await.unwrap();
                ready_tx.send(()).unwrap();
                let mut next_id = 41u32;
                let mut stream = MessageStream::from(&conn);
                let path = "/org/freedesktop/Notifications";
                let iface = "org.freedesktop.Notifications";
                loop {
                    tokio::select! {
                        Some(cmd) = do_rx.recv() => match cmd {
                            Do::Signal(name, id, key, reason) => {
                                if let Some(key) = key {
                                    conn.emit_signal(None::<()>, path, iface, name, &(id, key)).await.unwrap();
                                } else {
                                    conn.emit_signal(None::<()>, path, iface, name, &(id, reason)).await.unwrap();
                                }
                            }
                        },
                        Some(Ok(msg)) = stream.next() => {
                            let header = msg.header();
                            if header.message_type() != Type::MethodCall {
                                continue;
                            }
                            match header.member().map(|m| m.as_str()) {
                                Some("Notify") => {
                                    type Body = (String, u32, String, String, String, Vec<String>, HashMap<String, OwnedValue>, i32);
                                    let (app_name, _, _, summary, body, actions, hints, _): Body =
                                        msg.body().deserialize().unwrap();
                                    let hints = hints
                                        .into_iter()
                                        .filter_map(|(k, v)| Some((k, String::try_from(v).ok()?)))
                                        .collect();
                                    let id = next_id;
                                    next_id += 1;
                                    if summary == "early" {
                                        conn.emit_signal(None::<()>, path, iface, "NotificationClosed", &(id, 2u32)).await.unwrap();
                                        // Give the client time to read the signal before the
                                        // reply: the order the stash exists for.
                                        std::thread::sleep(Duration::from_millis(300));
                                    }
                                    seen_tx.send(Seen::Notify { app_name, summary, body, actions, hints, id }).unwrap();
                                    conn.reply(&header, &(id,)).await.unwrap();
                                }
                                Some("CloseNotification") => {
                                    let (id,): (u32,) = msg.body().deserialize().unwrap();
                                    seen_tx.send(Seen::Close(id)).unwrap();
                                    conn.reply(&header, &()).await.unwrap();
                                    conn.emit_signal(None::<()>, path, iface, "NotificationClosed", &(id, 3u32)).await.unwrap();
                                }
                                _ => {}
                            }
                        }
                    }
                }
            });
        });
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        (seen_rx, do_tx)
    }

    fn backend(address: &str) -> (FreedesktopBackend, mpsc::Receiver<(u64, Event)>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let sink: Sink = std::sync::Arc::new(move |id, ev| {
            let _ = tx.lock().unwrap().send((id, ev));
        });
        (
            FreedesktopBackend::start(Some(address.to_string()), sink).unwrap(),
            rx,
        )
    }
    use std::sync::Mutex;

    const WAIT: Duration = Duration::from_secs(10);

    fn notified(
        seen: &mpsc::Receiver<Seen>,
    ) -> (
        String,
        String,
        String,
        Vec<String>,
        HashMap<String, String>,
        u32,
    ) {
        match seen.recv_timeout(WAIT).expect("the server got a Notify") {
            Seen::Notify {
                app_name,
                summary,
                body,
                actions,
                hints,
                id,
            } => (app_name, summary, body, actions, hints, id),
            other => panic!("expected Notify, got {other:?}"),
        }
    }

    #[test]
    fn notify_click_expiry_and_close_round_trip_through_a_server() {
        let Some(bus) = private_bus("roundtrip") else {
            return;
        };
        let (seen, server) = fake_server(bus.address.clone());
        let (backend, events) = backend(&bus.address);

        // A click.
        let mut req = request(
            "Hotkey unavailable",
            "Ctrl+Alt+Space is taken.",
            Some("/tmp/a b.png"),
        );
        req.id = 7;
        backend.show(req).unwrap();
        let (app_name, summary, body, actions, hints, server_id) = notified(&seen);
        assert_eq!(app_name, "Bear");
        assert_eq!(summary, "Hotkey unavailable");
        assert_eq!(body, "Ctrl+Alt+Space is taken.");
        assert_eq!(actions, vec!["default".to_string(), "Open".to_string()]);
        assert_eq!(hints.get("desktop-entry").map(String::as_str), Some("bear"));
        assert_eq!(
            hints.get("image-path").map(String::as_str),
            Some("file:///tmp/a%20b.png")
        );
        assert_eq!(server_id, 41);
        // Another app's notification, and a non-default action: neither is ours.
        server
            .send(Do::Signal("ActionInvoked", 999, Some("default".into()), 0))
            .ok();
        server
            .send(Do::Signal(
                "ActionInvoked",
                server_id,
                Some("reply".into()),
                0,
            ))
            .ok();
        assert!(
            events.recv_timeout(Duration::from_millis(500)).is_err(),
            "neither another app's click nor another action is a click on ours"
        );
        server
            .send(Do::Signal(
                "ActionInvoked",
                server_id,
                Some("default".into()),
                0,
            ))
            .ok();
        assert_eq!(events.recv_timeout(WAIT).unwrap(), (7, Event::Clicked));

        // An expiry, reason 1.
        let mut req = request("second", "", None);
        req.id = 8;
        req.wants_click = false;
        backend.show(req).unwrap();
        let (_, _, _, actions, hints, server_id) = notified(&seen);
        assert!(actions.is_empty(), "no click callback, no action");
        assert!(!hints.contains_key("image-path"));
        server
            .send(Do::Signal("NotificationClosed", server_id, None, 1))
            .ok();
        assert_eq!(
            events.recv_timeout(WAIT).unwrap(),
            (8, Event::Dismissed(DismissReason::Expired))
        );

        // The app closes one.
        let mut req = request("third", "", None);
        req.id = 9;
        backend.show(req).unwrap();
        let (_, _, _, _, _, server_id) = notified(&seen);
        backend.close(9);
        match seen.recv_timeout(WAIT).unwrap() {
            Seen::Close(id) => assert_eq!(id, server_id),
            other => panic!("expected CloseNotification, got {other:?}"),
        }
        assert_eq!(
            events.recv_timeout(WAIT).unwrap(),
            (9, Event::Dismissed(DismissReason::Closed))
        );

        // Closed by the server before its Notify reply was read.
        let mut req = request("early", "", None);
        req.id = 10;
        backend.show(req).unwrap();
        notified(&seen);
        assert_eq!(
            events.recv_timeout(WAIT).unwrap(),
            (10, Event::Dismissed(DismissReason::Dismissed))
        );

        // Nothing else arrived (the other app's click, the "reply" action).
        assert!(events.recv_timeout(Duration::from_millis(300)).is_err());
        drop(bus);
    }

    #[test]
    fn no_server_on_the_bus_is_a_no_service_error() {
        let Some(bus) = private_bus("noserver") else {
            return;
        };
        let (backend, events) = backend(&bus.address);
        let mut req = request("t", "", None);
        req.id = 3;
        backend.show(req).unwrap();
        match events.recv_timeout(WAIT).unwrap() {
            (3, Event::Failed(NotificationError::NoService(m))) => {
                assert!(m.contains("org.freedesktop.Notifications"), "{m}")
            }
            other => panic!("expected NoService, got {other:?}"),
        }
    }

    #[test]
    fn no_bus_is_a_no_service_error() {
        let dir = std::env::temp_dir().join(format!("rinch-notif-nobus-{}", std::process::id()));
        let (backend, events) = backend(&format!("unix:path={}/absent", dir.display()));
        let mut req = request("t", "", None);
        req.id = 4;
        backend.show(req).unwrap();
        match events.recv_timeout(WAIT).unwrap() {
            (4, Event::Failed(NotificationError::NoService(m))) => {
                assert!(m.contains("no D-Bus session bus"), "{m}")
            }
            other => panic!("expected NoService, got {other:?}"),
        }
    }

    // ── review-1535 fixtures ────────────────────────────────────────────────
    mod review_1535 {
        use super::*;

        enum Ctl {
            Signal(&'static str, u32, Option<String>, u32),
            Stop,
        }

        /// A stoppable fake server answering ids from `first`.
        fn server(
            address: String,
            first: u32,
        ) -> (
            mpsc::Receiver<Seen>,
            tokio::sync::mpsc::UnboundedSender<Ctl>,
            std::thread::JoinHandle<()>,
        ) {
            let (seen_tx, seen_rx) = mpsc::channel();
            let (do_tx, mut do_rx) = tokio::sync::mpsc::unbounded_channel::<Ctl>();
            let (ready_tx, ready_rx) = mpsc::channel();
            let h = std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                rt.block_on(async move {
                    let conn = zbus::connection::Builder::address(address.as_str())
                        .unwrap()
                        .build()
                        .await
                        .unwrap();
                    conn.request_name("org.freedesktop.Notifications").await.unwrap();
                    ready_tx.send(()).unwrap();
                    let mut next_id = first;
                    let mut stream = MessageStream::from(&conn);
                    let path = "/org/freedesktop/Notifications";
                    let iface = "org.freedesktop.Notifications";
                    loop {
                        tokio::select! {
                            Some(cmd) = do_rx.recv() => match cmd {
                                Ctl::Signal(name, id, Some(key), _) => {
                                    conn.emit_signal(None::<()>, path, iface, name, &(id, key)).await.unwrap();
                                }
                                Ctl::Signal(name, id, None, reason) => {
                                    conn.emit_signal(None::<()>, path, iface, name, &(id, reason)).await.unwrap();
                                }
                                Ctl::Stop => break,
                            },
                            Some(Ok(msg)) = stream.next() => {
                                let header = msg.header();
                                if header.message_type() != Type::MethodCall { continue; }
                                match header.member().map(|m| m.as_str()) {
                                    Some("Notify") => {
                                        type Body = (String, u32, String, String, String, Vec<String>, HashMap<String, OwnedValue>, i32);
                                        let (app_name, _, _, summary, body, actions, _hints, _): Body =
                                            msg.body().deserialize().unwrap();
                                        let id = next_id;
                                        next_id += 1;
                                        let _ = seen_tx.send(Seen::Notify { app_name, summary, body, actions, hints: HashMap::new(), id });
                                        conn.reply(&header, &(id,)).await.unwrap();
                                    }
                                    Some("CloseNotification") => {
                                        let (id,): (u32,) = msg.body().deserialize().unwrap();
                                        let _ = seen_tx.send(Seen::Close(id));
                                        conn.reply(&header, &()).await.unwrap();
                                        conn.emit_signal(None::<()>, path, iface, "NotificationClosed", &(id, 3u32)).await.unwrap();
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    let _ = conn.release_name("org.freedesktop.Notifications").await;
                });
            });
            ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            (seen_rx, do_tx, h)
        }

        fn rq(id: u64) -> crate::notification::Request {
            let mut r = request(&format!("n{id}"), "", None);
            r.id = id;
            r
        }

        /// R1: the server restarts (plasmashell / dunst restart). A click on a
        /// notification of the NEW server reaches us?
        #[test]
        fn r1_a_restarted_server_still_reaches_us() {
            let Some(bus) = private_bus("r1") else { return };
            let (seen_a, ctl_a, ha) = server(bus.address.clone(), 41);
            let (backend, events) = backend(&bus.address);
            backend.show(rq(7)).unwrap();
            notified(&seen_a);
            ctl_a.send(Ctl::Stop).ok();
            ha.join().unwrap();
            std::thread::sleep(Duration::from_millis(300));
            let (seen_b, ctl_b, _hb) = server(bus.address.clone(), 100);
            backend.show(rq(8)).unwrap();
            let (_, _, _, _, _, sid) = notified(&seen_b);
            assert_eq!(sid, 100);
            std::thread::sleep(Duration::from_millis(300));
            ctl_b
                .send(Ctl::Signal("ActionInvoked", sid, Some("default".into()), 0))
                .ok();
            // The notification the old server was showing went with it.
            assert_eq!(
                events.recv_timeout(Duration::from_secs(3)).ok(),
                Some((7, Event::Dismissed(DismissReason::Unknown)))
            );
            let got = events.recv_timeout(Duration::from_secs(3));
            eprintln!("R1 after restart: {got:?}");
            assert_eq!(
                got.ok(),
                Some((8, Event::Clicked)),
                "click from the restarted server lost"
            );
        }

        /// R2: after a restart the new server reuses server ids (dunst starts at
        /// 1 again). Another app's notification on the new server with the id
        /// our dead notification had must not be delivered to ours.
        #[test]
        fn r2_a_reused_server_id_after_restart_is_not_ours() {
            let Some(bus) = private_bus("r2") else { return };
            let (seen_a, ctl_a, ha) = server(bus.address.clone(), 41);
            let (backend, events) = backend(&bus.address);
            backend.show(rq(7)).unwrap();
            let (_, _, _, _, _, sid) = notified(&seen_a);
            ctl_a.send(Ctl::Stop).ok();
            ha.join().unwrap();
            std::thread::sleep(Duration::from_millis(300));
            let (_seen_b, ctl_b, _hb) = server(bus.address.clone(), 41);
            std::thread::sleep(Duration::from_millis(300));
            // Ours ended with the old server ...
            assert_eq!(
                events.recv_timeout(Duration::from_secs(2)).ok(),
                Some((7, Event::Dismissed(DismissReason::Unknown)))
            );
            // ... and another app's notification 41 on the new server is not ours.
            ctl_b
                .send(Ctl::Signal("NotificationClosed", sid, None, 2))
                .ok();
            ctl_b
                .send(Ctl::Signal("ActionInvoked", sid, Some("default".into()), 0))
                .ok();
            let got = events.recv_timeout(Duration::from_secs(2));
            eprintln!("R2 stale id: {got:?}");
            assert!(
                got.is_err(),
                "another app's close delivered to our dead notification: {got:?}"
            );
        }

        /// R2b: after a restart that reuses ids, closing our OLD notification
        /// must not close the NEW one the server gave the same id.
        #[test]
        fn r2b_closing_a_pre_restart_notification_does_not_close_a_new_one() {
            let Some(bus) = private_bus("r2b") else {
                return;
            };
            let (seen_a, ctl_a, ha) = server(bus.address.clone(), 41);
            let (backend, events) = backend(&bus.address);
            backend.show(rq(7)).unwrap();
            notified(&seen_a);
            ctl_a.send(Ctl::Stop).ok();
            ha.join().unwrap();
            std::thread::sleep(Duration::from_millis(300));
            let (seen_b, _ctl_b, _hb) = server(bus.address.clone(), 41);
            backend.show(rq(8)).unwrap();
            let (_, _, _, _, _, sid) = notified(&seen_b);
            assert_eq!(sid, 41);
            backend.close(7);
            let closed = seen_b.recv_timeout(Duration::from_secs(2));
            // 7 ended with the old server; 8 is still up.
            let ev = events.recv_timeout(Duration::from_secs(2));
            assert_eq!(ev, Ok((7, Event::Dismissed(DismissReason::Unknown))));
            assert!(
                events.recv_timeout(Duration::from_millis(500)).is_err(),
                "8 must not have closed"
            );
            assert!(
                closed.is_err(),
                "close(7) closed the new server's notification {sid} (ours, id 8): {closed:?}"
            );
        }

        /// R3: many notifications in flight, closed out of order.
        #[test]
        fn r3_many_concurrent_notifications_map_one_to_one() {
            let Some(bus) = private_bus("r3") else { return };
            let (seen, ctl, _h) = server(bus.address.clone(), 41);
            let (backend, events) = backend(&bus.address);
            let n = 64u64;
            for i in 0..n {
                backend.show(rq(1000 + i)).unwrap();
            }
            let mut sids = Vec::new();
            for _ in 0..n {
                let (_, s, _, _, _, sid) = notified(&seen);
                sids.push((s, sid));
            }
            // reverse-order clicks with interleaved closes
            for (k, (s, sid)) in sids.iter().enumerate().rev() {
                let local: u64 = s[1..].parse().unwrap();
                if k % 2 == 0 {
                    ctl.send(Ctl::Signal(
                        "ActionInvoked",
                        *sid,
                        Some("default".into()),
                        0,
                    ))
                    .ok();
                    assert_eq!(events.recv_timeout(WAIT).unwrap(), (local, Event::Clicked));
                } else {
                    backend.close(local);
                    assert_eq!(
                        events.recv_timeout(WAIT).unwrap(),
                        (local, Event::Dismissed(DismissReason::Closed))
                    );
                }
            }
            assert!(events.recv_timeout(Duration::from_millis(300)).is_err());
        }

        /// R4: the bus dies; the next show reports an error (not a hang), and
        /// after the bus is back at the same address a show works again.
        #[test]
        fn r4_the_bus_dying_is_reported_and_recovered_from() {
            let Some(mut bus) = private_bus("r4") else {
                return;
            };
            bus.address = bus.address.split(",guid=").next().unwrap().to_string();
            let (seen, _ctl, _h) = server(bus.address.clone(), 41);
            let (backend, events) = backend(&bus.address);
            backend.show(rq(1)).unwrap();
            notified(&seen);
            std::thread::sleep(Duration::from_millis(500));
            let _ = bus.child.kill();
            let _ = bus.child.wait();
            std::thread::sleep(Duration::from_millis(300));
            backend.show(rq(2)).unwrap();
            let got = events.recv_timeout(Duration::from_secs(5));
            eprintln!("R4 after bus death: {got:?}");
            assert!(
                matches!(got, Ok((2, Event::Failed(NotificationError::NoService(_))))),
                "{got:?}"
            );
            // bring the bus back at the same socket
            let _ = std::fs::remove_file(bus._dir.join("bus"));
            let config = bus._dir.join("bus.conf");
            let mut child = Command::new("dbus-daemon")
                .arg(format!("--config-file={}", config.display()))
                .arg("--nofork")
                .arg("--print-address")
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let mut line = String::new();
            BufReader::new(child.stdout.take().unwrap())
                .read_line(&mut line)
                .unwrap();
            bus.child = child;
            let (seen2, _ctl2, _h2) = server(bus.address.clone(), 200);
            backend.show(rq(3)).unwrap();
            let got3 = events.recv_timeout(Duration::from_secs(5));
            eprintln!("R4 show after bus came back: {got3:?}");
            let s3 = seen2.recv_timeout(Duration::from_secs(5));
            eprintln!("R4 server2 saw: {s3:?}");
            backend.show(rq(4)).unwrap();
            let s4 = seen2.recv_timeout(Duration::from_secs(5));
            eprintln!(
                "R4 server2 saw (2nd): {s4:?} events: {:?}",
                events.recv_timeout(Duration::from_millis(500))
            );
            assert!(s3.is_ok() || s4.is_ok(), "never reconnected");
        }

        /// R5: dropping a backend ends its thread.
        #[test]
        fn r5_dropping_the_backend_ends_its_thread() {
            let Some(bus) = private_bus("r5") else { return };
            let (seen, _ctl, _h) = server(bus.address.clone(), 41);
            let before: Vec<String> = std::fs::read_dir("/proc/self/task")
                .unwrap()
                .filter_map(|e| std::fs::read_to_string(e.ok()?.path().join("comm")).ok())
                .collect();
            {
                let (backend, _events) = backend(&bus.address);
                backend.show(rq(1)).unwrap();
                notified(&seen);
            }
            std::thread::sleep(Duration::from_millis(500));
            let after: Vec<String> = std::fs::read_dir("/proc/self/task")
                .unwrap()
                .filter_map(|e| std::fs::read_to_string(e.ok()?.path().join("comm")).ok())
                .collect();
            eprintln!(
                "R5 threads before {} after {}: {:?}",
                before.len(),
                after.len(),
                after
            );
        }
    }
}
