//! The app's callback slots: change, link, image-hover, key, selection,
//! caret-moved and margin hooks.

use super::*;

impl EditorHandle {
    /// Register a callback invoked after a **local edit changes the document**.
    /// Replaces any previously registered callback.
    ///
    /// This is the cross-platform way to know the user edited — for autosave,
    /// dirty-marking or word counts. There is no DOM `input` event to listen for:
    /// the editor is model-first and deliberately never `contenteditable`.
    ///
    /// Fires for every edit path — [`update`](Self::update) (which typing, paste
    /// and IME commit all funnel through), [`command`](Self::command) and
    /// [`insert_image`](Self::insert_image). It does **not** fire for:
    /// - selection-only changes (the document is unchanged),
    /// - [`load_doc`](Self::load_doc) / [`load_html`](Self::load_html) — a
    ///   programmatic load is not a user edit, and firing would make an autosave
    ///   consumer immediately re-save freshly loaded content,
    /// - remote collaboration integration ([`collab_receive`](Self::collab_receive)),
    ///   which is already in the shared CRDT.
    ///
    /// The callback runs with no internal borrow held, so it may freely re-enter
    /// the handle (e.g. call [`doc`](Self::doc) to serialize for a save).
    ///
    /// ```ignore
    /// let editor = create_editor();
    /// editor.on_change({
    ///     let editor = editor.clone();
    ///     move || schedule_autosave(editor.doc())
    /// });
    /// ```
    ///
    /// Registered while a component renders, the callback belongs to that
    /// component: it runs inside it, and is not called any more once it
    /// unmounts, though the handle lives on (#147/#183). Registered outside any
    /// render it keeps app lifetime.
    pub fn on_change(&self, cb: impl Fn() + 'static) {
        self.core_mut().on_change = Some(Hook::new(Rc::new(cb)));
    }

    /// Invoke the change callback, if any, with **no borrow held** — the callback
    /// commonly re-enters the handle, which would otherwise double-borrow.
    pub(super) fn notify_change(&self) {
        let cb = self.core().on_change.clone();
        if let Some(cb) = cb {
            cb.invoke(|cb| cb());
        }
    }

    /// The link carrying the character that starts at `pos`, as the whole run
    /// of adjacent content with the same `href` — [`rinch_editor_core::link_at`]
    /// over the current document. A position just **after** a link's last
    /// character is not on it and the position at its start is, while
    /// [`active_link_href`](Self::active_link_href) reports no link for a caret
    /// at either: this is the question a pointer asks, about the character
    /// under it.
    pub fn link_at(&self, pos: Pos) -> Option<LinkSpan> {
        rinch_editor_core::link_at(&self.core().state.doc, pos)
    }

    /// Register a callback for a primary-button **press on a link**. Replaces
    /// any previously registered callback.
    ///
    /// The runtime calls it when a single press of the primary button lands on
    /// a linked character, **before** it places the caret. Return `true` to
    /// claim the press: the caret does not move, no drag-select starts and the
    /// selection is left exactly as it was (the editor still takes keyboard
    /// focus, as any press in it does). Return `false` and the press is an
    /// ordinary one: it places the caret, extends with Shift, and arms a drag.
    /// So the usual "Ctrl/Cmd+click opens a link, a plain click edits it" is
    ///
    /// ```ignore
    /// editor.on_link_click(move |click| {
    ///     if !click.primary {
    ///         return false; // a plain click puts the caret in the link
    ///     }
    ///     open(&click.link.href);
    ///     true
    /// });
    /// ```
    ///
    /// Not called for a double or triple press (word and block selection), for
    /// the secondary button (the context menu keeps its own link handling) or
    /// for a press on an image, which node-selects it. It is called in a
    /// read-only editor too: following a link is not an edit. The editor never
    /// follows a link by itself.
    ///
    /// In the browser an editor link is a real `<a href>`, and its click's
    /// native navigation is prevented when the editor is **editable** (a click
    /// there is an edit gesture) or has this callback registered. A
    /// **read-only** editor with no callback leaves its links to the browser:
    /// a click follows the link, and middle-click or Ctrl/Cmd+click opens it
    /// in a new tab. Keyboard activation (Enter on a focused editor link) is
    /// offered here too, as a click with no pointer; claiming it prevents the
    /// navigation.
    ///
    /// The callback runs with no internal borrow held, so it may re-enter the
    /// handle (read [`doc`](Self::doc), load another document).
    ///
    /// The callback belongs to the component rendering when it was registered,
    /// as [`on_change`](Self::on_change)'s does: once that component unmounts it
    /// is not called any more (a press is then not claimed, and the editor
    /// counts as having no callback), a live one runs inside it, and one
    /// registered outside any render keeps app lifetime.
    pub fn on_link_click(&self, cb: impl Fn(&LinkClick) -> bool + 'static) {
        self.core_mut().on_link_click = Some(Hook::new(Rc::new(cb)));
    }

    /// Offer a press on a link to the [`on_link_click`](Self::on_link_click)
    /// callback; `true` when the callback claimed it. Answers `false` with no
    /// callback registered. The platform runtime calls this; the callback runs
    /// with no internal borrow held.
    pub fn dispatch_link_click(&self, click: &LinkClick) -> bool {
        let hook = self.core().on_link_click.clone();
        // Untracked, inside its owner, and not at all once that owner is
        // disposed (#931, #1504).
        hook.and_then(|hook| hook.invoke(|cb| cb(click)))
            .unwrap_or(false)
    }

    /// Whether an [`on_link_click`](Self::on_link_click) callback is
    /// registered. The browser runtime asks it to decide whether a click on an
    /// editor link keeps its native navigation: a read-only editor with no
    /// callback leaves links to the browser. A callback whose component has
    /// unmounted does not count.
    pub fn has_link_click_callback(&self) -> bool {
        self.core()
            .on_link_click
            .as_ref()
            .is_some_and(Hook::is_live)
    }

    /// Register a callback for the pointer **entering and leaving links**.
    /// Replaces any previously registered callback.
    ///
    /// It is called with `Some(hover)` when the pointer comes to rest over a
    /// link — from plain text, from outside the editor, or from a different
    /// link — and with `None` when it leaves every link of this editor. It is
    /// called only when that answer changes, never once per pointer move, so
    /// it is the place to show and hide a tooltip:
    ///
    /// ```ignore
    /// editor.on_link_hover(move |hover| match hover {
    ///     Some(h) => tooltip.set(Some((h.link.href.clone(), h.rect))),
    ///     None => tooltip.set(None),
    /// });
    /// ```
    ///
    /// [`LinkHover::rect`] says where the link is painted, in the frame an app
    /// positions a popup in. Hover is not tracked during a drag-select or a
    /// drag-and-drop, and is only as fresh as the last pointer move: an edit
    /// that moves a link out from under a resting pointer is reported on the
    /// next move.
    ///
    /// A pointer move costs the runtime nothing extra while no mounted or
    /// unmounted editor on the thread has a hover callback. The callback runs
    /// with no internal borrow held, so it may re-enter the handle.
    ///
    /// The callback belongs to the component rendering when it was registered,
    /// as [`on_change`](Self::on_change)'s does: once that component unmounts it
    /// is not called any more (a handle can outlive it), a live one runs inside
    /// it, and one registered outside any render keeps app lifetime.
    pub fn on_link_hover(&self, cb: impl Fn(Option<&LinkHover>) + 'static) {
        let mut core = self.core_mut();
        if core.on_link_hover.is_none() {
            registry::link_hover_listener_added();
        }
        core.on_link_hover = Some(Hook::new(Rc::new(cb)));
    }

    /// Invoke the hover callback, if any, with no borrow held: inside its
    /// owner, untracked, and not at all once that owner is disposed.
    pub(crate) fn notify_link_hover(&self, hover: Option<&LinkHover>) {
        let hook = self.core().on_link_hover.clone();
        if let Some(hook) = hook {
            hook.invoke(|cb| cb(hover));
        }
    }

    /// Register a callback for the pointer **entering and leaving images**.
    /// Replaces any previously registered callback.
    ///
    /// It is called with `Some(hover)` when the pointer comes over an image —
    /// from text, from outside the editor, or from another image — and with
    /// `None` when it leaves every image of this editor. Like
    /// [`on_link_hover`](Self::on_link_hover) it is called only when that
    /// answer changes, never once per pointer move: an image is reported
    /// again only when the next move finds it changed (another position, an
    /// attribute edited, or a box a scroll or a relayout moved).
    /// [`ImageHover::rect`] says where the picture is painted, so it is the
    /// place to show controls over it:
    ///
    /// ```ignore
    /// editor.on_image_hover(move |hover| match hover {
    ///     // An absolutely positioned button in the picture's top-right corner.
    ///     Some(h) => eye.set(Some((h.pos, h.attrs.clone(), h.rect))),
    ///     None => eye.set(None),
    /// });
    /// ```
    ///
    /// The controls are the app's own elements, outside the editor's
    /// document, so moving the pointer from the picture onto them leaves the
    /// image and fires `None`: an app keeps them while the pointer is over
    /// them (their own `onmouseenter` / `onmouseleave`).
    ///
    /// Hover is not tracked during a drag-select or a drag-and-drop, and a
    /// pointer move costs the runtime nothing extra while no editor on the
    /// thread has an image hover callback. The callback runs with no internal
    /// borrow held, so it may re-enter the handle (an `update` that sets one
    /// of the image's attributes at `hover.pos`).
    ///
    /// Hover is measured on pointer moves only. A wheel scroll moves the
    /// picture under a resting pointer and reports nothing until the next
    /// move, so an app hides its controls when the content scrolls (an
    /// `onscroll` on the scroller). On desktop the pointer leaving the window
    /// reports nothing either; the browser reports `None` then.
    ///
    /// The callback belongs to the component rendering when it was registered,
    /// as [`on_change`](Self::on_change)'s does: once that component unmounts it
    /// is not called any more (a handle can outlive it), a live one runs inside
    /// it, and one registered outside any render keeps app lifetime.
    pub fn on_image_hover(&self, cb: impl Fn(Option<&ImageHover>) + 'static) {
        let mut core = self.core_mut();
        if core.on_image_hover.is_none() {
            registry::image_hover_listener_added();
        }
        core.on_image_hover = Some(Hook::new(Rc::new(cb)));
    }

    /// Invoke the image hover callback, if any, with no borrow held: inside
    /// its owner, untracked, and not at all once that owner is disposed.
    pub(crate) fn notify_image_hover(&self, hover: Option<&ImageHover>) {
        let hook = self.core().on_image_hover.clone();
        if let Some(hook) = hook {
            hook.invoke(|cb| cb(hover));
        }
    }

    /// The `image` node whose `<img>` is the host element `host_id`, as the
    /// position before it and its attributes; `None` for any other element.
    /// The runtimes ask it on a pointer move to build an [`ImageHover`].
    pub fn image_at_host(&self, host_id: u64) -> Option<(Pos, rinch_editor_core::Attrs)> {
        let core = self.core();
        let (pos, node) = core.view.as_ref()?.node_pos_for_host(host_id)?;
        (node.type_name() == "image").then(|| (Pos(pos), node.attrs().clone()))
    }

    /// A handle that does not keep the editor alive, for tests whose callbacks
    /// re-enter the handle they are registered on.
    #[cfg(test)]
    pub(crate) fn downgrade_for_tests(&self) -> WeakEditorHandle {
        WeakEditorHandle(Rc::downgrade(&self.inner))
    }

    /// Whether `self` and `other` are handles to the same editor.
    pub(crate) fn same_editor(&self, other: &EditorHandle) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }

    /// Register a callback **offered every key press** in this editor before
    /// the editor acts on it. Replaces any previously registered callback.
    ///
    /// It is called for each key press the focused editor receives, **before**
    /// caret movement, table-cell Tab, the clipboard chords, the keymap and text
    /// insertion. Returning `true` consumes the key: the editor does nothing
    /// with it (and in the browser the `keydown` is `preventDefault`ed and
    /// stopped, as for a key the editor handles itself). Returning `false`
    /// leaves the key to the editor exactly as if no callback were registered.
    /// That is what an autocomplete popup needs — Up/Down/Enter/Tab/Escape
    /// drive the popup while it is open and the caret otherwise:
    ///
    /// ```ignore
    /// editor.on_key(move |key| {
    ///     if !picker.is_open() {
    ///         return false;
    ///     }
    ///     match key.key {
    ///         "ArrowDown" => picker.next(),
    ///         "ArrowUp" => picker.previous(),
    ///         "Enter" | "Tab" => picker.accept(),
    ///         "Escape" => picker.close(),
    ///         _ => return false, // typing goes on narrowing the list
    ///     }
    ///     true
    /// });
    /// ```
    ///
    /// **What is offered.** A key press the platform delivers as a key event
    /// while this editor holds the keyboard, auto-repeats included
    /// ([`EditorKey::repeat`]), in a read-only editor too.
    ///
    /// **Who sees it first.** Only a menu shortcut (a chord an app menu or the
    /// DOM menu bar registered) wins over `on_key`. Everything else comes
    /// after it, on both backends: the document-wide keyboard interceptor
    /// (`set_keyboard_interceptor`) and the dismiss stack — `Modal`, `Drawer`
    /// and `Popover`'s `close_on_escape`, the DOM menu bar, a `<select>`
    /// popup. So a popup that claims Escape closes itself and leaves the
    /// `Modal` around the editor open; an Escape it leaves (`false`) goes on to
    /// the dismiss stack and closes the modal as before. (In the browser this
    /// is the capture phase against the bubble phase; desktop offers the key
    /// before either to match.)
    ///
    /// **Not offered**:
    /// a key pressed while an IME composition is in progress (the input method
    /// owns it, and the composed text arrives as a commit, which is typing); in
    /// the browser, a soft keyboard's input, which reaches the page as
    /// `beforeinput` rather than as keys (`key` `"Unidentified"` or `"Process"`
    /// keys are not offered either); a key released (only presses are).
    ///
    /// **With an input method active** (on Linux, IBus or Wayland
    /// `text-input`, which the editor switches on while it has focus), plain
    /// printable characters can arrive as input-method *commits* rather than
    /// as keys, and then `on_key` never sees them. Named keys — arrows, Enter,
    /// Tab, Escape — still arrive as keys. Detect a typed trigger such as `[[`
    /// from [`on_selection_change`](Self::on_selection_change) and the
    /// document around the caret, which sees the result whichever way the text
    /// came in.
    ///
    /// The callback runs with no internal borrow held, so it may re-enter the
    /// handle — read the selection, or replace the typed trigger with a link
    /// through [`update`](Self::update) before consuming Enter.
    ///
    /// Registered while a component renders, the callback belongs to that
    /// component: it runs inside it, and is not called any more once it
    /// unmounts, though the handle lives on (#147/#183). Registered outside any
    /// render it keeps app lifetime.
    pub fn on_key(&self, cb: impl Fn(&EditorKey<'_>) -> bool + 'static) {
        self.core_mut().on_key = Some(Hook::new(Rc::new(cb)));
    }

    /// Offer a key press to the [`on_key`](Self::on_key) callback; `true` when
    /// the callback consumed it. The platform runtime calls this before it
    /// acts on the key. Answers `false` without calling anything when no
    /// callback is registered, or while an IME composition is shown (the key
    /// is the input method's). The callback runs with no borrow held.
    pub fn offer_key(&self, key: &EditorKey<'_>) -> bool {
        let cb = {
            let core = self.core();
            if core
                .view
                .as_ref()
                .is_some_and(RinchDomEditorView::is_composing)
            {
                return false;
            }
            core.on_key.clone()
        };
        cb.and_then(|cb| cb.invoke(|cb| cb(key))).unwrap_or(false)
    }

    /// Register a callback told **whenever the selection changes**. Replaces
    /// any previously registered callback.
    ///
    /// It is called with the new selection after every change that leaves the
    /// selection different from what it was: typing (the caret moves on),
    /// deleting, a click, a drag-select, the arrow keys, a command, an
    /// [`update`](Self::update) or [`set_selection`](Self::set_selection) from
    /// the app, a [`load_doc`](Self::load_doc) that resets it, and a peer's
    /// collaborative edit that maps it
    /// ([`collab_receive`](Self::collab_receive)). It is **not** called when a
    /// change leaves the selection as it was: a formatting command over a
    /// range, a click where the caret already is, a remote edit after the
    /// caret. One change, one call, whichever path made it; an operation that
    /// is several changes in a row (typing over a table-cell selection first
    /// clears the cells) may make one call per change. For an edit that also
    /// changes the document it comes **before** [`on_change`](Self::on_change).
    ///
    /// The callback runs with no internal borrow held, so it may re-enter the
    /// handle: read the document around the caret (the `[[` of a link picker),
    /// open or close a popup, even move the selection again —
    /// which calls it again, from inside itself; a callback that always moves
    /// the selection recurses without end.
    ///
    /// Free when no callback is registered.
    ///
    /// Registered while a component renders, the callback belongs to that
    /// component: it runs inside it, and is not called any more once it
    /// unmounts, though the handle lives on (#147/#183). Registered outside any
    /// render it keeps app lifetime.
    pub fn on_selection_change(&self, cb: impl Fn(&Selection) + 'static) {
        self.core_mut().on_selection_change = Some(Hook::new(Rc::new(cb)));
    }

    /// Register a callback told **when the runtime has placed this editor's
    /// caret (or selection highlight) somewhere new on screen**. Replaces any
    /// previously registered callback.
    ///
    /// This is the moment to position a popup from
    /// [`caret_rect`](Self::caret_rect): it comes from the runtime's caret pass,
    /// which runs once the text is laid out, so the geometry it reads is the
    /// geometry being painted. That matters on desktop, where
    /// [`on_selection_change`](Self::on_selection_change) and
    /// [`on_key`](Self::on_key) run *before* the next layout and `caret_rect` of
    /// a block just edited answers `None`:
    ///
    /// ```ignore
    /// // The picker holds the position its trigger (`[[`) began at.
    /// editor.on_caret_moved({
    ///     let editor = editor.clone();
    ///     move || {
    ///         if let Some(anchor) = picker.anchor()
    ///             && let Some(r) = editor.caret_rect(anchor)
    ///         {
    ///             popup_at.set((r.x, r.y + r.height));
    ///         }
    ///     }
    /// });
    /// ```
    ///
    /// Called for the **focused** editor only (a blurred editor draws no
    /// caret), after any change that moved its overlays: an edit, a selection
    /// move, a reflow (a resize, a peer's edit above the caret). Not called
    /// when a pass leaves them where they were, and not for a scroll, which
    /// moves the editor rather than the caret within it. The desktop runtime
    /// may run a caret pass before an event's layout as well as after it, so
    /// a call can come with the text not yet laid out (`caret_rect` answers
    /// `None` for a block just edited); the call after the layout follows in
    /// the same event. The callback runs with no internal borrow held.
    ///
    /// Free when no callback is registered.
    ///
    /// Registered while a component renders, the callback belongs to that
    /// component: it runs inside it, and is not called any more once it
    /// unmounts, though the handle lives on (#147/#183). Registered outside any
    /// render it keeps app lifetime.
    pub fn on_caret_moved(&self, cb: impl Fn() + 'static) {
        self.core_mut().on_caret_moved = Some(Hook::new(Rc::new(cb)));
    }

    /// Replace this editor's **margin anchors**: the places whose lines an app
    /// draws something beside (a comment's marker, a bookmark flag, a change
    /// bar). [`Self::on_margin_layout`] then reports where each one's line is
    /// drawn, and reports again whenever a line moves — through local edits, a
    /// peer's edits, a reflow or a resize — so the app can keep its markers
    /// level with the text.
    ///
    /// An anchor is a [`MarginAt::Pos`](crate::MarginAt::Pos) — a model
    /// position, carried through every change the editor applies, as a
    /// [`SelectionAnchor`] is — or, while collaborating, a sticky index
    /// ([`MarginAnchor::sticky`]), resolved afresh on each pass so it names the
    /// same character on every replica. Positions are given in the document as
    /// it is now.
    ///
    /// Anchors are view state: setting them changes nothing in the document,
    /// its undo history or what collaborators receive, and works on a
    /// [read-only](Self::set_read_only) editor. They are kept across a
    /// re-mount and before mount; an empty `Vec` clears them (the callback then
    /// hears one empty report).
    pub fn set_margin_anchors(&self, anchors: Vec<MarginAnchor>) {
        let doc_key = {
            let mut core = self.core_mut();
            core.margins.set(anchors);
            // Owed only when someone listens: a set with no callback measures
            // nothing (an emptied set still owes its one empty report).
            (core.on_margin_layout.is_some() && !core.margins.is_idle())
                .then(|| core.view.as_ref().map(RinchDomEditorView::doc_key))
                .flatten()
        };
        if let Some(doc_key) = doc_key {
            registry::owe_overlay_pass(doc_key);
        }
    }

    /// Be told where the [margin anchors](Self::set_margin_anchors)' lines
    /// are drawn: one [`MarginLine`] per anchor that has geometry, in the order
    /// the anchors were given — its key, and the top and height of the line
    /// box it sits in, in the editor container's content coordinates (the
    /// frame the `Editor { … }` overlay slot's children are placed in, which
    /// scrolls with the text; see [`MarginLine`]). Anchors on one line report
    /// the same box.
    ///
    /// Called after a layout or overlay pass that changed any reported line
    /// (or the set of anchors reported, or [`Self::margin_text_right`]) — not
    /// every frame — and once when anchors and a callback are both set on a
    /// mounted editor (also after a re-mount), whatever it finds. A layout
    /// that could not have moved an anchor's line (no anchored block moved or,
    /// on desktop, had its text laid out again) measures nothing. An
    /// anchor is left out while it has no geometry: its block is not laid out
    /// (a virtualized block far out of view), or its position was lost
    /// ([`MarginAt::Pos`](crate::MarginAt::Pos)). A position inside a table
    /// cell reports that cell's line; a position right before a leaf block (a
    /// rule, an embed, a node view) reports that block's box. One pass
    /// measures every anchor once, so `N` anchors cost one pass per layout
    /// that could move a line.
    ///
    /// The callback runs with no internal borrow held, so it may move the
    /// app's markers, read the editor, or set the anchors again (which owes
    /// another pass rather than recursing). Registered while a component
    /// renders, it belongs to that component and is not called once it
    /// unmounts (#147/#183); registered outside any render it keeps app
    /// lifetime. Registering it again replaces it.
    ///
    /// Free when no anchors are set: the pass is one check per editor.
    pub fn on_margin_layout(&self, cb: impl Fn(&[MarginLine]) + 'static) {
        let doc_key = {
            let mut core = self.core_mut();
            core.on_margin_layout = Some(Hook::new(Rc::new(cb)));
            core.margins.forget_report();
            (!core.margins.anchors.is_empty())
                .then(|| core.view.as_ref().map(RinchDomEditorView::doc_key))
                .flatten()
        };
        if let Some(doc_key) = doc_key {
            registry::owe_overlay_pass(doc_key);
        }
    }

    /// The right edge of the text column — the first laid-out top-level
    /// block's border box — in the coordinates [`MarginLine`] uses, as of
    /// the last margin report. A margin marker placed at this `x` and beyond
    /// is clear of the text. `None` before the first report, or with no block
    /// laid out.
    pub fn margin_text_right(&self) -> Option<f32> {
        self.inner
            .try_borrow()
            .ok()?
            .margins
            .last
            .as_ref()
            .and_then(|(_, right)| *right)
    }
}
