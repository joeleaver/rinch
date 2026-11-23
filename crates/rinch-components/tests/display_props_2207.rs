//! #2207: configurability gaps in the display components — Text, Title,
//! Highlight, Blockquote, Divider, List, Skeleton and SimpleGrid — and the
//! bugs found beside them.
//!
//! Each new prop is checked on a rendered tree (the mock document), and each
//! default is checked to render what the component rendered before. What only
//! layout can show (SimpleGrid's template yielding to an app stylesheet, the
//! loaded Skeleton's wrapper generating no box, Text's line clamp) is pinned in
//! `crates/rinch/src/app/display_props_2207_tests.rs`.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::{Blockquote, Highlight, Skeleton};
use rinch_core::Component;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};

mod common;
use common::{collect_by_class, find_by_class, has_class, sheet};

/// A rendered component, with its document and scope kept alive around it.
struct Mounted {
    _doc: Rc<RefCell<MockDomDocument>>,
    scope: RenderScope,
    root: NodeHandle,
}

fn mount_with<C: Component>(component: C, children: &[&str]) -> Mounted {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let children: Vec<NodeHandle> = children.iter().map(|t| scope.create_text(t)).collect();
    let root = component.render(&mut scope, &children);
    Mounted {
        _doc: doc,
        scope,
        root,
    }
}

fn mount<C: Component>(component: C) -> Mounted {
    mount_with(component, &[])
}

/// The text of every `<mark>` a Highlight rendered, in order.
fn marks(root: &NodeHandle) -> Vec<String> {
    let mut out = Vec::new();
    collect_by_class(root, "rinch-highlight__match", &mut out);
    out.iter()
        .map(|m| m.text_content().unwrap_or_default())
        .collect()
}

/// All of a node's text, its children's included.
fn all_text(node: &NodeHandle) -> String {
    let mut s = node.text_content().unwrap_or_default();
    if node.tag_name().is_some() {
        s.clear();
        for c in node.children() {
            s.push_str(&all_text(&c));
        }
    }
    s
}

// ------------------------------------------------------- Highlight (bug)

/// `İ` (U+0130, 2 bytes) lowercases to `i̇` (3 bytes). Matching on the
/// lowercased copy and slicing the original with its offsets highlighted the
/// wrong letters, or panicked on a slice past the end.
#[test]
fn a_case_insensitive_match_after_a_length_changing_letter_marks_the_right_text() {
    let m = mount(Highlight {
        text: "İab".into(),
        highlight: "a".into(),
        ..Default::default()
    });
    assert_eq!(marks(&m.root), vec!["a".to_string()]);
    assert_eq!(all_text(&m.root), "İab");
}

#[test]
fn a_case_insensitive_match_at_the_end_after_a_length_changing_letter_does_not_panic() {
    let m = mount(Highlight {
        text: "İx".into(),
        highlight: "x".into(),
        ..Default::default()
    });
    assert_eq!(marks(&m.root), vec!["x".to_string()]);
    assert_eq!(all_text(&m.root), "İx");
}

#[test]
fn a_case_insensitive_term_matches_the_original_case() {
    let m = mount(Highlight {
        text: "Find FIND find".into(),
        highlight: "find".into(),
        ..Default::default()
    });
    assert_eq!(marks(&m.root), vec!["Find", "FIND", "find"]);
    assert_eq!(all_text(&m.root), "Find FIND find");

    let m = mount(Highlight {
        text: "Find FIND find".into(),
        highlight: "find".into(),
        ignore_case: false,
        ..Default::default()
    });
    assert_eq!(marks(&m.root), vec!["find"]);
}

// ------------------------------------------------------ Blockquote (bug)

/// The body was a `<p>`, so a block child was invalid HTML (and the web
/// parser would close the paragraph before it).
#[test]
fn a_blockquotes_body_is_a_div_so_block_children_are_valid() {
    let m = mount_with(Blockquote::default(), &["quote"]);
    let body = find_by_class(&m.root, "rinch-blockquote__body").expect("body");
    assert_eq!(body.tag_name().as_deref(), Some("div"));
    assert_eq!(all_text(&body), "quote");
}

// -------------------------------------------------------- Skeleton (bug)

/// The loaded wrapper was a bare inline `<span>`, so block children sat in
/// an inline box and a flex or grid parent saw one item. It is now a span that
/// generates no box (`display: contents`).
#[test]
fn a_loaded_skeletons_wrapper_generates_no_box() {
    let m = mount_with(
        Skeleton {
            visible: false,
            ..Default::default()
        },
        &["loaded"],
    );
    assert!(has_class(&m.root, "rinch-skeleton__content"));
    assert!(!has_class(&m.root, "rinch-skeleton"));
    assert_eq!(all_text(&m.root), "loaded");
    let css = sheet();
    let rule = css
        .split(".rinch-skeleton__content")
        .nth(1)
        .expect("a rule for the wrapper");
    let body = &rule[..rule.find('}').unwrap()];
    assert!(body.contains("display: contents"), "{body}");
}
