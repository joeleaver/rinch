//! An app's narrower allow-list ([`Policy::allow`](crate::Policy::allow)):
//! for a document whose format fixes what it may contain, such as a
//! protocol's message bodies, which name the elements and attributes a
//! client keeps.
//!
//! It only ever narrows. An element is kept when rinch's own list keeps it
//! *and* the app's names it; an attribute likewise. So an app that names
//! `script`, `onclick` or a `data-*` attribute rinch's DOM reads (`data-rid`,
//! `data-oninput`…) gets none of them: the safety of rinch's list does not
//! depend on the app's. Removing an element with everything inside it is the
//! one thing an app adds, and removing never lets anything through.

use std::collections::{HashMap, HashSet};

/// The elements and attributes a document may keep, beyond which nothing
/// rinch's own list keeps survives.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AllowList {
    /// Elements kept (lower-case names). Any other element is removed and
    /// what is inside it kept, unless [`removed_with_content`] names it.
    ///
    /// [`removed_with_content`]: AllowList::removed_with_content
    pub tags: Vec<String>,
    /// Attributes kept, as `(element, attribute)`; `"*"` for the element
    /// keeps the attribute on every kept element. Every other attribute is
    /// removed. A kept `href` or `src`, and a kept `style`, still go through
    /// the [`Policy`](crate::Policy)'s link and picture rules and rinch's
    /// style filter.
    pub attributes: Vec<(String, String)>,
    /// Elements removed together with everything inside them. `script`,
    /// `style` and `title` always are.
    pub removed_with_content: Vec<String>,
}

/// What rinch removes with its content whatever the app says.
const ALWAYS_REMOVED: &[&str] = &["script", "style", "title"];

impl AllowList {
    /// Narrow `builder`, set up with rinch's own list, to this one. `marks`
    /// are attributes rinch adds itself (the user sheet's), kept on every
    /// element.
    pub(crate) fn narrow<'a>(&'a self, builder: &mut ammonia::Builder<'a>, marks: &[&'a str]) {
        let removed: HashSet<&str> = ALWAYS_REMOVED
            .iter()
            .copied()
            .chain(self.removed_with_content.iter().map(String::as_str))
            .collect();

        let rinch_tags = builder.clone_tags();
        let tags: HashSet<&str> = self
            .tags
            .iter()
            .map(String::as_str)
            .filter(|t| rinch_tags.contains(t) && !removed.contains(t))
            .collect();

        let rinch_generic = builder.clone_generic_attributes();
        let rinch_tag_attributes = builder.clone_tag_attributes();
        let rinch_keeps = |tag: &str, attribute: &str| {
            rinch_generic.contains(attribute)
                || rinch_tag_attributes
                    .get(tag)
                    .is_some_and(|kept| kept.contains(attribute))
        };
        let mut generic: HashSet<&str> = marks.iter().copied().collect();
        let mut per_tag: HashMap<&str, HashSet<&str>> = HashMap::new();
        for (tag, attribute) in &self.attributes {
            let (tag, attribute) = (tag.as_str(), attribute.as_str());
            if tag == "*" {
                if rinch_generic.contains(attribute) {
                    generic.insert(attribute);
                }
            } else if tags.contains(tag) && rinch_keeps(tag, attribute) {
                per_tag.entry(tag).or_default().insert(attribute);
            }
        }

        builder
            .tags(tags)
            .clean_content_tags(removed)
            .generic_attributes(generic)
            .tag_attributes(per_tag);
    }
}
