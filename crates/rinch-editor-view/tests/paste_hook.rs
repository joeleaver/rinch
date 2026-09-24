//! `EditorHandle::paste`: the one paste entry point desktop and web share. The
//! plugins see the paste first (`Plugin::handle_paste`); what none claims gets
//! the default (html, else text). The claim is one edit like any other: one
//! undo step, refused by a read-only editor, `on_change`, and recorded onto a
//! collaboration session.

use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::{
    AttrValue, Attrs, EditorState, Fragment, Mark, PasteContent, Plugin, PluginKey, Pos, Selection,
    Transaction,
};
use rinch_editor_view::{EditorHandle, create_editor};
use std::cell::Cell;
use std::rc::Rc;

fn is_url(text: &str) -> bool {
    ["https://", "http://", "pimble:"]
        .iter()
        .any(|scheme| text.starts_with(scheme))
        && !text.contains(char::is_whitespace)
}

/// A pasted URL links the selected words, or at a caret inserts linked text
/// (a note's title for `pimble:`, the URL for the web). Else: the default.
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

/// Claims every paste and does nothing with it.
struct Swallows;

impl Plugin for Swallows {
    fn key(&self) -> PluginKey {
        PluginKey("test.swallows")
    }

    fn handle_paste(&self, state: &EditorState, _paste: &PasteContent) -> Option<Transaction> {
        Some(state.tr())
    }
}

fn editor(html: &str) -> EditorHandle {
    let h = create_editor();
    assert!(h.add_plugin(Rc::new(Linker)));
    assert!(h.load_html(html));
    h
}

fn html(h: &EditorHandle) -> String {
    node_to_html(&h.doc())
}

fn counter(h: &EditorHandle) -> Rc<Cell<u32>> {
    let n = Rc::new(Cell::new(0));
    let m = n.clone();
    h.on_change(move || m.set(m.get() + 1));
    n
}

const LINKED: &str = "<p>see <a href=\"https://example.test/\">the docs</a> here</p>";

#[test]
fn a_url_pasted_over_a_selection_links_it_through_the_handle() {
    let h = editor("<p>see the docs here</p>");
    let changes = counter(&h);
    h.set_selection(Selection::text(Pos(5), Pos(13)));
    assert!(h.paste(&PasteContent::text("https://example.test/")));
    assert_eq!(html(&h), LINKED);
    assert_eq!(changes.get(), 1, "on_change fires once for the paste");
}

#[test]
fn a_url_pasted_at_a_caret_inserts_linked_text_and_puts_the_caret_after_it() {
    let h = editor("<p>ab</p>");
    h.set_selection(Selection::cursor(Pos(2)));
    assert!(h.paste(&PasteContent::text("pimble:1234/5678")));
    assert_eq!(
        html(&h),
        "<p>a<a href=\"pimble:1234/5678\">Linked note</a>b</p>"
    );
    assert_eq!(h.selection(), Selection::cursor(Pos(13)));
}

#[test]
fn what_no_plugin_claims_gets_the_default_html_then_text() {
    let h = editor("<p>ab</p>");
    h.set_selection(Selection::cursor(Pos(2)));
    assert!(h.paste(&PasteContent::new(
        Some("plain".into()),
        Some("<b>RICH</b>".into())
    )));
    assert_eq!(html(&h), "<p>a<strong>RICH</strong>b</p>", "html wins");

    let h = editor("<p>ab</p>");
    h.set_selection(Selection::cursor(Pos(2)));
    assert!(h.paste(&PasteContent::new(
        Some("plain".into()),
        Some("<script>x</script>".into())
    )));
    assert_eq!(
        html(&h),
        "<p>aplainb</p>",
        "html that parses to nothing: the text"
    );

    let h = editor("<p>ab</p>");
    h.set_selection(Selection::cursor(Pos(2)));
    assert!(h.paste(&PasteContent::text("not a url")));
    assert_eq!(html(&h), "<p>anot a urlb</p>");

    assert!(!h.paste(&PasteContent::default()), "nothing to paste");
}

/// Both shapes of the claim go back in one undo, and come back in one redo.
#[test]
fn undo_takes_a_claimed_paste_back_in_one_step() {
    for (selection, paste) in [
        (
            Selection::text(Pos(5), Pos(13)),
            PasteContent::text("https://example.test/"),
        ),
        (Selection::cursor(Pos(5)), PasteContent::text("pimble:1/2")),
    ] {
        let h = editor("<p>see the docs here</p>");
        // Typing first, then a caret move: the paste is its own undo step.
        h.set_selection(Selection::cursor(Pos(18)));
        assert!(h.insert_text("!"));
        h.set_selection(selection.clone());
        let before = html(&h);
        assert!(h.paste(&paste));
        let after = html(&h);
        assert_ne!(before, after);
        assert!(h.command("undo"));
        assert_eq!(html(&h), before, "one undo takes the whole paste back");
        assert!(h.command("redo"));
        assert_eq!(html(&h), after, "one redo puts it back");
        assert!(h.command("undo"));
        assert!(h.command("undo"));
        assert_eq!(
            html(&h),
            "<p>see the docs here</p>",
            "the typing before it is a separate step"
        );
    }
}

/// A read-only editor refuses a claimed paste as it refuses any edit, and an
/// unclaimed one (the default) too.
#[test]
fn a_read_only_editor_refuses_a_claimed_paste_and_the_default() {
    for paste in [
        PasteContent::text("https://example.test/"),
        PasteContent::text("not a url"),
        PasteContent::html("<b>x</b>"),
    ] {
        let h = editor("<p>see the docs here</p>");
        let changes = counter(&h);
        h.set_selection(Selection::text(Pos(5), Pos(13)));
        h.set_read_only(true);
        assert!(!h.paste(&paste), "{paste:?}");
        assert_eq!(html(&h), "<p>see the docs here</p>");
        assert_eq!(changes.get(), 0);
        // Control: the same paste lands once the editor is writable again.
        h.set_read_only(false);
        assert!(h.paste(&paste), "{paste:?}");
    }
}

/// A claim that changes nothing swallows the paste: the default does not run,
/// and nothing is reported as changed.
#[test]
fn a_claim_that_changes_nothing_swallows_the_paste() {
    let h = create_editor();
    assert!(h.add_plugin(Rc::new(Swallows)));
    assert!(h.load_html("<p>ab</p>"));
    let changes = counter(&h);
    assert!(h.paste(&PasteContent::text("x")), "claimed");
    assert_eq!(html(&h), "<p>ab</p>");
    assert_eq!(changes.get(), 0);
}

/// The plugin runs inside the paste's transaction, before the default: added
/// after the built-ins, it still gets the paste the default would have taken.
#[test]
fn a_plugin_added_after_the_built_ins_still_sees_the_paste() {
    let h = create_editor();
    assert!(h.load_html("<p>see the docs here</p>"));
    h.set_selection(Selection::text(Pos(5), Pos(13)));
    assert!(h.paste(&PasteContent::text("https://example.test/")));
    assert_eq!(
        html(&h),
        "<p>see https://example.test/ here</p>",
        "control: without the plugin the URL replaces the words"
    );

    let h = editor("<p>see the docs here</p>");
    h.set_selection(Selection::text(Pos(5), Pos(13)));
    assert!(h.paste(&PasteContent::text("https://example.test/")));
    assert_eq!(html(&h), LINKED);
}

#[cfg(feature = "collaboration")]
mod collab {
    use super::*;

    /// A claimed paste is a local edit like any other: recorded onto the CRDT
    /// and broadcast, so the peer sees the link.
    #[test]
    fn a_claimed_paste_reaches_a_collaborating_peer() {
        let host = editor("<p>see the docs here</p>");
        let guest = create_editor();
        let guest_in = guest.clone();
        let snapshot = host
            .start_collaboration_host(move |delta| {
                guest_in.collab_receive(&delta);
            })
            .expect("host projects its document");
        let host_in = host.clone();
        guest
            .start_collaboration_guest(&snapshot, move |delta| {
                host_in.collab_receive(&delta);
            })
            .expect("guest joins");
        assert_eq!(html(&guest), "<p>see the docs here</p>");

        host.set_selection(Selection::text(Pos(5), Pos(13)));
        assert!(host.paste(&PasteContent::text("https://example.test/")));
        assert_eq!(html(&host), LINKED);
        assert_eq!(html(&guest), LINKED, "the peer got the link");
        assert!(host.collab_take_error().is_none());
    }
}
