//! Highlight component.
//!
//! Highlights matching substrings within text.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

/// Highlights matching parts of text.
///
/// Every occurrence of `highlight`, and of each term in `highlights`, is
/// wrapped in a `<mark class="rinch-highlight__match">`. Where two terms match
/// at the same place the longer one wins (`"New York"` over `"New"`), and
/// matches never overlap.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     Highlight {
///         text: "Search results for query",
///         highlight: "query",
///     }
///     Highlight {
///         text: "Apples, pears and plums",
///         highlights: vec!["apple".into(), "plum".into()],
///         match_class: "my-hit",
///     }
/// }
/// ```
#[derive(Debug)]
pub struct Highlight {
    /// The full text to display.
    pub text: String,
    /// A substring to highlight. Empty means none.
    pub highlight: String,
    /// More substrings to highlight, alongside `highlight`.
    pub highlights: Vec<String>,
    /// Background color for highlighted parts.
    pub color: String,
    /// Whether to do case-insensitive matching (default `true`, as a browser's
    /// find-in-page is). The comparison is per character, by full Unicode
    /// lowercase, and always marks the text as written.
    pub ignore_case: bool,
    /// Extra class(es) for each `<mark>`, beside `rinch-highlight__match`.
    pub match_class: String,
}

impl Default for Highlight {
    fn default() -> Self {
        Self {
            text: String::new(),
            highlight: String::new(),
            highlights: Vec::new(),
            color: String::new(),
            ignore_case: true,
            match_class: String::new(),
        }
    }
}

/// The byte ranges of `text` that the `terms` match, left to right and not
/// overlapping; where several terms match at one position the longest wins.
/// Empty terms are ignored.
///
/// Case-insensitive matching compares character by character, each side by its
/// full lowercase mapping, and measures the match in `text`'s own bytes. It
/// used to lowercase the whole text and slice the original with the lowercased
/// copy's offsets, which differ wherever a letter's lowercase has another UTF-8
/// length (`İ`, 2 bytes, lowercases to `i̇`, 3): the wrong letters were marked,
/// or a slice ran past the end and panicked (#2207).
pub fn match_ranges(text: &str, terms: &[&str], ignore_case: bool) -> Vec<(usize, usize)> {
    let fold = |s: &str| -> Vec<char> {
        if ignore_case {
            s.chars().flat_map(char::to_lowercase).collect()
        } else {
            s.chars().collect()
        }
    };
    let terms: Vec<Vec<char>> = terms
        .iter()
        .filter(|t| !t.is_empty())
        .map(|t| fold(t))
        .collect();
    let mut out = Vec::new();
    if terms.is_empty() {
        return out;
    }
    let mut start = 0;
    while let Some(first) = text[start..].chars().next() {
        let rest = &text[start..];
        match terms
            .iter()
            .filter_map(|t| match_len(rest, t, ignore_case))
            .max()
        {
            Some(len) => {
                out.push((start, start + len));
                start += len;
            }
            None => start += first.len_utf8(),
        }
    }
    out
}

/// The byte length of the prefix of `text` that `term` (already folded)
/// matches, if one does. The match must end on a whole character of `text`: a
/// term that stops partway through a letter's lowercase expansion does not
/// match it.
fn match_len(text: &str, term: &[char], ignore_case: bool) -> Option<usize> {
    let mut want = term.iter().peekable();
    for (i, c) in text.char_indices() {
        let mut fold_one = |l: char| -> bool { want.next() == Some(&l) };
        let all = if ignore_case {
            c.to_lowercase().all(&mut fold_one)
        } else {
            fold_one(c)
        };
        if !all {
            return None;
        }
        if want.peek().is_none() {
            return Some(i + c.len_utf8());
        }
    }
    None
}

impl Component for Highlight {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let text = &self.text;

        let container = rinch_macros::rsx! { span { class: "rinch-highlight" } };

        // Color style
        if !self.color.is_empty() {
            let c = &self.color;
            let style = if c.starts_with('#') || c.starts_with("rgb") || c.starts_with("hsl") {
                format!("--rinch-highlight-color: {}", c)
            } else {
                format!("--rinch-highlight-color: var(--rinch-color-{}-2)", c)
            };
            container.set_attribute("style", &style);
        }

        let terms: Vec<&str> = std::iter::once(self.highlight.as_str())
            .chain(self.highlights.iter().map(String::as_str))
            .collect();
        let mark_class = match self.match_class.trim() {
            "" => "rinch-highlight__match".to_string(),
            extra => format!("rinch-highlight__match {extra}"),
        };

        let mut last_end = 0;
        for (start, end) in match_ranges(text, &terms, self.ignore_case) {
            if start > last_end {
                let before =
                    rinch_core::IntoNode::into_node(text[last_end..start].to_string(), __scope);
                container.append_child(&before);
            }
            // The match as written, whatever case the term was in.
            let mark = rinch_macros::rsx! { mark { class: "rinch-highlight__match", {text[start..end].to_string()} } };
            mark.set_attribute("class", &mark_class);
            container.append_child(&mark);
            last_end = end;
        }

        // The rest (all of it when nothing matched, and the empty text node an
        // empty `text` always rendered).
        if last_end < text.len() || text.is_empty() {
            let remaining = rinch_core::IntoNode::into_node(text[last_end..].to_string(), __scope);
            container.append_child(&remaining);
        }

        container
    }
}
