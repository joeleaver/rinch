//! The allow-list: an HTML document someone else wrote (mail, a feed item,
//! a help page from a server), reduced to what is safe to put into the
//! app's own DOM with `set_inner_html`.
//!
//! - **Parsed as a browser parses** (html5ever, through ammonia), then kept
//!   only as far as the allow-list says: no scripts, forms, frames, objects,
//!   event attributes or `<style>`/`<link>` elements.
//! - **Style sheets are applied, not kept.** A document's `<style>` rules
//!   are copied onto the elements they match (css-inline), and the sheets
//!   dropped: a sheet in the app's DOM would style the app's own elements.
//!   Rules inside `@media` blocks are left out (mostly phone layouts), and a
//!   linked sheet is never fetched.
//! - **Inline styles are filtered**: nothing that lifts content out of its
//!   own box (`position` other than `static`/`relative`, `z-index`), no
//!   `expression()`/`javascript:`, no `behavior`/`-moz-binding`, `cursor`
//!   or `pointer-events`.
//! - **Every picture goes through [`Policy::picture`]**: `<img src>` and each
//!   `url()` in a style. The default keeps only `data:image/` pictures, so a
//!   cleaned document fetches nothing unless the app says it may.
//! - **Links keep only [`Policy::link_schemes`]** (`http`, `https`, `mailto`
//!   by default) and get `rel="noopener noreferrer"`.
//! - Legacy presentational attributes (`bgcolor`, `width`, `align`,
//!   `valign`, `cellpadding`, `border`, `<font color>`…) are kept: rinch maps
//!   them as presentational hints, as a browser does (`rinch-dom`'s
//!   `legacy_hints`).

use std::collections::HashSet;
use std::sync::Arc;

/// Decides a picture's source: `Some` with the source to load (the same, or
/// rewritten, such as a `cid:` reference turned into a `data:` URL), `None`
/// to leave the picture out.
pub type PictureRule = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// What [`clean`] lets through beyond its fixed allow-list.
#[derive(Clone)]
pub struct Policy {
    /// Every picture source, from `<img src>` and from `url()` in styles.
    /// The default keeps `data:image/` sources only.
    pub picture: PictureRule,
    /// The schemes a link may have; a link with another loses its `href`.
    pub link_schemes: Vec<String>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            picture: Arc::new(|src| {
                let lower = src.trim().to_ascii_lowercase();
                lower
                    .starts_with("data:image/")
                    .then(|| src.trim().to_owned())
            }),
            link_schemes: vec!["http".into(), "https".into(), "mailto".into()],
        }
    }
}

impl std::fmt::Debug for Policy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Policy")
            .field("picture", &"<rule>")
            .field("link_schemes", &self.link_schemes)
            .finish()
    }
}

/// A document, cleaned: safe to put into the app's DOM.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cleaned {
    /// The inside of the document's `<body>`.
    pub html: String,
    /// The `<body>`'s own style (its background, margin, colour, font),
    /// filtered, for the element that stands for the body. Its `bgcolor` and
    /// `text` attributes are included as CSS.
    pub body_style: String,
}

impl Cleaned {
    /// Plain text, escaped, its line breaks and runs of spaces kept.
    pub fn from_text(text: &str) -> Self {
        let escaped = text
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace("\r\n", "\n");
        Self {
            html: format!(
                "<div style=\"white-space: pre-wrap; overflow-wrap: anywhere\">{escaped}</div>"
            ),
            body_style: String::new(),
        }
    }
}

/// Tags kept beyond ammonia's defaults: the legacy ones documents still use.
const EXTRA_TAGS: &[&str] = &["font", "center", "big", "strike"];
/// Attributes kept on any tag: layout, and the legacy presentational ones
/// rinch maps as hints.
const LAYOUT_ATTRIBUTES: &[&str] = &[
    "style",
    "align",
    "valign",
    "bgcolor",
    "width",
    "height",
    "border",
    "bordercolor",
    "cellpadding",
    "cellspacing",
    "color",
    "face",
    "size",
    "dir",
    "colspan",
    "rowspan",
    "nowrap",
];
/// Inline style properties dropped whatever their value.
const DROPPED_PROPERTIES: &[&str] = &["behavior", "-moz-binding", "cursor", "pointer-events"];

/// Clean `document` (a whole document or a fragment) under `policy`.
pub fn clean(document: &str, policy: &Policy) -> Cleaned {
    let inlined = inline_sheets(document);
    let document = inlined.as_str();
    let body_style = body_style(body_tag(document).unwrap_or_default(), policy);

    // ammonia checks the scheme of every URL attribute first; pictures and
    // links get their own rules below, so it lets through what either may.
    let mut schemes: HashSet<String> = policy.link_schemes.iter().cloned().collect();
    schemes.extend(["http", "https", "data", "cid"].map(String::from));
    let schemes: HashSet<&str> = schemes.iter().map(String::as_str).collect();

    let picture = policy.picture.clone();
    let links: Vec<String> = policy.link_schemes.clone();
    let mut builder = ammonia::Builder::default();
    builder
        .add_tags(EXTRA_TAGS)
        .add_generic_attributes(LAYOUT_ATTRIBUTES)
        .add_clean_content_tags(["title"])
        .url_schemes(schemes)
        .link_rel(Some("noopener noreferrer"))
        .attribute_filter(
            move |element, attribute, value| match (element, attribute) {
                (_, "style") => Some(clean_style(value, &picture).into()),
                ("img", "src") => picture(value).map(Into::into),
                ("a", "href") => link(value, &links).map(Into::into),
                _ => Some(value.into()),
            },
        );
    Cleaned {
        html: builder.clean(document).to_string(),
        body_style,
    }
}

/// `document` with its `<style>` sheets applied as inline styles. As it was
/// if the sheets do not parse.
fn inline_sheets(document: &str) -> String {
    let inliner = css_inline::CSSInliner::options()
        .keep_style_tags(false)
        .keep_link_tags(false)
        .keep_at_rules(false)
        .load_remote_stylesheets(false)
        .build();
    inliner
        .inline(document)
        .unwrap_or_else(|_| document.to_owned())
}

/// A link's `href`, if its scheme is allowed (a relative link has none and
/// is kept: there is no page for it to be relative to, so it goes nowhere).
fn link(href: &str, schemes: &[String]) -> Option<String> {
    let trimmed = href.trim();
    match trimmed.split_once(':') {
        Some((scheme, _))
            if scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) =>
        {
            schemes
                .iter()
                .any(|s| s.eq_ignore_ascii_case(scheme))
                .then(|| trimmed.to_owned())
        }
        _ => Some(trimmed.to_owned()),
    }
}

/// An inline style without what could cover the app, run code, or fetch a
/// picture the policy does not allow.
pub(crate) fn clean_style(style: &str, picture: &PictureRule) -> String {
    let mut kept = Vec::new();
    for declaration in style.split(';') {
        let Some((property, value)) = declaration.split_once(':') else {
            continue;
        };
        let property = property.trim().to_ascii_lowercase();
        let value = value.trim();
        let lower = value.to_ascii_lowercase();
        if DROPPED_PROPERTIES.contains(&property.as_str())
            || lower.contains("expression(")
            || lower.contains("javascript:")
        {
            continue;
        }
        // A document cannot lift itself out of its own box onto the app.
        if property == "position" && !matches!(lower.as_str(), "static" | "relative") {
            continue;
        }
        if property == "z-index" {
            continue;
        }
        let value = if lower.contains("url(") {
            match pictures_in(value, picture) {
                Some(value) => value,
                None => continue,
            }
        } else {
            value.to_owned()
        };
        kept.push(format!("{property}: {value}"));
    }
    kept.join("; ")
}

/// `value` with each `url(...)` replaced by what the picture rule makes of
/// it; `None` if the rule refuses any of them.
fn pictures_in(value: &str, picture: &PictureRule) -> Option<String> {
    let mut out = String::new();
    let mut rest = value;
    while let Some(at) = rest.to_ascii_lowercase().find("url(") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 4..];
        let end = after.find(')')?;
        let src = after[..end].trim().trim_matches(['"', '\'']);
        let kept = picture(src)?;
        if kept.contains(['"', '\\', '\n']) {
            return None;
        }
        out.push_str(&format!("url(\"{kept}\")"));
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// The document's `<body ...>` start tag: ammonia keeps what is inside the
/// body, not the body's own attributes.
fn body_tag(source: &str) -> Option<&str> {
    let lower = source.to_ascii_lowercase();
    let start = lower.find("<body")?;
    let end = lower[start..].find('>')? + start;
    Some(&source[start..end])
}

/// The style for the element that stands for the body.
fn body_style(tag: &str, policy: &Policy) -> String {
    let mut declarations = Vec::new();
    let colour = |name: &str| {
        attribute(tag, name)
            .map(|c| {
                c.chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '#')
                    .collect::<String>()
            })
            .filter(|c| !c.is_empty())
    };
    if let Some(c) = colour("bgcolor") {
        declarations.push(format!("background-color: {c}"));
    }
    if let Some(c) = colour("text") {
        declarations.push(format!("color: {c}"));
    }
    let own = clean_style(
        &attribute(tag, "style").unwrap_or_default(),
        &policy.picture,
    );
    if !own.is_empty() {
        declarations.push(own);
    }
    declarations.join("; ")
}

/// One attribute's value in a start tag's text.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find(name) {
        let start = from + at;
        from = start + name.len();
        let before = lower[..start].chars().last();
        if !before.is_some_and(char::is_whitespace) {
            continue;
        }
        let rest = tag[from..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        return Some(match rest.chars().next() {
            Some(q @ ('"' | '\'')) => rest[1..].split(q).next().unwrap_or_default().to_owned(),
            _ => rest.split([' ', '>']).next().unwrap_or_default().to_owned(),
        });
    }
    None
}
