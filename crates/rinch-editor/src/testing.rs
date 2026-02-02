//! Test utilities for editor testing.

use crate::editor::{Editor, EditorConfig};
use crate::schema::Schema;
use crate::error::EditorError;

/// Create a test editor with a basic schema.
pub fn create_test_editor() -> Result<Editor, EditorError> {
    let schema = create_basic_schema();
    Editor::new(schema, EditorConfig::default())
}

/// Create a basic schema for testing.
pub fn create_basic_schema() -> Schema {
    Schema::builder().build()
}

/// Assert that two documents have the same content.
#[macro_export]
macro_rules! assert_doc_eq {
    ($left:expr, $right:expr) => {
        assert_eq!($left.text(), $right.text())
    };
}
