//! Comprehensive round-trip tests for EditorDocument ↔ BlockData.
//!
//! These tests simulate realistic user editing scenarios and verify that
//! content survives the full persistence cycle:
//!   DOM → extract_content → BlockData → from_block_data → EditorDocument
//!       → to_bytes → from_bytes → to_block_data → BlockData → load_content → DOM
//!
//! Each test builds BlockData (simulating what extract_content produces from the DOM),
//! round-trips it through EditorDocument, and asserts the output matches the input.

use std::collections::HashMap;

use rinch_core::ce::{BlockData, InlineMarkData, InlineRunData};

use super::{EditorDocument, MarkData};
use crate::document::Position;

// ============================================================================
// Helpers
// ============================================================================

/// Create a plain text run (no marks).
fn plain(text: &str) -> InlineRunData {
    InlineRunData {
        text: text.to_string(),
        marks: vec![],
    }
}

/// Create a text run with a single mark.
fn marked(text: &str, mark_type: &str) -> InlineRunData {
    InlineRunData {
        text: text.to_string(),
        marks: vec![InlineMarkData {
            mark_type: mark_type.to_string(),
            attrs: HashMap::new(),
        }],
    }
}

/// Create a text run with multiple marks.
fn multi_marked(text: &str, mark_types: &[&str]) -> InlineRunData {
    InlineRunData {
        text: text.to_string(),
        marks: mark_types
            .iter()
            .map(|mt| InlineMarkData {
                mark_type: mt.to_string(),
                attrs: HashMap::new(),
            })
            .collect(),
    }
}

/// Create a paragraph block.
fn para(runs: Vec<InlineRunData>) -> BlockData {
    BlockData {
        block_type: "paragraph".to_string(),
        attrs: HashMap::new(),
        content: runs,
    }
}

/// Create a heading block.
fn heading(level: &str, runs: Vec<InlineRunData>) -> BlockData {
    let mut attrs = HashMap::new();
    attrs.insert("level".to_string(), level.to_string());
    BlockData {
        block_type: "heading".to_string(),
        attrs,
        content: runs,
    }
}

/// Create a blockquote block.
fn blockquote(runs: Vec<InlineRunData>) -> BlockData {
    BlockData {
        block_type: "blockquote".to_string(),
        attrs: HashMap::new(),
        content: runs,
    }
}

/// Create a bullet list item block.
fn bullet(runs: Vec<InlineRunData>) -> BlockData {
    BlockData {
        block_type: "bullet_list".to_string(),
        attrs: HashMap::new(),
        content: runs,
    }
}

/// Create an ordered list item block.
fn ordered(runs: Vec<InlineRunData>) -> BlockData {
    BlockData {
        block_type: "ordered_list".to_string(),
        attrs: HashMap::new(),
        content: runs,
    }
}

/// Assert that two sets of blocks are equivalent (ignoring empty-text runs).
fn assert_blocks_eq(actual: &[BlockData], expected: &[BlockData]) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "block count mismatch: got {} blocks, expected {}\nactual: {:#?}",
        actual.len(),
        expected.len(),
        actual,
    );
    for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        assert_eq!(
            a.block_type, e.block_type,
            "block {i}: type mismatch: got {:?}, expected {:?}",
            a.block_type, e.block_type,
        );
        // Filter out empty runs for comparison (they're normalized away)
        let a_runs: Vec<_> = a.content.iter().filter(|r| !r.text.is_empty()).collect();
        let e_runs: Vec<_> = e.content.iter().filter(|r| !r.text.is_empty()).collect();
        assert_eq!(
            a_runs.len(),
            e_runs.len(),
            "block {i}: run count mismatch: got {} runs, expected {}\nactual runs: {:#?}\nexpected runs: {:#?}",
            a_runs.len(),
            e_runs.len(),
            a_runs,
            e_runs,
        );
        for (j, (ar, er)) in a_runs.iter().zip(e_runs.iter()).enumerate() {
            assert_eq!(
                ar.text, er.text,
                "block {i}, run {j}: text mismatch: got {:?}, expected {:?}",
                ar.text, er.text,
            );
            let a_marks: Vec<&str> = ar.marks.iter().map(|m| m.mark_type.as_str()).collect();
            let e_marks: Vec<&str> = er.marks.iter().map(|m| m.mark_type.as_str()).collect();
            assert_eq!(
                a_marks, e_marks,
                "block {i}, run {j}: marks mismatch: got {:?}, expected {:?}",
                a_marks, e_marks,
            );
        }
    }
}

/// Round-trip through from_block_data → to_block_data and assert equivalence.
fn roundtrip(blocks: &[BlockData]) -> Vec<BlockData> {
    let doc = EditorDocument::from_block_data(blocks);
    doc.to_block_data()
}

/// Full round-trip through bytes: from_block_data → to_bytes → from_bytes → to_block_data.
fn roundtrip_via_bytes(blocks: &[BlockData]) -> Vec<BlockData> {
    let mut doc = EditorDocument::from_block_data(blocks);
    let bytes = doc.to_bytes();
    let doc2 = EditorDocument::from_bytes(&bytes).unwrap();
    doc2.to_block_data()
}

// ============================================================================
// Basic content round-trips
// ============================================================================

#[test]
fn single_plain_paragraph() {
    let input = vec![para(vec![plain("Hello, world!")])];
    assert_blocks_eq(&roundtrip(&input), &input);
    assert_blocks_eq(&roundtrip_via_bytes(&input), &input);
}

#[test]
fn empty_paragraph() {
    let input = vec![para(vec![plain("")])];
    let result = roundtrip(&input);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].block_type, "paragraph");
    // Empty text is fine — just shouldn't crash or create extra blocks
}

#[test]
fn multiple_plain_paragraphs() {
    let input = vec![
        para(vec![plain("First paragraph")]),
        para(vec![plain("Second paragraph")]),
        para(vec![plain("Third paragraph")]),
    ];
    assert_blocks_eq(&roundtrip(&input), &input);
    assert_blocks_eq(&roundtrip_via_bytes(&input), &input);
}

#[test]
fn paragraph_with_trailing_empty_paragraph() {
    // Simulates pressing Enter at the end of a line
    let input = vec![para(vec![plain("Some text")]), para(vec![plain("")])];
    let result = roundtrip(&input);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].block_type, "paragraph");
}

// ============================================================================
// Mark preservation
// ============================================================================

#[test]
fn single_bold_run() {
    let input = vec![para(vec![marked("bold text", "bold")])];
    assert_blocks_eq(&roundtrip(&input), &input);
    assert_blocks_eq(&roundtrip_via_bytes(&input), &input);
}

#[test]
fn plain_bold_plain_sequence() {
    let input = vec![para(vec![
        plain("before "),
        marked("bold", "bold"),
        plain(" after"),
    ])];
    assert_blocks_eq(&roundtrip(&input), &input);
    assert_blocks_eq(&roundtrip_via_bytes(&input), &input);
}

#[test]
fn bold_italic_sequence() {
    let input = vec![para(vec![
        marked("bold part", "bold"),
        marked("italic part", "italic"),
    ])];
    assert_blocks_eq(&roundtrip(&input), &input);
    assert_blocks_eq(&roundtrip_via_bytes(&input), &input);
}

#[test]
fn plain_bold_italic_plain() {
    let input = vec![para(vec![
        plain("start "),
        marked("bold", "bold"),
        plain(" middle "),
        marked("italic", "italic"),
        plain(" end"),
    ])];
    assert_blocks_eq(&roundtrip(&input), &input);
    assert_blocks_eq(&roundtrip_via_bytes(&input), &input);
}

#[test]
fn nested_marks_bold_italic() {
    let input = vec![para(vec![
        plain("normal "),
        multi_marked("bold+italic", &["bold", "italic"]),
        plain(" normal"),
    ])];
    assert_blocks_eq(&roundtrip(&input), &input);
    assert_blocks_eq(&roundtrip_via_bytes(&input), &input);
}

#[test]
fn all_mark_types() {
    let input = vec![para(vec![
        marked("bold", "bold"),
        marked("italic", "italic"),
        marked("underline", "underline"),
        marked("strike", "strikethrough"),
        marked("code", "code"),
    ])];
    assert_blocks_eq(&roundtrip(&input), &input);
    assert_blocks_eq(&roundtrip_via_bytes(&input), &input);
}

#[test]
fn adjacent_same_marks_merge_is_ok() {
    // Two adjacent bold runs may merge into one — that's fine (normalization)
    let input = vec![para(vec![
        marked("hello ", "bold"),
        marked("world", "bold"),
    ])];
    let result = roundtrip(&input);
    assert_eq!(result.len(), 1);
    // May be 1 or 2 runs, but total text and marks must be correct
    let total_text: String = result[0].content.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(total_text, "hello world");
    for run in &result[0].content {
        if !run.text.is_empty() {
            assert!(
                run.marks.iter().any(|m| m.mark_type == "bold"),
                "all runs should be bold"
            );
        }
    }
}

// ============================================================================
// Block types
// ============================================================================

#[test]
fn heading_blocks() {
    let input = vec![
        heading("1", vec![plain("Title")]),
        heading("2", vec![plain("Subtitle")]),
        heading("3", vec![plain("Section")]),
        para(vec![plain("Body text")]),
    ];
    assert_blocks_eq(&roundtrip(&input), &input);
    assert_blocks_eq(&roundtrip_via_bytes(&input), &input);
}

#[test]
fn blockquote_block() {
    let input = vec![
        para(vec![plain("Before quote")]),
        blockquote(vec![plain("Quoted text")]),
        para(vec![plain("After quote")]),
    ];
    assert_blocks_eq(&roundtrip(&input), &input);
    assert_blocks_eq(&roundtrip_via_bytes(&input), &input);
}

#[test]
fn bullet_list_items() {
    let input = vec![
        bullet(vec![plain("Item one")]),
        bullet(vec![plain("Item two")]),
        bullet(vec![plain("Item three")]),
    ];
    assert_blocks_eq(&roundtrip(&input), &input);
    assert_blocks_eq(&roundtrip_via_bytes(&input), &input);
}

#[test]
fn ordered_list_items() {
    let input = vec![
        ordered(vec![plain("First")]),
        ordered(vec![plain("Second")]),
        ordered(vec![plain("Third")]),
    ];
    assert_blocks_eq(&roundtrip(&input), &input);
    assert_blocks_eq(&roundtrip_via_bytes(&input), &input);
}

#[test]
fn mixed_block_types() {
    let input = vec![
        heading("1", vec![plain("Document Title")]),
        para(vec![plain("Introduction paragraph.")]),
        heading("2", vec![plain("Section One")]),
        para(vec![
            plain("Some "),
            marked("bold", "bold"),
            plain(" text."),
        ]),
        blockquote(vec![plain("A notable quote.")]),
        bullet(vec![plain("Point A")]),
        bullet(vec![plain("Point B")]),
        para(vec![plain("Conclusion.")]),
    ];
    assert_blocks_eq(&roundtrip(&input), &input);
    assert_blocks_eq(&roundtrip_via_bytes(&input), &input);
}

// ============================================================================
// Simulated editing sequences
// ============================================================================

#[test]
fn simulate_type_then_bold_then_type() {
    // User types "Hello ", toggles bold, types "world", toggles bold off, types "!"
    let input = vec![para(vec![
        plain("Hello "),
        marked("world", "bold"),
        plain("!"),
    ])];
    assert_blocks_eq(&roundtrip(&input), &input);
    assert_blocks_eq(&roundtrip_via_bytes(&input), &input);
}

#[test]
fn simulate_multiple_enters() {
    // User types text, presses Enter several times
    let input = vec![
        para(vec![plain("Line 1")]),
        para(vec![plain("Line 2")]),
        para(vec![plain("")]),
        para(vec![plain("Line 4 after blank")]),
    ];
    let result = roundtrip(&input);
    assert_eq!(result.len(), 4, "should preserve 4 blocks: {:#?}", result);
    assert_blocks_eq(&roundtrip_via_bytes(&input), &input);
}

#[test]
fn simulate_formatting_across_save_load_cycles() {
    // Simulate: type, format, save, load, type more, save, load
    let initial = vec![para(vec![
        plain("Hello "),
        marked("beautiful", "italic"),
        plain(" world"),
    ])];

    // First save/load cycle
    let mut doc1 = EditorDocument::from_block_data(&initial);
    let bytes1 = doc1.to_bytes();
    let doc2 = EditorDocument::from_bytes(&bytes1).unwrap();
    let blocks_after_1 = doc2.to_block_data();
    assert_blocks_eq(&blocks_after_1, &initial);

    // Simulate user adding more text: append " and universe" to the end
    // This is what happens after load_content + user edits + extract_content
    let edited = vec![para(vec![
        plain("Hello "),
        marked("beautiful", "italic"),
        plain(" world and universe"),
    ])];

    // Second save/load cycle
    let mut doc3 = EditorDocument::from_block_data(&edited);
    let bytes2 = doc3.to_bytes();
    let doc4 = EditorDocument::from_bytes(&bytes2).unwrap();
    let blocks_after_2 = doc4.to_block_data();
    assert_blocks_eq(&blocks_after_2, &edited);
}

#[test]
fn simulate_split_block_in_middle_of_formatted_text() {
    // User has "Hello **world**" and presses Enter after "wor"
    // Result: ["Hello **wor**", "**ld**"]
    let input = vec![
        para(vec![plain("Hello "), marked("wor", "bold")]),
        para(vec![marked("ld", "bold")]),
    ];
    assert_blocks_eq(&roundtrip(&input), &input);
    assert_blocks_eq(&roundtrip_via_bytes(&input), &input);
}

// ============================================================================
// Unicode / multi-byte content
// ============================================================================

#[test]
fn unicode_emoji() {
    let input = vec![para(vec![plain("Hello 🌍 world 🎉!")])];
    assert_blocks_eq(&roundtrip(&input), &input);
    assert_blocks_eq(&roundtrip_via_bytes(&input), &input);
}

#[test]
fn unicode_accented_chars() {
    let input = vec![para(vec![plain("café résumé naïve")])];
    assert_blocks_eq(&roundtrip(&input), &input);
    assert_blocks_eq(&roundtrip_via_bytes(&input), &input);
}

#[test]
fn unicode_cjk() {
    let input = vec![para(vec![plain("你好世界")])];
    assert_blocks_eq(&roundtrip(&input), &input);
    assert_blocks_eq(&roundtrip_via_bytes(&input), &input);
}

#[test]
fn unicode_mixed_with_marks() {
    let input = vec![para(vec![
        plain("café "),
        marked("résumé", "bold"),
        plain(" 🎉"),
    ])];
    assert_blocks_eq(&roundtrip(&input), &input);
    assert_blocks_eq(&roundtrip_via_bytes(&input), &input);
}

#[test]
fn emoji_between_formatted_runs() {
    let input = vec![para(vec![
        marked("bold", "bold"),
        plain("🎉"),
        marked("italic", "italic"),
    ])];
    assert_blocks_eq(&roundtrip(&input), &input);
    assert_blocks_eq(&roundtrip_via_bytes(&input), &input);
}

// ============================================================================
// Edge cases
// ============================================================================

#[test]
fn single_character_runs() {
    let input = vec![para(vec![
        plain("a"),
        marked("b", "bold"),
        plain("c"),
        marked("d", "italic"),
        plain("e"),
    ])];
    assert_blocks_eq(&roundtrip(&input), &input);
    assert_blocks_eq(&roundtrip_via_bytes(&input), &input);
}

#[test]
fn long_document() {
    let mut blocks = Vec::new();
    blocks.push(heading("1", vec![plain("Long Document")]));
    for i in 0..20 {
        blocks.push(para(vec![
            plain(&format!("Paragraph {} with ", i)),
            marked(&format!("bold part {}", i), "bold"),
            plain(" and normal."),
        ]));
    }
    assert_blocks_eq(&roundtrip(&blocks), &blocks);
    assert_blocks_eq(&roundtrip_via_bytes(&blocks), &blocks);
}

#[test]
fn marks_with_attrs() {
    let input = vec![para(vec![InlineRunData {
        text: "linked text".to_string(),
        marks: vec![InlineMarkData {
            mark_type: "link".to_string(),
            attrs: {
                let mut m = HashMap::new();
                m.insert("href".to_string(), "https://example.com".to_string());
                m
            },
        }],
    }])];
    let result = roundtrip(&input);
    assert_eq!(result.len(), 1);
    let runs: Vec<_> = result[0]
        .content
        .iter()
        .filter(|r| !r.text.is_empty())
        .collect();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].text, "linked text");
    assert_eq!(runs[0].marks[0].mark_type, "link");
    assert_eq!(
        runs[0].marks[0].attrs.get("href").map(|s| s.as_str()),
        Some("https://example.com")
    );
}

#[test]
fn special_characters_in_text() {
    let input = vec![para(vec![plain(
        r#"Angle <brackets>, "quotes", 'apostrophes', &amp; entities"#,
    )])];
    assert_blocks_eq(&roundtrip(&input), &input);
    assert_blocks_eq(&roundtrip_via_bytes(&input), &input);
}

#[test]
fn whitespace_preservation() {
    let input = vec![para(vec![plain("  leading and trailing  ")])];
    assert_blocks_eq(&roundtrip(&input), &input);
    assert_blocks_eq(&roundtrip_via_bytes(&input), &input);
}

// ============================================================================
// EditorDocument mutation → round-trip
// ============================================================================

#[test]
fn insert_text_then_roundtrip() {
    let mut doc = EditorDocument::new();
    doc.insert_text(Position::new(0), "Hello World").unwrap();
    let bytes = doc.to_bytes();

    let doc2 = EditorDocument::from_bytes(&bytes).unwrap();
    assert_eq!(doc2.block_count(), 1);
    assert_eq!(doc2.block_text(0), Some("Hello World".into()));

    // Round-trip through BlockData
    let blocks = doc2.to_block_data();
    let doc3 = EditorDocument::from_block_data(&blocks);
    assert_eq!(doc3.block_count(), 1);
    assert_eq!(doc3.block_text(0), Some("Hello World".into()));
}

#[test]
fn insert_text_with_marks_then_roundtrip() {
    let mut doc = EditorDocument::new();
    doc.insert_text(Position::new(0), "Hello ").unwrap();
    doc.insert_text_with_marks(Position::new(6), "bold", &[MarkData::new("bold")])
        .unwrap();
    doc.insert_text_with_marks(Position::new(10), " world", &[])
        .unwrap();

    let blocks = doc.to_block_data();
    assert_eq!(blocks.len(), 1);
    let runs: Vec<_> = blocks[0]
        .content
        .iter()
        .filter(|r| !r.text.is_empty())
        .collect();
    assert_eq!(runs.len(), 3, "runs: {:#?}", runs);
    assert_eq!(runs[0].text, "Hello ");
    assert!(runs[0].marks.is_empty());
    assert_eq!(runs[1].text, "bold");
    assert!(runs[1].marks.iter().any(|m| m.mark_type == "bold"));
    assert_eq!(runs[2].text, " world");
    assert!(runs[2].marks.is_empty());

    // Full bytes round-trip
    assert_blocks_eq(&roundtrip_via_bytes(&blocks), &blocks);
}

#[test]
fn split_block_then_roundtrip() {
    let mut doc = EditorDocument::new();
    doc.insert_text(Position::new(0), "HelloWorld").unwrap();
    doc.split_block(Position::new(5)).unwrap();

    assert_eq!(doc.block_count(), 2);
    assert_eq!(doc.block_text(0), Some("Hello".into()));
    assert_eq!(doc.block_text(1), Some("World".into()));

    let blocks = doc.to_block_data();
    assert_blocks_eq(
        &roundtrip(&blocks),
        &[para(vec![plain("Hello")]), para(vec![plain("World")])],
    );
    assert_blocks_eq(&roundtrip_via_bytes(&blocks), &blocks);
}

#[test]
fn delete_range_then_roundtrip() {
    let mut doc = EditorDocument::new();
    doc.insert_text(Position::new(0), "Hello Beautiful World")
        .unwrap();
    // Delete " Beautiful" (positions 5..15)
    doc.delete_range(crate::document::Range::new(5usize, 15usize))
        .unwrap();

    assert_eq!(doc.block_text(0), Some("Hello World".into()));

    let blocks = doc.to_block_data();
    assert_blocks_eq(&roundtrip_via_bytes(&blocks), &blocks);
}

#[test]
fn add_mark_then_roundtrip() {
    let mut doc = EditorDocument::new();
    doc.insert_text(Position::new(0), "Hello World").unwrap();
    doc.add_mark(
        crate::document::Range::new(6usize, 11usize),
        MarkData::new("bold"),
    )
    .unwrap();

    let blocks = doc.to_block_data();
    let runs: Vec<_> = blocks[0]
        .content
        .iter()
        .filter(|r| !r.text.is_empty())
        .collect();
    assert_eq!(runs.len(), 2, "runs: {:#?}", runs);
    assert_eq!(runs[0].text, "Hello ");
    assert!(runs[0].marks.is_empty());
    assert_eq!(runs[1].text, "World");
    assert!(runs[1].marks.iter().any(|m| m.mark_type == "bold"));

    assert_blocks_eq(&roundtrip_via_bytes(&blocks), &blocks);
}

// ============================================================================
// Repeated round-trip stability (idempotency)
// ============================================================================

#[test]
fn triple_roundtrip_stability() {
    // Content should not degrade over multiple save/load cycles
    let input = vec![
        heading("1", vec![plain("Title")]),
        para(vec![
            plain("Normal "),
            marked("bold", "bold"),
            plain(" "),
            marked("italic", "italic"),
            plain(" end."),
        ]),
        blockquote(vec![marked("quoted bold", "bold")]),
        bullet(vec![plain("item 1")]),
        bullet(vec![plain("item 2 with "), marked("emphasis", "italic")]),
    ];

    let r1 = roundtrip_via_bytes(&input);
    assert_blocks_eq(&r1, &input);

    let r2 = roundtrip_via_bytes(&r1);
    assert_blocks_eq(&r2, &input);

    let r3 = roundtrip_via_bytes(&r2);
    assert_blocks_eq(&r3, &input);
}
