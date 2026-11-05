//! Nothing in an `HtmlView` paints or takes a click outside its own box
//! (#2213 review): mail that moves itself up over the app's header cannot
//! turn a click on the header into a click on its link.

// `rsx!` writes absolute `rinch::` paths, and this *is* the rinch crate.
use super::*;
use crate as rinch;
use rinch_core::{Component, InputCallback};
use rinch_macros::rsx;
use std::cell::Cell;

const VP: (u32, u32) = (800, 600);

/// A 100px header with its own click handler, and under it an `HtmlView`
/// holding `html` (a link the mail tries to lift over the header) with its
/// body styled `body_style`. Returns the header's clicks and `onlink`'s.
fn click_the_header(html: &str, body_style: &str) -> (u32, Vec<String>) {
    let header_clicks = Rc::new(Cell::new(0u32));
    let links: Rc<RefCell<Vec<String>>> = Rc::default();
    let (h, l) = (header_clicks.clone(), links.clone());
    let (html, body_style) = (html.to_owned(), body_style.to_owned());
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let h = h.clone();
        let l = l.clone();
        let header = rsx! {
            div {
                style: "height: 100px; background: #eee",
                onclick: move || h.set(h.get() + 1),
            }
        };
        let view = rinch_html::HtmlView {
            html: html.clone(),
            body_style: body_style.clone(),
            onlink: Some(InputCallback::new(move |href: String| {
                l.borrow_mut().push(href)
            })),
            ..Default::default()
        }
        .render(__scope, &[]);
        let root = rsx! { div {} };
        root.append_child(&header);
        root.append_child(&view);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let (x, y, button) = (400.0, 50.0, MouseButton::Left);
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
    (header_clicks.get(), links.take())
}

/// A link 800x100, styled `style`.
fn link(style: &str) -> String {
    format!(
        r#"<a href="https://evil.example/" style="display: block; width: 800px; height: 100px; {style}">claim your prize</a>"#
    )
}

#[test]
fn mail_cannot_move_a_link_over_the_app() {
    for (html, body) in [
        (link("position: relative; top: -100px"), ""),
        (link("transform: translateY(-100px)"), ""),
        (link("margin-top: -100px"), ""),
        (link(""), "position: relative; top: -100px"),
    ] {
        let (header, links) = click_the_header(&html, body);
        assert_eq!(
            (header, links),
            (1, vec![]),
            "the header got the click: {html} / body {body}"
        );
    }
}
