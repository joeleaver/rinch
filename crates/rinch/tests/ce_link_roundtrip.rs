//! Link mark `href` round-trips through `load_content()` → `extract_content()`.
//!
//! This guards the CE DOM-bridge fix in `ce_render.rs` (recognize `<a>` on
//! extract, apply `mark.attrs` on render, map `"link"` → `"a"`). The bug is
//! independent of collaboration, so — unlike `ce_crdt_sync.rs` — this file is
//! NOT gated behind `#![cfg(feature = "collaboration")]` and runs in a plain
//! `cargo test -p rinch`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rinch_core::ce::{BlockData, ContentEditableApi, DomCursor, InlineMarkData, InlineRunData};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

/// Build a bare CeOps with one empty `<p>` — no collaboration enabled.
fn new_ce() -> rinch::ce_ops::CeOps {
    let doc = Rc::new(RefCell::new(RinchDocument::new()));
    let ce_root;
    let initial_cursor;
    {
        let mut d = doc.borrow_mut();
        let body = NodeId(1);
        ce_root = d.create_element("div");
        d.append_child(body, ce_root);
        d.set_attribute(ce_root, "contenteditable", "true");

        let p = d.create_element("p");
        d.append_child(ce_root, p);
        let text = d.create_text("");
        d.append_child(p, text);
        initial_cursor = DomCursor::new(text.0, 0);
    }
    rinch::ce_ops::CeOps::new(doc, ce_root.0, initial_cursor)
}

#[test]
fn link_href_round_trips_through_load_and_extract() {
    let mut ops = new_ce();

    let mut href_attrs = HashMap::new();
    href_attrs.insert("href".to_string(), "https://example.com".to_string());

    let blocks = vec![BlockData {
        block_type: "paragraph".to_string(),
        attrs: HashMap::new(),
        content: vec![
            InlineRunData {
                text: "See ".to_string(),
                marks: vec![],
            },
            InlineRunData {
                text: "this link".to_string(),
                marks: vec![InlineMarkData {
                    mark_type: "link".to_string(),
                    attrs: href_attrs,
                }],
            },
        ],
    }];

    // load_content renders the DOM (render_inline_content); extract_content reads
    // it back (extract_inline_runs). Before the fix, the <a>'s href was dropped on
    // both legs (and "link" produced a bogus <link> element).
    ops.load_content(&blocks);
    let extracted = ops.extract_content();

    let link_run = extracted
        .iter()
        .flat_map(|b| &b.content)
        .find(|r| r.text == "this link")
        .expect("link run should survive round-trip");
    let link_mark = link_run
        .marks
        .iter()
        .find(|m| m.mark_type == "link")
        .expect("link mark should survive round-trip");
    assert_eq!(
        link_mark.attrs.get("href").map(|s| s.as_str()),
        Some("https://example.com"),
        "href must round-trip through load_content → extract_content"
    );

    // The CRDT view (always present, no collaboration needed) must agree with the
    // DOM extract — same link, same href.
    assert_eq!(
        extracted,
        ops.editor_doc().to_block_data(),
        "DOM extract and CRDT to_block_data must agree on the link"
    );
}
