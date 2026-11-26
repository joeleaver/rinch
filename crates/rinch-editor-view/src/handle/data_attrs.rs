//! An app's `data-*` attributes on the selection's textblocks, from the handle.

use std::rc::Rc;

use rinch_editor_core::schema::{DataAttrSplit, is_app_data_attr};
use rinch_editor_core::{DATA_ATTR_SPLIT_KEY, DataAttrSplitPlugin, Plugin, commands};

use super::EditorHandle;

impl EditorHandle {
    /// Set (`Some`) or remove (`None`) the app data attribute `name` on every
    /// textblock of the selection that keeps data attributes (the starter kit's
    /// paragraph and heading), as one undo step
    /// ([`commands::set_block_data_attr`]). The editor writes it on each
    /// block's host element, so the app's stylesheet can select on it
    /// (`p[data-role="note"]`).
    ///
    /// Returns whether anything changed: `false` when no block would change,
    /// when the name or value is refused (a name rinch reserves or HTML does
    /// not allow, a value past the bound) and on a
    /// [read-only](EditorHandle::set_read_only) editor.
    pub fn set_block_data_attr(&self, name: &str, value: Option<&str>) -> bool {
        self.run_command(commands::set_block_data_attr(name, value))
    }

    /// Choose what a split of a textblock (Enter, and lines of pasted plain
    /// text) does with the app data attribute `name`
    /// ([`DataAttrSplit`]; the default is `Copy`).
    ///
    /// ```ignore
    /// handle.set_data_attr_split("data-role", DataAttrSplit::CopyToText);
    /// ```
    ///
    /// Callable at any time, mounted or not, read-only or not. It is not an
    /// edit: the document, selection, stored marks and **undo history** stay,
    /// and [`on_change`](Self::on_change) does not fire. It holds for splits
    /// made afterwards.
    ///
    /// The choice is the app's own: the last one made for a name wins, whether
    /// made here or by [`add_plugin`](Self::add_plugin) of a
    /// [`DataAttrSplitPlugin`], and it wins over what a plugin of another key
    /// answers from [`Plugin::data_attr_splits`].
    /// [`remove_plugin`](Self::remove_plugin)`(DATA_ATTR_SPLIT_KEY)` drops
    /// every choice made here or so.
    ///
    /// Returns whether the choice is in force: `false` for a name that is not
    /// an app data attribute ([`is_app_data_attr`]).
    pub fn set_data_attr_split(&self, name: &str, split: DataAttrSplit) -> bool {
        is_app_data_attr(name) && self.merge_data_attr_splits(vec![(name.to_string(), split)])
    }

    /// Merge `newer` over the app's split choices: one [`DataAttrSplitPlugin`]
    /// holding both takes the place of whichever is installed, keeping every
    /// plugin's folded state.
    pub(super) fn merge_data_attr_splits(&self, newer: Vec<(String, DataAttrSplit)>) -> bool {
        let installed: Vec<Rc<dyn Plugin>> = self
            .core()
            .plugins
            .iter()
            .filter(|p| p.key() == DATA_ATTR_SPLIT_KEY)
            .cloned()
            .collect();
        let merged = installed
            .iter()
            .flat_map(|p| p.data_attr_splits())
            .chain(newer)
            .fold(DataAttrSplitPlugin::new(), |plugin, (name, split)| {
                plugin.with(name, split)
            });
        let mut plugins = self.core().plugins.clone();
        plugins.retain(|p| p.key() != DATA_ATTR_SPLIT_KEY);
        plugins.push(Rc::new(merged));
        self.reconfigure_plugins(plugins, true)
    }

    /// The value of the app data attribute `name` that every textblock of the
    /// selection which keeps data attributes holds, if they share one
    /// ([`commands::block_data_attr`]).
    pub fn block_data_attr(&self, name: &str) -> Option<String> {
        commands::block_data_attr(&self.core().state, name)
    }
}
