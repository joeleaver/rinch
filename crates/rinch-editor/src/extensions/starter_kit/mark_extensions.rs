//! Mark extensions: BoldExt, ItalicExt, StrikeExt, CodeExt, UnderlineExt,
//! LinkExt, HighlightExt, SubscriptExt, SuperscriptExt, TextColorExt,
//! TextAlignExt.

use regex::Regex;
use std::collections::HashMap;

use crate::commands::{FormattingCommands, StructureCommands};
use crate::extensions::{CommandRegistration, Extension};
use crate::input::{InputRule, KeyboardShortcut};
use crate::schema::mark::MarkSpec;
use crate::schema::node::AttrSpec;

/// Bold formatting (`<strong>`).
#[derive(Debug)]
pub struct BoldExt;

impl Extension for BoldExt {
    fn name(&self) -> &str {
        "bold"
    }

    fn marks(&self) -> Vec<MarkSpec> {
        let mut spec = MarkSpec::simple("bold");
        spec.parse_html_tags = vec!["strong".into(), "b".into()];
        vec![spec]
    }

    fn commands(&self) -> Vec<CommandRegistration> {
        vec![CommandRegistration::new("toggle_bold", |editor| {
            FormattingCommands::toggle_mark(editor, "bold")
        })]
    }

    fn keyboard_shortcuts(&self) -> Vec<(KeyboardShortcut, String)> {
        vec![(
            KeyboardShortcut::new("Mod-B", "Toggle bold"),
            "toggle_bold".into(),
        )]
    }

    fn input_rules(&self) -> Vec<InputRule> {
        vec![InputRule::new(
            Regex::new(r"\*\*(.+)\*\*$").unwrap(),
            "Bold (markdown)",
            |editor, _caps| FormattingCommands::toggle_mark(editor, "bold"),
        )]
    }
}

/// Italic formatting (`<em>`).
#[derive(Debug)]
pub struct ItalicExt;

impl Extension for ItalicExt {
    fn name(&self) -> &str {
        "italic"
    }

    fn marks(&self) -> Vec<MarkSpec> {
        let mut spec = MarkSpec::simple("italic");
        spec.parse_html_tags = vec!["em".into(), "i".into()];
        vec![spec]
    }

    fn commands(&self) -> Vec<CommandRegistration> {
        vec![CommandRegistration::new("toggle_italic", |editor| {
            FormattingCommands::toggle_mark(editor, "italic")
        })]
    }

    fn keyboard_shortcuts(&self) -> Vec<(KeyboardShortcut, String)> {
        vec![(
            KeyboardShortcut::new("Mod-I", "Toggle italic"),
            "toggle_italic".into(),
        )]
    }

    fn input_rules(&self) -> Vec<InputRule> {
        vec![InputRule::new(
            Regex::new(r"\*(.+)\*$").unwrap(),
            "Italic (markdown)",
            |editor, _caps| FormattingCommands::toggle_mark(editor, "italic"),
        )]
    }
}

/// Strikethrough formatting (`<s>`).
#[derive(Debug)]
pub struct StrikeExt;

impl Extension for StrikeExt {
    fn name(&self) -> &str {
        "strike"
    }

    fn marks(&self) -> Vec<MarkSpec> {
        let mut spec = MarkSpec::simple("strike");
        spec.parse_html_tags = vec!["s".into(), "del".into(), "strike".into()];
        vec![spec]
    }

    fn commands(&self) -> Vec<CommandRegistration> {
        vec![CommandRegistration::new("toggle_strike", |editor| {
            FormattingCommands::toggle_mark(editor, "strike")
        })]
    }

    fn keyboard_shortcuts(&self) -> Vec<(KeyboardShortcut, String)> {
        vec![(
            KeyboardShortcut::new("Mod-Shift-X", "Toggle strikethrough"),
            "toggle_strike".into(),
        )]
    }

    fn input_rules(&self) -> Vec<InputRule> {
        vec![InputRule::new(
            Regex::new(r"~~(.+)~~$").unwrap(),
            "Strikethrough (markdown)",
            |editor, _caps| FormattingCommands::toggle_mark(editor, "strike"),
        )]
    }
}

/// Inline code formatting (`<code>`).
#[derive(Debug)]
pub struct CodeExt;

impl Extension for CodeExt {
    fn name(&self) -> &str {
        "code"
    }

    fn marks(&self) -> Vec<MarkSpec> {
        let mut spec = MarkSpec::simple("code");
        spec.excludes = Some("bold italic underline strike link".to_string());
        spec.parse_html_tags = vec!["code".into()];
        vec![spec]
    }

    fn commands(&self) -> Vec<CommandRegistration> {
        vec![CommandRegistration::new("toggle_code", |editor| {
            FormattingCommands::toggle_mark(editor, "code")
        })]
    }

    fn keyboard_shortcuts(&self) -> Vec<(KeyboardShortcut, String)> {
        vec![(
            KeyboardShortcut::new("Mod-E", "Toggle inline code"),
            "toggle_code".into(),
        )]
    }
}

/// Underline formatting (`<u>`).
#[derive(Debug)]
pub struct UnderlineExt;

impl Extension for UnderlineExt {
    fn name(&self) -> &str {
        "underline"
    }

    fn marks(&self) -> Vec<MarkSpec> {
        let mut spec = MarkSpec::simple("underline");
        spec.parse_html_tags = vec!["u".into()];
        vec![spec]
    }

    fn commands(&self) -> Vec<CommandRegistration> {
        vec![CommandRegistration::new("toggle_underline", |editor| {
            FormattingCommands::toggle_mark(editor, "underline")
        })]
    }

    fn keyboard_shortcuts(&self) -> Vec<(KeyboardShortcut, String)> {
        vec![(
            KeyboardShortcut::new("Mod-U", "Toggle underline"),
            "toggle_underline".into(),
        )]
    }
}

/// Link mark (`<a href>`).
#[derive(Debug)]
pub struct LinkExt;

impl Extension for LinkExt {
    fn name(&self) -> &str {
        "link"
    }

    fn marks(&self) -> Vec<MarkSpec> {
        let mut attrs = HashMap::new();
        attrs.insert("href".into(), AttrSpec::required());
        attrs.insert("title".into(), AttrSpec::optional(""));
        attrs.insert("target".into(), AttrSpec::optional(""));
        let mut spec = MarkSpec::with_attrs("link", attrs);
        spec.parse_html_tags = vec!["a".into()];
        vec![spec]
    }

    fn commands(&self) -> Vec<CommandRegistration> {
        vec![
            CommandRegistration::new("set_link", |editor| {
                // Link setting requires a URL, typically provided via dialog.
                // This is a no-op placeholder; real usage passes attrs.
                FormattingCommands::toggle_mark(editor, "link")
            }),
            CommandRegistration::new("unset_link", |editor| {
                FormattingCommands::remove_mark(editor, "link")
            }),
        ]
    }
}

/// Highlight mark (`<mark>`).
#[derive(Debug)]
pub struct HighlightExt;

impl Extension for HighlightExt {
    fn name(&self) -> &str {
        "highlight"
    }

    fn marks(&self) -> Vec<MarkSpec> {
        let mut attrs = HashMap::new();
        attrs.insert("color".into(), AttrSpec::optional(""));
        let mut spec = MarkSpec::with_attrs("highlight", attrs);
        spec.parse_html_tags = vec!["mark".into()];
        vec![spec]
    }

    fn commands(&self) -> Vec<CommandRegistration> {
        vec![CommandRegistration::new("toggle_highlight", |editor| {
            FormattingCommands::toggle_mark(editor, "highlight")
        })]
    }

    fn keyboard_shortcuts(&self) -> Vec<(KeyboardShortcut, String)> {
        vec![(
            KeyboardShortcut::new("Mod-Shift-H", "Toggle highlight"),
            "toggle_highlight".into(),
        )]
    }
}

/// Subscript mark (`<sub>`).
#[derive(Debug)]
pub struct SubscriptExt;

impl Extension for SubscriptExt {
    fn name(&self) -> &str {
        "subscript"
    }

    fn marks(&self) -> Vec<MarkSpec> {
        let mut spec = MarkSpec::simple("subscript");
        spec.excludes = Some("superscript".to_string());
        spec.parse_html_tags = vec!["sub".into()];
        vec![spec]
    }

    fn commands(&self) -> Vec<CommandRegistration> {
        vec![CommandRegistration::new("toggle_subscript", |editor| {
            FormattingCommands::toggle_mark(editor, "subscript")
        })]
    }

    fn keyboard_shortcuts(&self) -> Vec<(KeyboardShortcut, String)> {
        vec![(
            KeyboardShortcut::new("Mod-,", "Toggle subscript"),
            "toggle_subscript".into(),
        )]
    }
}

/// Superscript mark (`<sup>`).
#[derive(Debug)]
pub struct SuperscriptExt;

impl Extension for SuperscriptExt {
    fn name(&self) -> &str {
        "superscript"
    }

    fn marks(&self) -> Vec<MarkSpec> {
        let mut spec = MarkSpec::simple("superscript");
        spec.excludes = Some("subscript".to_string());
        spec.parse_html_tags = vec!["sup".into()];
        vec![spec]
    }

    fn commands(&self) -> Vec<CommandRegistration> {
        vec![CommandRegistration::new("toggle_superscript", |editor| {
            FormattingCommands::toggle_mark(editor, "superscript")
        })]
    }

    fn keyboard_shortcuts(&self) -> Vec<(KeyboardShortcut, String)> {
        vec![(
            KeyboardShortcut::new("Mod-.", "Toggle superscript"),
            "toggle_superscript".into(),
        )]
    }
}

/// Custom text color mark.
#[derive(Debug)]
pub struct TextColorExt;

impl Extension for TextColorExt {
    fn name(&self) -> &str {
        "text_color"
    }

    fn marks(&self) -> Vec<MarkSpec> {
        let mut attrs = HashMap::new();
        attrs.insert("color".into(), AttrSpec::required());
        vec![MarkSpec::with_attrs("text_color", attrs)]
    }

    fn commands(&self) -> Vec<CommandRegistration> {
        vec![
            CommandRegistration::new("set_text_color", |editor| {
                // Color value would be provided via dialog or programmatic API.
                FormattingCommands::toggle_mark(editor, "text_color")
            }),
            CommandRegistration::new("unset_text_color", |editor| {
                FormattingCommands::remove_mark(editor, "text_color")
            }),
        ]
    }
}

/// Text alignment extension (block-level attribute).
#[derive(Debug)]
pub struct TextAlignExt;

impl Extension for TextAlignExt {
    fn name(&self) -> &str {
        "text_align"
    }

    fn commands(&self) -> Vec<CommandRegistration> {
        vec![
            CommandRegistration::new("align_left", |editor| {
                StructureCommands::set_block_attr(editor, "text_align", "left")
            }),
            CommandRegistration::new("align_center", |editor| {
                StructureCommands::set_block_attr(editor, "text_align", "center")
            }),
            CommandRegistration::new("align_right", |editor| {
                StructureCommands::set_block_attr(editor, "text_align", "right")
            }),
            CommandRegistration::new("align_justify", |editor| {
                StructureCommands::set_block_attr(editor, "text_align", "justify")
            }),
        ]
    }

    fn keyboard_shortcuts(&self) -> Vec<(KeyboardShortcut, String)> {
        vec![
            (
                KeyboardShortcut::new("Mod-Shift-L", "Align left"),
                "align_left".into(),
            ),
            (
                KeyboardShortcut::new("Mod-Shift-E", "Align center"),
                "align_center".into(),
            ),
            (
                KeyboardShortcut::new("Mod-Shift-R", "Align right"),
                "align_right".into(),
            ),
            (
                KeyboardShortcut::new("Mod-Shift-J", "Align justify"),
                "align_justify".into(),
            ),
        ]
    }
}
