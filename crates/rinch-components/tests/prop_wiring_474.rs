//! The props #474 wired, at the component level: five `radius`, five
//! `z_index`, `CloseButton::icon_size`, the two `overlay_opacity`, and — from
//! the dismissal PR — `Popover::close_on_click_outside`.
//!
//! Each was declared, documented in `docs/src/guide/component-props.md`, and
//! read by nothing. `tests/no_dead_props.rs` is what keeps that from recurring;
//! these are what say the wiring is *right*, which a source scan cannot.
//!
//! Every assertion here is made against a rendered tree rather than a class
//! helper, so it fails if the class or style stops reaching the element, and
//! **each prop is checked against the stylesheet as well**. A class nothing
//! styles and a custom property no rule reads are the same bug as the one being
//! fixed, one layer down — `Badge::radius` emitting `rinch-badge--radius-md`
//! into a sheet with no such selector would satisfy the scan and change nothing
//! on screen.
//!
//! The end-to-end half of `z_index` — that a published custom property really
//! moves the computed `z-index`, through Stylo and the `calc()` offsets — needs
//! a CSS engine, so it lives in `rinch`'s `app::overlay_z_index_tests`.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::badge::Badge;
use rinch_components::class_utils::RADIUS_SCALE;
use rinch_components::close_button::CloseButton;
use rinch_components::drawer::Drawer;
use rinch_components::dropdown_menu::DropdownMenu;
use rinch_components::modal::Modal;
use rinch_components::notification::Notification;
use rinch_components::number_input::NumberInput;
use rinch_components::password_input::PasswordInput;
use rinch_components::popover::Popover;
use rinch_components::tabs::Tabs;
use rinch_components::text_input::TextInput;
use rinch_core::Component;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};

/// A rendered component, with its document and scope kept alive around it.
struct Mounted {
    _doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    root: NodeHandle,
}

impl Mounted {
    fn new<C: Component>(component: C) -> Self {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let root = component.render(&mut scope, &[]);
        Self {
            _doc: doc,
            _scope: scope,
            root,
        }
    }

    fn root_classes(&self) -> Vec<String> {
        self.root
            .get_attribute("class")
            .expect("the root is classed")
            .split_whitespace()
            .map(str::to_string)
            .collect()
    }

    fn has_class(&self, class: &str) -> bool {
        self.root_classes().iter().any(|c| c == class)
    }

    /// Classes naming a radius step, whichever step.
    fn radius_classes(&self) -> Vec<String> {
        self.root_classes()
            .into_iter()
            .filter(|c| c.contains("--radius-"))
            .collect()
    }

    /// The root's inline `style`, or the empty string when it carries none.
    fn root_style(&self) -> String {
        self.root.get_attribute("style").unwrap_or_default()
    }

    fn find(&self, class: &str) -> Option<NodeHandle> {
        find_by_class(&self.root, class)
    }
}

fn find_by_class(node: &NodeHandle, class: &str) -> Option<NodeHandle> {
    let matches = node
        .get_attribute("class")
        .is_some_and(|attr| attr.split_whitespace().any(|c| c == class));
    if matches {
        return Some(node.clone());
    }
    node.children().iter().find_map(|c| find_by_class(c, class))
}

/// Every component's CSS, as the runtime loads it.
fn sheet() -> String {
    rinch_components::styles::generate_all_component_styles()
}

// ------------------------------------------------------------------ radius

/// What each of the five newly wired components should emit, and the selector
/// the sheet must carry for it. The selector is not always the class itself:
/// `TextInput` and `PasswordInput` put the class on a wrapper and style the box
/// inside it, and `NumberInput` rebinds the radius variable its five boxes share.
struct RadiusCase {
    component: &'static str,
    /// `{step}` is filled with each scale step.
    class: &'static str,
    /// The full declaration the sheet must carry for that step.
    declaration: &'static str,
}

const RADIUS_CASES: &[RadiusCase] = &[
    RadiusCase {
        component: "Badge",
        class: "rinch-badge--radius-{step}",
        declaration: ".rinch-badge--radius-{step} { border-radius: var(--rinch-radius-{step}); }",
    },
    RadiusCase {
        component: "TextInput",
        class: "rinch-text-input--radius-{step}",
        declaration: ".rinch-text-input--radius-{step} .rinch-text-input__input { border-radius: var(--rinch-radius-{step}); }",
    },
    RadiusCase {
        component: "PasswordInput",
        class: "rinch-password-input--radius-{step}",
        declaration: ".rinch-password-input--radius-{step} .rinch-password-input__wrapper { border-radius: var(--rinch-radius-{step}); }",
    },
    RadiusCase {
        component: "NumberInput",
        class: "rinch-number-input--radius-{step}",
        declaration: ".rinch-number-input--radius-{step} { --rinch-radius-default: var(--rinch-radius-{step}); }",
    },
    RadiusCase {
        component: "Tabs",
        class: "rinch-tabs--radius-{step}",
        declaration: ".rinch-tabs--radius-{step} .rinch-tabs__tab { border-radius: var(--rinch-radius-{step}); }",
    },
];

/// Render one of the five with `radius` set to `radius`.
fn mount_with_radius(component: &str, radius: &str) -> Mounted {
    let r = radius.to_string();
    match component {
        "Badge" => Mounted::new(Badge {
            radius: r,
            ..Default::default()
        }),
        "TextInput" => Mounted::new(TextInput {
            radius: r,
            ..Default::default()
        }),
        "PasswordInput" => Mounted::new(PasswordInput {
            radius: r,
            ..Default::default()
        }),
        "NumberInput" => Mounted::new(NumberInput {
            radius: r,
            ..Default::default()
        }),
        "Tabs" => Mounted::new(Tabs {
            radius: r,
            ..Default::default()
        }),
        other => panic!("no radius fixture for {other}"),
    }
}

#[test]
fn every_radius_step_reaches_the_rendered_root() {
    for case in RADIUS_CASES {
        for step in RADIUS_SCALE {
            let m = mount_with_radius(case.component, step);
            let expected = case.class.replace("{step}", step);
            assert!(
                m.has_class(&expected),
                "{}: radius {step:?} must render {expected}; got {:?}",
                case.component,
                m.root_classes()
            );
            assert_eq!(
                m.radius_classes(),
                vec![expected],
                "{}: radius {step:?} must emit exactly one radius class — two \
                 would both apply and the winner would be stylesheet order",
                case.component
            );
        }
    }
}

#[test]
fn an_unset_or_unknown_radius_emits_no_class() {
    for case in RADIUS_CASES {
        // "" is this crate's spelling of "unset"; the rest are values the scale
        // does not name, including the case-shifted step a `size` prop would
        // have accepted.
        for radius in ["", "none", "MD", "md ", "2rem", "xxl"] {
            let m = mount_with_radius(case.component, radius);
            assert!(
                m.radius_classes().is_empty(),
                "{}: radius {radius:?} must leave the stylesheet's own \
                 border-radius alone; got {:?}",
                case.component,
                m.radius_classes()
            );
        }
    }
}

/// A class with no rule behind it is the same bug one layer down.
#[test]
fn the_stylesheet_carries_a_rule_for_every_radius_class() {
    let css = sheet();
    for case in RADIUS_CASES {
        for step in RADIUS_SCALE {
            let declaration = case.declaration.replace("{step}", step);
            assert!(
                css.contains(&declaration),
                "{}: the sheet must carry `{declaration}`",
                case.component
            );
        }
    }
}

/// `outline` tabs sit on the panel's top edge, so their bottom corners stay
/// square — the one radius case with a shape rather than a single length.
#[test]
fn outline_tabs_keep_square_bottom_corners_at_every_step() {
    let css = sheet();
    for step in RADIUS_SCALE {
        let declaration = format!(
            ".rinch-tabs--outline.rinch-tabs--radius-{step} .rinch-tabs__tab \
             {{ border-radius: var(--rinch-radius-{step}) var(--rinch-radius-{step}) 0 0; }}"
        );
        assert!(
            css.contains(&declaration),
            "the sheet must carry `{declaration}`"
        );
    }
}

// -------------------------------------------------------- CloseButton icon

/// The glyph carries the size step's own px, and `icon_size` overrides it.
#[test]
fn the_close_glyph_is_sized_by_icon_size_then_by_size() {
    // (size, icon_size, expected px) — the `size` defaults come from
    // `CloseButtonSize::icon_size`, which until #474 nothing called.
    for (size, icon_size, expected) in [
        ("", None, "16"),
        ("xs", None, "12"),
        ("sm", None, "14"),
        ("md", None, "16"),
        ("lg", None, "20"),
        ("xl", None, "24"),
        // `icon_size` wins over every step, including the one that agrees with
        // it by accident at `lg`.
        ("", Some(9), "9"),
        ("xs", Some(9), "9"),
        ("xl", Some(9), "9"),
        ("lg", Some(20), "20"),
    ] {
        let m = Mounted::new(CloseButton {
            size: size.to_string(),
            icon_size,
            ..Default::default()
        });
        let glyph = m
            .find("rinch-icon--close")
            .expect("the button renders a close glyph");
        assert_eq!(
            glyph.get_attribute("width").as_deref(),
            Some(expected),
            "size {size:?} + icon_size {icon_size:?} must size the glyph {expected}px"
        );
        assert_eq!(
            glyph.get_attribute("height").as_deref(),
            Some(expected),
            "the glyph must be square"
        );
        let style = glyph.get_attribute("style").unwrap_or_default();
        assert!(
            style.contains(&format!("width: {expected}px"))
                && style.contains(&format!("height: {expected}px")),
            "the size must also be an inline style — an svg sized by attributes \
             alone is what `render_tabler_icon_with_options` does not rely on; \
             got {style:?}"
        );
    }
}

/// The glyph has geometry, which the `<span>` it replaced did not: nothing in
/// the workspace styles `.rinch-icon--close`, so that span drew no X.
#[test]
fn the_close_glyph_has_paths_rather_than_an_empty_span() {
    let m = Mounted::new(CloseButton::default());
    let glyph = m.find("rinch-icon--close").expect("a close glyph");
    assert!(
        !glyph.children().is_empty(),
        "the glyph must carry its own path; an element with no children and no \
         CSS is the invisible span #474 replaced"
    );
    assert!(
        !sheet().contains(".rinch-icon--close"),
        "if a rule for .rinch-icon--close ever lands, the reason this glyph \
         carries its own size can be revisited"
    );
}

// ------------------------------------------------------- overlay_opacity

/// `Modal` and `Drawer` published their overlay opacity as a custom property
/// that neither stylesheet spent — declared, documented, inert, exactly like
/// the five `z_index` props. Note the scan in `no_dead_props` cannot catch this
/// shape: reading a prop into a variable nobody consumes *is* a read.
#[test]
fn overlay_opacity_is_published_and_spent() {
    for (component, property, selector) in [
        (
            "Modal",
            "--rinch-modal-overlay-opacity",
            "rgba(0, 0, 0, var(--rinch-modal-overlay-opacity, 0.75))",
        ),
        (
            "Drawer",
            "--rinch-drawer-overlay-opacity",
            "rgba(0, 0, 0, var(--rinch-drawer-overlay-opacity, 0.75))",
        ),
    ] {
        let m = match component {
            "Modal" => Mounted::new(Modal {
                opened: true,
                overlay_opacity: Some(0.25),
                ..Default::default()
            }),
            _ => Mounted::new(Drawer {
                opened: true,
                overlay_opacity: Some(0.25),
                ..Default::default()
            }),
        };
        let overlay = m
            .find(&format!("rinch-{}__overlay", component.to_lowercase()))
            .expect("the component renders an overlay");
        let style = overlay.get_attribute("style").unwrap_or_default();
        assert!(
            style.contains(&format!("{property}: 0.25")),
            "{component}: overlay_opacity must publish `{property}`; got {style:?}"
        );
        assert!(
            sheet().contains(selector),
            "{component}: the sheet must spend it on the backdrop's alpha —              `{selector}`"
        );
    }
}

#[test]
fn an_unset_overlay_opacity_publishes_nothing() {
    for component in ["Modal", "Drawer"] {
        let m = match component {
            "Modal" => Mounted::new(Modal {
                opened: true,
                ..Default::default()
            }),
            _ => Mounted::new(Drawer {
                opened: true,
                ..Default::default()
            }),
        };
        let overlay = m
            .find(&format!("rinch-{}__overlay", component.to_lowercase()))
            .expect("the component renders an overlay");
        let style = overlay.get_attribute("style").unwrap_or_default();
        assert!(
            !style.contains("overlay-opacity"),
            "{component}: an unset overlay_opacity must leave the sheet's own              level in place; got style {style:?}"
        );
    }
}

// ----------------------------------------------------------------- z_index

/// Each overlay publishes one custom property, and the sheet spends it on the
/// layers that used to hard-code a number. `base` is the level the prop names;
/// `derived` is the layer that keeps its offset from it.
struct ZCase {
    component: &'static str,
    property: &'static str,
    /// The declaration the base layer's rule must carry.
    base: &'static str,
    /// The declaration of the layer offset from it, if the component has one.
    derived: Option<&'static str>,
}

const Z_CASES: &[ZCase] = &[
    ZCase {
        component: "Modal",
        property: "--rinch-modal-z-index",
        base: "z-index: var(--rinch-modal-z-index, 200);",
        derived: Some("z-index: calc(var(--rinch-modal-z-index, 200) + 1);"),
    },
    ZCase {
        component: "Drawer",
        property: "--rinch-drawer-z-index",
        base: "z-index: var(--rinch-drawer-z-index, 200);",
        derived: Some("z-index: calc(var(--rinch-drawer-z-index, 200) + 1);"),
    },
    ZCase {
        component: "Popover",
        property: "--rinch-popover-z-index",
        base: "z-index: var(--rinch-popover-z-index, 100);",
        // The outside-click backdrop (#474), one level under the panel.
        derived: Some("z-index: calc(var(--rinch-popover-z-index, 100) - 1);"),
    },
    ZCase {
        component: "DropdownMenu",
        property: "--rinch-dropdown-menu-z-index",
        base: "z-index: var(--rinch-dropdown-menu-z-index, 100);",
        derived: Some("z-index: calc(var(--rinch-dropdown-menu-z-index, 100) - 1);"),
    },
    ZCase {
        component: "Notification",
        property: "--rinch-notification-z-index",
        base: "z-index: var(--rinch-notification-z-index, 300);",
        derived: None,
    },
];

/// Render one of the five overlays with `z_index` set (or not).
fn mount_with_z_index(component: &str, z_index: Option<i32>) -> Mounted {
    match component {
        "Modal" => Mounted::new(Modal {
            z_index,
            ..Default::default()
        }),
        "Drawer" => Mounted::new(Drawer {
            z_index,
            ..Default::default()
        }),
        "Popover" => Mounted::new(Popover {
            z_index,
            ..Default::default()
        }),
        "DropdownMenu" => Mounted::new(DropdownMenu {
            z_index,
            ..Default::default()
        }),
        "Notification" => Mounted::new(Notification {
            z_index,
            ..Default::default()
        }),
        other => panic!("no z_index fixture for {other}"),
    }
}

#[test]
fn z_index_publishes_its_level_on_the_rendered_root() {
    for case in Z_CASES {
        // 517 rather than a round number: it is not any component's default, not
        // adjacent to one, and a `+ 1`/`- 1` confusion in the sheet shows up as
        // 518/516 rather than colliding with a neighbouring fixture's value.
        let m = mount_with_z_index(case.component, Some(517));
        let style = m.root_style();
        assert!(
            style.contains(&format!("{}: 517", case.property)),
            "{}: z_index must publish `{}`; got style {style:?}",
            case.component,
            case.property
        );
    }
}

#[test]
fn no_z_index_publishes_nothing_inline() {
    for case in Z_CASES {
        let m = mount_with_z_index(case.component, None);
        let style = m.root_style();
        assert!(
            !style.contains("z-index"),
            "{}: an unset z_index must leave the stylesheet's own level in \
             place, inline-free; got style {style:?}",
            case.component
        );
    }
}

/// The Rust half publishes a number; the *offsets* live in the sheet. Without
/// this, a property no rule reads would pass every assertion above.
#[test]
fn the_stylesheet_spends_every_published_z_index() {
    let css = sheet();
    for case in Z_CASES {
        assert!(
            css.contains(case.base),
            "{}: the sheet must read the property for its base layer — \
             `{}`",
            case.component,
            case.base
        );
        if let Some(derived) = case.derived {
            assert!(
                css.contains(derived),
                "{}: the layer offset from the base must derive from the same \
                 property, or one prop would split the pair — `{derived}`",
                case.component
            );
        }
    }
}

/// No layer of a wired overlay may be left with a bare number: a second
/// hard-coded `z-index` inside one of these components is a layer the prop
/// cannot move, which is how a backdrop ends up above the panel it guards.
#[test]
fn the_wired_overlays_hard_code_no_z_index() {
    // Every selector these five own. Each is a prefix of its own modifiers and
    // elements (`.rinch-dropdown-menu__backdrop`) and of nothing else's.
    const OWNED: &[&str] = &[
        ".rinch-modal",
        ".rinch-drawer",
        ".rinch-popover",
        ".rinch-dropdown-menu",
        ".rinch-notification",
    ];

    let css = sheet();
    let mut selector = String::new();
    let mut checked = 0usize;

    for line in css.lines() {
        let line = line.trim();
        if line.ends_with('{') {
            selector = line.trim_end_matches('{').trim().to_string();
            continue;
        }
        let Some(value) = line.strip_prefix("z-index:") else {
            continue;
        };
        if !OWNED.iter().any(|owned| selector.contains(owned)) {
            continue;
        }
        checked += 1;
        assert!(
            value.contains("var(--rinch-"),
            "`{selector}` declares `{line}` — a level the component's `z_index` \
             prop cannot move"
        );
    }

    // Positive control: a selector tracker that matched nothing would pass the
    // loop above without looking at a single declaration.
    assert_eq!(
        checked, 9,
        "expected the nine z-index declarations these five own (2 Modal, \
         2 Drawer, 2 Popover, 2 DropdownMenu, 1 Notification), saw {checked}"
    );
}

// ------------------------------------------------- Popover outside clicks

/// `Popover::close_on_click_outside` renders a dismiss backdrop, and that
/// backdrop carries a live `data-rid` — the attribute both backends dispatch,
/// which is what makes this one component change work on desktop and on the
/// web with no backend edit.
///
/// The off case is beside it because "always renders a backdrop" satisfies the
/// on case and silently makes a `Popover` swallow the page's clicks.
#[test]
fn close_on_click_outside_renders_a_backdrop_that_carries_a_handler() {
    let m = Mounted::new(Popover {
        close_on_click_outside: true,
        onclose: Some(rinch_core::Callback::new(|| {})),
        ..Default::default()
    });
    let backdrop = m
        .find("rinch-popover__backdrop")
        .expect("close_on_click_outside must render a backdrop");
    assert!(
        backdrop
            .get_attribute("data-rid")
            .is_some_and(|rid| !rid.is_empty()),
        "the backdrop must carry a data-rid, or clicking it does nothing"
    );

    let off = Mounted::new(Popover {
        close_on_click_outside: false,
        onclose: Some(rinch_core::Callback::new(|| {})),
        ..Default::default()
    });
    assert!(
        off.find("rinch-popover__backdrop").is_none(),
        "with the prop off there must be no backdrop to swallow clicks"
    );

    // And nothing to send the click to means nothing to click: a `Popover`
    // with no `onclose` cannot close itself, so a backdrop would only eat the
    // page's clicks.
    let no_callback = Mounted::new(Popover {
        close_on_click_outside: true,
        ..Default::default()
    });
    assert!(no_callback.find("rinch-popover__backdrop").is_none());
}

/// The sheet must actually style that class. A backdrop with no rule is
/// `display: block`, `position: static` and zero-sized — the same bug one layer
/// down.
#[test]
fn the_sheet_styles_the_popover_backdrop() {
    let css = sheet();
    assert!(
        css.contains(".rinch-popover__backdrop {"),
        "no rule styles `.rinch-popover__backdrop`"
    );
    assert!(
        css.contains("position: fixed"),
        "the backdrop must be fixed — an absolute one is clipped by whatever \
         clips the popover, so \"outside\" stops at the enclosing panel"
    );
}
