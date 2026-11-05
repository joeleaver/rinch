//! Read-only selection gestures as a browser has them: a double click
//! selects a word, a triple click the paragraph, Shift+click extends, Select
//! All takes all the text a selection is in, and a link inside selectable
//! text follows on a click but selects on a drag.

use super::text_selection_across_tests::{THREE, at, drag, highlighted, mount, press, release};
use super::*;
use rinch_core::{Component, InputCallback};

const VP: (u32, u32) = (800, 600);
const SELECTABLE: &str = "user-select: text; font: 16px/20px sans-serif; width: 600px";

fn click(app: &mut RinchApp, p: (f32, f32), times: usize) {
    for _ in 0..times {
        press(app, p);
        release(app, p);
    }
}

fn modifiers(app: &mut RinchApp, m: Modifiers) {
    app.handle_event(PlatformEvent::ModifiersChanged(m), VP, 1.0);
}

/// A point in the word "paragraph" of `#a` ("first paragraph").
fn in_paragraph_word(app: &RinchApp) -> (f32, f32) {
    let (x, y) = at(app, "a", 0.0);
    (x + 60.0, y)
}

#[test]
fn a_double_click_selects_the_word_and_a_triple_click_the_paragraph() {
    let mut app = mount(THREE, SELECTABLE);
    let p = in_paragraph_word(&app);
    click(&mut app, p, 2);
    assert_eq!(app.text_selection_text().unwrap(), "paragraph");
    // The jitter after a click does not collapse it.
    drag(&mut app, (p.0 + 1.0, p.1));
    assert_eq!(app.text_selection_text().unwrap(), "paragraph");
    click(&mut app, p, 1);
    assert_eq!(app.text_selection_text().unwrap(), "first paragraph");
}

#[test]
fn shift_click_extends_the_selection_to_the_press() {
    let mut app = mount(THREE, SELECTABLE);
    {
        let p = at(&app, "a", 0.0);
        click(&mut app, p, 1);
    }
    // Far enough from the last press not to count as a second click.
    modifiers(
        &mut app,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    {
        let p = at(&app, "c", 0.99);
        click(&mut app, p, 1);
    }
    modifiers(&mut app, Modifiers::default());
    assert_eq!(highlighted(&app, "b"), Some((0, "second paragraph".len())));
    assert_eq!(
        app.text_selection_text().unwrap(),
        "first paragraph\nsecond paragraph\nthird paragraph"
    );
}

#[test]
fn select_all_takes_all_the_text_a_selection_is_in() {
    // A click in the text (an empty selection) is enough to go by.
    let mut app = mount(THREE, SELECTABLE);
    {
        let p = at(&app, "b", 0.3);
        click(&mut app, p, 1);
    }
    let ctrl = Modifiers {
        ctrl: true,
        ..Modifiers::default()
    };
    modifiers(&mut app, ctrl);
    app.handle_event(
        PlatformEvent::KeyDown {
            key: KeyCode::KeyA,
            logical_key: None,
            text: None,
            modifiers: ctrl,
            repeat: KeyRepeat::Unknown,
        },
        VP,
        1.0,
    );
    assert_eq!(
        app.text_selection_text().unwrap(),
        "first paragraph\nsecond paragraph\nthird paragraph"
    );
}

/// An `HtmlView` holding a paragraph with a link in it; returns what its
/// `onlink` is given.
fn mail_with_a_link() -> (RinchApp, Rc<RefCell<Vec<String>>>) {
    let links: Rc<RefCell<Vec<String>>> = Rc::default();
    let l = links.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "font: 16px/20px sans-serif; width: 600px");
        let l = l.clone();
        let view = rinch_html::HtmlView {
            html: r#"<div>Read <a href="https://shop.example/">the whole story</a> online.</div>"#
                .into(),
            onlink: Some(InputCallback::new(move |href: String| {
                l.borrow_mut().push(href)
            })),
            ..Default::default()
        }
        .render(scope, &[]);
        view.set_attribute("id", "view");
        root.append_child(&view);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    (app, links)
}

#[test]
fn a_click_on_a_link_in_selectable_text_follows_it() {
    let (mut app, links) = mail_with_a_link();
    let (x, y) = at(&app, "view", 0.0);
    // "Read " is about 36px; the link text follows it.
    let on_link = (x + 70.0, y);
    press(&mut app, on_link);
    eprintln!(
        "held={} sel={:?}",
        app.held_link_click.is_some(),
        app.text_selection
            .as_ref()
            .map(|s| (s.ifc_node_id, s.anchor_offset, s.focus_offset))
    );
    assert!(links.borrow().is_empty(), "not yet: a drag would select");
    release(&mut app, on_link);
    assert_eq!(*links.borrow(), vec!["https://shop.example/".to_owned()]);
}

#[test]
fn a_drag_across_a_link_selects_it_and_follows_nothing() {
    let (mut app, links) = mail_with_a_link();
    let (x, y) = at(&app, "view", 0.0);
    press(&mut app, (x + 70.0, y));
    drag(&mut app, (x + 400.0, y));
    release(&mut app, (x + 400.0, y));
    assert!(
        links.borrow().is_empty(),
        "a drag selects; it follows nothing"
    );
    let text = app.text_selection_text().unwrap();
    assert!(text.contains("story") && text.contains("online"), "{text}");
}
