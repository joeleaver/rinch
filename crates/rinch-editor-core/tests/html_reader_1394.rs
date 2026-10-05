//! The HTML reader on what browsers and Google Docs really write (review of
//! PR #1394): lists inside lists, stray list children, and marks around
//! blocks. `load_html` reads through the same code, so these hold for a
//! loaded document as for a paste.
use rinch_editor_core::Schema;
use rinch_editor_core::serialize::{slice_from_html, slice_to_html};

fn read(html: &str) -> String {
    let schema = Schema::starter_kit();
    slice_to_html(&slice_from_html(&schema, html).unwrap())
}

/// Google Docs writes a nested list as a sibling of the item it belongs to.
/// It goes under that item; a list that comes first gets an item of its own.
#[test]
fn a_list_directly_inside_a_list_nests_under_the_item_before_it() {
    assert_eq!(
        read("<ul><li>one</li><ul><li>nested</li></ul><li>two</li></ul>"),
        "<ul><li><p>one</p><ul><li><p>nested</p></li></ul></li><li><p>two</p></li></ul>"
    );
    assert_eq!(
        read("<ol><li>one</li><ul><li>n1</li></ul><ol><li>n2</li></ol></ol>"),
        "<ol><li><p>one</p><ul><li><p>n1</p></li></ul><ol><li><p>n2</p></li></ol></li></ol>"
    );
    assert_eq!(
        read("<ul><ul><li>first</li></ul><li>two</li></ul>"),
        "<ul><li><ul><li><p>first</p></li></ul></li><li><p>two</p></li></ul>"
    );
    // In a task list too, and the item keeps its checkbox.
    assert_eq!(
        read(
            "<ul data-type=\"taskList\"><li data-type=\"taskItem\" data-checked=\"true\">a</li>\
             <ul><li>n</li></ul></ul>"
        ),
        "<ul data-type=\"taskList\"><li data-type=\"taskItem\" data-checked=\"true\"><p>a</p>\
         <ul><li><p>n</p></li></ul></li></ul>"
    );
}

/// Nothing in a list is dropped for not being an `<li>`: other content gets
/// an item of its own. Whitespace and dropped elements make no item.
#[test]
fn stray_list_children_keep_their_content() {
    assert_eq!(
        read("<ol><li>a</li>stray<p>para</p>\n<script>x</script><li>b</li></ol>"),
        "<ol><li><p>a</p></li><li><p>stray</p></li><li><p>para</p></li><li><p>b</p></li></ol>"
    );
    assert_eq!(
        read("<ul>\n  <li>a</li>\n  <li>b</li>\n</ul>"),
        "<ul><li><p>a</p></li><li><p>b</p></li></ul>"
    );
}

/// A mark element around blocks: the blocks stay blocks and the mark goes on
/// the inline content inside them, where the block allows it and no mark of
/// that type is there already.
#[test]
fn a_mark_around_blocks_goes_on_their_inline_content() {
    assert_eq!(
        read("<a href=\"https://e.x/post\"><h3>Card title</h3><p>summary</p></a>"),
        "<h3><a href=\"https://e.x/post\">Card title</a></h3>\
         <p><a href=\"https://e.x/post\">summary</a></p>"
    );
    assert_eq!(
        read("<strong><p>x</p><p>y</p></strong>"),
        "<p><strong>x</strong></p><p><strong>y</strong></p>"
    );
    assert_eq!(read("<i>a<hr></i>"), "<p><em>a</em></p><hr>");
    // Through any depth of structure, and through a neutral container.
    assert_eq!(
        read("<em><ul><li>a</li></ul></em>"),
        "<ul><li><p><em>a</em></p></li></ul>"
    );
    assert_eq!(
        read("<b><div><p>x</p></div></b>"),
        "<p><strong>x</strong></p>"
    );
    // An inner link keeps its own href; a code block takes no mark.
    assert_eq!(
        read(
            "<a href=\"https://o.x/\"><p>t <a href=\"https://i.x/\">in</a></p><pre>code</pre></a>"
        ),
        "<p><a href=\"https://o.x/\">t </a><a href=\"https://i.x/\">in</a></p><pre>code</pre>"
    );
}

/// A wrapper that says it is not bold carries nothing: Google Docs'
/// `<b style="font-weight:normal">`, around blocks or around text. Nor does a
/// `<span>` around blocks.
#[test]
fn a_neutralised_wrapper_carries_no_mark() {
    assert_eq!(
        read("<b style=\"font-weight:normal;\" id=\"docs-internal-guid-1\"><p>x</p><p>y</p></b>"),
        "<p>x</p><p>y</p>"
    );
    // The blocks may be any depth down in the wrapper.
    assert_eq!(
        read("<b style=\"font-weight:normal\"><div><p>x</p><p>y</p></div></b>"),
        "<p>x</p><p>y</p>"
    );
    assert_eq!(read("<b style=\"font-weight: 400\">t</b>"), "<p>t</p>");
    assert_eq!(
        read("<strong style=\"FONT-WEIGHT: Normal\">t</strong>"),
        "<p>t</p>"
    );
    assert_eq!(
        read("<b style=\"font-weight:700\">t</b>"),
        "<p><strong>t</strong></p>"
    );
    assert_eq!(read("<b>t</b>"), "<p><strong>t</strong></p>");
    assert_eq!(
        read("<span style=\"color:red\"><p>x</p></span>"),
        "<p>x</p>"
    );
}
