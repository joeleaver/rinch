//! #2357: the app's style sheets stop at an `HtmlView`'s edge.
//!
//! A document in an `HtmlView` is laid out as a browser lays out mail put in
//! a `<div>` of a page that has no style sheet of its own: only the UA sheet,
//! the document's presentational attributes and its own (inlined) styles
//! match its elements, while the host's inherited properties (`font-*`,
//! `line-height`, …) still flow in. The theme's reset (`* { box-sizing:
//! border-box }`, `h1`–`h6 { line-height; font-size; margin }`) used to reach
//! the document and made mail 19–94px taller than in Chrome.
//!
//! `HtmlView { app_styles: true }` lets the app's sheets match again.
//!
//! Chrome 156's numbers for the repro (the issue's table): `#c` is 282.66px
//! wide (200pt content box, plus 6pt of padding each side); `#h` is 14px
//! high (its inherited `line-height: 1` at 14px).

// `rsx!` writes absolute `rinch::` paths, and this *is* the rinch crate.
use super::*;
use rinch_core::Component;
use rinch_macros::rsx;

const VP: (f32, f32) = (1000.0, 600.0);

/// The issue's synthetic repro, given to the view as cleaned HTML (`html`),
/// so the ids survive. Every text measure here has a declared `font-size`
/// and `line-height`.
const REPRO: &str = r#"<table style="border-collapse:collapse"><tr><td id="c" style="width:200pt; padding:0 6pt; height:10px"></td></tr></table><div style="font-size:14px; line-height:1"><h5 id="h" style="margin:0; font-size:14px">Heading</h5></div><h3 id="t" style="margin:0">Title</h3><p id="p" style="margin:0">plain</p><a id="a" href="https://example.com/">link</a>"#;

/// The host's inherited properties, as Postrinch's `.mail-html` sets them.
/// (`color` too, which the view's own `color: #000` replaces.)
const HOST_STYLE: &str = "font-family: monospace; font-size: 13px; line-height: 2; color: #ff0000";

/// Mounts the repro in an `HtmlView` inside a host `div` carrying
/// [`HOST_STYLE`], under the real theme and component sheets.
fn mount(app_styles: bool) -> RinchApp {
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let host = rsx! { div { id: "host" } };
        host.set_attribute("style", HOST_STYLE);
        let view = rinch_html::HtmlView {
            html: REPRO.into(),
            app_styles,
            ..Default::default()
        }
        .render(__scope, &[]);
        host.append_child(&view);
        host
    });
    app.mount_component(VP.0, VP.1);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VP.0, VP.1);
    app
}

/// The node whose `id` attribute is `id`.
fn node<'a>(app: &'a RinchApp, id: &str) -> std::cell::Ref<'a, rinch_dom::Node> {
    let d = app.doc.as_ref().unwrap().borrow();
    std::cell::Ref::map(d, |d| {
        d.tree
            .nodes
            .iter()
            .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some(id))
            .map(|(_, n)| n)
            .unwrap_or_else(|| panic!("no node #{id}"))
    })
}

/// The node's computed `color`, as `(r, g, b)`.
fn rgb(app: &RinchApp, id: &str) -> Option<(u8, u8, u8)> {
    node(app, id)
        .computed_style
        .color
        .map(|c| c.to_rgba8())
        .map(|c| (c.r, c.g, c.b))
}

#[test]
fn the_theme_reset_does_not_reach_the_document() {
    let app = mount(false);
    let c = node(&app, "c");
    assert!(
        !c.computed_style.box_sizing_border_box,
        "the theme's `* {{ box-sizing: border-box }}` matched inside the view"
    );
    // 200pt = 266.67px of content, plus 2 × 6pt = 16px of padding.
    assert!(
        (c.layout.width - 282.67).abs() < 1.0,
        "#c is {} wide; Chrome: 282.66",
        c.layout.width
    );
    drop(c);
    let h = node(&app, "h");
    assert!(
        (h.layout.height - 14.0).abs() < 0.5,
        "#h is {} high; Chrome: 14 (its inherited `line-height: 1`)",
        h.layout.height
    );
    drop(h);
    // The UA heading scale, not the theme's: 1.17em of the host's 13px.
    let t = node(&app, "t");
    assert!(
        (t.computed_style.font_size - 13.0 * 1.17).abs() < 0.01,
        "#t font-size {}; the UA sheet's 1.17em of 13px is 15.21",
        t.computed_style.font_size
    );
}

#[test]
fn the_hosts_inherited_properties_still_reach_the_document() {
    let app = mount(false);
    let p = node(&app, "p");
    let s = &p.computed_style;
    assert_eq!(s.font_size, 13.0, "font-size inherits from the host");
    assert!(
        s.font_family.contains("monospace"),
        "font-family inherits from the host: {:?}",
        s.font_family
    );
    assert!(
        matches!(
            s.line_height,
            rinch_dom::computed_style::LineHeightValue::Relative(v) if v == 2.0
        ),
        "line-height inherits from the host: {:?}",
        s.line_height
    );
    drop(p);
    // The view itself is still styled by its own sheet (and an app's): its
    // `color: #000` replaces the host's red, and the document inherits it.
    assert_eq!(rgb(&app, "p"), Some((0, 0, 0)));
}

#[test]
fn a_link_in_the_document_gets_the_browsers_link_style() {
    let app = mount(false);
    assert_eq!(rgb(&app, "a"), Some((0, 0, 0xee)), "Chrome's -webkit-link");
    assert!(node(&app, "a").computed_style.text_decoration.underline);

    // Under `app_styles`, the theme's `a { text-decoration: none }` and its
    // primary colour, as before #2357.
    let app = mount(true);
    assert_ne!(rgb(&app, "a"), Some((0, 0, 0xee)));
    assert!(!node(&app, "a").computed_style.text_decoration.underline);
}

#[test]
fn app_styles_lets_the_theme_match_inside_the_view() {
    let app = mount(true);
    let c = node(&app, "c");
    assert!(c.computed_style.box_sizing_border_box);
    // The theme's border-box: 200pt including the padding.
    assert!(
        (c.layout.width - 266.67).abs() < 1.0,
        "#c is {} wide; themed: 266.67",
        c.layout.width
    );
    drop(c);
    let h = node(&app, "h");
    assert!(
        (h.layout.height - 21.0).abs() < 0.5,
        "#h is {} high; themed: 21 (the theme's `line-height: 1.5`)",
        h.layout.height
    );
}
