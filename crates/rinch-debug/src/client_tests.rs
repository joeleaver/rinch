//! The server and the client together, in this process, with every
//! directory in a temporary one and the peer's credentials injected: no
//! test touches the real runtime directory or `~/.rinch`.

use std::fs;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, mpsc};
use std::thread;

use super::*;
use crate::location::Locations;
use crate::peer::{Peer, PeerCredentials, kernel_credentials};
use crate::server::{DebugServer, ServerOptions, SocketPlace};
use crate::{CommandReceiver, CommandSender, DebugCommandKind};

/// A server's temporary home: the runtime directory and the legacy
/// discovery directory.
struct Setup {
    root: tempfile::TempDir,
}

impl Setup {
    fn new() -> Self {
        Self {
            root: tempfile::tempdir().unwrap(),
        }
    }

    fn locations(&self) -> Locations {
        Locations {
            xdg_runtime_dir: None,
            tmpdir: None,
            prefer_tmpdir: false,
            fallback_root: self.root.path().to_path_buf(),
            uid: rustix::process::geteuid().as_raw(),
        }
    }

    fn legacy_dir(&self) -> PathBuf {
        self.root.path().join("home/.rinch/debug")
    }

    fn listing(&self) -> Listing {
        Listing {
            socket_dir: self.locations().existing_socket_dir(),
            legacy_dir: Some(self.legacy_dir()),
        }
    }

    /// Starts a server for `name` whose runtime answers every command, its
    /// peers' credentials read by `credentials`.
    fn start(&self, name: &str, credentials: PeerCredentials) -> DebugServer {
        let (tx, rx) = mpsc::channel();
        answer(CommandReceiver(rx));
        let options = ServerOptions {
            tcp_port: 0,
            discovery_dir: Some(self.legacy_dir()),
            socket: SocketPlace::Directory(self.locations()),
            peer_credentials: credentials,
        };
        DebugServer::start(name, CommandSender::new(tx, None), options).unwrap()
    }
}

/// A runtime that answers each command with its name.
fn answer(commands: CommandReceiver) {
    thread::spawn(move || {
        for command in commands.0 {
            let name = format!("{:?}", command.kind);
            let _ = command.response_tx.send(DebugResult::Json {
                data: serde_json::json!(name),
            });
        }
    });
}

/// Credentials that make every peer `uid`.
fn as_uid(uid: u32) -> PeerCredentials {
    Arc::new(move |_| Ok(Peer { uid, pid: None }))
}

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn makes_the_socket_and_its_directories_owner_only() {
    let setup = Setup::new();
    let server = setup.start("modes", kernel_credentials());
    let socket = server.socket_path().unwrap();
    let metadata = fs::symlink_metadata(socket).unwrap();
    assert!(metadata.file_type().is_socket());
    assert_eq!(mode(socket), 0o600);
    let debug = socket.parent().unwrap();
    let rinch = debug.parent().unwrap();
    assert_eq!(
        (mode(debug), mode(rinch), mode(rinch.parent().unwrap())),
        (0o700, 0o700, 0o700)
    );
    assert_eq!(
        socket.file_name().unwrap().to_str().unwrap(),
        format!("{}.sock", std::process::id())
    );
}

#[test]
fn lists_and_drives_an_app_over_its_socket() {
    let setup = Setup::new();
    let _server = setup.start("socket app", kernel_credentials());
    let apps = setup.listing().apps();
    // The server also wrote its legacy file; it is listed once, by socket.
    assert_eq!(apps.len(), 1, "{apps:?}");
    let app = &apps[0];
    assert_eq!(
        (app.pid, app.app_name.as_str()),
        (std::process::id(), "socket app")
    );
    assert_eq!(app.transport, Transport::Socket);
    assert_eq!(app.transport.label(), "socket");
    let mut client = Client::connect(app).unwrap();
    assert_eq!(client.transport, Transport::Socket);
    let result = client.execute(DebugCommandKind::WaitFrame).unwrap();
    assert!(matches!(result, DebugResult::Json { data } if data == "WaitFrame"));
    // Requests are numbered; a second one is answered as itself.
    assert!(
        client
            .execute(DebugCommandKind::PerfStats { reset: false })
            .is_ok()
    );
}

#[test]
fn refuses_a_peer_of_another_user_without_a_byte() {
    let setup = Setup::new();
    let own = rustix::process::geteuid().as_raw();
    for credentials in [
        as_uid(own + 1),
        as_uid(0),
        Arc::new(|_: &std::os::unix::net::UnixStream| Err(io::Error::other("unreadable")))
            as PeerCredentials,
    ] {
        let server = setup.start("guarded", credentials);
        let path = server.socket_path().unwrap().to_path_buf();
        let error = Client::connect_socket(&path).unwrap_err();
        // The server closed the connection before the handshake: the end of
        // the stream, or a reset if the request was still unread.
        assert!(
            matches!(
                error.kind(),
                io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset
            ),
            "{error}"
        );
        drop(server);
    }
    // The same user is let in.
    let server = setup.start("guarded", as_uid(own));
    assert!(Client::connect_socket(server.socket_path().unwrap()).is_ok());
}

#[test]
fn lists_legacy_tcp_apps_while_their_process_lives() {
    let setup = Setup::new();
    let legacy = setup.legacy_dir();
    fs::create_dir_all(&legacy).unwrap();
    let mut living = Command::new("sleep")
        .arg("30")
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let mut dead = Command::new("true").spawn().unwrap();
    dead.wait().unwrap();
    for (pid, name, port) in [
        (living.id(), "old app", 9100),
        (dead.id(), "gone app", 9101),
    ] {
        let entry = discovery::DiscoveryEntry {
            pid,
            port,
            app_name: name.into(),
            protocol_version: 1,
            started_at: "then".into(),
        };
        fs::write(
            legacy.join(format!("{pid}.json")),
            serde_json::to_string(&entry).unwrap(),
        )
        .unwrap();
    }
    fs::write(legacy.join("broken.json"), "not json").unwrap();

    let _server = setup.start("new app", kernel_credentials());
    let apps = setup.listing().apps();
    let listed: Vec<(&str, &str)> = apps
        .iter()
        .map(|app| (app.app_name.as_str(), app.transport.label()))
        .collect();
    // Socket apps first, then the legacy ones still running.
    assert_eq!(listed, [("new app", "socket"), ("old app", "legacy-tcp")]);
    assert_eq!(apps[1].endpoint, Endpoint::Tcp(9100));
    assert!(!apps[1].transport.is_restricted());
    // A legacy file is left as it is.
    assert!(legacy.join(format!("{}.json", dead.id())).exists());
    living.kill().unwrap();
    living.wait().unwrap();
}

#[test]
fn reaches_an_app_over_the_legacy_port_too() {
    let setup = Setup::new();
    let server = setup.start("both", kernel_credentials());
    let mut client = Client::connect_tcp(server.port()).unwrap();
    assert_eq!(
        (client.transport, client.app_name.as_str()),
        (Transport::LegacyTcp, "both")
    );
    assert!(client.execute(DebugCommandKind::WaitFrame).is_ok());
}

#[test]
fn removes_the_socket_of_an_app_that_died() {
    let setup = Setup::new();
    let dir = setup.locations().create_socket_dir().unwrap();
    let dead = dir.join("4194303.sock");
    drop(UnixListener::bind(&dead).unwrap());
    assert!(dead.exists());
    let not_a_socket = dir.join("notes.sock");
    fs::write(&not_a_socket, "").unwrap();
    assert_eq!(setup.listing().apps(), []);
    assert!(!dead.exists());
    // Only sockets are removed.
    assert!(not_a_socket.exists());
}

#[test]
fn serves_several_clients_at_once() {
    let setup = Setup::new();
    let server = setup.start("busy", kernel_credentials());
    let path = server.socket_path().unwrap();
    let mut first = Client::connect_socket(path).unwrap();
    // A listing connects while another client stays connected.
    assert_eq!(setup.listing().apps().len(), 1);
    let mut second = Client::connect_socket(path).unwrap();
    assert!(first.execute(DebugCommandKind::WaitFrame).is_ok());
    assert!(second.execute(DebugCommandKind::WaitFrame).is_ok());
}

#[test]
fn cleans_up_and_ends_clients_when_dropped() {
    let setup = Setup::new();
    let server = setup.start("leaving", kernel_credentials());
    let socket = server.socket_path().unwrap().to_path_buf();
    let discovery_file = setup
        .legacy_dir()
        .join(format!("{}.json", std::process::id()));
    assert!(discovery_file.exists());
    let mut client = Client::connect_socket(&socket).unwrap();
    drop(server);
    assert!(!socket.exists());
    assert!(!discovery_file.exists());
    assert!(client.execute(DebugCommandKind::WaitFrame).is_err());
}

#[test]
fn takes_an_exact_socket_path_only_in_a_private_directory() {
    let (tx, rx) = mpsc::channel();
    answer(CommandReceiver(rx));
    let dir = tempfile::tempdir().unwrap();
    // Whatever the umask made it, a directory only we can write.
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let path = dir.path().join("app.sock");
    let options = |socket| ServerOptions {
        tcp_port: 0,
        discovery_dir: None,
        socket,
        peer_credentials: kernel_credentials(),
    };
    let server = DebugServer::start(
        "exact",
        CommandSender::new(tx.clone(), None),
        options(SocketPlace::Exact(path.clone())),
    )
    .unwrap();
    assert_eq!(server.socket_path(), Some(path.as_path()));
    assert_eq!(Client::connect_socket(&path).unwrap().app_name, "exact");
    drop(server);

    let open = tempfile::tempdir().unwrap();
    fs::set_permissions(open.path(), fs::Permissions::from_mode(0o777)).unwrap();
    let refused = DebugServer::start(
        "exact",
        CommandSender::new(tx, None),
        options(SocketPlace::Exact(open.path().join("app.sock"))),
    )
    .unwrap();
    // No socket in a directory others can write; the legacy port remains.
    assert_eq!(refused.socket_path(), None);
    assert!(refused.port() > 0);
}

#[test]
fn reads_a_handshake_with_identity_fields_it_does_not_know() {
    // Room for a server to say more about itself (its windows, say): a
    // client ignores what it does not know.
    let reply = r#"{"protocol":"rinch-debug","version":1,"app_name":"a","pid":7,"windows":[1,2]}"#;
    let response: crate::protocol::HandshakeResponse = serde_json::from_str(reply).unwrap();
    assert_eq!((response.app_name.as_str(), response.pid), ("a", 7));
}
