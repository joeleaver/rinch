//! A re-rendering component with reactive root bindings inside an `if` body
//! (issue #1190, review of PR #1193 round 3).
//!
//! The branch closure runs more than once, so the site bundle built inside it
//! must shadow-clone what it captures from outside the branch; without those
//! clones the branch closure moves `s` into the bundle and is only `FnOnce`
//! (E0525). Nothing else in the suite builds a bundle inside a repeatable body.

use rinch::prelude::*;
use rinch_core::dom::DomDocument;
use rinch_core::dom::mock::MockDomDocument;
use std::cell::RefCell;
use std::rc::Rc;

#[component]
fn Probe(label: String, children: &[NodeHandle]) -> NodeHandle {
    let root = __scope.create_element("section");
    root.set_attribute("class", "probe");
    root.set_attribute("aria-label", &label);
    for c in children {
        root.append_child(c);
    }
    root
}

#[component]
fn site(sig: Signal<String>, s: String) -> NodeHandle {
    rsx! {
        div {
            if sig.get().is_empty() {
                ""
            } else {
                Probe {
                    label: {move || sig.get()},
                    data-a: {|| s.clone()},
                    style: {|| format!("--s: {s}")},
                    mt: {|| s.len().to_string()},
                    class: s.clone(),
                    "text "
                    {s.clone()}
                }
            }
        }
    }
}

fn find_probe(node: &NodeHandle) -> Option<NodeHandle> {
    if node.get_attribute("aria-label").is_some() {
        return Some(node.clone());
    }
    node.children().iter().find_map(find_probe)
}

#[test]
fn a_bundle_inside_a_branch_compiles_and_survives_re_showing() {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let sig = Signal::new("a".to_string());
    let root = site(&mut scope, sig, "vv".to_string());
    assert_eq!(
        find_probe(&root)
            .unwrap()
            .get_attribute("data-a")
            .as_deref(),
        Some("vv")
    );
    sig.set(String::new());
    assert!(find_probe(&root).is_none());
    sig.set("b".to_string());
    let p = find_probe(&root).expect("shown again");
    assert_eq!(p.get_attribute("aria-label").as_deref(), Some("b"));
    assert_eq!(p.get_attribute("data-a").as_deref(), Some("vv"));
}
