//! `EditorHandle::on_image_input`: a picture pasted into or dropped on an editor
//! is offered to the app, which stores the bytes and answers with the `src` the
//! document carries. The platform runtime calls `offer_image_input` with the
//! selection already where the picture goes; these pin what the handle does
//! from there, on the model alone (no window, no clipboard).

use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::{Pos, Selection};
use rinch_editor_view::{
    EditorHandle, ImageInput, ImageInputSource, SelectionAnchor, create_editor,
};
use std::cell::RefCell;
use std::rc::Rc;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n-not-really-a-png-";

fn editor(html: &str, caret: usize) -> EditorHandle {
    let handle = create_editor();
    assert!(handle.load_html(html));
    handle.set_selection(Selection::cursor(Pos(caret)));
    handle
}

fn html(handle: &EditorHandle) -> String {
    node_to_html(&handle.doc())
}

fn offer(handle: &EditorHandle) -> bool {
    handle.offer_image_input(ImageInputSource::Paste, PNG.to_vec(), "image/png", None)
}

#[test]
fn an_answer_at_once_inserts_the_image_where_the_picture_was_aimed() {
    let handle = editor("<p>ab</p>", 2); // a|b
    let seen = Rc::new(RefCell::new(None));
    let seen_in = seen.clone();
    handle.on_image_input(move |input: ImageInput| {
        *seen_in.borrow_mut() = Some((
            input.source,
            input.bytes.clone(),
            input.mime.clone(),
            input.name.clone(),
        ));
        Some(("app-blob:store/1".to_string(), "a cat".to_string()))
    });
    assert!(handle.has_image_input_callback());

    assert!(handle.offer_image_input(
        ImageInputSource::Drop,
        PNG.to_vec(),
        "image/png",
        Some("cat.png".to_string()),
    ));
    assert_eq!(
        *seen.borrow(),
        Some((
            ImageInputSource::Drop,
            PNG.to_vec(),
            "image/png".to_string(),
            Some("cat.png".to_string())
        )),
        "the app is handed the bytes, their type, the name and how they arrived"
    );
    assert_eq!(
        html(&handle),
        r#"<p>a<img alt="a cat" src="app-blob:store/1">b</p>"#
    );
    assert_eq!(
        handle.selection(),
        Selection::cursor(Pos(3)),
        "the caret is after the image"
    );
}

#[test]
fn a_refusal_inserts_nothing_and_no_data_url_is_made() {
    let handle = editor("<p>ab</p>", 2);
    handle.on_image_input(|_| None);
    assert!(!offer(&handle));
    assert_eq!(html(&handle), "<p>ab</p>");
}

#[test]
fn with_no_callback_nothing_is_offered() {
    let handle = editor("<p>ab</p>", 2);
    assert!(!handle.has_image_input_callback());
    assert!(!offer(&handle));
    assert_eq!(html(&handle), "<p>ab</p>");
}

#[test]
fn an_answer_that_comes_later_lands_where_the_picture_was_aimed_not_at_the_live_caret() {
    let handle = editor("<p>hello world</p>", 6); // hello| world
    let kept: Rc<RefCell<Option<SelectionAnchor>>> = Rc::new(RefCell::new(None));
    let kept_in = kept.clone();
    handle.on_image_input(move |input| {
        *kept_in.borrow_mut() = Some(input.anchor);
        None // storing the bytes is a round trip; the answer comes later
    });
    assert!(!offer(&handle));
    assert_eq!(html(&handle), "<p>hello world</p>", "nothing yet");

    // The person types on, before the place and then somewhere else entirely.
    handle.set_selection(Selection::cursor(Pos(1)));
    assert!(handle.insert_text(">> "));
    handle.set_selection(Selection::cursor(Pos(15))); // the end

    let anchor = kept.borrow_mut().take().expect("the app kept the anchor");
    assert!(handle.insert_image_at(&anchor, "app-blob:store/2", ""));
    assert_eq!(
        html(&handle),
        r#"<p>&gt;&gt; hello<img src="app-blob:store/2"> world</p>"#
    );
}

#[test]
fn a_callback_that_moves_the_selection_does_not_move_the_picture() {
    let handle = editor("<p>ab</p>", 2);
    let inner = handle.clone();
    handle.on_image_input(move |_| {
        // Re-entering the handle is allowed: no borrow is held.
        inner.set_selection(Selection::cursor(Pos(3)));
        Some(("app-blob:store/3".to_string(), String::new()))
    });
    assert!(offer(&handle));
    assert_eq!(html(&handle), r#"<p>a<img src="app-blob:store/3">b</p>"#);
}

#[test]
fn a_read_only_editor_is_offered_nothing_and_refuses_a_late_insert() {
    let handle = editor("<p>ab</p>", 2);
    let calls = Rc::new(RefCell::new(0));
    let calls_in = calls.clone();
    handle.on_image_input(move |_| {
        *calls_in.borrow_mut() += 1;
        Some(("app-blob:store/4".to_string(), String::new()))
    });
    let anchor = handle.anchor_selection();
    handle.set_read_only(true);
    assert!(!offer(&handle));
    assert_eq!(
        *calls.borrow(),
        0,
        "the app is not asked to store a picture for nothing"
    );
    assert!(!handle.insert_image_at(&anchor, "app-blob:store/4", ""));
    assert_eq!(html(&handle), "<p>ab</p>");

    handle.set_read_only(false);
    assert!(offer(&handle));
    assert_eq!(*calls.borrow(), 1);
}

#[test]
fn an_anchor_whose_document_was_replaced_or_that_is_another_editors_inserts_nothing() {
    let handle = editor("<p>ab</p>", 2);
    let anchor = handle.anchor_selection();
    assert!(handle.load_html("<p>another note</p>"));
    assert!(
        !handle.insert_image_at(&anchor, "app-blob:store/5", ""),
        "the note the picture was aimed at is gone"
    );
    assert_eq!(html(&handle), "<p>another note</p>");

    let other = editor("<p>cd</p>", 2);
    let theirs = other.anchor_selection();
    assert!(!handle.insert_image_at(&theirs, "app-blob:store/5", ""));
    assert_eq!(html(&handle), "<p>another note</p>");
    assert_eq!(html(&other), "<p>cd</p>");
}

#[test]
fn an_inserted_image_is_one_undo_step_and_fires_on_change() {
    let handle = editor("<p>ab</p>", 2);
    let changes = Rc::new(RefCell::new(0));
    let changes_in = changes.clone();
    handle.on_change(move || *changes_in.borrow_mut() += 1);
    handle.on_image_input(|_| Some(("app-blob:store/6".to_string(), String::new())));
    assert!(offer(&handle));
    assert_eq!(*changes.borrow(), 1);
    assert!(handle.command("undo"));
    assert_eq!(html(&handle), "<p>ab</p>");
}

#[test]
fn html_that_is_pictures_and_nothing_else_is_told_from_html_with_words() {
    let handle = create_editor();
    // What browsers put on the clipboard for "Copy image".
    assert!(handle.html_is_only_images(
        r#"<meta charset='utf-8'><img src="https://example.com/cat.png" alt="cat"/>"#
    ));
    assert!(handle.html_is_only_images(
        "<html><body>\n<!--StartFragment--><img src=\"a.png\"><!--EndFragment-->\n</body></html>"
    ));
    assert!(handle.html_is_only_images(r#"<p><img src="a.png"><img src="b.png"></p>"#));
    // A linked picture is still only a picture.
    assert!(handle.html_is_only_images(r#"<a href="https://example.com"><img src="a.png"></a>"#));

    assert!(!handle.html_is_only_images(r#"<p>look: <img src="a.png"></p>"#));
    assert!(!handle.html_is_only_images(r#"<p><img src="a.png"></p><p>caption</p>"#));
    assert!(!handle.html_is_only_images("<p>words</p>"));
    assert!(!handle.html_is_only_images(r#"<p><img src="a.png"></p><hr>"#));
    assert!(!handle.html_is_only_images(""));
}
