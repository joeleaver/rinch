//! The XML of a Windows toast. Platform-neutral so it is tested everywhere.

use std::path::Path;

use crate::notification::Request;

/// The toast's XML: the generic template, title and body as text (escaped),
/// the image as the app-logo override.
pub(crate) fn toast_xml(req: &Request) -> String {
    let mut xml = String::from("<toast><visual><binding template=\"ToastGeneric\">");
    xml.push_str(&format!("<text>{}</text>", xml_escape(&req.title)));
    if !req.body.is_empty() {
        xml.push_str(&format!("<text>{}</text>", xml_escape(&req.body)));
    }
    if let Some(icon) = &req.icon {
        xml.push_str(&format!(
            "<image placement=\"appLogoOverride\" src=\"{}\"/>",
            xml_escape(&file_uri(icon))
        ));
    }
    xml.push_str("</binding></visual></toast>");
    xml
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // XML 1.0 has no representation for the other C0 controls.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

fn file_uri(path: &Path) -> String {
    format!("file:///{}", path.to_string_lossy().replace('\\', "/"))
}
