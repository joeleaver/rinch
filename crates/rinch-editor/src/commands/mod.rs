//! Command system for document mutations.

mod dispatch;
mod text;
mod formatting;
mod structure;

pub use dispatch::{Command, CommandDispatcher};
pub use text::TextCommands;
pub use formatting::FormattingCommands;
pub use structure::StructureCommands;
