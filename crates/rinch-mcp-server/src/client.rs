use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Mutex;

/// Wire protocol types (mirrored from rinch-debug)

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum DebugCommandKind {
    #[serde(rename = "screenshot")]
    Screenshot,
    #[serde(rename = "dom_tree")]
    DomTree,
    #[serde(rename = "query_selector")]
    QuerySelector { selector: String },
    #[serde(rename = "get_node")]
    GetNode { id: usize },
    #[serde(rename = "get_text_content")]
    GetTextContent { id: usize },
    #[serde(rename = "click")]
    Click { x: f32, y: f32 },
    #[serde(rename = "type_text")]
    TypeText { text: String },
    #[serde(rename = "wait_frame")]
    WaitFrame,
    #[serde(rename = "close_app")]
    CloseApp,
    #[serde(rename = "get_computed_styles")]
    GetComputedStyles { id: usize },
    #[serde(rename = "mouse_move")]
    MouseMove { x: f32, y: f32 },
    #[serde(rename = "scroll")]
    Scroll { x: f32, y: f32, delta_x: f64, delta_y: f64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DebugResult {
    #[serde(rename = "json")]
    Json { data: serde_json::Value },
    #[serde(rename = "bytes")]
    Bytes { data: String },
    #[serde(rename = "error")]
    Error { message: String },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    #[serde(flatten)]
    pub command: DebugCommandKind,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(flatten)]
    pub result: DebugResult,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HandshakeRequest {
    pub protocol: String,
    pub version: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HandshakeResponse {
    pub protocol: String,
    pub version: u32,
    pub app_name: String,
    pub pid: u32,
}

fn write_frame(stream: &mut TcpStream, data: &[u8]) -> std::io::Result<()> {
    let len = data.len() as u32;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(data)?;
    stream.flush()
}

fn read_frame(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 64 * 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Frame too large",
        ));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    Ok(buf)
}

pub struct DebugClient {
    stream: Mutex<TcpStream>,
    next_id: Mutex<u64>,
    pub app_name: String,
    pub pid: u32,
}

impl DebugClient {
    pub fn connect(port: u16) -> Result<Self, String> {
        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
            .map_err(|e| format!("Failed to connect to 127.0.0.1:{}: {}", port, e))?;

        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(30)))
            .ok();

        // Handshake
        let handshake = HandshakeRequest {
            protocol: "rinch-debug".into(),
            version: 1,
        };
        let data = serde_json::to_vec(&handshake).unwrap();
        write_frame(&mut stream, &data).map_err(|e| format!("Handshake write failed: {}", e))?;

        let resp_data =
            read_frame(&mut stream).map_err(|e| format!("Handshake read failed: {}", e))?;
        let resp: HandshakeResponse = serde_json::from_slice(&resp_data)
            .map_err(|e| format!("Invalid handshake response: {}", e))?;

        Ok(Self {
            stream: Mutex::new(stream),
            next_id: Mutex::new(1),
            app_name: resp.app_name,
            pid: resp.pid,
        })
    }

    pub fn execute(&self, command: DebugCommandKind) -> Result<DebugResult, String> {
        let mut stream = self.stream.lock().unwrap();
        let mut id = self.next_id.lock().unwrap();

        let request = Request {
            id: *id,
            command,
        };
        *id += 1;

        let data = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
        write_frame(&mut *stream, &data).map_err(|e| format!("Write failed: {}", e))?;

        let resp_data = read_frame(&mut *stream).map_err(|e| format!("Read failed: {}", e))?;
        let response: Response =
            serde_json::from_slice(&resp_data).map_err(|e| format!("Invalid response: {}", e))?;

        Ok(response.result)
    }
}
