//! #2200 (Alert), #2201 (Badge), #2204 (Progress): colour beyond the built-in
//! names, Alert's `role` and title slot, Badge's sections, and Progress's live
//! `aria-valuenow`, `max`, indeterminate state and accessible name.
//!
//! Each prop is checked on a rendered tree, and each default is checked to
//! render what it rendered before (the defaults that change are named on
//! their test: Alert's `role`, a value-less Progress).

// `rsx!` expands to `rinch::…` paths.
use rinch_components as rinch;

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::SectionRenderer;
use rinch_components::alert::Alert;
use rinch_components::badge::Badge;
use rinch_components::progress::Progress;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::{Component, Signal};
use rinch_macros::rsx;

mod common;
use common::{find_by_class, has_class, sheet, strip_comments};

/// A rendered component, with its document and scope kept alive around it.
struct Mounted {
    _doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    root: NodeHandle,
}

fn build(f: impl FnOnce(&mut RenderScope) -> NodeHandle) -> Mounted {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let root = f(&mut scope);
    Mounted {
        _doc: doc,
        _scope: scope,
        root,
    }
}

fn mount_with<C: Component>(component: C, child: Option<&str>) -> Mounted {
    build(|scope| {
        let children: Vec<NodeHandle> = child.map(|t| scope.create_text(t)).into_iter().collect();
        component.render(scope, &children)
    })
}

fn mount<C: Component>(component: C) -> Mounted {
    mount_with(component, None)
}

/// A renderer drawing `<i class="{class}">`.
fn marker(class: &'static str) -> SectionRenderer {
    Rc::new(move |scope: &mut RenderScope| {
        let i = scope.create_element("i");
        i.set_attribute("class", class);
        i
    })
}

fn style(node: &NodeHandle) -> String {
    node.get_attribute("style").unwrap_or_default()
}

/// The stylesheet with whitespace collapsed, for substring checks.
fn css() -> String {
    strip_comments(&sheet())
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

// ------------------------------------------------------------------- Alert

#[test]
fn an_alert_takes_any_palette_name_through_its_variables() {
    let m = mount(Alert {
        color: "violet".into(),
        ..Default::default()
    });
    assert!(has_class(&m.root, "rinch-alert--colored"));
    assert!(
        !has_class(&m.root, "rinch-alert--blue"),
        "violet used to fall back to blue"
    );
    let s = style(&m.root);
    for want in [
        "--rinch-alert-color: var(--rinch-color-violet-6);",
        "--rinch-alert-light: var(--rinch-color-violet-light);",
        "--rinch-alert-light-text: var(--rinch-color-violet-light-text);",
        "--rinch-alert-light-strong: var(--rinch-color-violet-light-color-strong);",
    ] {
        assert!(s.contains(want), "{want} in {s:?}");
    }
}

#[test]
fn an_alert_takes_a_raw_css_colour() {
    let m = mount(Alert {
        color: "#7048e8".into(),
        variant: "filled".into(),
        ..Default::default()
    });
    let s = style(&m.root);
    assert!(s.contains("--rinch-alert-color: #7048e8;"), "{s:?}");
    assert!(
        s.contains("--rinch-alert-light: color-mix(in srgb, #7048e8 10%, transparent);"),
        "{s:?}"
    );
    assert!(has_class(&m.root, "rinch-alert--filled"));
}

#[test]
fn an_alerts_built_in_colours_keep_their_class_and_no_inline_style() {
    for (color, class) in [
        ("", "rinch-alert--blue"),
        ("info", "rinch-alert--blue"),
        ("green", "rinch-alert--green"),
        ("warning", "rinch-alert--yellow"),
        ("red", "rinch-alert--red"),
        ("gray", "rinch-alert--gray"),
        ("primary", "rinch-alert--primary"),
    ] {
        let m = mount(Alert {
            color: color.into(),
            ..Default::default()
        });
        assert!(has_class(&m.root, class), "{color:?} → {class}");
        assert!(!has_class(&m.root, "rinch-alert--colored"));
        assert_eq!(m.root.get_attribute("style"), None, "{color:?}");
    }
}

#[test]
fn every_alert_variant_reads_the_colour_variables() {
    let css = css();
    for rule in [
        ".rinch-alert--light { border: 1px solid transparent; background-color: var(--rinch-alert-light); color: var(--rinch-alert-light-text); }",
        ".rinch-alert--light .rinch-alert__title { color: var(--rinch-alert-light-strong); }",
        "background-color: var(--rinch-alert-color, var(--rinch-primary-color)); color: var(--rinch-alert-filled-text, var(--rinch-primary-color-contrast, white));",
        "border: 1px solid var(--rinch-alert-color); color: var(--rinch-alert-outline-text, var(--rinch-alert-color));",
    ] {
        assert!(css.contains(rule), "missing: {rule}");
    }
}

/// A default that changes: Mantine's Alert is `role="alert"`, and rinch's
/// had no role at all.
#[test]
fn an_alert_is_role_alert_by_default_and_role_replaces_it() {
    let m = mount(Alert::default());
    assert_eq!(m.root.get_attribute("role").as_deref(), Some("alert"));

    let m = mount(Alert {
        role: "status".into(),
        ..Default::default()
    });
    assert_eq!(m.root.get_attribute("role").as_deref(), Some("status"));
}

#[test]
fn an_alerts_title_section_renders_nodes_in_the_title_box() {
    let m = mount(Alert {
        title: "ignored".into(),
        title_section: Some(marker("my-title")),
        ..Default::default()
    });
    let title = find_by_class(&m.root, "rinch-alert__title").expect("a title box");
    assert!(find_by_class(&title, "my-title").is_some());
    assert!(
        title
            .children()
            .iter()
            .all(|c| c.get_attribute("class").is_some()),
        "the section wins over the title text"
    );
    assert!(has_class(&m.root, "rinch-alert--with-title"));

    // The text title is unchanged.
    let m = mount(Alert {
        title: "Saved".into(),
        ..Default::default()
    });
    let title = find_by_class(&m.root, "rinch-alert__title").expect("a title box");
    assert_eq!(title.text_content().as_deref(), Some("Saved"));
}

#[test]
fn alert_props_through_rsx() {
    let m = build(|__scope| {
        rsx! {
            Alert { color: "teal", role: "status", title_section: Some(marker("t")), "Body" }
        }
    });
    assert_eq!(m.root.get_attribute("role").as_deref(), Some("status"));
    assert!(style(&m.root).contains("var(--rinch-color-teal-6)"));
    assert!(find_by_class(&m.root, "t").is_some());
}

// ------------------------------------------------------------------- Badge

#[test]
fn a_badge_takes_a_raw_css_colour_through_its_variables() {
    let m = mount(Badge {
        color: "#123456".into(),
        ..Default::default()
    });
    let s = style(&m.root);
    assert!(s.contains("--rinch-badge-color: #123456;"), "{s:?}");
    assert!(
        s.contains("--rinch-badge-light: color-mix(in srgb, #123456 10%, transparent);"),
        "{s:?}"
    );
    assert!(s.contains("--rinch-badge-light-color: #123456;"), "{s:?}");
    assert_eq!(
        m.root.get_attribute("data-color").as_deref(),
        Some("#123456")
    );
}

#[test]
fn a_badge_reads_primary_and_other_token_families() {
    let m = mount(Badge {
        color: "primary".into(),
        ..Default::default()
    });
    let s = style(&m.root);
    assert!(
        s.contains("--rinch-badge-color: var(--rinch-primary-color);"),
        "{s:?}"
    );
    assert!(
        s.contains("--rinch-badge-light: var(--rinch-primary-color-light);"),
        "{s:?}"
    );

    let m = mount(Badge {
        color: "brand".into(),
        ..Default::default()
    });
    assert!(style(&m.root).contains("--rinch-badge-color: var(--rinch-color-brand-6);"));
}

#[test]
fn a_badges_built_in_colours_render_as_before() {
    for color in ["red", "lime", "teal"] {
        let m = mount(Badge {
            color: color.into(),
            ..Default::default()
        });
        assert_eq!(m.root.get_attribute("data-color").as_deref(), Some(color));
        assert_eq!(m.root.get_attribute("style"), None, "{color}");
    }
    let m = mount(Badge::default());
    assert_eq!(m.root.get_attribute("style"), None);
    assert_eq!(m.root.get_attribute("data-color"), None);
}

#[test]
fn every_badge_variant_reads_the_colour_variables() {
    let css = css();
    for rule in [
        "background-color: var(--rinch-badge-color, var(--rinch-primary-color)); color: var(--rinch-badge-text, var(--rinch-primary-color-contrast, white));",
        "background-color: var(--rinch-badge-light, var(--rinch-primary-color-light)); color: var(--rinch-badge-light-color, var(--rinch-primary-color-light-color));",
        "color: var(--rinch-badge-color, var(--rinch-primary-color)); border: 1px solid var(--rinch-badge-color, var(--rinch-primary-color));",
        ".rinch-badge[data-color=\"yellow\"] { --rinch-badge-color: var(--rinch-color-yellow-6); --rinch-badge-light: var(--rinch-color-yellow-light); --rinch-badge-light-color: var(--rinch-color-yellow-light-color); --rinch-badge-text: var(--rinch-color-yellow-contrast, black); }",
    ] {
        assert!(css.contains(rule), "missing: {rule}");
    }
}

#[test]
fn a_badges_sections_sit_either_side_of_its_label() {
    let m = mount_with(
        Badge {
            left_section: Some(marker("dot")),
            right_section: Some(marker("remove")),
            ..Default::default()
        },
        Some("New"),
    );
    let kids = m.root.children();
    assert_eq!(kids.len(), 3, "left section, label, right section");
    assert!(has_class(&kids[0], "rinch-badge__section"));
    assert_eq!(
        kids[0].get_attribute("data-position").as_deref(),
        Some("left")
    );
    assert!(find_by_class(&kids[0], "dot").is_some());
    assert!(has_class(&kids[1], "rinch-badge__label"));
    assert_eq!(kids[1].text_content().as_deref(), Some("New"));
    assert_eq!(
        kids[2].get_attribute("data-position").as_deref(),
        Some("right")
    );
    assert!(find_by_class(&kids[2], "remove").is_some());

    // One section alone.
    let m = mount_with(
        Badge {
            right_section: Some(marker("remove")),
            ..Default::default()
        },
        Some("New"),
    );
    let kids = m.root.children();
    assert_eq!(kids.len(), 2);
    assert!(has_class(&kids[0], "rinch-badge__label"));

    // No section: the children sit directly in the badge, as before.
    let m = mount_with(Badge::default(), Some("New"));
    let kids = m.root.children();
    assert_eq!(kids.len(), 1);
    assert_eq!(kids[0].get_attribute("class"), None);
}

// ---------------------------------------------------------------- Progress

fn bar(root: &NodeHandle) -> NodeHandle {
    find_by_class(root, "rinch-progress__bar").expect("a bar")
}

#[test]
fn a_progress_value_renders_as_before() {
    let m = mount(Progress {
        value: Some(65.0),
        color: "teal".into(),
        ..Default::default()
    });
    assert_eq!(m.root.get_attribute("aria-valuenow").as_deref(), Some("65"));
    assert_eq!(m.root.get_attribute("aria-valuemin").as_deref(), Some("0"));
    assert_eq!(
        m.root.get_attribute("aria-valuemax").as_deref(),
        Some("100")
    );
    assert_eq!(
        style(&bar(&m.root)),
        "width: 65%;--rinch-progress-color: var(--rinch-color-teal-6);"
    );
    assert!(!has_class(&m.root, "rinch-progress--indeterminate"));
    assert_eq!(m.root.get_attribute("aria-label"), None);
}

/// The bug: `value_fn` moved the bar but left `aria-valuenow` at its first
/// value.
#[test]
fn a_reactive_value_keeps_aria_valuenow_current() {
    let v = Signal::new(20.0f32);
    let m = mount(Progress {
        value_fn: Some(Rc::new(move || v.get())),
        ..Default::default()
    });
    assert_eq!(m.root.get_attribute("aria-valuenow").as_deref(), Some("20"));
    v.set(70.0);
    assert_eq!(style(&bar(&m.root)), "width: 70%;");
    assert_eq!(m.root.get_attribute("aria-valuenow").as_deref(), Some("70"));
    v.set(140.0);
    assert_eq!(
        m.root.get_attribute("aria-valuenow").as_deref(),
        Some("100"),
        "clamped to max"
    );
}

#[test]
fn max_sets_the_range() {
    let m = mount(Progress {
        value: Some(0.25),
        max: Some(0.5),
        ..Default::default()
    });
    assert_eq!(
        m.root.get_attribute("aria-valuemax").as_deref(),
        Some("0.5")
    );
    assert_eq!(
        m.root.get_attribute("aria-valuenow").as_deref(),
        Some("0.25")
    );
    assert_eq!(style(&bar(&m.root)), "width: 50%;");

    // Not a positive number: the default 100, as `<progress>` ignores one.
    for max in [0.0, -3.0, f32::NAN] {
        let m = mount(Progress {
            value: Some(30.0),
            max: Some(max),
            ..Default::default()
        });
        assert_eq!(
            m.root.get_attribute("aria-valuemax").as_deref(),
            Some("100")
        );
        assert_eq!(style(&bar(&m.root)), "width: 30%;");
    }
}

/// A default that changes: like `<progress>` with no `value`, a Progress with
/// none is indeterminate (it was a 0% bar).
#[test]
fn no_value_is_indeterminate() {
    let m = mount(Progress {
        color: "red".into(),
        ..Default::default()
    });
    assert!(has_class(&m.root, "rinch-progress--indeterminate"));
    assert_eq!(m.root.get_attribute("aria-valuenow"), None);
    let s = style(&bar(&m.root));
    assert!(!s.contains("width"), "{s:?}");
    assert!(s.contains("--rinch-progress-color: var(--rinch-color-red-6);"));

    let m = mount(Progress {
        value: Some(40.0),
        indeterminate: true,
        ..Default::default()
    });
    assert!(has_class(&m.root, "rinch-progress--indeterminate"));
    assert_eq!(m.root.get_attribute("aria-valuenow"), None);
}

#[test]
fn indeterminate_fn_switches_without_re_rendering() {
    let busy = Signal::new(true);
    let v = Signal::new(10.0f32);
    let m = build(|__scope| {
        rsx! {
            Progress {
                value_fn: move || v.get(),
                indeterminate_fn: move || busy.get(),
                max: 50.0,
                aria_label: "Upload"
            }
        }
    });
    assert!(has_class(&m.root, "rinch-progress--indeterminate"));
    assert_eq!(m.root.get_attribute("aria-valuenow"), None);
    assert_eq!(
        m.root.get_attribute("aria-label").as_deref(),
        Some("Upload")
    );

    busy.set(false);
    assert!(!has_class(&m.root, "rinch-progress--indeterminate"));
    assert_eq!(m.root.get_attribute("aria-valuenow").as_deref(), Some("10"));
    assert_eq!(style(&bar(&m.root)), "width: 20%;");

    busy.set(true);
    assert!(has_class(&m.root, "rinch-progress--indeterminate"));
    assert_eq!(m.root.get_attribute("aria-valuenow"), None);
}

#[test]
fn the_indeterminate_bar_animates_and_the_width_transition_is_a_variable() {
    let css = css();
    assert!(
        css.contains(".rinch-progress--indeterminate .rinch-progress__bar { width: 40%; animation: rinch-progress-indeterminate var(--rinch-progress-indeterminate-duration, 1.5s) ease-in-out infinite; }"),
        "the indeterminate rule"
    );
    assert!(css.contains("@keyframes rinch-progress-indeterminate"));
    assert!(
        css.contains("transition: var(--rinch-progress-transition, width 200ms ease);"),
        "the width transition Progress always had, with `none` as the escape hatch"
    );
}

/// A colour the stylesheet has no rule for writes its contrast as the filled
/// text (#2208 on top of #2200's inline variables): computed from a raw
/// colour, the theme's token for a name, the primary token for `primary`.
#[test]
fn an_inline_colour_writes_its_contrast_as_the_filled_text() {
    for (color, want) in [
        ("#ffe066", "black"),
        ("#1864ab", "white"),
        ("violet", "var(--rinch-color-violet-contrast, white)"),
        ("primary", "var(--rinch-primary-color-contrast, white)"),
    ] {
        let alert = mount(Alert {
            color: color.into(),
            variant: "filled".into(),
            ..Default::default()
        });
        if color != "primary" {
            let s = style(&alert.root);
            assert!(
                s.contains(&format!("--rinch-alert-filled-text: {want};")),
                "alert {color}: {s:?}"
            );
        }
        let badge = mount(Badge {
            color: color.into(),
            ..Default::default()
        });
        if color != "violet" {
            let s = style(&badge.root);
            assert!(
                s.contains(&format!("--rinch-badge-text: {want};")),
                "badge {color}: {s:?}"
            );
        }
    }
    let css = css();
    assert!(css.contains("--rinch-alert-filled-text: var(--rinch-primary-color-contrast, white);"));
    assert!(css.contains("--rinch-alert-filled-text: var(--rinch-color-red-contrast, white);"));
    assert!(css.contains("--rinch-badge-text: var(--rinch-primary-color-contrast, white);"));
}
