//! Node extensions: DocumentExt, ParagraphExt, TextExt, HeadingExt,
//! BlockquoteExt, BulletListExt, OrderedListExt, ListItemExt,
//! CodeBlockExt, HorizontalRuleExt, HardBreakExt, ImageExt.

use regex::Regex;
use std::collections::HashMap;

use crate::commands::{StructureCommands, TextCommands};
use crate::document::Range;
use crate::editor::Editor;
use crate::error::EditorError;
use crate::extensions::{CommandRegistration, Extension};
use crate::input::{InputRule, KeyboardShortcut};
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
