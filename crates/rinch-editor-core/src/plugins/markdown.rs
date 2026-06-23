//! The markdown-shortcut input rules as a [`Plugin`] (`## ` → heading, `- ` →
//! bullet list, `> ` → blockquote, `1. ` → ordered list, ` ``` ` → code block).

use crate::input_rules::{InputRule, markdown_input_rules};
use crate::plugin::{Plugin, PluginKey};

/// Contributes the default markdown input rules.
#[derive(Debug, Default)]
pub struct MarkdownInputRulesPlugin;

impl Plugin for MarkdownInputRulesPlugin {
    fn key(&self) -> PluginKey {
        PluginKey("markdown-input-rules")
    }

    fn input_rules(&self) -> Vec<InputRule> {
        markdown_input_rules()
    }
}
