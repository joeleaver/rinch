//! Comprehensive tests for RinchDocument's DomDocument implementation.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

// === Node Creation ===

#[test]
fn test_create_element() {
    let mut doc = RinchDocument::new();
    let div = doc.create_element("div");
    assert!(doc.tree.contains(div.0));
    assert_eq!(doc.tree.get(div.0).unwrap().tag(), Some("div"));
}

#[test]
fn test_create_text() {
    let mut doc = RinchDocument::new();
    let text = doc.create_text("hello");
    assert!(doc.tree.contains(text.0));
    assert_eq!(doc.tree.get(text.0).unwrap().text_content(), Some("hello"));
}

#[test]
fn test_create_comment() {
    let mut doc = RinchDocument::new();
    let comment = doc.create_comment("marker");
    assert!(doc.tree.contains(comment.0));
    // Comments shouldn't have tag or text_content accessors
    assert!(doc.tree.get(comment.0).unwrap().tag().is_none());
}

#[test]
fn test_create_multiple_elements() {
    let mut doc = RinchDocument::new();
    let a = doc.create_element("div");
    let b = doc.create_element("span");
    let c = doc.create_element("p");
    // All should have distinct IDs
    assert_ne!(a, b);
    assert_ne!(b, c);
    assert_ne!(a, c);
}

// === Tree Structure ===

#[test]
fn test_initial_structure() {
    let doc = RinchDocument::new();
    let root = doc.root();
    let body = doc.body();
    // Body's parent chain should lead to root
    let body_parent = doc.parent_node(body);
    assert!(body_parent.is_some());
    // html element
    let html = body_parent.unwrap();
    let html_parent = doc.parent_node(html);
    assert_eq!(html_parent, Some(root));
}

#[test]
fn test_append_child() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.append_child(body, div);

    let children = doc.get_children(body);
    assert_eq!(children.len(), 1);
    assert_eq!(children[0], div);
    assert_eq!(doc.parent_node(div), Some(body));
}

#[test]
fn test_append_multiple_children() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let a = doc.create_element("div");
    let b = doc.create_element("span");
    let c = doc.create_element("p");
    doc.append_child(body, a);
    doc.append_child(body, b);
    doc.append_child(body, c);

    let children = doc.get_children(body);
    assert_eq!(children, vec![a, b, c]);
}

#[test]
fn test_append_child_moves_from_old_parent() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let parent1 = doc.create_element("div");
    let parent2 = doc.create_element("div");
    let child = doc.create_element("span");
    doc.append_child(body, parent1);
    doc.append_child(body, parent2);
    doc.append_child(parent1, child);

    assert_eq!(doc.get_children(parent1), vec![child]);
    assert_eq!(doc.get_children(parent2), vec![]);

    // Move child to parent2
    doc.append_child(parent2, child);
    assert_eq!(doc.get_children(parent1), vec![]);
    assert_eq!(doc.get_children(parent2), vec![child]);
    assert_eq!(doc.parent_node(child), Some(parent2));
}

#[test]
fn test_remove_child() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.append_child(body, div);
    assert_eq!(doc.get_children(body).len(), 1);

    doc.remove_child(body, div);
    assert_eq!(doc.get_children(body).len(), 0);
    assert_eq!(doc.parent_node(div), None);
}

#[test]
fn test_insert_before() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let a = doc.create_element("div");
    let b = doc.create_element("span");
    let c = doc.create_element("p");
    doc.append_child(body, a);
    doc.append_child(body, c);

    // Insert b before c
    doc.insert_before(body, b, c);
    let children = doc.get_children(body);
    assert_eq!(children, vec![a, b, c]);
}

#[test]
fn test_insert_before_at_start() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let a = doc.create_element("div");
    let b = doc.create_element("span");
    doc.append_child(body, b);

    // Insert a before b (at the start)
    doc.insert_before(body, a, b);
    let children = doc.get_children(body);
    assert_eq!(children, vec![a, b]);
}

#[test]
fn test_replace_node() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let old = doc.create_element("div");
    let new = doc.create_element("span");
    doc.append_child(body, old);

    doc.replace_node(old, new);
    let children = doc.get_children(body);
    assert_eq!(children, vec![new]);
    assert_eq!(doc.parent_node(new), Some(body));
    assert_eq!(doc.parent_node(old), None);
}

#[test]
fn test_remove_node() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.append_child(body, div);

    doc.remove_node(div);
    assert_eq!(doc.get_children(body).len(), 0);
    assert_eq!(doc.parent_node(div), None);
}

#[test]
fn test_insert_child_at_index() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let a = doc.create_element("div");
    let b = doc.create_element("span");
    let c = doc.create_element("p");
    doc.append_child(body, a);
    doc.append_child(body, c);

    // Insert b at index 1 (between a and c)
    doc.insert_child(body, b, 1);
    let children = doc.get_children(body);
    assert_eq!(children, vec![a, b, c]);
}

#[test]
fn test_insert_child_at_end() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let a = doc.create_element("div");
    let b = doc.create_element("span");
    doc.append_child(body, a);

    // Insert at index >= len should append
    doc.insert_child(body, b, 999);
    let children = doc.get_children(body);
    assert_eq!(children, vec![a, b]);
}

// === Siblings ===

#[test]
fn test_next_sibling() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let a = doc.create_element("div");
    let b = doc.create_element("span");
    let c = doc.create_element("p");
    doc.append_child(body, a);
    doc.append_child(body, b);
    doc.append_child(body, c);

    assert_eq!(doc.next_sibling(a), Some(b));
    assert_eq!(doc.next_sibling(b), Some(c));
    assert_eq!(doc.next_sibling(c), None);
}

// === Text Content ===

#[test]
fn test_set_text_content_on_text_node() {
    let mut doc = RinchDocument::new();
    let text = doc.create_text("hello");
    doc.set_text_content(text, "world");
    assert_eq!(doc.tree.get(text.0).unwrap().text_content(), Some("world"));
}

#[test]
fn test_set_text_content_on_element() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.append_child(body, div);

    doc.set_text_content(div, "hello");
    let children = doc.get_children(div);
    assert_eq!(children.len(), 1);
    assert_eq!(
        doc.tree.get(children[0].0).unwrap().text_content(),
        Some("hello")
    );
}

#[test]
fn test_set_text_content_replaces_existing_children() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    let old_child = doc.create_element("span");
    doc.append_child(body, div);
    doc.append_child(div, old_child);

    doc.set_text_content(div, "replaced");
    let children = doc.get_children(div);
    assert_eq!(children.len(), 1);
    // Old child should be detached
    assert_eq!(doc.parent_node(old_child), None);
}

// === Attributes ===

#[test]
fn test_set_and_get_attribute() {
    let mut doc = RinchDocument::new();
    let div = doc.create_element("div");
    doc.set_attribute(div, "id", "test");
    assert_eq!(doc.get_attribute(div, "id"), Some("test".to_string()));
}

#[test]
fn test_remove_attribute() {
    let mut doc = RinchDocument::new();
    let div = doc.create_element("div");
    doc.set_attribute(div, "id", "test");
    doc.remove_attribute(div, "id");
    assert_eq!(doc.get_attribute(div, "id"), None);
}

#[test]
fn test_get_nonexistent_attribute() {
    let mut doc = RinchDocument::new();
    let div = doc.create_element("div");
    assert_eq!(doc.get_attribute(div, "foo"), None);
}

#[test]
fn test_set_attribute_overwrites() {
    let mut doc = RinchDocument::new();
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "old");
    doc.set_attribute(div, "class", "new");
    assert_eq!(doc.get_attribute(div, "class"), Some("new".to_string()));
}

// === Style ===

#[test]
fn test_set_style() {
    let mut doc = RinchDocument::new();
    let div = doc.create_element("div");
    doc.set_style(div, "display", "flex");
    // Should be stored in the style attribute
    let style = doc.get_attribute(div, "style").unwrap();
    assert!(style.contains("display: flex") || style.contains("display:flex"));
}

#[test]
fn test_set_multiple_styles() {
    let mut doc = RinchDocument::new();
    let div = doc.create_element("div");
    doc.set_style(div, "display", "flex");
    doc.set_style(div, "gap", "8px");
    let style = doc.get_attribute(div, "style").unwrap();
    assert!(style.contains("display"));
    assert!(style.contains("gap"));
}

// === Inline style declaration order (#265) ===
//
// `set_style` rewrites the whole `style` attribute, and `set_styles` parses
// that string into the declaration block Stylo cascades — so the order these
// tests assert is the order the cascade resolves, not cosmetics. It used to
// come out of a `HashMap`, i.e. a fresh random order per process, which made
// "does this longhand beat that shorthand" a coin flip that a green local run
// said nothing about. Every assertion here is full equality on purpose: a
// `contains` cannot see an ordering bug.
//
// One of these tests is not like the others, and the difference is the whole
// lesson of the bug: **a per-case assertion cannot reliably catch a
// per-process randomisation; an invariance assertion can.** Each two-declaration
// test below fails only when the hasher happens to pick the wrong one of two
// orders — measured at 3-in-5, 2-in-5 and 4-in-5 runs on the unfixed code, so
// any of them can be green all afternoon and red in CI. `merged_inline_style_is_
// the_same_in_every_document` asserts that fifty documents agree with each
// other rather than asserting what they agree *on*, and failed 5 runs out of 5.
// Prefer that shape whenever the thing under test is nondeterministic.

/// A property `set_style` adds is appended, so it lands *after* a shorthand
/// already in the attribute and wins — the #265/#387 repro. Under the
/// `HashMap` this string came out `"left: 25px; inset: 0"` about half the time,
/// silently discarding the caller's write.
#[test]
fn set_style_appends_after_an_existing_shorthand() {
    let mut doc = RinchDocument::new();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "inset: 0");
    doc.set_style(div, "left", "25px");
    assert_eq!(
        doc.get_attribute(div, "style").unwrap(),
        "inset: 0; left: 25px"
    );
}

/// A property that is *already* declared is replaced where it stands; the
/// declarations around it do not move.
#[test]
fn set_style_replaces_a_declaration_in_place() {
    let mut doc = RinchDocument::new();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "color: red; gap: 4px");
    doc.set_style(div, "color", "blue");
    assert_eq!(
        doc.get_attribute(div, "style").unwrap(),
        "color: blue; gap: 4px"
    );
}

/// A batch that mixes a replacement with an addition does both: `color` stays
/// where it was, `padding` goes on the end.
#[test]
fn set_styles_batch_replaces_in_place_and_appends() {
    let mut doc = RinchDocument::new();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "color: red; gap: 4px");
    doc.set_styles(div, &[("color", "blue"), ("padding", "2px")]);
    assert_eq!(
        doc.get_attribute(div, "style").unwrap(),
        "color: blue; gap: 4px; padding: 2px"
    );
}

/// Two *new* properties in one batch land in the caller's order. This needs
/// two additions to say anything: a batch of one replacement plus one addition
/// gives the same string whichever order it is applied in, so it cannot pin
/// this.
#[test]
fn set_styles_batch_keeps_its_own_order() {
    let mut doc = RinchDocument::new();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "color: red");
    doc.set_styles(div, &[("padding", "2px"), ("margin", "1px")]);
    assert_eq!(
        doc.get_attribute(div, "style").unwrap(),
        "color: red; padding: 2px; margin: 1px"
    );
}

/// A property declared twice in one authored attribute collapses the way CSSOM
/// collapses it: the last value, at the **last** declaration's position — so
/// the `color` here moves past the `gap` that sat between its two
/// declarations. It said "at the first position" until #670; see
/// `a_repeated_property_collapses_at_its_last_position` below for the Chrome
/// measurement and `collapsing_at_the_last_position_is_what_the_cascade_
/// resolves` for why the position is behaviour rather than spelling.
#[test]
fn parsing_an_attribute_collapses_a_repeated_property_in_place() {
    let mut doc = RinchDocument::new();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "color: red; gap: 4px; color: green");
    doc.set_style(div, "gap", "8px");
    assert_eq!(
        doc.get_attribute(div, "style").unwrap(),
        "gap: 8px; color: green"
    );
}

/// Belt and braces for the property the other tests rest on: the same input
/// gives the same string in fifty freshly built documents. This is the test
/// that would have caught the original measurement (92 differing results in
/// 200 documents) without depending on which order the hasher happened to pick
/// in the run that reported it.
#[test]
fn merged_inline_style_is_the_same_in_every_document() {
    let mut seen: Vec<String> = Vec::new();
    for _ in 0..50 {
        let mut doc = RinchDocument::new();
        let div = doc.create_element("div");
        doc.set_attribute(div, "style", "inset: 0; width: 10px; height: 10px");
        doc.set_style(div, "left", "25px");
        doc.set_style(div, "width", "20px");
        seen.push(doc.get_attribute(div, "style").unwrap());
    }
    let first = &seen[0];
    assert_eq!(
        first, "inset: 0; width: 20px; height: 10px; left: 25px",
        "the one order every document must produce"
    );
    assert!(
        seen.iter().all(|s| s == first),
        "inline style order differs between documents: {seen:?}"
    );
}

// === Dirty Tracking ===

#[test]
fn test_dirty_nodes_empty_initially() {
    let mut doc = RinchDocument::new();
    // take_dirty_nodes should include the initial structure nodes
    let _dirty = doc.take_dirty_nodes();
    // After take, should be empty
    let dirty2 = doc.take_dirty_nodes();
    assert!(dirty2.is_empty());
}

#[test]
fn test_append_child_marks_parent_dirty() {
    let mut doc = RinchDocument::new();
    let _ = doc.take_dirty_nodes(); // Clear initial dirty
    let body = doc.body();
    let div = doc.create_element("div");
    doc.append_child(body, div);

    let dirty = doc.take_dirty_nodes();
    assert!(dirty.contains(&body));
}

#[test]
fn test_set_text_marks_dirty() {
    let mut doc = RinchDocument::new();
    let text = doc.create_text("hello");
    let body = doc.body();
    doc.append_child(body, text);
    let _ = doc.take_dirty_nodes(); // Clear

    doc.set_text_content(text, "world");
    let dirty = doc.take_dirty_nodes();
    assert!(dirty.contains(&text));
}

#[test]
fn test_set_attribute_marks_dirty() {
    let mut doc = RinchDocument::new();
    let div = doc.create_element("div");
    let body = doc.body();
    doc.append_child(body, div);
    let _ = doc.take_dirty_nodes(); // Clear

    doc.set_attribute(div, "id", "test");
    let dirty = doc.take_dirty_nodes();
    assert!(dirty.contains(&div));
}

#[test]
fn test_mark_dirty() {
    let mut doc = RinchDocument::new();
    let div = doc.create_element("div");
    let body = doc.body();
    doc.append_child(body, div);
    let _ = doc.take_dirty_nodes(); // Clear

    doc.mark_dirty(div);
    let dirty = doc.take_dirty_nodes();
    assert!(dirty.contains(&div));
}

#[test]
fn test_dirty_deduplication() {
    let mut doc = RinchDocument::new();
    let div = doc.create_element("div");
    let body = doc.body();
    doc.append_child(body, div);
    let _ = doc.take_dirty_nodes(); // Clear

    doc.mark_dirty(div);
    doc.mark_dirty(div);
    doc.mark_dirty(div);
    let dirty = doc.take_dirty_nodes();
    // div should appear only once
    assert_eq!(dirty.iter().filter(|&&n| n == div).count(), 1);
}

// === Query Selector ===

#[test]
fn test_query_selector_by_id() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "id", "target");
    doc.append_child(body, div);

    assert_eq!(doc.query_selector("#target"), Some(div));
}

#[test]
fn test_query_selector_by_class() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "foo bar");
    doc.append_child(body, div);

    assert_eq!(doc.query_selector(".foo"), Some(div));
    assert_eq!(doc.query_selector(".bar"), Some(div));
    assert_eq!(doc.query_selector(".baz"), None);
}

#[test]
fn test_query_selector_by_tag() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let p = doc.create_element("p");
    doc.append_child(body, p);

    // "html" and "body" exist in the initial tree
    assert!(doc.query_selector("html").is_some());
    assert!(doc.query_selector("body").is_some());
    assert_eq!(doc.query_selector("p"), Some(p));
    assert_eq!(doc.query_selector("nonexistent"), None);
}

#[test]
fn test_query_selector_deep_nesting() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let outer = doc.create_element("div");
    let inner = doc.create_element("div");
    let target = doc.create_element("span");
    doc.set_attribute(target, "id", "deep");
    doc.append_child(body, outer);
    doc.append_child(outer, inner);
    doc.append_child(inner, target);

    assert_eq!(doc.query_selector("#deep"), Some(target));
}

// === Scroll ===

#[test]
fn test_set_scroll_top() {
    let mut doc = RinchDocument::new();
    let div = doc.create_element("div");
    doc.set_scroll_top(div, 42.0);
    assert_eq!(doc.tree.get(div.0).unwrap().scroll_offset.1, 42.0);
}

// === parse_html and set_inner_html (Phase 9 stubs) ===

#[test]
fn test_parse_html_returns_none() {
    let mut doc = RinchDocument::new();
    // Phase 1: parse_html is not implemented yet
    assert_eq!(doc.parse_html("<div>test</div>"), None);
}

// === Nested Structure ===

#[test]
fn test_deep_nested_tree() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let mut parent = body;
    let mut nodes = vec![];
    for i in 0..10 {
        let child = doc.create_element("div");
        doc.set_attribute(child, "id", &format!("level-{}", i));
        doc.append_child(parent, child);
        nodes.push(child);
        parent = child;
    }

    // Verify parent chain
    for i in (1..10).rev() {
        assert_eq!(doc.parent_node(nodes[i]), Some(nodes[i - 1]));
    }
    assert_eq!(doc.parent_node(nodes[0]), Some(body));

    // Verify query_selector finds deepest node
    assert_eq!(doc.query_selector("#level-9"), Some(nodes[9]));
}

#[test]
fn test_mixed_node_types_as_children() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    let text = doc.create_text("hello");
    let comment = doc.create_comment("marker");
    let span = doc.create_element("span");
    doc.append_child(body, div);
    doc.append_child(div, text);
    doc.append_child(div, comment);
    doc.append_child(div, span);

    let children = doc.get_children(div);
    assert_eq!(children.len(), 3);
    assert_eq!(children[0], text);
    assert_eq!(children[1], comment);
    assert_eq!(children[2], span);
}

// === Inline style parsing: one parser, CSSOM's collapse position (#670) ===
//
// `set_style`/`set_styles` rewrite the whole `style` attribute from its own
// parsed contents, so *every* declaration already there is re-serialised on
// every write. Two properties of the parser are therefore load-bearing for
// declarations the caller never touched:
//
//   1. a `;` or `:` inside `url(…)` or a quoted string is part of a value, not
//      a separator — a parser that misses this destroys the value on the next
//      unrelated `set_style`;
//   2. a property declared twice collapses at the **last** declaration's
//      position, the way CSSOM collapses it.
//
// Both come from `rinch_core::dom::split_declarations`, which `rinch-web`'s
// `style:` prop path shares — one parser, one rule, both backends.

/// A `url()` carrying a `;` and a `:` (every `data:` URI does) survives an
/// unrelated `set_style` on the same node.
///
/// Splitting on a bare `;` cut this value in half: the attribute came back as
/// `background-image: url(data:image/png` — a declaration Stylo cannot parse,
/// so the image was gone — followed by a `base64,AAA=)` fragment.
#[test]
fn a_url_value_survives_the_next_set_style() {
    let mut doc = RinchDocument::new();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "background-image: url(data:image/png;base64,AAA=); color: red",
    );
    doc.set_style(div, "padding", "12px");
    assert_eq!(
        doc.get_attribute(div, "style").unwrap(),
        "background-image: url(data:image/png;base64,AAA=); color: red; padding: 12px"
    );
}

/// The same for a `;` inside a quoted value, which no bracket depth would
/// catch.
#[test]
fn a_semicolon_in_a_quoted_value_survives_the_next_set_style() {
    let mut doc = RinchDocument::new();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "content: \"a;b\"; color: red");
    doc.set_style(div, "padding", "12px");
    assert_eq!(
        doc.get_attribute(div, "style").unwrap(),
        "content: \"a;b\"; color: red; padding: 12px"
    );
}

/// The end-to-end half of the `url()` case: the value has to survive as far as
/// the *computed* style, not merely as far as the attribute string.
///
/// The declaration is the `background` shorthand — the spelling #670 reports —
/// so this also covers a `;` inside a shorthand's value rather than a
/// longhand's.
#[test]
fn a_background_url_still_paints_after_an_unrelated_set_style() {
    use rinch_dom::computed_style::BackgroundValue;

    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "width: 10px; height: 10px; background: url(data:image/png;base64,AAA=) no-repeat",
    );
    doc.append_child(body, div);
    doc.set_style(div, "padding", "12px");
    doc.resolve_layout(800.0, 600.0);

    match &doc.tree.get(div.0).unwrap().computed_style.background {
        BackgroundValue::Image { url } => assert_eq!(url, "data:image/png;base64,AAA="),
        other => panic!("the data URI was destroyed before the cascade saw it: {other:?}"),
    }
}

/// A property declared twice collapses to its **last** position, which is what
/// Chrome 150 does: `margin: 1px; color: red; gap: 2px; color: blue` serialises
/// as `margin: 1px; gap: 2px; color: blue`.
#[test]
fn a_repeated_property_collapses_at_its_last_position() {
    let mut doc = RinchDocument::new();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "margin: 1px; color: red; gap: 2px; color: blue",
    );
    doc.set_style(div, "padding", "12px");
    assert_eq!(
        doc.get_attribute(div, "style").unwrap(),
        "margin: 1px; gap: 2px; color: blue; padding: 12px"
    );
}

/// Why the collapse position is behaviour and not cosmetics: with a shorthand
/// involved, the two positions compute different values.
///
/// `inset: 0px; left: 25px; inset: 4px` gives Chrome a computed `left` of
/// `4px`, because the surviving `inset` sits *after* the `left` it overrides.
/// Collapsing at the first position re-serialises it as
/// `inset: 4px; left: 25px` and computes `25px` instead.
#[test]
fn collapsing_at_the_last_position_is_what_the_cascade_resolves() {
    use rinch_dom::computed_style::LengthPercentageAutoValue;

    let mut doc = RinchDocument::new();
    let body = doc.body();
    let parent = doc.create_element("div");
    doc.set_attribute(
        parent,
        "style",
        "position: relative; width: 300px; height: 200px",
    );
    doc.append_child(body, parent);
    let child = doc.create_element("div");
    doc.set_attribute(
        child,
        "style",
        "position: absolute; inset: 0px; left: 25px; inset: 4px",
    );
    doc.append_child(parent, child);
    // Any declaration the node does not already carry: the point is that the
    // *rewrite* is what collapses the duplicate, not this property.
    doc.set_style(child, "color", "red");
    doc.resolve_layout(800.0, 600.0);

    assert!(
        matches!(
            doc.tree.get(child.0).unwrap().computed_style.left,
            LengthPercentageAutoValue::Length(px) if px == 4.0
        ),
        "the surviving `inset` must keep the *last* duplicate's position, after \
         the `left` it overrides; computed left is {:?}",
        doc.tree.get(child.0).unwrap().computed_style.left
    );
    assert_eq!(
        doc.tree.get(child.0).unwrap().layout.x,
        4.0,
        "and the box must actually be laid out there"
    );
}

/// `!important` is part of the value and survives the round trip an unrelated
/// `set_style` puts every other declaration through.
///
/// The attribute is re-serialised from parsed declarations, so a parser that
/// split the priority off — or a join that dropped it — would silently demote
/// an author's `!important` on the first `set_style` to touch the node. The
/// computed assertion is what says the re-serialised string still *parses*:
/// string equality alone cannot tell `red !important` from a value Stylo
/// rejects.
#[test]
fn an_important_priority_survives_the_next_set_style() {
    use rinch_dom::computed_style::BackgroundValue;

    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "width: 10px; height: 10px; background-color: red !important",
    );
    doc.append_child(body, div);
    doc.set_style(div, "padding", "12px");
    assert_eq!(
        doc.get_attribute(div, "style").unwrap(),
        "width: 10px; height: 10px; background-color: red !important; padding: 12px"
    );

    doc.resolve_layout(800.0, 600.0);
    match doc.tree.get(div.0).unwrap().computed_style.background {
        BackgroundValue::Color(c) => assert_eq!(
            (c.components[0], c.components[1], c.components[2]),
            (1.0, 0.0, 0.0),
            "the re-serialised `red !important` must still parse"
        ),
        ref other => panic!("the declaration did not survive re-serialisation: {other:?}"),
    }
}

/// The inline `user-select` override — Stylo's servo build does not carry the
/// property, so `style_resolution` reads it off the attribute itself — reads
/// through the same parser as everything else (#670).
///
/// A bare `split(';')` fabricates a "declaration" out of the inside of a
/// quoted value: `content: "a; user-select: none; b"` gives it the part
/// `user-select: none`, which it then applies. Nothing in the document
/// declares `user-select` here, so the box must keep the default.
///
/// The quoted value needs a `;` on *both* sides of the smuggled declaration,
/// and that is the fixed-point trap in this fixture rather than a flourish:
/// with `content: "a; user-select: none"` the fabricated part is
/// `user-select: none"`, whose trailing quote `UserSelectValue::parse` does
/// not recognise — so it falls back to `Auto` and the naive reader passes the
/// fixture while still being wrong. Measured: that spelling let the mutant
/// survive.
#[test]
fn a_user_select_inside_a_quoted_value_is_not_a_declaration() {
    use rinch_dom::computed_style::UserSelectValue;

    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "width: 10px; height: 10px; content: \"a; user-select: none; b\"",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    assert!(
        matches!(
            doc.tree.get(div.0).unwrap().computed_style.user_select,
            UserSelectValue::Auto
        ),
        "the `user-select` inside the quoted `content` value is not a \
         declaration; got {:?}",
        doc.tree.get(div.0).unwrap().computed_style.user_select
    );

    // Positive control: a real declaration still reaches the computed style,
    // so a fixture that passes because the reader stopped working entirely
    // cannot pass silently.
    let other = doc.create_element("div");
    doc.set_attribute(other, "style", "user-select: none");
    doc.append_child(body, other);
    doc.resolve_layout(801.0, 600.0);
    assert!(
        matches!(
            doc.tree.get(other.0).unwrap().computed_style.user_select,
            UserSelectValue::None
        ),
        "positive control: an actual `user-select: none` must still apply"
    );
}

/// A `/*` inside an unquoted `url(…)` is part of the URL, and the round trip
/// an unrelated `set_style` puts the attribute through must keep it.
///
/// This is the end-to-end half of
/// `inline_style::a_comment_marker_inside_an_unquoted_url_is_part_of_the_url`,
/// and it is here because the computed value is what makes the defect
/// *silent*: stripping `/*b*/` leaves a perfectly well-formed declaration
/// naming a **different image**, so an attribute-string assertion is the only
/// thing that would catch it in the terminated case and nothing would catch
/// the consequence. Found by the PR #706 review (F1).
#[test]
fn a_comment_marker_inside_a_url_survives_the_next_set_style() {
    use rinch_dom::computed_style::BackgroundValue;

    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "width: 10px; height: 10px; background: url(http://example.test/a/*b*/c.png) no-repeat",
    );
    doc.append_child(body, div);
    doc.set_style(div, "padding", "12px");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        doc.get_attribute(div, "style").unwrap(),
        "width: 10px; height: 10px; \
         background: url(http://example.test/a/*b*/c.png) no-repeat; padding: 12px"
    );
    match &doc.tree.get(div.0).unwrap().computed_style.background {
        BackgroundValue::Image { url } => assert_eq!(
            url, "http://example.test/a/*b*/c.png",
            "stripping the `/*b*/` names a different image, and says nothing"
        ),
        other => panic!("the url did not survive re-serialisation: {other:?}"),
    }
}

/// An *unterminated* `/*` inside a url is the destructive half of the same
/// rule: an unterminated comment runs to the end of the string, so reading one
/// here would swallow every declaration after the url.
#[test]
fn an_unterminated_comment_marker_in_a_url_does_not_eat_the_rest() {
    let mut doc = RinchDocument::new();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "background-image: url(http://example.test/a/*b.png); color: red",
    );
    doc.set_style(div, "padding", "12px");
    assert_eq!(
        doc.get_attribute(div, "style").unwrap(),
        "background-image: url(http://example.test/a/*b.png); color: red; padding: 12px"
    );
}
