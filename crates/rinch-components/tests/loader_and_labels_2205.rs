//! The props #2197, #2198, #2199 and #2205 added: the shared `Loader` behind
//! every loading state, the accessible names that were fixed English, the
//! `type` attributes, and ActionIcon's and CloseButton's glyph options.
//!
//! Each prop is checked on a rendered tree, and its default is checked to be
//! what the component rendered before (the visual default is unchanged).

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::action_icon::ActionIcon;
use rinch_components::alert::Alert;
use rinch_components::close_button::CloseButton;
use rinch_components::stepper::StepperStep;
use rinch_components::{Button, Loader, LoaderDefaults, SectionRenderer};
use rinch_core::Component;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle};

mod common;
use common::{find_by_class, glyph, has_class, sheet};

/// A rendered component, with its document and scope kept alive around it.
struct Mounted {
    _doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    root: NodeHandle,
}

fn mount_with<C: Component>(component: C, child: Option<&str>) -> Mounted {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let children: Vec<NodeHandle> = child.map(|t| scope.create_text(t)).into_iter().collect();
    let root = component.render(&mut scope, &children);
    Mounted {
        _doc: doc,
        _scope: scope,
        root,
    }
}

fn mount<C: Component>(component: C) -> Mounted {
    mount_with(component, None)
}

/// A custom loader renderer that draws `<i class="my-spinner">`.
fn my_spinner() -> SectionRenderer {
    Rc::new(|scope: &mut RenderScope| {
        let i = scope.create_element("i");
        i.set_attribute("class", "my-spinner");
        i
    })
}

// ------------------------------------------------------------------ Loader

#[test]
fn a_loaders_accessible_name_defaults_to_loading_and_is_a_prop() {
    let m = mount(Loader::default());
    assert_eq!(
        m.root.get_attribute("aria-label").as_deref(),
        Some("Loading")
    );
    assert_eq!(m.root.get_attribute("role").as_deref(), Some("status"));

    let m = mount(Loader {
        aria_label: "Chargement".into(),
        ..Default::default()
    });
    assert_eq!(
        m.root.get_attribute("aria-label").as_deref(),
        Some("Chargement")
    );
}

#[test]
fn a_custom_loader_replaces_the_built_in_kinds_inside_the_sized_box() {
    let m = mount(Loader {
        custom: Some(my_spinner()),
        r#type: "bars".into(),
        size: "lg".into(),
        ..Default::default()
    });
    assert!(has_class(&m.root, "rinch-loader"));
    assert!(has_class(&m.root, "rinch-loader--lg"), "the size box stays");
    assert!(has_class(&m.root, "rinch-loader--custom"));
    assert!(
        !has_class(&m.root, "rinch-loader--bars"),
        "custom wins over type"
    );
    assert!(find_by_class(&m.root, "my-spinner").is_some());
    assert!(find_by_class(&m.root, "rinch-loader__bar").is_none());
}

/// The app-wide default: a loader with no type of its own takes the
/// context's, its own type still wins, and a context custom loader reaches the
/// loaders the other components draw.
#[test]
fn loader_defaults_set_the_type_and_custom_loader_app_wide() {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);

    // No context: oval, as before.
    let plain = Loader::default().render(&mut scope, &[]);
    assert!(has_class(&plain, "rinch-loader--oval"));

    rinch_core::create_context(LoaderDefaults {
        r#type: "dots".into(),
        custom: None,
    });
    let defaulted = Loader::default().render(&mut scope, &[]);
    assert!(has_class(&defaulted, "rinch-loader--dots"));
    let mut dots = Vec::new();
    common::collect_by_class(&defaulted, "rinch-loader__dot", &mut dots);
    assert_eq!(dots.len(), 3);
    let own = Loader {
        r#type: "bars".into(),
        ..Default::default()
    }
    .render(&mut scope, &[]);
    assert!(
        has_class(&own, "rinch-loader--bars"),
        "a loader's own type wins"
    );

    // A Button's loading state goes through the same Loader.
    let button = Button {
        loading: true,
        ..Default::default()
    }
    .render(&mut scope, &[]);
    let in_button = find_by_class(&button, "rinch-loader").expect("a loader");
    assert!(has_class(&in_button, "rinch-loader--dots"));

    rinch_core::create_context(LoaderDefaults {
        r#type: String::new(),
        custom: Some(my_spinner()),
    });
    let step = StepperStep {
        loading: true,
        ..Default::default()
    }
    .render(&mut scope, &[]);
    let slot = find_by_class(&step, "rinch-stepper__loader").expect("the stepper's slot");
    assert!(
        find_by_class(&slot, "my-spinner").is_some(),
        "the app's custom loader reaches a loading stepper step"
    );
}

// ------------------------------------------------------------------ Button

#[test]
fn a_buttons_type_is_written_only_when_given() {
    for t in ["button", "submit", "reset"] {
        let m = mount(Button {
            r#type: t.into(),
            ..Default::default()
        });
        assert_eq!(m.root.get_attribute("type").as_deref(), Some(t));
    }
    assert_eq!(mount(Button::default()).root.get_attribute("type"), None);
}

#[test]
fn a_loading_buttons_loader_type_and_custom_loader() {
    let m = mount_with(
        Button {
            loading: true,
            loader_type: "bars".into(),
            ..Default::default()
        },
        Some("Save"),
    );
    let slot = find_by_class(&m.root, "rinch-button__loader").expect("a loader slot");
    let loader = find_by_class(&slot, "rinch-loader").expect("a Loader");
    assert!(has_class(&loader, "rinch-loader--bars"));
    assert!(
        has_class(&loader, "rinch-loader--xs"),
        "an md button's loader is the xs step (1rem), the size of the spinner \
         the stylesheet always meant to draw"
    );

    let m = mount(Button {
        loading: true,
        loader: Some(my_spinner()),
        ..Default::default()
    });
    let slot = find_by_class(&m.root, "rinch-button__loader").expect("a loader slot");
    assert!(find_by_class(&slot, "my-spinner").is_some());

    // The loader inherits the button's text colour rather than drawing the
    // primary colour on a filled button's primary background.
    assert!(sheet().contains(".rinch-button__loader .rinch-loader {\n    color: inherit;"));
}

// -------------------------------------------------------------- ActionIcon

#[test]
fn an_action_icons_aria_label_is_written_only_when_given() {
    assert_eq!(
        mount(ActionIcon::default())
            .root
            .get_attribute("aria-label"),
        None
    );
    let m = mount(ActionIcon {
        aria_label: "Delete row".into(),
        title: "Delete".into(),
        ..Default::default()
    });
    assert_eq!(
        m.root.get_attribute("aria-label").as_deref(),
        Some("Delete row")
    );
    assert_eq!(m.root.get_attribute("title").as_deref(), Some("Delete"));
}

#[test]
fn an_action_icons_type_defaults_to_button_and_is_a_prop() {
    assert_eq!(
        mount(ActionIcon::default())
            .root
            .get_attribute("type")
            .as_deref(),
        Some("button")
    );
    let m = mount(ActionIcon {
        r#type: "submit".into(),
        ..Default::default()
    });
    assert_eq!(m.root.get_attribute("type").as_deref(), Some("submit"));
}

#[test]
fn an_action_icons_icon_size_and_style_override_the_table() {
    let svg_of = |m: &Mounted| m.root.children().into_iter().next().expect("an icon");
    // Default: the size table (lg = 22) and the outline style.
    let default = mount(ActionIcon {
        icon: Some(TablerIcon::Heart),
        size: "lg".into(),
        ..Default::default()
    });
    assert_eq!(
        svg_of(&default).get_attribute("width").as_deref(),
        Some("22")
    );

    let sized = mount(ActionIcon {
        icon: Some(TablerIcon::Heart),
        size: "lg".into(),
        icon_size: Some(31),
        ..Default::default()
    });
    let svg = svg_of(&sized);
    assert_eq!(svg.get_attribute("width").as_deref(), Some("31"));
    assert!(
        svg.get_attribute("style")
            .unwrap_or_default()
            .contains("width: 31px"),
        "the inline size follows icon_size too"
    );

    let filled = mount(ActionIcon {
        icon: Some(TablerIcon::Heart),
        icon_style: TablerIconStyle::Filled,
        ..Default::default()
    });
    assert_ne!(
        glyph(&svg_of(&filled)),
        glyph(&svg_of(&default)),
        "the filled heart is a different glyph from the outline one"
    );
}

#[test]
fn a_loading_action_icon_draws_the_shared_loader() {
    let m = mount(ActionIcon {
        icon: Some(TablerIcon::Heart),
        loading: true,
        loader_type: "dots".into(),
        ..Default::default()
    });
    let slot = find_by_class(&m.root, "rinch-action-icon__loader").expect("the loader slot");
    let loader = find_by_class(&slot, "rinch-loader").expect("a Loader");
    assert!(has_class(&loader, "rinch-loader--dots"));
    assert!(glyph(&m.root).is_empty(), "no icon while loading");
}

// ------------------------------------------------------------- CloseButton

#[test]
fn a_close_buttons_label_defaults_to_close_and_is_a_prop() {
    assert_eq!(
        mount(CloseButton::default())
            .root
            .get_attribute("aria-label")
            .as_deref(),
        Some("Close")
    );
    let m = mount(CloseButton {
        aria_label: "Fermer".into(),
        ..Default::default()
    });
    assert_eq!(
        m.root.get_attribute("aria-label").as_deref(),
        Some("Fermer")
    );
}

#[test]
fn a_close_buttons_type_defaults_to_button_and_is_a_prop() {
    assert_eq!(
        mount(CloseButton::default())
            .root
            .get_attribute("type")
            .as_deref(),
        Some("button")
    );
    let m = mount(CloseButton {
        r#type: "reset".into(),
        ..Default::default()
    });
    assert_eq!(m.root.get_attribute("type").as_deref(), Some("reset"));
}

#[test]
fn a_close_buttons_icon_replaces_the_x_and_children_replace_both() {
    let x = mount(CloseButton::default());
    let x_glyph = glyph(&x.root);
    assert_eq!(x_glyph, vec!["M18 6L6 18M6 6l12 12".to_string()]);

    let icon = mount(CloseButton {
        icon: Some(TablerIcon::Trash),
        size: "lg".into(),
        ..Default::default()
    });
    let svg = find_by_class(&icon.root, "rinch-icon--close").expect("the glyph keeps its hook");
    assert_ne!(glyph(&icon.root), x_glyph, "a different glyph");
    assert!(!glyph(&icon.root).is_empty());
    assert_eq!(svg.get_attribute("width").as_deref(), Some("20"));

    let kids = mount_with(
        CloseButton {
            icon: Some(TablerIcon::Trash),
            ..Default::default()
        },
        Some("Done"),
    );
    assert!(glyph(&kids.root).is_empty(), "children win over the glyph");
    assert!(find_by_class(&kids.root, "rinch-icon--close").is_none());
    assert_eq!(kids.root.children().len(), 1, "just the caller's child");
}

/// The glyph's size is a custom property the stylesheet spends on one class,
/// not an inline width/height — so an app rule can resize it.
#[test]
fn a_close_glyphs_size_is_not_an_inline_width() {
    let m = mount(CloseButton {
        size: "xl".into(),
        ..Default::default()
    });
    let svg = find_by_class(&m.root, "rinch-icon--close").expect("a glyph");
    let style = svg.get_attribute("style").unwrap_or_default();
    assert_eq!(style, "--rinch-close-button-icon-size: 24px;");
    assert!(sheet().contains(
        ".rinch-icon--close {\n    width: var(--rinch-close-button-icon-size, 16px);\n    \
         height: var(--rinch-close-button-icon-size, 16px);"
    ));
}

// ------------------------------------------------------------------- Alert

#[test]
fn an_alerts_close_label_defaults_to_close_and_is_a_prop() {
    let close_of = |alert: Alert| {
        let m = mount(alert);
        find_by_class(&m.root, "rinch-alert__close")
            .expect("a close button")
            .get_attribute("aria-label")
    };
    assert_eq!(
        close_of(Alert {
            with_close_button: true,
            ..Default::default()
        })
        .as_deref(),
        Some("Close")
    );
    assert_eq!(
        close_of(Alert {
            with_close_button: true,
            close_button_label: "Schließen".into(),
            ..Default::default()
        })
        .as_deref(),
        Some("Schließen")
    );
}

// ----------------------------------------------------------------- Stepper

#[test]
fn a_loading_stepper_step_draws_the_shared_loader() {
    let m = mount(StepperStep {
        loading: true,
        loader_type: "bars".into(),
        ..Default::default()
    });
    let slot = find_by_class(&m.root, "rinch-stepper__loader").expect("the slot");
    let loader = find_by_class(&slot, "rinch-loader").expect("a Loader");
    assert!(has_class(&loader, "rinch-loader--bars"));
}
