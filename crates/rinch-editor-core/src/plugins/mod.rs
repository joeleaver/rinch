//! Built-in plugins: history (undo/redo), markdown input rules, and the
//! empty-editor placeholder (M5). Tables (M7), accessibility (M7b), collaboration
//! (M9), and IME-preedit decorations (M6) join here (or in their own crates) as
//! later milestones land.

pub mod history;
pub mod markdown;
pub mod placeholder;

pub use history::{HISTORY_KEY, HistoryPlugin, HistoryState};
pub use markdown::MarkdownInputRulesPlugin;
pub use placeholder::{PLACEHOLDER_KEY, PlaceholderPlugin};
