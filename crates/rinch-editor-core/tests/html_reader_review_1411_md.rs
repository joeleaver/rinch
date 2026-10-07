//! From the review of PR #1411: the strict markdown table guard.
#![cfg(feature = "markdown")]
use rinch_editor_core::Schema;
use rinch_editor_core::serialize::{doc_from_markdown, doc_from_markdown_strict, node_to_html};

/// Kills M17b (`|| (self.strict && !tables.iter().all(is_table))` removed),
/// which the PR reports as surviving because "the strict text check behind it
/// refuses the same input". It does not when the stray content is text: the
/// text check compares all the text read, table or not.
#[test]
fn pin_strict_markdown_refuses_a_table_with_text_outside_its_cells() {
    let schema = Schema::starter_kit();
    for md in [
        "<table>loose<tr><td>a</td></tr></table>",
        "<table><tr>loose<td>a</td></tr></table>",
        "<table><tr><td>a</td></tr><tr><td>b</td>loose</tr></table>",
    ] {
        let lenient = doc_from_markdown(&schema, md).unwrap();
        assert!(node_to_html(&lenient).contains("loose"), "{md}");
        assert!(doc_from_markdown_strict(&schema, md).is_err(), "{md}");
    }
}
