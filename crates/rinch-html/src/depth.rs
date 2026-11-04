//! A document's nesting, cut short before anything recursive reads it.
//!
//! css-inline serializes recursively: ~16k nested elements (80 KB) overflow
//! an 8 MB stack and abort the process, and the time grows about fourfold
//! per doubling well before that. Mail is a few dozen levels deep, so
//! [`cap`] drops whatever is nested deeper than [`MAX_DEPTH`].
//!
//! The scan follows the HTML parser closely enough for this: void elements
//! open nothing; `<p>`, `<li>`, `<dt>`/`<dd>`, `<option>`, cells and rows
//! close the open one of their kind (as the parser closes an unclosed `<p>`
//! at the next), a row closes an open cell, and an end tag closes up to the
//! nearest open element of its name. Comments and the text of `<script>`,
//! `<style>`, `<textarea>` and `<title>` are passed over whole. Where it
//! counts deeper than the parser would, it only cuts sooner.

use std::borrow::Cow;

/// The deepest nesting kept.
pub const MAX_DEPTH: usize = 512;

/// `document` with every element nested deeper than [`MAX_DEPTH`] left out,
/// borrowed when nothing is.
pub(crate) fn cap(document: &str) -> Cow<'_, str> {
    let mut stack: Vec<String> = Vec::new();
    let mut out: Option<String> = None;
    // Where the copy has got to, and where a cut started.
    let mut copied = 0;
    let mut cut_from: Option<usize> = None;
    // How many elements are open inside the cut: they are counted, not
    // named, so an end tag there costs nothing to match.
    let mut deeper = 0usize;
    let bytes = document.as_bytes();
    let mut i = 0;
    while let Some(lt) = document[i..].find('<').map(|p| p + i) {
        if document[lt..].starts_with("<!--") {
            i = document[lt + 4..]
                .find("-->")
                .map_or(document.len(), |p| lt + 4 + p + 3);
            continue;
        }
        let Some(gt) = document[lt..].find('>').map(|p| lt + p) else {
            break;
        };
        let tag = &document[lt + 1..gt];
        i = gt + 1;
        if let Some(name) = tag.strip_prefix('/') {
            if deeper > 0 {
                deeper -= 1;
                continue;
            }
            let name = tag_name(name);
            if let Some(at) = stack.iter().rposition(|n| *n == name) {
                stack.truncate(at);
                if let Some(from) = cut_from
                    && stack.len() <= MAX_DEPTH
                {
                    // The cut ends with the element that crossed the limit.
                    let o = out.get_or_insert_with(String::new);
                    o.push_str(&document[copied..from]);
                    copied = i;
                    cut_from = None;
                }
            }
            continue;
        }
        // A `<` that starts no tag (`a < b`) is text.
        if !bytes.get(lt + 1).is_some_and(u8::is_ascii_alphabetic) {
            continue;
        }
        let name = tag_name(tag);
        if let Some(raw) = raw_text_end(document, i, &name) {
            i = raw;
            continue;
        }
        if is_void(&name) || tag.ends_with('/') {
            continue;
        }
        if cut_from.is_some() {
            deeper += 1;
            continue;
        }
        close_implied(&mut stack, &name);
        stack.push(name);
        if stack.len() > MAX_DEPTH {
            cut_from = Some(lt);
        }
    }
    match (out, cut_from) {
        (None, None) => Cow::Borrowed(document),
        (out, cut) => {
            let mut o = out.unwrap_or_default();
            o.push_str(&document[copied..cut.unwrap_or(document.len())]);
            Cow::Owned(o)
        }
    }
}

fn tag_name(tag: &str) -> String {
    tag.split(|c: char| c.is_whitespace() || c == '/' || c == '>')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Past the end tag of an element whose content is raw text, if `name` is one.
fn raw_text_end(document: &str, from: usize, name: &str) -> Option<usize> {
    if !matches!(name, "script" | "style" | "textarea" | "title") {
        return None;
    }
    let close = format!("</{name}");
    let at = document[from..].to_ascii_lowercase().find(&close)? + from;
    Some(
        document[at..]
            .find('>')
            .map_or(document.len(), |p| at + p + 1),
    )
}

/// Close what opening `name` closes in the parser.
fn close_implied(stack: &mut Vec<String>, name: &str) {
    let closes: &[&str] = match name {
        "p" | "div" | "table" | "ul" | "ol" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => &["p"],
        "li" => &["li"],
        "dt" | "dd" => &["dt", "dd"],
        "option" => &["option"],
        "td" | "th" => &["td", "th"],
        "tr" => &["tr", "td", "th"],
        "tbody" | "thead" | "tfoot" => &["tbody", "thead", "tfoot", "tr", "td", "th"],
        _ => &[],
    };
    while stack
        .last()
        .is_some_and(|top| closes.contains(&top.as_str()))
    {
        stack.pop();
    }
}

fn is_void(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "source"
            | "track"
            | "wbr"
    )
}

#[cfg(test)]
mod tests {
    use super::{MAX_DEPTH, cap};

    #[test]
    fn ordinary_documents_are_untouched() {
        let doc = "<table><tr><td><p>one<p>two<ul><li>a<li>b</ul></td><td>x</table><!-- <div> -->\
                   <style>div > p { }</style><img src=x><br>";
        assert!(matches!(cap(doc), std::borrow::Cow::Borrowed(_)));
        let mut many_ps = "<p>x".repeat(5_000);
        many_ps.push_str("<div>end</div>");
        assert!(matches!(cap(&many_ps), std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn what_is_nested_too_deep_is_left_out() {
        let depth = MAX_DEPTH + 100;
        let doc = format!(
            "<p>before</p>{}deep{}<p>after</p>",
            "<div>".repeat(depth),
            "</div>".repeat(depth)
        );
        let out = cap(&doc);
        assert!(!out.contains("deep"), "the deep text is gone");
        assert!(out.contains("before") && out.contains("after"));
        assert_eq!(out.matches("<div>").count(), MAX_DEPTH);
    }
}
