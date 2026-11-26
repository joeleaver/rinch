//! A textblock's app `data-*` attributes: on its host element, and through the
//! handle's `set_block_data_attr` / `block_data_attr`.

use super::*;

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
    assert_eq!(roles(&h).iter().flatten().count(), 1);
    // An attribute change patches the element it has.
    assert_eq!(children(&h, h.container_id)[0], first);
}

#[test]
fn the_host_element_follows_a_split_a_retype_and_a_load() {
    let h = two();
    assert!(h.handle.set_block_data_attr("data-role", Some("sub")));
    h.handle.set_selection(Selection::cursor(Pos(3)));
    assert!(h.handle.command("enter"));
    assert_eq!(roles(&h), [Some("sub".into()), None, None]);
    h.handle.set_selection(Selection::cursor(Pos(1)));
    assert!(h.handle.command("toggleHeading2") || h.handle.command("setHeading2"));
    let first = children(&h, h.container_id)[0];
    assert_eq!(tag(&h, first).as_deref(), Some("h2"));
    assert_eq!(attr(&h, first, "data-role").as_deref(), Some("sub"));
    assert!(h.handle.load_html(r#"<p data-role="x">a</p><p>b</p>"#));
    assert_eq!(roles(&h), [Some("x".into()), None]);
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
