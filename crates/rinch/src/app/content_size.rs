//! A window sized by its content (#2552, R0): the mode the document of a
//! window with `WindowProps::size_to_content` is in (the shell's half is
//! `shell/rinch_runtime/content_window.rs`), and later a popup's.
//!
//! **The box.** A content-sized document's content is mounted in one element
//! rinch makes for it, under `<body>`: the content box, marked
//! `data-rinch-content-box`. Its inline style is rinch's alone (the app's
//! `<body>` and `<html>` styles are untouched): fixed at the window's origin,
//! `max-content`, with `overflow: auto`, and at most the box's limit, per
//! dimension: a dimension is free (the largest size) until the compositor
//! grants less than was asked, and from then on its limit is the largest
//! size the compositor has granted since. A limited dimension does not become
//! free again on its own: rinch cannot learn that the compositor would now
//! give more without asking for more than it was last refused, and the mode
//! does not probe. A grant that leaves the limit as it is writes nothing.
//!
//! It is the window: margin, padding and border on `<html>`
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
//! pixels, at most the largest size. In a free dimension it is what the
//! content wants, so content that grows asks for more, up to the largest
//! size; in a limited one it is the content's extent at the limit (text wraps
//! at a limited width, and what is left over scrolls).
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
//! How a layout's cause is told: before each layout (the frame's,
//! `resolve_and_repaint`, and a `Resized` event's, `resize_layout`), the
//! nodes waiting for it (`NodeTree::dirty_nodes` and `style_dirty_nodes`,
//! which every layout empties) are read; any is a change of the document.
//! Passes inside one of those (bounds subscribers' writes settled in the same
//! frame) belong to its cause. A grant that changes
//! the box's limit writes the box's style, which dirties the box and what
//! inherits from it, so it takes the document's waiting changes first, writes,
//! and lays the document out at once, at the granted size, without counting
//! that layout: everything dirty after it is the document's, whenever the
//! frame comes (an effect that runs between the configure and the frame
//! counts in full, even on a node the grant had dirtied). A grant that leaves
//! the limit as it is (the ordinary one: what was asked) writes nothing and
//! dirties nothing. A window's resize dirties no node, and a viewport change
//! restyles inside the layout, after the read. The paint-only lists are not
//! read: a paint empties them, not a layout, and a change that only repaints
//! changes no size.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::{DomDocument, NodeHandle, NodeId, RenderScope};

use super::RinchApp;

/// The attribute that marks the content box in a DOM dump and in DevTools.
const CONTENT_BOX_ATTRIBUTE: &str = "data-rinch-content-box";

/// The content-sized mode's state.
#[derive(Debug, Clone)]
pub(crate) struct ContentSized {
    /// The largest size the content may take, logical px.
    max: (f32, f32),
    /// The axes the window's compositor assigns (width, height): the content
    /// box fills the window there instead of sizing it (see the module docs).
    fill: (bool, bool),
    /// The box's limit, written in its style: per dimension the largest size,
    /// or a smaller grant.
    limit: (f32, f32),
    /// The size last asked for.
    asked: Option<(u32, u32)>,
    /// A change of the document was laid out since the last measure.
    document_changed: bool,
    /// The next layout is a grant's own (its write to the box): not counted.
    grant_layout: bool,
    /// The content box, once the document is mounted.
    content_box: Option<NodeId>,
}

/// The box's limit under a grant, per dimension (see the module docs): a
/// dimension is free (`max`) until the compositor grants less than was asked;
/// from then on its limit is the largest size granted since, never above
/// `max`. Before anything was asked, the grant, never above `max`.
fn grant_limit(
    max: (f32, f32),
    limit: (f32, f32),
    asked: Option<(u32, u32)>,
    granted: (f32, f32),
) -> (f32, f32) {
    let one = |max: f32, limit: f32, asked: Option<u32>, granted: f32| match asked {
        None => granted.min(max),
        // Less than asked: constrained, or tightened.
        Some(asked) if granted < asked as f32 => granted,
        // What was asked, in a dimension never limited: still free.
        Some(_) if limit >= max => max,
        // What was asked, in a limited dimension: an ask there is at most
        // the limit, so a grant of it says nothing of more; a larger grant
        // shows the compositor gives that much.
        Some(_) => limit.max(granted).min(max),
    };
    (
        one(max.0, limit.0, asked.map(|a| a.0), granted.0),
        one(max.1, limit.1, asked.map(|a| a.1), granted.1),
    )
}

/// The content box's inline style: at most `(w, h)` logical px on an axis
/// the content sizes, the whole window on one it fills.
fn box_style((w, h): (f32, f32), (fill_w, fill_h): (bool, bool)) -> String {
    let width = if fill_w {
        "right: 0; width: auto; max-width: none".to_string()
    } else {
        format!("width: max-content; max-width: {w}px")
    };
    let height = if fill_h {
        "bottom: 0; height: auto; max-height: none".to_string()
    } else {
        format!("height: max-content; max-height: {h}px")
    };
    format!(
        "position: fixed; left: 0; top: 0; margin: 0; {width}; {height}; overflow: auto; \
         box-sizing: border-box"
    )
}

impl RinchApp {
    /// An app whose document is content-sized, at most `max` logical pixels,
    /// filling the window on the axes `fill` names (width, height): those the
    /// window's compositor assigns. The mode is fixed at construction: the
    /// content box is made by the mount, before the content, and a mounted
    /// tree is never re-parented into one.
    pub(crate) fn new_content_sized(
        component: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static,
        max: (f32, f32),
        fill: (bool, bool),
    ) -> Self {
        let mut app = Self::new(component);
        app.content_sized = Some(ContentSized {
            max,
            fill,
            limit: max,
            asked: None,
            document_changed: true,
            grant_layout: false,
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
        d.set_attribute(content_box, "style", &box_style(cs.limit, cs.fill));
        d.append_child(body, content_box);
        cs.content_box = Some(content_box);
        content_box
    }

    /// The size the window was given (a configure), in logical px. It sets
    /// the box's limit per dimension (see the module docs): a grant smaller
    /// than the size asked for limits that dimension to it, and the content
    /// is laid out at most that size and scrolls in it; a larger grant raises
    /// a limited dimension's limit; a free dimension stays free. Not a change
    /// of the document: no new size is asked for from it. A grant that
    /// changes the box's limit lays the document out at once, at this size,
    /// so the frame after it starts clean; one that leaves it writes nothing.
    pub(crate) fn set_granted_size(&mut self, width: f32, height: f32) {
        // A change already waiting is the document's, whatever comes next.
        self.note_content_sized_layout();
        let Some(cs) = self.content_sized.as_mut() else {
            return;
        };
        let limit = grant_limit(cs.max, cs.limit, cs.asked, (width, height));
        self.write_content_limit(limit, (width, height));
    }

    /// A new negotiation with a new largest size (a new surface: a show, an
    /// output change, a change of the layer surface's configuration): every
    /// dimension is free again, at most `max`. Laid out at once when the box
    /// changed (at the viewport the document has), so [`Self::content_size`]
    /// measures the new box. Not a change of the document.
    pub(crate) fn restart_content_limit(&mut self, max: (f32, f32)) {
        self.note_content_sized_layout();
        let viewport = self.doc.as_ref().map(|doc| {
            let v = doc.borrow().tree.viewport;
            (v.width, v.height)
        });
        let Some(cs) = self.content_sized.as_mut() else {
            return;
        };
        cs.max = max;
        self.write_content_limit(max, viewport.unwrap_or(max));
    }

    /// Write `limit` to the box when it changes it, and lay the document out
    /// at once at `viewport`, a layout whose cause is not counted.
    fn write_content_limit(&mut self, limit: (f32, f32), viewport: (f32, f32)) {
        let Some(cs) = self.content_sized.as_mut() else {
            return;
        };
        if limit == cs.limit {
            return;
        }
        cs.limit = limit;
        let (Some(content_box), Some(doc)) = (cs.content_box, self.doc.clone()) else {
            return;
        };
        cs.grant_layout = true;
        doc.borrow_mut()
            .set_attribute(content_box, "style", &box_style(limit, cs.fill));
        self.resolve_and_repaint(viewport.0, viewport.1);
    }

    /// Whether the document is in the content-sized mode.
    pub(crate) fn is_content_sized(&self) -> bool {
        self.content_sized.is_some()
    }

    /// Measure again at the next [`Self::content_size_changed`], as after a
    /// change of the document: a new negotiation that may want another size.
    pub(crate) fn remeasure_content(&mut self) {
        if let Some(cs) = self.content_sized.as_mut() {
            cs.document_changed = true;
        }
    }

    /// The content's size from the last layout, without asking for it: the
    /// size a hidden content-sized window is prepared and shown at. `None`
    /// off the mode or before the mount.
    pub(crate) fn measured_content_size(&self) -> Option<(u32, u32)> {
        self.measure_content()
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
        if std::mem::take(&mut cs.grant_layout) {
            return;
        }
        let tree = &doc.borrow().tree;
        if !tree.dirty_nodes.is_empty() || !tree.style_dirty_nodes.is_empty() {
            cs.document_changed = true;
        }
    }

    /// The content box's border box, or its scrollable extent where larger,
    /// rounded up to whole logical pixels, at most the largest size; on an
    /// axis the box fills, the viewport's.
    fn measure_content(&self) -> Option<(u32, u32)> {
        let cs = self.content_sized.as_ref()?;
        let content_box = cs.content_box?.0;
        let doc = self.doc.as_ref()?;
        let d = doc.borrow();
        let (_, _, bw, bh) = super::hit_testing::painted_element_box(&d.tree, content_box);
        let (ew, eh) = rinch_dom::paint::scrollbar::content_extents(&d.tree, content_box);
        let viewport = d.tree.viewport;
        let axis = |fill: bool, viewport: f32, border: f32, extent: f64, max: f32| {
            if fill {
                viewport.ceil() as u32
            } else {
                (border as f64).max(extent).min(max as f64).ceil() as u32
            }
        };
        Some((
            axis(cs.fill.0, viewport.width, bw, ew, cs.max.0),
            axis(cs.fill.1, viewport.height, bh, eh, cs.max.1),
        ))
    }

    /// The content box, for tests.
    #[cfg(test)]
    pub(crate) fn content_box(&self) -> Option<NodeId> {
        self.content_sized.as_ref()?.content_box
    }

    /// The box's limit, for tests.
    #[cfg(test)]
    pub(crate) fn content_box_limit(&self) -> Option<(f32, f32)> {
        Some(self.content_sized.as_ref()?.limit)
    }
}

#[cfg(test)]
#[path = "content_size_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "content_size_body_tests.rs"]
mod body_tests;

#[cfg(test)]
#[path = "content_size_limit_tests.rs"]
mod limit_tests;
