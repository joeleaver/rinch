//! `set_inner_html` decodes character references as the HTML standard does
//! (#2239): `&#8203;` was left as literal text, because the fragment parser
//! knew six named references and no numeric one, and decoded those six one
//! after another (`&amp;lt;` became `<`). The cases are shared with the
//! browser test (`support/char_refs_2239.rs`), where Chrome's and Firefox's
//! own parsers answer.

#[path = "support/char_refs_2239.rs"]
mod char_refs;

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

#[test]
fn inner_html_decodes_character_references_as_a_browser_does() {
    let mut d = RinchDocument::new();
    let body = d.body();
    let host = d.create_element("div");
    d.append_child(body, host);
    char_refs::check(&mut d, host);
}

/// Desktop reads the legacy cases the same with or without the `<svg>`
/// that keeps Chrome off its fast `innerHTML` path.
#[test]
fn a_legacy_name_before_an_alphanumeric_in_an_attribute_is_left_alone() {
    let mut d = RinchDocument::new();
    let body = d.body();
    let host = d.create_element("div");
    d.append_child(body, host);
    for &(value, want) in char_refs::LEGACY_IN_ATTRIBUTES {
        d.set_inner_html(host, &char_refs::legacy_markup(value, true));
        let span = d.get_children(host)[0];
        assert_eq!(d.get_attribute(span, "title").as_deref(), Some(want));
    }
}
