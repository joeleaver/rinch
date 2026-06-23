//! The durable, schema-derived JSON wire shape (`DocNode`) — feature `serde`.
//!
//! This replaces the old flat `BlockData`/`InlineRunData`/`InlineMarkData`
//! interchange (which could not express lists, tables, or atoms, and dropped
//! marks — #59). The shape is **recursive** (nesting works), **typed**
//! (attributes carry their `AttrValue` kind, not stringly values), and **total**:
//!
//! - **Serialize** ([`Node::to_doc`]) walks the schema, applies attribute
//!   defaults, and would fail (not silently emit garbage) on a node missing a
//!   required attribute.
//! - **Deserialize** ([`Schema::node_from_doc`]) consults the schema: an unknown
//!   node or mark type is a hard [`EditorError::SchemaValidation`], never a silent
//!   drop, and a missing required attribute is an error. The boundary either
//!   round-trips a thing faithfully or rejects it.
//!
//! The snake_case key conventions (`type`, `attrs`, `content`, `text`, `marks`)
//! are kept for familiarity and wire stability.

use crate::EditorError;
use crate::model::{AttrValue, Attrs, Fragment, Mark, Node};
use crate::schema::Schema;
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use std::fmt;

/// A single attribute value on the wire. A thin, serde-friendly mirror of
/// [`AttrValue`] (uses `String` rather than `Box<str>`), kept distinct from the
/// model type so the durable wire format is decoupled from the in-memory model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JsonAttr {
    /// JSON `null` — explicit absence (distinct from "key not present").
    Null,
    /// JSON boolean.
    Bool(bool),
    /// JSON integer.
    Int(i64),
    /// JSON string.
    Str(String),
}

impl From<&AttrValue> for JsonAttr {
    fn from(v: &AttrValue) -> Self {
        match v {
            AttrValue::Null => JsonAttr::Null,
            AttrValue::Bool(b) => JsonAttr::Bool(*b),
            AttrValue::Int(i) => JsonAttr::Int(*i),
            AttrValue::Str(s) => JsonAttr::Str(s.to_string()),
        }
    }
}

impl From<&JsonAttr> for AttrValue {
    fn from(v: &JsonAttr) -> Self {
        match v {
            JsonAttr::Null => AttrValue::Null,
            JsonAttr::Bool(b) => AttrValue::Bool(*b),
            JsonAttr::Int(i) => AttrValue::Int(*i),
            JsonAttr::Str(s) => AttrValue::Str(s.as_str().into()),
        }
    }
}

// Manual serde so a `JsonAttr` serializes as a *bare* JSON scalar (`null`, `true`,
// `42`, `"x"`) rather than a tagged enum — the natural, stable wire form — and so
// the crate needs no `serde_json` dependency (we use `deserialize_any`, which all
// self-describing formats including JSON support).
impl Serialize for JsonAttr {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            JsonAttr::Null => serializer.serialize_unit(),
            JsonAttr::Bool(b) => serializer.serialize_bool(*b),
            JsonAttr::Int(i) => serializer.serialize_i64(*i),
            JsonAttr::Str(s) => serializer.serialize_str(s),
        }
    }
}

impl<'de> Deserialize<'de> for JsonAttr {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct AttrVisitor;
        impl<'de> Visitor<'de> for AttrVisitor {
            type Value = JsonAttr;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a null, boolean, integer, or string attribute value")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<JsonAttr, E> {
                Ok(JsonAttr::Bool(v))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<JsonAttr, E> {
                Ok(JsonAttr::Int(v))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<JsonAttr, E> {
                i64::try_from(v)
                    .map(JsonAttr::Int)
                    .map_err(|_| E::custom("integer attribute value out of i64 range"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<JsonAttr, E> {
                Ok(JsonAttr::Str(v.to_string()))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<JsonAttr, E> {
                Ok(JsonAttr::Str(v))
            }
            fn visit_unit<E: de::Error>(self) -> Result<JsonAttr, E> {
                Ok(JsonAttr::Null)
            }
            fn visit_none<E: de::Error>(self) -> Result<JsonAttr, E> {
                Ok(JsonAttr::Null)
            }
        }
        deserializer.deserialize_any(AttrVisitor)
    }
}

/// The durable wire shape for a document node. Recursive, so nesting (lists,
/// blockquotes, tables) is expressible; total, so every node carries its schema
/// type name and (defaults-applied) typed attributes.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DocNode {
    /// The schema node-type name (`paragraph`, `heading`, `text`, …). Always present.
    #[serde(rename = "type")]
    pub node_type: String,
    /// Typed attributes, defaults applied. Omitted from JSON when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: BTreeMap<String, JsonAttr>,
    /// Child nodes (recursive). Omitted from JSON when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<DocNode>,
    /// The string of a text node. Omitted for non-text nodes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Marks on a text node or inline leaf. Omitted from JSON when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<DocMark>,
}

/// The durable wire shape for a mark.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DocMark {
    /// The schema mark-type name (`bold`, `link`, …).
    #[serde(rename = "type")]
    pub mark_type: String,
    /// Typed mark attributes, defaults applied. Omitted from JSON when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: BTreeMap<String, JsonAttr>,
}

impl Node {
    /// Serialize this node (and its subtree) to the durable [`DocNode`] wire shape.
    ///
    /// Attribute defaults are applied so the wire is total. Returns
    /// [`EditorError::SchemaValidation`] if any node is missing a required
    /// attribute (such a node was never a valid document and must not be silently
    /// persisted).
    pub fn to_doc(&self) -> Result<DocNode, EditorError> {
        doc_from_node(self)
    }
}

impl Schema {
    /// Deserialize a [`DocNode`] into a model [`Node`], validated against this
    /// schema. Unknown node/mark types and missing required attributes are hard
    /// errors — the structural fix for #59.
    pub fn node_from_doc(&self, doc: &DocNode) -> Result<Node, EditorError> {
        node_from_doc_node(self, doc)
    }
}

fn attrs_to_wire(attrs: &Attrs) -> BTreeMap<String, JsonAttr> {
    attrs
        .iter()
        .map(|(k, v)| (k.to_string(), JsonAttr::from(v)))
        .collect()
}

fn wire_to_attrs(wire: &BTreeMap<String, JsonAttr>) -> Attrs {
    wire.iter()
        .map(|(k, v)| (k.clone(), AttrValue::from(v)))
        .collect()
}

fn doc_from_node(node: &Node) -> Result<DocNode, EditorError> {
    let marks = node
        .marks()
        .iter()
        .map(doc_from_mark)
        .collect::<Result<Vec<_>, _>>()?;

    if let Some(text) = node.text() {
        return Ok(DocNode {
            node_type: node.type_name().to_string(),
            attrs: BTreeMap::new(),
            content: Vec::new(),
            text: Some(text.to_string()),
            marks,
        });
    }

    let attrs = node.node_type().compute_attrs(node.attrs())?;
    let content = node
        .content()
        .children()
        .iter()
        .map(doc_from_node)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(DocNode {
        node_type: node.type_name().to_string(),
        attrs: attrs_to_wire(&attrs),
        content,
        text: None,
        marks,
    })
}

fn doc_from_mark(mark: &Mark) -> Result<DocMark, EditorError> {
    let attrs = mark.typ.compute_attrs(&mark.attrs)?;
    Ok(DocMark {
        mark_type: mark.type_name().to_string(),
        attrs: attrs_to_wire(&attrs),
    })
}

fn node_from_doc_node(schema: &Schema, doc: &DocNode) -> Result<Node, EditorError> {
    if let Some(text) = &doc.text {
        let nt = schema.node_type(&doc.node_type).ok_or_else(|| {
            EditorError::SchemaValidation(format!("unknown node type '{}'", doc.node_type))
        })?;
        if !nt.is_text() {
            return Err(EditorError::SchemaValidation(format!(
                "node '{}' carries text but is not a text node type",
                doc.node_type
            )));
        }
        // A text node may not also carry content or attrs — reject rather than
        // silently discard them (totality; the #59 contract for the durable path).
        if !doc.content.is_empty() {
            return Err(EditorError::SchemaValidation(format!(
                "text node '{}' must not carry content",
                doc.node_type
            )));
        }
        if !doc.attrs.is_empty() {
            return Err(EditorError::SchemaValidation(format!(
                "text node '{}' must not carry attributes",
                doc.node_type
            )));
        }
        let marks = doc
            .marks
            .iter()
            .map(|m| mark_from_doc_mark(schema, m))
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(Node::new_text(nt.clone(), text.as_str().into(), marks));
    }

    let nt = schema
        .node_type(&doc.node_type)
        .ok_or_else(|| {
            EditorError::SchemaValidation(format!("unknown node type '{}'", doc.node_type))
        })?
        .clone();
    let attrs = nt.compute_attrs(&wire_to_attrs(&doc.attrs))?;
    let children = doc
        .content
        .iter()
        .map(|c| node_from_doc_node(schema, c))
        .collect::<Result<Vec<_>, _>>()?;
    let marks = doc
        .marks
        .iter()
        .map(|m| mark_from_doc_mark(schema, m))
        .collect::<Result<Vec<_>, _>>()?;
    let node = Node::new_branch(nt, attrs, Fragment::from_children(children));
    Ok(if marks.is_empty() {
        node
    } else {
        node.mark(marks)
    })
}

fn mark_from_doc_mark(schema: &Schema, doc: &DocMark) -> Result<Mark, EditorError> {
    let mt = schema
        .mark_type(&doc.mark_type)
        .ok_or_else(|| {
            EditorError::SchemaValidation(format!("unknown mark type '{}'", doc.mark_type))
        })?
        .clone();
    let attrs = mt.compute_attrs(&wire_to_attrs(&doc.attrs))?;
    Ok(Mark::new(mt, attrs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Schema;

    /// Build a normalized (defaults-applied) non-text node.
    fn node(s: &Schema, ty: &str, attrs: Attrs, content: Fragment) -> Node {
        let nt = s.node_type(ty).unwrap().clone();
        let attrs = nt.compute_attrs(&attrs).unwrap();
        Node::new_branch(nt, attrs, content)
    }

    /// Build a normalized mark.
    fn mark(s: &Schema, ty: &str, attrs: Attrs) -> Mark {
        let mt = s.mark_type(ty).unwrap().clone();
        let attrs = mt.compute_attrs(&attrs).unwrap();
        Mark::new(mt, attrs)
    }

    fn attrs(pairs: &[(&str, AttrValue)]) -> Attrs {
        Attrs::from_iter(pairs.iter().map(|(k, v)| (*k, v.clone())))
    }

    /// A rich document touching every starter-kit node and mark with attrs.
    fn rich_doc(s: &Schema) -> Node {
        let frag = Fragment::from_children;

        let heading = node(
            s,
            "heading",
            attrs(&[("level", AttrValue::Int(2))]),
            frag(vec![s.text("Title").unwrap()]),
        );

        let para = node(
            s,
            "paragraph",
            Attrs::new(),
            frag(vec![
                s.text("plain ").unwrap(),
                s.text_with_marks("bold", vec![mark(s, "bold", Attrs::new())])
                    .unwrap(),
                s.text(" ").unwrap(),
                s.text_with_marks(
                    "linked",
                    vec![mark(
                        s,
                        "link",
                        attrs(&[("href", AttrValue::from("https://example.com"))]),
                    )],
                )
                .unwrap(),
                s.text_with_marks(
                    "hi",
                    vec![mark(
                        s,
                        "highlight",
                        attrs(&[("color", AttrValue::from("yellow"))]),
                    )],
                )
                .unwrap(),
                s.text_with_marks(
                    "colored",
                    vec![mark(
                        s,
                        "text_color",
                        attrs(&[("color", AttrValue::from("#f00"))]),
                    )],
                )
                .unwrap(),
                node(
                    s,
                    "image",
                    attrs(&[("src", AttrValue::from("a.png"))]),
                    Fragment::empty(),
                ),
                node(s, "hard_break", Attrs::new(), Fragment::empty()),
            ]),
        );

        let bq = node(
            s,
            "blockquote",
            Attrs::new(),
            frag(vec![node(
                s,
                "paragraph",
                Attrs::new(),
                frag(vec![s.text("quote").unwrap()]),
            )]),
        );

        let list = node(
            s,
            "bullet_list",
            Attrs::new(),
            frag(vec![node(
                s,
                "list_item",
                Attrs::new(),
                frag(vec![node(
                    s,
                    "paragraph",
                    Attrs::new(),
                    frag(vec![s.text("item").unwrap()]),
                )]),
            )]),
        );

        let code = node(
            s,
            "code_block",
            attrs(&[("language", AttrValue::from("rust"))]),
            frag(vec![s.text("fn main() {}").unwrap()]),
        );

        let hr = node(s, "horizontal_rule", Attrs::new(), Fragment::empty());

        node(
            s,
            "doc",
            Attrs::new(),
            frag(vec![heading, para, bq, list, code, hr]),
        )
    }

    #[test]
    fn round_trip_identity_for_rich_doc() {
        let s = Schema::starter_kit();
        let doc = rich_doc(&s);
        let wire = doc.to_doc().unwrap();
        let back = s.node_from_doc(&wire).unwrap();
        assert_eq!(doc, back, "Node -> DocNode -> Node must be identity");
    }

    #[test]
    fn round_trip_through_json_string() {
        let s = Schema::starter_kit();
        let doc = rich_doc(&s);
        let wire = doc.to_doc().unwrap();
        let json = serde_json::to_string(&wire).unwrap();
        let parsed: DocNode = serde_json::from_str(&json).unwrap();
        assert_eq!(wire, parsed);
        let back = s.node_from_doc(&parsed).unwrap();
        assert_eq!(doc, back);
    }

    #[test]
    fn wire_uses_snake_case_and_type_key() {
        let s = Schema::starter_kit();
        let heading = node(
            &s,
            "heading",
            attrs(&[("level", AttrValue::Int(2))]),
            Fragment::from_node(s.text("Hi").unwrap()),
        );
        let json = serde_json::to_string(&heading.to_doc().unwrap()).unwrap();
        // `type` key (not `node_type`), `level` as a bare number, snake_case text key.
        assert!(json.contains(r#""type":"heading""#), "json: {json}");
        assert!(json.contains(r#""level":2"#), "json: {json}");
        assert!(json.contains(r#""type":"text""#), "json: {json}");
        assert!(json.contains(r#""text":"Hi""#), "json: {json}");
    }

    #[test]
    fn unknown_node_type_is_error_not_silent_drop() {
        let s = Schema::starter_kit();
        let wire = DocNode {
            node_type: "marquee".to_string(),
            attrs: BTreeMap::new(),
            content: vec![],
            text: None,
            marks: vec![],
        };
        assert!(s.node_from_doc(&wire).is_err());
    }

    #[test]
    fn unknown_mark_type_is_error() {
        let s = Schema::starter_kit();
        let wire = DocNode {
            node_type: "text".to_string(),
            attrs: BTreeMap::new(),
            content: vec![],
            text: Some("x".to_string()),
            marks: vec![DocMark {
                mark_type: "blink".to_string(),
                attrs: BTreeMap::new(),
            }],
        };
        assert!(s.node_from_doc(&wire).is_err());
    }

    #[test]
    fn missing_required_attr_is_error() {
        let s = Schema::starter_kit();
        // image with no `src` (required)
        let wire = DocNode {
            node_type: "image".to_string(),
            attrs: BTreeMap::new(),
            content: vec![],
            text: None,
            marks: vec![],
        };
        assert!(s.node_from_doc(&wire).is_err());
    }

    #[test]
    fn deserialize_fills_optional_defaults() {
        let s = Schema::starter_kit();
        // heading with no `level` on the wire → default Int(1) on the node
        let wire = DocNode {
            node_type: "heading".to_string(),
            attrs: BTreeMap::new(),
            content: vec![],
            text: None,
            marks: vec![],
        };
        let node = s.node_from_doc(&wire).unwrap();
        assert_eq!(node.attrs().get_int("level"), Some(1));
    }

    #[test]
    fn text_field_on_non_text_type_is_error() {
        let s = Schema::starter_kit();
        let wire = DocNode {
            node_type: "paragraph".to_string(),
            attrs: BTreeMap::new(),
            content: vec![],
            text: Some("oops".to_string()),
            marks: vec![],
        };
        assert!(s.node_from_doc(&wire).is_err());
    }

    #[test]
    fn text_node_with_content_is_error() {
        let s = Schema::starter_kit();
        let wire = DocNode {
            node_type: "text".to_string(),
            attrs: BTreeMap::new(),
            content: vec![DocNode {
                node_type: "text".to_string(),
                attrs: BTreeMap::new(),
                content: vec![],
                text: Some("inner".to_string()),
                marks: vec![],
            }],
            text: Some("outer".to_string()),
            marks: vec![],
        };
        // Must reject, not silently drop the `inner` child (the #59 totality contract).
        assert!(s.node_from_doc(&wire).is_err());
    }

    #[test]
    fn text_node_with_attrs_is_error() {
        let s = Schema::starter_kit();
        let wire = DocNode {
            node_type: "text".to_string(),
            attrs: attrs_to_wire(&Attrs::from_iter([("level", AttrValue::Int(9))])),
            content: vec![],
            text: Some("x".to_string()),
            marks: vec![],
        };
        assert!(s.node_from_doc(&wire).is_err());
    }

    #[test]
    fn null_attr_round_trips_through_json() {
        // A Null attribute value survives a JSON round-trip as JSON `null`.
        let attr = JsonAttr::Null;
        let json = serde_json::to_string(&attr).unwrap();
        assert_eq!(json, "null");
        let back: JsonAttr = serde_json::from_str(&json).unwrap();
        assert_eq!(attr, back);
    }
}
