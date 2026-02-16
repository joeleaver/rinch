//! StarterKit: 23 default extensions for a full-featured rich-text editor.
//!
//! Modeled after TipTap/Mantine's StarterKit, this module provides all the
//! standard node and mark extensions with keyboard shortcuts, commands,
//! and markdown input rules.

use regex::Regex;
use std::collections::HashMap;

use super::{CommandRegistration, Extension};
use crate::commands::{FormattingCommands, StructureCommands, TextCommands};
use crate::document::Range;
use crate::editor::Editor;
use crate::error::EditorError;
use crate::input::{InputRule, KeyboardShortcut};
use crate::schema::mark::MarkSpec;
use crate::schema::node::{AttrSpec, MarkSet, NodeSpec};

// =============================================================================
// Input rule helpers
// =============================================================================

/// Delete the matched prefix text and set the block type.
///
/// Used by markdown input rules (e.g., `# ` -> heading, `> ` -> blockquote).
/// Deletes the prefix from the block start, adjusts cursor, and sets block type.
fn apply_block_input_rule(
    editor: &mut Editor,
    caps: &regex::Captures,
    block_type: &str,
) -> Result<(), EditorError> {
    let prefix_len = caps.get(0).unwrap().as_str().len();
    let sel = editor.get_selection().clone();
    let resolved = editor.doc.resolve_position(sel.head)?;
    let block_start = editor.doc.block_start_position(resolved.block_index);
    TextCommands::delete_range(editor, Range::new(block_start, block_start + prefix_len))?;
    StructureCommands::set_block_type(editor, block_type)?;
    Ok(())
}

/// Heading input rule handler: delete prefix, set heading type with level.
fn heading_input_rule_handler(
    editor: &mut Editor,
    caps: &regex::Captures,
) -> Result<(), EditorError> {
    let hashes = caps.get(1).unwrap().as_str();
    let level = hashes.len();
    let prefix_len = caps.get(0).unwrap().as_str().len();

    let sel = editor.get_selection().clone();
    let resolved = editor.doc.resolve_position(sel.head)?;
    let block_start = editor.doc.block_start_position(resolved.block_index);

    // Delete the prefix text ("# ", "## ", etc.)
    TextCommands::delete_range(editor, Range::new(block_start, block_start + prefix_len))?;

    // Set heading with level
    let mut attrs = HashMap::new();
    attrs.insert("level".to_string(), level.to_string());
    StructureCommands::set_block_type_with_attrs(editor, "heading", attrs)?;

    Ok(())
}

// =============================================================================
// Node Extensions
// =============================================================================

/// Root document node. Content: `block+`.
#[derive(Debug)]
pub struct DocumentExt;

impl Extension for DocumentExt {
    fn name(&self) -> &str {
        "document"
    }
    fn priority(&self) -> i32 {
        0
    }

    fn nodes(&self) -> Vec<NodeSpec> {
        vec![NodeSpec::builder("doc").content("block+").build()]
    }
}

/// Default text block (`<p>`).
#[derive(Debug)]
pub struct ParagraphExt;

impl Extension for ParagraphExt {
    fn name(&self) -> &str {
        "paragraph"
    }
    fn priority(&self) -> i32 {
        10
    }

    fn nodes(&self) -> Vec<NodeSpec> {
        vec![
            NodeSpec::builder("paragraph")
                .content("inline*")
                .group("block")
                .parse_html(vec!["p".into()])
                .build(),
        ]
    }

    fn commands(&self) -> Vec<CommandRegistration> {
        vec![CommandRegistration::new("set_paragraph", |editor| {
            StructureCommands::set_block_type(editor, "paragraph")
        })]
    }

    fn keyboard_shortcuts(&self) -> Vec<(KeyboardShortcut, String)> {
        vec![(
            KeyboardShortcut::new("Mod-Alt-0", "Set paragraph"),
            "set_paragraph".into(),
        )]
    }
}

/// Inline text node.
#[derive(Debug)]
pub struct TextExt;

impl Extension for TextExt {
    fn name(&self) -> &str {
        "text"
    }
    fn priority(&self) -> i32 {
        10
    }

    fn nodes(&self) -> Vec<NodeSpec> {
        vec![NodeSpec::builder("text").group("inline").inline().build()]
    }
}

/// Heading levels 1-6 (`<h1>`..`<h6>`).
#[derive(Debug)]
pub struct HeadingExt;

impl Extension for HeadingExt {
    fn name(&self) -> &str {
        "heading"
    }

    fn nodes(&self) -> Vec<NodeSpec> {
        vec![
            NodeSpec::builder("heading")
                .content("inline*")
                .group("block")
                .attr("level", AttrSpec::optional("1"))
                .parse_html(vec![
                    "h1".into(),
                    "h2".into(),
                    "h3".into(),
                    "h4".into(),
                    "h5".into(),
                    "h6".into(),
                ])
                .build(),
        ]
    }

    fn commands(&self) -> Vec<CommandRegistration> {
        vec![
            CommandRegistration::new("set_heading_1", |editor| {
                StructureCommands::set_block_type(editor, "heading")
            }),
            CommandRegistration::new("set_heading_2", |editor| {
                StructureCommands::set_block_type(editor, "heading")
            }),
            CommandRegistration::new("set_heading_3", |editor| {
                StructureCommands::set_block_type(editor, "heading")
            }),
            CommandRegistration::new("set_heading_4", |editor| {
                StructureCommands::set_block_type(editor, "heading")
            }),
            CommandRegistration::new("set_heading_5", |editor| {
                StructureCommands::set_block_type(editor, "heading")
            }),
            CommandRegistration::new("set_heading_6", |editor| {
                StructureCommands::set_block_type(editor, "heading")
            }),
            CommandRegistration::new("toggle_heading", |editor| {
                let current = editor.doc.block_type(0);
                if current.as_deref() == Some("heading") {
                    StructureCommands::set_block_type(editor, "paragraph")
                } else {
                    StructureCommands::set_block_type(editor, "heading")
                }
            }),
        ]
    }

    fn keyboard_shortcuts(&self) -> Vec<(KeyboardShortcut, String)> {
        vec![
            (
                KeyboardShortcut::new("Mod-Alt-1", "Heading 1"),
                "set_heading_1".into(),
            ),
            (
                KeyboardShortcut::new("Mod-Alt-2", "Heading 2"),
                "set_heading_2".into(),
            ),
            (
                KeyboardShortcut::new("Mod-Alt-3", "Heading 3"),
                "set_heading_3".into(),
            ),
            (
                KeyboardShortcut::new("Mod-Alt-4", "Heading 4"),
                "set_heading_4".into(),
            ),
            (
                KeyboardShortcut::new("Mod-Alt-5", "Heading 5"),
                "set_heading_5".into(),
            ),
            (
                KeyboardShortcut::new("Mod-Alt-6", "Heading 6"),
                "set_heading_6".into(),
            ),
        ]
    }

    fn input_rules(&self) -> Vec<InputRule> {
        vec![InputRule::new(
            Regex::new(r"^(#{1,6}) $").unwrap(),
            "Heading",
            heading_input_rule_handler,
        )]
    }
}

/// Blockquote (`<blockquote>`).
#[derive(Debug)]
pub struct BlockquoteExt;

impl Extension for BlockquoteExt {
    fn name(&self) -> &str {
        "blockquote"
    }

    fn nodes(&self) -> Vec<NodeSpec> {
        vec![
            NodeSpec::builder("blockquote")
                .content("block+")
                .group("block")
                .parse_html(vec!["blockquote".into()])
                .build(),
        ]
    }

    fn commands(&self) -> Vec<CommandRegistration> {
        vec![CommandRegistration::new("toggle_blockquote", |editor| {
            let current = editor.doc.block_type(0);
            if current.as_deref() == Some("blockquote") {
                StructureCommands::set_block_type(editor, "paragraph")
            } else {
                StructureCommands::wrap_in(editor, "blockquote")
            }
        })]
    }

    fn keyboard_shortcuts(&self) -> Vec<(KeyboardShortcut, String)> {
        vec![(
            KeyboardShortcut::new("Mod-Shift-B", "Toggle blockquote"),
            "toggle_blockquote".into(),
        )]
    }

    fn input_rules(&self) -> Vec<InputRule> {
        vec![InputRule::new(
            Regex::new(r"^> $").unwrap(),
            "Blockquote",
            |editor, caps| apply_block_input_rule(editor, caps, "blockquote"),
        )]
    }
}

/// Unordered list (`<ul>`).
#[derive(Debug)]
pub struct BulletListExt;

impl Extension for BulletListExt {
    fn name(&self) -> &str {
        "bullet_list"
    }

    fn nodes(&self) -> Vec<NodeSpec> {
        vec![
            NodeSpec::builder("bullet_list")
                .content("list_item+")
                .group("block")
                .parse_html(vec!["ul".into()])
                .build(),
        ]
    }

    fn commands(&self) -> Vec<CommandRegistration> {
        vec![CommandRegistration::new("toggle_bullet_list", |editor| {
            let current = editor.doc.block_type(0);
            if current.as_deref() == Some("bullet_list") {
                StructureCommands::lift(editor)
            } else {
                StructureCommands::wrap_in(editor, "bullet_list")
            }
        })]
    }

    fn keyboard_shortcuts(&self) -> Vec<(KeyboardShortcut, String)> {
        vec![(
            KeyboardShortcut::new("Mod-Shift-8", "Toggle bullet list"),
            "toggle_bullet_list".into(),
        )]
    }

    fn input_rules(&self) -> Vec<InputRule> {
        vec![
            InputRule::new(
                Regex::new(r"^- $").unwrap(),
                "Bullet list (dash)",
                |editor, caps| apply_block_input_rule(editor, caps, "bullet_list"),
            ),
            InputRule::new(
                Regex::new(r"^\* $").unwrap(),
                "Bullet list (asterisk)",
                |editor, caps| apply_block_input_rule(editor, caps, "bullet_list"),
            ),
        ]
    }
}

/// Ordered list (`<ol>`).
#[derive(Debug)]
pub struct OrderedListExt;

impl Extension for OrderedListExt {
    fn name(&self) -> &str {
        "ordered_list"
    }

    fn nodes(&self) -> Vec<NodeSpec> {
        vec![
            NodeSpec::builder("ordered_list")
                .content("list_item+")
                .group("block")
                .attr("start", AttrSpec::optional("1"))
                .parse_html(vec!["ol".into()])
                .build(),
        ]
    }

    fn commands(&self) -> Vec<CommandRegistration> {
        vec![CommandRegistration::new("toggle_ordered_list", |editor| {
            let current = editor.doc.block_type(0);
            if current.as_deref() == Some("ordered_list") {
                StructureCommands::lift(editor)
            } else {
                StructureCommands::wrap_in(editor, "ordered_list")
            }
        })]
    }

    fn keyboard_shortcuts(&self) -> Vec<(KeyboardShortcut, String)> {
        vec![(
            KeyboardShortcut::new("Mod-Shift-7", "Toggle ordered list"),
            "toggle_ordered_list".into(),
        )]
    }

    fn input_rules(&self) -> Vec<InputRule> {
        vec![InputRule::new(
            Regex::new(r"^\d+\. $").unwrap(),
            "Ordered list",
            |editor, caps| apply_block_input_rule(editor, caps, "ordered_list"),
        )]
    }
}

/// List item (`<li>`).
#[derive(Debug)]
pub struct ListItemExt;

impl Extension for ListItemExt {
    fn name(&self) -> &str {
        "list_item"
    }

    fn nodes(&self) -> Vec<NodeSpec> {
        vec![
            NodeSpec::builder("list_item")
                .content("block+")
                .parse_html(vec!["li".into()])
                .build(),
        ]
    }
}

/// Fenced code block (`<pre><code>`).
#[derive(Debug)]
pub struct CodeBlockExt;

impl Extension for CodeBlockExt {
    fn name(&self) -> &str {
        "code_block"
    }

    fn nodes(&self) -> Vec<NodeSpec> {
        vec![
            NodeSpec::builder("code_block")
                .content("text*")
                .group("block")
                .marks(MarkSet::None)
                .attr("language", AttrSpec::optional(""))
                .parse_html(vec!["pre".into()])
                .build(),
        ]
    }

    fn commands(&self) -> Vec<CommandRegistration> {
        vec![CommandRegistration::new("toggle_code_block", |editor| {
            let current = editor.doc.block_type(0);
            if current.as_deref() == Some("code_block") {
                StructureCommands::set_block_type(editor, "paragraph")
            } else {
                StructureCommands::set_block_type(editor, "code_block")
            }
        })]
    }

    fn keyboard_shortcuts(&self) -> Vec<(KeyboardShortcut, String)> {
        vec![(
            KeyboardShortcut::new("Mod-Alt-C", "Toggle code block"),
            "toggle_code_block".into(),
        )]
    }

    fn input_rules(&self) -> Vec<InputRule> {
        vec![InputRule::new(
            Regex::new(r"^``` $").unwrap(),
            "Code block",
            |editor, caps| apply_block_input_rule(editor, caps, "code_block"),
        )]
    }
}

/// Horizontal rule (`<hr>`).
#[derive(Debug)]
pub struct HorizontalRuleExt;

impl Extension for HorizontalRuleExt {
    fn name(&self) -> &str {
        "horizontal_rule"
    }

    fn nodes(&self) -> Vec<NodeSpec> {
        let mut spec = NodeSpec::atom("horizontal_rule");
        spec.group = Some("block".into());
        spec.parse_html_tags = vec!["hr".into()];
        vec![spec]
    }

    fn commands(&self) -> Vec<CommandRegistration> {
        vec![CommandRegistration::new("set_horizontal_rule", |editor| {
            StructureCommands::set_block_type(editor, "horizontal_rule")
        })]
    }

    fn input_rules(&self) -> Vec<InputRule> {
        vec![InputRule::new(
            Regex::new(r"^--- $").unwrap(),
            "Horizontal rule",
            |editor, caps| apply_block_input_rule(editor, caps, "horizontal_rule"),
        )]
    }
}

/// Hard break (`<br>`).
#[derive(Debug)]
pub struct HardBreakExt;

impl Extension for HardBreakExt {
    fn name(&self) -> &str {
        "hard_break"
    }

    fn nodes(&self) -> Vec<NodeSpec> {
        let mut spec = NodeSpec::atom("hard_break");
        spec.group = Some("inline".into());
        spec.inline = true;
        spec.parse_html_tags = vec!["br".into()];
        vec![spec]
    }

    fn commands(&self) -> Vec<CommandRegistration> {
        vec![CommandRegistration::new("insert_hard_break", |editor| {
            TextCommands::insert_text(editor, "\n")
        })]
    }

    fn keyboard_shortcuts(&self) -> Vec<(KeyboardShortcut, String)> {
        vec![(
            KeyboardShortcut::new("Shift-Enter", "Insert hard break"),
            "insert_hard_break".into(),
        )]
    }
}

/// Image node (atom, inline).
#[derive(Debug)]
pub struct ImageExt;

impl Extension for ImageExt {
    fn name(&self) -> &str {
        "image"
    }

    fn nodes(&self) -> Vec<NodeSpec> {
        let mut spec = NodeSpec::atom("image");
        spec.group = Some("inline".into());
        spec.inline = true;
        spec.attrs.insert("src".into(), AttrSpec::required());
        spec.attrs.insert("alt".into(), AttrSpec::optional(""));
        spec.attrs.insert("title".into(), AttrSpec::optional(""));
        spec.parse_html_tags = vec!["img".into()];
        vec![spec]
    }

    fn commands(&self) -> Vec<CommandRegistration> {
        vec![CommandRegistration::new("set_image", |_editor| {
            // Image insertion requires additional data (src, alt, title)
            // which would be provided through a dialog or programmatic API.
            Ok(())
        })]
    }
}

// =============================================================================
// Mark Extensions
// =============================================================================

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

// =============================================================================
// History Extension
// =============================================================================

/// History extension for undo/redo functionality.
#[derive(Debug)]
pub struct HistoryExt;

impl Extension for HistoryExt {
    fn name(&self) -> &str {
        "history"
    }

    fn commands(&self) -> Vec<CommandRegistration> {
        vec![
            CommandRegistration::new("undo", |editor| editor.undo().map(|_| ())),
            CommandRegistration::new("redo", |editor| editor.redo().map(|_| ())),
        ]
    }

    fn keyboard_shortcuts(&self) -> Vec<(KeyboardShortcut, String)> {
        vec![
            (KeyboardShortcut::new("Mod-Z", "Undo"), "undo".into()),
            (KeyboardShortcut::new("Mod-Shift-Z", "Redo"), "redo".into()),
        ]
    }
}

// =============================================================================
// StarterKit Bundle
// =============================================================================

/// StarterKit bundles all 24 default extensions.
///
/// This provides the same default editing experience as TipTap/Mantine's
/// StarterKit, including paragraph, headings, lists, blockquotes, code blocks,
/// and all standard formatting marks with keyboard shortcuts and markdown
/// input rules.
#[derive(Debug)]
pub struct StarterKit;

impl StarterKit {
    /// Get all 24 starter kit extensions as boxed trait objects.
    pub fn extensions() -> Vec<Box<dyn Extension>> {
        vec![
            // Node extensions
            Box::new(DocumentExt),
            Box::new(ParagraphExt),
            Box::new(TextExt),
            Box::new(HeadingExt),
            Box::new(BlockquoteExt),
            Box::new(BulletListExt),
            Box::new(OrderedListExt),
            Box::new(ListItemExt),
            Box::new(CodeBlockExt),
            Box::new(HorizontalRuleExt),
            Box::new(HardBreakExt),
            Box::new(ImageExt),
            // Mark extensions
            Box::new(BoldExt),
            Box::new(ItalicExt),
            Box::new(StrikeExt),
            Box::new(CodeExt),
            Box::new(UnderlineExt),
            Box::new(LinkExt),
            Box::new(HighlightExt),
            Box::new(SubscriptExt),
            Box::new(SuperscriptExt),
            Box::new(TextColorExt),
            // Block attribute extensions
            Box::new(TextAlignExt),
            // History extension
            Box::new(HistoryExt),
        ]
    }

    /// Get the number of extensions in the starter kit.
    pub fn count() -> usize {
        24
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- StarterKit bundle tests ----

    #[test]
    fn starter_kit_has_24_extensions() {
        let exts = StarterKit::extensions();
        assert_eq!(exts.len(), 24);
        assert_eq!(StarterKit::count(), 24);
    }

    #[test]
    fn starter_kit_all_names_unique() {
        let exts = StarterKit::extensions();
        let mut names: Vec<&str> = exts.iter().map(|e| e.name()).collect();
        let len_before = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), len_before, "Duplicate extension names found");
    }

    #[test]
    fn starter_kit_extension_names() {
        let exts = StarterKit::extensions();
        let names: Vec<&str> = exts.iter().map(|e| e.name()).collect();
        assert!(names.contains(&"document"));
        assert!(names.contains(&"paragraph"));
        assert!(names.contains(&"text"));
        assert!(names.contains(&"heading"));
        assert!(names.contains(&"blockquote"));
        assert!(names.contains(&"bullet_list"));
        assert!(names.contains(&"ordered_list"));
        assert!(names.contains(&"list_item"));
        assert!(names.contains(&"code_block"));
        assert!(names.contains(&"horizontal_rule"));
        assert!(names.contains(&"hard_break"));
        assert!(names.contains(&"image"));
        assert!(names.contains(&"bold"));
        assert!(names.contains(&"italic"));
        assert!(names.contains(&"strike"));
        assert!(names.contains(&"code"));
        assert!(names.contains(&"underline"));
        assert!(names.contains(&"link"));
        assert!(names.contains(&"highlight"));
        assert!(names.contains(&"subscript"));
        assert!(names.contains(&"superscript"));
        assert!(names.contains(&"text_color"));
        assert!(names.contains(&"text_align"));
        assert!(names.contains(&"history"));
    }

    // ---- Node extension tests ----

    #[test]
    fn document_ext_provides_doc_node() {
        let ext = DocumentExt;
        assert_eq!(ext.name(), "document");
        assert_eq!(ext.priority(), 0);
        let nodes = ext.nodes();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].name, "doc");
        assert_eq!(nodes[0].content, Some("block+".to_string()));
    }

    #[test]
    fn paragraph_ext_provides_node_and_command() {
        let ext = ParagraphExt;
        assert_eq!(ext.name(), "paragraph");
        let nodes = ext.nodes();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].name, "paragraph");
        assert_eq!(nodes[0].group, Some("block".to_string()));
        let cmds = ext.commands();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].name, "set_paragraph");
    }

    #[test]
    fn paragraph_ext_has_shortcut() {
        let ext = ParagraphExt;
        let shortcuts = ext.keyboard_shortcuts();
        assert_eq!(shortcuts.len(), 1);
        assert!(shortcuts[0].0.matches("Ctrl-Alt-0"));
        assert_eq!(shortcuts[0].1, "set_paragraph");
    }

    #[test]
    fn text_ext_is_inline() {
        let ext = TextExt;
        let nodes = ext.nodes();
        assert_eq!(nodes[0].name, "text");
        assert!(nodes[0].inline);
        assert_eq!(nodes[0].group, Some("inline".to_string()));
    }

    #[test]
    fn heading_ext_has_6_shortcuts() {
        let ext = HeadingExt;
        let shortcuts = ext.keyboard_shortcuts();
        assert_eq!(shortcuts.len(), 6);
        for (i, (shortcut, cmd)) in shortcuts.iter().enumerate() {
            let expected_key = format!("Ctrl+Alt+{}", i + 1);
            assert!(
                shortcut.matches(&expected_key),
                "Shortcut {} should match {}",
                shortcut.key,
                expected_key
            );
            assert_eq!(*cmd, format!("set_heading_{}", i + 1));
        }
    }

    #[test]
    fn heading_ext_has_input_rules() {
        let ext = HeadingExt;
        let rules = ext.input_rules();
        assert_eq!(rules.len(), 1);
        // Single combined rule matches all heading levels
        assert!(rules[0].matches("# ").is_some());
        assert!(rules[0].matches("## ").is_some());
        assert!(rules[0].matches("###### ").is_some());
        assert!(rules[0].matches("####### ").is_none()); // 7 hashes = no match
    }

    #[test]
    fn heading_ext_has_level_attr() {
        let ext = HeadingExt;
        let nodes = ext.nodes();
        assert!(nodes[0].attrs.contains_key("level"));
    }

    #[test]
    fn blockquote_ext_has_input_rule() {
        let ext = BlockquoteExt;
        let rules = ext.input_rules();
        assert_eq!(rules.len(), 1);
        assert!(rules[0].matches("> ").is_some());
        assert!(rules[0].matches("hello").is_none());
    }

    #[test]
    fn bullet_list_ext_has_two_input_rules() {
        let ext = BulletListExt;
        let rules = ext.input_rules();
        assert_eq!(rules.len(), 2);
        assert!(rules[0].matches("- ").is_some());
        assert!(rules[1].matches("* ").is_some());
    }

    #[test]
    fn ordered_list_ext_input_rule() {
        let ext = OrderedListExt;
        let rules = ext.input_rules();
        assert_eq!(rules.len(), 1);
        assert!(rules[0].matches("1. ").is_some());
        assert!(rules[0].matches("42. ").is_some());
        assert!(rules[0].matches("hello").is_none());
    }

    #[test]
    fn list_item_ext_minimal() {
        let ext = ListItemExt;
        assert_eq!(ext.name(), "list_item");
        assert!(ext.commands().is_empty());
        assert!(ext.keyboard_shortcuts().is_empty());
        let nodes = ext.nodes();
        assert_eq!(nodes[0].content, Some("block+".to_string()));
    }

    #[test]
    fn code_block_ext_disallows_marks() {
        let ext = CodeBlockExt;
        let nodes = ext.nodes();
        assert!(matches!(nodes[0].marks, MarkSet::None));
    }

    #[test]
    fn code_block_ext_input_rule() {
        let ext = CodeBlockExt;
        let rules = ext.input_rules();
        assert_eq!(rules.len(), 1);
        assert!(rules[0].matches("``` ").is_some());
    }

    #[test]
    fn horizontal_rule_is_atom() {
        let ext = HorizontalRuleExt;
        let nodes = ext.nodes();
        assert!(nodes[0].atom);
        assert_eq!(nodes[0].group, Some("block".to_string()));
    }

    #[test]
    fn horizontal_rule_input_rule() {
        let ext = HorizontalRuleExt;
        let rules = ext.input_rules();
        assert!(rules[0].matches("--- ").is_some());
        assert!(rules[0].matches("-- ").is_none());
    }

    #[test]
    fn hard_break_is_inline_atom() {
        let ext = HardBreakExt;
        let nodes = ext.nodes();
        assert!(nodes[0].atom);
        assert!(nodes[0].inline);
    }

    #[test]
    fn hard_break_shortcut() {
        let ext = HardBreakExt;
        let shortcuts = ext.keyboard_shortcuts();
        assert_eq!(shortcuts.len(), 1);
        assert!(shortcuts[0].0.matches("Shift-Enter"));
        assert_eq!(shortcuts[0].1, "insert_hard_break");
    }

    #[test]
    fn image_ext_has_src_attr() {
        let ext = ImageExt;
        let nodes = ext.nodes();
        assert!(nodes[0].attrs["src"].required);
        assert!(!nodes[0].attrs["alt"].required);
    }

    // ---- Mark extension tests ----

    #[test]
    fn bold_ext_shortcut_and_command() {
        let ext = BoldExt;
        let marks = ext.marks();
        assert_eq!(marks[0].name, "bold");
        let cmds = ext.commands();
        assert_eq!(cmds[0].name, "toggle_bold");
        let shortcuts = ext.keyboard_shortcuts();
        assert!(shortcuts[0].0.matches("Ctrl-B"));
    }

    #[test]
    fn bold_ext_input_rule() {
        let ext = BoldExt;
        let rules = ext.input_rules();
        assert_eq!(rules.len(), 1);
        assert!(rules[0].matches("**hello**").is_some());
        assert!(rules[0].matches("hello").is_none());
    }

    #[test]
    fn italic_ext_shortcut() {
        let ext = ItalicExt;
        let shortcuts = ext.keyboard_shortcuts();
        assert!(shortcuts[0].0.matches("Ctrl-I"));
    }

    #[test]
    fn italic_ext_input_rule() {
        let ext = ItalicExt;
        let rules = ext.input_rules();
        assert!(rules[0].matches("*hello*").is_some());
    }

    #[test]
    fn strike_ext_input_rule() {
        let ext = StrikeExt;
        let rules = ext.input_rules();
        assert!(rules[0].matches("~~hello~~").is_some());
    }

    #[test]
    fn code_mark_excludes_formatting() {
        let ext = CodeExt;
        let marks = ext.marks();
        assert!(marks[0].excludes_mark("bold"));
        assert!(marks[0].excludes_mark("italic"));
        assert!(!marks[0].excludes_mark("highlight"));
    }

    #[test]
    fn underline_ext_shortcut() {
        let ext = UnderlineExt;
        let shortcuts = ext.keyboard_shortcuts();
        assert!(shortcuts[0].0.matches("Ctrl-U"));
    }

    #[test]
    fn link_ext_has_href_attr() {
        let ext = LinkExt;
        let marks = ext.marks();
        assert!(marks[0].attrs["href"].required);
        assert!(!marks[0].attrs["title"].required);
    }

    #[test]
    fn link_ext_no_shortcut() {
        let ext = LinkExt;
        assert!(ext.keyboard_shortcuts().is_empty());
    }

    #[test]
    fn highlight_ext_shortcut() {
        let ext = HighlightExt;
        let shortcuts = ext.keyboard_shortcuts();
        assert!(shortcuts[0].0.matches("Ctrl-Shift-H"));
    }

    #[test]
    fn subscript_superscript_mutual_exclusion() {
        let sub_ext = SubscriptExt;
        let sup_ext = SuperscriptExt;
        let sub_marks = sub_ext.marks();
        let sup_marks = sup_ext.marks();
        assert!(sub_marks[0].excludes_mark("superscript"));
        assert!(sup_marks[0].excludes_mark("subscript"));
    }

    #[test]
    fn text_color_ext_no_shortcut() {
        let ext = TextColorExt;
        assert!(ext.keyboard_shortcuts().is_empty());
        assert!(ext.input_rules().is_empty());
    }

    #[test]
    fn text_color_has_color_attr() {
        let ext = TextColorExt;
        let marks = ext.marks();
        assert!(marks[0].attrs["color"].required);
    }

    #[test]
    fn history_ext_has_undo_redo_commands() {
        let ext = HistoryExt;
        assert_eq!(ext.name(), "history");
        let cmds = ext.commands();
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].name, "undo");
        assert_eq!(cmds[1].name, "redo");
    }

    #[test]
    fn history_ext_shortcuts() {
        let ext = HistoryExt;
        let shortcuts = ext.keyboard_shortcuts();
        assert_eq!(shortcuts.len(), 2);
        assert!(shortcuts[0].0.matches("Ctrl-Z"));
        assert_eq!(shortcuts[0].1, "undo");
        assert!(shortcuts[1].0.matches("Ctrl-Shift-Z"));
        assert_eq!(shortcuts[1].1, "redo");
    }

    #[test]
    fn history_ext_no_nodes_or_marks() {
        let ext = HistoryExt;
        assert!(ext.nodes().is_empty());
        assert!(ext.marks().is_empty());
    }

    // ---- Aggregate tests ----

    #[test]
    fn all_node_extensions_provide_nodes() {
        let node_exts: Vec<Box<dyn Extension>> = vec![
            Box::new(DocumentExt),
            Box::new(ParagraphExt),
            Box::new(TextExt),
            Box::new(HeadingExt),
            Box::new(BlockquoteExt),
            Box::new(BulletListExt),
            Box::new(OrderedListExt),
            Box::new(ListItemExt),
            Box::new(CodeBlockExt),
            Box::new(HorizontalRuleExt),
            Box::new(HardBreakExt),
            Box::new(ImageExt),
        ];
        for ext in &node_exts {
            assert!(
                !ext.nodes().is_empty(),
                "{} should provide nodes",
                ext.name()
            );
            assert!(
                ext.marks().is_empty(),
                "{} should not provide marks",
                ext.name()
            );
        }
    }

    #[test]
    fn all_mark_extensions_provide_marks() {
        let mark_exts: Vec<Box<dyn Extension>> = vec![
            Box::new(BoldExt),
            Box::new(ItalicExt),
            Box::new(StrikeExt),
            Box::new(CodeExt),
            Box::new(UnderlineExt),
            Box::new(LinkExt),
            Box::new(HighlightExt),
            Box::new(SubscriptExt),
            Box::new(SuperscriptExt),
            Box::new(TextColorExt),
        ];
        for ext in &mark_exts {
            assert!(
                !ext.marks().is_empty(),
                "{} should provide marks",
                ext.name()
            );
            assert!(
                ext.nodes().is_empty(),
                "{} should not provide nodes",
                ext.name()
            );
        }
    }

    #[test]
    fn total_commands_count() {
        let exts = StarterKit::extensions();
        let total: usize = exts.iter().map(|e| e.commands().len()).sum();
        // At minimum, each mark ext has 1+ command, most node exts have 1+ command
        assert!(total >= 20, "Expected at least 20 commands, got {}", total);
    }

    #[test]
    fn total_shortcuts_count() {
        let exts = StarterKit::extensions();
        let total: usize = exts.iter().map(|e| e.keyboard_shortcuts().len()).sum();
        // Mod-B, Mod-I, Mod-Shift-X, Mod-E, Mod-U, Mod-Shift-H, Mod-,, Mod-.,
        // Shift-Enter, Mod-Alt-0..6, Mod-Shift-B, Mod-Shift-8, Mod-Shift-7, Mod-Alt-C
        assert!(total >= 16, "Expected at least 16 shortcuts, got {}", total);
    }

    #[test]
    fn total_input_rules_count() {
        let exts = StarterKit::extensions();
        let total: usize = exts.iter().map(|e| e.input_rules().len()).sum();
        // headings (1 combined) + blockquote (1) + bullet_list (2) + ordered_list (1) +
        // code_block (1) + hr (1) + bold (1) + italic (1) + strike (1) = 10
        assert!(
            total >= 9,
            "Expected at least 9 input rules, got {}",
            total
        );
    }

    #[test]
    fn text_align_ext_has_four_commands() {
        let ext = TextAlignExt;
        let cmds = ext.commands();
        assert_eq!(cmds.len(), 4);
        assert_eq!(cmds[0].name, "align_left");
        assert_eq!(cmds[1].name, "align_center");
        assert_eq!(cmds[2].name, "align_right");
        assert_eq!(cmds[3].name, "align_justify");
    }

    #[test]
    fn text_align_ext_has_four_shortcuts() {
        let ext = TextAlignExt;
        let shortcuts = ext.keyboard_shortcuts();
        assert_eq!(shortcuts.len(), 4);
        assert!(shortcuts[0].0.matches("Ctrl-Shift-L"));
        assert_eq!(shortcuts[0].1, "align_left");
        assert!(shortcuts[1].0.matches("Ctrl-Shift-E"));
        assert_eq!(shortcuts[1].1, "align_center");
        assert!(shortcuts[2].0.matches("Ctrl-Shift-R"));
        assert_eq!(shortcuts[2].1, "align_right");
        assert!(shortcuts[3].0.matches("Ctrl-Shift-J"));
        assert_eq!(shortcuts[3].1, "align_justify");
    }

    #[test]
    fn text_align_ext_no_nodes_or_marks() {
        let ext = TextAlignExt;
        assert!(ext.nodes().is_empty());
        assert!(ext.marks().is_empty());
    }
}
