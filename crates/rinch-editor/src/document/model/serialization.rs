//! Serialization methods for EditorDocument (to_html, to_bytes, from_bytes, to_text).

use automerge::{AutoCommit, ObjType, ReadDoc};

use super::{EditorDocument, MarkData};
use crate::error::EditorError;

impl EditorDocument {
    /// Create from Automerge bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, EditorError> {
        let doc = AutoCommit::load(bytes).map_err(|e| EditorError::Automerge(e.to_string()))?;
        // Find the content_id
        let content_id = doc
            .get(&automerge::ROOT, "content")
            .map_err(|e| EditorError::Automerge(e.to_string()))?
            .and_then(|(val, id)| {
                if matches!(val, automerge::Value::Object(ObjType::List)) {
                    Some(id)
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                EditorError::Automerge("Missing or invalid 'content' field".to_string())
            })?;

        Ok(Self { doc, content_id })
    }

    /// Export to Automerge bytes.
    pub fn to_bytes(&mut self) -> Vec<u8> {
        self.doc.save()
    }

    /// Get markdown representation of the document.
    pub fn to_markdown(&self) -> String {
        use crate::document::fragment::{Fragment, FragmentBlock, FragmentInline};

        let count = self.block_count();
        let mut blocks = Vec::with_capacity(count);

        for i in 0..count {
            let block_type = self.block_type(i).unwrap_or_else(|| "paragraph".into());
            let attrs = self.block_attrs(i).unwrap_or_default();
            let runs = self.block_inline_runs(i);

            let content: Vec<FragmentInline> = runs
                .iter()
                .map(|run| {
                    if run.inline_type == "hard_break" {
                        FragmentInline::HardBreak
                    } else {
                        FragmentInline::Text {
                            text: run.text.clone(),
                            marks: run.marks.clone(),
                        }
                    }
                })
                .collect();

            blocks.push(FragmentBlock {
                block_type,
                attrs,
                content,
            });
        }

        Fragment { blocks }.to_markdown()
    }

    /// Get all text as a single string (blocks separated by newlines).
    pub fn to_text(&self) -> String {
        let count = self.block_count();
        let mut parts = Vec::with_capacity(count);
        for i in 0..count {
            parts.push(self.block_text(i).unwrap_or_default());
        }
        parts.join("\n")
    }

    /// Get simple HTML representation.
    pub fn to_html(&self) -> String {
        let count = self.block_count();
        let mut html = String::new();
        for i in 0..count {
            let block_type = self.block_type(i).unwrap_or_else(|| "p".to_string());
            let tag = match block_type.as_str() {
                "paragraph" => "p",
                "heading" => {
                    let attrs = self.block_attrs(i).unwrap_or_default();
                    match attrs.get("level").map(|s| s.as_str()) {
                        Some("1") => "h1",
                        Some("2") => "h2",
                        Some("3") => "h3",
                        Some("4") => "h4",
                        Some("5") => "h5",
                        Some("6") => "h6",
                        _ => "h1",
                    }
                }
                "blockquote" => "blockquote",
                "code_block" => "pre",
                "bullet_list" => "ul",
                "ordered_list" => "ol",
                "list_item" => "li",
                "horizontal_rule" => {
                    html.push_str("<hr />");
                    continue;
                }
                other => other,
            };
            html.push_str(&format!("<{}>", tag));
            html.push_str(&self.block_inline_html(i));
            html.push_str(&format!("</{}>", tag));
        }
        html
    }

    /// Get HTML for inline content of a block.
    fn block_inline_html(&self, block_index: usize) -> String {
        let block_id = match self.block_obj(block_index) {
            Some(id) => id,
            None => return String::new(),
        };
        let content_id = match self.block_content_obj(&block_id) {
            Some(id) => id,
            None => return String::new(),
        };
        let mut html = String::new();
        let len = self.doc.length(&content_id);
        for i in 0..len {
            if let Some((_, inline_id)) = self.doc.get(&content_id, i).ok().flatten() {
                let inline_type = self.get_str(&inline_id, "type").unwrap_or_default();
                match inline_type.as_str() {
                    "text" => {
                        let text = self.get_str(&inline_id, "text").unwrap_or_default();
                        let marks = self.read_marks(&inline_id);
                        let mut result = html_escape(&text);
                        // Wrap in mark tags (innermost first)
                        for mark in &marks {
                            result = wrap_mark(&result, mark);
                        }
                        html.push_str(&result);
                    }
                    "hard_break" => {
                        html.push_str("<br>");
                    }
                    _ => {}
                }
            }
        }
        html
    }
}

/// Simple HTML escaping.
pub(crate) fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Wrap text in an HTML tag for a mark.
pub(crate) fn wrap_mark(content: &str, mark: &MarkData) -> String {
    match mark.mark_type.as_str() {
        "bold" => format!("<strong>{}</strong>", content),
        "italic" => format!("<em>{}</em>", content),
        "underline" => format!("<u>{}</u>", content),
        "strikethrough" | "strike" => format!("<s>{}</s>", content),
        "code" => format!("<code>{}</code>", content),
        "link" => {
            let href = mark.attrs.get("href").map(|s| s.as_str()).unwrap_or("#");
            format!("<a href=\"{}\">{}</a>", html_escape(href), content)
        }
        "highlight" => format!("<mark>{}</mark>", content),
        "subscript" => format!("<sub>{}</sub>", content),
        "superscript" => format!("<sup>{}</sup>", content),
        "textColor" | "text_color" => {
            let color = mark.attrs.get("color").map(|s| s.as_str()).unwrap_or("");
            if color.is_empty() {
                content.to_string()
            } else {
                format!("<span style=\"color:{}\">{}</span>", html_escape(color), content)
            }
        }
        _ => content.to_string(),
    }
}
