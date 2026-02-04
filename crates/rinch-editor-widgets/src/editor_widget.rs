//! Main RichTextEditor widget configuration.
//!
//! Provides a builder-style API for configuring the editor widget.
//! Pure configuration data — rendering is handled by the view layer.

use crate::toolbar::ToolbarConfig;

/// Configuration for the RichTextEditor widget.
#[derive(Debug, Clone)]
pub struct RichTextEditorConfig {
    pub toolbar: ToolbarConfig,
    pub extensions: Vec<String>,
    pub editable: bool,
    pub autofocus: bool,
    pub placeholder: Option<String>,
    pub initial_content: Option<String>,
    pub class: Option<String>,
}

impl Default for RichTextEditorConfig {
    fn default() -> Self {
        Self {
            toolbar: ToolbarConfig::default_full(),
            extensions: Vec::new(),
            editable: true,
            autofocus: false,
            placeholder: None,
            initial_content: None,
            class: None,
        }
    }
}

impl RichTextEditorConfig {
    /// Create a new default config.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set toolbar configuration.
    pub fn with_toolbar(mut self, toolbar: ToolbarConfig) -> Self {
        self.toolbar = toolbar;
        self
    }

    /// Set placeholder text.
    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    /// Set initial HTML content.
    pub fn with_content(mut self, html: impl Into<String>) -> Self {
        self.initial_content = Some(html.into());
        self
    }

    /// Set extra CSS class.
    pub fn with_class(mut self, class: impl Into<String>) -> Self {
        self.class = Some(class.into());
        self
    }

    /// Enable autofocus.
    pub fn autofocus(mut self) -> Self {
        self.autofocus = true;
        self
    }

    /// Make editor read-only.
    pub fn readonly(mut self) -> Self {
        self.editable = false;
        self
    }

    /// Add an extension by name.
    pub fn with_extension(mut self, name: impl Into<String>) -> Self {
        self.extensions.push(name.into());
        self
    }

    /// Add multiple extensions.
    pub fn with_extensions(mut self, names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.extensions.extend(names.into_iter().map(|n| n.into()));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolbar::ToolbarStyle;

    #[test]
    fn default_config() {
        let config = RichTextEditorConfig::default();
        assert!(config.editable);
        assert!(!config.autofocus);
        assert!(config.placeholder.is_none());
        assert!(config.initial_content.is_none());
        assert!(config.extensions.is_empty());
    }

    #[test]
    fn builder_readonly() {
        let config = RichTextEditorConfig::new().readonly();
        assert!(!config.editable);
    }

    #[test]
    fn builder_placeholder() {
        let config = RichTextEditorConfig::new().with_placeholder("Type here...");
        assert_eq!(config.placeholder.as_deref(), Some("Type here..."));
    }

    #[test]
    fn builder_content() {
        let config = RichTextEditorConfig::new().with_content("<p>Hello</p>");
        assert_eq!(config.initial_content.as_deref(), Some("<p>Hello</p>"));
    }

    #[test]
    fn builder_autofocus() {
        let config = RichTextEditorConfig::new().autofocus();
        assert!(config.autofocus);
    }

    #[test]
    fn builder_class() {
        let config = RichTextEditorConfig::new().with_class("my-editor");
        assert_eq!(config.class.as_deref(), Some("my-editor"));
    }

    #[test]
    fn builder_extensions() {
        let config = RichTextEditorConfig::new()
            .with_extension("starterKit")
            .with_extension("table");
        assert_eq!(config.extensions, vec!["starterKit", "table"]);
    }

    #[test]
    fn builder_extensions_batch() {
        let config = RichTextEditorConfig::new()
            .with_extensions(["starterKit", "table", "highlight"]);
        assert_eq!(config.extensions.len(), 3);
    }

    #[test]
    fn builder_with_minimal_toolbar() {
        let config = RichTextEditorConfig::new()
            .with_toolbar(ToolbarConfig::default_minimal());
        assert!(config.toolbar.control_count() < 10);
    }

    #[test]
    fn builder_chaining() {
        let config = RichTextEditorConfig::new()
            .with_placeholder("Write something...")
            .with_content("<p>Initial</p>")
            .with_class("editor")
            .with_toolbar(ToolbarConfig::default_markdown())
            .autofocus()
            .with_extension("table");

        assert!(config.autofocus);
        assert_eq!(config.placeholder.as_deref(), Some("Write something..."));
        assert_eq!(config.initial_content.as_deref(), Some("<p>Initial</p>"));
        assert_eq!(config.class.as_deref(), Some("editor"));
        assert_eq!(config.extensions, vec!["table"]);
        assert_eq!(config.toolbar.style, ToolbarStyle::Fixed);
    }
}
