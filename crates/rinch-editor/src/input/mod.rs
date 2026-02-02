//! Input handling (keyboard, rules, shortcuts).

mod rules;
mod shortcuts;

pub use rules::{InputRule, InputRuleSet};
pub use shortcuts::{KeyboardShortcut, ShortcutRegistry};
