//! The tokio runtime that rfd's portal dialogs run inside (#1526).
//!
//! On Linux and the BSDs rfd shows its dialogs through the XDG desktop portal:
//! ashpd over zbus, blocked on with `pollster` on the caller's thread. zbus is
//! one crate per build, and its mode is the union of every crate's features.
//! Once anything in the app turns on `zbus/tokio` (ksni for rinch's
//! `system-tray`, ashpd's default features, an app's own zbus), zbus spawns its
//! tasks with `tokio::spawn` and `spawn_blocking`. On a thread with no tokio
//! runtime that panics: "there is no reactor running, must be called from the
//! context of a Tokio 1.x runtime". rinch cannot know which mode an app's build
//! ends up in, so every dialog (`rinch::dialogs`, and `file::FilePicker` on the
//! desktop) runs with this runtime entered. In async-io mode
//! zbus never looks at it.
//!
//! A multi-thread runtime with one worker: its worker drives zbus's tasks while
//! the caller's thread is blocked in `pollster`. One runtime for the whole
//! process, never one per dialog: ashpd keeps its session connection in a
//! static, and that connection's tasks live on the runtime that was current
//! when it was made, so dropping a runtime after the first dialog would leave
//! the next one waiting on a dead connection. Entering it is allowed inside
//! another runtime's context too (an app's own tokio thread), and replaces that
//! context only for the length of the dialog.

#[cfg(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
mod portal {
    use std::sync::OnceLock;
    use tokio::runtime::{Builder, Runtime};

    /// Built at the first dialog. `None` when the runtime could not be built
    /// (no thread could be spawned); the dialog then runs without it, which is
    /// right in async-io mode and the old panic in tokio mode.
    static RUNTIME: OnceLock<Option<Runtime>> = OnceLock::new();

    fn runtime() -> Option<&'static Runtime> {
        RUNTIME
            .get_or_init(|| {
                Builder::new_multi_thread()
                    .worker_threads(1)
                    .thread_name("rinch-dialogs")
                    .enable_all()
                    .build()
                    .map_err(|e| log::warn!("rinch dialogs: no tokio runtime for the portal: {e}"))
                    .ok()
            })
            .as_ref()
    }

    /// Run `dialog` (a blocking rfd call) with the portal runtime entered.
    pub(crate) fn run<R>(dialog: impl FnOnce() -> R) -> R {
        let _entered = runtime().map(Runtime::enter);
        dialog()
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
pub(crate) use portal::run;

/// No portal and no zbus here: rfd calls the platform's own dialogs.
#[cfg(not(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
)))]
pub(crate) fn run<R>(dialog: impl FnOnce() -> R) -> R {
    dialog()
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::run;
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};

    /// A `dbus-daemon` of its own, killed on drop. The test connects to it by
    /// address, so no process-wide `DBUS_SESSION_BUS_ADDRESS` changes.
    struct PrivateBus {
        child: Child,
        address: String,
        dir: std::path::PathBuf,
    }

    impl Drop for PrivateBus {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    impl PrivateBus {
        /// `None` when the host has no `dbus-daemon` (said on stderr).
        fn start() -> Option<Self> {
            let dir = std::env::temp_dir().join(format!("rinch-1526-bus-{}", std::process::id()));
            std::fs::create_dir_all(&dir).ok()?;
            let config = dir.join("bus.conf");
            std::fs::write(
                &config,
                format!(
                    "<!DOCTYPE busconfig PUBLIC \"-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN\" \
                     \"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd\">\
                     <busconfig><type>session</type>\
                     <listen>unix:path={}/bus</listen>\
                     <auth>EXTERNAL</auth>\
                     <policy context=\"default\"><allow send_destination=\"*\"/>\
                     <allow receive_sender=\"*\"/><allow own=\"*\"/></policy></busconfig>",
                    dir.display()
                ),
            )
            .ok()?;
            let mut child = match Command::new("dbus-daemon")
                .arg(format!("--config-file={}", config.display()))
                .args(["--nofork", "--print-address"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("SKIPPED: no dbus-daemon on this host ({e})");
                    let _ = std::fs::remove_dir_all(&dir);
                    return None;
                }
            };
            let mut line = String::new();
            BufReader::new(child.stdout.take()?)
                .read_line(&mut line)
                .ok()?;
            Some(Self {
                child,
                address: line.trim().to_string(),
                dir,
            })
        }
    }

    /// What rfd does for every portal dialog: a zbus call on the caller's
    /// thread, blocked on with pollster, no tokio runtime of the caller's.
    fn portal_shaped_call(address: &str) -> zbus::Result<String> {
        pollster::block_on(async {
            let connection = zbus::connection::Builder::address(address)?.build().await?;
            bus_id(&connection).await
        })
    }

    /// The dialogs' calls, on one connection kept in a static as ashpd keeps
    /// its session connection: made by the first dialog, used by every later
    /// one, so its tasks must outlive the dialog that made it.
    fn cached_connection_call(address: &str) -> zbus::Result<String> {
        static CONNECTION: std::sync::OnceLock<zbus::Connection> = std::sync::OnceLock::new();
        pollster::block_on(async {
            let connection = match CONNECTION.get() {
                Some(c) => c,
                None => {
                    let made = zbus::connection::Builder::address(address)?.build().await?;
                    CONNECTION.get_or_init(|| made)
                }
            };
            bus_id(connection).await
        })
    }

    async fn bus_id(connection: &zbus::Connection) -> zbus::Result<String> {
        let id = zbus::fdo::DBusProxy::new(connection)
            .await?
            .get_id()
            .await?;
        Ok(id.to_string())
    }

    /// How a call on a thread of its own ended.
    enum Ended {
        Answered(zbus::Result<String>),
        Panicked,
    }

    /// Run `call` on a thread of its own, so a panic is caught whole and the
    /// thread never had a runtime of the caller's. A call that has not ended
    /// in 30 s fails the test rather than hanging it.
    fn on_a_thread(call: impl FnOnce() -> zbus::Result<String> + Send + 'static) -> Ended {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(call());
        });
        match rx.recv_timeout(std::time::Duration::from_secs(30)) {
            Ok(answer) => Ended::Answered(answer),
            // The sender went with the thread's unwinding.
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Ended::Panicked,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                panic!("a zbus call on the private bus did not end in 30 s")
            }
        }
    }

    /// #1526: a portal dialog works whichever mode zbus is in. In tokio mode
    /// (`system-tray`'s ksni, the workspace's ashpd) the same call outside the
    /// runtime panicked, "there is no reactor running"; in async-io mode it
    /// works either way. The call outside is the control: it says which mode
    /// this build is in, and `RINCH_1526_ZBUS_MODE` (`tokio` or `async-io`),
    /// set by CI, asserts that the build is the one the line meant to test.
    #[test]
    fn a_portal_call_works_in_either_zbus_mode() {
        let expected = std::env::var("RINCH_1526_ZBUS_MODE").ok();
        let Some(bus) = PrivateBus::start() else {
            // CI names the mode it means to test; there a skip would pass
            // without testing anything.
            assert!(
                expected.is_none(),
                "RINCH_1526_ZBUS_MODE is set but there is no dbus-daemon"
            );
            return;
        };
        let address = bus.address.clone();
        let mode = match on_a_thread(move || portal_shaped_call(&address)) {
            Ended::Panicked => "tokio",
            Ended::Answered(Ok(_)) => "async-io",
            Ended::Answered(Err(e)) => panic!("the control call failed on the private bus: {e}"),
        };
        eprintln!("zbus in {mode} mode in this build");
        if let Some(expected) = expected {
            assert_eq!(mode, expected, "this build's zbus mode");
        }

        let dialog = |address: String| match on_a_thread(move || {
            run(|| cached_connection_call(&address))
        }) {
            Ended::Answered(answer) => answer.expect("the call inside the portal runtime"),
            Ended::Panicked => panic!("a dialog's zbus call panicked inside the portal runtime"),
        };
        let first = dialog(bus.address.clone());
        assert!(!first.is_empty(), "the bus answered with an id");
        // A second dialog, on the connection the first one made: it answers
        // only while the runtime that connection's tasks run on is alive.
        assert_eq!(dialog(bus.address.clone()), first, "the same bus");
    }
}
