pub mod discovery;
pub mod protocol;
pub mod server;

pub use protocol::{DebugCommand, DebugCommandKind, DebugResult, fold_modifier_names};
pub use server::DebugServer;

use std::sync::{Arc, mpsc};

pub struct CommandSender {
    tx: mpsc::Sender<DebugCommand>,
    notify: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl CommandSender {
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

/// Start the debug IPC server. Returns a DebugServer handle and CommandReceiver.
pub fn attach(
    app_name: &str,
    notify: impl Fn() + Send + Sync + 'static,
) -> std::io::Result<(DebugServer, CommandReceiver)> {
    let (tx, rx) = mpsc::channel();
    let sender = CommandSender {
        tx,
        notify: Some(Arc::new(notify)),
    };
    let port_override = std::env::var("RINCH_DEBUG_PORT")
        .ok()
        .and_then(|p| p.parse().ok());
    let server = DebugServer::start(app_name, sender, port_override)?;
    Ok((server, CommandReceiver(rx)))
}
