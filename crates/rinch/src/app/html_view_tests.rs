//! `rinch_html::HtmlView` cleans its document at its own width, and again
//! when a resize crosses one of the document's `@media` breakpoints.

// `rsx!` writes absolute `rinch::` paths, and this *is* the rinch crate.
use super::*;
use rinch_core::Component;
use rinch_macros::rsx;

const VP: (u32, u32) = (1000, 600);

const DOCUMENT: &str = r#"<html><head><style>
    .mob { display: none }
    @media only screen and (max-width: 600px) {
        .mob { display: block !important }
        .desk { display: none !important }
    }
</style></head><body>
    <div class="desk" id="desk">desktop</div><div class="mob" id="mob">mobile</div>
    <a href="https://shop.example/sale" id="link">sale</a>
</body></html>"#;

fn mount(width: &str) -> (RinchApp, NodeHandle) {
    let holder: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let h = holder.clone();
    let first = width.to_owned();
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let host = rsx! { div { id: "host" } };
        host.set_attribute("style", &format!("width: {first}"));
        let view = rinch_html::HtmlView {
            document: DOCUMENT.into(),
            ..Default::default()
        }
        .render(__scope, &[]);
        host.append_child(&view);
        *h.borrow_mut() = Some(host.clone());
        host
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    let host = holder.borrow().clone().unwrap();
    // A layout after the mount publishes the view's width (bounds signals
    // are published by a layout pass, and the mount's own ran before the
    // view's watch); its effect then cleans the document again.
    host.set_attribute("style", &format!("width: {width}; min-height: 1px"));
    for _ in 0..3 {
        app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    }
    (app, host)
}

/// Whether a `<div>` under `host` whose text is `text` is laid out with a
/// height. (The cleaner drops `id`s, so the document's elements are found by
/// text, and only under the host: a node a re-clean removed may stay in the
/// node table, #723.)
fn shown(app: &RinchApp, host: &NodeHandle, text: &str) -> bool {
    let d = app.doc.as_ref().unwrap().borrow();
    let mut stack = vec![host.node_id().0];
    while let Some(id) = stack.pop() {
        let Some(n) = d.tree.get(id) else { continue };
        if n.tag() == Some("div")
            && n.children.len() == 1
            && d.tree.get(n.children[0]).and_then(|c| c.text_content()) == Some(text)
        {
            return n.layout.height > 0.0;
        }
        stack.extend(n.children.iter().copied());
    }
    false
}

#[test]
fn a_narrow_view_gets_the_documents_narrow_layout_and_follows_a_resize() {
    let (mut app, host) = mount("400px");
    assert!(
        shown(&app, &host, "mobile") && !shown(&app, &host, "desktop"),
        "400px: the phone layout"
    );

    host.set_attribute("style", "width: 800px");
    for _ in 0..3 {
        app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    }
    assert!(
        shown(&app, &host, "desktop") && !shown(&app, &host, "mobile"),
        "800px: the desktop layout"
    );
}

#[test]
fn links_carry_their_address_as_a_tooltip() {
    let (app, _) = mount("800px");
    let d = app.doc.as_ref().unwrap().borrow();
    let title = d
        .tree
        .nodes
        .iter()
        .find(|(_, n)| n.tag() == Some("a"))
        .and_then(|(_, n)| n.attributes.get("title").cloned());
    assert_eq!(title.as_deref(), Some("https://shop.example/sale"));
}
