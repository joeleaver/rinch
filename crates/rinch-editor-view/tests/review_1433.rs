//! Review of PR #1433 (`EditorHandle::on_image_input`): what the PR's own
//! fixtures do not sample. Each test asserts the behaviour an app that answers
//! LATER (an upload) needs; the review report says which are red at the PR head.

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

/// Keep every anchor the app is handed and answer `None` (an upload).
fn keeping_anchors(handle: &EditorHandle) -> Rc<RefCell<Vec<SelectionAnchor>>> {
    let kept: Rc<RefCell<Vec<SelectionAnchor>>> = Rc::default();
    let kept_in = kept.clone();
    handle.on_image_input(move |input: ImageInput| {
        kept_in.borrow_mut().push(input.anchor);
        None
    });
    kept
}

/// R1. The person pastes a picture, goes on typing somewhere else, and the
/// upload finishes: the picture goes where it was aimed, and the person's caret
/// stays where THEY are (the next letter they type must not land beside the
/// picture).
#[test]
fn r1_a_late_answer_does_not_take_the_persons_caret() {
    let handle = editor("<p>hello world</p><p>second</p>", 6); // hello| world
    let kept = keeping_anchors(&handle);
    assert!(!offer(&handle));

    // The person moves to the second paragraph and types there.
    handle.set_selection(Selection::cursor(Pos(20))); // second|
    assert!(handle.insert_text("!"));
    let live = handle.selection();

    let anchor = kept.borrow_mut().remove(0);
    assert!(handle.insert_image_at(&anchor, "app-blob:1", ""));
    assert_eq!(
        html(&handle),
        r#"<p>hello<img src="app-blob:1"> world</p><p>second!</p>"#
    );
    // The image is one position, before the live caret: it shifts by one.
    assert_eq!(
        handle.selection(),
        Selection::cursor(Pos(live.head().0 + 1)),
        "the live caret is still at the end of `second!`"
    );
    assert!(handle.insert_text("?"));
    assert_eq!(
        html(&handle),
        r#"<p>hello<img src="app-blob:1"> world</p><p>second!?</p>"#,
        "typing continues where the person was"
    );
}

/// R2. Several pictures dropped at once, each answered later (in the order they
/// were offered): they land in the order they were dropped.
#[test]
fn r2_several_late_answers_at_one_place_keep_their_order() {
    let handle = editor("<p>ab</p>", 2);
    let kept = keeping_anchors(&handle);
    for _ in 0..3 {
        assert!(!offer(&handle));
    }
    let anchors: Vec<SelectionAnchor> = kept.borrow_mut().drain(..).collect();
    for (i, anchor) in anchors.iter().enumerate() {
        assert!(handle.insert_image_at(anchor, &format!("app-blob:{}", i + 1), ""));
    }
    assert_eq!(
        html(&handle),
        r#"<p>a<img src="app-blob:1"><img src="app-blob:2"><img src="app-blob:3">b</p>"#
    );
}

/// R3. A place that takes no image (a code block): the app is not asked to
/// store a picture that cannot be inserted.
#[test]
fn r3_a_place_that_takes_no_image_does_not_ask_the_app_to_store_one() {
    let handle = editor("<pre><code>let x = 1;</code></pre>", 4);
    let calls = Rc::new(RefCell::new(0));
    let calls_in = calls.clone();
    handle.on_image_input(move |_| {
        *calls_in.borrow_mut() += 1;
        Some(("app-blob:1".to_string(), String::new()))
    });
    let inserted = offer(&handle);
    assert!(
        inserted || *calls.borrow() == 0,
        "the app stored a picture ({} call) and nothing was inserted",
        calls.borrow()
    );
}

/// R4. A late insert a read-only editor refuses changes nothing: not the
/// document and not the selection.
#[test]
fn r4_a_refused_late_insert_leaves_the_selection_alone() {
    let handle = editor("<p>hello world</p>", 6);
    let anchor = handle.anchor_selection();
    handle.set_selection(Selection::cursor(Pos(1)));
    handle.set_read_only(true);
    assert!(!handle.insert_image_at(&anchor, "app-blob:1", ""));
    assert_eq!(handle.selection(), Selection::cursor(Pos(1)));
}

/// R5. A picture pasted OVER a selection and answered later replaces what was
/// selected; this pins that the range is mapped, not that it is right to
/// delete text the person may have retyped.
#[test]
fn r5_a_late_answer_over_a_range_replaces_the_mapped_range() {
    let handle = editor("<p>hello world</p>", 1);
    handle.set_selection(Selection::text(Pos(1), Pos(6))); // [hello]
    let kept = keeping_anchors(&handle);
    assert!(!offer(&handle));
    handle.set_selection(Selection::cursor(Pos(12)));
    assert!(handle.insert_text("!"));
    let anchor = kept.borrow_mut().remove(0);
    assert!(handle.insert_image_at(&anchor, "app-blob:1", ""));
    assert_eq!(html(&handle), r#"<p><img src="app-blob:1"> world!</p>"#);
}

/// R6. One undo takes a late-inserted picture out again and nothing else.
#[test]
fn r6_a_late_insert_is_one_undo_step() {
    let handle = editor("<p>ab</p>", 2);
    let kept = keeping_anchors(&handle);
    assert!(!offer(&handle));
    handle.set_selection(Selection::cursor(Pos(3)));
    assert!(handle.insert_text("X"));
    let anchor = kept.borrow_mut().remove(0);
    assert!(handle.insert_image_at(&anchor, "app-blob:1", ""));
    assert_eq!(html(&handle), r#"<p>a<img src="app-blob:1">bX</p>"#);
    assert!(handle.command("undo"));
    assert_eq!(html(&handle), "<p>abX</p>");
}

/// R7. What `html_is_only_images` says about shapes the PR's fixture leaves out.
#[test]
fn r7_html_is_only_images_edges() {
    let handle = create_editor();
    // Whitespace and a non-breaking space around the picture.
    assert!(handle.html_is_only_images("\n  <img src=\"a.png\">  \n"));
    // A line break after the picture is not words.
    assert!(
        handle.html_is_only_images(r#"<img src="a.png"><br>"#),
        "a trailing <br> (Safari, some mail clients) makes it an html paste"
    );
    // A figure with a caption is words.
    assert!(
        !handle.html_is_only_images(
            r#"<figure><img src="a.png"><figcaption>cat</figcaption></figure>"#
        )
    );
}

#[cfg(feature = "collaboration")]
mod collab {
    use super::*;

    fn pair(doc: &str) -> (EditorHandle, EditorHandle) {
        let host = create_editor();
        assert!(host.load_html(doc));
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
        (host, guest)
    }

    /// Positive control: with no peer activity a late answer lands and reaches
    /// the peer (the image is recorded and broadcast like any edit).
    #[test]
    fn c0_a_late_answer_reaches_the_peer() {
        let (host, guest) = pair("<p>hello world</p><p>second</p>");
        host.set_selection(Selection::cursor(Pos(6)));
        let kept = keeping_anchors(&host);
        assert!(!offer(&host));
        let anchor = kept.borrow_mut().remove(0);
        assert!(host.insert_image_at(&anchor, "app-blob:1", ""));
        let want = r#"<p>hello<img src="app-blob:1"> world</p><p>second</p>"#;
        assert_eq!(html(&host), want);
        assert_eq!(html(&guest), want);
        assert!(host.collab_take_error().is_none());
        assert!(host.collab_outbound_stall().is_none());
    }

    /// C1. THE CASE: the picture is uploading and a peer types one letter in
    /// ANOTHER paragraph. The picture must still land where it was aimed.
    #[test]
    fn c1_a_peers_keystroke_during_the_upload_does_not_lose_the_picture() {
        let (host, guest) = pair("<p>hello world</p><p>second</p>");
        host.set_selection(Selection::cursor(Pos(6)));
        let kept = keeping_anchors(&host);
        assert!(!offer(&host));

        // A peer types at the end of the other paragraph.
        guest.set_selection(Selection::cursor(Pos(20)));
        assert!(guest.insert_text("!"));
        assert_eq!(html(&host), "<p>hello world</p><p>second!</p>");

        let anchor = kept.borrow_mut().remove(0);
        assert!(
            anchor.selection().is_some(),
            "the anchor died on a peer's keystroke in another paragraph"
        );
        assert!(host.insert_image_at(&anchor, "app-blob:1", ""));
        let want = r#"<p>hello<img src="app-blob:1"> world</p><p>second!</p>"#;
        assert_eq!(html(&host), want);
        assert_eq!(html(&guest), want);
    }

    /// C3. A peer types in the SAME paragraph, before the place: the anchor
    /// keeps its place in the text (it moves with the words), where the
    /// block-level step map alone would put it at the paragraph's end.
    #[test]
    fn c3_a_peer_typing_in_the_same_paragraph_moves_the_anchor_with_the_text() {
        let (host, guest) = pair("<p>hello world</p><p>second</p>");
        host.set_selection(Selection::cursor(Pos(6)));
        let kept = keeping_anchors(&host);
        assert!(!offer(&host));
        // The host's own caret goes elsewhere, so only the anchor is tested.
        host.set_selection(Selection::cursor(Pos(20)));

        guest.set_selection(Selection::cursor(Pos(1)));
        assert!(guest.insert_text("XY"));
        assert_eq!(html(&host), "<p>XYhello world</p><p>second</p>");

        let anchor = kept.borrow_mut().remove(0);
        assert_eq!(anchor.selection(), Some(Selection::cursor(Pos(8))));
        assert!(host.insert_image_at(&anchor, "app-blob:1", ""));
        let want = r#"<p>XYhello<img src="app-blob:1"> world</p><p>second</p>"#;
        assert_eq!(html(&host), want);
        assert_eq!(html(&guest), want);
    }

    /// C4. A peer types in an EARLIER paragraph: the anchor's own block is
    /// untouched and shifts by what was typed. (C1's peer types in a later
    /// paragraph, where a carry that left every position alone would pass.)
    #[test]
    fn c4_a_peer_typing_in_an_earlier_paragraph_shifts_the_anchor() {
        let (host, guest) = pair("<p>first</p><p>hello world</p>");
        host.set_selection(Selection::text(Pos(8), Pos(13))); // [hello]
        let kept = keeping_anchors(&host);
        assert!(!offer(&host));
        host.set_selection(Selection::cursor(Pos(1)));

        guest.set_selection(Selection::cursor(Pos(3)));
        assert!(guest.insert_text("XYZ"));
        assert_eq!(html(&host), "<p>fiXYZrst</p><p>hello world</p>");

        let anchor = kept.borrow_mut().remove(0);
        assert_eq!(
            anchor.selection(),
            Some(Selection::text(Pos(11), Pos(16))),
            "the range still covers `hello`"
        );
        assert!(host.insert_image_at(&anchor, "app-blob:1", ""));
        let want = r#"<p>fiXYZrst</p><p><img src="app-blob:1"> world</p>"#;
        assert_eq!(html(&host), want);
        assert_eq!(html(&guest), want);
    }

    /// C5. A peer removes the paragraph the picture was aimed at: the place is
    /// gone, and the anchor says so rather than pointing at whatever is near.
    /// The refused insert changes nothing, the host's caret included.
    #[test]
    fn c5_an_anchor_whose_paragraph_a_peer_removed_names_no_place() {
        let (host, guest) = pair("<p>hello world</p><p>second</p><p>third</p>");
        host.set_selection(Selection::cursor(Pos(6)));
        let kept = keeping_anchors(&host);
        assert!(!offer(&host));
        host.set_selection(Selection::cursor(Pos(24))); // in `third`

        // The peer selects from the start of the first paragraph into the
        // second and deletes: the first paragraph's text is gone.
        guest.set_selection(Selection::text(Pos(1), Pos(16)));
        assert!(guest.command("deleteSelection"));
        assert_eq!(html(&host), "<p>cond</p><p>third</p>");
        let live = host.selection();

        let anchor = kept.borrow_mut().remove(0);
        assert_eq!(anchor.selection(), None);
        assert!(!host.insert_image_at(&anchor, "app-blob:1", ""));
        assert_eq!(html(&host), "<p>cond</p><p>third</p>");
        assert_eq!(host.selection(), live);
    }

    /// C2. The synchronous answer is not exposed to this: a control.
    #[test]
    fn c2_an_answer_at_once_is_unaffected_by_peers() {
        let (host, guest) = pair("<p>hello world</p>");
        guest.set_selection(Selection::cursor(Pos(12)));
        assert!(guest.insert_text("!"));
        host.set_selection(Selection::cursor(Pos(6)));
        host.on_image_input(|_| Some(("app-blob:1".to_string(), String::new())));
        assert!(offer(&host));
        let want = r#"<p>hello<img src="app-blob:1"> world!</p>"#;
        assert_eq!(html(&host), want);
        assert_eq!(html(&guest), want);
    }
}

/// R8. The owner rule (#147/#183), which the PR's doc states and no PR fixture
/// samples (mutant M4: `has_image_input_callback` ignoring the owner survives
/// the PR's suite): a callback registered by a component is not called once the
/// component unmounts, and the runtime is told there is none, so it goes back
/// to its default instead of dropping the picture.
#[test]
fn r8_a_callback_whose_component_unmounted_is_gone() {
    use rinch_core::reactive::{Scope, Signal};
    let handle = editor("<p>ab</p>", 2);
    let scope = Scope::new();
    let calls = Rc::new(RefCell::new(0));
    scope.run(|| {
        let sig = Signal::new(0u32);
        let calls = calls.clone();
        handle.on_image_input(move |_| {
            *calls.borrow_mut() += 1;
            let _ = sig.get();
            Some(("app-blob:1".to_string(), String::new()))
        });
    });
    assert!(handle.has_image_input_callback(), "control: live");
    scope.dispose();
    assert!(!handle.has_image_input_callback());
    assert!(!offer(&handle));
    assert_eq!(*calls.borrow(), 0);
    assert_eq!(html(&handle), "<p>ab</p>");
}

/// R9. The runtime offers a picture AT the place it was aimed
/// (`offer_image_input_at`), which need not be where the person is by the time
/// the clipboard or the disk has answered: an answer at once lands there and
/// the person's caret stays theirs. An anchor that names no place any more
/// does not ask the app to store anything.
#[test]
fn r9_a_picture_offered_at_an_anchor_leaves_the_live_caret_and_a_dead_anchor_asks_nobody() {
    let handle = editor("<p>hello world</p><p>second</p>", 6);
    let calls = Rc::new(RefCell::new(0));
    let calls_in = calls.clone();
    handle.on_image_input(move |_| {
        *calls_in.borrow_mut() += 1;
        Some(("app-blob:1".to_string(), String::new()))
    });
    let at = handle.anchor_selection();
    handle.set_selection(Selection::cursor(Pos(20))); // second|
    assert!(handle.offer_image_input_at(
        &at,
        ImageInputSource::Drop,
        PNG.to_vec(),
        "image/png",
        None
    ));
    assert_eq!(
        html(&handle),
        r#"<p>hello<img src="app-blob:1"> world</p><p>second</p>"#
    );
    assert_eq!(handle.selection(), Selection::cursor(Pos(21)));

    let dead = handle.anchor_selection();
    assert!(handle.load_html("<p>another note</p>"));
    assert!(!handle.offer_image_input_at(
        &dead,
        ImageInputSource::Drop,
        PNG.to_vec(),
        "image/png",
        None
    ));
    assert_eq!(*calls.borrow(), 1, "the app was not asked for a dead place");
}

/// R10. A picture that arrives while the person types is an undo step of its
/// own on both sides: it is not grouped with the letters typed just before it
/// (R6) nor with the ones typed just after.
#[test]
fn r10_what_is_typed_after_a_late_picture_is_not_grouped_with_it() {
    let handle = editor("<p>ab</p><p>cd</p>", 2);
    let kept = keeping_anchors(&handle);
    assert!(!offer(&handle));
    handle.set_selection(Selection::cursor(Pos(7))); // cd|
    let anchor = kept.borrow_mut().remove(0);
    assert!(handle.insert_image_at(&anchor, "app-blob:1", ""));
    assert!(handle.insert_text("?"));
    assert_eq!(
        html(&handle),
        r#"<p>a<img src="app-blob:1">b</p><p>cd?</p>"#
    );
    assert!(handle.command("undo"));
    assert_eq!(
        html(&handle),
        r#"<p>a<img src="app-blob:1">b</p><p>cd</p>"#,
        "one undo takes back the letter and leaves the picture"
    );
}
