//! The editor's configurability and Chrome-default fixes (#2210), against the
//! mock host: markdown shortcuts opt-in, `remove_plugin`, a live placeholder,
//! no inline block styles at their defaults, and overlay colours an app can
//! name through `--rinch-editor-*` variables.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::{DomDocument, NodeHandle, NodeId};
use rinch_editor_core::{
    HistoryPlugin, InputRule, Plugin, PluginKey, Pos, Selection, plugins::PLACEHOLDER_KEY,
};

use crate::{EditorHandle, TabBehavior, create_editor};

struct Host {
    doc: Rc<RefCell<dyn DomDocument>>,
    container_id: NodeId,
}

/// `handle` projected into a fresh mock container.
fn attach(handle: &EditorHandle) -> Host {
    let mock = Rc::new(RefCell::new(MockDomDocument::new()));
    let doc: Rc<RefCell<dyn DomDocument>> = mock;
    let container_id = doc.borrow_mut().create_element("div");
    let container = NodeHandle::new(container_id, Rc::downgrade(&doc));
    handle.attach(container, Rc::downgrade(&doc));
    Host { doc, container_id }
}

/// A fresh editor with the caret at the end of `html`'s first paragraph.
fn editor_with(html: &str) -> EditorHandle {
    let h = create_editor();
    h.load_html(html);
    let end = h.doc().child(0).content_size() + 1;
    h.set_selection(Selection::cursor(Pos(end)));
    h
}

/// The first block's text (its text children joined).
fn first_text(h: &EditorHandle) -> String {
    let doc = h.doc();
    let block = doc.child(0);
    (0..block.child_count())
        .filter_map(|i| block.child(i).text().map(str::to_string))
        .collect()
}

fn first_block_type(h: &EditorHandle) -> String {
    h.doc().child(0).type_name().to_string()
}

fn placeholder_texts(host: &Host) -> Vec<String> {
    let doc = host.doc.borrow();
    doc.get_children(host.container_id)
        .into_iter()
        .filter(|&id| doc.get_attribute(id, "data-pm-placeholder").is_some())
        .map(|id| doc.text_content(id).unwrap_or_default())
        .collect()
}

fn style_of(host: &Host, id: NodeId) -> String {
    host.doc
        .borrow()
        .get_attribute(id, "style")
        .unwrap_or_default()
}

/// The first block's host element.
fn first_block(host: &Host) -> NodeId {
    let doc = host.doc.borrow();
    doc.get_children(host.container_id)
        .into_iter()
        .find(|&id| doc.get_attribute(id, "data-pm-type").is_some())
        .expect("a block")
}

// ── Markdown shortcuts ──────────────────────────────────────────────────────

/// A fresh editor inserts what is typed literally, as a Chrome
/// `contenteditable` does: `# ` stays text, `*x*` stays asterisks.
#[test]
fn a_fresh_editor_has_no_markdown_shortcuts() {
    let h = editor_with("<p></p>");
    assert!(!h.markdown_shortcuts());
    assert!(h.insert_text("#"));
    assert!(h.insert_text(" "));
    assert_eq!(first_block_type(&h), "paragraph", "`# ` is not a heading");
    assert_eq!(first_text(&h), "# ");
}

/// Turned on, the same keystrokes make a heading; turned off again, they do
/// not. The control for the test above: the rule exists and fires.
#[test]
fn set_markdown_shortcuts_turns_the_rules_on_and_off() {
    let h = editor_with("<p></p>");
    assert!(h.set_markdown_shortcuts(true));
    assert!(h.markdown_shortcuts());
    h.insert_text("#");
    h.insert_text(" ");
    assert_eq!(first_block_type(&h), "heading", "`# ` is a heading");

    let h = editor_with("<p></p>");
    h.set_markdown_shortcuts(true);
    assert!(!h.set_markdown_shortcuts(false));
    h.insert_text("#");
    h.insert_text(" ");
    assert_eq!(first_block_type(&h), "paragraph");
}

/// An app plugin with one input rule, `^zz$`, which declines every match.
#[derive(Debug)]
struct AppRules;

impl Plugin for AppRules {
    fn key(&self) -> PluginKey {
        PluginKey("app-rules-2210")
    }
    fn input_rules(&self) -> Vec<InputRule> {
        vec![InputRule::new("^zz$", |_, _, _, _| None)]
    }
}

/// Turned on later, the shortcuts take their `default_plugins` place, before
/// the rules of a plugin the app added (#2210), not after them.
#[test]
fn set_markdown_shortcuts_installs_them_at_their_default_place() {
    let h = editor_with("<p></p>");
    assert!(h.add_plugin(Rc::new(AppRules)));
    assert!(h.set_markdown_shortcuts(true));
    let state = h.state();
    let patterns: Vec<&str> = state
        .input_rules()
        .iter()
        .map(|r| r.pattern.as_str())
        .collect();
    assert!(patterns.len() > 1, "{patterns:?}");
    assert_eq!(
        patterns.last(),
        Some(&"^zz$"),
        "the app's rule stays last: {patterns:?}"
    );
}

// ── remove_plugin ───────────────────────────────────────────────────────────

/// The history can be taken out (an outer undo stack owns it) and put back
/// with other settings; a key that is not installed is refused.
#[test]
fn remove_plugin_takes_the_history_out_and_a_new_one_goes_in() {
    let h = editor_with("<p>a</p>");
    let key = HistoryPlugin::new().key();
    assert!(h.remove_plugin(key));
    assert!(!h.remove_plugin(key), "no longer installed");
    h.insert_text("b");
    assert!(!h.command("undo"), "no history, no undo");
    assert_eq!(first_text(&h), "ab");

    assert!(h.add_plugin(Rc::new(HistoryPlugin::with(0, 10))));
    h.insert_text("c");
    assert!(h.command("undo"));
    assert_eq!(first_text(&h), "ab");
}

// ── Placeholder ─────────────────────────────────────────────────────────────

/// Shown while empty, changed live, cleared by empty text — and changing it
/// is not an edit: the undo history survives.
#[test]
fn the_placeholder_is_live_and_changing_it_keeps_the_history() {
    let h = editor_with("<p></p>");
    assert!(h.set_placeholder("Write a comment"));
    let host = attach(&h);
    assert_eq!(placeholder_texts(&host), ["Write a comment"]);

    // An edit to undo later, then back to empty.
    h.insert_text("x");
    assert!(placeholder_texts(&host).is_empty(), "hidden once not empty");
    assert!(h.set_placeholder("Reply…"), "changed while hidden");
    assert!(h.command("undo"), "the history survived the change");
    assert_eq!(placeholder_texts(&host), ["Reply…"], "the new text shows");

    assert!(h.set_placeholder("Antworten…"));
    assert_eq!(
        placeholder_texts(&host),
        ["Antworten…"],
        "changed while shown"
    );
    assert_eq!(h.placeholder(), "Antworten…");

    assert!(h.set_placeholder(""));
    assert!(placeholder_texts(&host).is_empty(), "empty text shows none");
    assert!(h.set_placeholder("Back"));
    assert_eq!(placeholder_texts(&host), ["Back"]);
}

/// `remove_plugin(PLACEHOLDER_KEY)` takes the placeholder away for good: the
/// handle forgets it, and a later `set_placeholder` installs a new one that
/// shows (#2210), rather than changing the text of a plugin no state holds.
#[test]
fn removing_the_placeholder_plugin_forgets_it() {
    let h = editor_with("<p></p>");
    assert!(h.set_placeholder("First"));
    let host = attach(&h);
    assert!(h.remove_plugin(PLACEHOLDER_KEY));
    assert!(placeholder_texts(&host).is_empty(), "removed");
    assert_eq!(h.placeholder(), "", "the handle forgot it");

    assert!(h.set_placeholder("Second"));
    assert_eq!(placeholder_texts(&host), ["Second"], "installed again");
}

// ── Inline block styles ─────────────────────────────────────────────────────

/// A block at the default alignment and indent carries no inline
/// `text-align` / `margin-left`, so the app's stylesheet (and RTL `start`)
/// decides them; a command writes them, and going back takes them off. An
/// explicit align-left is written as `text-align: left`, so it holds in a
/// `th` and an RTL editor; the Paragraph reset unsets it.
#[test]
fn default_alignment_and_indent_write_no_inline_style() {
    let h = editor_with("<p>abc</p>");
    let host = attach(&h);
    let p = first_block(&host);
    let style = style_of(&host, p);
    assert!(!style.contains("text-align"), "default align: {style:?}");
    assert!(!style.contains("margin-left"), "default indent: {style:?}");

    assert!(h.command("setTextAlignCenter"));
    assert!(h.command("indent"));
    let style = style_of(&host, first_block(&host));
    assert!(style.contains("text-align: center"), "{style:?}");
    assert!(
        style.contains("margin-left: calc(var(--rinch-editor-indent-step, 2em) * 1)"),
        "{style:?}"
    );

    assert!(h.command("setTextAlignLeft"));
    assert!(h.command("outdent"));
    let style = style_of(&host, first_block(&host));
    assert!(
        style.contains("text-align: left"),
        "explicit left: {style:?}"
    );
    assert!(!style.contains("margin-left"), "back to default: {style:?}");

    assert!(h.command("setParagraph"));
    let style = style_of(&host, first_block(&host));
    assert!(!style.contains("text-align"), "unset again: {style:?}");
}

/// An explicit `left` loaded from HTML projects as `text-align: left`
/// (#2210); a fresh block's unset alignment projects as nothing.
#[test]
fn an_explicit_left_from_html_is_written_inline() {
    let h = editor_with(r#"<p style="text-align:left">abc</p>"#);
    let host = attach(&h);
    let style = style_of(&host, first_block(&host));
    assert!(style.contains("text-align: left"), "{style:?}");

    assert!(h.command("setTextAlignCenter"));
    assert!(h.command("setTextAlignLeft"));
    let style = style_of(&host, first_block(&host));
    assert!(style.contains("text-align: left"), "{style:?}");
}

/// The default stylesheet start-aligns a header cell (#2210), so the UA
/// sheet's `th { text-align: center }` does not reach its paragraphs.
#[test]
fn the_default_stylesheet_start_aligns_header_cells() {
    let css = crate::styles::DEFAULT_EDITOR_CSS;
    assert!(
        css.lines()
            .any(|l| l.trim() == "[data-pm-editor] th { text-align: start; }"),
        "a `th` text-align rule"
    );
}

// ── Overlay colours ─────────────────────────────────────────────────────────

/// The caret, the selection and the node outline are written as `var()`s an
/// app sets; the caret falls back to the text colour, as Chrome's
/// `caret-color: auto`.
#[test]
fn overlay_colours_are_variables_with_chrome_like_fallbacks() {
    assert_eq!(
        crate::view::CARET_COLOR,
        "var(--rinch-editor-caret-color, currentColor)"
    );
    assert_eq!(
        crate::view::PREEDIT_UNDERLINE,
        "var(--rinch-editor-preedit-underline, 1px solid currentColor)"
    );
    assert!(
        crate::view::PREEDIT_BG.contains("var(--rinch-editor-default-bg"),
        "the composition takes the editor's (scheme-dependent) background"
    );
}

// ── Tab ─────────────────────────────────────────────────────────────────────

/// `tab_key`: a plain paragraph has no use for Tab; a list item that can
/// sink does, unless `MoveFocus`; a read-only editor never does.
#[test]
fn tab_key_reports_whether_the_editor_used_tab() {
    let html = "<p>plain</p><ul><li><p>one</p></li><li><p>two</p></li></ul>";
    let h = editor_with(html);
    h.set_selection(Selection::cursor(Pos(3)));
    assert!(!h.tab_key(false), "plain paragraph");
    assert!(!h.tab_key(true), "plain paragraph, Shift");

    h.set_selection(Selection::cursor(Pos(18)));
    h.set_tab_behavior(TabBehavior::MoveFocus);
    let doc = h.doc();
    assert!(!h.tab_key(false), "MoveFocus");
    assert!(h.doc().same_ref(&doc));

    h.set_tab_behavior(TabBehavior::IndentInListsAndTables);
    h.set_read_only(true);
    assert!(!h.tab_key(false), "read-only");
    h.set_read_only(false);
    assert!(h.tab_key(false), "the second item sinks");
    assert!(!h.doc().same_ref(&doc));
}
