use serde::{Deserialize, Serialize};
use std::sync::mpsc;

/// A command sent from an IPC client to the rinch event loop.
pub struct DebugCommand {
    pub kind: DebugCommandKind,
    pub response_tx: mpsc::Sender<DebugResult>,
}

/// The different commands the debug server can receive.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum DebugCommandKind {
    #[serde(rename = "screenshot")]
    Screenshot,
    #[serde(rename = "dom_tree")]
    DomTree {
        #[serde(default)]
        max_depth: Option<u32>,
        #[serde(default)]
        root_id: Option<usize>,
    },
    #[serde(rename = "query_selector")]
    QuerySelector { selector: String },
    #[serde(rename = "get_node")]
    GetNode { id: usize },
    #[serde(rename = "get_text_content")]
    GetTextContent { id: usize },
    #[serde(rename = "click")]
    Click {
        x: f32,
        y: f32,
        #[serde(default)]
        button: Option<String>,
    },
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
    #[serde(rename = "mouse_down")]
    MouseDown {
        x: f32,
        y: f32,
        #[serde(default)]
        button: Option<String>,
    },
    #[serde(rename = "mouse_up")]
    MouseUp {
        x: f32,
        y: f32,
        #[serde(default)]
        button: Option<String>,
    },
    #[serde(rename = "scroll")]
    Scroll {
        x: f32,
        y: f32,
        delta_x: f64,
        delta_y: f64,
    },
    #[serde(rename = "key_press")]
    KeyPress {
        key: String,
        shift: bool,
        ctrl: bool,
        #[serde(default)]
        alt: bool,
        /// Modifier names OR'd into the flat booleans by the runtime via
        /// [`fold_modifier_names`] — the natural array shape other automation
        /// protocols use, and the only way to request `meta`. Unknown names
        /// fail loud instead of silently altering the simulated input.
        #[serde(default)]
        modifiers: Vec<String>,
    },
    #[serde(rename = "ime")]
    Ime {
        /// One of `"enable"`, `"preedit"`, `"commit"`, `"disable"`.
        action: String,
        /// The preedit composition string, or the committed text.
        #[serde(default)]
        text: String,
        /// Optional `(begin, end)` byte cursor within a preedit composition.
        #[serde(default)]
        cursor: Option<(usize, usize)>,
    },
    #[serde(rename = "get_caret_position")]
    GetCaretPosition { node_id: usize, byte_offset: usize },
    #[serde(rename = "get_glyph_bounds")]
    GetGlyphBounds { node_id: usize, byte_offset: usize },
}

/// Result of a debug command.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DebugResult {
    #[serde(rename = "json")]
    Json { data: serde_json::Value },
    #[serde(rename = "bytes")]
    Bytes { data: String }, // base64-encoded
    #[serde(rename = "error")]
    Error { message: String },
}

/// Wire request envelope.
#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    #[serde(flatten)]
    pub command: DebugCommandKind,
}

/// Wire response envelope.
#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(flatten)]
    pub result: DebugResult,
}

/// Handshake request from client.
#[derive(Debug, Serialize, Deserialize)]
pub struct HandshakeRequest {
    pub protocol: String,
    pub version: u32,
}

/// Handshake response from server.
#[derive(Debug, Serialize, Deserialize)]
pub struct HandshakeResponse {
    pub protocol: String,
    pub version: u32,
    pub app_name: String,
    pub pid: u32,
}

/// Fold a `key_press` `modifiers` name array into flat modifier booleans.
///
/// Recognized names (case-insensitive): `"ctrl"`/`"control"`, `"shift"`,
/// `"alt"`/`"option"`, `"meta"`/`"cmd"`/`"super"`. Returns the offending name
/// on failure so callers fail loud instead of silently dropping a modifier.
pub fn fold_modifier_names(
    names: &[String],
    shift: &mut bool,
    ctrl: &mut bool,
    alt: &mut bool,
    meta: &mut bool,
) -> Result<(), String> {
    for name in names {
        match name.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => *ctrl = true,
            "shift" => *shift = true,
            "alt" | "option" => *alt = true,
            "meta" | "cmd" | "super" => *meta = true,
            _ => return Err(name.clone()),
        }
    }
    Ok(())
}

/// Write a length-prefixed frame (4-byte big-endian length + JSON payload).
pub fn write_frame(stream: &mut impl std::io::Write, data: &[u8]) -> std::io::Result<()> {
    let len = data.len() as u32;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(data)?;
    stream.flush()
}

/// Read a length-prefixed frame (4-byte big-endian length + JSON payload).
pub fn read_frame(stream: &mut impl std::io::Read) -> std::io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 64 * 1024 * 1024 {
        // 64MB safety limit
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Frame too large",
        ));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    Ok(buf)
}

#[cfg(test)]
mod key_press_modifiers_tests {
    use super::*;

    #[test]
    fn modifiers_array_deserializes_and_folds_to_ctrl_true() {
        // The array shape guessed from CDP/WebDriver/Playwright must not be
        // silently dropped (issue #152): it deserializes into the variant and
        // folds onto the flat booleans.
        let req: Request = serde_json::from_str(
            r#"{"id":1,"method":"key_press","params":{"key":"End","shift":false,"ctrl":false,"modifiers":["ctrl"]}}"#,
        )
        .unwrap();
        let DebugCommandKind::KeyPress {
            key,
            mut shift,
            mut ctrl,
            mut alt,
            modifiers,
        } = req.command
        else {
            panic!("expected KeyPress");
        };
        assert_eq!(key, "End");
        assert_eq!(modifiers, vec!["ctrl".to_string()]);
        let mut meta = false;
        fold_modifier_names(&modifiers, &mut shift, &mut ctrl, &mut alt, &mut meta).unwrap();
        assert!(ctrl);
        assert!(!shift && !alt && !meta);
    }

    #[test]
    fn key_press_round_trips_with_modifiers() {
        let original = Request {
            id: 7,
            command: DebugCommandKind::KeyPress {
                key: "End".into(),
                shift: false,
                ctrl: false,
                alt: false,
                modifiers: vec!["ctrl".into(), "Shift".into()],
            },
        };
        let json = serde_json::to_string(&original).unwrap();
        let back: Request = serde_json::from_str(&json).unwrap();
        let DebugCommandKind::KeyPress { key, modifiers, .. } = back.command else {
            panic!("expected KeyPress");
        };
        assert_eq!(key, "End");
        assert_eq!(modifiers, vec!["ctrl".to_string(), "Shift".to_string()]);
    }

    #[test]
    fn omitting_the_modifiers_array_defaults_to_empty() {
        // The stricter raw-TCP contract is preserved: shift/ctrl stay
        // required, the new array is optional.
        let req: Request = serde_json::from_str(
            r#"{"id":2,"method":"key_press","params":{"key":"End","shift":true,"ctrl":true}}"#,
        )
        .unwrap();
        let DebugCommandKind::KeyPress {
            shift,
            ctrl,
            modifiers,
            ..
        } = req.command
        else {
            panic!("expected KeyPress");
        };
        assert!(shift && ctrl);
        assert!(modifiers.is_empty());
    }

    #[test]
    fn unknown_modifier_name_fails_loud() {
        let (mut s, mut c, mut a, mut m) = (false, false, false, false);
        let err = fold_modifier_names(
            &["ctrl".into(), "hyper".into()],
            &mut s,
            &mut c,
            &mut a,
            &mut m,
        )
        .unwrap_err();
        assert_eq!(err, "hyper");
    }

    #[test]
    fn modifier_names_are_case_insensitive_with_aliases() {
        let (mut s, mut c, mut a, mut m) = (false, false, false, false);
        fold_modifier_names(
            &[
                "Control".into(),
                "SHIFT".into(),
                "option".into(),
                "Cmd".into(),
            ],
            &mut s,
            &mut c,
            &mut a,
            &mut m,
        )
        .unwrap();
        assert!(s && c && a && m);
    }
}
