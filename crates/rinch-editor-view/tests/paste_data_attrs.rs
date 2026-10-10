//! The default paste keeps an image's `data-*` attributes only when rinch's own
//! copy-out wrote them (review of #1518): a copy between two rinch editors
//! keeps an app's attributes with no plugin, another application's
//! (`data-v-…`, `data-src`, `data-reactid`) are dropped, and a plugin's
//! `handle_paste` still sees the pasted markup whole.

use rinch_editor_core::{EditorState, Node, PasteContent, Plugin, PluginKey, Pos, Selection};
use rinch_editor_view::{EditorHandle, create_editor};
use std::cell::RefCell;
use std::rc::Rc;

fn image_of(h: &EditorHandle) -> Node {
    fn find(n: &Node) -> Option<Node> {
        if n.type_name() == "image" {
            return Some(n.clone());
        }
        n.content().children().iter().find_map(find)
    }
    find(&h.doc()).expect("an image")
}

fn data_attrs(h: &EditorHandle) -> Vec<(String, String)> {
    image_of(h)
        .attrs()
        .iter()
        .filter(|(k, _)| k.starts_with("data-"))
        .map(|(k, v)| (k.to_string(), v.as_str().unwrap_or("").to_string()))
        .collect()
}

fn empty_editor() -> EditorHandle {
    let h = create_editor();
    assert!(h.load_html("<p>z</p>"));
    h.set_selection(Selection::cursor(Pos(2)));
    h
}

#[test]
fn a_copy_between_rinch_editors_keeps_an_images_data_attrs() {
    let from = create_editor();
    assert!(from.load_html(r#"<p>a<img src="a.png" data-ref="r1" data-annotation-id="n7">b</p>"#));
    from.set_selection(Selection::text(Pos(1), Pos(4)));
    let (html, text) = from.selection_clipboard().expect("a selection");
    let to = empty_editor();
    assert!(to.paste(&PasteContent::new(Some(text), Some(html))));
    assert_eq!(
        data_attrs(&to),
        [
            ("data-annotation-id".to_string(), "n7".to_string()),
            ("data-ref".to_string(), "r1".to_string())
        ]
    );
}

#[test]
fn another_applications_data_attrs_are_not_pasted() {
    const FOREIGN: &str = r#"<p><img src="a.png" data-v-7ba5bd90="" data-reactid=".0" data-src="lazy.png" data-stringify-type="emoji"></p>"#;
    let to = empty_editor();
    assert!(to.paste(&PasteContent::new(None, Some(FOREIGN.into()))));
    assert_eq!(data_attrs(&to), Vec::<(String, String)>::new());
    // A load of the same markup keeps them: it is the app's document.
    let loaded = create_editor();
    assert!(loaded.load_html(FOREIGN));
    assert_eq!(data_attrs(&loaded).len(), 4);
}

/// Sees every paste, claims none.
struct Watches(Rc<RefCell<Vec<String>>>);

impl Plugin for Watches {
    fn key(&self) -> PluginKey {
        PluginKey("test.watches")
    }
    fn handle_paste(
        &self,
        _state: &EditorState,
        paste: &PasteContent,
    ) -> Option<rinch_editor_core::Transaction> {
        self.0.borrow_mut().extend(paste.html.clone());
        None
    }
}

#[test]
fn a_plugin_sees_the_pasted_markup_whole() {
    const FOREIGN: &str = r#"<p><img src="a.png" data-src="lazy.png"></p>"#;
    let seen = Rc::new(RefCell::new(Vec::new()));
    let to = empty_editor();
    assert!(to.add_plugin(Rc::new(Watches(seen.clone()))));
    assert!(to.paste(&PasteContent::new(None, Some(FOREIGN.into()))));
    assert_eq!(*seen.borrow(), [FOREIGN.to_string()]);
    // Unclaimed, the default dropped it.
    assert_eq!(data_attrs(&to), Vec::<(String, String)>::new());
}

/// An app inserting its own markup through `replace_selection_with_html` keeps
/// its data attributes by marking the element as rinch's copy-out does
/// (`CLIPBOARD_MARK`); unmarked, they are dropped like any paste's.
#[test]
fn an_apps_own_html_keeps_its_data_attrs_with_the_mark() {
    use rinch_editor_core::serialize::CLIPBOARD_MARK;
    let h = empty_editor();
    assert!(h.replace_selection_with_html(r#"<img src="a.png" data-ref="mine">"#));
    assert_eq!(data_attrs(&h), Vec::<(String, String)>::new());
    let h = empty_editor();
    assert!(h.replace_selection_with_html(&format!(
        r#"<img src="a.png" data-ref="mine" {CLIPBOARD_MARK}="">"#
    )));
    assert_eq!(
        data_attrs(&h),
        [("data-ref".to_string(), "mine".to_string())]
    );
}
