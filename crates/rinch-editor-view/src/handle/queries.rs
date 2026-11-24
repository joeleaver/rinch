//! Read-only queries: command availability, marks, the document and
//! selection, caret geometry and position lookups.

use super::*;

impl EditorHandle {
    /// Where a caret at `pos` is on screen: its top-left and height, with zero
    /// width, in the frame an app positions a popup in. On desktop that is
    /// logical window pixels (the frame of
    /// [`NodeHandle::bounds_signal`](rinch_core::dom::NodeHandle::bounds_signal)
    /// and of `position: fixed` / root-level absolutely positioned elements); in
    /// the browser it is viewport client pixels (`getBoundingClientRect`). It
    /// is where the editor paints the caret for `pos` — on a blank line, at the
    /// line's start — whether or not the caret is there now, so an app can
    /// anchor a popup to where a typed trigger began rather than to the moving
    /// caret.
    ///
    /// `None` when the editor is not mounted, `pos` is not inside a textblock
    /// (a node selection's boundary, a position between two blocks), or its
    /// block has not been laid out (a virtualized block never scrolled into
    /// view).
    ///
    /// **As fresh as the last layout.** The browser lays out on demand, so
    /// the web answer is always current. The desktop runtime lays out once per
    /// input event, after the editor has handled it, so asked from inside an
    /// [`on_selection_change`](Self::on_selection_change) or
    /// [`on_key`](Self::on_key) callback, a position in a block the edit just
    /// changed answers `None` (other blocks answer where they were painted).
    /// Position a popup from [`on_caret_moved`](Self::on_caret_moved), which
    /// comes after the layout, on both platforms.
    pub fn caret_rect(&self, pos: Pos) -> Option<rinch_core::reactive::ElementBounds> {
        let core = self.core();
        let affinity = core.affinity_at(pos);
        let (x, y, height) = core.view.as_ref()?.caret_rect(
            &core.state.doc,
            pos,
            affinity,
            core.stored_marks_at(pos),
        )?;
        Some(rinch_core::reactive::ElementBounds {
            x,
            y,
            width: 0.0,
            height,
        })
    }

    /// [`caret_rect`](Self::caret_rect) for a caret at `pos` drawn on
    /// `affinity`'s side of a soft line wrap (#301), whatever the hint says —
    /// what a platform asks when it has to decide which side a caret it is
    /// about to place belongs on.
    pub fn caret_rect_with_affinity(
        &self,
        pos: Pos,
        affinity: CaretAffinity,
    ) -> Option<rinch_core::reactive::ElementBounds> {
        let core = self.core();
        let (x, y, height) = core.view.as_ref()?.caret_rect(
            &core.state.doc,
            pos,
            affinity,
            core.stored_marks_at(pos),
        )?;
        Some(rinch_core::reactive::ElementBounds {
            x,
            y,
            width: 0.0,
            height,
        })
    }

    /// Whether the named command currently applies (toolbar enablement).
    ///
    /// A query: it asks the command's applicability branch (`cmd(state, None)`,
    /// the [`Command`](rinch_editor_core::Command) contract's "would it apply")
    /// and never runs its dispatch branch, so nothing is built, applied or folded
    /// through a plugin, and an app's command whose dispatch branch does something
    /// of its own never sees it fire from a toolbar asking (#834).
    ///
    /// In a [read-only](Self::set_read_only) editor that means "applies **and**
    /// changes only the selection": `can_run("toggleBold")` is `false`, `can_run(
    /// "selectAll")` still `true`, so a toolbar that greys its buttons from here
    /// goes inert with the switch. Whether a command changes only the selection is
    /// what its plugin declares ([`Plugin::selection_commands`]); an undeclared
    /// command answers `false` there. That is the answer for a toolbar, not the
    /// gate: [`command`](Self::command) on a read-only editor is still judged on
    /// what the command would do.
    pub fn can_run(&self, name: &str) -> bool {
        let core = self.core();
        if core.locked() && !core.state.is_selection_command(name) {
            return false;
        }
        core.state.can_run(name)
    }

    /// Look up `binding` in the editor's aggregated keymap and run the bound command.
    /// Returns `Some(applied)` when the key is **consumed**, `None` when it is not:
    /// the caller keys "consumed" on `is_some()` and lets a `None` key go on as if
    /// the editor had no binding for it (on the web: not `preventDefault`ed, so it
    /// reaches the page's `keydown` listeners and the browser's default).
    ///
    /// A matched binding is consumed whether or not its command applied
    /// (`Some(false)`: Tab at the top level sinks nothing and is still the
    /// editor's; Ctrl+Shift+R on a right-aligned paragraph never reaches the
    /// browser's reload), unless the binding is declared fall-through
    /// ([`Plugin::fall_through_keys`]) and its command answered `false`: that
    /// answers `None`, as ProseMirror's keymap does — Mod-Enter (`exitCode`) in a
    /// paragraph. Unbound keys answer `None`. This is the single keymap entry
    /// point: both the desktop and web key dispatchers translate their native
    /// event into a platform-agnostic [`KeyBinding`] and route through here, so a
    /// binding added in editor-core works on every platform (design §5).
    pub fn dispatch_key(&self, binding: KeyBinding) -> Option<bool> {
        let (name, falls_through) = {
            let core = self.core();
            let keymap = core.state.keymap();
            let name = keymap.command_for(&binding)?.to_string();
            (name, keymap.falls_through_when_inapplicable(&binding))
        };
        let applied = self.command(&name);
        (applied || !falls_through).then_some(applied)
    }

    /// Whether the mark named `mark` is active for the current selection (toolbar
    /// "on" state) — reads **state**, never the host.
    pub fn is_mark_active(&self, mark: &str) -> bool {
        let core = self.core();
        match core.state.schema().mark_type(mark) {
            Some(mt) => is_mark_active(&core.state, mt),
            None => false,
        }
    }

    /// The `href` of the link the selection is on, or `None` — for pre-filling an
    /// "edit link" dialog (`is_mark_active("link")` only reports presence, not the
    /// target). Reads **state**, never the host.
    ///
    /// For a **range**, the first link in it (as `is_mark_active` looks at the range
    /// too). For a **caret**, the link a character typed there would carry: `link`
    /// is non-inclusive ([`MarkSpec::inclusive`](rinch_editor_core::MarkSpec::inclusive)),
    /// so a caret inside a link reports it and a caret at either of its edges does
    /// not — right after the last letter is outside the link, as typing there is.
    /// The one exception is the seam of two different links, where the caret is in
    /// the first (`ResolvedPos::marks`' departure from ProseMirror).
    pub fn active_link_href(&self) -> Option<String> {
        let core = self.core();
        let state = &core.state;
        let mt = state.schema().mark_type("link")?;
        let href = |m: &Mark| {
            (&m.typ == mt)
                .then(|| m.attrs.get_str("href").map(str::to_string))
                .flatten()
        };
        let sel = &state.selection;
        if sel.is_empty() {
            return marks_at(state, sel.head().0).iter().find_map(href);
        }
        let mut found = None;
        state
            .doc
            .nodes_between(sel.from().0, sel.to().0, &mut |node, _pos, _parent| {
                if found.is_none() && node.is_inline() {
                    found = node.marks().iter().find_map(href);
                }
                found.is_none()
            });
        found
    }

    /// The schema type name of the block the cursor is in (e.g. `"heading"`), or
    /// `None` across a multi-block selection.
    pub fn current_block_type(&self) -> Option<String> {
        current_block_type(&self.core().state).map(|nt| nt.name().to_string())
    }

    /// Whether the selection is inside a node of the given type (e.g. `"blockquote"`,
    /// `"bullet_list"`) — drives the List/Blockquote toolbar active states (A6).
    pub fn in_node_type(&self, type_name: &str) -> bool {
        in_node_type(&self.core().state, type_name)
    }

    /// The current document (the save shape; serialize with `to_doc()` under the
    /// `serde` feature).
    pub fn doc(&self) -> Node {
        self.core().state.doc.clone()
    }

    /// A snapshot of the whole editor state.
    pub fn state(&self) -> EditorState {
        self.core().state.clone()
    }

    /// The current selection.
    pub fn selection(&self) -> Selection {
        self.core().state.selection.clone()
    }

    /// The host id of the editor container element, or `0` if not yet mounted.
    pub fn container_id(&self) -> u64 {
        self.core().view.as_ref().map_or(0, |v| v.container_id())
    }

    /// The host caret address `(textblock element id, flat UTF-8 byte offset)` for a
    /// model `pos` (used by app-side geometry: caret point, vertical movement).
    pub fn caret_address(&self, pos: Pos) -> Option<(u64, usize)> {
        let core = self.core();
        core.view.as_ref()?.caret_address(&core.state.doc, pos)
    }

    /// Map a host caret address `(textblock element id, flat UTF-8 byte offset)` —
    /// e.g. from a pointer hit-test — to a model [`Pos`] (without moving the cursor).
    pub fn pos_at(&self, textblock_dom_id: u64, ifc_byte: usize) -> Option<Pos> {
        self.core()
            .view
            .as_ref()?
            .pos_at(textblock_dom_id, ifc_byte)
    }

    /// The model position before the inline leaf (an image) whose host element
    /// is `leaf_dom_id`, or after it when `after` — what a pointer hit on one
    /// half of an image resolves to. A flat byte offset cannot say this on
    /// desktop, where an image is an inline box with no bytes, so the byte on
    /// its two sides is one number (#1104). `None` if that element is not an
    /// inline leaf of this editor.
    pub fn pos_beside_inline_leaf(&self, leaf_dom_id: u64, after: bool) -> Option<Pos> {
        self.core()
            .view
            .as_ref()?
            .pos_beside_inline_leaf(leaf_dom_id, after)
    }

    /// The inline leaf a caret at `pos` is drawn against rather than placed by
    /// its flat byte offset, as `(textblock element id, leaf element id,
    /// after)`: for a `Downstream` caret the image just after `pos`, else the
    /// one just before it (#1104, #1116). The host draws such a caret from the leaf's
    /// box (`DomDocument::query_inline_box_caret`). `None` anywhere else.
    pub fn caret_inline_box(&self, pos: Pos, affinity: CaretAffinity) -> Option<(u64, u64, bool)> {
        let core = self.core();
        let view = core.view.as_ref()?;
        let (tb, _) = view.caret_address(&core.state.doc, pos)?;
        let (leaf, after) = view.inline_box_beside(&core.state.doc, pos, affinity)?;
        Some((tb, leaf, after))
    }

    /// Map `(textblock element id, model offset within its content)` to a model
    /// [`Pos`] — a text character and an inline leaf (image, hard break) each
    /// count one, and an offset past the block's end is clamped to it. For a
    /// host that can count leaves where it found the point, as the web does
    /// from the DOM: in the browser's flat byte offsets ([`Self::pos_at`]) a leaf
    /// has no width, so they cannot tell the position before a hard break from
    /// the one after it (#1025).
    pub fn pos_in_textblock(&self, textblock_dom_id: u64, offset: usize) -> Option<Pos> {
        self.core()
            .view
            .as_ref()?
            .pos_in_textblock(textblock_dom_id, offset)
    }

    /// The inverse of [`Self::pos_in_textblock`]: `(textblock element id, offset
    /// into its content)` for `pos`, a text character and an inline leaf each
    /// counting one. Unlike [`Self::caret_address`]'s flat byte offset it tells
    /// the two sides of a leaf apart. `None` when `pos` is not in a textblock.
    pub fn textblock_offset(&self, pos: Pos) -> Option<(u64, usize)> {
        let core = self.core();
        core.view.as_ref()?.textblock_offset(&core.state.doc, pos)
    }

    /// Model-based vertical fallback for the geometry-driven Up/Down caret step: the
    /// caret position in the **adjacent textblock** — just before (`down = false`) or
    /// after (`down = true`) the current textblock. Used when geometry can't resolve
    /// the next visual line: the target line is an *empty* block (no text to hit-test)
    /// or a block atom is in the way, so the screen-space probe snaps back to the
    /// current line. Stepping in the model lets the caret still land on a blank line /
    /// past an atom. `None` at the document edge. Mirrors the desktop `vertical_step`
    /// stuck-path so both platforms behave the same.
    pub fn vertical_block_fallback(&self, down: bool) -> Option<Selection> {
        let core = self.core();
        let doc = &core.state.doc;
        let head = core.state.selection.head();
        let r = doc.resolve(head).ok()?;
        let probe = if r.parent().is_textblock() {
            let content_start = head.0 - r.parent_offset();
            if down {
                content_start + r.parent().content().size() + 1
            } else {
                content_start.checked_sub(1)?
            }
        } else {
            head.0
        };
        Selection::near_text(
            doc,
            Pos(probe.min(doc.content_size())),
            if down { 1 } else { -1 },
        )
    }

    /// Where an Up/Down step from the caret into a table lands when the
    /// platform's geometry fell back to `target` (the next textblock in
    /// document order): the cell of the table's edge row under `goal_x`
    /// instead of its last (first) cell
    /// ([`vertical_entry`](rinch_editor_core::commands::table_ops::vertical_entry)).
    /// `x_of` gives a position's caret x in the platform's coordinates.
    /// `None` leaves `target` as it is.
    pub fn vertical_table_entry(
        &self,
        target: Pos,
        down: bool,
        goal_x: f32,
        x_of: impl Fn(Pos) -> Option<f32>,
    ) -> Option<Pos> {
        let (doc, head) = {
            let core = self.core();
            (core.state.doc.clone(), core.state.selection.head())
        };
        rinch_editor_core::commands::table_ops::vertical_entry(
            &doc, head, target, down, goal_x, x_of,
        )
    }

    /// A [`Selection::Node`] for the leaf node (image / horizontal rule) whose host
    /// element is `host_id` — the pointer hit-test path for node-selecting a leaf the
    /// user clicks. `None` if `host_id` isn't a placed node, or its node isn't
    /// selectable (design §6 node-views).
    ///
    /// [`Selection::Node`]: rinch_editor_core::Selection::Node
    pub fn node_selection_at_host(&self, host_id: u64) -> Option<Selection> {
        let core = self.core();
        let (pos, node) = core.view.as_ref()?.node_pos_for_host(host_id)?;
        // Node-views are *leaf* atoms (image / horizontal rule). A block container
        // (paragraph, list, blockquote) is `selectable` in the schema but is never
        // node-selected by a click, so restrict to leaves here.
        if !node.node_type().is_leaf() {
            return None;
        }
        Selection::node_at(&core.state.doc, Pos(pos))
    }

    /// Place the cursor at a host caret address `(textblock element id, flat UTF-8
    /// byte offset)` produced by a pointer hit-test (the click→`Pos` path). Returns
    /// whether the address resolved to a model position.
    pub fn set_cursor_from_ifc(&self, textblock_dom_id: u64, ifc_byte: usize) -> bool {
        match self.pos_at(textblock_dom_id, ifc_byte) {
            Some(pos) => {
                self.set_selection(Selection::cursor(pos));
                true
            }
            None => false,
        }
    }
}
