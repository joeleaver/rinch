//! StarterKit: 23 default extensions for a full-featured rich-text editor.
//!
//! Modeled after TipTap/Mantine's StarterKit, this module provides all the
//! standard node and mark extensions with keyboard shortcuts, commands,
//! and markdown input rules.

mod history;
mod mark_extensions;
mod node_extensions;

pub use history::HistoryExt;
pub use mark_extensions::{
    BoldExt, CodeExt, HighlightExt, ItalicExt, LinkExt, StrikeExt, SubscriptExt, SuperscriptExt,
    TextAlignExt, TextColorExt, UnderlineExt,
};
pub use node_extensions::{
    BlockquoteExt, BulletListExt, CodeBlockExt, DocumentExt, HardBreakExt, HeadingExt,
    HorizontalRuleExt, ImageExt, ListItemExt, OrderedListExt, ParagraphExt, TextExt,
};

use super::Extension;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::MarkSet;

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

    // TODO: heading input rules not yet implemented
    // #[test]
    // fn heading_ext_has_input_rules() { ... }

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
        assert!(rules[0].matches("--").is_none());
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
        // blockquote (1) + bullet_list (2) + ordered_list (1) +
        // code_block (1) + hr (1) + bold (1) + italic (1) + strike (1) = 9
        // (heading input rules not yet implemented)
        assert!(total >= 8, "Expected at least 8 input rules, got {}", total);
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
