//! Typed node/mark attributes.
//!
//! The old engine stored attributes as a stringly-typed `HashMap<String, String>`
//! (see the audit's attr hazard and amendment A11): a `heading.level` of `"1"`,
//! a `task.checked` of `"false"`. That made every consumer re-parse and made
//! round-trips lossy. Here attributes are **typed** ([`AttrValue`]) and stored in
//! an ordered, structurally-shared map ([`Attrs`]) so that:
//!
//! - serialization is deterministic (BTreeMap key order is stable),
//! - cloning is cheap (an `Rc` bump — attrs ride along in the persistent tree),
//! - the empty case allocates nothing (`Attrs(None)`).

use std::collections::BTreeMap;
use std::rc::Rc;

/// A single attribute value. Deliberately small and `Hash`/`Eq` so that [`Attrs`]
/// — and therefore `Node`/`Mark` — can derive structural equality and hashing.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AttrValue {
    /// Explicit absence (distinct from "attribute not present").
    Null,
    /// Boolean, e.g. `task_item.checked`.
    Bool(bool),
    /// Integer, e.g. `heading.level`, `ordered_list.start`.
    Int(i64),
    /// String, e.g. `link.href`, `text_color.color`, `image.src`.
    Str(Box<str>),
}

impl AttrValue {
    /// Borrow as a string slice, if this is a [`AttrValue::Str`].
    pub fn as_str(&self) -> Option<&str> {
        match self {
            AttrValue::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Read as an integer, if this is an [`AttrValue::Int`].
    pub fn as_int(&self) -> Option<i64> {
        match self {
            AttrValue::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// Read as a boolean, if this is an [`AttrValue::Bool`].
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            AttrValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// True for [`AttrValue::Null`].
    pub fn is_null(&self) -> bool {
        matches!(self, AttrValue::Null)
    }
}

impl From<&str> for AttrValue {
    fn from(s: &str) -> Self {
        AttrValue::Str(s.into())
    }
}
impl From<String> for AttrValue {
    fn from(s: String) -> Self {
        AttrValue::Str(s.into_boxed_str())
    }
}
impl From<bool> for AttrValue {
    fn from(b: bool) -> Self {
        AttrValue::Bool(b)
    }
}
impl From<i64> for AttrValue {
    fn from(i: i64) -> Self {
        AttrValue::Int(i)
    }
}

/// An ordered, structurally-shared attribute map.
///
/// `None` represents the empty map and allocates nothing — the overwhelmingly
/// common case (most text nodes and many block nodes have no attrs). When
/// non-empty, the inner `BTreeMap` is shared via `Rc`, so cloning an `Attrs`
/// (and therefore a `Node`) is a refcount bump.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Attrs(Option<Rc<BTreeMap<Box<str>, AttrValue>>>);

impl Attrs {
    /// The empty attribute map.
    pub const fn new() -> Self {
        Attrs(None)
    }

    /// True if there are no attributes.
    pub fn is_empty(&self) -> bool {
        self.0.as_ref().is_none_or(|m| m.is_empty())
    }

    /// Number of attributes.
    pub fn len(&self) -> usize {
        self.0.as_ref().map_or(0, |m| m.len())
    }

    /// Look up an attribute by key.
    pub fn get(&self, key: &str) -> Option<&AttrValue> {
        self.0.as_ref().and_then(|m| m.get(key))
    }

    /// Convenience: the value at `key` as a `&str`.
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(AttrValue::as_str)
    }

    /// Convenience: the value at `key` as an `i64`.
    pub fn get_int(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(AttrValue::as_int)
    }

    /// Convenience: the value at `key` as a `bool`.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(AttrValue::as_bool)
    }

    /// Iterate attributes in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &AttrValue)> {
        self.0
            .iter()
            .flat_map(|m| m.iter().map(|(k, v)| (k.as_ref(), v)))
    }

    /// Return a new `Attrs` with `key` set to `value` (copy-on-write).
    pub fn with(&self, key: impl Into<Box<str>>, value: impl Into<AttrValue>) -> Self {
        let mut map = match &self.0 {
            Some(rc) => (**rc).clone(),
            None => BTreeMap::new(),
        };
        map.insert(key.into(), value.into());
        Attrs(Some(Rc::new(map)))
    }

    /// Return a new `Attrs` with `key` removed (copy-on-write).
    pub fn without(&self, key: &str) -> Self {
        match &self.0 {
            Some(rc) if rc.contains_key(key) => {
                let mut map = (**rc).clone();
                map.remove(key);
                if map.is_empty() {
                    Attrs(None)
                } else {
                    Attrs(Some(Rc::new(map)))
                }
            }
            _ => self.clone(),
        }
    }
}

impl<K, V> FromIterator<(K, V)> for Attrs
where
    K: Into<Box<str>>,
    V: Into<AttrValue>,
{
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let map: BTreeMap<Box<str>, AttrValue> = iter
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        if map.is_empty() {
            Attrs(None)
        } else {
            Attrs(Some(Rc::new(map)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_attrs_allocate_nothing() {
        let a = Attrs::new();
        assert!(a.is_empty());
        assert_eq!(a.len(), 0);
        assert_eq!(a.get("anything"), None);
        assert_eq!(a, Attrs::default());
    }

    #[test]
    fn typed_accessors() {
        let a = Attrs::from_iter([
            ("level", AttrValue::Int(2)),
            ("checked", AttrValue::Bool(true)),
            ("href", AttrValue::from("https://example.com")),
        ]);
        assert_eq!(a.get_int("level"), Some(2));
        assert_eq!(a.get_bool("checked"), Some(true));
        assert_eq!(a.get_str("href"), Some("https://example.com"));
        // wrong-type access returns None rather than a bogus parse
        assert_eq!(a.get_int("href"), None);
        assert_eq!(a.get_str("level"), None);
    }

    #[test]
    fn with_is_copy_on_write() {
        let a = Attrs::from_iter([("level", AttrValue::Int(1))]);
        let b = a.with("level", AttrValue::Int(3));
        // original is untouched (persistent/value semantics)
        assert_eq!(a.get_int("level"), Some(1));
        assert_eq!(b.get_int("level"), Some(3));
    }

    #[test]
    fn without_collapses_to_empty() {
        let a = Attrs::from_iter([("k", AttrValue::from("v"))]);
        let b = a.without("k");
        assert!(b.is_empty());
        // removing an absent key is a no-op clone
        assert_eq!(a.without("absent"), a);
    }

    #[test]
    fn iteration_is_key_ordered_and_deterministic() {
        let a = Attrs::from_iter([
            ("z", AttrValue::Int(1)),
            ("a", AttrValue::Int(2)),
            ("m", AttrValue::Int(3)),
        ]);
        let keys: Vec<&str> = a.iter().map(|(k, _)| k).collect();
        assert_eq!(keys, ["a", "m", "z"]); // BTreeMap order — stable for serde
    }

    #[test]
    fn equality_and_hash_are_structural() {
        use std::collections::HashSet;
        let a = Attrs::from_iter([("x", AttrValue::Int(1)), ("y", AttrValue::Bool(false))]);
        let b = Attrs::from_iter([("y", AttrValue::Bool(false)), ("x", AttrValue::Int(1))]);
        assert_eq!(a, b); // insertion order doesn't matter
        let mut set = HashSet::new();
        set.insert(a);
        assert!(set.contains(&b));
    }

    #[test]
    fn null_distinct_from_absent() {
        let a = Attrs::from_iter([("color", AttrValue::Null)]);
        assert_eq!(a.get("color"), Some(&AttrValue::Null));
        assert_eq!(a.get("missing"), None);
        assert!(a.get("color").unwrap().is_null());
    }
}
