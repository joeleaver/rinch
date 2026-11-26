//! What a split of a textblock does with an app's `data-*` attributes
//! ([`crate::schema::data_attrs`]): the per-attribute choice, and the plugin an
//! app makes it with.

use std::collections::HashMap;

use crate::plugin::{Plugin, PluginKey};
use crate::schema::DataAttrSplit;

/// The key of [`DataAttrSplitPlugin`].
pub const DATA_ATTR_SPLIT_KEY: PluginKey = PluginKey("data_attr_split");

/// Chooses, by name, what a split of a textblock does with an app's `data-*`
/// attributes. An attribute it does not name is copied
/// ([`DataAttrSplit::Copy`]).
///
/// ```
/// use rinch_editor_core::DataAttrSplitPlugin;
/// use rinch_editor_core::schema::DataAttrSplit;
///
/// let splits = DataAttrSplitPlugin::new()
///     // Enter at the end of a styled paragraph starts a plain one.
///     .with("data-role", DataAttrSplit::CopyToText)
///     // An id names one block.
///     .with("data-id", DataAttrSplit::Keep);
/// # let _ = splits;
/// ```
#[derive(Clone, Debug, Default)]
pub struct DataAttrSplitPlugin {
    splits: HashMap<String, DataAttrSplit>,
}

impl DataAttrSplitPlugin {
    /// A plugin that names no attribute yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Choose what a split does with the attribute `name` (`data-` included).
    pub fn with(mut self, name: impl Into<String>, split: DataAttrSplit) -> Self {
        self.splits.insert(name.into(), split);
        self
    }
}

impl Plugin for DataAttrSplitPlugin {
    fn key(&self) -> PluginKey {
        DATA_ATTR_SPLIT_KEY
    }

    fn data_attr_splits(&self) -> Vec<(String, DataAttrSplit)> {
        self.splits.iter().map(|(n, s)| (n.clone(), *s)).collect()
    }
}
