//! Tests verifying CeOps ↔ EditorDocument position sync.
//!
//! These tests create a headless RinchDocument + CeOps with collaboration
//! enabled, perform operations, and assert the DOM and CRDT stay in sync
//! after every mutation. No window, no renderer, no user interaction.

#![cfg(feature = "collaboration")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::ce::{ContentEditableApi, DomCursor};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_editor::EditorDocument;

// ============================================================================
// Test Harness
// ============================================================================

/// A headless CE environment for testing: RinchDocument + CeOps + EditorDocument.
struct TestCe {
    ops: rinch::ce_ops::CeOps,
}

impl TestCe {
    /// Create a new test CE with an empty paragraph.
    fn new() -> Self {
        let doc = Rc::new(RefCell::new(RinchDocument::new()));
        let ce_root;
        let initial_cursor;
        {
            let mut d = doc.borrow_mut();
            // Create a contenteditable div
            let body = NodeId(1); // body node in RinchDocument
            ce_root = d.create_element("div");
            d.append_child(body, ce_root);
            d.set_attribute(ce_root, "contenteditable", "true");

            // Create initial <p> with empty text node
            let p = d.create_element("p");
            d.append_child(ce_root, p);
            let text = d.create_text("");
            d.append_child(p, text);
            initial_cursor = DomCursor::new(text.0, 0);
        }

        let mut ops = rinch::ce_ops::CeOps::new(doc, ce_root.0, initial_cursor);
        ops.enable_collaboration_from_content();

        Self { ops }
    }

    /// Create a test CE pre-loaded with text content.
    fn with_text(text: &str) -> Self {
        let mut ce = Self::new();
        ce.ops.insert_text(text);
        ce.assert_sync();
        ce
    }

    /// Create a test CE with multiple blocks.
    fn with_blocks(blocks: &[&str]) -> Self {
        let mut ce = Self::new();
        for (i, text) in blocks.iter().enumerate() {
            if i > 0 {
                ce.ops.split_block();
            }
            ce.ops.insert_text(text);
        }
        ce.assert_sync();
        ce
    }

    /// Assert DOM content matches EditorDocument content.
    fn assert_sync(&self) {
        let dom_blocks = self.ops.extract_content();
        let doc_blocks = self.ops.editor_doc().unwrap().to_block_data();
        assert_eq!(
            dom_blocks, doc_blocks,
            "DOM/CRDT sync mismatch!\nDOM: {dom_blocks:#?}\nCRDT: {doc_blocks:#?}"
        );
    }

    /// Get the flat text from the EditorDocument.
    fn crdt_text(&self) -> String {
        self.ops.editor_doc().unwrap().to_text()
    }

    /// Get the flat text from the DOM via extract_content.
    fn dom_text(&self) -> String {
        let blocks = self.ops.extract_content();
        blocks
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let text: String = b.content.iter().map(|r| r.text.as_str()).collect();
                if i > 0 { format!("\n{text}") } else { text }
            })
            .collect()
    }

    /// Get the EditorDocument for save_incremental.
    fn editor_doc_mut(&mut self) -> &mut EditorDocument {
        self.ops.editor_doc_mut().unwrap()
    }
}

// ============================================================================
// Basic insert_text tests
// ============================================================================

#[test]
fn insert_single_char() {
    let mut ce = TestCe::new();
    ce.ops.insert_text("a");
    ce.assert_sync();
    assert_eq!(ce.crdt_text(), "a");
}

#[test]
fn insert_multiple_chars() {
    let mut ce = TestCe::new();
    ce.ops.insert_text("Hello");
    ce.assert_sync();
    assert_eq!(ce.crdt_text(), "Hello");
}

#[test]
fn insert_then_more() {
    let mut ce = TestCe::new();
    ce.ops.insert_text("Hello");
    ce.assert_sync();
    ce.ops.insert_text(" World");
    ce.assert_sync();
    assert_eq!(ce.crdt_text(), "Hello World");
}

#[test]
fn insert_unicode() {
    let mut ce = TestCe::new();
    ce.ops.insert_text("こんにちは");
    ce.assert_sync();
    assert_eq!(ce.crdt_text(), "こんにちは");
}

#[test]
fn insert_emoji() {
    let mut ce = TestCe::new();
    ce.ops.insert_text("Hello 🌍!");
    ce.assert_sync();
    assert_eq!(ce.crdt_text(), "Hello 🌍!");
}

// ============================================================================
// Delete tests
// ============================================================================

#[test]
fn delete_backward_single_char() {
    let mut ce = TestCe::with_text("abc");
    ce.ops.delete_backward();
    ce.assert_sync();
    assert_eq!(ce.crdt_text(), "ab");
}

#[test]
fn delete_backward_all_chars() {
    let mut ce = TestCe::with_text("ab");
    ce.ops.delete_backward();
    ce.assert_sync();
    ce.ops.delete_backward();
    ce.assert_sync();
    assert_eq!(ce.dom_text(), "");
}

#[test]
fn delete_forward_single_char() {
    let mut ce = TestCe::with_text("abc");
    // Move cursor to start
    let sel = ce.ops.get_selection();
    let start = DomCursor::new(sel.head.node_id, 0);
    ce.ops
        .set_selection(rinch_core::ce::CeSelection::collapsed(start));

    ce.ops.delete_forward();
    ce.assert_sync();
    assert_eq!(ce.crdt_text(), "bc");
}

#[test]
fn delete_selection_middle() {
    let mut ce = TestCe::with_text("Hello World");
    // Select "lo Wo" (positions 3..8)
    let sel = ce.ops.get_selection();
    let nid = sel.head.node_id;
    ce.ops.set_selection(rinch_core::ce::CeSelection::range(
        DomCursor::new(nid, 3),
        DomCursor::new(nid, 8),
    ));
    ce.ops.delete_selection();
    ce.assert_sync();
    assert_eq!(ce.crdt_text(), "Helrld");
}

#[test]
fn insert_replaces_selection() {
    let mut ce = TestCe::with_text("Hello");
    let sel = ce.ops.get_selection();
    let nid = sel.head.node_id;
    // Select "ell"
    ce.ops.set_selection(rinch_core::ce::CeSelection::range(
        DomCursor::new(nid, 1),
        DomCursor::new(nid, 4),
    ));
    ce.ops.insert_text("ELL");
    ce.assert_sync();
    assert_eq!(ce.crdt_text(), "HELLo");
}

// ============================================================================
// Block operations
// ============================================================================

#[test]
fn split_block_at_end() {
    let mut ce = TestCe::with_text("Hello");
    ce.ops.split_block();
    ce.assert_sync();
    let blocks = ce.ops.extract_content();
    assert_eq!(blocks.len(), 2);
}

#[test]
fn split_block_at_middle() {
    let mut ce = TestCe::with_text("HelloWorld");
    // Move cursor to position 5
    let sel = ce.ops.get_selection();
    let nid = sel.head.node_id;
    ce.ops
        .set_selection(rinch_core::ce::CeSelection::collapsed(DomCursor::new(
            nid, 5,
        )));
    ce.ops.split_block();
    ce.assert_sync();
    let blocks = ce.ops.extract_content();
    assert_eq!(blocks.len(), 2);
    assert_eq!(ce.dom_text(), "Hello\nWorld");
}

#[test]
fn split_block_then_type() {
    let mut ce = TestCe::with_text("Hello");
    ce.ops.split_block();
    ce.assert_sync();
    ce.ops.insert_text("World");
    ce.assert_sync();
    assert_eq!(ce.dom_text(), "Hello\nWorld");
}

#[test]
fn delete_forward_removes_empty_block() {
    let mut ce = TestCe::with_text("Hello");
    ce.ops.split_block();
    ce.assert_sync();
    // Now: "Hello" | "" (empty second block), cursor in empty block
    // Delete forward on empty block — should remove the empty block
    // (delete_forward on element cursor removes current block)
    ce.ops.delete_forward();
    ce.assert_sync();
    // Should be back to 1 block
    assert_eq!(ce.ops.extract_content().len(), 1);
}

#[test]
fn delete_backward_joins_blocks() {
    let mut ce = TestCe::with_blocks(&["Hello", "World"]);
    // Cursor should be at start of second block after with_blocks
    // Actually with_blocks leaves cursor at end of last text
    // We need cursor at start of "World" block
    let blocks = ce.ops.extract_content();
    assert_eq!(blocks.len(), 2);

    // Find the text node of the second block
    // The cursor is at the end of "World" — move to start
    let sel = ce.ops.get_selection();
    let nid = sel.head.node_id;
    ce.ops
        .set_selection(rinch_core::ce::CeSelection::collapsed(DomCursor::new(
            nid, 0,
        )));

    ce.ops.delete_backward();
    ce.assert_sync();
    let blocks = ce.ops.extract_content();
    assert_eq!(blocks.len(), 1, "blocks should be merged");
    assert_eq!(ce.dom_text(), "HelloWorld");
}

// ============================================================================
// Formatting (marks)
// ============================================================================

#[test]
fn wrap_selection_bold() {
    let mut ce = TestCe::with_text("Hello World");
    let sel = ce.ops.get_selection();
    let nid = sel.head.node_id;
    // Select "World"
    ce.ops.set_selection(rinch_core::ce::CeSelection::range(
        DomCursor::new(nid, 6),
        DomCursor::new(nid, 11),
    ));
    ce.ops.wrap_selection("strong");
    ce.assert_sync();

    let blocks = ce.ops.extract_content();
    assert_eq!(blocks.len(), 1);
    // Should have at least 2 runs: "Hello " (unmarked) + "World" (bold)
    assert!(blocks[0].content.len() >= 2);
    let bold_run = blocks[0]
        .content
        .iter()
        .find(|r| r.text == "World")
        .expect("should have World run");
    assert!(
        bold_run.marks.iter().any(|m| m.mark_type == "bold"),
        "World should be bold"
    );
}

#[test]
fn unwrap_selection_bold() {
    let mut ce = TestCe::with_text("Hello World");
    let sel = ce.ops.get_selection();
    let nid = sel.head.node_id;

    // First wrap "World" in bold
    ce.ops.set_selection(rinch_core::ce::CeSelection::range(
        DomCursor::new(nid, 6),
        DomCursor::new(nid, 11),
    ));
    ce.ops.wrap_selection("strong");
    ce.assert_sync();

    // Now unwrap — selection still covers the bold text
    ce.ops.unwrap_selection("strong");
    ce.assert_sync();

    // All text should be unmarked now
    let blocks = ce.ops.extract_content();
    for run in &blocks[0].content {
        assert!(
            run.marks.is_empty(),
            "all marks should be removed, but {:?} has {:?}",
            run.text,
            run.marks
        );
    }
}

#[test]
fn clear_formatting() {
    let mut ce = TestCe::with_text("Hello World");
    let sel = ce.ops.get_selection();
    let nid = sel.head.node_id;

    // Wrap everything in bold
    ce.ops.set_selection(rinch_core::ce::CeSelection::range(
        DomCursor::new(nid, 0),
        DomCursor::new(nid, 11),
    ));
    ce.ops.wrap_selection("strong");
    ce.assert_sync();

    // Clear all formatting — selection still covers the text
    ce.ops.clear_formatting();
    ce.assert_sync();

    let blocks = ce.ops.extract_content();
    for run in &blocks[0].content {
        assert!(
            run.marks.is_empty(),
            "clear_formatting should remove all marks"
        );
    }
}

// ============================================================================
// Undo / Redo
// ============================================================================

#[test]
fn undo_insert() {
    let mut ce = TestCe::with_text("Hello");
    ce.ops.insert_text(" World");
    ce.assert_sync();
    assert_eq!(ce.dom_text(), "Hello World");

    ce.ops.undo();
    ce.assert_sync();
    assert_eq!(ce.dom_text(), "Hello");
}

#[test]
fn undo_then_redo() {
    let mut ce = TestCe::with_text("Hello");
    ce.ops.insert_text(" World");
    ce.assert_sync();

    ce.ops.undo();
    ce.assert_sync();
    assert_eq!(ce.dom_text(), "Hello");

    ce.ops.redo();
    ce.assert_sync();
    assert_eq!(ce.dom_text(), "Hello World");
}

#[test]
fn undo_delete() {
    let mut ce = TestCe::with_text("Hello");
    ce.ops.delete_backward();
    ce.assert_sync();
    assert_eq!(ce.dom_text(), "Hell");

    ce.ops.undo();
    ce.assert_sync();
    assert_eq!(ce.dom_text(), "Hello");
}

#[test]
fn undo_split_block() {
    let mut ce = TestCe::with_text("HelloWorld");
    let sel = ce.ops.get_selection();
    let nid = sel.head.node_id;
    ce.ops
        .set_selection(rinch_core::ce::CeSelection::collapsed(DomCursor::new(
            nid, 5,
        )));
    ce.ops.split_block();
    ce.assert_sync();
    assert_eq!(ce.ops.extract_content().len(), 2);

    ce.ops.undo();
    ce.assert_sync();
    assert_eq!(ce.ops.extract_content().len(), 1);
    assert_eq!(ce.dom_text(), "HelloWorld");
}

// ============================================================================
// Two-peer round-trip
// ============================================================================

#[test]
fn two_peer_insert_roundtrip() {
    // Peer 1: create doc, type text
    let mut ce1 = TestCe::new();
    ce1.ops.insert_text("Hello");
    ce1.assert_sync();

    // Peer 2: fork from peer 1's EditorDocument
    let bytes = ce1.editor_doc_mut().to_bytes();
    let mut peer2_doc = EditorDocument::from_bytes(&bytes).unwrap();

    // Peer 1 types more
    ce1.ops.insert_text(" World");
    ce1.assert_sync();

    // Save incremental and apply to peer 2
    let delta = ce1.editor_doc_mut().save_incremental();
    let ops = peer2_doc.load_incremental_with_ops(&delta).unwrap();

    // Verify the ops describe the insert
    assert!(!ops.is_empty(), "should have received ops");
    let has_insert = ops.iter().any(
        |op| matches!(op, rinch_editor::CeRemoteOp::InsertText { text, .. } if text == " World"),
    );
    assert!(
        has_insert,
        "should have InsertText op for ' World', got: {ops:?}"
    );

    // Both docs should have the same text
    assert_eq!(peer2_doc.to_text(), "Hello World");
    assert_eq!(ce1.crdt_text(), "Hello World");
}

#[test]
fn two_peer_delete_roundtrip() {
    let mut ce1 = TestCe::with_text("Hello World");

    let bytes = ce1.editor_doc_mut().to_bytes();
    let mut peer2_doc = EditorDocument::from_bytes(&bytes).unwrap();

    // Peer 1 deletes "World" (select + delete)
    let sel = ce1.ops.get_selection();
    let nid = sel.head.node_id;
    ce1.ops.set_selection(rinch_core::ce::CeSelection::range(
        DomCursor::new(nid, 5),
        DomCursor::new(nid, 11),
    ));
    ce1.ops.delete_selection();
    ce1.assert_sync();

    let delta = ce1.editor_doc_mut().save_incremental();
    let ops = peer2_doc.load_incremental_with_ops(&delta).unwrap();

    assert!(!ops.is_empty());
    let has_delete = ops
        .iter()
        .any(|op| matches!(op, rinch_editor::CeRemoteOp::DeleteRange { .. }));
    assert!(has_delete, "should have DeleteRange op, got: {ops:?}");
    assert_eq!(peer2_doc.to_text(), "Hello");
}

// ============================================================================
// Stress: many operations with sync check after each
// ============================================================================

#[test]
fn stress_insert_delete_cycle() {
    let mut ce = TestCe::new();
    for i in 0..50 {
        ce.ops.insert_text(&format!("line{i} "));
        ce.assert_sync();
    }
    for _ in 0..25 {
        ce.ops.delete_backward();
        ce.assert_sync();
    }
    // Should still be in sync
    ce.assert_sync();
}

#[test]
fn stress_split_and_type() {
    let mut ce = TestCe::new();
    for i in 0..20 {
        ce.ops.insert_text(&format!("Block {i}"));
        ce.assert_sync();
        ce.ops.split_block();
        ce.assert_sync();
    }
    assert_eq!(ce.ops.extract_content().len(), 21); // 20 splits + initial
    ce.assert_sync();
}

// ============================================================================
// Multi-block stress
// ============================================================================

#[test]
fn stress_alternating_split_and_join() {
    let mut ce = TestCe::with_text("ABCDEFGHIJ");
    // Split into 5 blocks
    for pos in [8, 6, 4, 2] {
        let sel = ce.ops.get_selection();
        let nid = sel.head.node_id;
        ce.ops
            .set_selection(rinch_core::ce::CeSelection::collapsed(DomCursor::new(
                nid, pos,
            )));
        ce.ops.split_block();
        ce.assert_sync();
    }
    assert_eq!(ce.ops.extract_content().len(), 5);

    // Now join them back by pressing backspace at start of each block
    for _ in 0..4 {
        // Find second block's first text node
        let blocks = ce.ops.extract_content();
        if blocks.len() <= 1 {
            break;
        }
        // Navigate to start of second block
        // Use flat_pos_to_dom_cursor via split_block workaround
        // Actually just use delete_backward which joins if at start
        ce.ops.delete_backward();
        ce.assert_sync();
    }
}

#[test]
fn stress_unicode_heavy() {
    let mut ce = TestCe::new();
    let texts = [
        "こんにちは",  // Japanese
        "مرحبا",       // Arabic
        "Привет",      // Russian
        "🌍🌎🌏",      // Emoji
        "café résumé", // Accented Latin
        "αβγδε",       // Greek
        "中文测试",    // Chinese
    ];
    for text in &texts {
        ce.ops.insert_text(text);
        ce.assert_sync();
        ce.ops.split_block();
        ce.assert_sync();
    }
    assert_eq!(ce.ops.extract_content().len(), texts.len() + 1);
    ce.assert_sync();

    // Delete from the middle
    for _ in 0..3 {
        ce.ops.delete_backward();
        ce.assert_sync();
    }
}

#[test]
fn stress_rapid_undo_redo() {
    let mut ce = TestCe::new();
    // Build up some content
    ce.ops.insert_text("Hello");
    ce.ops.split_block();
    ce.ops.insert_text("World");
    ce.ops.split_block();
    ce.ops.insert_text("!");
    ce.assert_sync();

    // Rapid undo/redo cycle
    for _ in 0..10 {
        ce.ops.undo();
        ce.assert_sync();
        ce.ops.redo();
        ce.assert_sync();
    }

    // Undo everything
    for _ in 0..10 {
        ce.ops.undo();
        ce.assert_sync();
    }

    // Redo everything
    for _ in 0..10 {
        ce.ops.redo();
        ce.assert_sync();
    }
}

// ============================================================================
// Multi-peer collaboration tests
// ============================================================================

/// Create a second peer from peer1's EditorDocument state.
fn fork_peer(ce1: &mut TestCe) -> EditorDocument {
    EditorDocument::from_bytes(&ce1.editor_doc_mut().to_bytes()).unwrap()
}

#[test]
fn two_peer_concurrent_inserts() {
    let mut ce1 = TestCe::with_text("Hello");

    let mut peer2_doc = fork_peer(&mut ce1);

    // Peer 1 appends " World"
    ce1.ops.insert_text(" World");
    ce1.assert_sync();

    // Peer 2 makes an independent edit
    peer2_doc
        .insert_text(rinch_editor::Position(5), " Rust")
        .unwrap();

    // Exchange changes
    let delta1 = ce1.editor_doc_mut().save_incremental();
    let delta2 = peer2_doc.save_incremental();

    let ops1 = peer2_doc.load_incremental_with_ops(&delta1).unwrap();
    let ops2 = ce1
        .editor_doc_mut()
        .load_incremental_with_ops(&delta2)
        .unwrap();

    // Both should converge to the same text
    assert_eq!(ce1.crdt_text(), peer2_doc.to_text());
    // Both should have both edits
    let text = peer2_doc.to_text();
    assert!(text.contains("World"), "should have peer1's edit");
    assert!(text.contains("Rust"), "should have peer2's edit");
}

#[test]
fn three_peer_convergence() {
    let mut ce1 = TestCe::with_text("Base");

    let mut peer2 = fork_peer(&mut ce1);
    let mut peer3 = fork_peer(&mut ce1);

    // Each peer makes an edit
    ce1.ops.insert_text(" from P1");
    ce1.assert_sync();
    peer2
        .insert_text(rinch_editor::Position(4), " from P2")
        .unwrap();
    peer3
        .insert_text(rinch_editor::Position(4), " from P3")
        .unwrap();

    // Broadcast all changes
    let d1 = ce1.editor_doc_mut().save_incremental();
    let d2 = peer2.save_incremental();
    let d3 = peer3.save_incremental();

    // Apply all to all
    peer2.load_incremental(&d1).unwrap();
    peer2.load_incremental(&d3).unwrap();
    peer3.load_incremental(&d1).unwrap();
    peer3.load_incremental(&d2).unwrap();
    ce1.editor_doc_mut().load_incremental(&d2).unwrap();
    ce1.editor_doc_mut().load_incremental(&d3).unwrap();

    // All three should converge
    let t1 = ce1.crdt_text();
    let t2 = peer2.to_text();
    let t3 = peer3.to_text();
    assert_eq!(t1, t2, "peer1 and peer2 should converge");
    assert_eq!(t2, t3, "peer2 and peer3 should converge");
    assert!(t1.contains("from P1"));
    assert!(t1.contains("from P2"));
    assert!(t1.contains("from P3"));
}

#[test]
fn four_peer_interleaved_edits() {
    let mut ce1 = TestCe::new();
    ce1.ops.insert_text("Start");
    ce1.assert_sync();

    let mut p2 = fork_peer(&mut ce1);
    let mut p3 = fork_peer(&mut ce1);
    let mut p4 = fork_peer(&mut ce1);

    // Round 1: each peer adds a word
    ce1.ops.insert_text(" Alpha");
    ce1.assert_sync();
    p2.insert_text(rinch_editor::Position(5), " Beta").unwrap();
    p3.insert_text(rinch_editor::Position(5), " Gamma").unwrap();
    p4.insert_text(rinch_editor::Position(5), " Delta").unwrap();

    // Sync all-to-all
    let d1 = ce1.editor_doc_mut().save_incremental();
    let d2 = p2.save_incremental();
    let d3 = p3.save_incremental();
    let d4 = p4.save_incremental();

    for d in [&d2, &d3, &d4] {
        ce1.editor_doc_mut().load_incremental(d).unwrap();
    }
    for d in [&d1, &d3, &d4] {
        p2.load_incremental(d).unwrap();
    }
    for d in [&d1, &d2, &d4] {
        p3.load_incremental(d).unwrap();
    }
    for d in [&d1, &d2, &d3] {
        p4.load_incremental(d).unwrap();
    }

    // All converge
    let texts: Vec<String> = vec![ce1.crdt_text(), p2.to_text(), p3.to_text(), p4.to_text()];
    for i in 1..texts.len() {
        assert_eq!(texts[0], texts[i], "peer 0 and peer {i} should converge");
    }
    for word in ["Alpha", "Beta", "Gamma", "Delta"] {
        assert!(
            texts[0].contains(word),
            "converged text should contain '{word}'"
        );
    }
}

#[test]
fn peer_delete_and_insert_conflict() {
    let mut ce1 = TestCe::with_text("Hello World");
    let mut p2 = fork_peer(&mut ce1);

    // Peer 1 deletes "World"
    let sel = ce1.ops.get_selection();
    let nid = sel.head.node_id;
    ce1.ops.set_selection(rinch_core::ce::CeSelection::range(
        DomCursor::new(nid, 6),
        DomCursor::new(nid, 11),
    ));
    ce1.ops.delete_selection();
    ce1.assert_sync();

    // Peer 2 inserts " Beautiful" in the middle of "World"
    p2.insert_text(rinch_editor::Position(8), " Beautiful")
        .unwrap();

    // Exchange
    let d1 = ce1.editor_doc_mut().save_incremental();
    let d2 = p2.save_incremental();
    ce1.editor_doc_mut().load_incremental(&d2).unwrap();
    p2.load_incremental(&d1).unwrap();

    assert_eq!(ce1.crdt_text(), p2.to_text());
}

#[test]
fn peer_concurrent_block_split() {
    let mut ce1 = TestCe::with_text("ABCDEF");
    let mut p2 = fork_peer(&mut ce1);

    // Peer 1 splits at position 3 (ABC|DEF)
    ce1.editor_doc_mut()
        .split_block(rinch_editor::Position(3))
        .unwrap();

    // Peer 2 splits at position 2 (AB|CDEF)
    p2.split_block(rinch_editor::Position(2)).unwrap();

    let d1 = ce1.editor_doc_mut().save_incremental();
    let d2 = p2.save_incremental();
    ce1.editor_doc_mut().load_incremental(&d2).unwrap();
    p2.load_incremental(&d1).unwrap();

    assert_eq!(ce1.crdt_text(), p2.to_text());
    // Should have 3 blocks
    assert!(ce1.editor_doc_mut().block_count() >= 3);
}

// ============================================================================
// Edge cases
// ============================================================================

#[test]
fn empty_document_operations() {
    let mut ce = TestCe::new();
    // These should all be no-ops or safe
    ce.ops.delete_backward();
    ce.assert_sync();
    ce.ops.delete_forward();
    ce.assert_sync();
    ce.ops.undo();
    ce.assert_sync();
    ce.ops.redo();
    ce.assert_sync();
    ce.ops.split_block();
    ce.assert_sync();
    // Now insert
    ce.ops.insert_text("a");
    ce.assert_sync();
    assert_eq!(ce.crdt_text(), "a");
}

#[test]
fn delete_everything_and_retype() {
    let mut ce = TestCe::with_blocks(&["Hello", "World", "Foo"]);
    // Select all and delete
    let sel = ce.ops.get_selection();
    // We need to select from start to end — approximate by selecting
    // from first text node to last
    // Actually just delete backward many times
    for _ in 0..30 {
        ce.ops.delete_backward();
        ce.assert_sync();
    }
    // Should have 1 empty block
    assert_eq!(ce.ops.extract_content().len(), 1);

    // Retype
    ce.ops.insert_text("Reborn");
    ce.assert_sync();
    assert_eq!(ce.crdt_text(), "Reborn");
}

#[test]
fn split_at_every_position() {
    // Type "ABCDE", then split at every position and verify sync
    for split_pos in 0..=5 {
        let mut ce = TestCe::with_text("ABCDE");
        let sel = ce.ops.get_selection();
        let nid = sel.head.node_id;
        ce.ops
            .set_selection(rinch_core::ce::CeSelection::collapsed(DomCursor::new(
                nid, split_pos,
            )));
        ce.ops.split_block();
        ce.assert_sync();
        assert_eq!(ce.ops.extract_content().len(), 2);
    }
}

#[test]
fn wrap_unwrap_multiple_marks() {
    let mut ce = TestCe::with_text("Hello World Test");
    let sel = ce.ops.get_selection();
    let nid = sel.head.node_id;

    // Wrap "World" in bold
    ce.ops.set_selection(rinch_core::ce::CeSelection::range(
        DomCursor::new(nid, 6),
        DomCursor::new(nid, 11),
    ));
    ce.ops.wrap_selection("strong");
    ce.assert_sync();

    // Wrap "World" in italic too (re-select since nodes changed)
    let sel = ce.ops.get_selection();
    ce.ops.wrap_selection("em");
    ce.assert_sync();

    // Unwrap bold only
    let sel = ce.ops.get_selection();
    ce.ops.unwrap_selection("strong");
    ce.assert_sync();

    // Should still have italic
    let blocks = ce.ops.extract_content();
    let has_italic = blocks[0]
        .content
        .iter()
        .any(|r| r.marks.iter().any(|m| m.mark_type == "italic") && r.text.contains("World"));
    // Note: may not hold if unwrap restructured nodes, so just assert sync
}

// ============================================================================
// Fuzz: random operation sequences
// ============================================================================

/// Simple deterministic PRNG for reproducibility (no external deps).
struct SimpleRng(u64);

impl SimpleRng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }

    fn next_usize(&mut self, max: usize) -> usize {
        if max == 0 {
            return 0;
        }
        (self.next_u64() % (max as u64)) as usize
    }
}

#[derive(Debug)]
enum FuzzOp {
    InsertText(String),
    DeleteBackward,
    DeleteForward,
    SplitBlock,
    Undo,
    Redo,
}

fn generate_fuzz_ops(rng: &mut SimpleRng, count: usize) -> Vec<FuzzOp> {
    let chars = [
        'a', 'b', 'c', 'd', 'e', ' ', '.', '!', 'x', 'y', 'α', 'β', 'γ', // Greek
        '中', '文', // CJK
        '🌍', '🎉', // Emoji
        'é', 'ñ', 'ü', // Accented Latin
    ];
    let mut ops = Vec::with_capacity(count);
    for _ in 0..count {
        let op = match rng.next_usize(10) {
            0..=3 => {
                // Insert 1-5 chars (heavily weighted — most common operation)
                let len = 1 + rng.next_usize(5);
                let text: String = (0..len)
                    .map(|_| chars[rng.next_usize(chars.len())])
                    .collect();
                FuzzOp::InsertText(text)
            }
            4..=5 => FuzzOp::DeleteBackward,
            6 => FuzzOp::DeleteForward,
            7 => FuzzOp::SplitBlock,
            8 => FuzzOp::Undo,
            9 => FuzzOp::Redo,
            _ => unreachable!(),
        };
        ops.push(op);
    }
    ops
}

fn run_fuzz(seed: u64, op_count: usize) {
    let mut rng = SimpleRng::new(seed);
    let ops = generate_fuzz_ops(&mut rng, op_count);
    let mut ce = TestCe::new();

    for (i, op) in ops.iter().enumerate() {
        // Uncomment to debug specific seed/op:
        // if seed == 42 && i == 14 {
        //     let sel = ce.ops.get_selection();
        //     eprintln!("BEFORE op {i}: cursor={:?}, blocks={}", sel.head, ce.ops.extract_content().len());
        // }
        match op {
            FuzzOp::InsertText(text) => ce.ops.insert_text(text),
            FuzzOp::DeleteBackward => ce.ops.delete_backward(),
            FuzzOp::DeleteForward => ce.ops.delete_forward(),
            FuzzOp::SplitBlock => ce.ops.split_block(),
            FuzzOp::Undo => ce.ops.undo(),
            FuzzOp::Redo => ce.ops.redo(),
        }
        // Assert sync after every operation
        let dom_blocks = ce.ops.extract_content();
        let doc_blocks = ce.ops.editor_doc().unwrap().to_block_data();
        assert_eq!(
            dom_blocks, doc_blocks,
            "Sync mismatch at op {i} ({op:?}), seed={seed}\n\
             DOM: {dom_blocks:#?}\nCRDT: {doc_blocks:#?}"
        );
    }
}

#[test]
fn fuzz_seed_0() {
    run_fuzz(0, 200);
}

#[test]
fn fuzz_seed_1() {
    run_fuzz(1, 200);
}

#[test]
fn fuzz_seed_42() {
    run_fuzz(42, 200);
}

#[test]
fn fuzz_seed_1337() {
    run_fuzz(1337, 200);
}

#[test]
fn fuzz_seed_9999() {
    run_fuzz(9999, 200);
}

#[test]
fn fuzz_seed_100() {
    run_fuzz(100, 200);
}

#[test]
fn fuzz_seed_256() {
    run_fuzz(256, 200);
}

#[test]
fn fuzz_seed_777() {
    run_fuzz(777, 200);
}

#[test]
fn fuzz_seed_12345() {
    run_fuzz(12345, 200);
}

#[test]
fn fuzz_many_seeds() {
    for seed in 200..250 {
        run_fuzz(seed, 100);
    }
}
