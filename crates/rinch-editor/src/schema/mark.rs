//! Mark types and specifications.

use std::collections::HashMap;
use super::node::AttrSpec;

/// Trait for mark types (formatting that spans ranges of text).
pub trait Mark: std::fmt::Debug {
    /// Get the mark type name.
    fn type_name(&self) -> &str;

    /// Check if this mark excludes other marks.
    fn excludes(&self, _other: &dyn Mark) -> bool {
        false
    }
}

/// Specification for a mark type in the schema.
#[derive(Debug, Clone)]
pub struct MarkSpec {
    /// Mark name (e.g., "bold", "link")
    pub name: String,

    /// Mark attributes
    pub attrs: HashMap<String, AttrSpec>,

    /// Whether this mark spans across nodes
    pub spanning: bool,

    /// Marks that this mark excludes (can't coexist with).
    /// None = excludes nothing.
    /// Some("") = excludes all other marks.
    /// Some("bold italic") = excludes specific marks (space-separated).
    pub excludes: Option<String>,

    /// HTML tags to parse as this mark (e.g., ["strong", "b"])
    pub parse_html_tags: Vec<String>,
}

impl MarkSpec {
    /// Create a simple mark with no attrs.
    pub fn simple(name: &str) -> Self {
        Self {
            name: name.to_string(),
            attrs: HashMap::new(),
            spanning: true,
            excludes: None,
            parse_html_tags: Vec::new(),
        }
    }

    /// Create a mark with attributes.
    pub fn with_attrs(name: &str, attrs: HashMap<String, AttrSpec>) -> Self {
        Self {
            name: name.to_string(),
            attrs,
            spanning: true,
            excludes: None,
            parse_html_tags: Vec::new(),
        }
    }

    /// Create a new mark spec builder.
    pub fn builder(name: impl Into<String>) -> MarkSpecBuilder {
        MarkSpecBuilder::new(name)
    }

    /// Check if this mark excludes another mark type.
    pub fn excludes_mark(&self, other: &str) -> bool {
        match &self.excludes {
            None => false,
            Some(s) if s.is_empty() => true, // empty string = excludes all
            Some(s) => s.split_whitespace().any(|m| m == other),
        }
    }
}

/// Builder for mark specifications.
#[derive(Debug)]
pub struct MarkSpecBuilder {
    spec: MarkSpec,
}

impl MarkSpecBuilder {
    /// Create a new builder with the given mark name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            spec: MarkSpec::simple(&name.into()),
        }
    }

    /// Add an attribute.
    pub fn attr(mut self, name: impl Into<String>, attr: AttrSpec) -> Self {
        self.spec.attrs.insert(name.into(), attr);
        self
    }

    /// Set HTML tags that parse to this mark.
    pub fn parse_html(mut self, tags: Vec<String>) -> Self {
        self.spec.parse_html_tags = tags;
        self
    }

    /// Set the excludes expression.
    pub fn excludes(mut self, excludes: impl Into<String>) -> Self {
        self.spec.excludes = Some(excludes.into());
        self
    }

    /// Set whether this mark spans across nodes.
    pub fn spanning(mut self, spanning: bool) -> Self {
        self.spec.spanning = spanning;
        self
    }

    /// Build the mark spec.
    pub fn build(self) -> MarkSpec {
        self.spec
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_mark_defaults() {
        let spec = MarkSpec::simple("bold");
        assert_eq!(spec.name, "bold");
        assert!(spec.attrs.is_empty());
        assert!(spec.spanning);
        assert!(spec.excludes.is_none());
    }

    #[test]
    fn with_attrs_constructor() {
        let mut attrs = HashMap::new();
        attrs.insert("href".to_string(), AttrSpec::required());
        attrs.insert("title".to_string(), AttrSpec::optional(""));
        let spec = MarkSpec::with_attrs("link", attrs);
        assert_eq!(spec.name, "link");
        assert_eq!(spec.attrs.len(), 2);
        assert!(spec.attrs["href"].required);
    }

    #[test]
    fn excludes_none_excludes_nothing() {
        let spec = MarkSpec::simple("bold");
        assert!(!spec.excludes_mark("italic"));
        assert!(!spec.excludes_mark("bold"));
    }

    #[test]
    fn excludes_empty_excludes_all() {
        let mut spec = MarkSpec::simple("code");
        spec.excludes = Some("".to_string());
        assert!(spec.excludes_mark("bold"));
        assert!(spec.excludes_mark("italic"));
        assert!(spec.excludes_mark("anything"));
    }

    #[test]
    fn excludes_specific_marks() {
        let mut spec = MarkSpec::simple("code");
        spec.excludes = Some("bold italic underline".to_string());
        assert!(spec.excludes_mark("bold"));
        assert!(spec.excludes_mark("italic"));
        assert!(spec.excludes_mark("underline"));
        assert!(!spec.excludes_mark("link"));
        assert!(!spec.excludes_mark("strike"));
    }

    #[test]
    fn builder_works() {
        let spec = MarkSpec::builder("link")
            .attr("href", AttrSpec::required())
            .attr("title", AttrSpec::optional(""))
            .parse_html(vec!["a".into()])
            .excludes("code")
            .build();

        assert_eq!(spec.name, "link");
        assert_eq!(spec.attrs.len(), 2);
        assert_eq!(spec.parse_html_tags, vec!["a"]);
        assert!(spec.excludes_mark("code"));
        assert!(!spec.excludes_mark("bold"));
    }
}
