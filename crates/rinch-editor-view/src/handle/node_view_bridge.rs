//! The handle's side of node views: registering views, embeds, keys and
//! arrow motion into and out of a hosted view, and returning focus.

use super::*;

impl EditorHandle {
    /// Draw nodes of `spec.node_type` with an app's own component — a node view
    /// (see `crate::node_views`). For every such node whose attributes the spec
    /// [`matches`](NodeViewSpec::matches) — the **first** registered spec that
    /// matches wins — `render` builds the component inside the node's host
    /// element, now for the nodes already in the document and later for every one
    /// that arrives.
    ///
    /// The view is mounted while the node stays the same node: through typing
    /// elsewhere and through changes to its own attributes (local, undone, redone,
    /// or a collaborator's), which its [`NodeViewCtx`](crate::NodeViewCtx) memos
    /// follow. It is disposed when the node leaves the document, and remounted
    /// when anything but an attribute change puts a node in its place — a view is
    /// never handed another node. Pointer and keyboard events inside the view are
    /// the view's; a press on the host's frame, or an arrow onto it, node-selects
    /// the node.
    ///
    /// Released when the returned registration is dropped — and, when made while
    /// a component renders, when that component unmounts: call
    /// [`NodeViewRegistration::keep`] to tie it to that alone. Releasing it
    /// unmounts every view it drew (another spec that matches takes over).
    pub fn register_node_view(&self, spec: NodeViewSpec) -> NodeViewRegistration {
        let registry = self.node_view_registry();
        let token = registry.add(spec);
        let weak = self.weak();
        rinch_core::reactive::on_cleanup(move || {
            if let Some(handle) = weak.upgrade() {
                handle.unregister_node_view(token);
            }
        });
        if let Some(view) = self.core_mut().view.as_mut() {
            view.rescan_node_views();
        }
        NodeViewRegistration::new(self.weak(), token)
    }

    /// Release the spec registered with `token`.
    pub(crate) fn unregister_node_view(&self, token: u64) {
        if !self.node_view_registry().remove(token) {
            return;
        }
        if let Ok(mut core) = self.try_core_mut()
            && let Some(view) = core.view.as_mut()
        {
            view.rescan_node_views();
        }
    }

    /// Insert an `embed` (the starter kit's block atom an app draws with a node
    /// view) carrying `attrs` at the selection, as `insertHorizontalRule` inserts a
    /// rule: a caret inside a paragraph splits it, a range is replaced. The caret
    /// lands after it. `false` when the schema has no `embed`, the attributes are
    /// refused, or the editor refuses edits ([read-only](Self::set_read_only)).
    pub fn insert_embed(&self, attrs: rinch_editor_core::Attrs) -> bool {
        self.dispatch(
            |state| {
                let node = state
                    .schema()
                    .create_node("embed", attrs, Fragment::empty())
                    .ok()?;
                let mut tr = state.tr();
                tr.insert_block(node).ok()?;
                Some(tr)
            },
            true,
        )
    }

    /// Whether `host_id` is the host element of a mounted node view of this
    /// editor.
    pub fn is_node_view_host(&self, host_id: u64) -> bool {
        self.core()
            .view
            .as_ref()
            .and_then(|v| v.slot_by_host(host_id))
            .is_some_and(|s| s.is_mounted())
    }

    /// A key pressed while the keyboard is inside the node view whose host is
    /// `host_id`: offered to its [`on_key`](NodeViewSpec::on_key) first, and an
    /// Escape it leaves gives the keyboard back to the editor with the caret
    /// right after the node. `true` when the key was consumed either way — the
    /// platform then stops it. The platform's entry point, called before the
    /// element that holds the keyboard sees the key.
    pub fn offer_node_view_key(&self, host_id: u64, key: &EditorKey<'_>) -> bool {
        let Some(slot) = self
            .core()
            .view
            .as_ref()
            .and_then(|v| v.slot_by_host(host_id))
        else {
            return false;
        };
        let registry = self.node_view_registry();
        if node_views::offer_key(&registry, &slot, key) == Some(true) {
            return true;
        }
        if key.key == "Escape" {
            if let Some(pos) = self.node_view_pos(&slot) {
                self.return_focus_after(Pos(pos));
            }
            return true;
        }
        false
    }

    /// Set `selection`, reached by an arrow key moving the caret `from` a side,
    /// and — when it node-selects a node a view is mounted for — offer the view
    /// the caret ([`NodeViewSpec::on_caret_enter`]). Whether the view took the
    /// keyboard. The platforms' vertical motion calls this; horizontal motion
    /// ([`Self::move_cursor`]) does it itself.
    pub fn select_from_arrow(&self, selection: Selection, from: CaretFrom) -> bool {
        self.set_selection(selection.clone());
        let Selection::Node(n) = selection else {
            return false;
        };
        let Some(slot) = ({
            let core = self.core();
            core.view
                .as_ref()
                .and_then(|v| v.slot_at(&core.state.doc, n.anchor))
        }) else {
            return false;
        };
        node_views::offer_caret_enter(&self.node_view_registry(), &slot, from)
    }

    /// The node selection an Up (`down = false`) or Down arrow makes when the
    /// block right above or below the caret's textblock (or the selected node) is
    /// a node with a **mounted view**: vertical motion steps over a horizontal rule
    /// ([`Selection::near_text`]), but must not step over an app's component, or
    /// it could not be reached from the keyboard. At the last (first) block of a
    /// list item, a quote or a list, the block below (above) is the one after
    /// (before) that container. `None` otherwise.
    pub fn vertical_node_view(&self, down: bool) -> Option<Selection> {
        let core = self.core();
        let doc = &core.state.doc;
        let sel = &core.state.selection;
        let mut edge = match sel {
            Selection::Node(n) if down => n.head.0,
            Selection::Node(n) => n.anchor.0,
            _ => {
                let r = doc.resolve(sel.head()).ok()?;
                if !r.parent().is_textblock() {
                    return None;
                }
                if down {
                    r.after(r.depth())?
                } else {
                    r.before(r.depth())?
                }
            }
        };
        // The block right below (above): at the edge of a list item, a quote or
        // a list, the next one out — Down from a list's last line reaches the
        // node after the list (review of #1575, F6).
        let at = loop {
            let r = doc.resolve(Pos(edge)).ok()?;
            let next = if down {
                r.node_after()
            } else {
                r.node_before()
            };
            match next {
                Some(_) if down => break edge,
                Some(n) => break edge - n.node_size(),
                None if r.depth() > 0 => {
                    edge = if down {
                        r.after(r.depth())?
                    } else {
                        r.before(r.depth())?
                    };
                }
                None => return None,
            }
        };
        let view = core.view.as_ref()?;
        view.has_mounted_view_at(doc, Pos(at))
            .then(|| Selection::node_at(doc, Pos(at)))
            .flatten()
    }

    /// Give the keyboard to the editor with the caret right after the node at
    /// `pos`: the first text position after it, or the node itself, selected, when
    /// no textblock follows it.
    pub(crate) fn return_focus_after(&self, pos: Pos) {
        let doc = self.doc();
        let Some(node) = doc.node_at(pos.0) else {
            return;
        };
        let after = pos.0 + node.node_size();
        let sel = Selection::near_text(&doc, Pos(after), 1)
            .filter(|s| s.head().0 >= after)
            .or_else(|| Selection::node_at(&doc, pos));
        if let Some(sel) = sel {
            self.set_selection(sel);
        }
        self.focus();
    }

    /// A weak handle to this editor.
    pub(crate) fn weak(&self) -> WeakHandle {
        WeakHandle(Rc::downgrade(&self.inner))
    }

    /// The editor's node-view registry.
    pub(crate) fn node_view_registry(&self) -> Rc<node_views::Registry> {
        self.inner.borrow().node_views.clone()
    }

    /// The editor's document key and container, while mounted.
    pub(crate) fn node_view_host_cx(&self) -> Option<node_views::HostCx> {
        Self::host_cx_of(&self.inner.borrow())
    }

    pub(super) fn host_cx_of(core: &EditorCore) -> Option<node_views::HostCx> {
        let view = core.view.as_ref()?;
        Some(node_views::HostCx {
            doc_key: view.doc_key(),
            container: view.container_id(),
        })
    }

    /// Swap the host element of the inline leaf `slot` is for (see
    /// [`RinchDomEditorView::swap_inline_host`]). Called with no borrow held, from
    /// [`node_views::run`]; `false` when the editor is busy or the slot is gone.
    pub(crate) fn swap_node_view_host(&self, slot: &Rc<node_views::Slot>, view_host: bool) -> bool {
        let Ok(mut core) = self.try_core_mut() else {
            return false;
        };
        let Some(view) = core.view.as_mut() else {
            return false;
        };
        let swapped = view.swap_inline_host(slot, view_host);
        if swapped {
            // The caret beside it and the node outline are drawn from its box.
            crate::registry::owe_overlay_pass(view.doc_key());
        }
        drop(core);
        if swapped {
            crate::registry::request_overlay_refresh();
        }
        swapped
    }

    /// The position just before the node `slot` is for.
    pub(crate) fn node_view_pos(&self, slot: &Rc<node_views::Slot>) -> Option<usize> {
        self.core().view.as_ref()?.slot_pos(slot)
    }

    /// Dispose every node view — the editor's unmount.
    pub(crate) fn dispose_node_views(&self) {
        let (slots, host_cx) = {
            let Ok(mut core) = self.try_core_mut() else {
                return;
            };
            let host_cx = Self::host_cx_of(&core);
            (
                core.view
                    .as_mut()
                    .map(RinchDomEditorView::dispose_node_views)
                    .unwrap_or_default(),
                host_cx,
            )
        };
        node_views::dispose_all(slots, host_cx.as_ref());
    }
}
