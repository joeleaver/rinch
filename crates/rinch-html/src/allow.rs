//! An app's narrower allow-list ([`Policy::allow`](crate::Policy::allow)):
//! for a document whose format fixes what it may contain, such as a
//! protocol's message bodies, which name the elements and attributes a
//! client keeps.
//!
//! It only ever narrows. An element is kept when rinch's own list keeps it
//! *and* the app's names it; an attribute likewise. So an app that names
//! `script`, `onclick` or a `data-*` attribute rinch's DOM reads (`data-rid`,
//! `data-oninput`…) gets none of them: the safety of rinch's list does not
//! depend on the app's. Removing an element with everything inside it is
//! one thing an app adds, and removing never lets anything through.
//!
//! The other is **carrying** an attribute the format gives a meaning to
//! (OFSCP's `<img data-attachment="<id>">`) without letting it reach the DOM
//! as itself: it comes out renamed, [`CARRIED`]`<attribute>`, a name rinch's
//! DOM never reads, for the app to read back from the cleaned document. Any
//! attribute the document wrote under such a name is removed first, so only
//! a carried value ever bears one.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use html5ever::tendril::TendrilSink;
use html5ever::{Attribute, LocalName, QualName, ns};

use crate::depth::{Arena, Kind};

/// The prefix a carried attribute comes out under:
/// `data-attachment` on an `img` comes out as
/// `data-rinch-carried-data-attachment`.
pub const CARRIED: &str = "data-rinch-carried-";

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
    /// Attributes carried through renamed, as `(element, attribute)` (see
    /// the module docs): on a kept element, `attribute` comes out as
    /// [`CARRIED`]`attribute`, its value as the document wrote it (escaped as
    /// every value is). It is never kept as itself.
    pub carried: Vec<(String, String)>,
}

/// What rinch removes with its content whatever the app says.
const ALWAYS_REMOVED: &[&str] = &["script", "style", "title"];

impl AllowList {
    /// Narrow `builder`, set up with rinch's own list, to this one. `marks`
    /// are attributes rinch adds itself (the user sheet's), kept on every
    /// element.
    pub(crate) fn narrow<'a>(
        &'a self,
        builder: &mut ammonia::Builder<'a>,
        marks: &[&'a str],
        carried_names: &'a [String],
    ) {
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
        generic.extend(carried_names.iter().map(String::as_str));
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

/// The names the carried attributes come out under.
pub(crate) fn carried_names(allow: &AllowList) -> Vec<String> {
    let mut names: Vec<String> = allow
        .carried
        .iter()
        .map(|(_, attribute)| format!("{CARRIED}{attribute}"))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// `document` (already cut to depth) with the carried attributes renamed
/// and any it wrote under a carried name removed: one walk over its parsed
/// elements. Borrowed when nothing is carried.
pub(crate) fn carry<'a>(document: &'a str, allow: &AllowList) -> Cow<'a, str> {
    if allow.carried.is_empty() {
        return Cow::Borrowed(document);
    }
    let arena = html5ever::parse_document(Arena::default(), Default::default()).one(document);
    let mut nodes = arena.nodes.borrow_mut();
    for node in nodes.iter_mut() {
        let Kind::Element { name, attrs, .. } = &mut node.kind else {
            continue;
        };
        attrs.retain(|a| !a.name.local.starts_with(CARRIED));
        let mut renamed = Vec::new();
        for (tag, attribute) in &allow.carried {
            if tag != "*" && *tag != *name.local {
                continue;
            }
            if let Some(found) = attrs.iter().find(|a| *a.name.local == **attribute) {
                renamed.push(Attribute {
                    name: QualName::new(
                        None,
                        ns!(),
                        LocalName::from(format!("{CARRIED}{attribute}")),
                    ),
                    value: found.value.clone(),
                });
            }
        }
        attrs.extend(renamed);
    }
    drop(nodes);
    Cow::Owned(crate::depth::write(&arena, document.len()))
}
