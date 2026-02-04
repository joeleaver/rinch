//! Command system for document mutations.

mod dispatch;
mod formatting;
mod structure;
mod text;

pub use dispatch::{Command, CommandDispatcher};
pub use formatting::FormattingCommands;
pub use structure::StructureCommands;
pub use text::TextCommands;
