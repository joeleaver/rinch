//! What the textblock data attribute tests share.
#![allow(dead_code)]

use rinch_editor_core::commands::set_block_data_attr;
use rinch_editor_core::serialize::{
    node_to_html, slice_from_html, slice_from_pasted_html, slice_from_text, slice_to_html,
};
use rinch_editor_core::{
    Command, DataAttrSplitPlugin, EditorState, Pos, Schema, Selection, default_plugins,
};
use std::rc::Rc;

/// An editor over `html` (loaded, so every data attribute is read) with the
/// caret at `at`.
pub fn state(html: &str, at: usize) -> EditorState {
    state_with(html, at, DataAttrSplitPlugin::new())
}

/// [`state`] with an app's choices for what a split does.
pub fn state_with(html: &str, at: usize, splits: DataAttrSplitPlugin) -> EditorState {
    let schema = Rc::new(Schema::starter_kit());
    let slice = slice_from_html(&schema, html).unwrap();
    let doc = schema.branch("doc", slice.content).unwrap();
    let mut plugins = default_plugins();
    plugins.push(Rc::new(splits));
    let s = EditorState::create(schema, doc, plugins);
    select(s, at, at)
}

pub fn select(s: EditorState, from: usize, to: usize) -> EditorState {
    let mut tr = s.tr();
    tr.set_selection(Selection::text(Pos(from), Pos(to)));
    s.apply(tr)
}

pub fn run(s: &EditorState, cmd: Command) -> EditorState {
    let mut out = None;
    let mut dispatch = |tr| out = Some(s.apply(tr));
    assert!(cmd(s, Some(&mut dispatch)), "the command applies");
    out.expect("a transaction")
}

pub fn applies(s: &EditorState, cmd: Command) -> bool {
    cmd(s, None)
}

/// The document as exported HTML.
pub fn html(s: &EditorState) -> String {
    node_to_html(&s.doc)
}

/// `html` without the `data-role` attributes: the shape of the document.
pub fn shape(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(i) = rest.find(" data-role=\"") {
        out.push_str(&rest[..i]);
        let value = &rest[i + " data-role=\"".len()..];
        rest = &value[value.find('"').expect("a closed value") + 1..];
    }
    out + rest
}

pub fn role(name: &str) -> Command {
    set_block_data_attr("data-role", Some(name))
}

/// Paste the clipboard HTML `clip` over the selection.
pub fn paste(s: &EditorState, clip: &str) -> EditorState {
    let slice = slice_from_pasted_html(s.schema(), clip).unwrap();
    let mut tr = s.tr();
    tr.replace_selection(slice).unwrap();
    s.apply(tr)
}

/// Paste the plain text `text` over the selection.
pub fn paste_text(s: &EditorState, text: &str) -> EditorState {
    let slice = slice_from_text(s.schema(), text).unwrap();
    let mut tr = s.tr();
    tr.replace_selection_with_lines(slice).unwrap();
    s.apply(tr)
}

/// All of `s`'s document, copied.
pub fn copy(s: &EditorState) -> String {
    slice_to_html(&s.doc.slice(0, s.doc.content_size()).unwrap())
}

/// A slice cut from inside the two blocks of [`TWO`], as a selection's copy is.
pub fn open_copy(from: usize, to: usize) -> String {
    slice_to_html(&state(TWO, 1).doc.slice(from, to).unwrap())
}

pub const SUB: &str = r#"<p data-role="sub">abcd</p>"#;

pub const TWO: &str = r#"<p data-role="a">ab</p><p data-role="b">cd</p>"#;
