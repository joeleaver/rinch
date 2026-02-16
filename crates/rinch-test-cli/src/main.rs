//! CLI client for the rinch test harness.
//!
//! Connects to a rinch application's test server via TCP and sends commands.
//!
//! # Usage
//!
//! ```bash
//! # Take a screenshot
//! rinch-test screenshot --port 9999 --output screenshot.png
//!
//! # Dump DOM tree
//! rinch-test dom --port 9999
//!
//! # Query elements
//! rinch-test query --port 9999 div
//! rinch-test query --port 9999 "[data-rid]"
//!
//! # Click at coordinates
//! rinch-test click --port 9999 100 200
//!
//! # Get text content
//! rinch-test text --port 9999 --id 5
//!
//! # Get node details
//! rinch-test node --port 9999 --id 5
//!
//! # Wait for a frame
//! rinch-test wait-frame --port 9999
//! ```

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;

use clap::{Parser, Subcommand};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(name = "rinch-test", about = "CLI client for rinch test harness")]
struct Cli {
    /// Port to connect to
    #[arg(short, long, default_value = "9999")]
    port: u16,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Take a screenshot and save as PNG
    Screenshot {
        /// Output file path
        #[arg(short, long, default_value = "screenshot.png")]
        output: String,
    },
    /// Dump the DOM tree as JSON
    Dom,
    /// Query elements by selector (tag name or [attribute])
    Query {
        /// CSS-like selector
        selector: String,
    },
    /// Click at screen coordinates
    Click {
        /// X coordinate
        x: f32,
        /// Y coordinate
        y: f32,
    },
    /// Get text content of a node subtree
    Text {
        /// Node ID
        #[arg(long)]
        id: usize,
    },
    /// Get detailed info about a node
    Node {
        /// Node ID
        #[arg(long)]
        id: usize,
    },
    /// Wait for the next render frame
    WaitFrame,
}

fn send_command(port: u16, method: &str, params: Value) -> Result<Value, String> {
    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .map_err(|e| format!("Connect failed: {}", e))?;

    let request = json!({"method": method, "params": params});
    writeln!(stream, "{}", request).map_err(|e| format!("Write failed: {}", e))?;

    let reader = BufReader::new(stream);
    let line = reader
        .lines()
        .next()
        .ok_or("No response")?
        .map_err(|e| format!("Read failed: {}", e))?;

    serde_json::from_str(&line).map_err(|e| format!("Invalid JSON response: {}", e))
}

fn main() {
    let cli = Cli::parse();

    let result = match &cli.command {
        Command::Screenshot { output } => {
            let resp = send_command(cli.port, "screenshot", json!({}));
            match resp {
                Ok(v) => {
                    if v["ok"].as_bool() == Some(true) {
                        let b64 = v["result"]["png_base64"].as_str().unwrap_or("");
                        use base64::Engine;
                        match base64::engine::general_purpose::STANDARD.decode(b64) {
                            Ok(bytes) => {
                                std::fs::write(output, &bytes)
                                    .expect("Failed to write screenshot file");
                                eprintln!("Screenshot saved to {} ({} bytes)", output, bytes.len());
                                Ok(json!({"ok": true, "file": output, "size": bytes.len()}))
                            }
                            Err(e) => Err(format!("Base64 decode failed: {}", e)),
                        }
                    } else {
                        Err(v["error"].as_str().unwrap_or("Unknown error").to_string())
                    }
                }
                Err(e) => Err(e),
            }
        }
        Command::Dom => send_command(cli.port, "dom_tree", json!({})),
        Command::Query { selector } => {
            send_command(cli.port, "query_selector", json!({"selector": selector}))
        }
        Command::Click { x, y } => send_command(cli.port, "click", json!({"x": x, "y": y})),
        Command::Text { id } => send_command(cli.port, "get_text_content", json!({"id": id})),
        Command::Node { id } => send_command(cli.port, "get_node", json!({"id": id})),
        Command::WaitFrame => send_command(cli.port, "wait_frame", json!({})),
    };

    match result {
        Ok(v) => {
            println!("{}", serde_json::to_string_pretty(&v).unwrap());
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}
