//! WebRTC section — demonstrates peer-to-peer audio/video calling.
//!
//! Shows the VideoCall API with signaling, connection state, and controls.
//! Requires a running signaling server (`cargo run -p rinch-signaling-server`).

use rinch::prelude::*;
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};
use rinch_webrtc::{ConnectionState, VideoCall};

pub fn webrtc_section(__scope: &mut RenderScope) -> NodeHandle {
    let room_id = Signal::new("test-room".to_string());
    let server_url = Signal::new("ws://127.0.0.1:9090".to_string());
    let call_state = Signal::new(ConnectionState::New);
    let status_text = Signal::new("Not connected".to_string());
    let is_calling = Signal::new(false);

    rsx! {
        Fragment {
            Title { order: 2, "WebRTC" }
            Space { h: "md" }
            Text { color: "dimmed",
                "Peer-to-peer audio/video calls using str0m (native) or web_sys (WASM). "
                "Start the signaling server with: cargo run -p rinch-signaling-server"
            }
            Space { h: "xl" }

            // Connection setup
            SimpleGrid { cols: Some(2), spacing: "md",
                // Call Controls
                Paper { p: "lg", radius: "md", shadow: "sm",
                    Stack { gap: "md",
                        Title { order: 4, "Call Controls" }

                        TextInput {
                            label: "Signaling Server",
                            value_fn: move || server_url.get(),
                            oninput: move |val: String| server_url.set(val)
                        }
                        TextInput {
                            label: "Room ID",
                            value_fn: move || room_id.get(),
                            oninput: move |val: String| room_id.set(val)
                        }

                        Group { gap: "sm",
                            Button {
                                variant: "filled",
                                color: "green",
                                disabled: {|| is_calling.get()},
                                onclick: {
                                    move || {
                                        let url = format!("{}/room/{}", server_url.get(), room_id.get());
                                        status_text.set(format!("Connecting to {}...", url));
                                        is_calling.set(true);

                                        match rinch_signaling::WebSocketSignaling::connect(&url) {
                                            Ok(signaling) => {
                                                match VideoCall::new() {
                                                    Ok(mut call) => {
                                                        // Forward state changes
                                                        call.peer.on_connection_state_change(move |s| {
                                                            call_state.send(s);
                                                            let text = match s {
                                                                ConnectionState::New => "New".to_string(),
                                                                ConnectionState::Connecting => "Connecting...".to_string(),
                                                                ConnectionState::Connected => "Connected!".to_string(),
                                                                ConnectionState::Disconnected => "Disconnected".to_string(),
                                                                ConnectionState::Failed => "Connection failed".to_string(),
                                                                ConnectionState::Closed => "Closed".to_string(),
                                                            };
                                                            status_text.send(text);
                                                        });

                                                        if let Err(e) = call.start(signaling, true, false) {
                                                            status_text.set(format!("Start failed: {e}"));
                                                            is_calling.set(false);
                                                        } else {
                                                            status_text.set("Offer sent, waiting for peer...".to_string());
                                                        }
                                                    }
                                                    Err(e) => {
                                                        status_text.set(format!("Create call failed: {e}"));
                                                        is_calling.set(false);
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                status_text.set(format!("Connect failed: {e}"));
                                                is_calling.set(false);
                                            }
                                        }
                                    }
                                },
                                {render_tabler_icon(__scope, TablerIcon::Phone, TablerIconStyle::Outline)}
                                " Start Call"
                            }
                            Button {
                                variant: "filled",
                                color: "blue",
                                disabled: {|| is_calling.get()},
                                onclick: {
                                    move || {
                                        let url = format!("{}/room/{}", server_url.get(), room_id.get());
                                        status_text.set(format!("Connecting to {}...", url));
                                        is_calling.set(true);

                                        match rinch_signaling::WebSocketSignaling::connect(&url) {
                                            Ok(signaling) => {
                                                match VideoCall::new() {
                                                    Ok(mut call) => {
                                                        call.peer.on_connection_state_change(move |s| {
                                                            call_state.send(s);
                                                            let text = match s {
                                                                ConnectionState::New => "New".to_string(),
                                                                ConnectionState::Connecting => "Connecting...".to_string(),
                                                                ConnectionState::Connected => "Connected!".to_string(),
                                                                ConnectionState::Disconnected => "Disconnected".to_string(),
                                                                ConnectionState::Failed => "Connection failed".to_string(),
                                                                ConnectionState::Closed => "Closed".to_string(),
                                                            };
                                                            status_text.send(text);
                                                        });

                                                        if let Err(e) = call.join(signaling) {
                                                            status_text.set(format!("Join failed: {e}"));
                                                            is_calling.set(false);
                                                        } else {
                                                            status_text.set("Joined, connected!".to_string());
                                                        }
                                                    }
                                                    Err(e) => {
                                                        status_text.set(format!("Create call failed: {e}"));
                                                        is_calling.set(false);
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                status_text.set(format!("Connect failed: {e}"));
                                                is_calling.set(false);
                                            }
                                        }
                                    }
                                },
                                {render_tabler_icon(__scope, TablerIcon::PhoneIncoming, TablerIconStyle::Outline)}
                                " Join Call"
                            }
                        }
                    }
                }

                // Status
                Paper { p: "lg", radius: "md", shadow: "sm",
                    Stack { gap: "md",
                        Title { order: 4, "Connection Status" }

                        Group { gap: "sm",
                            div {
                                style: {|| {
                                    let color = match call_state.get() {
                                        ConnectionState::Connected => "var(--rinch-color-green-6)",
                                        ConnectionState::Connecting => "var(--rinch-color-yellow-6)",
                                        ConnectionState::Failed => "var(--rinch-color-red-6)",
                                        ConnectionState::Disconnected => "var(--rinch-color-orange-6)",
                                        _ => "var(--rinch-color-gray-5)",
                                    };
                                    format!(
                                        "width: 12px; height: 12px; border-radius: 50%; background: {color}"
                                    )
                                }}
                            }
                            Text { size: "lg", weight: "600",
                                {|| match call_state.get() {
                                    ConnectionState::New => "New".to_string(),
                                    ConnectionState::Connecting => "Connecting".to_string(),
                                    ConnectionState::Connected => "Connected".to_string(),
                                    ConnectionState::Disconnected => "Disconnected".to_string(),
                                    ConnectionState::Failed => "Failed".to_string(),
                                    ConnectionState::Closed => "Closed".to_string(),
                                }}
                            }
                        }

                        Text { color: "dimmed", {|| status_text.get()} }

                        Space { h: "md" }
                        Title { order: 5, "Architecture" }
                        Text { size: "sm", color: "dimmed",
                            "Native: str0m (Rust WebRTC) + Opus audio codec + UDP transport"
                        }
                        Text { size: "sm", color: "dimmed",
                            "WASM: web_sys RTCPeerConnection (browser-native WebRTC)"
                        }
                        Text { size: "sm", color: "dimmed",
                            "Signaling: WebSocket relay server (room-based)"
                        }
                    }
                }
            }

            Space { h: "xl" }

            // API reference
            Paper { p: "lg", radius: "md", shadow: "sm",
                Stack { gap: "sm",
                    Title { order: 4, "Quick Start" }
                    Code { block: true,
                        "// Start the signaling server:\n"
                        "// cargo run -p rinch-signaling-server\n\n"
                        "// Caller:\n"
                        "let signaling = WebSocketSignaling::connect(\"ws://127.0.0.1:9090/room/my-room\")?;\n"
                        "let mut call = VideoCall::new()?;\n"
                        "call.start(signaling, true, false)?; // audio only\n\n"
                        "// Callee (in another instance):\n"
                        "let signaling = WebSocketSignaling::connect(\"ws://127.0.0.1:9090/room/my-room\")?;\n"
                        "let mut call = VideoCall::new()?;\n"
                        "call.join(signaling)?;\n\n"
                        "// Both sides:\n"
                        "call.poll(); // call periodically from main thread\n"
                        "call.hangup();"
                    }
                }
            }
        }
    }
}
