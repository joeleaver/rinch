//! Character references of the HTML reader ([`super::html`]): the HTML
//! standard's, decoded by [`rinch_core::html_entities`] (#1415), each `&`
//! looked at counted as a reading step ([`super::html_tree::step`]).

/// `s`, the text of an element, with its character references replaced, in
/// one pass.
pub(super) fn decode_entities(s: &str) -> String {
    rinch_core::html_entities::decode(s, false, super::html_tree::step)
}

/// `s`, an attribute's value, with its character references replaced.
pub(super) fn decode_attribute(s: &str) -> String {
    rinch_core::html_entities::decode(s, true, super::html_tree::step)
}
