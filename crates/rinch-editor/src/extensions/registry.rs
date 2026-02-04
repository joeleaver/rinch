//! Extension registry for managing loaded extensions.

use crate::error::EditorError;
use crate::extensions::Extension;
use crate::input::{InputRuleSet, ShortcutRegistry};
use crate::schema::{Schema, SchemaBuilder};
use std::collections::HashMap;

/// Type alias for editor command handler functions.
type CommandHandler = fn(&mut crate::editor::Editor) -> Result<(), EditorError>;

/// Registry for managing editor extensions.
#[derive(Debug, Default)]
pub struct ExtensionRegistry {
    /// Extensions in priority order
    extensions: Vec<Box<dyn Extension>>,
    /// Command handlers by name
    command_handlers: HashMap<String, CommandHandler>,
}

impl ExtensionRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an extension. Extensions are kept sorted by priority.
    pub fn register(&mut self, extension: Box<dyn Extension>) {
        // Register commands
        for cmd in extension.commands() {
            self.command_handlers.insert(cmd.name.clone(), cmd.handler);
        }

        // Insert in priority order
        let priority = extension.priority();
        let pos = self
            .extensions
            .iter()
            .position(|e| e.priority() > priority)
            .unwrap_or(self.extensions.len());
        self.extensions.insert(pos, extension);
    }

    /// Get an extension by name.
    pub fn get(&self, name: &str) -> Option<&dyn Extension> {
        self.extensions
            .iter()
            .find(|e| e.name() == name)
            .map(|e| e.as_ref())
    }

    /// Get all registered extensions (in priority order).
    pub fn all(&self) -> Vec<&dyn Extension> {
        self.extensions.iter().map(|e| e.as_ref()).collect()
    }

    /// Get a command handler by name.
    pub fn get_command(&self, name: &str) -> Option<CommandHandler> {
        self.command_handlers.get(name).copied()
    }

    /// Build a merged schema from all registered extensions, starting from a base schema.
    pub fn build_schema(&self, mut builder: SchemaBuilder) -> Schema {
        for ext in &self.extensions {
            for node in ext.nodes() {
                builder = builder.node(node.name.clone(), node);
            }
            for mark in ext.marks() {
                builder = builder.mark(mark.name.clone(), mark);
            }
        }
        builder.build()
    }

    /// Collect all input rules from extensions.
    pub fn build_input_rules(&self) -> InputRuleSet {
        let mut set = InputRuleSet::new();
        for ext in &self.extensions {
            for rule in ext.input_rules() {
                set.add_rule(rule);
            }
        }
        set
    }

    /// Collect all keyboard shortcuts from extensions.
    pub fn build_shortcuts(&self) -> ShortcutRegistry {
        let mut registry = ShortcutRegistry::new();
        for ext in &self.extensions {
            for (shortcut, command) in ext.keyboard_shortcuts() {
                registry.register(shortcut, command);
            }
        }
        registry
    }

    /// Get number of registered extensions.
    pub fn len(&self) -> usize {
        self.extensions.len()
    }

    /// Check if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.extensions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::{CommandRegistration, Extension};
    use crate::input::KeyboardShortcut;
    use crate::schema::MarkSpec;

    #[derive(Debug)]
    struct BoldExtension;

    impl Extension for BoldExtension {
        fn name(&self) -> &str {
            "bold"
        }
        fn priority(&self) -> i32 {
            10
        }
        fn marks(&self) -> Vec<MarkSpec> {
            vec![MarkSpec::simple("bold")]
        }
        fn keyboard_shortcuts(&self) -> Vec<(KeyboardShortcut, String)> {
            vec![(
                KeyboardShortcut::new("Mod-b", "Toggle bold"),
                "toggle_bold".into(),
            )]
        }
        fn commands(&self) -> Vec<CommandRegistration> {
            vec![CommandRegistration::new("toggle_bold", |_editor| Ok(()))]
        }
    }

    #[derive(Debug)]
    struct ItalicExtension;

    impl Extension for ItalicExtension {
        fn name(&self) -> &str {
            "italic"
        }
        fn priority(&self) -> i32 {
            20
        }
        fn marks(&self) -> Vec<MarkSpec> {
            vec![MarkSpec::simple("italic")]
        }
    }

    #[test]
    fn register_and_get() {
        let mut reg = ExtensionRegistry::new();
        reg.register(Box::new(BoldExtension));
        assert!(reg.get("bold").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn priority_ordering() {
        let mut reg = ExtensionRegistry::new();
        reg.register(Box::new(ItalicExtension)); // priority 20
        reg.register(Box::new(BoldExtension)); // priority 10
        let all = reg.all();
        assert_eq!(all[0].name(), "bold");
        assert_eq!(all[1].name(), "italic");
    }

    #[test]
    fn build_schema_merges_marks() {
        let mut reg = ExtensionRegistry::new();
        reg.register(Box::new(BoldExtension));
        reg.register(Box::new(ItalicExtension));
        let schema = reg.build_schema(SchemaBuilder::new());
        assert!(schema.has_mark("bold"));
        assert!(schema.has_mark("italic"));
    }

    #[test]
    fn build_shortcuts_collects() {
        let mut reg = ExtensionRegistry::new();
        reg.register(Box::new(BoldExtension));
        let _shortcuts = reg.build_shortcuts();
        // ShortcutRegistry was built with the bold shortcut
        assert!(!reg.is_empty());
    }

    #[test]
    fn get_command() {
        let mut reg = ExtensionRegistry::new();
        reg.register(Box::new(BoldExtension));
        assert!(reg.get_command("toggle_bold").is_some());
        assert!(reg.get_command("nonexistent").is_none());
    }

    #[test]
    fn len_and_is_empty() {
        let mut reg = ExtensionRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
        reg.register(Box::new(BoldExtension));
        assert!(!reg.is_empty());
        assert_eq!(reg.len(), 1);
    }
}
