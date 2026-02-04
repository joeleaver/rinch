//! Individual control button metadata for toolbar controls.
//!
//! Maps each [`ToolbarControl`] variant to its label, tooltip, keyboard shortcut,
//! editor command name, and optional icon name. Pure data — no rendering.

use crate::toolbar::ToolbarControl;

/// Metadata for a single toolbar button.
#[derive(Debug, Clone)]
pub struct ControlButton {
    pub control: ToolbarControl,
    pub label: String,
    pub tooltip: String,
    pub shortcut: Option<String>,
    pub is_active: bool,
    pub is_disabled: bool,
}

impl ControlButton {
    /// Build metadata from a [`ToolbarControl`].
    pub fn from_control(control: ToolbarControl) -> Self {
        let (label, tooltip, shortcut) = control_metadata(&control);
        Self {
            control,
            label: label.to_string(),
            tooltip: tooltip.to_string(),
            shortcut: shortcut.map(|s| s.to_string()),
            is_active: false,
            is_disabled: false,
        }
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn tooltip(&self) -> &str {
        &self.tooltip
    }

    pub fn shortcut_hint(&self) -> Option<&str> {
        self.shortcut.as_deref()
    }

    /// The editor command name this control triggers.
    pub fn command_name(&self) -> &str {
        control_command(&self.control)
    }

    /// Optional icon identifier for future icon integration.
    pub fn icon_name(&self) -> Option<&str> {
        control_icon(&self.control)
    }
}

/// Returns (label, tooltip, optional shortcut) for a control.
fn control_metadata(
    control: &ToolbarControl,
) -> (&'static str, &'static str, Option<&'static str>) {
    match control {
        ToolbarControl::Bold => ("B", "Bold", Some("Ctrl+B")),
        ToolbarControl::Italic => ("I", "Italic", Some("Ctrl+I")),
        ToolbarControl::Underline => ("U", "Underline", Some("Ctrl+U")),
        ToolbarControl::Strike => ("S", "Strikethrough", Some("Ctrl+Shift+X")),
        ToolbarControl::Code => ("<>", "Inline code", Some("Ctrl+E")),
        ToolbarControl::Highlight => ("H", "Highlight", Some("Ctrl+Shift+H")),
        ToolbarControl::Subscript => ("x₂", "Subscript", Some("Ctrl+,")),
        ToolbarControl::Superscript => ("x²", "Superscript", Some("Ctrl+.")),
        ToolbarControl::Link => ("🔗", "Insert link", Some("Ctrl+K")),
        ToolbarControl::TextColor(_) => ("A", "Text color", None),
        ToolbarControl::Heading(1) => ("H1", "Heading 1", Some("Ctrl+Alt+1")),
        ToolbarControl::Heading(2) => ("H2", "Heading 2", Some("Ctrl+Alt+2")),
        ToolbarControl::Heading(3) => ("H3", "Heading 3", Some("Ctrl+Alt+3")),
        ToolbarControl::Heading(4) => ("H4", "Heading 4", Some("Ctrl+Alt+4")),
        ToolbarControl::Heading(5) => ("H5", "Heading 5", Some("Ctrl+Alt+5")),
        ToolbarControl::Heading(6) => ("H6", "Heading 6", Some("Ctrl+Alt+6")),
        ToolbarControl::Heading(_) => ("H?", "Heading", None),
        ToolbarControl::Paragraph => ("¶", "Paragraph", Some("Ctrl+Alt+0")),
        ToolbarControl::BulletList => ("•", "Bullet list", Some("Ctrl+Shift+8")),
        ToolbarControl::OrderedList => ("1.", "Ordered list", Some("Ctrl+Shift+7")),
        ToolbarControl::Blockquote => ("❝", "Blockquote", Some("Ctrl+Shift+B")),
        ToolbarControl::CodeBlock => ("⌨", "Code block", Some("Ctrl+Alt+C")),
        ToolbarControl::HorizontalRule => ("—", "Horizontal rule", None),
        ToolbarControl::HardBreak => ("↵", "Hard break", Some("Shift+Enter")),
        ToolbarControl::Undo => ("↶", "Undo", Some("Ctrl+Z")),
        ToolbarControl::Redo => ("↷", "Redo", Some("Ctrl+Y")),
        ToolbarControl::ClearFormatting => ("Tx", "Clear formatting", None),
        ToolbarControl::InsertTable => ("⊞", "Insert table", None),
        ToolbarControl::AlignLeft => ("≡", "Align left", Some("Ctrl+Shift+L")),
        ToolbarControl::AlignCenter => ("≡", "Align center", Some("Ctrl+Shift+E")),
        ToolbarControl::AlignRight => ("≡", "Align right", Some("Ctrl+Shift+R")),
        ToolbarControl::AlignJustify => ("≡", "Align justify", Some("Ctrl+Shift+J")),
        ToolbarControl::Custom { label, .. } => {
            // Can't return &'static str from dynamic data; handled specially.
            // We leak here for simplicity in a config-only context.
            // In practice custom controls are few.
            let _ = label;
            ("?", "Custom action", None)
        }
    }
}

/// Editor command name for a control.
fn control_command(control: &ToolbarControl) -> &str {
    match control {
        ToolbarControl::Bold => "toggleBold",
        ToolbarControl::Italic => "toggleItalic",
        ToolbarControl::Underline => "toggleUnderline",
        ToolbarControl::Strike => "toggleStrike",
        ToolbarControl::Code => "toggleCode",
        ToolbarControl::Highlight => "toggleHighlight",
        ToolbarControl::Subscript => "toggleSubscript",
        ToolbarControl::Superscript => "toggleSuperscript",
        ToolbarControl::Link => "setLink",
        ToolbarControl::TextColor(_) => "setColor",
        ToolbarControl::Heading(n) => match n {
            1 => "setHeading1",
            2 => "setHeading2",
            3 => "setHeading3",
            4 => "setHeading4",
            5 => "setHeading5",
            6 => "setHeading6",
            _ => "setHeading",
        },
        ToolbarControl::Paragraph => "setParagraph",
        ToolbarControl::BulletList => "toggleBulletList",
        ToolbarControl::OrderedList => "toggleOrderedList",
        ToolbarControl::Blockquote => "toggleBlockquote",
        ToolbarControl::CodeBlock => "toggleCodeBlock",
        ToolbarControl::HorizontalRule => "setHorizontalRule",
        ToolbarControl::HardBreak => "setHardBreak",
        ToolbarControl::Undo => "undo",
        ToolbarControl::Redo => "redo",
        ToolbarControl::ClearFormatting => "clearFormatting",
        ToolbarControl::InsertTable => "insertTable",
        ToolbarControl::AlignLeft => "setTextAlign",
        ToolbarControl::AlignCenter => "setTextAlign",
        ToolbarControl::AlignRight => "setTextAlign",
        ToolbarControl::AlignJustify => "setTextAlign",
        ToolbarControl::Custom { command, .. } => command.as_str(),
    }
}

/// Optional icon identifier.
fn control_icon(control: &ToolbarControl) -> Option<&str> {
    match control {
        ToolbarControl::Bold => Some("bold"),
        ToolbarControl::Italic => Some("italic"),
        ToolbarControl::Underline => Some("underline"),
        ToolbarControl::Strike => Some("strikethrough"),
        ToolbarControl::Code => Some("code"),
        ToolbarControl::Highlight => Some("highlight"),
        ToolbarControl::Subscript => Some("subscript"),
        ToolbarControl::Superscript => Some("superscript"),
        ToolbarControl::Link => Some("link"),
        ToolbarControl::TextColor(_) => Some("palette"),
        ToolbarControl::Heading(_) => Some("heading"),
        ToolbarControl::Paragraph => Some("pilcrow"),
        ToolbarControl::BulletList => Some("list"),
        ToolbarControl::OrderedList => Some("list-numbers"),
        ToolbarControl::Blockquote => Some("blockquote"),
        ToolbarControl::CodeBlock => Some("source-code"),
        ToolbarControl::HorizontalRule => Some("separator-horizontal"),
        ToolbarControl::HardBreak => Some("text-wrap"),
        ToolbarControl::Undo => Some("arrow-back-up"),
        ToolbarControl::Redo => Some("arrow-forward-up"),
        ToolbarControl::ClearFormatting => Some("clear-formatting"),
        ToolbarControl::InsertTable => Some("table"),
        ToolbarControl::AlignLeft => Some("align-left"),
        ToolbarControl::AlignCenter => Some("align-center"),
        ToolbarControl::AlignRight => Some("align-right"),
        ToolbarControl::AlignJustify => Some("align-justified"),
        ToolbarControl::Custom { icon, .. } => icon.as_deref(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bold_metadata() {
        let btn = ControlButton::from_control(ToolbarControl::Bold);
        assert_eq!(btn.label(), "B");
        assert_eq!(btn.tooltip(), "Bold");
        assert_eq!(btn.shortcut_hint(), Some("Ctrl+B"));
        assert_eq!(btn.command_name(), "toggleBold");
        assert_eq!(btn.icon_name(), Some("bold"));
    }

    #[test]
    fn italic_metadata() {
        let btn = ControlButton::from_control(ToolbarControl::Italic);
        assert_eq!(btn.label(), "I");
        assert_eq!(btn.command_name(), "toggleItalic");
        assert_eq!(btn.shortcut_hint(), Some("Ctrl+I"));
    }

    #[test]
    fn heading_levels() {
        for level in 1..=6 {
            let btn = ControlButton::from_control(ToolbarControl::Heading(level));
            assert_eq!(btn.label(), format!("H{}", level));
            assert!(btn.shortcut_hint().is_some());
            assert_eq!(btn.command_name(), format!("setHeading{}", level));
        }
    }

    #[test]
    fn undo_redo() {
        let undo = ControlButton::from_control(ToolbarControl::Undo);
        assert_eq!(undo.command_name(), "undo");
        assert_eq!(undo.shortcut_hint(), Some("Ctrl+Z"));

        let redo = ControlButton::from_control(ToolbarControl::Redo);
        assert_eq!(redo.command_name(), "redo");
        assert_eq!(redo.shortcut_hint(), Some("Ctrl+Y"));
    }

    #[test]
    fn custom_control_command() {
        let btn = ControlButton::from_control(ToolbarControl::Custom {
            label: "Emoji".into(),
            command: "insertEmoji".into(),
            icon: Some("mood-smile".into()),
        });
        assert_eq!(btn.command_name(), "insertEmoji");
        assert_eq!(btn.icon_name(), Some("mood-smile"));
    }

    #[test]
    fn default_state_inactive_and_enabled() {
        let btn = ControlButton::from_control(ToolbarControl::Bold);
        assert!(!btn.is_active);
        assert!(!btn.is_disabled);
    }

    #[test]
    fn all_controls_have_icons() {
        let controls = vec![
            ToolbarControl::Bold,
            ToolbarControl::Italic,
            ToolbarControl::Underline,
            ToolbarControl::Strike,
            ToolbarControl::Code,
            ToolbarControl::Highlight,
            ToolbarControl::Link,
            ToolbarControl::BulletList,
            ToolbarControl::OrderedList,
            ToolbarControl::Undo,
            ToolbarControl::Redo,
        ];
        for c in controls {
            let btn = ControlButton::from_control(c);
            assert!(
                btn.icon_name().is_some(),
                "missing icon for {}",
                btn.label()
            );
        }
    }

    #[test]
    fn alignment_controls() {
        let left = ControlButton::from_control(ToolbarControl::AlignLeft);
        assert_eq!(left.command_name(), "setTextAlign");
        assert_eq!(left.icon_name(), Some("align-left"));

        let center = ControlButton::from_control(ToolbarControl::AlignCenter);
        assert_eq!(center.icon_name(), Some("align-center"));
    }

    #[test]
    fn text_color_control() {
        let btn = ControlButton::from_control(ToolbarControl::TextColor("#ff0000".into()));
        assert_eq!(btn.command_name(), "setColor");
        assert_eq!(btn.icon_name(), Some("palette"));
    }

    #[test]
    fn blockquote_metadata() {
        let btn = ControlButton::from_control(ToolbarControl::Blockquote);
        assert_eq!(btn.command_name(), "toggleBlockquote");
        assert!(btn.shortcut_hint().is_some());
    }

    #[test]
    fn code_block_metadata() {
        let btn = ControlButton::from_control(ToolbarControl::CodeBlock);
        assert_eq!(btn.command_name(), "toggleCodeBlock");
        assert_eq!(btn.icon_name(), Some("source-code"));
    }

    #[test]
    fn horizontal_rule_no_shortcut() {
        let btn = ControlButton::from_control(ToolbarControl::HorizontalRule);
        assert!(btn.shortcut_hint().is_none());
        assert_eq!(btn.command_name(), "setHorizontalRule");
    }

    #[test]
    fn insert_table_metadata() {
        let btn = ControlButton::from_control(ToolbarControl::InsertTable);
        assert_eq!(btn.command_name(), "insertTable");
        assert_eq!(btn.icon_name(), Some("table"));
    }
}
