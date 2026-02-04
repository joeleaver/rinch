//! Extension system for adding functionality to the editor.

mod registry;
pub mod starter_kit;
pub mod table;
pub mod table_model;

pub use registry::ExtensionRegistry;
pub use starter_kit::StarterKit;
pub use table::TableExtension;
pub use table_model::TableModel;

use crate::schema::{NodeSpec, MarkSpec};
use crate::input::{InputRule, KeyboardShortcut};
use crate::editor::Editor;
use crate::error::EditorError;

/// A command registration: name + handler function.
#[derive(Clone)]
pub struct CommandRegistration {
    /// Command name (e.g., "toggle_bold")
    pub name: String,
    /// Command handler
    pub handler: fn(&mut Editor) -> Result<(), EditorError>,
}

impl CommandRegistration {
    /// Create a new command registration.
    pub fn new(name: impl Into<String>, handler: fn(&mut Editor) -> Result<(), EditorError>) -> Self {
        Self {
            name: name.into(),
            handler,
        }
    }
}

impl std::fmt::Debug for CommandRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandRegistration")
            .field("name", &self.name)
            .finish()
    }
}

/// Trait for editor extensions.
///
/// Extensions can add:
/// - New node types
/// - New mark types
/// - Commands
/// - Keyboard shortcuts
/// - Input rules
pub trait Extension: std::fmt::Debug {
    /// Get the extension name.
    fn name(&self) -> &str;

    /// Get the priority for ordering (lower = earlier). Default is 100.
    fn priority(&self) -> i32 {
        100
    }

    /// Get node specs provided by this extension.
    fn nodes(&self) -> Vec<NodeSpec> {
        Vec::new()
    }

    /// Get mark specs provided by this extension.
    fn marks(&self) -> Vec<MarkSpec> {
        Vec::new()
    }

    /// Get input rules provided by this extension.
    fn input_rules(&self) -> Vec<InputRule> {
        Vec::new()
    }

    /// Get keyboard shortcuts provided by this extension.
    fn keyboard_shortcuts(&self) -> Vec<(KeyboardShortcut, String)> {
        Vec::new()
    }

    /// Get commands provided by this extension.
    fn commands(&self) -> Vec<CommandRegistration> {
        Vec::new()
    }

    /// Called when the extension is initialized with the editor.
    fn on_init(&self, _editor: &mut Editor) -> Result<(), EditorError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct TestExtension;

    impl Extension for TestExtension {
        fn name(&self) -> &str { "test" }
        fn priority(&self) -> i32 { 50 }
        fn commands(&self) -> Vec<CommandRegistration> {
            vec![CommandRegistration::new("test_cmd", |_editor| Ok(()))]
        }
    }

    #[test]
    fn extension_default_priority() {
        #[derive(Debug)]
        struct DefaultExt;
        impl Extension for DefaultExt {
            fn name(&self) -> &str { "default" }
        }
        let ext = DefaultExt;
        assert_eq!(ext.priority(), 100);
    }

    #[test]
    fn extension_custom_priority() {
        let ext = TestExtension;
        assert_eq!(ext.priority(), 50);
    }

    #[test]
    fn extension_commands() {
        let ext = TestExtension;
        let cmds = ext.commands();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].name, "test_cmd");
    }

    #[test]
    fn extension_default_methods_return_empty() {
        #[derive(Debug)]
        struct MinimalExt;
        impl Extension for MinimalExt {
            fn name(&self) -> &str { "minimal" }
        }
        let ext = MinimalExt;
        assert!(ext.nodes().is_empty());
        assert!(ext.marks().is_empty());
        assert!(ext.input_rules().is_empty());
        assert!(ext.keyboard_shortcuts().is_empty());
        assert!(ext.commands().is_empty());
    }

    #[test]
    fn command_registration_debug() {
        let reg = CommandRegistration::new("test", |_| Ok(()));
        let dbg = format!("{:?}", reg);
        assert!(dbg.contains("test"));
    }
}
