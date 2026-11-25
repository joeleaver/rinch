//! Tree multi-select (#1598): clicks with Ctrl and Shift, Shift with the
//! keys, Enter and Space in a `multiple` tree, and what the app is told
//! (Ctrl+A is the app's since #3062). Started as the review probes of #1598;
//! each was red before its fix.

use super::*;
use std::cell::RefCell;

use rinch_components::{
    Tree, TreeKey, TreeMove, TreeNodeData, TreeSlot, UseTreeOptions, UseTreeReturn,
};
use rinch_core::Component;
use rinch_core::element::ValueCallback;
use rinch_core::reactive::Signal;
use rinch_platform::{KeyCode, KeyRepeat, Modifiers};

const VIEWPORT: (f32, f32) = (800.0, 600.0);

type Log<T> = Rc<RefCell<Vec<T>>>;

#[derive(Clone, Copy)]
struct Opts {
    multiple: bool,
    select_on_click: bool,
    keyboard: bool,
}

/// What the app's `handle_key` saw: key, below, carried, drop_for(Down) slot.
type Seen = (
    String,
    Option<String>,
    Vec<String>,
    Option<(Option<String>, TreeSlot)>,
);

struct P {
    app: RinchApp,
    state: UseTreeReturn,
    data: Signal<Vec<TreeNodeData>>,
    changes: Log<Vec<String>>,
    selects: Log<String>,
    rested: Log<String>,
    seen: Log<Seen>,
}

fn build(opts: Opts, initial: Vec<TreeNodeData>, expanded: &[&str]) -> P {
    let changes: Log<Vec<String>> = Rc::default();
    let selects: Log<String> = Rc::default();
    let rested: Log<String> = Rc::default();
    let seen: Log<Seen> = Rc::default();
    let signal: Rc<RefCell<Option<Signal<Vec<TreeNodeData>>>>> = Rc::default();
    let state = UseTreeReturn::new(UseTreeOptions {
        initial_expanded: expanded.iter().map(|s| s.to_string()).collect(),
        multiple: opts.multiple,
        ..Default::default()
    });
    let (c, o, r, s, sig) = (
        changes.clone(),
        selects.clone(),
        rested.clone(),
        seen.clone(),
        signal.clone(),
    );
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let (c, o, r, s) = (c.clone(), o.clone(), r.clone(), s.clone());
        let data = Signal::new(initial.clone());
        *sig.borrow_mut() = Some(data);
        let tree = Tree {
            data: data.get(),
            data_source: Some(Rc::new(move || data.get())),
            tree: Some(state),
            select_on_click: opts.select_on_click,
            keyboard: opts.keyboard,
            cursor_delay_ms: 0,
            onselect: Some(ValueCallback::new(move |v| o.borrow_mut().push(v))),
            oncursor: Some(ValueCallback::new(move |v| r.borrow_mut().push(v))),
            onselectionchange: Some(ValueCallback::new(move |v| c.borrow_mut().push(v))),
            handle_key: Some(Rc::new(move |key: &TreeKey| {
                let drop = key.drop_for(TreeMove::Down).map(|d| (d.parent, d.slot));
                s.borrow_mut().push((
                    key.key.clone(),
                    key.below.clone(),
                    key.carried().to_vec(),
                    drop,
                ));
                false
            })),
            ..Default::default()
        }
        .render(scope, &[]);
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 300px");
        root.append_child(&tree);
        root
    });
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let data = signal.borrow().unwrap();
    P {
        app,
        state,
        data,
        changes,
        selects,
        rested,
        seen,
    }
}

fn flat(values: &[&str]) -> Vec<TreeNodeData> {
    values
        .iter()
        .map(|v| TreeNodeData::new(*v, v.to_uppercase()))
        .collect()
}

fn has_class(n: &rinch_dom::Node, class: &str) -> bool {
    n.attributes
        .get("class")
        .is_some_and(|c| c.split_whitespace().any(|one| one == class))
}

/// The connected content box of `value`.
fn row(app: &RinchApp, value: &str) -> u64 {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let li = d
        .tree
        .nodes
        .iter()
        .find(|(_, n)| {
            n.attributes.get("data-value").map(String::as_str) == Some(value) && n.parent.is_some()
        })
        .map(|(id, _)| id)
        .expect("the row");
    d.tree.nodes[li]
        .children
        .iter()
        .copied()
        .find(|&c| has_class(&d.tree.nodes[c], "rinch-tree__node-content"))
        .unwrap()
}

fn centre(app: &RinchApp, id: u64) -> (f32, f32) {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let (x, y, w, h) = hit_testing::painted_element_box(&d.tree, id);
    (x + w / 2.0, y + h / 2.0)
}

fn click(p: &mut P, value: &str, modifiers: Modifiers) {
    let (x, y) = centre(&p.app, row(&p.app, value));
    p.app
        .handle_event(PlatformEvent::ModifiersChanged(modifiers), (800, 600), 1.0);
    for event in [
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        },
    ] {
        p.app.handle_event(event, (800, 600), 1.0);
    }
    p.app.handle_event(
        PlatformEvent::ModifiersChanged(Modifiers::default()),
        (800, 600),
        1.0,
    );
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
}

fn press(p: &mut P, key: KeyCode, text: Option<&str>, modifiers: Modifiers) {
    p.app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers,
            repeat: KeyRepeat::Unknown,
        },
        (800, 600),
        1.0,
    );
    p.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
}

fn sel(p: &P) -> Vec<String> {
    let mut s: Vec<String> = p.state.selected.get().into_iter().collect();
    s.sort();
    s
}

fn shift() -> Modifiers {
    Modifiers {
        shift: true,
        ..Default::default()
    }
}

fn ctrl() -> Modifiers {
    Modifiers {
        ctrl: true,
        ..Default::default()
    }
}

const MULTI: Opts = Opts {
    multiple: true,
    select_on_click: true,
    keyboard: true,
};

/// `multiple`, but not selecting on click.
const MULTI_NO_SELECT: Opts = Opts {
    select_on_click: false,
    ..MULTI
};

/// a (a1, a2), b, c.
fn nested() -> Vec<TreeNodeData> {
    vec![
        TreeNodeData::new("a", "A").with_children(flat(&["a1", "a2"])),
        TreeNodeData::new("b", "B"),
        TreeNodeData::new("c", "C"),
    ]
}

/// a, b (disabled), c, d.
fn with_disabled_b() -> Vec<TreeNodeData> {
    let mut data = flat(&["a", "b", "c", "d"]);
    data[1] = TreeNodeData::new("b", "B").disabled(true);
    data
}

fn last_report(p: &P) -> Vec<String> {
    p.changes.borrow().last().cloned().expect("a report")
}

/// Trees with an app's `drag_values` (the second review of #1598), and
/// single-select controls.
mod custom_trees;
mod selection;

/// `TreeController::set_anchor`: an app whose selection follows the cursor
/// says where the next Shift run starts.
mod set_anchor;
