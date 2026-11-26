//! Block formatting: block type, margin indent, text alignment and the
//! paragraph reset.

use crate::command::Command;
use crate::model::Attrs;
use crate::pos::Pos;
use crate::schema::data_attrs::carry_app_data;
use crate::state::{EditorState, Transaction};
use crate::transform::block_range_simple;

use super::marks::remove_marks_of_type;
use super::queries::sel_ranges;
use super::wrapping::{build_lift_list_item, build_sink};
use super::{command_tr, in_node_type};

// ===================================================================
// Block types
// ===================================================================

/// Set every textblock overlapping the selection to `node_type` with `attrs` —
/// `setParagraph` / `setHeading{level}` / `setCodeBlock`. Applies only if some
/// textblock in range does not already have that markup. A block keeps its app
/// data attributes where it stays the same element
/// ([`Transform::set_block_type`](crate::transform::Transform::set_block_type)).
pub fn set_block_type(node_type: &'static str, attrs: Attrs) -> Command {
    command_tr(move |state| {
        let ty = state.schema().node_type(node_type)?.clone();
        if !ty.is_textblock() {
            return None;
        }
        let ranges = sel_ranges(state);
        let mut applies = false;
        for &(from, to) in &ranges {
            state
                .doc
                .nodes_between(from, to, &mut |node, _pos, _parent| {
                    if node.is_textblock()
                        && !node.has_markup(&ty, &carry_app_data(&ty, &attrs, node))
                    {
                        applies = true;
                    }
                    true
                });
        }
        if !applies {
            return None;
        }
        // A block-type change keeps every block's size, so the ranges hold.
        let mut tr = state.tr();
        for (from, to) in ranges {
            tr.set_block_type(from, to, ty.clone(), attrs.clone())
                .ok()?;
        }
        tr.doc_changed().then_some(tr)
    })
}

/// Maximum textblock indent depth (a defensive clamp).
const MAX_INDENT: i64 = 12;

/// Increase (`delta > 0`) or decrease (`delta < 0`) the `indent` attribute of every
/// indentable textblock overlapping the selection, clamped to `[0, MAX_INDENT]`.
/// Applies only when at least one block's indent actually changes, so outdent at
/// indent 0 is a no-op. **Skips a textblock inside a list/task item** (depth is
/// controlled by nesting, not a margin — otherwise the bullet/checkbox would split
/// from its text) **or a table cell**. Returns the transaction, or `None` when nothing
/// changed.
fn build_adjust_indent(state: &EditorState, delta: i64) -> Option<Transaction> {
    let (from, to) = (state.selection.from().0, state.selection.to().0);
    // `(position-before-block, current indent)` for each indentable textblock.
    let mut targets: Vec<(usize, i64)> = Vec::new();
    state.doc.nodes_between(from, to, &mut |node, pos, parent| {
        // A textblock inside a list/task item (depth via nesting) or a table cell (Tab
        // is cell navigation there) isn't margin-indented.
        let excluded_parent = parent.is_some_and(|p| {
            matches!(
                p.type_name(),
                "list_item" | "task_item" | "table_cell" | "table_header_cell"
            )
        });
        if matches!(node.type_name(), "paragraph" | "heading") && !excluded_parent {
            targets.push((pos, node.attrs().get_int("indent").unwrap_or(0)));
        }
        true
    });
    if targets.is_empty() {
        return None;
    }
    let mut tr = state.tr();
    // `SetNodeAttrStep` has an empty position map, so the collected `pos` values stay
    // valid across every step — no remapping needed.
    for (pos, cur) in targets {
        let next = (cur + delta).clamp(0, MAX_INDENT);
        if next != cur {
            tr.set_node_attr(pos, "indent", crate::AttrValue::Int(next))
                .ok()?;
        }
    }
    tr.doc_changed().then_some(tr)
}

/// **Indent**: when the cursor is in a list or task list, nest the current item under
/// its previous sibling (a no-op for the first item — there is nothing to nest under).
/// Otherwise increase the textblock's indent level. Indenting in a (task) list never
/// margin-indents the item's paragraph; list depth is purely nesting.
pub fn indent() -> Command {
    command_tr(|state| {
        if in_node_type(state, "list_item") {
            build_sink(state, "list_item")
        } else if in_node_type(state, "task_item") {
            build_sink(state, "task_item")
        } else {
            build_adjust_indent(state, 1)
        }
    })
}

/// **Outdent**: when the cursor is in a list or task list, un-nest the current item
/// (lifting it out of the list entirely at the top level). Otherwise decrease the
/// textblock's indent level. It leaves blockquotes alone (that un-quote is the
/// Paragraph button's job).
pub fn outdent() -> Command {
    command_tr(|state| {
        if in_node_type(state, "list_item") {
            build_lift_list_item(state, "list_item")
        } else if in_node_type(state, "task_item") {
            build_lift_list_item(state, "task_item")
        } else {
            build_adjust_indent(state, -1)
        }
    })
}

/// The alignment values the `text_align` attribute accepts. An **unset**
/// alignment (the attribute absent) is the default: it projects to no
/// `text-align`, so the stylesheet decides (a `th`'s, an RTL editor's `start`).
/// A stored `"left"` is an explicit choice and projects to `text-align: left`
/// in the view and in HTML (#2210).
pub const TEXT_ALIGN_VALUES: [&str; 4] = ["left", "center", "right", "justify"];

/// Set the `text_align` attribute of every alignable textblock (paragraph /
/// heading) overlapping the selection to `align` (`setTextAlignLeft` /
/// `setTextAlignCenter` / `setTextAlignRight` / `setTextAlignJustify`). Applies
/// only when at least one block's alignment actually changes, so re-selecting the
/// current alignment is a no-op (the toolbar button can stay inert). An unset
/// alignment is not `left`: align-left on it stores `left` (#2210). Unlike the
/// margin indent, alignment is fine inside lists and table cells, so no block is
/// excluded by its parent.
pub fn set_text_align(align: &'static str) -> Command {
    command_tr(move |state| build_set_text_align(state, align))
}

/// Build the set-alignment transaction. `align` must be one of
/// [`TEXT_ALIGN_VALUES`]; an unknown value yields no transaction.
fn build_set_text_align(state: &EditorState, align: &str) -> Option<Transaction> {
    if !TEXT_ALIGN_VALUES.contains(&align) {
        return None;
    }
    // `(position-before-block, current alignment)` for each alignable textblock;
    // `None` when unset, which no command value equals (#2210).
    let mut targets: Vec<(usize, Option<String>)> = Vec::new();
    for (from, to) in sel_ranges(state) {
        state
            .doc
            .nodes_between(from, to, &mut |node, pos, _parent| {
                if matches!(node.type_name(), "paragraph" | "heading") {
                    let cur = node.attrs().get_str("text_align").map(str::to_string);
                    targets.push((pos, cur));
                }
                true
            });
    }
    if targets.is_empty() {
        return None;
    }
    let mut tr = state.tr();
    // `SetNodeAttrStep` has an empty position map, so the collected `pos` values
    // stay valid across every step — no remapping needed (mirrors the indent cmd).
    for (pos, cur) in targets {
        if cur.as_deref() != Some(align) {
            tr.set_node_attr(pos, "text_align", crate::AttrValue::Str(align.into()))
                .ok()?;
        }
    }
    tr.doc_changed().then_some(tr)
}

/// The **Paragraph** command — "reset to normal text". It strips *every* kind of
/// formatting off the selection so the result is plain paragraph text:
///
/// 1. removes all inline marks (bold/italic/underline/link/…),
/// 2. converts the block(s) to paragraphs (heading / code block → paragraph),
/// 3. resets the indent level to 0, unsets the alignment and removes the
///    blocks' [app data attributes](crate::schema::data_attrs),
/// 4. lifts the block(s) out of every wrapper (blockquote, list) to the top level.
///
/// A no-op (returns `None`) only when the selection is already plain top-level
/// paragraph text with no marks and no app data attributes.
pub fn set_paragraph() -> Command {
    command_tr(|state| {
        let para_ty = state.schema().node_type("paragraph")?.clone();
        // Each selected cell's content for a cell selection. Every step below
        // keeps positions (marks, block types and attrs change no size).
        let ranges = sel_ranges(state);
        let mut tr = state.tr();

        // 1. Strip every inline mark over the selection. Mark steps don't shift
        //    positions, so `from`/`to` stay valid for the block ops below. Iterate the
        //    schema's marks in a deterministic (sorted) order.
        let mut mark_names: Vec<String> = state.schema().marks.keys().cloned().collect();
        mark_names.sort();
        for name in &mark_names {
            if let Some(mt) = state.schema().mark_type(name) {
                for &(from, to) in &ranges {
                    remove_marks_of_type(&mut tr, &state.doc, from, to, mt).ok()?;
                }
            }
        }

        // 2. Convert the selected textblock(s) to plain paragraphs. `set_block_type`
        //    preserves block-boundary positions (same open/close token count).
        for &(from, to) in &ranges {
            let _ = tr.set_block_type(from, to, para_ty.clone(), Attrs::new());
        }

        // 3. Reset the indent attribute to 0 and unset the alignment (an explicit
        //    `left` included, #2210) on the (now-)paragraphs in range — but only
        //    where they aren't already at their defaults, so a plain paragraph
        //    stays a true no-op.
        let mut indent_targets: Vec<usize> = Vec::new();
        let mut align_targets: Vec<usize> = Vec::new();
        for &(from, to) in &ranges {
            state
                .doc
                .nodes_between(from, to, &mut |node, pos, _parent| {
                    if matches!(node.type_name(), "paragraph" | "heading") {
                        if node.attrs().get_int("indent").unwrap_or(0) != 0 {
                            indent_targets.push(pos);
                        }
                        if node.attrs().get("text_align").is_some() {
                            align_targets.push(pos);
                        }
                    }
                    true
                });
        }
        for pos in indent_targets {
            let _ = tr.set_node_attr(pos, "indent", crate::AttrValue::Int(0));
        }
        let removals = align_targets
            .into_iter()
            .map(|pos| (pos, "text_align".to_string()));
        for (pos, attr) in removals {
            let _ = tr.step(Box::new(crate::transform::steps::SetNodeAttrStep {
                pos,
                attr,
                value: None,
            }));
        }

        // 4. Lift every selected block out of its blockquote/list wrappers to the top
        //    level (never out of a table cell — see `lift_blocks_to_top`).
        lift_blocks_to_top(&mut tr);

        tr.doc_changed().then_some(tr)
    })
}

/// A wrapper the Paragraph reset is allowed to peel off — a blockquote or any list
/// node. Notably NOT a table cell (or any other isolating container): lifting a
/// paragraph out of a cell would hoist it out of the table and corrupt the structure.
fn is_liftable_wrapper(type_name: &str) -> bool {
    matches!(
        type_name,
        "blockquote" | "bullet_list" | "ordered_list" | "list_item" | "task_list" | "task_item"
    )
}

/// Lift every textblock in the selection out of its enclosing blockquote/list wrappers
/// until it is a direct child of the document. Works block-by-block (so a multi-block
/// selection spanning different wrappers is fully reset), peeling one wrapper per pass
/// and re-scanning — the transaction maps its selection forward through each lift.
/// A block whose ancestor chain includes a non-liftable container (e.g. a table cell)
/// is left untouched, so the Paragraph button never breaks a table.
fn lift_blocks_to_top(tr: &mut Transaction) {
    loop {
        let (from, to) = (tr.selection().from().0, tr.selection().to().0);
        let doc = tr.doc().clone();
        // The first textblock in range that is still wrapped (depth >= 2) and whose
        // every wrapper is liftable. Resolve a point *inside* the block (pos + 1).
        let mut inside: Option<usize> = None;
        doc.nodes_between(from, to, &mut |node, pos, _parent| {
            if inside.is_none()
                && node.is_textblock()
                && let Ok(r) = doc.resolve(Pos(pos + 1))
                && r.depth() >= 2
                && (1..r.depth()).all(|d| is_liftable_wrapper(r.node(d).type_name()))
            {
                inside = Some(pos + 1);
            }
            true
        });
        let Some(inside) = inside else { break };
        let Ok(r) = doc.resolve(Pos(inside)) else {
            break;
        };
        let range = match block_range_simple(&r, &r) {
            Some(x) if x.depth > 0 => x,
            _ => break,
        };
        let before = tr.doc().clone();
        let mut lifted = false;
        for target in (0..range.depth).rev() {
            if tr.lift(&range, target).is_ok() && tr.doc() != &before {
                lifted = true;
                break;
            }
        }
        if !lifted {
            break;
        }
    }
}
