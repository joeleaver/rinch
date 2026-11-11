//! The debug server embedded in a rinch app (the `debug` feature), and the
//! client every tool reaches it with.
//!
//! On Unix an app listens on a socket in a directory only its user can
//! enter, and admits only peers running as that user ([`location`],
//! [`peer`]). It also still listens on the legacy TCP port on `127.0.0.1`,
//! found through `~/.rinch/debug/{pid}.json` ([`discovery`]), while clients
//! move to the socket. [`client`] lists apps over both and says which
//! [`Transport`] reaches each.

pub mod client;
pub mod discovery;
#[cfg(unix)]
pub mod location;
#[cfg(unix)]
pub mod peer;
pub mod protocol;
pub mod server;
pub mod transport;

pub use protocol::{DebugCommand, DebugCommandKind, DebugResult, ScrollUnit, fold_modifier_names};
pub use server::{DebugServer, ServerOptions};
pub use transport::Transport;

use std::sync::{Arc, mpsc};

/// Hands commands to the app's runtime and waits for their results. One is
/// cloned for each connected client.
#[derive(Clone)]
pub struct CommandSender {
    tx: mpsc::Sender<DebugCommand>,
    notify: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl CommandSender {
    /// A sender on `tx`, calling `notify` after each command to wake the
    /// runtime.
    pub fn new(
        tx: mpsc::Sender<DebugCommand>,
        notify: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> Self {
        Self { tx, notify }
    }

    pub fn execute(&self, kind: DebugCommandKind) -> DebugResult {
        let (response_tx, response_rx) = mpsc::channel();
        let cmd = DebugCommand { kind, response_tx };
        if self.tx.send(cmd).is_err() {
            return DebugResult::Error {
                message: "Runtime shut down".into(),
            };
        }
        if let Some(notify) = &self.notify {
            notify();
        }
        match response_rx.recv_timeout(std::time::Duration::from_secs(30)) {
            Ok(result) => result,
            Err(_) => DebugResult::Error {
                message: "Timeout waiting for response".into(),
            },
        }
    }
}

pub struct CommandReceiver(pub mpsc::Receiver<DebugCommand>);

/// Start the debug IPC server where this process's environment says (see
/// [`ServerOptions::from_process`]). Returns a DebugServer handle and
/// CommandReceiver.
pub fn attach(
    app_name: &str,
    notify: impl Fn() + Send + Sync + 'static,
) -> std::io::Result<(DebugServer, CommandReceiver)> {
    let (tx, rx) = mpsc::channel();
    let sender = CommandSender::new(tx, Some(Arc::new(notify)));
    let server = DebugServer::start(app_name, sender, ServerOptions::from_process())?;
    Ok((server, CommandReceiver(rx)))
}
