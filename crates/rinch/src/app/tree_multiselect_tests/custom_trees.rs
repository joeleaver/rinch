use super::*;
use rinch_components::DragValues;

type Customize = Rc<dyn Fn(&mut Tree, UseTreeReturn)>;

fn build_with(opts: Opts, initial: Vec<TreeNodeData>, expanded: &[&str], custom: Customize) -> P {
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
        let mut tree = Tree {
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
        };
        custom(&mut tree, state);
        let tree = tree.render(scope, &[]);
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

const SINGLE: Opts = Opts {
    multiple: false,
    ..MULTI
};

/// The documented whole-selection `drag_values` ("an app that drags its
/// whole selection") carries the selection once, not once a row.
#[test]
fn a_whole_selection_drag_values_carries_the_selection_once() {
    let custom: Customize = Rc::new(|tree, state| {
        let dv: DragValues = Rc::new(move |_v: &str| {
            let mut s: Vec<String> = state.selected.get().into_iter().collect();
            s.sort();
            s
        });
        tree.drag_values = Some(dv);
    });
    let mut p = build_with(MULTI, flat(&["a", "b", "c", "d"]), &[], custom);
    click(&mut p, "a", Modifiers::default());
    click(&mut p, "c", ctrl());
    press(&mut p, KeyCode::F2, None, Modifiers::default());
    let seen = p.seen.borrow();
    assert_eq!(seen[0].2, ["a", "c"], "carried");
}

/// A single-select tree whose `drag_values` names app ids (not row
/// values) still moves the cursor row (#1601).
#[test]
fn drop_for_moves_the_cursor_row_whatever_drag_values_names() {
    let custom: Customize = Rc::new(|tree, _| {
        tree.drag_values = Some(Rc::new(|v: &str| vec![format!("id:{v}")]));
    });
    let mut p = build_with(SINGLE, flat(&["a", "b", "c", "d"]), &[], custom);
    click(&mut p, "b", Modifiers::default());
    press(&mut p, KeyCode::F2, None, Modifiers::default());
    let seen = p.seen.borrow();
    assert_eq!(seen[0].2, ["id:b"], "carried is the app's");
    assert_eq!(seen[0].3, Some((None, TreeSlot::After("c".into()))));
}

/// The block is the selected rows in tree order, whatever order
/// `drag_values` hands back: Down of {b, c} lands after d.
#[test]
fn a_block_is_in_tree_order_whatever_drag_values_returns() {
    let custom: Customize = Rc::new(|tree, _| {
        tree.drag_values = Some(Rc::new(|v: &str| {
            if v == "b" || v == "c" {
                vec!["c".into(), "b".into()]
            } else {
                vec![v.into()]
            }
        }));
    });
    let mut p = build_with(MULTI, flat(&["a", "b", "c", "d", "e"]), &[], custom);
    click(&mut p, "b", Modifiers::default());
    click(&mut p, "c", ctrl());
    press(&mut p, KeyCode::F2, None, Modifiers::default());
    let seen = p.seen.borrow();
    assert_eq!(seen[0].2, ["c", "b"], "carried is the app's order");
    assert_eq!(seen[0].3, Some((None, TreeSlot::After("d".into()))));
}

/// Single-select: Shift+Space and Enter select as before, and
/// `onselectionchange` is never called.
#[test]
fn single_select_enter_and_shift_space_are_unchanged() {
    let mut p = build(SINGLE, flat(&["a", "b", "c"]), &[]);
    click(&mut p, "a", Modifiers::default());
    press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
    press(&mut p, KeyCode::Space, Some(" "), shift());
    press(&mut p, KeyCode::ArrowDown, None, Modifiers::default());
    press(&mut p, KeyCode::Enter, None, Modifiers::default());
    assert_eq!(sel(&p), ["c"]);
    assert!(p.changes.borrow().is_empty());
}

/// Shift+letter is type-ahead of a capital: it moves the cursor and
/// selects nothing.
#[test]
fn a_shift_letter_is_type_ahead() {
    let mut p = build(MULTI, flat(&["a", "b", "c"]), &[]);
    click(&mut p, "a", Modifiers::default());
    press(&mut p, KeyCode::KeyC, Some("C"), shift());
    assert_eq!(p.state.cursor.get().as_deref(), Some("c"));
    assert_eq!(sel(&p), ["a"]);
}

/// Ctrl- and Shift-clicks change the selection without telling
/// `onselect`.
#[test]
fn ctrl_and_shift_clicks_do_not_tell_onselect() {
    let mut p = build(MULTI, flat(&["a", "b", "c"]), &[]);
    click(&mut p, "a", Modifiers::default());
    click(&mut p, "c", ctrl());
    click(&mut p, "b", shift());
    assert_eq!(sel(&p), ["b", "c"]);
    assert_eq!(*p.selects.borrow(), ["a"]);
}
