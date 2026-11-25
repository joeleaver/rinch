//! [`DomFocus`] for [`RinchDocument`]: focus, text-control selection and
//! the work deferred until the borrow is released.

use rinch_core::dom::{CaretAffinity, DomFocus, DomTree, NodeId};

use super::RinchDocument;

impl DomFocus for RinchDocument {
    fn focus_element(&mut self, node_id: NodeId) {
        // Request focus via the event system - the runtime will apply it.
        // Keyed by this document so another document's runtime on the same
        // thread doesn't consume it (issue #134).
        rinch_core::request_focus(self.doc_key, node_id.0);
    }

    /// Posts the request via the same kind of channel `focus_element`
    /// uses, keyed by this document (issue #134) — but a **separate** one
    /// from [`FocusRequest`](rinch_core::FocusRequest), so a `focus()` and a
    /// `set_selection_range()`/`select()` posted from the same effect (the
    /// "open a rename box, focused with its name selected" shape #552 exists
    /// for) both survive to be applied, rather than the later post silently
    /// discarding the earlier one in a shared slot.
    ///
    /// The runtime (`RinchApp`) applies this directly to the focused input's
    /// `EditableState` if `node_id` already holds the keyboard, converting
    /// the UTF-16 offsets to the byte offsets that state keeps; otherwise it
    /// stashes the request for the next time `node_id` is focused
    /// (`try_focus_input` consults the stash instead of defaulting to a caret
    /// at the text's end). `start`/`end` are passed through unchanged — the
    /// runtime is what knows the control's text and can convert and swap.
    fn set_selection_range(
        &mut self,
        node_id: NodeId,
        start: usize,
        end: usize,
        direction: rinch_core::dom::SelectionDirection,
    ) {
        rinch_core::post_text_selection_request(self.doc_key, node_id.0, start, end, direction);
    }

    /// Newest first (issue #1678): a request posted and not yet applied, the
    /// focused field's live selection (the bytes the runtime syncs into
    /// `data-selection-start` / `data-cursor-pos`), the one remembered when
    /// it last lost the keyboard or stashed for its next focus
    /// (`NodeTree::text_selections`), else a caret at the start. Clamped to
    /// the current value, `start <= end`.
    fn text_selection_range(
        &self,
        node_id: NodeId,
    ) -> Option<(usize, usize, rinch_core::dom::SelectionDirection)> {
        use crate::form_control;
        use rinch_core::dom::SelectionDirection;
        let node = self.tree.get(node_id.0)?;
        if !form_control::has_text_selection(node) {
            return None;
        }
        let value = form_control::control_value(&self.tree.nodes, node_id.0).unwrap_or_default();
        let len = value.encode_utf16().count();
        // Chrome reads an unspecified direction back as `forward`, before
        // focus as after (and desktop's focused selection has no "none").
        let order = |(a, b, d): (usize, usize, SelectionDirection)| {
            let (a, b) = (a.min(len), b.min(len));
            let d = if d == SelectionDirection::None {
                SelectionDirection::Forward
            } else {
                d
            };
            Some((a.min(b), a.max(b), d))
        };
        if let Some(posted) =
            rinch_core::peek_pending_text_selection_request(self.doc_key, node_id.0)
        {
            return order(posted);
        }
        if node.attributes.get("data-focused").map(String::as_str) == Some("true")
            && let Some(head) = node
                .attributes
                .get("data-cursor-pos")
                .and_then(|v| v.parse::<usize>().ok())
        {
            let anchor = node
                .attributes
                .get("data-selection-start")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(head);
            let (head, anchor) = (
                form_control::utf16_of_byte(&value, head),
                form_control::utf16_of_byte(&value, anchor),
            );
            let direction = if anchor > head {
                SelectionDirection::Backward
            } else {
                SelectionDirection::Forward
            };
            return order((anchor, head, direction));
        }
        if let Some(&kept) = self.tree.text_selections.get(&node_id.0) {
            return order(kept);
        }
        // Untouched (a parsed `value`, a textarea's default value): a caret at
        // the start, Chrome 155's answer. A `value` write moved it to the end
        // above, through `text_selections`.
        Some((0, 0, SelectionDirection::Forward))
    }

    /// The caret paint draws (or would draw) at that offset, through
    /// [`crate::form_control::painted_caret_rect`] with this document's own
    /// font contexts, which carry the app's registered faces (issue #1678).
    fn text_caret_rect(
        &mut self,
        node_id: NodeId,
        offset: Option<usize>,
    ) -> Option<rinch_core::reactive::ElementBounds> {
        use crate::form_control;
        use rinch_core::dom::SelectionDirection;
        if !self.is_rendered(node_id) {
            // No box, no caret (`display: none`, `visibility: hidden`, out of
            // the document).
            return None;
        }
        let node = self.tree.get(node_id.0)?;
        if !form_control::has_text_selection(node) {
            return None;
        }
        let focused = node.attributes.get("data-focused").map(String::as_str) == Some("true");
        let affinity = if focused && offset.is_none() {
            form_control::caret_affinity(node)
        } else {
            CaretAffinity::Downstream
        };
        let units = match offset {
            Some(units) => units,
            None => {
                let (start, end, dir) = self.text_selection_range(node_id)?;
                if dir == SelectionDirection::Backward {
                    start
                } else {
                    end
                }
            }
        };
        let value = form_control::control_value(&self.tree.nodes, node_id.0).unwrap_or_default();
        let byte = form_control::byte_of_utf16(&value, units);
        let (x, y, height) = form_control::painted_caret_rect(
            &self.tree,
            &mut self.font_cx,
            &mut self.layout_cx,
            node_id.0,
            byte,
            affinity,
        )?;
        Some(rinch_core::reactive::ElementBounds {
            x,
            y,
            width: 0.0,
            height,
        })
    }

    /// Whether `node` is shown, from the computed styles of the last resolve
    /// (issue #802; see [`DomFocus::is_rendered`]).
    ///
    /// rinch styles hidden subtrees (#703), so a node under a `display: none`
    /// ancestor has its own `display` and the walk up is what finds the
    /// ancestor; its `visibility` is the inherited one, so the node's own value
    /// answers for `visibility: hidden` ancestors.
    fn is_rendered(&self, node: NodeId) -> bool {
        use crate::computed_style::values::DisplayValue;
        use crate::computed_style::values::VisibilityValue;
        if !self.is_connected(node) {
            return false;
        }
        let Some(n) = self.tree.get(node.0) else {
            return false;
        };
        if matches!(
            n.computed_style.visibility,
            VisibilityValue::Hidden | VisibilityValue::Collapse
        ) {
            return false;
        }
        let mut current = Some(node.0);
        while let Some(id) = current {
            let Some(n) = self.tree.get(id) else {
                return false;
            };
            if n.computed_style.display == DisplayValue::None {
                return false;
            }
            current = n.parent;
        }
        true
    }

    /// The DOM mirror of the focus arbiter's claim (issue #695).
    ///
    /// `focused_node` is what `RinchApp::set_focus_target` and its teardown keep
    /// in step with `FocusTarget`, and it is already the fallback
    /// `RinchApp::tab_trap_root` consults for a claim the arbiter does not model
    /// as a node id. Reading it here means the *document* can answer "who has
    /// focus" without the runtime having to publish its enum.
    fn active_element(&self) -> Option<NodeId> {
        // The editor holding the keyboard is where the focus is (issue #1126),
        // while its container is in the document: the arbiter keeps an
        // `Editor` claim on an editor a branch unmounted until the next key
        // or IME event heals it, and a detached container is nobody's focus.
        self.tree
            .keyboard_editor
            .filter(|&id| self.is_connected(NodeId(id)))
            .or(self.tree.focused_node)
            .map(NodeId)
    }

    /// `showModal()`'s focusing steps (issue #695), parked for the runtime.
    ///
    /// **Parked rather than answered**, because the answer is not knowable yet:
    /// an overlay opening is a class removal in the same effect flush, so at
    /// this instant every node inside `root` still has the zero-size box a
    /// `display: none` ancestor gave it, and every visibility filter — the one
    /// `RinchApp::collect_focusable_nodes_from` applies included — would reject
    /// the lot. The runtime consumes the request after the next
    /// `resolve_and_repaint`, when the boxes are real.
    fn focus_into(&mut self, root: NodeId, policy: rinch_core::dom::FocusIntoPolicy) {
        rinch_core::post_focus_request(
            self.doc_key,
            rinch_core::FocusRequest::Into(root.0, policy),
        );
    }

    /// The close half (issue #695), parked for the same reason as
    /// [`focus_into`](Self::focus_into) and the mirror image of it: whether the
    /// remembered opener can still take the keyboard is a question about boxes,
    /// and the boxes are a layout behind at the moment the overlay's effect runs.
    fn restore_focus(&mut self, opener: Option<NodeId>, root: NodeId) {
        rinch_core::post_focus_request(
            self.doc_key,
            rinch_core::FocusRequest::Restore {
                opener: opener.map(|o| o.0),
                root: root.0,
            },
        );
    }
}
