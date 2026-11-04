//! `events::click_link`: which link a click went through on its way to the
//! handler that runs, for HTML the app did not build itself
//! (`set_inner_html`), and `rinch_html::HtmlView`'s `onlink` on top of it.

// `rsx!` writes absolute `rinch::` paths, and this *is* the rinch crate.
use super::hit_testing::painted_element_box;
use super::*;
use crate as rinch;
use rinch_core::{Component, InputCallback};
use rinch_macros::rsx;

const VP: (u32, u32) = (800, 600);

type Log = Rc<RefCell<Vec<Option<String>>>>;

/// A container with a click handler, holding HTML set as a string, then an
/// `HtmlView` with `onlink`.
fn mount() -> (RinchApp, Log, Rc<RefCell<Vec<String>>>) {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let links: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let (l, k) = (log.clone(), links.clone());
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let l = l.clone();
        let host = rsx! {
            div {
                id: "host",
                style: "font-size: 16px; line-height: 20px; font-family: sans-serif",
                onclick: move || l.borrow_mut().push(rinch_core::click_link()),
            }
        };
        host.set_inner_html(
            r#"<p><a id="plain" href="https://one.example/" style="display: inline-block; width: 200px">one</a></p>
               <p><span id="text" style="display: inline-block; width: 200px">no link here</span></p>
               <p><a href="mailto:two@example.org"><b><span id="deep" style="display: inline-block; width: 200px">two</span></b></a></p>"#,
        );
        let k = k.clone();
        let view = rinch_html::HtmlView {
            html: r#"<p><a href="https://three.example/"><span id="view-link" style="display: inline-block; width: 200px">three</span></a></p>
                     <p><span id="view-text" style="display: inline-block; width: 200px">plain</span></p>"#
                .into(),
            body_style: String::new(),
            onlink: Some(InputCallback::new(move |href: String| {
                k.borrow_mut().push(href)
            })),
            ..Default::default()
        }
        .render(__scope, &[]);
        let root = rsx! { div {} };
        root.append_child(&host);
        root.append_child(&view);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    (app, log, links)
}

fn click(app: &mut RinchApp, wanted: &str) {
    let (x, y) = {
        let d = app.doc.as_ref().unwrap().borrow();
        let nid = d
            .tree
            .nodes
            .iter()
            .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some(wanted))
            .map(|(id, _)| id)
            .unwrap_or_else(|| panic!("no #{wanted}"));
        let (x, y, w, h) = painted_element_box(&d.tree, nid);
        assert!(w > 8.0 && h > 8.0, "#{wanted} has a box: {w}x{h}");
        (x + 4.0, y + h / 2.0)
    };
    let button = MouseButton::Left;
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
}

#[test]
fn a_container_handler_learns_which_link_was_clicked() {
    let (mut app, log, _) = mount();
    click(&mut app, "plain");
    click(&mut app, "text");
    click(&mut app, "deep");
    assert_eq!(
        *log.borrow(),
        vec![
            Some("https://one.example/".to_owned()),
            None,
            Some("mailto:two@example.org".to_owned()),
        ]
    );
    assert_eq!(
        rinch_core::click_link(),
        None,
        "cleared once the handler has run"
    );
}

#[test]
fn html_view_calls_onlink_for_a_link_only() {
    let (mut app, _, links) = mount();
    click(&mut app, "view-text");
    click(&mut app, "view-link");
    assert_eq!(*links.borrow(), vec!["https://three.example/".to_owned()]);
}
