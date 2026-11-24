//! Selection anchors: selections captured by in-flight asynchronous work,
//! carried forward through every edit until the work comes back.

use super::*;

/// Selections captured by in-flight asynchronous work, each carried forward
/// through every document change until its operation completes.
///
/// Lives behind its **own** `RefCell`, not inside [`EditorCore`], on purpose: a
/// [`SelectionAnchor`] releases itself when dropped, and that drop can land while
/// the core is borrowed (a callback dropping its anchor from inside an `update`
/// closure). Borrowing only this map keeps the release from being a double-borrow
/// panic.
#[derive(Default)]
pub(super) struct AnchorMap {
    pub(super) next_id: u64,
    /// `None` marks an anchor whose document is gone — see
    /// [`EditorCore::carry_anchors`].
    pub(super) live: HashMap<u64, Option<Selection>>,
}

/// A selection captured for an operation that finishes *later*, kept pointing at
/// the same content as the user keeps editing.
///
/// The problem it solves: an asynchronous paste is dispatched when the user hits
/// Ctrl+V but lands once the clipboard answers — and because the UI no longer
/// freezes while that happens (issue #149), the user can type in between. A raw
/// position captured at dispatch would be stale by then; the live caret would be
/// wherever they wandered to. An anchor is neither: it is the captured selection
/// **mapped through the intervening steps**, which is what a transactional editor
/// can offer and a `contenteditable` one cannot.
///
/// Obtain one from [`EditorHandle::anchor_selection`]. Dropping it releases the
/// capture, so an operation that is abandoned leaves nothing behind.
///
/// ```ignore
/// let anchor = handle.anchor_selection();
/// paste_text_async(move |text| {
///     // ... marshalled back onto the UI thread ...
///     if let Some(sel) = anchor.selection() {
///         handle.set_selection(sel);
///         handle.replace_selection_with_text(&text);
///     }
/// });
/// ```
pub struct SelectionAnchor {
    pub(super) anchors: Weak<RefCell<AnchorMap>>,
    pub(super) id: u64,
}

impl SelectionAnchor {
    /// Where the captured selection sits **now**, after every change applied
    /// since it was taken.
    ///
    /// `None` once the anchor can no longer mean anything: the editor was
    /// dropped, the document it pointed into was replaced wholesale
    /// (`load_doc`/`load_html`), or a collaborating peer removed, split, joined
    /// or changed the kind of the block it sat in. A caller should then abandon
    /// the operation rather than guess — the content the user aimed at is gone.
    ///
    /// Any other change by a peer carries the anchor as a local edit does: text
    /// typed elsewhere shifts it, and text typed in its own paragraph leaves it
    /// at the same place in the words.
    pub fn selection(&self) -> Option<Selection> {
        let anchors = self.anchors.upgrade()?;
        let anchors = anchors.borrow();
        anchors.live.get(&self.id).cloned().flatten()
    }
}

impl Drop for SelectionAnchor {
    fn drop(&mut self) {
        if let Some(anchors) = self.anchors.upgrade() {
            // A `Selection` drop runs no user code, so this cannot re-enter.
            if let Ok(mut anchors) = anchors.try_borrow_mut() {
                anchors.live.remove(&self.id);
            }
        }
    }
}

impl fmt::Debug for SelectionAnchor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SelectionAnchor")
            .field("id", &self.id)
            .field("selection", &self.selection())
            .finish()
    }
}

impl EditorHandle {
    /// Capture the current selection for an operation that will finish later,
    /// returning an anchor that stays pointed at the same content as the user
    /// keeps editing.
    ///
    /// This is what makes an asynchronous insertion land where the user asked for
    /// it. The built-in paste uses it: Ctrl+V anchors the selection, the clipboard
    /// read runs off the UI thread (so the user can keep typing — issue #149), and
    /// the content is inserted at the anchor, mapped through everything typed in
    /// the meantime.
    ///
    /// The anchor releases itself when dropped, and reports `None` from
    /// [`SelectionAnchor::selection`] once its document has been replaced.
    pub fn anchor_selection(&self) -> SelectionAnchor {
        let core = self.core();
        let anchors = core.anchors.clone();
        let id = {
            let mut map = anchors.borrow_mut();
            map.next_id += 1;
            let id = map.next_id;
            map.live.insert(id, Some(core.state.selection.clone()));
            id
        };
        SelectionAnchor {
            anchors: Rc::downgrade(&anchors),
            id,
        }
    }
}
