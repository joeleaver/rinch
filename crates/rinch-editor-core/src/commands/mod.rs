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

use crate::command::Command;
use crate::state::{EditorState, Transaction};

mod base_plugin;
mod block_format;
pub mod code;
mod data_attrs;
mod delete;
mod marks;
mod queries;
mod structure;
pub mod table_ops;
mod wrapping;

pub use base_plugin::BaseCommandsPlugin;
pub use block_format::{
    TEXT_ALIGN_VALUES, indent, outdent, set_block_type, set_paragraph, set_text_align,
};
pub use code::{exit_code, insert_line_break, newline_in_code};
pub use data_attrs::{block_data_attr, set_block_data_attr};
pub use delete::{
    delete_char_backward, delete_char_forward, delete_selection, delete_word_backward,
    delete_word_forward,
};
pub use marks::{remove_link, select_all, toggle_link, toggle_mark};
pub use queries::{
    can_lift, can_sink, current_block_type, in_node_type, is_mark_active, marks_at, range_has_mark,
};
pub use structure::{
    build_table, create_paragraph_near, insert_hard_break, insert_horizontal_rule, insert_image,
    insert_table, lift_empty_block, split_block, split_block_or_list_item, split_list_item,
};
pub use wrapping::{lift, sink_list_item, sink_list_item_any, toggle_list, wrap_in};

// ===================================================================
// Command plumbing
// ===================================================================

/// Wrap a transaction-builder into a [`Command`]. The builder returns `Some(tr)`
/// when the command applies (even a stored-marks-only transaction with no doc
/// change), or `None` when it does not.
pub(crate) fn command_tr(build: impl Fn(&EditorState) -> Option<Transaction> + 'static) -> Command {
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

#[cfg(test)]
mod tests;
