//! Lists the running rinch apps this user can reach, and how:
//! `cargo run -p rinch-debug --example list_apps`.
//!
//! Each line is the app's pid, its transport and its name. `socket` apps are
//! reachable by this user alone; `legacy-tcp` apps (built on a rinch from
//! before the socket transport) are open to every local user.

use rinch_debug::client::{Endpoint, Listing};

fn main() {
    let listing = Listing::from_process();
    let apps = listing.apps();
    if apps.is_empty() {
        println!("no rinch apps with the debug server running");
        return;
    }
    for app in apps {
        let endpoint = match &app.endpoint {
            Endpoint::Socket(path) => path.display().to_string(),
            Endpoint::Tcp(port) => format!("127.0.0.1:{port}"),
        };
        let restricted = if app.transport.is_restricted() {
            "this user only"
        } else {
            "open to every local user"
        };
        println!(
            "{:>7}  {:<10}  {}  ({endpoint}; {restricted})",
            app.pid, app.transport, app.app_name
        );
    }
}
