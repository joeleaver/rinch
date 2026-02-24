//! Status bar rendering for the editor.
//!
//! Shows cursor position, block type, active marks, and document stats.
//! Reactively updates when `change_count` signal changes.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::reactive::{Effect, Signal};

use rinch_editor::Editor;

/// Render the status bar showing editor state.
///
/// The status bar reactively updates whenever `change_count` changes.
/// The caller should increment `change_count` on each `on_change` callback
/// from the editor.
pub fn render_status_bar(
    scope: &mut RenderScope,
    editor: &Rc<RefCell<Editor>>,
    change_count: Signal<usize>,
) -> NodeHandle {
    let bar = scope.create_element("div");
    bar.set_attribute("class", "editor-status-bar");
    bar.set_attribute(
        "style",
        "display: flex; gap: 16px; align-items: center; padding: 6px 12px; \
         font-size: 12px; color: var(--rinch-color-dimmed); \
         border-top: 1px solid var(--rinch-color-gray-3); \
         background: var(--rinch-color-gray-0);",
    );

    // Block type label
    let block_label = scope.create_element("span");
    block_label.set_attribute(
        "style",
        "padding: 2px 8px; border-radius: 4px; background: var(--rinch-color-gray-2); \
         font-weight: 500;",
    );
    bar.append_child(&block_label);

    // Cursor position label
    let pos_label = scope.create_element("span");
    bar.append_child(&pos_label);

    // Active marks container
    let marks_container = scope.create_element("span");
    marks_container.set_attribute("style", "display: flex; gap: 4px;");
    bar.append_child(&marks_container);

    // Spacer
    let spacer = scope.create_element("div");
    spacer.set_attribute("style", "flex: 1;");
    bar.append_child(&spacer);

    // Stats label (word count, char count)
    let stats_label = scope.create_element("span");
    bar.append_child(&stats_label);

    // Single effect that updates all dynamic content when change_count changes
    let editor_clone = editor.clone();
    Effect::new(move || {
        let _ = change_count.get(); // Track the signal

        if let Ok(ed) = editor_clone.try_borrow() {
            let sel = ed.get_selection().clone();

            // Update block type label
            let block_type_str = if let Ok(rp) = ed.doc.resolve_position(sel.head) {
                let bt = ed
                    .doc
                    .block_type(rp.block_index)
                    .unwrap_or_else(|| "unknown".into());
                let attrs = ed.doc.block_attrs(rp.block_index).unwrap_or_default();
                match bt.as_str() {
                    "heading" => {
                        let level = attrs.get("level").map(|s| s.as_str()).unwrap_or("1");
                        format!("Heading {}", level)
                    }
                    "paragraph" => "Paragraph".into(),
                    "blockquote" => "Blockquote".into(),
                    "code_block" => "Code Block".into(),
                    "bullet_list" => "Bullet List".into(),
                    "ordered_list" => "Ordered List".into(),
                    "list_item" => "List Item".into(),
                    "horizontal_rule" => "Horizontal Rule".into(),
                    other => other.to_string(),
                }
            } else {
                "Unknown".into()
            };
            block_label.set_text(&block_type_str);

            // Update cursor position (block-relative offset, not inline-node-relative)
            let pos_text = if let Ok(rp) = ed.doc.resolve_position(sel.head) {
                let block_start = ed.doc.block_start_position(rp.block_index);
                let block_offset = sel.head.0.saturating_sub(block_start);
                format!("Block {}, Offset {}", rp.block_index + 1, block_offset)
            } else {
                format!("Pos {}", sel.head.0)
            };
            pos_label.set_text(&pos_text);

            // Update active marks at cursor
            let marks = ed.doc.marks_at(sel.head);
            if marks.is_empty() {
                marks_container.set_text("");
            } else {
                let mark_text = marks
                    .iter()
                    .map(|m| m.mark_type.as_str())
                    .collect::<Vec<_>>()
                    .join(" | ");
                marks_container.set_text(&mark_text);
            }

            // Update word count and character count
            let text = ed.doc.to_text();
            let char_count = text.chars().count();
            let word_count = text.split_whitespace().count();
            stats_label.set_text(&format!("{} words, {} chars", word_count, char_count));
        }
    });

    bar
}
