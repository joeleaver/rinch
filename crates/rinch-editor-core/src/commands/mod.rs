//! The built-in command catalogue (design §5 / amendment A6) and the toolbar
//! queries that drive button enabled/active state — all reading **state**, never
//! the DOM (the structural fix for the old `with_active_ce_api(...).has_active_mark`
//! anti-pattern).
//!
//! Commands compose the [`Transaction`] gesture API; structural commands use the
//! "attempt the steps, let the schema gate decide" pattern, so applicability and
//! execution share one code path and an invalid edit is reported as inapplicable
//! rather than producing a broken document.

use std::rc::Rc;

use crate::Fragment;
use crate::command::{Command, chain};
use crate::keymap::KeyBinding;
use crate::model::{Attrs, Mark, MarkType, Node, NodeType};
use crate::plugin::{Plugin, PluginKey};
use crate::pos::{Pos, ResolvedPos};
use crate::state::{EditorState, Transaction};
use crate::transform::steps::{ReplaceAroundStep, ReplaceStep};
use crate::transform::{block_range, block_range_simple};

// ===================================================================
// Toolbar queries — read state, never the host (A6)
// ===================================================================

/// The marks active at a position (what a character typed there would inherit).
pub fn marks_at(state: &EditorState, pos: usize) -> Vec<Mark> {
    state
        .doc
        .resolve(Pos(pos))
        .map(|r| r.marks())
        .unwrap_or_default()
}

/// True if any inline node in `from..to` carries a mark of `mark_type` (PM's
/// `rangeHasMark`).
pub fn range_has_mark(doc: &Node, from: usize, to: usize, mark_type: &MarkType) -> bool {
    if to <= from {
        return false;
    }
    let mut found = false;
    doc.nodes_between(from, to, &mut |node, _pos, _parent| {
        if !found && node.is_inline() && node.marks().iter().any(|m| &m.typ == mark_type) {
            found = true;
        }
        !found
    });
    found
}

/// Whether `mark_type` is active for the current selection — for a collapsed cursor,
/// whether it is in the stored/inherited marks; for a range, whether the range has
/// the mark. Drives a toolbar button's "on" state.
pub fn is_mark_active(state: &EditorState, mark_type: &MarkType) -> bool {
    let sel = &state.selection;
    if sel.is_empty() {
        let marks = state
            .stored_marks
            .clone()
            .unwrap_or_else(|| marks_at(state, sel.from().0));
        marks.iter().any(|m| &m.typ == mark_type)
    } else {
        range_has_mark(&state.doc, sel.from().0, sel.to().0, mark_type)
    }
}

/// The node type of the textblock at the selection head — drives the block-style
/// dropdown's current value.
pub fn current_block_type(state: &EditorState) -> Option<NodeType> {
    let r = state.doc.resolve(state.selection.head()).ok()?;
    Some(r.parent().node_type().clone())
}

/// Whether any ancestor of the selection head is of `type_name` — drives the
/// List/Blockquote button active-state (A6: `current_block_type` alone can't,
/// because the textblock stays `paragraph` inside a `list_item`).
pub fn in_node_type(state: &EditorState, type_name: &str) -> bool {
    if let Ok(r) = state.doc.resolve(state.selection.head()) {
        for d in 0..=r.depth() {
            if r.node(d).type_name() == type_name {
                return true;
            }
        }
    }
    false
}

/// Whether the outdent (lift) command currently applies.
pub fn can_lift(state: &EditorState) -> bool {
    build_lift(state).is_some()
}

/// Whether the indent (sink list item) command currently applies.
pub fn can_sink(state: &EditorState) -> bool {
    build_sink(state, "list_item").is_some()
}

// ===================================================================
// Command plumbing
// ===================================================================

/// Wrap a transaction-builder into a [`Command`]. The builder returns `Some(tr)`
/// when the command applies (even a stored-marks-only transaction with no doc
/// change), or `None` when it does not.
fn command_tr(build: impl Fn(&EditorState) -> Option<Transaction> + 'static) -> Command {
    Rc::new(move |state, dispatch| match build(state) {
        Some(tr) => {
            if let Some(d) = dispatch {
                d(tr);
            }
            true
        }
        None => false,
    })
}

// ===================================================================
// Marks
// ===================================================================

/// Toggle a simple (attribute-less) mark: bold, italic, underline, strike, code,
/// subscript, superscript, highlight. For a collapsed cursor this flips the stored
/// marks (the "click bold then type" path); for a range it adds or removes the
/// mark across it (remove if the range already has it anywhere — PM's toggle).
pub fn toggle_mark(mark_type: &'static str) -> Command {
    command_tr(move |state| {
        let mt = state.schema().mark_type(mark_type)?.clone();
        let sel = &state.selection;
        let mut tr = state.tr();
        if sel.is_empty() {
            let marks = state
                .stored_marks
                .clone()
                .unwrap_or_else(|| marks_at(state, sel.from().0));
            let mark = Mark::simple(mt.clone());
            if marks.iter().any(|m| m.typ == mt) {
                tr.remove_stored_mark(&mark);
            } else {
                tr.add_stored_mark(mark);
            }
        } else {
            let (from, to) = (sel.from().0, sel.to().0);
            let mark = Mark::simple(mt.clone());
            if range_has_mark(&state.doc, from, to, &mt) {
                tr.remove_mark(from, to, mark).ok()?;
            } else {
                tr.add_mark(from, to, mark).ok()?;
            }
        }
        Some(tr)
    })
}

/// Apply a `link` mark with `href` across the selection (no-op for a collapsed
/// cursor — a link needs a range). A builder (the href is supplied at call time).
pub fn toggle_link(href: String) -> Command {
    command_tr(move |state| {
        let mt = state.schema().mark_type("link")?.clone();
        let sel = &state.selection;
        if sel.is_empty() {
            return None;
        }
        let (from, to) = (sel.from().0, sel.to().0);
        let mut tr = state.tr();
        if range_has_mark(&state.doc, from, to, &mt) {
            // Remove every link in the range, regardless of href.
            remove_marks_of_type(&mut tr, &state.doc, from, to, &mt).ok()?;
        } else {
            let attrs = Attrs::from_iter([("href", crate::AttrValue::from(href.clone()))]);
            tr.add_mark(from, to, Mark::new(mt.clone(), attrs)).ok()?;
        }
        Some(tr)
    })
}

/// Remove all `link` marks from the selection.
pub fn remove_link() -> Command {
    command_tr(|state| {
        let mt = state.schema().mark_type("link")?.clone();
        let sel = &state.selection;
        let (from, to) = (sel.from().0, sel.to().0);
        if to <= from || !range_has_mark(&state.doc, from, to, &mt) {
            return None;
        }
        let mut tr = state.tr();
        remove_marks_of_type(&mut tr, &state.doc, from, to, &mt).ok()?;
        Some(tr)
    })
}

/// Remove every mark of `mark_type` (any attrs) from `from..to`.
fn remove_marks_of_type(
    tr: &mut Transaction,
    doc: &Node,
    from: usize,
    to: usize,
    mark_type: &MarkType,
) -> Result<(), crate::transform::StepError> {
    let mut targets: Vec<Mark> = Vec::new();
    doc.nodes_between(from, to, &mut |node, _pos, _parent| {
        for m in node.marks() {
            if &m.typ == mark_type && !targets.contains(m) {
                targets.push(m.clone());
            }
        }
        true
    });
    for m in targets {
        tr.remove_mark(from, to, m)?;
    }
    Ok(())
}

// ===================================================================
// Block types
// ===================================================================

/// Set every textblock overlapping the selection to `node_type` with `attrs` —
/// `setParagraph` / `setHeading{level}` / `setCodeBlock`. Applies only if some
/// textblock in range does not already have that markup.
pub fn set_block_type(node_type: &'static str, attrs: Attrs) -> Command {
    command_tr(move |state| {
        let ty = state.schema().node_type(node_type)?.clone();
        if !ty.is_textblock() {
            return None;
        }
        let (from, to) = (state.selection.from().0, state.selection.to().0);
        let mut applies = false;
        state
            .doc
            .nodes_between(from, to, &mut |node, _pos, _parent| {
                if node.is_textblock() && !node.has_markup(&ty, &attrs) {
                    applies = true;
                }
                true
            });
        if !applies {
            return None;
        }
        let mut tr = state.tr();
        tr.set_block_type(from, to, ty, attrs.clone()).ok()?;
        tr.doc_changed().then_some(tr)
    })
}

// ===================================================================
// Wrapping: blockquote, lists, lift/sink
// ===================================================================

/// Wrap the selected block range in `node_type` (e.g. `wrapInBlockquote`).
pub fn wrap_in(node_type: &'static str) -> Command {
    command_tr(move |state| {
        let ty = state.schema().node_type(node_type)?.clone();
        let r_from = state.doc.resolve(state.selection.from()).ok()?;
        let r_to = state.doc.resolve(state.selection.to()).ok()?;
        let range = block_range_simple(&r_from, &r_to)?;
        let mut tr = state.tr();
        tr.wrap(&range, &[(ty, Attrs::new())]).ok()?;
        tr.doc_changed().then_some(tr)
    })
}

/// Toggle a list of `list_type` (`toggleBulletList` / `toggleOrderedList`): wrap
/// the block range into `list > list_item` if not already in that list, or lift it
/// out if it is. **Wrap/lift, NOT set_block_type** (A6).
pub fn toggle_list(list_type: &'static str) -> Command {
    command_tr(move |state| {
        if in_node_type(state, list_type) {
            build_lift(state)
        } else {
            build_wrap_in_list(state, list_type)
        }
    })
}

/// Build the wrap-into-list transaction.
fn build_wrap_in_list(state: &EditorState, list_type: &str) -> Option<Transaction> {
    let list_ty = state.schema().node_type(list_type)?.clone();
    let item_name = if list_type == "task_list" {
        "task_item"
    } else {
        "list_item"
    };
    let item_ty = state.schema().node_type(item_name)?.clone();
    let r_from = state.doc.resolve(state.selection.from()).ok()?;
    let r_to = state.doc.resolve(state.selection.to()).ok()?;
    let range = block_range_simple(&r_from, &r_to)?;
    let mut tr = state.tr();
    tr.wrap(&range, &[(list_ty, Attrs::new()), (item_ty, Attrs::new())])
        .ok()?;
    tr.doc_changed().then_some(tr)
}

/// The outdent / lift command: lift the selected block range out of its enclosing
/// wrapper (list, blockquote). Tries the deepest valid target first.
pub fn lift() -> Command {
    command_tr(build_lift)
}

/// Build the lift transaction, choosing the shallowest schema-valid target depth.
fn build_lift(state: &EditorState) -> Option<Transaction> {
    let r_from = state.doc.resolve(state.selection.from()).ok()?;
    let r_to = state.doc.resolve(state.selection.to()).ok()?;
    let range = block_range_simple(&r_from, &r_to)?;
    if range.depth == 0 {
        return None;
    }
    // Try lifting to each target from one level out (depth-1) down to the doc;
    // the schema gate rejects invalid targets, so the first that applies wins.
    for target in (0..range.depth).rev() {
        let mut tr = state.tr();
        if tr.lift(&range, target).is_ok() && tr.doc_changed() {
            return Some(tr);
        }
    }
    None
}

/// The indent / sink-list-item command: nest the current list item under its
/// previous sibling. Port of PM's `sinkListItem` (without the `nestedBefore`
/// merge optimization).
pub fn sink_list_item(item_type: &'static str) -> Command {
    command_tr(move |state| build_sink(state, item_type))
}

/// Build the sink (indent) transaction.
fn build_sink(state: &EditorState, item_type: &str) -> Option<Transaction> {
    let item_ty = state.schema().node_type(item_type)?.clone();
    let r_from = state.doc.resolve(state.selection.from()).ok()?;
    let r_to = state.doc.resolve(state.selection.to()).ok()?;
    let range = block_range(&r_from, &r_to, |n| {
        n.child_count() > 0 && n.child(0).node_type() == &item_ty
    })?;
    let start_index = range.start_index();
    if start_index == 0 {
        return None;
    }
    let parent = range.parent();
    let node_before = parent.child(start_index - 1);
    if node_before.node_type() != &item_ty {
        return None;
    }
    // Build a `list_item(list())` shell; the ReplaceAroundStep drops the current
    // item into the inner list, joining onto the previous item.
    let list_ty = parent.node_type().clone();
    let inner_list = Node::new_branch(list_ty, parent.attrs().clone(), Fragment::empty());
    let new_item = Node::new_branch(
        item_ty.clone(),
        Attrs::new(),
        Fragment::from_node(inner_list),
    );
    let slice = crate::Slice::new(Fragment::from_node(new_item), 1, 0);
    let before = range.start();
    let after = range.end();
    let mut tr = state.tr();
    tr.step(Box::new(ReplaceAroundStep::new(
        before - 1,
        after,
        before,
        after,
        slice,
        1,
        true,
    )))
    .ok()?;
    tr.doc_changed().then_some(tr)
}

// ===================================================================
// Structure & deletion
// ===================================================================

/// Split the current textblock at the cursor (Enter). Deletes a non-empty
/// selection first; at the end of a heading, the new block becomes a paragraph.
pub fn split_block() -> Command {
    command_tr(|state| {
        let mut tr = state.tr();
        if !state.selection.is_empty() {
            tr.delete_selection().ok()?;
        }
        let pos = tr.selection().from().0;
        let r = tr.doc().resolve(Pos(pos)).ok()?;
        if r.depth() < 1 {
            return None;
        }
        let parent = r.parent();
        let at_end = r.parent_offset() == parent.content().size();
        let types_after = if at_end && parent.type_name() == "heading" {
            let para = state.schema().node_type("paragraph")?.clone();
            Some(vec![Some((para, Attrs::new()))])
        } else {
            None
        };
        tr.split(pos, 1, types_after).ok()?;
        Some(tr)
    })
}

/// Split the current list item into a new one (Enter inside a list). Port of
/// ProseMirror's `splitListItem`: when the cursor is in a textblock whose parent
/// is `item_type` (e.g. `list_item`), split **two levels** so the text after the
/// cursor moves into a fresh sibling item. Returns `None` (inapplicable) when not
/// in a list item or when the item's textblock is empty — the [`split_block_or_list_item`]
/// chain then falls through to [`lift_empty_block`] (which exits the list).
pub fn split_list_item(item_type: &'static str) -> Command {
    command_tr(move |state| {
        let r_from = state.doc.resolve(state.selection.from()).ok()?;
        let r_to = state.doc.resolve(state.selection.to()).ok()?;
        // Need at least `… > item > textblock` so node(depth-1) is the item, and a
        // selection contained within one textblock.
        if r_from.depth() < 2
            || r_from.depth() != r_to.depth()
            || r_from.start(r_from.depth()) != r_to.start(r_to.depth())
        {
            return None;
        }
        if r_from.node(r_from.depth() - 1).type_name() != item_type {
            return None;
        }
        // Empty textblock → not a split; let the chain lift it out of the list.
        if r_from.parent().content().size() == 0 {
            return None;
        }
        let mut tr = state.tr();
        if !state.selection.is_empty() {
            tr.delete_selection().ok()?;
        }
        let pos = tr.selection().from().0;
        let r = tr.doc().resolve(Pos(pos)).ok()?;
        // At the end of the textblock the new item starts with the list item's
        // default content (a paragraph) rather than copying a heading/code block.
        let at_end = r.parent_offset() == r.parent().content().size();
        let types = if at_end {
            let para = state.schema().node_type("paragraph")?.clone();
            // [outer = item (keep its type), inner = the new default textblock]
            Some(vec![None, Some((para, Attrs::new()))])
        } else {
            None
        };
        tr.split(pos, 2, types).ok()?;
        Some(tr)
    })
}

/// Lift an empty textblock out of its wrapper (PM `liftEmptyBlock`): pressing Enter
/// in an empty list item / blockquote paragraph exits it. If the empty block is not
/// the last child of its parent, split the parent before it instead.
pub fn lift_empty_block() -> Command {
    command_tr(|state| {
        if !state.selection.is_empty() {
            return None;
        }
        let r = state.doc.resolve(state.selection.from()).ok()?;
        if r.parent().content().size() != 0 {
            return None;
        }
        // Not the last child of its grandparent → split the grandparent before it.
        if r.depth() >= 2
            && r.after(r.depth()) != Some(r.end(r.depth() - 1))
            && let Some(before) = r.before(r.depth())
        {
            let mut tr = state.tr();
            if tr.split(before, 1, None).is_ok() && tr.doc_changed() {
                return Some(tr);
            }
        }
        // Otherwise lift the empty block out of its wrapper (exits a list, etc.).
        build_lift(state)
    })
}

/// The Enter handler (`splitBlock` is the toolbar/programmatic name; Enter binds
/// here): split a list item into a new item, else lift an empty block out of its
/// wrapper, else split the current block.
pub fn split_block_or_list_item() -> Command {
    chain(vec![
        split_list_item("list_item"),
        lift_empty_block(),
        split_block(),
    ])
}

/// Insert a hard break at the cursor (Shift+Enter).
pub fn insert_hard_break() -> Command {
    command_tr(|state| {
        let hb = state
            .schema()
            .create_node("hard_break", Attrs::new(), Fragment::empty())
            .ok()?;
        let mut tr = state.tr();
        tr.replace_selection_with(hb).ok()?;
        Some(tr)
    })
}

/// Insert a horizontal rule, replacing the selection.
pub fn insert_horizontal_rule() -> Command {
    command_tr(|state| {
        let hr = state
            .schema()
            .create_node("horizontal_rule", Attrs::new(), Fragment::empty())
            .ok()?;
        let mut tr = state.tr();
        tr.replace_selection_with(hr).ok()?;
        Some(tr)
    })
}

/// Insert an image with `src`/`alt`, replacing the selection. A builder.
pub fn insert_image(src: String, alt: String) -> Command {
    command_tr(move |state| {
        let attrs = Attrs::from_iter([
            ("src", crate::AttrValue::from(src.clone())),
            ("alt", crate::AttrValue::from(alt.clone())),
        ]);
        let img = state
            .schema()
            .create_node("image", attrs, Fragment::empty())
            .ok()?;
        let mut tr = state.tr();
        tr.replace_selection_with(img).ok()?;
        Some(tr)
    })
}

/// Delete the current selection (only applies when it is non-empty).
pub fn delete_selection() -> Command {
    command_tr(|state| {
        if state.selection.is_empty() {
            return None;
        }
        let mut tr = state.tr();
        tr.delete_selection().ok()?;
        Some(tr)
    })
}

/// PM `findCutBefore`: the document position just before the nearest ancestor of
/// the cursor (walking up from it) that has a sibling before it — the boundary a
/// Backspace at block-start would join across. `None` when the cursor is at the
/// very start of its top-level context (so Backspace should *lift*, not join).
/// Stops at (and never crosses) an isolating boundary (e.g. a table cell, M7).
fn find_cut_before(r: &ResolvedPos) -> Option<usize> {
    if r.parent().node_type().spec().isolating {
        return None;
    }
    let mut depth = r.depth() as isize - 1;
    while depth >= 0 {
        let d = depth as usize;
        if r.index(d) > 0 {
            return r.before(d + 1);
        }
        if r.node(d).node_type().spec().isolating {
            break;
        }
        depth -= 1;
    }
    None
}

/// PM `findCutAfter`: the mirror of [`find_cut_before`] — the position just after
/// the nearest ancestor that has a sibling after it, the boundary a forward-Delete
/// at block-end joins across.
fn find_cut_after(r: &ResolvedPos) -> Option<usize> {
    if r.parent().node_type().spec().isolating {
        return None;
    }
    let mut depth = r.depth() as isize - 1;
    while depth >= 0 {
        let d = depth as usize;
        let parent = r.node(d);
        if r.index(d) + 1 < parent.child_count() {
            return r.after(d + 1);
        }
        if parent.node_type().spec().isolating {
            break;
        }
        depth -= 1;
    }
    None
}

/// Lift the block that begins just after `cut` out of its wrapper (PM
/// `deleteBarrier`'s lift path, used when a forward-Delete can't directly join
/// across the barrier). Returns `None` if there is no liftable block there.
fn build_lift_after(state: &EditorState, cut: usize) -> Option<Transaction> {
    let sel_after = crate::selection::Selection::near(&state.doc, Pos(cut), 1);
    let r_from = state.doc.resolve(sel_after.from()).ok()?;
    let r_to = state.doc.resolve(sel_after.to()).ok()?;
    let range = block_range_simple(&r_from, &r_to)?;
    if range.depth == 0 {
        return None;
    }
    for target in (0..range.depth).rev() {
        let mut tr = state.tr();
        if tr.lift(&range, target).is_ok() && tr.doc_changed() {
            return Some(tr);
        }
    }
    None
}

/// Backspace: delete the selection, else the char before the cursor, else — at the
/// start of a textblock — join onto the previous block or lift this block out of
/// its wrapper.
///
/// At block start this is PM's `joinBackward`/`deleteBarrier` (the common cases):
/// it joins across the nearest cut with a **structural** replace of the boundary
/// token pair (its own undo group, never coalesced into typing), and when that
/// crosses a wrapper boundary it cannot join directly — into a blockquote or list
/// item — it instead **lifts** this block out of the wrapper, so a second
/// Backspace then joins.
///
/// Not yet ported (the remaining `deleteBarrier` branches; they need
/// `ContentMatch::find_wrapping`, and belong with the M5.6 keyboard integration):
/// the **wrap-and-move** branch — Backspace at the start of a paragraph that
/// *follows* a list moves the paragraph into the list as a new item (currently a
/// no-op here) — and the **content-move** + select-node-backward branches.
pub fn delete_char_backward() -> Command {
    command_tr(|state| {
        let sel = &state.selection;
        if !sel.is_empty() {
            let mut tr = state.tr();
            tr.delete_selection().ok()?;
            return Some(tr);
        }
        let pos = sel.from().0;
        if pos == 0 {
            return None;
        }
        let r = state.doc.resolve(Pos(pos)).ok()?;
        if r.parent_offset() > 0 {
            // Mid-text: delete the char before the cursor.
            let mut tr = state.tr();
            tr.delete(pos - 1, pos).ok()?;
            return tr.doc_changed().then_some(tr);
        }
        // At block start: join across the nearest cut, else lift out of the wrapper.
        if let Some(cut) = find_cut_before(&r) {
            let mut tr = state.tr();
            if tr
                .step(Box::new(ReplaceStep::structural(
                    cut - 1,
                    cut + 1,
                    crate::Slice::empty(),
                )))
                .is_ok()
                && tr.doc_changed()
            {
                return Some(tr);
            }
        }
        build_lift(state)
    })
}

/// Delete forward: delete the selection, else the char after the cursor, else — at
/// the end of a textblock — join the next block onto this one, or lift the next
/// block out of its wrapper across the barrier (PM `joinForward`/`deleteBarrier`;
/// see [`delete_char_backward`]). Unlike Backspace there is no "no cut → lift"
/// case: with nothing after, forward-Delete is a no-op.
pub fn delete_char_forward() -> Command {
    command_tr(|state| {
        let sel = &state.selection;
        if !sel.is_empty() {
            let mut tr = state.tr();
            tr.delete_selection().ok()?;
            return Some(tr);
        }
        let pos = sel.from().0;
        let r = state.doc.resolve(Pos(pos)).ok()?;
        if r.parent_offset() < r.parent().content().size() {
            // Mid-text: delete the char after the cursor.
            let mut tr = state.tr();
            tr.delete(pos, pos + 1).ok()?;
            return tr.doc_changed().then_some(tr);
        }
        // At block end: join across the nearest cut, else lift the next block out.
        let cut = find_cut_after(&r)?;
        let mut tr = state.tr();
        if tr
            .step(Box::new(ReplaceStep::structural(
                cut - 1,
                cut + 1,
                crate::Slice::empty(),
            )))
            .is_ok()
            && tr.doc_changed()
        {
            return Some(tr);
        }
        build_lift_after(state, cut)
    })
}

// ===================================================================
// The base-editing plugin (commands + keymap)
// ===================================================================

/// Bundles the built-in command catalogue and its default key bindings as a
/// [`Plugin`]. History (undo/redo) and markdown input rules are separate plugins.
#[derive(Debug, Default)]
pub struct BaseCommandsPlugin;

impl Plugin for BaseCommandsPlugin {
    fn key(&self) -> PluginKey {
        PluginKey("base-commands")
    }

    fn commands(&self) -> Vec<(&'static str, Command)> {
        let mut cmds: Vec<(&'static str, Command)> = vec![
            ("toggleBold", toggle_mark("bold")),
            ("toggleItalic", toggle_mark("italic")),
            ("toggleUnderline", toggle_mark("underline")),
            ("toggleStrike", toggle_mark("strike")),
            ("toggleCode", toggle_mark("code")),
            ("toggleSubscript", toggle_mark("subscript")),
            ("toggleSuperscript", toggle_mark("superscript")),
            ("toggleHighlight", toggle_mark("highlight")),
            ("removeLink", remove_link()),
            ("setParagraph", set_block_type("paragraph", Attrs::new())),
            ("setCodeBlock", set_block_type("code_block", Attrs::new())),
            ("toggleBulletList", toggle_list("bullet_list")),
            ("toggleOrderedList", toggle_list("ordered_list")),
            ("wrapInBlockquote", wrap_in("blockquote")),
            ("liftListItem", lift()),
            ("outdent", lift()),
            ("sinkListItem", sink_list_item("list_item")),
            ("indent", sink_list_item("list_item")),
            ("insertHorizontalRule", insert_horizontal_rule()),
            ("insertHardBreak", insert_hard_break()),
            ("splitBlock", split_block()),
            ("splitListItem", split_list_item("list_item")),
            ("enter", split_block_or_list_item()),
            ("deleteSelection", delete_selection()),
            ("deleteCharBackward", delete_char_backward()),
            ("deleteCharForward", delete_char_forward()),
        ];
        for level in 1..=6i64 {
            let name: &'static str = match level {
                1 => "setHeading1",
                2 => "setHeading2",
                3 => "setHeading3",
                4 => "setHeading4",
                5 => "setHeading5",
                _ => "setHeading6",
            };
            let attrs = Attrs::from_iter([("level", crate::AttrValue::Int(level))]);
            cmds.push((name, set_block_type("heading", attrs)));
        }
        cmds
    }

    fn keymap(&self) -> Vec<(KeyBinding, &'static str)> {
        [
            ("Mod-b", "toggleBold"),
            ("Mod-i", "toggleItalic"),
            ("Mod-u", "toggleUnderline"),
            ("Mod-Shift-s", "toggleStrike"),
            ("Mod-e", "toggleCode"),
            ("Enter", "enter"),
            ("Shift-Enter", "insertHardBreak"),
            ("Backspace", "deleteCharBackward"),
            ("Delete", "deleteCharForward"),
            ("Tab", "sinkListItem"),
            ("Shift-Tab", "liftListItem"),
            ("Mod-Shift-8", "toggleBulletList"),
            ("Mod-Shift-9", "toggleOrderedList"),
            ("Mod-Shift-b", "wrapInBlockquote"),
            ("Mod-Shift-0", "setParagraph"),
            ("Mod-Alt-1", "setHeading1"),
            ("Mod-Alt-2", "setHeading2"),
            ("Mod-Alt-3", "setHeading3"),
        ]
        .into_iter()
        .filter_map(|(b, cmd)| KeyBinding::parse(b).map(|kb| (kb, cmd)))
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::Selection;
    use crate::{Schema, default_plugins};

    fn editor(text: &str) -> EditorState {
        let s = Rc::new(Schema::starter_kit());
        let para = s
            .branch("paragraph", Fragment::from_node(s.text(text).unwrap()))
            .unwrap();
        let doc = s.branch("doc", Fragment::from_node(para)).unwrap();
        EditorState::create(s, doc, vec![Rc::new(BaseCommandsPlugin)])
    }

    fn all_text(node: &Node) -> String {
        if let Some(t) = node.text() {
            return t.to_string();
        }
        node.content().iter().map(all_text).collect()
    }

    #[test]
    fn toggle_bold_over_range() {
        let mut state = editor("abcd");
        state.selection = Selection::text(Pos(1), Pos(5)); // whole "abcd"
        let next = state.run("toggleBold").expect("toggleBold applies");
        let bolded = next
            .doc
            .child(0)
            .content()
            .iter()
            .all(|n| n.marks().iter().any(|m| m.type_name() == "bold"));
        assert!(bolded, "whole range should be bold");
        // toggling again removes it
        let cleared = next.run("toggleBold").unwrap();
        assert!(!range_has_mark(
            &cleared.doc,
            1,
            5,
            cleared.schema().mark_type("bold").unwrap()
        ));
    }

    #[test]
    fn cursor_bold_then_type_is_bold() {
        // audit S2 — toggle bold at a collapsed cursor sets stored marks; typing applies them
        let mut state = editor("ab");
        state.selection = Selection::cursor(Pos(3)); // end of "ab"
        let state = state
            .run("toggleBold")
            .expect("toggleBold (cursor) applies");
        assert!(is_mark_active(
            &state,
            state.schema().mark_type("bold").unwrap()
        ));
        let mut tr = state.tr();
        tr.insert_text("X").unwrap();
        let state = state.apply(tr);
        let x_is_bold =
            state.doc.child(0).content().iter().any(|n| {
                n.text() == Some("X") && n.marks().iter().any(|m| m.type_name() == "bold")
            });
        assert!(x_is_bold, "typed X after cursor-bold should be bold");
    }

    #[test]
    fn set_heading_and_back_to_paragraph() {
        let state = editor("title");
        let h = state.run("setHeading2").expect("setHeading2 applies");
        assert_eq!(h.doc.child(0).type_name(), "heading");
        assert_eq!(h.doc.child(0).attrs().get_int("level"), Some(2));
        // setHeading2 again is a no-op (already that markup) → not applicable
        assert!(!h.can_run("setHeading2"));
        let p = h.run("setParagraph").expect("setParagraph applies");
        assert_eq!(p.doc.child(0).type_name(), "paragraph");
    }

    #[test]
    fn wrap_blockquote_and_lift_back() {
        let state = editor("x");
        let bq = state.run("wrapInBlockquote").expect("wrap applies");
        assert_eq!(bq.doc.child(0).type_name(), "blockquote");
        assert!(in_node_type(&bq, "blockquote"));
        let lifted = bq.run("liftListItem").expect("lift applies");
        assert_eq!(lifted.doc.child(0).type_name(), "paragraph");
    }

    #[test]
    fn toggle_bullet_list_and_back() {
        let state = editor("item");
        let list = state.run("toggleBulletList").expect("wrap into list");
        assert_eq!(list.doc.child(0).type_name(), "bullet_list");
        assert_eq!(list.doc.child(0).child(0).type_name(), "list_item");
        assert!(in_node_type(&list, "bullet_list"));
        // toggling again lifts out of the list
        let back = list.run("toggleBulletList").expect("lift out of list");
        assert!(!in_node_type(&back, "bullet_list"));
        assert_eq!(back.doc.child(0).type_name(), "paragraph");
    }

    #[test]
    fn sink_nests_second_list_item() {
        // build doc(bullet_list(list_item(p"a"), list_item(p"b"))); cursor in 2nd item
        let s = Rc::new(Schema::starter_kit());
        let li = |t: &str| {
            s.branch(
                "list_item",
                Fragment::from_node(
                    s.branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
                        .unwrap(),
                ),
            )
            .unwrap()
        };
        let list = s
            .branch(
                "bullet_list",
                Fragment::from_children(vec![li("a"), li("b")]),
            )
            .unwrap();
        let doc = s.branch("doc", Fragment::from_node(list)).unwrap();
        let mut state = EditorState::create(s, doc, vec![Rc::new(BaseCommandsPlugin)]);
        // positions: 0[ul 1[li 2[p 3 a 4]5]6 [li 7[p 8 b 9]10]11]12 ; cursor in 2nd item's text
        state.selection = Selection::cursor(Pos(8));
        assert!(can_sink(&state));
        let nested = state.run("sinkListItem").expect("sink applies");
        // the second item is now nested inside the first item's sublist
        let outer_list = nested.doc.child(0);
        assert_eq!(outer_list.type_name(), "bullet_list");
        assert_eq!(
            outer_list.child_count(),
            1,
            "second item nested under the first"
        );
        let first_item = outer_list.child(0);
        // first item now holds: paragraph "a", bullet_list(list_item(p"b"))
        let inner_list = first_item.child(first_item.child_count() - 1);
        assert_eq!(inner_list.type_name(), "bullet_list");
        assert_eq!(all_text(inner_list), "b");
    }

    #[test]
    fn split_block_in_paragraph() {
        let mut state = editor("ab");
        state.selection = Selection::cursor(Pos(2)); // between a,b
        let split = state.run("splitBlock").expect("split applies");
        assert_eq!(split.doc.child_count(), 2);
        assert_eq!(all_text(split.doc.child(0)), "a");
        assert_eq!(all_text(split.doc.child(1)), "b");
        // cursor is at the start of the second block
        assert_eq!(split.selection, Selection::cursor(Pos(4)));
    }

    #[test]
    fn split_at_end_of_heading_makes_paragraph() {
        let s = Rc::new(Schema::starter_kit());
        let h = s
            .branch("heading", Fragment::from_node(s.text("T").unwrap()))
            .unwrap();
        let doc = s.branch("doc", Fragment::from_node(h)).unwrap();
        let mut state = EditorState::create(s, doc, vec![Rc::new(BaseCommandsPlugin)]);
        state.selection = Selection::cursor(Pos(2)); // end of heading "T"
        let split = state.run("splitBlock").unwrap();
        assert_eq!(split.doc.child(0).type_name(), "heading");
        assert_eq!(split.doc.child(1).type_name(), "paragraph");
    }

    // ── Enter in lists (splitListItem / liftEmptyBlock) ───────────────────────

    fn list_doc(s: &Schema, items: &[&str]) -> Node {
        let lis: Vec<Node> = items.iter().map(|t| li_of(s, t)).collect();
        let list = s
            .branch("bullet_list", Fragment::from_children(lis))
            .unwrap();
        s.branch("doc", Fragment::from_node(list)).unwrap()
    }

    #[test]
    fn enter_in_list_item_splits_into_new_item() {
        // doc(bullet_list(list_item(p"ab"))); cursor between a,b. Positions are
        // 0-based in doc content: list@0(c@1) item@1(c@2) p@2(c@3) "a"@3 "b"@4.
        let s = Rc::new(Schema::starter_kit());
        let state = state_of(list_doc(&s, &["ab"]), s, 4);
        let next = state.run("enter").expect("enter splits the list item");
        let list = next.doc.child(0);
        assert_eq!(list.type_name(), "bullet_list");
        assert_eq!(list.child_count(), 2, "split produced two list items");
        assert_eq!(list.child(0).type_name(), "list_item");
        assert_eq!(list.child(1).type_name(), "list_item");
        assert_eq!(all_text(list.child(0)), "a");
        assert_eq!(all_text(list.child(1)), "b");
    }

    #[test]
    fn enter_at_end_of_list_item_makes_empty_item() {
        // doc(bullet_list(list_item(p"ab"))); cursor at end of "ab" (after 'b').
        let s = Rc::new(Schema::starter_kit());
        let state = state_of(list_doc(&s, &["ab"]), s, 5);
        let next = state.run("enter").expect("enter adds a new item");
        let list = next.doc.child(0);
        assert_eq!(list.child_count(), 2);
        assert_eq!(all_text(list.child(0)), "ab");
        assert_eq!(all_text(list.child(1)), "", "new item is empty");
        assert_eq!(list.child(1).child(0).type_name(), "paragraph");
    }

    #[test]
    fn enter_in_empty_trailing_list_item_exits_list() {
        // doc(bullet_list(list_item(p"ab"), list_item(p""))); cursor in the empty
        // 2nd item — Enter should lift it out of the list.
        let s = Rc::new(Schema::starter_kit());
        let empty_li = s
            .branch(
                "list_item",
                Fragment::from_node(s.branch("paragraph", Fragment::empty()).unwrap()),
            )
            .unwrap();
        let list = s
            .branch(
                "bullet_list",
                Fragment::from_children(vec![li_of(&s, "ab"), empty_li]),
            )
            .unwrap();
        let doc = s.branch("doc", Fragment::from_node(list)).unwrap();
        // 0-based: list@0(c@1) li1@1(c@2) p"ab"@2(c@3) "ab"@3-5 li1.close@6 li2@7(c@8) p""@8(c@9)
        let state = state_of(doc, s, 9);
        let next = state.run("enter").expect("enter exits the empty item");
        // The empty item left the list: a top-level paragraph now follows a
        // single-item list (the paragraph is no longer inside any list).
        assert!(
            !in_node_type(&next, "bullet_list"),
            "cursor block escaped the list"
        );
        assert_eq!(next.doc.child(0).type_name(), "bullet_list");
        assert_eq!(
            next.doc.child(0).child_count(),
            1,
            "list shrank to one item"
        );
        assert_eq!(next.doc.child(1).type_name(), "paragraph");
    }

    #[test]
    fn enter_in_plain_paragraph_still_splits_block() {
        // The chain falls through to split_block when not in a list item.
        let mut state = editor("ab");
        state.selection = Selection::cursor(Pos(2));
        let next = state.run("enter").expect("enter splits the paragraph");
        assert_eq!(next.doc.child_count(), 2);
        assert_eq!(all_text(next.doc.child(0)), "a");
        assert_eq!(all_text(next.doc.child(1)), "b");
    }

    #[test]
    fn backspace_joins_blocks() {
        let s = Rc::new(Schema::starter_kit());
        let p = |t: &str| {
            s.branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
                .unwrap()
        };
        let doc = s
            .branch("doc", Fragment::from_children(vec![p("ab"), p("cd")]))
            .unwrap();
        let mut state = EditorState::create(s, doc, vec![Rc::new(BaseCommandsPlugin)]);
        state.selection = Selection::cursor(Pos(5)); // start of second paragraph
        let joined = state.run("deleteCharBackward").expect("join applies");
        assert_eq!(joined.doc.child_count(), 1);
        assert_eq!(all_text(&joined.doc), "abcd");
    }

    #[test]
    fn delete_forward_joins_next_block() {
        let s = Rc::new(Schema::starter_kit());
        let p = |t: &str| {
            s.branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
                .unwrap()
        };
        let doc = s
            .branch("doc", Fragment::from_children(vec![p("ab"), p("cd")]))
            .unwrap();
        let mut state = EditorState::create(s, doc, vec![Rc::new(BaseCommandsPlugin)]);
        state.selection = Selection::cursor(Pos(3)); // end of first paragraph
        let joined = state
            .run("deleteCharForward")
            .expect("forward-join applies");
        assert_eq!(joined.doc.child_count(), 1);
        assert_eq!(all_text(&joined.doc), "abcd");
        // at the very end of the doc, forward-delete does nothing
        let mut at_end = editor("xy");
        at_end.selection = Selection::cursor(Pos(3));
        assert!(!at_end.can_run("deleteCharForward"));
    }

    #[test]
    fn delete_forward_removes_char_mid_text() {
        let mut state = editor("abc");
        state.selection = Selection::cursor(Pos(2)); // between a,b
        let next = state.run("deleteCharForward").unwrap();
        assert_eq!(all_text(&next.doc), "ac");
    }

    #[test]
    fn backspace_deletes_char_mid_text() {
        let mut state = editor("abc");
        state.selection = Selection::cursor(Pos(4)); // after "abc"
        let next = state.run("deleteCharBackward").unwrap();
        assert_eq!(all_text(&next.doc), "ab");
    }

    // ── cross-wrapper joins (M5.3 — the M4 `deleteBarrier` deferral) ──────────

    fn p_of(s: &Schema, t: &str) -> Node {
        s.branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
            .unwrap()
    }
    fn li_of(s: &Schema, t: &str) -> Node {
        s.branch("list_item", Fragment::from_node(p_of(s, t)))
            .unwrap()
    }
    fn state_of(doc: Node, schema: Rc<Schema>, cursor: usize) -> EditorState {
        let mut state = EditorState::create(schema, doc, vec![Rc::new(BaseCommandsPlugin)]);
        state.selection = Selection::cursor(Pos(cursor));
        state
    }

    #[test]
    fn backspace_lifts_paragraph_out_of_blockquote() {
        // doc(blockquote(p"abc")); cursor at start of the paragraph (pos 2).
        let s = Rc::new(Schema::starter_kit());
        let bq = s
            .branch("blockquote", Fragment::from_node(p_of(&s, "abc")))
            .unwrap();
        let doc = s.branch("doc", Fragment::from_node(bq)).unwrap();
        let state = state_of(doc, s, 2);
        let lifted = state.run("deleteCharBackward").expect("backspace lifts");
        // The paragraph escaped the blockquote (the M4-deferred case is no longer a no-op).
        assert!(!in_node_type(&lifted, "blockquote"));
        assert_eq!(lifted.doc.child(0).type_name(), "paragraph");
        assert_eq!(all_text(&lifted.doc), "abc");
    }

    #[test]
    fn backspace_joins_sibling_paragraphs_inside_blockquote() {
        // doc(blockquote(p"a", p"b")); cursor at start of the 2nd paragraph.
        let s = Rc::new(Schema::starter_kit());
        let bq = s
            .branch(
                "blockquote",
                Fragment::from_children(vec![p_of(&s, "a"), p_of(&s, "b")]),
            )
            .unwrap();
        let doc = s.branch("doc", Fragment::from_node(bq)).unwrap();
        // positions: 0[bq 1[p 2 a 3]4[p 5 b 6]7]8 ; start of 2nd para = pos 5
        let state = state_of(doc, s, 5);
        let joined = state
            .run("deleteCharBackward")
            .expect("in-wrapper join applies");
        let bq = joined.doc.child(0);
        assert_eq!(bq.type_name(), "blockquote");
        assert_eq!(bq.child_count(), 1, "the two paragraphs joined into one");
        assert_eq!(all_text(&joined.doc), "ab");
    }

    #[test]
    fn backspace_joins_adjacent_list_items() {
        // doc(bullet_list(list_item(p"a"), list_item(p"b"))); cursor in 2nd item.
        let s = Rc::new(Schema::starter_kit());
        let ul = s
            .branch(
                "bullet_list",
                Fragment::from_children(vec![li_of(&s, "a"), li_of(&s, "b")]),
            )
            .unwrap();
        let doc = s.branch("doc", Fragment::from_node(ul)).unwrap();
        // start of 2nd list item's paragraph = pos 8
        let state = state_of(doc, s, 8);
        let joined = state
            .run("deleteCharBackward")
            .expect("list-item join applies");
        let ul = joined.doc.child(0);
        assert_eq!(ul.type_name(), "bullet_list");
        assert_eq!(ul.child_count(), 1, "the two items joined into one");
        // PM-faithful: a depth-1 join of the two list items leaves their two
        // paragraphs stacked inside the surviving item (NOT a single "ab"
        // paragraph). A further Backspace would then join the paragraphs.
        let item = ul.child(0);
        assert_eq!(item.type_name(), "list_item");
        assert_eq!(
            item.child_count(),
            2,
            "both paragraphs survive inside the item"
        );
        assert_eq!(all_text(&joined.doc), "ab");
    }

    #[test]
    fn forward_delete_near_block_atom_does_not_panic() {
        // Regression (adversarial Finding 1): forward-Delete at the end of a
        // paragraph followed by `blockquote(hr)` synthesizes a node-selection of
        // the hr, whose block range is degenerate; the lift path must fail
        // gracefully, never panic on an out-of-bounds path index.
        let s = Rc::new(Schema::starter_kit());
        let hr = s.branch("horizontal_rule", Fragment::empty()).unwrap();
        let bq = s.branch("blockquote", Fragment::from_node(hr)).unwrap();
        let doc = s
            .branch("doc", Fragment::from_children(vec![p_of(&s, "a"), bq]))
            .unwrap();
        let state = state_of(doc, s, 2); // end of "a"
        // Must return cleanly (a no-op here), not panic.
        assert!(state.run("deleteCharForward").is_none());
    }

    #[test]
    fn outdent_on_block_atom_node_selection_does_not_panic() {
        // Regression (adversarial Finding 1): the `lift`/outdent path on a
        // node-selection of a block atom must not panic.
        use crate::selection::{NodeSelection, Selection as Sel};
        let s = Rc::new(Schema::starter_kit());
        let hr = s.branch("horizontal_rule", Fragment::empty()).unwrap();
        let bq = s.branch("blockquote", Fragment::from_node(hr)).unwrap();
        let doc = s.branch("doc", Fragment::from_node(bq)).unwrap();
        let mut state = EditorState::create(s, doc, vec![Rc::new(BaseCommandsPlugin)]);
        // Select the hr (positions: 0[bq 1[hr]2]3 → node selection 1..2).
        state.selection = Sel::Node(NodeSelection {
            anchor: Pos(1),
            head: Pos(2),
        });
        // Should not panic; applicability/exec just reports inapplicable.
        let _ = state.can_run("outdent");
        let _ = state.run("outdent");
    }

    #[test]
    fn backspace_at_start_of_first_block_in_doc_is_noop() {
        // A top-level paragraph at the very start of the doc has nothing to join or lift.
        let mut state = editor("abc");
        state.selection = Selection::cursor(Pos(1)); // start of the only paragraph
        assert!(!state.can_run("deleteCharBackward"));
    }

    #[test]
    fn cross_wrapper_backspace_round_trips_under_undo() {
        // The lift is a real, invertible transaction — undo restores the blockquote.
        let s = Rc::new(Schema::starter_kit());
        let bq = s
            .branch("blockquote", Fragment::from_node(p_of(&s, "abc")))
            .unwrap();
        let doc = s.branch("doc", Fragment::from_node(bq.clone())).unwrap();
        let mut state = EditorState::create(s, doc, default_plugins());
        state.selection = Selection::cursor(Pos(2));
        let lifted = state.run("deleteCharBackward").expect("lift");
        assert!(!in_node_type(&lifted, "blockquote"));
        let undone = lifted.run("undo").expect("undo applies");
        assert!(in_node_type(&undone, "blockquote"));
        assert_eq!(all_text(&undone.doc), "abc");
    }

    #[test]
    fn insert_hard_break_and_hr() {
        let mut state = editor("ab");
        state.selection = Selection::cursor(Pos(2));
        let hb = state.run("insertHardBreak").unwrap();
        assert!(
            hb.doc
                .child(0)
                .content()
                .iter()
                .any(|n| n.type_name() == "hard_break")
        );

        let mut state2 = editor("ab");
        state2.selection = Selection::cursor(Pos(3)); // end of "ab"
        // hr inserts at a block gap; place cursor at the doc-level gap after the para
        state2.selection = Selection::text(Pos(4), Pos(4));
        let hr = state2.run("insertHorizontalRule").unwrap();
        assert!(
            hr.doc
                .content()
                .iter()
                .any(|n| n.type_name() == "horizontal_rule")
        );
    }

    #[test]
    fn link_command_adds_and_removes() {
        let mut state = editor("abcd");
        state.selection = Selection::text(Pos(1), Pos(5));
        let cmd = toggle_link("https://x.test".into());
        let linked = state.run_command(&cmd).expect("link applies");
        let has_link = linked.doc.child(0).content().iter().any(|n| {
            n.marks().iter().any(|m| {
                m.type_name() == "link" && m.attrs.get_str("href") == Some("https://x.test")
            })
        });
        assert!(has_link);
        let unlinked = linked.run("removeLink").expect("removeLink applies");
        assert!(!range_has_mark(
            &unlinked.doc,
            1,
            5,
            unlinked.schema().mark_type("link").unwrap()
        ));
    }

    #[test]
    fn toolbar_queries_read_state() {
        let mut state = editor("abcd");
        state.selection = Selection::text(Pos(1), Pos(5));
        let bold = state.run("toggleBold").unwrap();
        assert!(is_mark_active(
            &bold,
            bold.schema().mark_type("bold").unwrap()
        ));
        assert_eq!(
            current_block_type(&bold).map(|t| t.name().to_string()),
            Some("paragraph".to_string())
        );
        let listed = bold.run("toggleBulletList").unwrap();
        assert!(in_node_type(&listed, "bullet_list"));
        // inside a list the textblock is still a paragraph (A6's point)
        assert_eq!(
            current_block_type(&listed).map(|t| t.name().to_string()),
            Some("paragraph".to_string())
        );
    }
}
