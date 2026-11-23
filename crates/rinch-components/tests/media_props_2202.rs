//! `Anchor`, `Image` and `Avatar` props from the configurability audit
//! (issues #2206, #2203, #2202), read as markup on the mock document.
//!
//! A picture's load failure is the `<img>`'s `data-onerror` handler, which
//! both runtimes fire (the desktop after layout, the web from the browser's
//! `error` event; `rinch`'s `img_error_2202_tests` drives the desktop end to
//! end). Here it is dispatched by hand, as either runtime would.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch_components::anchor::Anchor;
use rinch_components::avatar::Avatar;
use rinch_components::image::Image;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::events::{EventHandlerId, dispatch_event};
use rinch_core::{Callback, Component};
use rinch_tabler_icons::TablerIcon;

mod common;
use common::{find_by_class, glyph};

struct Mounted {
    _doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    root: NodeHandle,
}

impl Mounted {
    fn build(build: impl FnOnce(&mut RenderScope) -> NodeHandle) -> Self {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let root = build(&mut scope);
        Self {
            _doc: doc,
            _scope: scope,
            root,
        }
    }
}

fn counter() -> (Rc<Cell<u32>>, Callback) {
    let n = Rc::new(Cell::new(0));
    let m = n.clone();
    (n, Callback::new(move || m.set(m.get() + 1)))
}

/// The `<img>` in `node`'s subtree, if any.
fn find_img(node: &NodeHandle) -> Option<NodeHandle> {
    if node
        .tag_name()
        .is_some_and(|t| t.eq_ignore_ascii_case("img"))
    {
        return Some(node.clone());
    }
    node.children().iter().find_map(find_img)
}

fn onerror(img: &NodeHandle) -> EventHandlerId {
    EventHandlerId(
        img.get_attribute("data-onerror")
            .expect("the <img> carries an onerror handler")
            .parse()
            .unwrap(),
    )
}

// ------------------------------------------------------------------ Anchor

/// #2206: no `href` writes none (a placeholder link); it used to be `#`.
#[test]
fn an_anchor_without_href_writes_none() {
    let m = Mounted::build(|s| Anchor::default().render(s, &[]));
    assert_eq!(m.root.get_attribute("href"), None);
    let m = Mounted::build(|s| {
        Anchor {
            href: "/docs".into(),
            ..Default::default()
        }
        .render(s, &[])
    });
    assert_eq!(m.root.get_attribute("href").as_deref(), Some("/docs"));
}

/// #2206: `rel` is written as given, and not at all when empty.
#[test]
fn an_anchor_writes_its_rel() {
    let m = Mounted::build(|s| {
        Anchor {
            href: "https://example.com".into(),
            target: "_blank".into(),
            rel: "noreferrer".into(),
            ..Default::default()
        }
        .render(s, &[])
    });
    assert_eq!(m.root.get_attribute("rel").as_deref(), Some("noreferrer"));
    assert_eq!(m.root.get_attribute("target").as_deref(), Some("_blank"));
    let m = Mounted::build(|s| Anchor::default().render(s, &[]));
    assert_eq!(m.root.get_attribute("rel"), None);
}

/// #2206: `onclick` is the anchor's click handler, and claims the link on
/// the web (keeping the browser's link menu); without one nothing is claimed.
#[test]
fn an_anchor_runs_its_onclick_and_claims_the_link() {
    let (clicks, cb) = counter();
    let m = Mounted::build(|s| {
        Anchor {
            href: "/settings".into(),
            onclick: Some(cb),
            ..Default::default()
        }
        .render(s, &[])
    });
    let rid: usize = m.root.get_attribute("data-rid").unwrap().parse().unwrap();
    assert!(dispatch_event(EventHandlerId(rid)));
    assert_eq!(clicks.get(), 1);
    assert_eq!(
        m.root.get_attribute("data-claim-links").as_deref(),
        Some("menu")
    );
    // It has an `href`: a link, focusable as one, no button role.
    assert_eq!(m.root.get_attribute("tabindex"), None);
    assert_eq!(m.root.get_attribute("role"), None);

    let m = Mounted::build(|s| {
        Anchor {
            href: "/settings".into(),
            ..Default::default()
        }
        .render(s, &[])
    });
    assert_eq!(m.root.get_attribute("data-rid"), None);
    assert_eq!(m.root.get_attribute("data-claim-links"), None);
}

/// #2206 review: with `onclick` and no `href` the placeholder `<a>` becomes a
/// keyboard stop: `tabindex="0"`, `role="button"`.
#[test]
fn an_anchor_with_onclick_and_no_href_is_a_button() {
    let (_, cb) = counter();
    let m = Mounted::build(|s| {
        Anchor {
            onclick: Some(cb),
            ..Default::default()
        }
        .render(s, &[])
    });
    assert_eq!(m.root.get_attribute("tabindex").as_deref(), Some("0"));
    assert_eq!(m.root.get_attribute("role").as_deref(), Some("button"));
    // No handler, no button: a plain placeholder.
    let m = Mounted::build(|s| Anchor::default().render(s, &[]));
    assert_eq!(m.root.get_attribute("tabindex"), None);
}

// ------------------------------------------------------------------- Image

/// #2203: `fallback_src` is no longer a JavaScript `onerror` string; a
/// failure swaps the picture once, and `on_error` runs for each failure.
#[test]
fn an_image_falls_back_to_its_fallback_src_once() {
    let (errors, cb) = counter();
    let m = Mounted::build(|s| {
        Image {
            src: "missing.png".into(),
            fallback_src: "spare.png".into(),
            on_error: Some(cb),
            ..Default::default()
        }
        .render(s, &[])
    });
    let img = find_img(&m.root).unwrap();
    assert_eq!(img.get_attribute("onerror"), None, "no JavaScript string");
    assert_eq!(img.get_attribute("src").as_deref(), Some("missing.png"));
    dispatch_event(onerror(&img));
    assert_eq!(img.get_attribute("src").as_deref(), Some("spare.png"));
    assert_eq!(errors.get(), 1);
    // The fallback fails too: no retry of itself, the error still reported.
    dispatch_event(onerror(&img));
    assert_eq!(img.get_attribute("src").as_deref(), Some("spare.png"));
    assert_eq!(errors.get(), 2);
}

/// #2203: `fallback` content replaces the `<img>` once no picture can be
/// shown — after `fallback_src` failed too.
#[test]
fn an_image_shows_its_fallback_content_when_no_picture_loads() {
    let fallback: rinch_components::SectionRenderer = Rc::new(|s: &mut RenderScope| {
        let el = s.create_element("span");
        el.set_attribute("class", "my-fallback");
        el
    });
    let m = Mounted::build(|s| {
        Image {
            src: "missing.png".into(),
            fallback_src: "spare.png".into(),
            fallback: Some(fallback),
            ..Default::default()
        }
        .render(s, &[])
    });
    let img = find_img(&m.root).unwrap();
    dispatch_event(onerror(&img));
    assert!(
        find_by_class(&m.root, "my-fallback").is_none(),
        "spare first"
    );
    dispatch_event(onerror(&img));
    assert!(find_by_class(&m.root, "my-fallback").is_some());
    assert!(find_img(&m.root).is_none(), "the <img> gave way");
}

/// An empty `src` takes the fallback straight away, as in Mantine.
#[test]
fn an_image_without_src_shows_its_fallback() {
    let m = Mounted::build(|s| {
        Image {
            fallback_src: "spare.png".into(),
            ..Default::default()
        }
        .render(s, &[])
    });
    let img = find_img(&m.root).unwrap();
    assert_eq!(img.get_attribute("src").as_deref(), Some("spare.png"));
}

/// Default rendering: no error handler on an `Image` that asked for none.
#[test]
fn a_plain_image_carries_no_error_handler() {
    let m = Mounted::build(|s| {
        Image {
            src: "photo.png".into(),
            ..Default::default()
        }
        .render(s, &[])
    });
    let img = find_img(&m.root).unwrap();
    assert_eq!(img.get_attribute("data-onerror"), None);
    assert_eq!(img.get_attribute("onerror"), None);
}

// ------------------------------------------------------------------ Avatar

/// #2202: a failed image gives way to the placeholder (initials here), and
/// `on_error` runs.
#[test]
fn an_avatar_falls_back_to_its_initials() {
    let (errors, cb) = counter();
    let m = Mounted::build(|s| {
        Avatar {
            src: "missing.png".into(),
            name: "Grace Hopper".into(),
            on_error: Some(cb),
            ..Default::default()
        }
        .render(s, &[])
    });
    let img = find_img(&m.root).unwrap();
    assert!(find_by_class(&m.root, "rinch-avatar__placeholder").is_none());
    dispatch_event(onerror(&img));
    assert_eq!(errors.get(), 1);
    assert!(find_img(&m.root).is_none());
    let placeholder = find_by_class(&m.root, "rinch-avatar__placeholder").unwrap();
    assert_eq!(placeholder.text_content().as_deref(), Some("GH"));
}

/// `keep_failed_image` keeps the image (its `alt` shows); `on_error` still
/// runs.
#[test]
fn an_avatar_can_keep_its_failed_image() {
    let (errors, cb) = counter();
    let m = Mounted::build(|s| {
        Avatar {
            src: "missing.png".into(),
            name: "Grace Hopper".into(),
            on_error: Some(cb),
            keep_failed_image: true,
            ..Default::default()
        }
        .render(s, &[])
    });
    let img = find_img(&m.root).unwrap();
    dispatch_event(onerror(&img));
    assert_eq!(errors.get(), 1);
    assert!(find_img(&m.root).is_some());
    assert!(find_by_class(&m.root, "rinch-avatar__placeholder").is_none());
}

/// #2202: `initials` overrides the ones `name` gives.
#[test]
fn an_avatar_shows_its_initials_override() {
    let m = Mounted::build(|s| {
        Avatar {
            name: "Grace Hopper".into(),
            initials: "G".into(),
            ..Default::default()
        }
        .render(s, &[])
    });
    let placeholder = find_by_class(&m.root, "rinch-avatar__placeholder").unwrap();
    assert_eq!(placeholder.text_content().as_deref(), Some("G"));
}

/// #2202: children are the placeholder, ahead of initials.
#[test]
fn an_avatar_shows_its_children() {
    let m = Mounted::build(|s| {
        let child = s.create_element("b");
        child.set_attribute("class", "my-mark");
        Avatar {
            name: "Grace Hopper".into(),
            ..Default::default()
        }
        .render(s, &[child])
    });
    let placeholder = find_by_class(&m.root, "rinch-avatar__placeholder").unwrap();
    assert!(find_by_class(&placeholder, "my-mark").is_some());
    assert_ne!(placeholder.text_content().as_deref(), Some("GH"));
}

/// #2202: the placeholder icon is drawn (it was an empty span), a person by
/// default, `placeholder_icon` when given.
#[test]
fn an_avatar_draws_its_placeholder_icon() {
    let icon_of =
        |m: &Mounted| glyph(&find_by_class(&m.root, "rinch-avatar__placeholder-icon").unwrap());
    let default = Mounted::build(|s| Avatar::default().render(s, &[]));
    let star = Mounted::build(|s| {
        Avatar {
            placeholder_icon: Some(TablerIcon::Star),
            ..Default::default()
        }
        .render(s, &[])
    });
    let user = Mounted::build(|s| {
        rinch_tabler_icons::render_tabler_icon(
            s,
            TablerIcon::User,
            rinch_tabler_icons::TablerIconStyle::Outline,
        )
    });
    let user = glyph(&user.root);
    assert!(!icon_of(&default).is_empty(), "a glyph is drawn");
    assert_eq!(icon_of(&default), user);
    assert_ne!(icon_of(&star), user);
    assert!(!icon_of(&star).is_empty());
}
