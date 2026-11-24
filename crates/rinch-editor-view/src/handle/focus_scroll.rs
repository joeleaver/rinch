//! Caret placement after layout, focus and blur, scrolling the caret into
//! view, reveals, margins and the overlays.

use super::*;

impl EditorHandle {
    /// Phase-2 projection: render caret/selection geometry from the current state.
    /// The runtime calls this **after** layout (design A3); headless it is a no-op.
    /// Returns whether an overlay actually moved (so the runtime can force a full
    /// repaint — the overlays are absolutely positioned and the software renderer's
    /// dirty-region cache can't clear their old rect).
    ///
    /// This is also where the view's [`ViewRequest`]s are *fulfilled*. The view owns
    /// no window and no scroll container, so it hands back what it needs as data
    /// (design §6); the handle is the nearest thing to a runtime that both platforms
    /// share, so it turns a scroll into a
    /// [`scroll_into_view`](rinch_core::dom::NodeHandle::scroll_into_view) on the
    /// view's *scroll anchor* — which element that is stays the view's knowledge.
    /// The backend then does the minimal "nearest" scroll (immediately on web,
    /// after the next layout on desktop).
    ///
    /// **When it scrolls is decided here, not by the view** (`ScrollGate`): only
    /// after a local edit or selection move — ProseMirror's `tr.scrollIntoView()`. The view's own
    /// `ScrollSelectionIntoView` means only "the overlay moved", which a resize
    /// reflow, a remote edit above the caret or a virtualized block being measured
    /// all cause without the caret moving in the document; scrolling on those
    /// would pull a user who scrolled away back to the caret.
    pub fn update_caret(&self) -> bool {
        let mut guard = self.core_mut();
        let core = &mut *guard;
        let affinity = core.caret_affinity();
        let Some(view) = core.view.as_mut() else {
            return false;
        };
        view.set_caret_affinity(affinity);
        let state = &core.state;
        // The view's `ScrollSelectionIntoView` says only that the overlay moved;
        // whether to scroll is the gate's decision (see above), so the requests
        // are read for nothing else.
        for request in view.update_caret(state) {
            match request {
                ViewRequest::ScrollSelectionIntoView => {}
            }
        }
        let range = matches!(&state.selection, Selection::Text(_)) && !state.selection.is_empty();
        let anchor = view.scroll_anchor();
        if core.scroll.on_pass(anchor.is_some(), range)
            && let Some(anchor) = anchor
        {
            anchor.scroll_into_view();
        }
        let moved = view.take_overlay_dirty();
        if moved && core.on_caret_moved.is_some() {
            // Called once the guard releases the borrow.
            core.caret_moved_owed = true;
        }
        moved
    }

    /// Give this editor the keyboard, as a press in it would, without moving its
    /// selection: typing, the arrows, the clipboard chords and an IME go to it
    /// from now on, and its caret or selection highlight is drawn. Whatever held
    /// the keyboard before loses it the way a click elsewhere takes it (an
    /// `<input>` commits its change, another editor hides its caret).
    ///
    /// It never **asks** for a scroll — call [`Self::scroll_into_view`] for
    /// that — so giving the keyboard back to an editor after a dialog closes
    /// does not jump to a caret the user scrolled away from. It does
    /// **perform a caret scroll already owed**: a [`set_selection`](Self::set_selection)
    /// of a caret (or an [`update`](Self::update) that sets one) on a blurred
    /// editor asks for its caret to be shown, and only the focused editor's
    /// caret pass can show it, so that scroll waits for the focus. So
    /// `set_selection(Selection::cursor(pos)); focus()` shows the caret, while
    /// a range selection — which never owes a scroll — needs
    /// [`Self::scroll_into_view`].
    ///
    /// - **Desktop** posts a focus request (the same one
    ///   [`NodeHandle::focus`](rinch_core::dom::NodeHandle::focus) posts, which
    ///   now focuses an editor container too). The runtime applies it after the
    ///   current event or effect, through the focus arbiter, as
    ///   `FocusTarget::Editor`: right after the handler of a click or of an
    ///   Enter/Space activation, otherwise before the next key press, pointer
    ///   press or IME event is routed, and at the latest on the next turn of
    ///   the event loop. [`selection`](Self::selection) is unaffected either way. A [`blur`](Self::blur) in the same turn is
    ///   applied in call order with it: the later of the two wins.
    /// - **Web** focuses the editor's hidden capture textarea on the spot, as a
    ///   mousedown in the editor does, with `preventScroll`.
    ///
    /// **Before mount, a no-op**: there is nothing to focus yet, and nothing is
    /// remembered for the mount. Focus after the `Editor {}` has rendered.
    /// Focusing the editor that already has the keyboard changes nothing.
    pub fn focus(&self) {
        let Some(container) = self.core().view.as_ref().map(|v| v.container().clone()) else {
            return;
        };
        match crate::registry::focus_handler() {
            // The web: the platform focuses its capture field itself.
            Some(focus) => focus(container.node_id().0),
            // Desktop: the runtime's focus request drain maps an editor
            // container to `FocusTarget::Editor`.
            None => container.focus(),
        }
    }

    /// Take the keyboard away from this editor, leaving it with **nothing** —
    /// what a press on page content that takes no focus does: typing, the
    /// arrows, the clipboard chords and an IME stop reaching it, and its caret
    /// and selection highlight are hidden, as on any blur. The selection itself
    /// is kept, so a later [`focus`](Self::focus) shows it again and typing
    /// continues where it left off.
    ///
    /// Only this editor lets go: **if it does not hold the keyboard, nothing
    /// happens** — another editor, an `<input>` or any other control that holds
    /// it keeps it. That is the call for "this pane no longer takes keys" in an
    /// app with several editors.
    ///
    /// - **Desktop** posts a release that the runtime applies through the
    ///   focus arbiter at the same point as [`focus`](Self::focus)'s request:
    ///   right after the handler of a click or of an Enter/Space activation,
    ///   otherwise before the next key press, pointer press or IME event is
    ///   routed (so a key already queued behind the one whose handler blurred
    ///   does not reach this editor), and at the latest on the next turn of
    ///   the event loop. The two are applied **in the order they were
    ///   called**, so `focus(); blur()` in one handler or effect leaves
    ///   nothing focused, `blur(); focus()` leaves the editor focused, and
    ///   neither call discards a focus request posted by anything else (an
    ///   overlay opening keeps its focus move). Whether the editor holds the
    ///   keyboard is judged then, after the requests before it. Effects a
    ///   signal write queued earlier in the same handler run first, as before
    ///   any `NodeHandle` call, so `open.set(false); editor.blur()` releases
    ///   after the closing overlay handed focus back. A composition in
    ///   progress is dropped with the keyboard (the preedit is cleared, not
    ///   committed).
    /// - **Web** blurs the editor's hidden capture textarea on the spot, if
    ///   this editor is the focused one and the textarea is the document's
    ///   active element; focus goes to the page body, as a browser's own
    ///   `blur()` does. The textarea is blurred while the editor is still the
    ///   focused one, so the `compositionend` a browser raises inside the
    ///   focus change commits a composition in progress, as a press that
    ///   moves focus away does.
    ///
    /// **Before mount, a no-op.** Blurring an editor that is not focused
    /// changes nothing.
    pub fn blur(&self) {
        // Program order (the `NodeHandle` rule): an effect queued by a signal
        // write before this call — an overlay closing and handing focus back,
        // an effect that focuses — posts its focus work first.
        rinch_core::reactive::flush_pending_effects();
        let Some((container, doc_key)) = self
            .core()
            .view
            .as_ref()
            .map(|v| (v.container().node_id().0, v.doc_key()))
        else {
            return;
        };
        match crate::registry::blur_handler() {
            // The web: the platform blurs its capture field itself.
            Some(blur) => blur(container),
            // Desktop: the runtime's focus drain releases the arbiter's
            // `FocusTarget::Editor` if this container still holds it.
            None => rinch_core::post_release_request(doc_key, container),
        }
    }

    /// Scroll so that `from..to` is on screen: its start, and as much of the
    /// range after it as fits, kept a 16px margin inside the scroller's edge
    /// where the document extends that far. Nothing moves when it is already in view.
    /// `from == to` reveals a caret position. Neither the selection nor focus
    /// is touched, and the editor need not be focused.
    ///
    /// This is how an app shows what it selected: [`Self::set_selection`]
    /// brings a *caret* into view on its own (on the focused editor's next
    /// caret pass), but never a range — a Shift+arrow head is not revealed —
    /// so after selecting the words a link points at, ask for them:
    ///
    /// ```ignore
    /// handle.set_selection(Selection::text(from, to));
    /// handle.scroll_into_view(from, to);
    /// handle.focus();
    /// ```
    ///
    /// The request is kept on the handle and fulfilled on the first overlay
    /// pass that has geometry for its start. The end is revealed with it when
    /// it has geometry too; when it does not (a virtualized editor whose end
    /// block is still collapsed), the start alone is revealed. It uses the same scroll the caret
    /// uses ([`NodeHandle::scroll_into_view`](rinch_core::dom::NodeHandle::scroll_into_view)),
    /// on two hidden probe boxes the view places over the range's end and then
    /// its start:
    ///
    /// - **Desktop** applies it after the next layout (the call wakes the
    ///   runtime for one). A virtualized editor lays out the blocks holding
    ///   `from` and `to` for it.
    /// - **Web** applies it immediately (the call runs the overlay refresh an
    ///   input event would).
    ///
    /// Both move every scroll container the range is in, the page included
    /// (on desktop since #842).
    ///
    /// A later call replaces an unfulfilled one. The range is carried through
    /// local edits made before it is fulfilled and dropped by
    /// [`load_doc`](Self::load_doc)/[`load_html`](Self::load_html); a peer's
    /// edit leaves the positions as they are (clamped to the document).
    /// Positions between blocks reveal the nearest text position.
    ///
    /// **A reveal that cannot be fulfilled does not wait for good.** In an
    /// editor that is not rendered (`display: none`, an inactive tab) its
    /// boxes are laid out at zero size, on both backends, so the first pass
    /// consumes the request and its scroll moves nothing. If instead
    /// [`REVEAL_PATIENCE`] overlay passes in a row find no geometry at all for
    /// the start — an editor whose blocks are never laid out — the request is
    /// dropped. Either way it is not kept to scroll the view much later, once
    /// the editor is shown and the user has moved on: ask again then.
    /// Overlay passes run with the runtime's frames and input events; an idle
    /// app runs none and uses none of the patience.
    ///
    /// **Before mount, a no-op.**
    ///
    /// The same as [`Self::scroll_into_view_aligned`] with
    /// [`ScrollAlign::Nearest`]; to put the range at a set place in the view
    /// (a third of the way down, say, for context above it), use that.
    pub fn scroll_into_view(&self, from: Pos, to: Pos) {
        self.scroll_into_view_aligned(from, to, ScrollAlign::Nearest);
    }

    /// [`Self::scroll_into_view`] with a choice of where the range lands:
    ///
    /// - [`ScrollAlign::Nearest`] — the least scroll that shows the range, as
    ///   `scroll_into_view` does; a range already in view moves nothing.
    /// - [`ScrollAlign::Fraction(f)`](ScrollAlign::Fraction) — the start's line
    ///   `f` of the way down the scroll container's visible height (`0.0` the
    ///   top, `1.0 / 3.0` a third down; clamped to `0.0..=1.0`), but never
    ///   nearer than 16px to either edge. It scrolls even when the range is
    ///   already in view. The scroll is clamped to what the content allows, so
    ///   a range near the top of the document stays near the top of the view
    ///   (and one near the end, lower down). A range taller than the space
    ///   below `f` still puts its start at `f`; the rest runs off the bottom.
    ///
    /// ```ignore
    /// // Open a note at a deep link's words, with context above them.
    /// handle.set_selection(Selection::text(from, to));
    /// handle.scroll_into_view_aligned(from, to, ScrollAlign::Fraction(1.0 / 3.0));
    /// handle.focus();
    /// ```
    ///
    /// Everything else — when it happens, what waits and what is carried or
    /// dropped, which containers move — is as [`Self::scroll_into_view`]
    /// describes, with one difference for `Fraction`: the nearest vertical
    /// scroll container (the page's scrolling element when no ancestor scrolls)
    /// is set to the computed `scrollTop`, and the start is then brought into
    /// view of the scrollers further out, the page included, the "nearest" way.
    pub fn scroll_into_view_aligned(&self, from: Pos, to: Pos, align: ScrollAlign) {
        let doc_key = {
            let mut core = self.core_mut();
            let Some(doc_key) = core.view.as_ref().map(|v| v.doc_key()) else {
                return;
            };
            core.reveal = Some((from.min(to), from.max(to), align.clamped()));
            core.reveal_misses = 0;
            doc_key
        };
        crate::registry::owe_overlay_pass(doc_key);
        crate::registry::request_overlay_refresh();
    }

    /// Fulfil a pending [`Self::scroll_into_view`], if there is one and both
    /// ends have geometry: place the view's probes and scroll to them. Called by
    /// [`update_all_carets`](crate::update_all_carets) for every mounted editor,
    /// focused or not. Returns whether it wrote to the document (the runtime
    /// then lays out again before it applies the queued scrolls).
    pub(crate) fn reveal_pass(&self) -> bool {
        if self.inner.borrow().reveal.is_none() {
            return false;
        }
        let mut guard = self.core_mut();
        let core = &mut *guard;
        let (Some((from, to, align)), Some(view)) = (core.reveal, core.view.as_mut()) else {
            return false;
        };
        let Some(probes) = view.position_reveal(&core.state.doc, from, to, align) else {
            core.reveal_misses += 1;
            if core.reveal_misses >= REVEAL_PATIENCE {
                core.reveal = None;
            }
            return false;
        };
        core.reveal = None;
        drop(guard);
        for probe in probes {
            match align {
                ScrollAlign::Nearest => probe.scroll_into_view(),
                ScrollAlign::Fraction(f) => {
                    probe.scroll_to_fraction(f, crate::view::REVEAL_MARGIN);
                }
            }
        }
        true
    }

    /// Measure the [margin anchors](Self::set_margin_anchors)' lines and, when
    /// the report changed, call [`Self::on_margin_layout`]. Called by
    /// [`update_all_carets`](crate::update_all_carets) for every mounted
    /// editor, before any caret pass writes an overlay. Returns whether the
    /// callback ran (the app may have moved its markers: the runtime then lays
    /// out again).
    ///
    /// One check and nothing more for an editor with no anchors and nothing
    /// reported. While the host has layout pending (desktop: a pass made
    /// before the layout an edit owes), nothing is measured and the pass is
    /// owed again, so lines are never reported where they are about to leave.
    pub(crate) fn margin_pass(&self) -> bool {
        {
            let Ok(core) = self.inner.try_borrow() else {
                return false;
            };
            if core.margins.is_idle()
                || (core.on_margin_layout.is_none() && core.margins.observed.is_none())
            {
                return false;
            }
        }
        let mut guard = self.core_mut();
        let core = &mut *guard;
        // While anchors are set and someone listens, the platform watches the
        // container's size (a reflow the editor did not cause: a resize).
        let want = (core.on_margin_layout.is_some() && !core.margins.anchors.is_empty())
            .then(|| {
                core.view
                    .as_ref()
                    .map(|v| (v.doc_key(), v.container().clone()))
            })
            .flatten();
        let mut notify: Vec<(u64, NodeHandle, bool)> = Vec::new();
        let observed = core.margins.observed.as_ref().map(|(_, n)| n.node_id());
        if observed != want.as_ref().map(|(_, n)| n.node_id()) {
            if let Some((dk, old)) = core.margins.observed.take() {
                notify.push((dk, old, false));
            }
            if let Some((dk, new)) = want {
                notify.push((dk, new.clone(), true));
                core.margins.observed = Some((dk, new));
            }
        }
        let mut report: Option<(MarginHook, Vec<MarginLine>)> = None;
        if let (Some(hook), Some(view)) = (core.on_margin_layout.clone(), core.view.as_ref()) {
            match view.layout_pending() {
                Some(false) => {
                    let doc = core.state.doc.clone();
                    // A position anchor only moves with the document (or a
                    // new set, which forgets it): the same document, the same
                    // positions. A sticky anchor also stops resolving when the
                    // session stops, stalls or is poisoned, with the document
                    // unchanged, so it is resolved every pass.
                    let positions = match &core.margins.key {
                        Some((d, positions, _))
                            if d.same_ref(&doc) && !core.margins.has_sticky() =>
                        {
                            positions.clone()
                        }
                        _ => Self::margin_positions(core),
                    };
                    let at: Vec<Pos> = positions.iter().flatten().copied().collect();
                    let key = view.margin_key(&doc, &at);
                    // Nothing the lines are read from moved since the last
                    // measure (a layout of something else): no measure.
                    let unmoved = core.margins.last.is_some()
                        && key.is_some()
                        && core.margins.key.as_ref().is_some_and(|(d, p, k)| {
                            d.same_ref(&doc) && Some(*k) == key && *p == positions
                        });
                    if !unmoved {
                        let (boxes, right) = view.measure_lines(&doc, &at);
                        margins::count_pass(at.len());
                        let mut boxes = boxes.into_iter();
                        let lines: Vec<MarginLine> = core
                            .margins
                            .anchors
                            .iter()
                            .zip(&positions)
                            .filter_map(|(a, p)| {
                                p.as_ref()?;
                                let (top, height) = boxes.next().flatten()?;
                                Some(MarginLine {
                                    key: a.key.clone(),
                                    top,
                                    height,
                                })
                            })
                            .collect();
                        let changed = core
                            .margins
                            .last
                            .as_ref()
                            .is_none_or(|(l, r)| *l != lines || *r != right);
                        core.margins.key = key.map(|k| (doc, positions, k));
                        if changed {
                            core.margins.last = Some((lines.clone(), right));
                            report = Some((hook, lines));
                        }
                    }
                }
                // Lines are about to move (or the host is busy): wait for the
                // pass after the layout.
                _ => registry::owe_overlay_pass(view.doc_key()),
            }
        }
        drop(guard);
        for (doc_key, node, mounted) in notify {
            registry::notify_node_view_host(doc_key, node.node_id().0, &node, mounted);
        }
        match report {
            Some((hook, lines)) => hook.invoke(|cb| cb(&lines)).is_some(),
            None => false,
        }
    }

    /// Where each margin anchor is now, in the order of the set: a
    /// position anchor's carried position, a sticky index resolved (all of
    /// them in one walk of the shared document). `None` for one with no place.
    fn margin_positions(core: &EditorCore) -> Vec<Option<Pos>> {
        #[cfg(feature = "collaboration")]
        let mut sticky = {
            let bytes: Vec<&[u8]> = core
                .margins
                .anchors
                .iter()
                .filter_map(|a| match &a.at {
                    margins::LiveAt::Sticky(b) => Some(b.as_slice()),
                    _ => None,
                })
                .collect();
            let located = match (bytes.is_empty(), core.collab.as_ref()) {
                (false, Some(bridge)) => bridge.session.locate_sticky_many(&core.state.doc, &bytes),
                _ => vec![None; bytes.len()],
            };
            located.into_iter()
        };
        core.margins
            .anchors
            .iter()
            .map(|a| match &a.at {
                margins::LiveAt::Pos(pos, _) => *pos,
                #[cfg(feature = "collaboration")]
                margins::LiveAt::Sticky(_) => sticky.next().flatten().map(|l| l.pos),
            })
            .collect()
    }

    /// Stop watching the container for the margin pass and forget the last
    /// report, so a re-mount reports afresh. Called when the editor unmounts.
    pub(super) fn release_margins(&self) {
        let observed = {
            let Ok(mut core) = self.inner.try_borrow_mut() else {
                return;
            };
            core.margins.forget_report();
            core.margins.observed.take()
        };
        if let Some((doc_key, node)) = observed {
            registry::notify_node_view_host(doc_key, node.node_id().0, &node, false);
        }
    }

    /// The start of a pending [`Self::scroll_into_view`] range, if one is
    /// waiting for geometry. Desktop block virtualization keeps the block
    /// holding it laid out, so a reveal of a collapsed block can be fulfilled.
    #[doc(hidden)]
    pub fn pending_reveal(&self) -> Option<Pos> {
        self.inner.borrow().reveal.map(|(from, _, _)| from)
    }

    /// Whether the caret owes a scroll into view that no caret pass has
    /// fulfilled yet (the [`ScrollGate`] is armed) — a caret move whose block
    /// has no geometry to scroll to. Desktop block virtualization lays out the
    /// block holding the caret then (#845): the window follows the scroll
    /// position, so it would never reach a block the caret moved to.
    #[doc(hidden)]
    pub fn caret_scroll_owed(&self) -> bool {
        self.inner.borrow().scroll.pending
    }

    /// Owe the caret a scroll into view again, as a caret move does: the next
    /// caret pass of the focused editor scrolls it into view if it is not.
    /// Desktop block virtualization calls it while the blocks around a caret
    /// it scrolled to by their *estimated* heights are being measured (#845):
    /// their real heights move the caret, and the scroll that was fulfilled on
    /// the estimate has to follow it.
    #[doc(hidden)]
    pub fn owe_caret_scroll(&self) {
        self.inner.borrow_mut().scroll.arm();
    }

    /// Both ends of a pending [`Self::scroll_into_view`] range, `from <= to`.
    /// Desktop block virtualization keeps the blocks holding both laid out.
    #[doc(hidden)]
    pub fn pending_reveal_range(&self) -> Option<(Pos, Pos)> {
        self.inner.borrow().reveal.map(|(from, to, _)| (from, to))
    }

    /// Show where an OS file drag hovering over this editor would land — a
    /// caret bar at `at` (a position and the side of a soft wrap), drawn apart
    /// from the caret — or hide it (`None`). The selection does not move and
    /// no scroll is asked for: the drop itself does both. Called by the
    /// desktop runtime while a picture file is dragged over an editor that
    /// would take it. Answers whether the overlay changed (the runtime then
    /// lays out and repaints). A no-op before mount.
    #[doc(hidden)]
    pub fn show_drop_caret(&self, at: Option<(Pos, CaretAffinity)>) -> bool {
        let mut guard = self.core_mut();
        let core = &mut *guard;
        match core.view.as_mut() {
            Some(view) => view.show_drop_caret(&core.state.doc, at),
            None => false,
        }
    }

    /// Hide this editor's overlays (caret + selection highlight) because it isn't
    /// focused. The runtime's focus-aware caret pass calls this for every editor
    /// that isn't the focused one. A no-op before mount. Returns whether an overlay
    /// was actually cleared (so the runtime can schedule a repaint).
    pub fn hide_overlays(&self) -> bool {
        match self.core_mut().view.as_mut() {
            Some(view) => {
                view.hide_overlays();
                view.take_overlay_dirty()
            }
            None => false,
        }
    }

    /// Apply a caret blink phase (the runtime's blink driver calls this each
    /// half-period). Returns `None` if there is no caret to blink (no collapsed
    /// cursor), `Some(true)` if the caret's visibility actually toggled (repaint
    /// needed), or `Some(false)` if the phase was already applied.
    pub fn set_caret_blink(&self, visible: bool) -> Option<bool> {
        self.core_mut()
            .view
            .as_mut()?
            .set_caret_blink_visible(visible)
    }

    // ── IME (input method editor) ────────────────────────────────────────────
    //
    // The composition (preedit) is a **view-local overlay** that is never part of
    // the document (design A5): it is shown at the caret while composing and
    // discarded on commit or clear. Commit inserts the final text as one ordinary
    // edit, so undo/history treat it exactly like typing.
}
