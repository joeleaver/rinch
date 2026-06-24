//! The editor's default stylesheet — a polished, batteries-included look for the
//! rich-text editor in both light and dark color schemes.
//!
//! Consumers get a beautiful editor out of the box: headings, lists, blockquotes,
//! code, horizontal rules, links, inline marks, and **tables** are all styled
//! without hand-rolling CSS. The view injects [`DEFAULT_EDITOR_CSS`] into the
//! document once on first mount ([`ensure_default_styles`]); everything is scoped
//! to `[data-pm-editor]` so it never leaks onto the rest of the app, and consumers
//! can override any rule with their own (higher-specificity) styles.
//!
//! **Color scheme:** light by default. Dark mode is opt-in via a
//! `data-pm-theme="dark"` attribute on the editor container (set by
//! [`EditorHandle::set_dark_mode`](super::EditorHandle::set_dark_mode)); the dark
//! rules below carry that extra attribute so they win by specificity.
//!
//! **Tables:** rinch-dom has no `display: table`, so tables are laid out with
//! flexbox (`table` = column, `row` = row, cells = equal-width flex items). This
//! is why the default stylesheet is load-bearing for tables, not just cosmetic.

use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Weak;

use rinch_core::dom::DomDocument;

/// The default editor stylesheet (light + dark), scoped to `[data-pm-editor]`.
pub(crate) const DEFAULT_EDITOR_CSS: &str = r#"
/* ── Container ─────────────────────────────────────────────────────────── */
[data-pm-editor] {
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
  font-size: 16px;
  line-height: 1.65;
  color: #1f2328;
  background: #ffffff;
  padding: 16px 20px;
  border: 1px solid #d0d7de;
  border-radius: 10px;
}

/* ── Headings ──────────────────────────────────────────────────────────── */
[data-pm-editor] h1 { font-size: 2em;    font-weight: 700; line-height: 1.25; margin: 0.5em 0 0.5em; color: #0d1117; }
[data-pm-editor] h2 { font-size: 1.5em;  font-weight: 700; line-height: 1.3;  margin: 0.7em 0 0.4em; color: #0d1117; }
[data-pm-editor] h3 { font-size: 1.25em; font-weight: 600; line-height: 1.35; margin: 0.7em 0 0.3em; }
[data-pm-editor] h4 { font-size: 1.05em; font-weight: 600; margin: 0.7em 0 0.3em; }
[data-pm-editor] h5 { font-size: 0.95em; font-weight: 600; margin: 0.7em 0 0.3em; color: #57606a; }
[data-pm-editor] h6 { font-size: 0.9em;  font-weight: 600; margin: 0.7em 0 0.3em; color: #57606a; }

/* ── Body text ─────────────────────────────────────────────────────────── */
[data-pm-editor] p { margin: 0 0 0.75em; }
[data-pm-editor] a { color: #0969da; text-decoration: underline; }

/* ── Inline marks ──────────────────────────────────────────────────────── */
[data-pm-editor] strong, [data-pm-editor] b { font-weight: 700; }
[data-pm-editor] em, [data-pm-editor] i { font-style: italic; }
[data-pm-editor] u { text-decoration: underline; }
[data-pm-editor] s { text-decoration: line-through; }
[data-pm-editor] mark { background: #fff3a3; color: #1f2328; padding: 0 2px; border-radius: 2px; }
[data-pm-editor] code {
  font-family: ui-monospace, "SF Mono", Menlo, Consolas, "Liberation Mono", monospace;
  font-size: 0.9em; background: #eff1f3; padding: 0.15em 0.35em; border-radius: 4px;
}

/* ── Blockquote ────────────────────────────────────────────────────────── */
[data-pm-editor] blockquote {
  margin: 0.8em 0; padding: 0.2em 0 0.2em 1em;
  border-left: 3px solid #d0d7de; color: #57606a;
}

/* ── Code block ────────────────────────────────────────────────────────── */
[data-pm-editor] pre {
  font-family: ui-monospace, "SF Mono", Menlo, Consolas, "Liberation Mono", monospace;
  font-size: 0.9em; line-height: 1.5; background: #f6f8fa;
  padding: 0.85em 1em; border-radius: 8px; margin: 0.8em 0; white-space: pre-wrap;
}

/* ── Lists ─────────────────────────────────────────────────────────────── */
/* `li` is flex so the bullet/number aligns with the first line of its content
   (rinch-dom renders list markers as block siblings otherwise). */
[data-pm-editor] ul, [data-pm-editor] ol { padding-left: 1.6em; margin: 0 0 0.75em; }
[data-pm-editor] li { display: flex; align-items: baseline; gap: 0.4em; margin: 0.15em 0; }

/* ── Horizontal rule ───────────────────────────────────────────────────── */
[data-pm-editor] hr { border: none; border-top: 2px solid #e1e4e8; margin: 1.2em 0; height: 0; }

/* ── Tables (flexbox — rinch-dom has no display:table) ─────────────────── */
[data-pm-editor] table {
  display: flex; flex-direction: column; margin: 0.85em 0;
  border: 1px solid #d0d7de; border-radius: 6px;
}
[data-pm-editor] tr { display: flex; flex-direction: row; }
[data-pm-editor] td, [data-pm-editor] th {
  display: block; flex-grow: 1; flex-basis: 0; min-width: 0;
  padding: 0.45em 0.7em; border: 1px solid #d0d7de;
}
[data-pm-editor] th { font-weight: 600; background: #f6f8fa; }

/* ── Placeholder (empty-document hint) ─────────────────────────────────── */
[data-pm-placeholder] { color: #8c959f; position: absolute; pointer-events: none; }

/* ═══ Dark color scheme (opt-in via data-pm-theme="dark") ═══════════════ */
[data-pm-editor][data-pm-theme="dark"] {
  color: #e6edf3;
  background: #0d1117;
  border-color: #30363d;
}
[data-pm-editor][data-pm-theme="dark"] h1,
[data-pm-editor][data-pm-theme="dark"] h2,
[data-pm-editor][data-pm-theme="dark"] h3,
[data-pm-editor][data-pm-theme="dark"] h4 { color: #f0f6fc; }
[data-pm-editor][data-pm-theme="dark"] h5,
[data-pm-editor][data-pm-theme="dark"] h6 { color: #9198a1; }
[data-pm-editor][data-pm-theme="dark"] a { color: #4493f8; }
[data-pm-editor][data-pm-theme="dark"] mark { background: #bb8009; color: #f0f6fc; }
[data-pm-editor][data-pm-theme="dark"] code { background: #2d333b; }
[data-pm-editor][data-pm-theme="dark"] blockquote { border-left-color: #3d444d; color: #9198a1; }
[data-pm-editor][data-pm-theme="dark"] pre { background: #161b22; }
[data-pm-editor][data-pm-theme="dark"] hr { border-top-color: #30363d; }
[data-pm-editor][data-pm-theme="dark"] table { border-color: #30363d; }
[data-pm-editor][data-pm-theme="dark"] td,
[data-pm-editor][data-pm-theme="dark"] th { border-color: #30363d; }
[data-pm-editor][data-pm-theme="dark"] th { background: #161b22; }
[data-pm-editor][data-pm-theme="dark"] [data-pm-placeholder] { color: #6e7681; }
"#;

thread_local! {
    /// Whether [`DEFAULT_EDITOR_CSS`] has already been injected on this thread.
    /// The stylesheet is global and idempotent, so one injection per UI thread
    /// (one document) suffices; the guard stops every mounted editor from adding
    /// its own duplicate copy.
    static INJECTED: Cell<bool> = const { Cell::new(false) };
}

/// Inject [`DEFAULT_EDITOR_CSS`] into the document `<body>` as a `<style>` element,
/// once per thread. A no-op if the document is gone or the styles are already
/// present. Called by [`RinchDomEditorView`](super::view::RinchDomEditorView) on
/// construction so consumers never have to wire the stylesheet by hand.
pub(crate) fn ensure_default_styles(doc: &Weak<RefCell<dyn DomDocument>>) {
    if INJECTED.with(|f| f.get()) {
        return;
    }
    let Some(doc) = doc.upgrade() else {
        return;
    };
    let mut d = doc.borrow_mut();
    let body = d.body();
    let style = d.create_element("style");
    d.set_attribute(style, "data-rinch-editor-css", "true");
    let text = d.create_text(DEFAULT_EDITOR_CSS);
    d.append_child(style, text);
    d.append_child(body, style);
    INJECTED.with(|f| f.set(true));
}
