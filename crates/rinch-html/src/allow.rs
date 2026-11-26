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
//! DOM never reads, for the app to read back from the cleaned document.
//!
//! Only a carried value ever bears such a name. The rename is done on
//! ammonia's own output, after the cleaning, never before it: ammonia keeps
//! the attribute as itself on the elements named, and removes every
//! attribute the document wrote under a carried name, as it removes any
//! attribute it was not told to keep. Renaming before the cleaning would not
//! do: HTML does not always parse the same twice (a `<form>` in MathML can
//! turn text into elements on the second parse), so an attribute a first
//! parse never saw could reach ammonia's.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use html5ever::tendril::TendrilSink;
use html5ever::{LocalName, QualName, local_name, ns};

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
    /// Attributes carried through renamed, as `(element, attribute)`, `"*"`
    /// for the element meaning every kept element (see the module docs): on
    /// a kept element, `attribute` comes out as [`CARRIED`]`attribute`, its
    /// value as the document wrote it (escaped as every value is). It is
    /// never kept as itself.
    pub carried: Vec<(String, String)>,
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
        // A carried attribute is kept as itself through the cleaning, then
        // renamed ([`carry`]) before rinch's DOM sees it.
        for (tag, attribute) in self.carried_attributes() {
            if tag == "*" {
                generic.insert(attribute);
            } else if tags.contains(tag) {
                per_tag.entry(tag).or_default().insert(attribute);
            }
        }

        builder
            .tags(tags)
            .clean_content_tags(removed)
            .generic_attributes(generic)
            .tag_attributes(per_tag);
    }

    /// The carried attributes, as `(element, attribute)`, leaving out any
    /// named under the carried prefix itself (a name the cleaning must always
    /// remove).
    fn carried_attributes(&self) -> impl Iterator<Item = (&str, &str)> {
        self.carried
            .iter()
            .map(|(tag, attribute)| (tag.as_str(), attribute.as_str()))
            .filter(|(_, attribute)| !attribute.starts_with(CARRIED))
    }
}

/// `cleaned`, ammonia's output, with the carried attributes renamed: one
/// parse of it (as ammonia parsed its input, a fragment in a `div`) and one
/// walk over its elements. Borrowed when nothing is carried.
pub(crate) fn carry<'a>(cleaned: &'a str, allow: &AllowList) -> Cow<'a, str> {
    if allow.carried_attributes().next().is_none() {
        return Cow::Borrowed(cleaned);
    }
    let arena = html5ever::parse_fragment(
        Arena::default(),
        Default::default(),
        QualName::new(None, ns!(html), local_name!("div")),
        Vec::new(),
        false,
    )
    .one(cleaned);
    let mut nodes = arena.nodes.borrow_mut();
    for node in nodes.iter_mut() {
        let Kind::Element { name, attrs, .. } = &mut node.kind else {
            continue;
        };
        for (tag, attribute) in allow.carried_attributes() {
            if tag != "*" && tag != &*name.local {
                continue;
            }
            for found in attrs.iter_mut() {
                if found.name.ns == ns!() && &*found.name.local == attribute {
                    found.name = QualName::new(
                        None,
                        ns!(),
                        LocalName::from(format!("{CARRIED}{attribute}")),
                    );
                }
            }
        }
    }
    // A fragment is parsed into an `html` element of its own.
    let root = nodes[0].children.first().copied();
    drop(nodes);
    match root {
        Some(root) => Cow::Owned(crate::depth::write_children(&arena, root, cleaned.len())),
        None => Cow::Owned(String::new()),
    }
}
