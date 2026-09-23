//! `Plugin::handle_paste` and `EditorState::handle_paste`: an app sees a paste
//! and rewrites it. The fixture is the case the hook was built for: plain text
//! that is a URL, pasted over selected words, links them instead of replacing
//! them; pasted at a caret, it inserts linked text the plugin chooses.

use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::*;
use std::cell::Cell;
use std::rc::Rc;

fn sk() -> Rc<Schema> {
    Rc::new(Schema::starter_kit())
}

fn doc_of(s: &Schema, text: &str) -> Node {
    let p = s
        .branch("paragraph", Fragment::from_node(s.text(text).unwrap()))
        .unwrap();
    s.branch("doc", Fragment::from_node(p)).unwrap()
}

fn state_with(text: &str, plugins: Vec<Rc<dyn Plugin>>) -> EditorState {
    let s = sk();
    let doc = doc_of(&s, text);
    EditorState::create(s, doc, plugins)
}

fn with_default(extra: Vec<Rc<dyn Plugin>>) -> Vec<Rc<dyn Plugin>> {
    let mut plugins = default_plugins();
    plugins.extend(extra);
    plugins
}

fn is_url(text: &str) -> bool {
    ["https://", "http://", "pimble:"]
        .iter()
        .any(|scheme| text.starts_with(scheme))
        && !text.contains(char::is_whitespace)
}

/// Links a pasted URL: over a selection it marks the selected words, at a
/// caret it inserts linked text (a note's title for a `pimble:` URL, the URL
/// itself for a web one). Anything else is left to the default.
struct Linker;

impl Plugin for Linker {
    fn key(&self) -> PluginKey {
        PluginKey("test.linker")
    }

    fn handle_paste(&self, state: &EditorState, paste: &PasteContent) -> Option<Transaction> {
        let url = paste.text.as_deref().map(str::trim).filter(|t| is_url(t))?;
        let link = Mark::new(
            state.schema().mark_type("link")?.clone(),
            Attrs::from_iter([("href", AttrValue::from(url.to_string()))]),
        );
        let (from, to) = (state.selection.from().0, state.selection.to().0);
        let mut tr = state.tr();
        if from < to {
            tr.add_mark(from, to, link).ok()?;
        } else {
            let label = if url.starts_with("pimble:") {
                "Linked note"
            } else {
                url
            };
            let text = state.schema().text_with_marks(label, vec![link]).ok()?;
            let len = text.text_len();
            tr.replace_with(from, from, Fragment::from_node(text))
                .ok()?;
            tr.set_selection(Selection::cursor(Pos(from + len)));
        }
        Some(tr)
    }
}

/// Claims every paste by inserting `self.0`, and counts the times it was asked.
struct Inserts(&'static str, &'static str, Rc<Cell<u32>>);

impl Plugin for Inserts {
    fn key(&self) -> PluginKey {
        PluginKey(self.0)
    }

    fn handle_paste(&self, state: &EditorState, _paste: &PasteContent) -> Option<Transaction> {
        self.2.set(self.2.get() + 1);
        let mut tr = state.tr();
        tr.insert_text(self.1).ok()?;
        Some(tr)
    }
}

/// Claims nothing, and counts the times it was asked.
struct Declines(Rc<Cell<u32>>);

impl Plugin for Declines {
    fn key(&self) -> PluginKey {
        PluginKey("test.declines")
    }

    fn handle_paste(&self, _state: &EditorState, _paste: &PasteContent) -> Option<Transaction> {
        self.0.set(self.0.get() + 1);
        None
    }
}

fn html_after(state: &EditorState, paste: &PasteContent) -> Option<String> {
    let tr = state.handle_paste(paste)?;
    Some(node_to_html(&state.apply(tr).doc))
}

#[test]
fn a_url_pasted_over_a_selection_links_the_selected_words() {
    let mut state = state_with("see the docs here", with_default(vec![Rc::new(Linker)]));
    // "the docs" is chars 4..12, positions 5..13 inside the paragraph.
    state.selection = Selection::text(Pos(5), Pos(13));
    let html = html_after(&state, &PasteContent::text("https://example.test/docs")).unwrap();
    assert_eq!(
        html,
        "<p>see <a href=\"https://example.test/docs\">the docs</a> here</p>"
    );
}

#[test]
fn a_url_pasted_at_a_caret_inserts_the_linked_text_the_plugin_chooses() {
    let mut state = state_with("ab", with_default(vec![Rc::new(Linker)]));
    state.selection = Selection::cursor(Pos(2));
    let html = html_after(&state, &PasteContent::text("pimble:1234/5678")).unwrap();
    assert_eq!(
        html,
        "<p>a<a href=\"pimble:1234/5678\">Linked note</a>b</p>"
    );

    let html = html_after(&state, &PasteContent::text("https://example.test/")).unwrap();
    assert_eq!(
        html,
        "<p>a<a href=\"https://example.test/\">https://example.test/</a>b</p>"
    );
}

/// The text decides, and html beside it does not hide it: a browser's copy of
/// a link offers both flavours.
#[test]
fn the_text_beside_html_is_what_the_plugin_reads() {
    let mut state = state_with("see the docs", with_default(vec![Rc::new(Linker)]));
    state.selection = Selection::text(Pos(5), Pos(13));
    let paste = PasteContent::new(
        Some("https://example.test/".into()),
        Some("<a href=\"https://example.test/\">Example</a>".into()),
    );
    assert_eq!(
        html_after(&state, &paste).unwrap(),
        "<p>see <a href=\"https://example.test/\">the docs</a></p>"
    );
}

#[test]
fn a_paste_no_plugin_claims_falls_through_to_the_default() {
    let mut state = state_with("see the docs", with_default(vec![Rc::new(Linker)]));
    state.selection = Selection::text(Pos(5), Pos(13));
    assert!(
        state
            .handle_paste(&PasteContent::text("not a url"))
            .is_none()
    );
    assert!(
        state
            .handle_paste(&PasteContent::text("https://two words.test/"))
            .is_none()
    );
    assert!(
        state
            .handle_paste(&PasteContent::html("<a href=\"https://x.test/\">x</a>"))
            .is_none(),
        "html alone: the fixture reads only the text"
    );
}

#[test]
fn the_built_in_plugins_claim_no_paste() {
    let state = state_with("ab", default_plugins());
    for paste in [
        PasteContent::text("https://example.test/"),
        PasteContent::html("<p><b>x</b></p>"),
        PasteContent::new(Some("x".into()), Some("<p>x</p>".into())),
    ] {
        assert!(state.handle_paste(&paste).is_none(), "{paste:?}");
    }
}

#[test]
fn the_first_plugin_to_claim_wins_and_later_ones_are_not_asked() {
    let (a, b, before) = (
        Rc::new(Cell::new(0)),
        Rc::new(Cell::new(0)),
        Rc::new(Cell::new(0)),
    );
    let first = || Rc::new(Inserts("test.first", "FIRST", a.clone())) as Rc<dyn Plugin>;
    let second = || Rc::new(Inserts("test.second", "SECOND", b.clone())) as Rc<dyn Plugin>;
    let declines = || Rc::new(Declines(before.clone())) as Rc<dyn Plugin>;

    let state = state_with("", with_default(vec![declines(), first(), second()]));
    assert_eq!(
        html_after(&state, &PasteContent::text("x")).unwrap(),
        "<p>FIRST</p>"
    );
    assert_eq!(
        (before.get(), a.get(), b.get()),
        (1, 1, 0),
        "a declining plugin is asked and passes it on; the claimer's successor is not asked"
    );

    let state = state_with("", with_default(vec![second(), first()]));
    assert_eq!(
        html_after(&state, &PasteContent::text("x")).unwrap(),
        "<p>SECOND</p>",
        "plugin order, not the plugin, decides"
    );
}

#[test]
fn an_empty_paste_asks_no_plugin() {
    let asked = Rc::new(Cell::new(0));
    let state = state_with("ab", vec![Rc::new(Declines(asked.clone()))]);
    assert!(state.handle_paste(&PasteContent::default()).is_none());
    assert!(
        state
            .handle_paste(&PasteContent::new(Some(String::new()), Some(" \n ".into())))
            .is_none()
    );
    assert_eq!(asked.get(), 0);
}

#[test]
fn paste_content_takes_empty_flavours_as_absent() {
    assert!(PasteContent::new(Some(String::new()), Some("  \n".into())).is_empty());
    assert_eq!(PasteContent::text(""), PasteContent::default());
    let p = PasteContent::new(Some(" x ".into()), Some(" <p>x</p> ".into()));
    assert_eq!(p.text.as_deref(), Some(" x "), "text is not trimmed");
    assert_eq!(p.html.as_deref(), Some(" <p>x</p> "), "nor is html");
}
