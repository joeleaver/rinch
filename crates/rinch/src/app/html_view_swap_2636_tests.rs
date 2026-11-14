//! #2636: swapping an `HtmlView`'s document restyles the view, not the window.
//!
//! An app shows one email at a time in an `HtmlView` keyed on the email. Each
//! switch unmounts one view and mounts another, and with them the view's own
//! `<style>`. Its rules were `:where(.rinch-html-view…)`, which Stylo's
//! stylesheet invalidation cannot bucket, so every switch restyled the whole
//! window twice (`full_restyle_stylesheet`). And `sync_style_element_sheets`
//! re-added every `<style>` sheet after the first one that changed, so an app
//! `<style>` later in the tree, with a `:root` rule, did the same.
//!
//! These pin what the switch must still get right: the new document is styled
//! by its own sheet (applied inline by the cleaner), none of a document's
//! rules — `*`, `:root`, `div` — reach the app's elements beside the view, the
//! view's own rules still apply, and switching back and forth lands on each
//! document's styles every time. The cost of the switch is pinned in
//! `perf_regression_tests::html_view_swap_restyles_the_views_not_the_window`.

// `rsx!` writes absolute `rinch::` paths, and this *is* the rinch crate.
use super::*;
use rinch_core::{Component, Signal};
use rinch_macros::rsx;

const VP: (f32, f32) = (1000.0, 600.0);

/// Two emails whose sheets are full of rules a browser would apply to the
/// whole page: `*`, `:root`, a bare tag. The cleaner applies them inline to
/// the document's own elements; none must reach the app.
pub(super) const DOCS: [&str; 2] = [
    r#"<html><head><style>
        :root { color-scheme: light }
        * { margin: 0 }
        div { color: rgb(200, 0, 0) }
        .a { padding-left: 11px }
    </style></head><body><div class="a">alpha</div></body></html>"#,
    r#"<html><head><style>
        * { margin-left: 13px }
        div { color: rgb(0, 0, 200) }
        .b { padding-left: 17px }
    </style></head><body><div class="b">beta</div></body></html>"#,
];

/// The app's own sheet: a colour for its sidebar row, nothing for `.plain`.
const APP_CSS: &str = "body { margin: 0 } .side { color: rgb(0, 128, 0); margin-left: 3px }";

/// An app with a sidebar row, a plain `div` (which the documents' `*` and
/// `div` rules would match if they leaked) and an `HtmlView` keyed on
/// `which`.
fn mount(which: Signal<usize>) -> RinchApp {
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        rsx! {
            div {
                style { {APP_CSS} }
                div { class: "side", "sidebar" }
                div { class: "plain", "plain" }
                div { class: "slot",
                    for d in vec![which.get()] {
                        div { key: d,
                            {
                                rinch_html::HtmlView {
                                    document: DOCS[d].into(),
                                    ..Default::default()
                                }
                                .render(__scope, &[])
                            }
                        }
                    }
                }
            }
        }
    });
    app.mount_component(VP.0, VP.1);
    settle(&mut app);
    app
}

fn settle(app: &mut RinchApp) {
    for _ in 0..3 {
        app.resolve_and_repaint(VP.0, VP.1);
    }
}

/// The connected element whose only child is the text `text`.
fn by_text(app: &RinchApp, text: &str) -> Option<u64> {
    let d = app.doc.as_ref().unwrap().borrow();
    let mut stack = vec![d.tree.root_id];
    while let Some(id) = stack.pop() {
        let Some(n) = d.tree.get(id) else { continue };
        if n.children.len() == 1
            && d.tree.get(n.children[0]).and_then(|c| c.text_content()) == Some(text)
        {
            return Some(id);
        }
        stack.extend(n.children.iter().copied());
    }
    None
}

/// `(color, padding-left, margin-left)` of the element whose text is `text`.
fn style_of(app: &RinchApp, text: &str) -> (Option<(u8, u8, u8)>, String, String) {
    let id = by_text(app, text).unwrap_or_else(|| panic!("no element reads {text:?}"));
    let d = app.doc.as_ref().unwrap().borrow();
    let s = &d.tree.get(id).unwrap().computed_style;
    (
        s.color.map(|c| c.to_rgba8()).map(|c| (c.r, c.g, c.b)),
        format!("{:?}", s.padding_left).replace("Zero", "Length(0.0)"),
        format!("{:?}", s.margin_left),
    )
}

/// The view's own element's `user-select`, from its own (layered) rule.
fn view_user_select(app: &RinchApp) -> String {
    let d = app.doc.as_ref().unwrap().borrow();
    let mut stack = vec![d.tree.root_id];
    while let Some(id) = stack.pop() {
        let Some(n) = d.tree.get(id) else { continue };
        if n.attributes
            .get("class")
            .is_some_and(|c| c.split_whitespace().any(|c| c == "rinch-html-view"))
        {
            return format!("{:?}", n.computed_style.user_select);
        }
        stack.extend(n.children.iter().copied());
    }
    panic!("no view")
}

/// The app's elements beside the view, as the app's sheet alone styles them.
fn assert_app_untouched(app: &RinchApp, when: &str) {
    assert_eq!(
        style_of(app, "sidebar"),
        (
            Some((0, 128, 0)),
            "Length(0.0)".to_owned(),
            "Length(3.0)".to_owned()
        ),
        "{when}: the sidebar row keeps the app's rule"
    );
    assert_eq!(
        style_of(app, "plain"),
        (
            Some((0, 0, 0)),
            "Length(0.0)".to_owned(),
            "Length(0.0)".to_owned()
        ),
        "{when}: no document's `*` or `div` rule reaches the app's plain div"
    );
}

fn assert_shows(app: &RinchApp, which: usize, when: &str) {
    let (shown, gone, expect) = if which == 0 {
        (
            "alpha",
            "beta",
            (
                Some((200, 0, 0)),
                "Length(11.0)".to_owned(),
                "Length(0.0)".to_owned(),
            ),
        )
    } else {
        (
            "beta",
            "alpha",
            (
                Some((0, 0, 200)),
                "Length(17.0)".to_owned(),
                "Length(13.0)".to_owned(),
            ),
        )
    };
    assert_eq!(
        style_of(app, shown),
        expect,
        "{when}: the document is styled by its own sheet"
    );
    assert!(
        by_text(app, gone).is_none(),
        "{when}: the other document is gone"
    );
    assert_eq!(
        view_user_select(app),
        "Text",
        "{when}: the view's own layered rule applies"
    );
}

#[test]
fn swapping_documents_styles_each_and_leaves_the_app_alone() {
    let which = Signal::new(0usize);
    let mut app = mount(which);
    assert_shows(&app, 0, "first document");
    assert_app_untouched(&app, "first document");

    for (step, next) in [1, 0, 1, 0].into_iter().enumerate() {
        which.set(next);
        settle(&mut app);
        let when = format!("swap {} to document {next}", step + 1);
        assert_shows(&app, next, &when);
        assert_app_untouched(&app, &when);
    }
}
