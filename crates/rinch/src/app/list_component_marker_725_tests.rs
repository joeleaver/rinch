//! #725: the component `List` with the real theme and component CSS. An item
//! with an icon is `display: flex`, so it draws no generated marker beside
//! its icon (it used to draw a bullet there); a plain `List` keeps its
//! bullets and an ordered one its numbers.
use super::*;
use crate as rinch;
use rinch_components::list::{List, ListItem};
use rinch_macros::rsx;
use rinch_tabler_icons::TablerIcon;

const VIEWPORT: (f32, f32) = (800.0, 600.0);

fn mount(build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> RinchApp {
    let mut app = RinchApp::new(build);
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
    app
}

/// (li node, its pseudo-children's text, display) for every `li` under body.
fn items(app: &RinchApp) -> Vec<(usize, Vec<String>, String)> {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let mut out = Vec::new();
    fn walk(d: &RinchDocument, id: usize, out: &mut Vec<(usize, Vec<String>, String)>) {
        let Some(n) = d.tree.get(id) else { return };
        if n.tag() == Some("li") {
            let ps = n
                .children
                .iter()
                .filter(|&&c| d.tree.get(c).is_some_and(|c| c.is_pseudo_element))
                .map(|&c| rinch_dom::testing::get_text_content(&d.tree, c))
                .collect();
            out.push((id, ps, format!("{:?}", n.computed_style.display)));
        }
        for c in n.children.clone() {
            walk(d, c, out);
        }
    }
    walk(&d, d.tree.body_id, &mut out);
    out
}

#[test]
fn list_with_icon_draws_no_bullet_beside_the_icon() {
    let app = mount(move |__scope: &mut RenderScope| {
        rsx! { div { List { icon: TablerIcon::Check, ListItem { "one" } ListItem { "two" } } } }
    });
    let it = items(&app);
    eprintln!("icon list items: {it:?}");
    assert_eq!(it.len(), 2);
    for (_, ps, _) in &it {
        assert!(ps.is_empty(), "no generated marker beside the icon: {ps:?}");
    }
}

#[test]
fn plain_list_still_has_bullets() {
    let app = mount(move |__scope: &mut RenderScope| {
        rsx! { div { List { ListItem { "one" } ListItem { "two" } } } }
    });
    let it = items(&app);
    eprintln!("plain list items: {it:?}");
    assert_eq!(it.len(), 2);
    for (_, ps, d) in &it {
        assert_eq!(d, "ListItem");
        assert_eq!(ps, &vec!["\u{2022}\u{2002}".to_string()]);
    }
}

#[test]
fn ordered_list_numbers() {
    let app = mount(move |__scope: &mut RenderScope| {
        rsx! { div { List { r#type: "ordered", ListItem { "one" } ListItem { "two" } } } }
    });
    let it = items(&app);
    eprintln!("ordered list items: {it:?}");
    assert_eq!(it[1].1, vec!["2.\u{2002}".to_string()]);
}
