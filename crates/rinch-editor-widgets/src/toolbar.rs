//! Toolbar configuration for the rich-text editor.
//!
//! Defines toolbar layout, grouping, and available controls.
//! This is pure configuration data — no DOM rendering logic.

/// Visual style of the toolbar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolbarStyle {
    /// Always visible at top of editor.
    Fixed,
    /// Shows on text selection (floating).
    Bubble,
    /// Both fixed toolbar and bubble menu.
    Both,
}

/// A single toolbar control (button/action).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolbarControl {
    Bold,
    Italic,
    Underline,
    Strike,
    Code,
    Highlight,
    Subscript,
    Superscript,
    Link,
    TextColor(String),
    Heading(u8),
    Paragraph,
    BulletList,
    OrderedList,
    Blockquote,
    CodeBlock,
    HorizontalRule,
    HardBreak,
    Undo,
    Redo,
    ClearFormatting,
    InsertTable,
    AlignLeft,
    AlignCenter,
    AlignRight,
    AlignJustify,
    Custom {
        label: String,
        command: String,
        icon: Option<String>,
    },
}

/// A named group of toolbar controls, rendered with a visual separator.
#[derive(Debug, Clone)]
pub struct ToolbarGroup {
    pub label: String,
    pub controls: Vec<ToolbarControl>,
}

impl ToolbarGroup {
    pub fn new(label: impl Into<String>, controls: Vec<ToolbarControl>) -> Self {
        Self {
            label: label.into(),
            controls,
        }
    }
}

/// Top-level toolbar configuration.
#[derive(Debug, Clone)]
pub struct ToolbarConfig {
    pub groups: Vec<ToolbarGroup>,
    pub sticky: bool,
    pub style: ToolbarStyle,
}

impl Default for ToolbarConfig {
    fn default() -> Self {
        Self::default_full()
    }
}

impl ToolbarConfig {
    /// Full toolbar with all controls (~35+), organized into groups.
    pub fn default_full() -> Self {
        Self {
            sticky: true,
            style: ToolbarStyle::Fixed,
            groups: vec![
                ToolbarGroup::new("History", vec![ToolbarControl::Undo, ToolbarControl::Redo]),
                ToolbarGroup::new(
                    "Text Formatting",
                    vec![
                        ToolbarControl::Bold,
                        ToolbarControl::Italic,
                        ToolbarControl::Underline,
                        ToolbarControl::Strike,
                        ToolbarControl::Code,
                        ToolbarControl::Highlight,
                        ToolbarControl::Subscript,
                        ToolbarControl::Superscript,
                    ],
                ),
                ToolbarGroup::new(
                    "Headings",
                    vec![
                        ToolbarControl::Heading(1),
                        ToolbarControl::Heading(2),
                        ToolbarControl::Heading(3),
                        ToolbarControl::Heading(4),
                        ToolbarControl::Heading(5),
                        ToolbarControl::Heading(6),
                        ToolbarControl::Paragraph,
                    ],
                ),
                ToolbarGroup::new(
                    "Lists",
                    vec![
                        ToolbarControl::BulletList,
                        ToolbarControl::OrderedList,
                        ToolbarControl::Blockquote,
                    ],
                ),
                ToolbarGroup::new(
                    "Alignment",
                    vec![
                        ToolbarControl::AlignLeft,
                        ToolbarControl::AlignCenter,
                        ToolbarControl::AlignRight,
                        ToolbarControl::AlignJustify,
                    ],
                ),
                ToolbarGroup::new(
                    "Insert",
                    vec![
                        ToolbarControl::Link,
                        ToolbarControl::HorizontalRule,
                        ToolbarControl::HardBreak,
                        ToolbarControl::CodeBlock,
                        ToolbarControl::InsertTable,
                    ],
                ),
                ToolbarGroup::new(
                    "Color",
                    vec![
                        ToolbarControl::TextColor("red".into()),
                        ToolbarControl::TextColor("blue".into()),
                        ToolbarControl::TextColor("green".into()),
                        ToolbarControl::TextColor("orange".into()),
                        ToolbarControl::TextColor("purple".into()),
                    ],
                ),
                ToolbarGroup::new("Clear", vec![ToolbarControl::ClearFormatting]),
            ],
        }
    }

    /// Minimal toolbar: bold, italic, link, headings 1-3.
    pub fn default_minimal() -> Self {
        Self {
            sticky: true,
            style: ToolbarStyle::Fixed,
            groups: vec![
                ToolbarGroup::new(
                    "Formatting",
                    vec![
                        ToolbarControl::Bold,
                        ToolbarControl::Italic,
                        ToolbarControl::Link,
                    ],
                ),
                ToolbarGroup::new(
                    "Headings",
                    vec![
                        ToolbarControl::Heading(1),
                        ToolbarControl::Heading(2),
                        ToolbarControl::Heading(3),
                    ],
                ),
            ],
        }
    }

    /// Toolbar optimized for markdown-style editing.
    pub fn default_markdown() -> Self {
        Self {
            sticky: true,
            style: ToolbarStyle::Fixed,
            groups: vec![
                ToolbarGroup::new(
                    "Formatting",
                    vec![
                        ToolbarControl::Bold,
                        ToolbarControl::Italic,
                        ToolbarControl::Strike,
                        ToolbarControl::Code,
                    ],
                ),
                ToolbarGroup::new(
                    "Headings",
                    vec![
                        ToolbarControl::Heading(1),
                        ToolbarControl::Heading(2),
                        ToolbarControl::Heading(3),
                        ToolbarControl::Paragraph,
                    ],
                ),
                ToolbarGroup::new(
                    "Blocks",
                    vec![
                        ToolbarControl::BulletList,
                        ToolbarControl::OrderedList,
                        ToolbarControl::Blockquote,
                        ToolbarControl::CodeBlock,
                    ],
                ),
                ToolbarGroup::new(
                    "Insert",
                    vec![ToolbarControl::Link, ToolbarControl::HorizontalRule],
                ),
                ToolbarGroup::new("History", vec![ToolbarControl::Undo, ToolbarControl::Redo]),
            ],
        }
    }

    /// Total number of controls across all groups.
    pub fn control_count(&self) -> usize {
        self.groups.iter().map(|g| g.controls.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_full_has_at_least_35_controls() {
        let config = ToolbarConfig::default_full();
        assert!(
            config.control_count() >= 35,
            "full toolbar has {} controls",
            config.control_count()
        );
    }

    #[test]
    fn default_full_has_expected_groups() {
        let config = ToolbarConfig::default_full();
        let labels: Vec<&str> = config.groups.iter().map(|g| g.label.as_str()).collect();
        assert!(labels.contains(&"History"));
        assert!(labels.contains(&"Text Formatting"));
        assert!(labels.contains(&"Headings"));
        assert!(labels.contains(&"Lists"));
        assert!(labels.contains(&"Alignment"));
        assert!(labels.contains(&"Insert"));
        assert!(labels.contains(&"Clear"));
    }

    #[test]
    fn default_minimal_is_small() {
        let config = ToolbarConfig::default_minimal();
        assert!(config.control_count() <= 10);
        assert!(config.control_count() >= 6);
    }

    #[test]
    fn default_markdown_includes_code_block() {
        let config = ToolbarConfig::default_markdown();
        let all: Vec<&ToolbarControl> = config.groups.iter().flat_map(|g| &g.controls).collect();
        assert!(all.contains(&&ToolbarControl::CodeBlock));
    }

    #[test]
    fn toolbar_style_defaults_to_fixed() {
        let config = ToolbarConfig::default_full();
        assert_eq!(config.style, ToolbarStyle::Fixed);
    }

    #[test]
    fn toolbar_group_new() {
        let g = ToolbarGroup::new("Test", vec![ToolbarControl::Bold]);
        assert_eq!(g.label, "Test");
        assert_eq!(g.controls.len(), 1);
    }

    #[test]
    fn toolbar_control_equality() {
        assert_eq!(ToolbarControl::Bold, ToolbarControl::Bold);
        assert_ne!(ToolbarControl::Bold, ToolbarControl::Italic);
        assert_eq!(ToolbarControl::Heading(1), ToolbarControl::Heading(1));
        assert_ne!(ToolbarControl::Heading(1), ToolbarControl::Heading(2));
    }

    #[test]
    fn custom_control() {
        let c = ToolbarControl::Custom {
            label: "Emoji".into(),
            command: "insertEmoji".into(),
            icon: Some("emoji".into()),
        };
        if let ToolbarControl::Custom {
            label,
            command,
            icon,
        } = &c
        {
            assert_eq!(label, "Emoji");
            assert_eq!(command, "insertEmoji");
            assert_eq!(icon.as_deref(), Some("emoji"));
        } else {
            panic!("expected Custom variant");
        }
    }

    #[test]
    fn text_color_control() {
        let c = ToolbarControl::TextColor("#ff0000".into());
        assert_eq!(c, ToolbarControl::TextColor("#ff0000".into()));
    }
}
