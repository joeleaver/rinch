//! `Select`'s open option list answers the keyboard (issue #434).
//!
//! #251 made the trigger reachable (Tab, Enter/Space to open) and stopped
//! there: once the list was open, every key fell through, so a keyboard user
//! could open a `Select` and do nothing else with it. What is pinned here, on
//! the mock (the key path — `dispatch_keyboard_event` — is the one both
//! backends call, so it is backend-agnostic by construction; the desktop and
//! Chrome twins drive real key events through each shell):
//!
//! - **The highlight starts at the selected option**, not at the first. Every
//!   fixture here selects an option in the *middle* of the list, so a highlight
//!   reset to index 0 on open reads differently from one reset to the selection
//!   — at index 0 the two agree (the fixed point).
//! - **ArrowDown/ArrowUp step and wrap; Home/End jump.**
//! - **Enter commits the highlighted option; Escape closes without committing**.
//! - **Keys belong to the list only while it is open** — a closed `Select`
//!   consumes nothing, so ArrowDown still scrolls the page.
//! - **Type-ahead** jumps to the first option whose label starts with what was
//!   typed.
//! - The ARIA: `role="listbox"`/`role="option"`, `aria-selected`, and
//!   `aria-activedescendant` naming the highlighted option's `id`.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::select::{Select, SelectOption};
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::events::{KeyEventData, dispatch_keyboard_event};
use rinch_core::{Component, InputCallback};

mod common;
use common::{collect_by_class, find_by_class, handler, has_class};

const TRIGGER: &str = "rinch-select__input";
const OPTION: &str = "rinch-select__option";
const HIGHLIGHTED: &str = "rinch-select__option--highlighted";

struct Mounted {
    _doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    root: NodeHandle,
    picks: Rc<RefCell<Vec<String>>>,
}

fn fruit() -> Vec<SelectOption> {
    vec![
        SelectOption::new("apple", "Apple"),
        SelectOption::new("banana", "Banana"),
        SelectOption::new("cherry", "Cherry"),
        SelectOption::new("blueberry", "Blueberry"),
        SelectOption::new("date", "Date"),
    ]
}

impl Mounted {
    fn new(value: &str) -> Self {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let picks = Rc::new(RefCell::new(Vec::new()));
        let sink = picks.clone();
        let select = Select {
            value: value.to_string(),
            data: fruit(),
            onchange: Some(InputCallback::new(move |v: String| {
                sink.borrow_mut().push(v)
            })),
            ..Default::default()
        };
        let root = select.render(&mut scope, &[]);
        Self {
            _doc: doc,
            _scope: scope,
            root,
            picks,
        }
    }

    fn trigger(&self) -> NodeHandle {
        find_by_class(&self.root, TRIGGER).expect("the trigger exists")
    }

    fn options(&self) -> Vec<NodeHandle> {
        let mut out = Vec::new();
        collect_by_class(&self.root, OPTION, &mut out);
        out
    }

    fn is_open(&self) -> bool {
        self.trigger().get_attribute("aria-expanded").as_deref() == Some("true")
    }

    /// Open it the way both backends do: the trigger's `data-rid` (a click, or
    /// Enter/Space on the focused trigger).
    fn open(&self) {
        rinch_core::events::dispatch_event(handler(&self.root, TRIGGER, "data-rid"));
        assert!(self.is_open(), "precondition: the trigger opened the list");
    }

    /// Press `key`; answers whether the key was consumed.
    fn press(&self, key: &str) -> bool {
        dispatch_keyboard_event(&KeyEventData::new(key, key))
    }

    /// The index of the one highlighted option, or `None`. Panics if two are.
    fn highlighted(&self) -> Option<usize> {
        let lit: Vec<usize> = self
            .options()
            .iter()
            .enumerate()
            .filter(|(_, o)| has_class(o, HIGHLIGHTED))
            .map(|(i, _)| i)
            .collect();
        assert!(
            lit.len() <= 1,
            "more than one option is highlighted: {lit:?}"
        );
        lit.first().copied()
    }

    fn value_shown(&self) -> String {
        let span = find_by_class(&self.root, "rinch-select__display").unwrap();
        span.children()
            .first()
            .and_then(|t| t.text_content())
            .unwrap_or_default()
    }
}

#[test]
fn opening_highlights_the_selected_option_not_the_first() {
    let m = Mounted::new("cherry");
    assert_eq!(m.highlighted(), None, "a closed list highlights nothing");
    m.open();
    assert_eq!(
        m.highlighted(),
        Some(2),
        "the highlight starts at the selection"
    );
}

#[test]
fn opening_with_no_selection_highlights_the_first_option() {
    let m = Mounted::new("");
    m.open();
    assert_eq!(m.highlighted(), Some(0));
}

#[test]
fn arrow_down_and_up_move_the_highlight_and_are_consumed() {
    let m = Mounted::new("banana");
    m.open();
    assert!(
        m.press("ArrowDown"),
        "ArrowDown on an open list is consumed"
    );
    assert_eq!(m.highlighted(), Some(2));
    assert!(m.press("ArrowDown"));
    assert_eq!(m.highlighted(), Some(3));
    assert!(m.press("ArrowUp"));
    assert_eq!(m.highlighted(), Some(2));
    assert!(m.is_open(), "moving the highlight does not close the list");
    assert!(m.picks.borrow().is_empty(), "nor does it commit anything");
}

#[test]
fn the_arrows_wrap_at_both_ends() {
    let m = Mounted::new("date");
    m.open();
    assert_eq!(m.highlighted(), Some(4));
    m.press("ArrowDown");
    assert_eq!(
        m.highlighted(),
        Some(0),
        "ArrowDown past the last wraps to the first"
    );
    m.press("ArrowUp");
    assert_eq!(
        m.highlighted(),
        Some(4),
        "ArrowUp past the first wraps to the last"
    );
}

#[test]
fn home_and_end_jump_to_the_ends() {
    let m = Mounted::new("cherry");
    m.open();
    assert!(m.press("End"));
    assert_eq!(m.highlighted(), Some(4));
    assert!(m.press("Home"));
    assert_eq!(m.highlighted(), Some(0));
}

#[test]
fn enter_commits_the_highlighted_option_and_closes() {
    let m = Mounted::new("banana");
    m.open();
    m.press("ArrowDown");
    m.press("ArrowDown");
    assert!(m.press("Enter"), "Enter on an open list is consumed");
    assert!(!m.is_open(), "Enter closes the list");
    assert_eq!(*m.picks.borrow(), vec!["blueberry".to_string()]);
    assert_eq!(m.value_shown(), "Blueberry");
}

#[test]
fn escape_closes_without_committing() {
    let m = Mounted::new("banana");
    m.open();
    m.press("ArrowDown");
    assert!(m.press("Escape"), "Escape on an open list is consumed");
    assert!(!m.is_open(), "Escape closes the list");
    assert!(m.picks.borrow().is_empty(), "and commits nothing");
    assert_eq!(m.value_shown(), "Banana", "the value is the one it had");

    // Reopening starts from the selection again, not from where the abandoned
    // highlight was.
    m.open();
    assert_eq!(m.highlighted(), Some(1));
}

#[test]
fn a_closed_select_consumes_no_key() {
    let m = Mounted::new("banana");
    for key in [
        "ArrowDown",
        "ArrowUp",
        "Home",
        "End",
        "Enter",
        "Escape",
        "c",
    ] {
        assert!(
            !m.press(key),
            "a closed Select must leave {key:?} to the page"
        );
    }
    m.open();
    m.press("Escape");
    for key in ["ArrowDown", "Enter", "Escape"] {
        assert!(
            !m.press(key),
            "a Select closed again must leave {key:?} to the page"
        );
    }
    assert!(m.picks.borrow().is_empty());
}

#[test]
fn typing_jumps_to_the_first_option_starting_with_the_prefix() {
    let m = Mounted::new("apple");
    m.open();
    assert!(m.press("c"), "a printable key on an open list is consumed");
    assert_eq!(m.highlighted(), Some(2), "c → Cherry");
    m.press("Escape");
    let m = Mounted::new("apple");
    m.open();
    m.press("b");
    assert_eq!(m.highlighted(), Some(1), "b → Banana");
    m.press("l");
    assert_eq!(m.highlighted(), Some(3), "bl → Blueberry");
}

#[test]
fn a_modified_key_is_left_to_the_app() {
    let m = Mounted::new("apple");
    m.open();
    let chord = KeyEventData::new("c", "KeyC").with_modifiers(true, false, false, false);
    assert!(
        !dispatch_keyboard_event(&chord),
        "Ctrl+C is a chord, not type-ahead"
    );
    assert_eq!(m.highlighted(), Some(0));
}

#[test]
fn the_aria_names_the_highlighted_option() {
    let m = Mounted::new("banana");
    let list = find_by_class(&m.root, "rinch-select__dropdown").unwrap();
    assert_eq!(list.get_attribute("role").as_deref(), Some("listbox"));
    let options = m.options();
    let ids: Vec<String> = options
        .iter()
        .map(|o| {
            assert_eq!(o.get_attribute("role").as_deref(), Some("option"));
            o.get_attribute("id").expect("every option has an id")
        })
        .collect();
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), ids.len(), "option ids are unique: {ids:?}");
    assert_eq!(
        m.trigger().get_attribute("aria-controls"),
        list.get_attribute("id"),
        "the trigger controls the list"
    );

    let selected: Vec<Option<String>> = options
        .iter()
        .map(|o| o.get_attribute("aria-selected"))
        .collect();
    assert_eq!(
        selected,
        ["false", "true", "false", "false", "false"]
            .map(|s| Some(s.to_string()))
            .to_vec()
    );

    assert_eq!(
        m.trigger().get_attribute("aria-activedescendant"),
        None,
        "a closed list has no active descendant"
    );
    m.open();
    m.press("ArrowDown");
    assert_eq!(
        m.trigger().get_attribute("aria-activedescendant"),
        Some(ids[2].clone())
    );
    m.press("Enter");
    assert_eq!(m.trigger().get_attribute("aria-activedescendant"), None);
    assert_eq!(
        options[2].get_attribute("aria-selected").as_deref(),
        Some("true")
    );
    assert_eq!(
        options[1].get_attribute("aria-selected").as_deref(),
        Some("false")
    );
}

/// Two `Select`s mint distinct option ids — `aria-activedescendant` is a
/// document-wide `id` reference.
#[test]
fn two_selects_do_not_share_option_ids() {
    let a = Mounted::new("");
    let b = Mounted::new("");
    let ida = a.options()[0].get_attribute("id");
    let idb = b.options()[0].get_attribute("id");
    assert!(ida.is_some() && ida != idb, "{ida:?} vs {idb:?}");
}

/// Tab closes the list without committing and is **not** consumed, so the
/// backend's own Tab moves focus on; Space commits like Enter (both spellings:
/// desktop names it `"Space"`, the browser sends `" "`).
#[test]
fn tab_closes_and_passes_on_and_space_commits() {
    let m = Mounted::new("banana");
    m.open();
    m.press("ArrowDown");
    assert!(!m.press("Tab"), "Tab must reach the focus order");
    assert!(!m.is_open(), "Tab closes the list");
    assert!(m.picks.borrow().is_empty(), "and commits nothing");

    for space in ["Space", " "] {
        m.open();
        m.press("ArrowDown");
        assert!(m.press(space), "{space:?} is consumed");
        assert!(!m.is_open());
    }
    assert_eq!(
        *m.picks.borrow(),
        vec!["cherry".to_string(), "blueberry".to_string()]
    );
}
