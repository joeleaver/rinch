//! `cargo run -p rtc-room-relay [port]` (default 8080, or `RTC_RELAY_PORT`).
//! See the library's docs for what the relay does.

use std::net::TcpListener;

fn main() {
    tracing_subscriber::fmt::init();
    let port = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("RTC_RELAY_PORT").ok())
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(8080);
    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr).unwrap_or_else(|e| {
        eprintln!("failed to bind {addr}: {e}");
        std::process::exit(1);
    });
    tracing::info!("room relay listening on ws://{addr}/room/<name>");
    rtc_room_relay::run(listener);
}
