//! A window sized by its content (#2552, R0): the mode a popup's document is
//! in. Nothing an app reaches; the popup role (R1) calls it.
//!
//! **The box.** A content-sized document's content is mounted in one element
//! rinch makes for it, under `<body>`: the content box, marked
//! `data-rinch-content-box`. Its inline style is rinch's alone (the app's
//! `<body>` and `<html>` styles are untouched): fixed at the window's origin,
//! `max-content`, at most the size the window has (the largest size before
//! the window exists, then the size the compositor granted), with
//! `overflow: auto`. It is the window: margin, padding and border on `<html>`
//! and `<body>` neither offset nor clip it. Fixed, not absolute, because
//! rinch's `<body>` clips what it holds (the UA sheet gives it
//! `overflow-y: auto`) and a fixed box is hoisted out of that clip; an
//! absolute one was cut by a body margin or border. A background on `<body>`
//! paints the body's box, which spans the viewport. The component's own
//! node is laid out inside it as anywhere: a node of `width: 300px` gives
//! exactly 300, one with no width shrinks to its content, and long text wraps
//! at the most the box may take. In a window sized by its content, pad the
//! content, not the body.
//!
//! **The measured size** ([`RinchApp::content_size`]) is the box's border
//! box, or its scrollable extent where that is larger, in whole logical
//! pixels, at most the largest size. Unconstrained (before the window exists,
//! or while the grant is at least what was asked) it is what the content
//! wants; once the grant is smaller, it is the content's extent at the
//! granted width.
//!
//! **The guard is by cause.** A new size is asked for
//! ([`RinchApp::content_size_changed`]) only after a layout a change of the
//! document caused (a mutation or a style change: a node was waiting to be
//! laid out), never after one only a size the window was given caused, and
//! only when it differs from the size last asked for. So a grant smaller than
//! asked just scrolls, text that wraps at the granted width asks nothing
//! more, and content sized against the viewport (which is the window, itself
//! sized by the content) settles after one request per change of the
//! document instead of shrinking the window step by step.
//!
//! How a layout's cause is told: before each layout, the nodes waiting for
//! it (`NodeTree::dirty_nodes` and `style_dirty_nodes`, which every layout
//! empties) are read. A grant writes the box's style, which dirties the box
//! and what inherits from it: it reads first (a change already waiting is the
//! document's), writes, and keeps what is dirty then as its own work; the
//! next layout counts only nodes beyond that. A window's resize dirties no
//! node, and a viewport change restyles inside the layout, after the read.
//! The paint-only lists are not read: a paint empties them, not a layout, and
//! a change that only repaints changes no size.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::{DomDocument, NodeHandle, NodeId, RenderScope};

use super::RinchApp;

/// The attribute that marks the content box in a DOM dump and in DevTools.
const CONTENT_BOX_ATTRIBUTE: &str = "data-rinch-content-box";

/// The content-sized mode's state.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the popup role (#2552 R1) is its caller")
)]
#[derive(Debug, Clone)]
pub(crate) struct ContentSized {
    /// The largest size the content may take, logical px.
    max: (f32, f32),
    /// The size the window was given, once it was.
    granted: Option<(f32, f32)>,
    /// The size last asked for.
    asked: Option<(u32, u32)>,
    /// A change of the document was laid out since the last measure.
    document_changed: bool,
    /// What was waiting for layout right after the last grant wrote the box:
    /// the grant's own work, which the next layout does not count.
    grant_dirt: Option<Vec<u64>>,
    /// The content box, once the document is mounted.
    content_box: Option<NodeId>,
}

/// The nodes waiting for the next layout, sorted.
fn waiting(tree: &rinch_dom::NodeTree) -> Vec<u64> {
    let mut nodes: Vec<u64> = tree
        .dirty_nodes
        .iter()
        .chain(tree.style_dirty_nodes.iter())
        .copied()
        .collect();
    nodes.sort_unstable();
    nodes.dedup();
    nodes
}

/// The content box's inline style, at most `(w, h)` logical px.
fn box_style((w, h): (f32, f32)) -> String {
    format!(
        "position: fixed; left: 0; top: 0; margin: 0; width: max-content; \
         height: max-content; max-width: {w}px; max-height: {h}px; overflow: auto; \
         box-sizing: border-box"
    )
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the popup role (#2552 R1) is its caller")
)]
impl RinchApp {
    /// An app whose document is content-sized, at most `max` logical pixels.
    /// The mode is fixed at construction: the content box is made by the
    /// mount, before the content, and a mounted tree is never re-parented
    /// into one.
    pub(crate) fn new_content_sized(
        component: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static,
        max: (f32, f32),
    ) -> Self {
        let mut app = Self::new(component);
        app.content_sized = Some(ContentSized {
            max,
            granted: None,
            asked: None,
            document_changed: true,
            grant_dirt: None,
            content_box: None,
        });
        app
    }

    /// The node the mount puts the content in: a content-sized document's
    /// content box, made here under `<body>`; `<body>` for every other.
    pub(crate) fn content_box_parent(
        &mut self,
        doc: &Rc<RefCell<rinch_dom::RinchDocument>>,
        body: NodeId,
    ) -> NodeId {
        let Some(cs) = self.content_sized.as_mut() else {
            return body;
        };
        let mut d = doc.borrow_mut();
        let content_box = d.create_element("div");
        d.set_attribute(content_box, CONTENT_BOX_ATTRIBUTE, "");
        d.set_attribute(
            content_box,
            "style",
            &box_style(cs.granted.unwrap_or(cs.max)),
        );
        d.append_child(body, content_box);
        cs.content_box = Some(content_box);
        content_box
    }

    /// The size the window was given (a configure): the content is laid out
    /// at most that size, and scrolls in it when it is smaller than the
    /// content. Not a change of the document: no new size is asked for from
    /// the layout it causes.
    pub(crate) fn set_granted_size(&mut self, width: f32, height: f32) {
        if self.content_sized.is_none() {
            return;
        }
        // A change already waiting is the document's, whatever comes next.
        self.note_content_sized_layout();
        let Some(cs) = self.content_sized.as_mut() else {
            return;
        };
        cs.granted = Some((width, height));
        let (Some(content_box), Some(doc)) = (cs.content_box, &self.doc) else {
            return;
        };
        doc.borrow_mut()
            .set_attribute(content_box, "style", &box_style((width, height)));
        // What the grant's own write dirtied: not a change of the document.
        let dirt = waiting(&doc.borrow().tree);
        cs.grant_dirt = Some(dirt);
    }

    /// The content's size, from the last layout (see the module docs), and
    /// remembered as the size asked for: what the window is made at.
    pub(crate) fn content_size(&mut self) -> Option<(u32, u32)> {
        let size = self.measure_content()?;
        let cs = self.content_sized.as_mut()?;
        cs.asked = Some(size);
        cs.document_changed = false;
        Some(size)
    }

    /// After a layout: the size to ask the window for, when a change of the
    /// document was laid out and the content's size is not the one last asked
    /// for (see the module docs); `None` otherwise, and always after a layout
    /// only a granted size caused.
    pub(crate) fn content_size_changed(&mut self) -> Option<(u32, u32)> {
        let caused = std::mem::take(&mut self.content_sized.as_mut()?.document_changed);
        if !caused {
            return None;
        }
        let size = self.measure_content()?;
        let cs = self.content_sized.as_mut()?;
        if cs.asked == Some(size) {
            return None;
        }
        cs.asked = Some(size);
        Some(size)
    }

    /// Before a layout of a content-sized document (and before a grant writes
    /// the box): whether a change of the document is waiting in it (see the
    /// module docs). Off the mode, nothing.
    pub(crate) fn note_content_sized_layout(&mut self) {
        let Some(cs) = self.content_sized.as_mut() else {
            return;
        };
        let Some(doc) = &self.doc else {
            return;
        };
        let d = doc.borrow();
        let now = waiting(&d.tree);
        let changed = match cs.grant_dirt.take() {
            Some(grant) => now.iter().any(|id| grant.binary_search(id).is_err()),
            None => !now.is_empty(),
        };
        if changed {
            cs.document_changed = true;
        }
    }

    /// The content box's border box, or its scrollable extent where larger,
    /// rounded up to whole logical pixels, at most the largest size.
    fn measure_content(&self) -> Option<(u32, u32)> {
        let cs = self.content_sized.as_ref()?;
        let content_box = cs.content_box?.0;
        let doc = self.doc.as_ref()?;
        let d = doc.borrow();
        let (_, _, bw, bh) = super::hit_testing::painted_element_box(&d.tree, content_box);
        let (ew, eh) = rinch_dom::paint::scrollbar::content_extents(&d.tree, content_box);
        let w = (bw as f64).max(ew).min(cs.max.0 as f64).ceil();
        let h = (bh as f64).max(eh).min(cs.max.1 as f64).ceil();
        Some((w as u32, h as u32))
    }

    /// The content box, for tests.
    #[cfg(test)]
    pub(crate) fn content_box(&self) -> Option<NodeId> {
        self.content_sized.as_ref()?.content_box
    }
}

#[cfg(test)]
#[path = "content_size_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "content_size_body_tests.rs"]
mod body_tests;
