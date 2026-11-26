//! A textblock's app `data-*` attributes: on its host element, and through the
//! handle's `set_block_data_attr` / `block_data_attr`.

use super::*;
use rinch_editor_core::schema::DataAttrSplit;
use rinch_editor_core::{DATA_ATTR_SPLIT_KEY, DataAttrSplitPlugin, Plugin, PluginKey};

fn attr(h: &Harness, id: NodeId, name: &str) -> Option<String> {
    NodeHandle::new(id, Rc::downgrade(&h.doc)).get_attribute(name)
}

/// `data-role` of each of the container's block hosts.
fn roles(h: &Harness) -> Vec<Option<String>> {
    children(h, h.container_id)
        .into_iter()
        .filter(|id| attr(h, *id, "data-pm-type").is_some())
        .map(|id| attr(h, id, "data-role"))
        .collect()
}

fn two() -> Harness {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "ab"), para(&s, "cd")]));
    h.handle.set_selection(Selection::cursor(Pos(2)));
    h
}

#[test]
fn the_host_element_follows_set_change_remove_and_undo() {
    let h = two();
    let first = children(&h, h.container_id)[0];
    assert!(h.handle.set_block_data_attr("data-role", Some("sub")));
    assert_eq!(
        h.handle.block_data_attr("data-role").as_deref(),
        Some("sub")
    );
    assert_eq!(roles(&h), [Some("sub".into()), None]);
    assert!(!h.handle.set_block_data_attr("data-role", Some("sub")));
    assert!(h.handle.set_block_data_attr("data-role", Some("")));
    assert_eq!(roles(&h), [Some(String::new()), None]);
    assert!(h.handle.set_block_data_attr("data-role", None));
    assert_eq!(roles(&h), [None, None]);
    assert!(h.handle.command("undo"));
    assert_eq!(roles(&h), [Some(String::new()), None]);
    // An attribute change patches the element it has.
    assert_eq!(children(&h, h.container_id)[0], first);
}

#[test]
fn the_host_element_follows_a_split_a_retype_and_a_load() {
    let h = two();
    assert!(h.handle.set_block_data_attr("data-role", Some("sub")));
    h.handle.set_selection(Selection::cursor(Pos(3)));
    assert!(h.handle.command("enter"));
    assert_eq!(roles(&h), [Some("sub".into()), Some("sub".into()), None]);
    h.handle.set_selection(Selection::cursor(Pos(1)));
    assert!(h.handle.command("setHeading2"));
    let first = children(&h, h.container_id)[0];
    assert_eq!(tag(&h, first).as_deref(), Some("h2"));
    assert_eq!(attr(&h, first, "data-role"), None);
    assert!(h.handle.load_html(r#"<p data-role="x">a</p><p>b</p>"#));
    assert_eq!(roles(&h), [Some("x".into()), None]);
}

#[test]
fn a_plain_text_paste_at_the_start_leaves_them_with_the_text() {
    let h = two();
    assert!(h.handle.set_block_data_attr("data-role", Some("sub")));
    h.handle.set_selection(Selection::cursor(Pos(1)));
    assert!(h.handle.replace_selection_with_text("1\n2"));
    assert_eq!(roles(&h), [Some("sub".into()), Some("sub".into()), None]);
}

/// An app's choice, added to a mounted editor, is what Enter and a paste follow.
#[test]
fn an_added_split_choice_is_followed() {
    let h = two();
    h.handle.add_plugin(Rc::new(
        DataAttrSplitPlugin::new().with("data-role", DataAttrSplit::CopyToText),
    ));
    assert!(h.handle.set_block_data_attr("data-role", Some("sub")));
    h.handle.set_selection(Selection::cursor(Pos(3)));
    assert!(h.handle.command("enter"));
    assert_eq!(roles(&h), [Some("sub".into()), None, None]);
    h.handle.set_selection(Selection::cursor(Pos(1)));
    assert!(h.handle.replace_selection_with_text("1\n2"));
    assert_eq!(roles(&h), [None, Some("sub".into()), None, None]);
}

/// Enter at the end of the first block, marked `data-role` and `data-id`:
/// what the new block got of each.
fn enter_at_end(h: &Harness) -> (Option<String>, Option<String>) {
    assert!(h.handle.load_html("<p>ab</p>"));
    h.handle.set_selection(Selection::cursor(Pos(3)));
    h.handle.set_block_data_attr("data-role", Some("sub"));
    h.handle.set_block_data_attr("data-id", Some("1"));
    assert!(h.handle.command("enter"));
    let new = children(h, h.container_id)[1];
    (attr(h, new, "data-role"), attr(h, new, "data-id"))
}

/// A second `DataAttrSplitPlugin` (a component library's and the app's) is
/// merged with the first; for a name both have, the later wins.
#[test]
fn two_split_plugins_are_merged_and_the_later_wins() {
    let h = two();
    assert!(
        h.handle.add_plugin(Rc::new(
            DataAttrSplitPlugin::new()
                .with("data-role", DataAttrSplit::CopyToText)
                .with("data-id", DataAttrSplit::Copy),
        ))
    );
    assert!(h.handle.add_plugin(Rc::new(
        DataAttrSplitPlugin::new().with("data-id", DataAttrSplit::Keep),
    )));
    assert_eq!(enter_at_end(&h), (None, None));
    assert!(h.handle.add_plugin(Rc::new(
        DataAttrSplitPlugin::new().with("data-role", DataAttrSplit::Copy),
    )));
    assert_eq!(enter_at_end(&h), (Some("sub".into()), None));
}

#[test]
fn a_split_choice_set_late_keeps_the_undo_history() {
    let h = two();
    assert!(h.handle.set_block_data_attr("data-role", Some("sub")));
    h.handle.set_selection(Selection::cursor(Pos(2)));
    assert!(
        h.handle
            .set_data_attr_split("data-role", DataAttrSplit::CopyToText)
    );
    assert_eq!(h.handle.selection(), Selection::cursor(Pos(2)));
    assert!(h.handle.command("undo"));
    assert_eq!(roles(&h), [None, None]);
    // Adding the plugin keeps it too, a first one and a second.
    for id in ["data-id", "data-other"] {
        assert!(h.handle.command("redo"));
        assert!(h.handle.add_plugin(Rc::new(
            DataAttrSplitPlugin::new().with(id, DataAttrSplit::Keep),
        )));
        assert!(h.handle.command("undo"));
        assert_eq!(roles(&h), [None, None]);
    }
    assert_eq!(enter_at_end(&h), (None, None));
    assert!(
        h.handle
            .set_data_attr_split("data-role", DataAttrSplit::Copy)
    );
    assert_eq!(enter_at_end(&h), (Some("sub".into()), None));
    // Removing the plugin drops every choice.
    assert!(h.handle.remove_plugin(DATA_ATTR_SPLIT_KEY));
    assert_eq!(enter_at_end(&h), (Some("sub".into()), Some("1".into())));
}

#[test]
fn a_split_choice_is_taken_read_only_and_refused_for_a_reserved_name() {
    let h = two();
    h.handle.set_read_only(true);
    for name in ["role", "data-pm-type", "data-rid", "data-A"] {
        assert!(
            !h.handle.set_data_attr_split(name, DataAttrSplit::Keep),
            "{name}"
        );
    }
    assert!(
        h.handle
            .set_data_attr_split("data-role", DataAttrSplit::CopyToText)
    );
    h.handle.set_read_only(false);
    assert_eq!(enter_at_end(&h), (None, Some("1".into())));
}

/// The app's choice wins over an extension's plugin, added before or after it.
#[test]
fn the_apps_choice_wins_over_another_plugins() {
    for app_first in [true, false] {
        let h = two();
        let app = |h: &Harness| {
            assert!(
                h.handle
                    .set_data_attr_split("data-role", DataAttrSplit::Copy)
            );
        };
        if app_first {
            app(&h);
        }
        assert!(h.handle.add_plugin(Rc::new(Extension("extension"))));
        if !app_first {
            app(&h);
        }
        // `data-id`, which only the extension names, follows it.
        assert_eq!(enter_at_end(&h), (Some("sub".into()), None), "{app_first}");
    }
}

/// An extension that keeps both attributes to one block.
struct Extension(&'static str);

impl Plugin for Extension {
    fn key(&self) -> PluginKey {
        PluginKey(self.0)
    }

    fn data_attr_splits(&self) -> Vec<(String, DataAttrSplit)> {
        ["data-role", "data-id"]
            .map(|n| (n.to_string(), DataAttrSplit::Keep))
            .to_vec()
    }
}

#[test]
fn a_read_only_editor_refuses_it_and_still_answers() {
    let h = two();
    assert!(h.handle.set_block_data_attr("data-role", Some("sub")));
    h.handle.set_read_only(true);
    assert!(!h.handle.set_block_data_attr("data-role", None));
    assert!(!h.handle.set_block_data_attr("data-role", Some("other")));
    assert_eq!(
        h.handle.block_data_attr("data-role").as_deref(),
        Some("sub")
    );
    assert_eq!(roles(&h), [Some("sub".into()), None]);
}

#[test]
fn a_refused_attribute_changes_nothing() {
    let h = two();
    for name in ["role", "data-pm-type", "data-rid", "data-A"] {
        assert!(!h.handle.set_block_data_attr(name, Some("x")), "{name}");
    }
    assert_eq!(roles(&h), [None, None]);
}
