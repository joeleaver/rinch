//! Link activation and link hover: what an app hears when the pointer presses
//! or rests on a link in an editor.
//!
//! The editor never follows a link itself. A press on a link is offered to the
//! app first ([`EditorHandle::on_link_click`](crate::EditorHandle::on_link_click)),
//! and the pointer entering or leaving a link is reported
//! ([`EditorHandle::on_link_hover`](crate::EditorHandle::on_link_hover)); what a
//! link *means* — a URL to open, a node to navigate to — is the app's business.
//! Which link is under the pointer is decided per character by
//! [`link_at`](rinch_editor_core::link_at), so the space just after a link is
//! not on it.

use rinch_core::reactive::ElementBounds;
pub use rinch_editor_core::LinkSpan;

/// A primary-button press on a link — see
/// [`EditorHandle::on_link_click`](crate::EditorHandle::on_link_click).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkClick {
    /// The link pressed: its `href`, `title` and the whole run it covers.
    pub link: LinkSpan,
    /// The platform's accelerator was held: Cmd on macOS, Ctrl elsewhere. This
    /// is the "open the link" chord of most editors (Ctrl/Cmd+click).
    pub primary: bool,
    /// Control was held.
    pub ctrl: bool,
    /// Meta (Cmd on macOS, the Windows/Super key elsewhere) was held.
    pub meta: bool,
    /// Shift was held.
    pub shift: bool,
    /// Alt (Option on macOS) was held.
    pub alt: bool,
}

/// The link the pointer is over — see
/// [`EditorHandle::on_link_hover`](crate::EditorHandle::on_link_hover).
#[derive(Clone, Debug, PartialEq)]
pub struct LinkHover {
    /// The link hovered: its `href`, `title` and the whole run it covers.
    pub link: LinkSpan,
    /// The box around the link's painted run, measured when the pointer
    /// entered it, in the frame an app positions an overlay in: logical window
    /// pixels on desktop (the frame of
    /// [`NodeHandle::bounds_signal`](rinch_core::dom::NodeHandle::bounds_signal)
    /// and of `position: fixed` / absolutely positioned popups at the root),
    /// viewport client pixels in the browser (`getBoundingClientRect`). A link
    /// that wraps over two lines is the union of both lines' boxes. Not
    /// re-measured while the pointer stays on the link, so a scroll in between
    /// leaves it stale.
    pub rect: ElementBounds,
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use rinch_editor_core::{Pos, Selection};

    use super::*;
    use crate::registry::{link_hover_wanted, set_link_hover};
    use crate::{EditorHandle, create_editor};

    // "go " 1..4, "here" 4..8 (its "er" bold), " and " 8..13, "there" 13..18.
    const HTML: &str = r#"<p>go <a href="pimble:a/b">h<strong>er</strong>e</a> and <a href="https://x.test/">there</a></p>"#;

    fn editor() -> EditorHandle {
        let h = create_editor();
        assert!(h.load_html(HTML));
        h
    }

    fn click(link: LinkSpan, primary: bool) -> LinkClick {
        LinkClick {
            link,
            primary,
            ctrl: primary,
            meta: false,
            shift: false,
            alt: false,
        }
    }

    fn hover(h: &EditorHandle, pos: usize) -> LinkHover {
        LinkHover {
            link: h.link_at(Pos(pos)).expect("on a link"),
            rect: ElementBounds::default(),
        }
    }

    #[test]
    fn the_handle_answers_the_whole_run_and_not_the_character_after_it() {
        let h = editor();
        let span = h.link_at(Pos(5)).expect("on the bold part");
        assert_eq!(span.href, "pimble:a/b");
        assert_eq!((span.from, span.to), (Pos(4), Pos(8)));
        assert_eq!(h.link_at(Pos(8)), None, "the space after the link");
        assert_eq!(
            h.link_at(Pos(13)).map(|s| s.href),
            Some("https://x.test/".to_string())
        );
    }

    #[test]
    fn a_press_is_offered_to_the_callback_which_may_reenter_the_handle() {
        let h = editor();
        let link = h.link_at(Pos(4)).unwrap();
        assert!(
            !h.dispatch_link_click(&click(link.clone(), true)),
            "no callback: nothing claims the press"
        );

        let seen: Rc<RefCell<Vec<(String, bool, Selection)>>> = Rc::default();
        h.on_link_click({
            let (h, seen) = (h.clone(), seen.clone());
            move |c| {
                // Re-entering the handle from the callback, mutably, must not
                // panic: nothing may be borrowed while it runs.
                h.set_selection(h.selection());
                seen.borrow_mut()
                    .push((c.link.href.clone(), c.primary, h.selection()));
                c.primary
            }
        });
        assert!(!h.dispatch_link_click(&click(link.clone(), false)));
        assert!(h.dispatch_link_click(&click(link, true)));
        let seen = seen.borrow();
        assert_eq!(seen.len(), 2);
        assert_eq!((seen[0].0.as_str(), seen[0].1), ("pimble:a/b", false));
        assert_eq!((seen[1].0.as_str(), seen[1].1), ("pimble:a/b", true));
    }

    /// Every hover callback call as `Some(href)` / `None`. The callback
    /// re-enters the handle, as an app's does; it reaches it through a weak
    /// reference so the editor (and its count in `link_hover_wanted`) still
    /// goes when the test drops it, whatever thread the harness reuses.
    fn record(h: &EditorHandle) -> Rc<RefCell<Vec<Option<String>>>> {
        let calls: Rc<RefCell<Vec<Option<String>>>> = Rc::default();
        let weak = h.downgrade_for_tests();
        h.on_link_hover({
            let calls = calls.clone();
            move |hover| {
                if let Some(h) = weak.upgrade() {
                    h.set_selection(h.selection()); // re-entrant, mutably
                }
                calls.borrow_mut().push(hover.map(|h| h.link.href.clone()));
            }
        });
        calls
    }

    #[test]
    fn hover_fires_once_per_enter_change_and_leave() {
        let h = editor();
        let calls = record(&h);
        let doc = Some(7);

        set_link_hover(doc, None);
        assert!(calls.borrow().is_empty(), "no link, no change");
        set_link_hover(doc, Some((h.clone(), hover(&h, 4))));
        set_link_hover(doc, Some((h.clone(), hover(&h, 5))));
        set_link_hover(doc, Some((h.clone(), hover(&h, 7))));
        assert_eq!(
            *calls.borrow(),
            vec![Some("pimble:a/b".to_string())],
            "enter fires once; moving along the same link fires nothing"
        );
        set_link_hover(doc, Some((h.clone(), hover(&h, 13))));
        assert_eq!(calls.borrow().len(), 2, "a different link fires once");
        assert_eq!(calls.borrow()[1].as_deref(), Some("https://x.test/"));
        set_link_hover(doc, None);
        set_link_hover(doc, None);
        assert_eq!(calls.borrow().len(), 3, "leaving fires once");
        assert_eq!(calls.borrow()[2], None);
    }

    #[test]
    fn moving_between_editors_leaves_one_and_enters_the_other() {
        let (a, b) = (editor(), editor());
        let (calls_a, calls_b) = (record(&a), record(&b));
        set_link_hover(None, Some((a.clone(), hover(&a, 4))));
        set_link_hover(None, Some((b.clone(), hover(&b, 4))));
        assert_eq!(
            *calls_a.borrow(),
            vec![Some("pimble:a/b".to_string()), None],
            "the same link text in another editor is another link"
        );
        assert_eq!(*calls_b.borrow(), vec![Some("pimble:a/b".to_string())]);
        set_link_hover(None, None);
    }

    #[test]
    fn documents_are_tracked_separately() {
        let h = editor();
        let calls = record(&h);
        set_link_hover(Some(1), Some((h.clone(), hover(&h, 4))));
        set_link_hover(Some(2), None);
        assert_eq!(
            calls.borrow().len(),
            1,
            "another document's move is not a leave"
        );
        set_link_hover(Some(1), None);
    }

    #[test]
    fn link_hover_is_wanted_only_while_an_editor_has_a_callback() {
        let h = editor();
        assert!(!link_hover_wanted(), "no editor has a hover callback");
        h.on_link_click(|_| true);
        assert!(
            !link_hover_wanted(),
            "a click callback is not a hover callback"
        );
        h.on_link_hover(|_| {});
        h.on_link_hover(|_| {});
        assert!(link_hover_wanted());
        let other = editor();
        other.on_link_hover(|_| {});
        drop(h);
        assert!(link_hover_wanted(), "one editor still has one");
        drop(other);
        assert!(
            !link_hover_wanted(),
            "gone with the last editor that had one"
        );
    }
}
