//! A `Stepper` is re-derived when a step leaves because something wrote over
//! the children of the element holding it — issue #1440.
//!
//! `NodeHandle::set_text` on an element replaces its child list with the text
//! (and `set_inner_html` with the parsed markup). Neither told an
//! `on_child_removed` observer, so the steps behind the one that went kept the
//! index and state of positions they no longer stood at. `late_children_716.rs`
//! has the same shape through `remove()`.

use rinch_components as rinch;

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::stepper::{Stepper, StepperStep};
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_macros::rsx;

mod common;
use common::{collect_by_class, has_class};

struct Tree {
    _doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    root: NodeHandle,
}

impl Tree {
    /// `a`, a wrapper `<div class="wrap">` holding `b`, then `c` and `d`;
    /// `active` is 2, so off both ends: `c` is in progress and `d` inactive.
    fn build() -> Self {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let root = {
            let __scope = &mut scope;
            rsx! {
                div {
                    Stepper { active: 2u32,
                        StepperStep { label: "a" }
                        div { class: "wrap", StepperStep { label: "b" } }
                        StepperStep { label: "c" }
                        StepperStep { label: "d" }
                    }
                }
            }
        };
        Self {
            _doc: doc,
            _scope: scope,
            root,
        }
    }

    fn find_all(&self, class: &str) -> Vec<NodeHandle> {
        let mut out = Vec::new();
        collect_by_class(&self.root, class, &mut out);
        out
    }

    fn steps(&self) -> Vec<(String, &'static str)> {
        self.find_all("rinch-stepper__step")
            .iter()
            .map(|s| {
                let state = if has_class(s, "rinch-stepper__step--completed") {
                    "completed"
                } else if has_class(s, "rinch-stepper__step--progress") {
                    "progress"
                } else {
                    "inactive"
                };
                (s.get_attribute("data-step").unwrap_or_default(), state)
            })
            .collect()
    }
}

fn at(index: &str, state: &'static str) -> (String, &'static str) {
    (index.to_string(), state)
}

fn before() -> Vec<(String, &'static str)> {
    vec![
        at("0", "completed"),
        at("1", "completed"),
        at("2", "progress"),
        at("3", "inactive"),
    ]
}

fn after() -> Vec<(String, &'static str)> {
    vec![
        at("0", "completed"),
        at("1", "completed"),
        at("2", "progress"),
    ]
}

#[test]
fn text_written_over_a_steps_wrapper_renumbers_the_steps_behind_it() {
    let tree = Tree::build();
    assert_eq!(tree.steps(), before(), "precondition");

    tree.find_all("wrap")[0].set_text("gone");

    assert_eq!(
        tree.steps(),
        after(),
        "#1440: `b` left with the wrapper's children, so `c` stands at 1 \
         (completed) and `d` at 2 (in progress)"
    );
}

#[test]
fn html_written_over_a_steps_wrapper_renumbers_the_steps_behind_it() {
    let tree = Tree::build();
    assert_eq!(tree.steps(), before(), "precondition");

    tree.find_all("wrap")[0].set_inner_html("<p>gone</p>");

    assert_eq!(tree.steps(), after(), "#1440, through `set_inner_html`");
}
