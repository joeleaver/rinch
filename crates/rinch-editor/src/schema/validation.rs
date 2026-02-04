//! Schema validation utilities.

use super::{Schema, matches_content};
use crate::error::EditorError;

/// Validate that a node's children match its content expression.
///
/// Returns Ok if the children satisfy the parent's content expression,
/// or an error describing the mismatch.
pub fn validate_content(
    schema: &Schema,
    parent_type: &str,
    children: &[&str],
) -> Result<(), EditorError> {
    let parent = schema.node(parent_type).ok_or_else(|| {
        EditorError::SchemaValidation(format!("Unknown node type: {}", parent_type))
    })?;

    match &parent.content {
        None => {
            if children.is_empty() {
                Ok(())
            } else {
                Err(EditorError::SchemaValidation(format!(
                    "Node '{}' is a leaf node and cannot have children",
                    parent_type
                )))
            }
        }
        Some(expr) => {
            if matches_content(schema, expr, children) {
                Ok(())
            } else {
                Err(EditorError::SchemaValidation(format!(
                    "Children {:?} do not match content expression '{}' for node '{}'",
                    children, expr, parent_type
                )))
            }
        }
    }
}

/// Check if a mark can be applied at the given node context.
///
/// A mark is allowed if the node's mark set permits it and
/// the mark type exists in the schema.
pub fn can_mark(schema: &Schema, node_type: &str, mark_type: &str) -> bool {
    schema.allows_mark(node_type, mark_type)
}

/// Check if a node can be inserted as a child of another.
///
/// Delegates to `Schema::allows_child`.
pub fn can_insert(schema: &Schema, parent_type: &str, child_type: &str) -> bool {
    schema.allows_child(parent_type, child_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_content_valid_doc() {
        let schema = Schema::starter_kit();
        assert!(validate_content(&schema, "doc", &["paragraph"]).is_ok());
        assert!(validate_content(&schema, "doc", &["paragraph", "heading"]).is_ok());
    }

    #[test]
    fn validate_content_empty_doc_fails() {
        let schema = Schema::starter_kit();
        assert!(validate_content(&schema, "doc", &[]).is_err());
    }

    #[test]
    fn validate_content_invalid_child() {
        let schema = Schema::starter_kit();
        assert!(validate_content(&schema, "doc", &["text"]).is_err());
    }

    #[test]
    fn validate_content_leaf_node_no_children() {
        let schema = Schema::starter_kit();
        assert!(validate_content(&schema, "horizontal_rule", &[]).is_ok());
        assert!(validate_content(&schema, "horizontal_rule", &["text"]).is_err());
    }

    #[test]
    fn validate_content_unknown_parent() {
        let schema = Schema::starter_kit();
        assert!(validate_content(&schema, "nonexistent", &["paragraph"]).is_err());
    }

    #[test]
    fn can_mark_delegates_correctly() {
        let schema = Schema::starter_kit();
        assert!(can_mark(&schema, "paragraph", "bold"));
        assert!(!can_mark(&schema, "code_block", "bold"));
    }

    #[test]
    fn can_insert_delegates_correctly() {
        let schema = Schema::starter_kit();
        assert!(can_insert(&schema, "doc", "paragraph"));
        assert!(!can_insert(&schema, "doc", "text"));
        assert!(can_insert(&schema, "paragraph", "text"));
    }
}
