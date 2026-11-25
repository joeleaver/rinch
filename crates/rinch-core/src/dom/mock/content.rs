//! [`DomContent`](crate::dom::DomContent) for [`MockDomDocument`].

use super::{MockDomDocument, MockNodeKind, is_value_control};
use crate::dom::{DomContent, DomTree, NodeId};

impl DomContent for MockDomDocument {
    fn set_text_content(&mut self, node: NodeId, text: &str) {
        self.count(|ops| ops.text_writes += 1);
        // On an element the text *replaces* the child list, as it does on both
        // real backends (issue #1440): every child is orphaned — detached, not
        // retired, so a handle to one still re-inserts, which is what
        // `rinch-dom` (`parent = None`, slab entry kept) and the browser
        // (`textContent = …`, the old nodes still in `rinch-web`'s table) leave.
        // The text itself is kept on the element and read back ahead of any
        // child appended later (`text_of`); it has no node id, as the browser's
        // new text node has none (`rinch-dom` mints one).
        let orphans = match self.nodes.get_mut(&node) {
            Some(n) => {
                n.text = text.to_string();
                match n.kind {
                    MockNodeKind::Element(_) => std::mem::take(&mut n.children),
                    MockNodeKind::Text | MockNodeKind::Comment => Vec::new(),
                }
            }
            None => Vec::new(),
        };
        for orphan in orphans {
            if let Some(o) = self.nodes.get_mut(&orphan) {
                o.parent = None;
            }
        }
        self.mark_dirty(node);
    }

    fn set_attribute(&mut self, node: NodeId, name: &str, value: &str) {
        self.count(|ops| ops.attribute_writes += 1);
        if let Some(n) = self.nodes.get_mut(&node) {
            // A programmatic write reaches the live text too, as `rinch-web`'s
            // `sync_reflected_property` makes it (issue #100).
            if name == "value"
                && matches!(&n.kind, MockNodeKind::Element(tag) if is_value_control(tag))
            {
                n.live_value = Some(value.to_string());
            }
            n.attributes.insert(name.to_string(), value.to_string());
        }
        self.mark_dirty(node);
    }

    fn remove_attribute(&mut self, node: NodeId, name: &str) {
        self.count(|ops| ops.attribute_writes += 1);
        if let Some(n) = self.nodes.get_mut(&node) {
            // `rinch-web` empties a control's live text when its `value`
            // attribute goes (`remove_value_attribute` writes `""`).
            if name == "value"
                && matches!(&n.kind, MockNodeKind::Element(tag) if is_value_control(tag))
            {
                n.live_value = Some(String::new());
            }
            n.attributes.remove(name);
        }
        self.mark_dirty(node);
    }

    fn attribute_list(&self, node: NodeId) -> Vec<(String, String)> {
        self.nodes.get(&node).map_or_else(Vec::new, |n| {
            n.attributes
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
    }

    fn get_attribute(&self, node: NodeId, name: &str) -> Option<String> {
        self.count(|ops| ops.attribute_reads += 1);
        self.nodes.get(&node)?.attributes.get(name).cloned()
    }

    /// `rinch-web`'s answer, not the default's: a form control reads its live
    /// text, which [`__type_into`](MockDomDocument::__type_into) can move away
    /// from the attribute, and always answers `Some` — `""` when empty.
    fn live_value(&self, node: NodeId) -> Option<String> {
        let n = self.nodes.get(&node)?;
        match &n.kind {
            MockNodeKind::Element(tag) if is_value_control(tag) => Some(
                n.live_value
                    .clone()
                    .or_else(|| n.attributes.get("value").cloned())
                    // A pristine `<textarea>`'s value is its text children,
                    // its default value (#1159).
                    .or_else(|| {
                        (tag == "textarea").then(|| {
                            n.children
                                .iter()
                                .filter_map(|c| self.nodes.get(c))
                                .filter(|c| matches!(c.kind, MockNodeKind::Text))
                                .map(|c| c.text.as_str())
                                .collect()
                        })
                    })
                    .unwrap_or_default(),
            ),
            _ => n.attributes.get("value").cloned(),
        }
    }

    /// Merge one declaration into the node's inline `style`, the way both real
    /// backends do (#666).
    ///
    /// This used to append `"{property}: {value};"` to whatever string was
    /// there, with no separator and no replacement: a node whose `style` said
    /// `"color: red"` came out of `set_style("padding", "12px")` as
    /// `"color: redpadding: 12px;"` — one declaration whose value is
    /// `redpadding: 12px` — and setting the same property twice appended
    /// twice. That made the mock unusable as an oracle for anything about
    /// inline-style *composition*, which is the one thing a `style:` prop and
    /// a style shorthand on the same element are about.
    ///
    /// The parse and the join are
    /// [`split_declarations`](crate::dom::split_declarations)/[`serialize_declarations`](crate::dom::serialize_declarations),
    /// which is also what `RinchDocument::set_styles` uses (#670) — so the mock
    /// now agrees with desktop declaration for declaration, quoted and
    /// bracketed values included. A property already declared is replaced
    /// **where it stands**, matching CSSOM's `setProperty` and desktop's
    /// `dom_tests::set_style_replaces_a_declaration_in_place`, and `property`
    /// is matched ASCII case-insensitively unless it is a custom property
    /// ([`normalize_property_name`](crate::dom::normalize_property_name), #711).
    ///
    /// The mock has no property table, so it does **not** move a write past a
    /// later declaration that covers it (`left` before `inset`), which
    /// `RinchDocument` does and CSSOM's longhand list makes moot (#470); the
    /// strings agree everywhere else.
    fn set_style(&mut self, node: NodeId, property: &str, value: &str) {
        self.count(|ops| ops.attribute_writes += 1);
        if let Some(n) = self.nodes.get_mut(&node) {
            let style = n.attributes.entry("style".to_string()).or_default();
            let mut decls = crate::dom::split_declarations(style);
            let property = crate::dom::normalize_property_name(property);
            match decls.iter_mut().find(|(k, _)| k.as_str() == &*property) {
                Some(slot) => slot.1 = value.to_string(),
                None => decls.push((property.into_owned(), value.to_string())),
            }
            *style = crate::dom::serialize_declarations(&decls);
        }
        self.mark_dirty(node);
    }

    fn text_content(&self, node: NodeId) -> Option<String> {
        self.count(|ops| ops.text_reads += 1);
        self.text_of(node)
    }
}
