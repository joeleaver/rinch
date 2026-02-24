//! Input handling: connects mouse events to editor commands.
//!
//! Registers contentEditable click/drag interceptors that route mouse
//! events to the appropriate editor commands. Keyboard input is handled
//! by app.rs → CeOps, not intercepted by the bridge.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::events::{
    ContentEditableClickData, ContentEditableDragData, clear_ce_click_interceptor,
    clear_ce_drag_interceptor, set_ce_click_interceptor, set_ce_drag_interceptor,
};

use crate::editor::Editor;

/// Install click/drag interceptors for the editor.
///
/// Registers:
/// - CE click interceptor (handles mouse clicks on contentEditable)
/// - CE drag interceptor (handles mouse drag on contentEditable)
///
/// Keyboard input is handled by app.rs → CeOps, not intercepted by the bridge.
/// `on_change` triggers reconciliation after click/drag operations.
pub fn install_interceptors(editor: Rc<RefCell<Editor>>, on_change: Rc<dyn Fn()>) {
    // CE click interceptor
    let editor_for_click = editor.clone();
    let on_change_for_click = on_change.clone();
    set_ce_click_interceptor(move |data: &ContentEditableClickData| {
        handle_ce_click(&editor_for_click, data, &*on_change_for_click)
    });

    // CE drag interceptor
    let editor_for_drag = editor.clone();
    let on_change_for_drag = on_change;
    set_ce_drag_interceptor(move |data: &ContentEditableDragData| {
        handle_ce_drag(&editor_for_drag, data, &*on_change_for_drag)
    });
}

/// Remove all input interceptors.
pub fn remove_interceptors() {
    clear_ce_click_interceptor();
    clear_ce_drag_interceptor();
}

/// Handle a contentEditable click. Returns true if handled.
fn handle_ce_click(
    editor: &Rc<RefCell<Editor>>,
    data: &ContentEditableClickData,
    on_change: &dyn Fn(),
) -> bool {
    // Check if click landed on a table cell
    if let Some(ref cell_ref) = data.closest_table_cell {
        let parts: Vec<&str> = cell_ref.split(':').collect();
        if parts.len() == 3
            && let (Ok(row), Ok(col)) = (parts[1].parse::<usize>(), parts[2].parse::<usize>())
        {
            let table_id = parts[0].to_string();
            if let Ok(mut ed) = editor.try_borrow_mut() {
                // Find the block index for this table
                let block_idx = (0..ed.doc.block_count())
                    .find(|&i| ed.table_id_for_block(i).as_deref() == Some(table_id.as_str()));
                if let Some(block_idx) = block_idx {
                    ed.select_table_cell(table_id, block_idx, row, col);
                    drop(ed);
                    on_change();
                    return true;
                }
            }
        }
    }

    // Not a table click — clear any table selection
    if let Ok(mut ed) = editor.try_borrow_mut()
        && ed.table_selection.is_some()
    {
        ed.clear_table_selection();
        drop(ed);
        on_change();
    }
    false
}

/// Handle a contentEditable drag. Returns true if handled.
fn handle_ce_drag(
    _editor: &Rc<RefCell<Editor>>,
    _data: &ContentEditableDragData,
    _on_change: &dyn Fn(),
) -> bool {
    // Let the browser handle drag selection natively.
    false
}
