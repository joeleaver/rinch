//! #1116: desktop Home and End on a line that starts or ends with an image.
//!
//! An image is an inline box with no bytes in desktop's IFC, so the positions
//! on its two sides share one flat byte, and `visual_line_bound` mapped the
//! line's edge byte back to a position with `EditorHandle::pos_at`, which
//! picks one side by where the text runs are: after a leading image (Home on
//! `<img>bravo` landed after it) and before a trailing one (End on
//! `bravo<img>` landed before it). The line's own items say which side: an
//! inline box that is the line's first item is the line's start, one that is
//! its last item its end, and an image at the edge byte that is not on the
//! line belongs to the neighbouring line.
//!
//! Chrome 155, the same paragraphs in a `contenteditable` (Inter 16px / 24px,
//! 40x16 images): Home lands before a leading image, End after a trailing
//! one; at a soft wrap before an image, End on the upper line stays before it
//! and Home on the lower line lands before it.

use super::*;
use rinch_editor_core::{Pos, Selection};

const VP: (u32, u32) = (800, 600);
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const LINE: f32 = 24.0;
const IMG: &str = "<img src=\"x.png\" alt=\"\">";

fn idle(app: &mut RinchApp) {
    for _ in 0..3 {
        let actions = app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
        if actions.contains(&AppAction::RequestRedraw) {
            if app.has_pending_layout() {
                app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
            }
            let _ = app.build_pixels(1.0, VP, false);
        }
    }
}

struct Page {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
}

/// One paragraph in an editor `width` px wide, Inter 16px / 24px, focused and
/// settled; every image 40x16.
fn page(html: &str, width: u32) -> Page {
    let handle = crate::editor::create_editor();
    assert!(handle.load_html(html));
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let sheet = scope.create_element("style");
        sheet.append_child(&scope.create_text("img { width: 40px; height: 16px; }"));
        root.append_child(&sheet);
        let editor = handle_in.mount(scope);
        editor.set_attribute(
            "style",
            &format!(
                "width: {width}px; height: 400px; padding: 0; font-size: 16px; line-height: 24px; \
                 font-family: sans-serif"
            ),
        );
        root.append_child(&editor);
        root
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let mut page = Page { app, handle };
    page.handle.focus();
    idle(&mut page.app);
    page
}

impl Page {
    /// The head as a char offset into the paragraph.
    fn head(&self) -> usize {
        self.handle.selection().head().0 - 1
    }
    fn at(&mut self, i: usize) {
        self.handle.set_selection(Selection::cursor(Pos(i + 1)));
        idle(&mut self.app);
    }
    fn key(&mut self, key: KeyCode) {
        let modifiers = Modifiers::default();
        self.app.handle_event(
            PlatformEvent::KeyDown {
                key,
                logical_key: None,
                text: None,
                modifiers,
                repeat: KeyRepeat::Fresh,
            },
            VP,
            1.0,
        );
        self.app.handle_event(
            PlatformEvent::KeyUp {
                key,
                logical_key: None,
                modifiers,
            },
            VP,
            1.0,
        );
        idle(&mut self.app);
    }
    /// Home (or End) from char `i`: the head it lands on.
    fn press(&mut self, i: usize, key: KeyCode) -> usize {
        self.at(i);
        self.key(key);
        self.head()
    }
    /// The layout-local line (0-based) of the painted image `n`.
    fn image_line(&self, n: usize) -> usize {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let id = doc.query_selector_all("img")[n];
        let img = super::hit_testing::painted_element_box(&doc.tree, id.0);
        let block = self.handle.caret_address(Pos(1)).unwrap().0;
        let b = super::hit_testing::painted_element_box(&doc.tree, block);
        ((img.1 - b.1) / LINE).floor() as usize
    }
    /// The line (0-based) the caret overlay is drawn on.
    fn drawn_line(&self) -> usize {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let block = self.handle.caret_address(Pos(1)).unwrap().0;
        let b = super::hit_testing::painted_element_box(&doc.tree, block);
        let c = doc
            .query_selector_all("[data-pm-caret]")
            .into_iter()
            .map(|id| super::hit_testing::painted_element_box(&doc.tree, id.0))
            .find(|c| c.3 > 0.0)
            .expect("a drawn caret");
        ((c.1 + c.3 / 2.0 - b.1) / LINE).floor() as usize
    }
    /// The layout-local line of the caret at char `i` (downstream).
    fn line_of(&self, i: usize) -> usize {
        let (block, byte) = self.handle.caret_address(Pos(i + 1)).unwrap();
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let (_, y) = doc.query_caret_position(block, byte).unwrap();
        (y / LINE).floor() as usize
    }
}

/// `<p><img>bravo</p>`, caret in `bravo`: Home lands before the image, at
/// the paragraph's start (char 0). It landed after it (1).
#[test]
fn home_on_a_line_that_starts_with_an_image_lands_before_it() {
    let mut p = page(&format!("<p>{IMG}bravo</p>"), 400);
    assert_eq!(p.press(4, KeyCode::Home), 0);
    // Two images: before the first.
    let mut p = page(&format!("<p>{IMG}{IMG}bravo</p>"), 400);
    assert_eq!(p.press(5, KeyCode::Home), 0);
    // Positive control: text first, Home is the text's start.
    let mut p = page("<p>bravo</p>", 400);
    assert_eq!(p.press(3, KeyCode::Home), 0);
}

/// `<p>bravo<img></p>`, caret in `bravo`: End lands after the image, at the
/// paragraph's end (char 6). It landed before it (5).
#[test]
fn end_on_a_line_that_ends_with_an_image_lands_after_it() {
    let mut p = page(&format!("<p>bravo{IMG}</p>"), 400);
    assert_eq!(p.press(2, KeyCode::End), 6);
    let mut p = page(&format!("<p>bravo{IMG}{IMG}</p>"), 400);
    assert_eq!(p.press(2, KeyCode::End), 7);
    // From before the image too.
    let mut p = page(&format!("<p>bravo{IMG}</p>"), 400);
    assert_eq!(p.press(5, KeyCode::End), 6);
}

/// A soft wrap between two images (`alpha ` + image / image + ` charlie` at
/// 100px): the first image ends line 1 and the second starts line 2, and all
/// four of their sides share the wrap's byte. End on line 1 lands after the
/// first image (char 7), drawn at line 1's end; Home on line 2 lands before
/// the second (also 7). Both landed before the first image (6) — Home on
/// line 2 went back to line 1.
#[test]
fn at_a_wrap_between_two_images_home_and_end_take_their_own_lines_image() {
    let mut p = page(&format!("<p>alpha {IMG}{IMG} charlie</p>"), 100);
    // Positive controls: the first image on line 1, the second on line 2.
    assert_eq!((p.image_line(0), p.image_line(1)), (0, 1));
    assert_eq!(p.line_of(12), 1, "charlie on line 2");
    assert_eq!(p.press(2, KeyCode::End), 7, "after the first image");
    assert_eq!(
        p.handle.caret_affinity(),
        rinch_core::dom::CaretAffinity::Upstream,
        "drawn at line 1's end"
    );
    assert_eq!(p.drawn_line(), 0, "drawn on line 1");
    assert_eq!(p.press(12, KeyCode::Home), 7, "before the second image");
    assert_eq!(p.drawn_line(), 1, "drawn on line 2");
    assert_eq!(p.press(2, KeyCode::Home), 0);
    assert_eq!(p.press(12, KeyCode::End), 16);
}

/// A soft wrap before an image that starts line 2 (`alpha bravo ` /
/// image + `charlie` at 100px): Home on line 2 lands before the image (12),
/// End on line 1 stays before it (12, drawn at line 1's end).
#[test]
fn at_a_wrap_before_an_image_home_and_end_stay_on_their_lines() {
    let mut p = page(&format!("<p>alpha bravo {IMG}charlie</p>"), 100);
    assert_eq!(p.image_line(0), 1, "the image starts line 2");
    assert_eq!(p.line_of(8), 0, "bravo on line 1");
    assert_eq!(p.press(16, KeyCode::Home), 12, "before the image");
    assert_eq!(p.drawn_line(), 1, "drawn on line 2");
    assert_eq!(p.press(8, KeyCode::End), 12, "before the image");
    assert_eq!(
        p.handle.caret_affinity(),
        rinch_core::dom::CaretAffinity::Upstream,
        "drawn at line 1's end"
    );
    assert_eq!(p.press(16, KeyCode::End), 20, "the paragraph's end");
}

/// A soft wrap after an image with text right after it (`alpha ` / `bravo` /
/// image / `charlie` at 80px, where neither neighbour fits beside the image):
/// Home on `charlie`'s line lands after the image (12), not before it (11),
/// which is the image's own line. The line's first item is text, so the
/// image at its start byte is the previous line's.
#[test]
fn home_on_text_after_an_image_that_ends_the_line_above_stays_after_it() {
    let mut p = page(&format!("<p>alpha bravo{IMG}charlie</p>"), 80);
    assert_eq!(p.image_line(0), 2, "positive control: the image on line 3");
    assert_eq!(p.press(15, KeyCode::Home), 12);
    assert_eq!(p.press(15, KeyCode::End), 19);
    assert_eq!(p.press(8, KeyCode::End), 11, "bravo's line ends before it");
}

/// An image right before a hard break: End on that line lands after the
/// image, before the break (char 3); Home on the line after the break is
/// right after it (4).
#[test]
fn end_before_a_hard_break_lands_after_an_image_before_it() {
    let mut p = page(&format!("<p>ab{IMG}<br>cd</p>"), 400);
    assert_eq!(p.press(1, KeyCode::End), 3);
    assert_eq!(p.press(5, KeyCode::Home), 4);
    let mut p = page(&format!("<p>ab<br>{IMG}cd</p>"), 400);
    assert_eq!(
        p.press(5, KeyCode::Home),
        3,
        "before the image after the break"
    );
    assert_eq!(p.press(1, KeyCode::End), 2, "before the break");
}

/// Two images end the line above `charlie` (`alpha ` / `bravo` / image image
/// / `charlie` at 80px): Home from `charlie` steps past both (13). Chrome
/// 155: 13. (Review of #1710: a single step landed between them, 12.)
#[test]
fn home_steps_past_two_images_ending_the_line_above() {
    let mut p = page(&format!("<p>alpha bravo{IMG}{IMG}charlie</p>"), 80);
    assert!(
        p.line_of(16) > p.image_line(1),
        "positive control: charlie below the images"
    );
    assert_eq!(p.press(16, KeyCode::Home), 13);
}

/// At the wrap between two images, keys pressed after Home stay on the line
/// the caret is drawn on (line 2, before the second image): Home again stays
/// at 7, End goes to 16. Chrome 155: 7, 16. Home and End looked the line up
/// from the caret's byte, which is line 1's, so Home again went to 0 and End
/// stayed at 7 (review of #1710).
#[test]
fn keys_after_home_at_a_two_image_wrap_stay_on_its_line() {
    let mut p = page(&format!("<p>alpha {IMG}{IMG} charlie</p>"), 100);
    assert_eq!(p.press(12, KeyCode::Home), 7);
    p.key(KeyCode::Home);
    assert_eq!((p.head(), p.drawn_line()), (7, 1), "Home again");
    p.press(12, KeyCode::Home);
    p.key(KeyCode::End);
    assert_eq!(p.head(), 16, "then End");
    // And End, End on line 1 stays after the first image.
    assert_eq!(p.press(2, KeyCode::End), 7);
    p.key(KeyCode::End);
    assert_eq!((p.head(), p.drawn_line()), (7, 0), "End again");
}

/// A line holding right-to-left text keeps the byte's answer: items are in
/// visual order, so the image that starts line 2 logically (`abc שלום ` /
/// image + `שלום`) sits at its visual right end. Home from the line's end
/// lands at 9, before the image. Chrome 155: 9 (rinch reads no `direction`;
/// the paragraph is left-to-right). The image-edge rule gave 10.
#[test]
fn a_line_holding_right_to_left_text_keeps_the_bytes_answer() {
    let mut matched = 0;
    for w in [80u32, 90, 100, 110] {
        let mut p = page(&format!("<p>abc שלום {IMG}שלום</p>"), w);
        if p.image_line(0) != 1 || p.line_of(2) != 0 {
            continue;
        }
        matched += 1;
        assert_eq!(p.press(14, KeyCode::Home), 9, "w={w}");
        // A known difference (the guide says so): End keeps the byte's answer,
        // the paragraph's end, where Chrome 155 lands after the image (10).
        assert_eq!(p.press(11, KeyCode::End), 14, "w={w}");
    }
    assert!(matched > 0, "positive control: the image starts line 2");
}
