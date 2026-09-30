//! `data-nofocus` written on a **component** keeps an editor's focus (#433).
//!
//! `data-nofocus` (#312) exists to be written once on a toolbar *container*,
//! and toolbars are `Group`/`Paper`/`Stack` components — which `rsx!` could not
//! give a hyphenated attribute, so the ui-zoo editor toolbar carried a wrapper
//! `div` for it. These drive the real macro output through the real press path:
//! `Group { data-nofocus: {move || on}, button { … } }` (a closure so one
//! fixture serves both halves; the literal form is pinned on the mock in
//! `rinch-macros/tests/rsx_component_attrs.rs`), editor focused, press the
//! button.

// `rsx!` writes absolute `rinch::` paths, and this *is* the rinch crate.
use super::*;
use crate as rinch;
use rinch_components::Group;
use rinch_macros::rsx;
use std::cell::Cell;

const VP: (u32, u32) = (800, 600);

struct Fixture {
    app: RinchApp,
    button: usize,
    editor: usize,
    clicks: Rc<Cell<u32>>,
}

/// A `Group` toolbar holding one `<button>` (a Tab stop by tag, #252) above an
/// editor. `nofocus` says whether the caller writes `data-nofocus` on the Group.
fn mount(nofocus: bool) -> Fixture {
    let clicks = Rc::new(Cell::new(0u32));
    let ids: Rc<Cell<Option<(usize, usize)>>> = Rc::new(Cell::new(None));
    let (clicks_in, ids_in) = (clicks.clone(), ids.clone());
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let clicks = clicks_in.clone();
        let toolbar = rsx! {
            Group { data-nofocus: {move || nofocus},
                button {
                    style: "width: 60px; height: 30px",
                    onclick: move || clicks.set(clicks.get() + 1),
                    "B"
                }
            }
        };
        let button = toolbar.children()[0].clone();
        assert_eq!(button.tag_name().as_deref(), Some("button"), "fixture shape");
        let (editor, handle) = crate::editor::mount_editor(__scope);
        handle.load_html("<p>hello world</p>");
        editor.set_attribute("style", "width: 400px; height: 100px");
        let root = __scope.create_element("div");
        root.append_child(&toolbar);
        root.append_child(&editor);
        ids_in.set(Some((button.node_id().0, editor.node_id().0)));
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let (button, editor) = ids.get().expect("ids captured at mount");
    Fixture {
        app,
        button,
        editor,
        clicks,
    }
}

fn press(app: &mut RinchApp, id: usize) {
    let (x, y, w, h) = {
        let d = app.doc.as_ref().unwrap().borrow();
        painted_element_box(&d.tree, id)
    };
    let (x, y) = (x + w / 2.0, y + h / 2.0);
    let button = MouseButton::Left;
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
}

fn focus_editor(f: &mut Fixture) {
    press(&mut f.app, f.editor);
    assert_eq!(
        f.app.focus_target,
        FocusTarget::Editor(f.editor),
        "positive control: a press in the editor focuses it"
    );
}

/// The issue's shape: the attribute on the Group, not on a wrapper, and the
/// button inside it takes the click without taking the keyboard.
#[test]
fn data_nofocus_on_a_group_toolbar_keeps_the_editor_focused() {
    let mut f = mount(true);
    focus_editor(&mut f);

    press(&mut f.app, f.button);

    assert_eq!(
        f.app.focus_target,
        FocusTarget::Editor(f.editor),
        "the editor keeps the keyboard, so Bold has a selection to act on"
    );
    assert_eq!(f.clicks.get(), 1, "and the click still fired");
}

/// The contrast: the same Group with the attribute written false (removed)
/// lets the button take the keyboard — so the test above is measuring the
/// attribute and not something else about the fixture.
#[test]
fn without_it_the_same_toolbar_button_takes_the_keyboard() {
    let mut f = mount(false);
    focus_editor(&mut f);

    press(&mut f.app, f.button);

    assert_eq!(f.app.focus_target, FocusTarget::Node(f.button));
    assert_eq!(f.clicks.get(), 1);
}
