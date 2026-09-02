//! `ColorInput`: `size` and `radius` reach the DOM (issue #263).
//!
//! Both were declared props that `render` never read — they reached neither the
//! field nor the dropdown. `size` now resolves to one of five steps and scales
//! the field and its preview swatch; `radius` emits a class only when set.
//!
//! A third prop, `close_on_click_outside`, was declared alongside them and is
//! **removed** rather than wired. The issue describes it as gating an existing
//! behaviour — "the dropdown closes on outside click unconditionally" — and that
//! is not what the component does. There is no backdrop and no outside-click
//! handler anywhere in it; the dropdown is dismissed by clicking the field
//! again. So the prop was not dead wiring over a live behaviour, it was a name
//! for a behaviour that does not exist, and supplying one is a new interaction
//! rather than the repair this change is. Filed separately.
//!
//! These tests assert against the rendered tree rather than the class helper
//! alone, so they fail if the classes stop being applied to the root, and they
//! cover the `--opened` effect, which rebuilds the root's class string from
//! scratch and is the obvious place for the new classes to be dropped.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::color_input::ColorInput;
use rinch_core::Component;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::events::{EventHandlerId, dispatch_event};

struct Mounted {
    _doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    root: NodeHandle,
}

impl Mounted {
    fn new(input: ColorInput) -> Self {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let root = input.render(&mut scope, &[]);
        Self {
            _doc: doc,
            _scope: scope,
            root,
        }
    }

    /// The root's class list, as rendered.
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

    fn find(&self, class: &str) -> Option<NodeHandle> {
        find_by_class(&self.root, class)
    }

    /// Fire the handler on the node carrying `class`.
    fn click(&self, class: &str) {
        let node = self.find(class).expect("the element exists");
        let id: usize = node
            .get_attribute("data-rid")
            .expect("the element carries a handler id")
            .parse()
            .expect("the handler id is numeric");
        dispatch_event(EventHandlerId(id));
    }

    fn is_open(&self) -> bool {
        self.has_class("rinch-color-input--opened")
    }

    /// The preview swatch's own `size`, which `ColorSwatch` renders as an
    /// inline `width`/`height`.
    fn swatch_style(&self) -> String {
        self.find("rinch-color-input__swatch-preview")
            .expect("the preview swatch exists")
            .get_attribute("style")
            .expect("the swatch is sized")
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

// ---------------------------------------------------------------- size

/// Every step reaches the root, and the swatch scales with it.
#[test]
fn size_reaches_the_root_class_and_the_preview_swatch() {
    for (size, class, swatch) in [
        ("xs", "rinch-color-input--xs", "16px"),
        ("sm", "rinch-color-input--sm", "18px"),
        ("md", "rinch-color-input--md", "22px"),
        ("lg", "rinch-color-input--lg", "26px"),
        ("xl", "rinch-color-input--xl", "30px"),
    ] {
        let m = Mounted::new(ColorInput {
            size: size.into(),
            ..Default::default()
        });
        assert!(
            m.has_class(class),
            "size {size:?} must render {class}; got {:?}",
            m.root_classes()
        );
        let style = m.swatch_style();
        assert!(
            style.contains(&format!("width: {swatch}")),
            "size {size:?} must scale the preview swatch to {swatch}; got {style:?}"
        );
    }
}

/// Exactly one size class, ever — otherwise two height rules would both apply
/// and the winner would depend on stylesheet order.
#[test]
fn exactly_one_size_class_is_emitted() {
    for size in ["", "xs", "sm", "md", "lg", "xl", "nonsense"] {
        let m = Mounted::new(ColorInput {
            size: size.into(),
            ..Default::default()
        });
        let n = m
            .root_classes()
            .iter()
            .filter(|c| {
                matches!(
                    c.as_str(),
                    "rinch-color-input--xs"
                        | "rinch-color-input--sm"
                        | "rinch-color-input--md"
                        | "rinch-color-input--lg"
                        | "rinch-color-input--xl"
                )
            })
            .count();
        assert_eq!(n, 1, "size {size:?} emitted {n} size classes");
    }
}

/// An unset or unrecognised size is `md`, matching `TextInput`, whose `size`
/// parses with `unwrap_or_default()` onto a `#[default] Md`. `md` is also the
/// step that reproduces the field's pre-#263 hard-coded geometry, so the
/// default rendering is unchanged.
#[test]
fn an_unset_or_unrecognised_size_falls_back_to_md() {
    for size in ["", "  ", "nonsense", "MEDIUM", "42"] {
        let m = Mounted::new(ColorInput {
            size: size.into(),
            ..Default::default()
        });
        assert!(
            m.has_class("rinch-color-input--md"),
            "size {size:?} must fall back to md; got {:?}",
            m.root_classes()
        );
    }
}

/// Spelling is normalized the way `TextInput`'s `FromStr` normalizes it.
#[test]
fn size_is_trimmed_and_case_insensitive() {
    for spelling in ["LG", " lg ", "Lg"] {
        let m = Mounted::new(ColorInput {
            size: spelling.into(),
            ..Default::default()
        });
        assert!(
            m.has_class("rinch-color-input--lg"),
            "size {spelling:?} must resolve to lg; got {:?}",
            m.root_classes()
        );
    }
}

// -------------------------------------------------------------- radius

#[test]
fn radius_reaches_the_root_class() {
    for (radius, class) in [
        ("xs", "rinch-color-input--radius-xs"),
        ("sm", "rinch-color-input--radius-sm"),
        ("md", "rinch-color-input--radius-md"),
        ("lg", "rinch-color-input--radius-lg"),
        ("xl", "rinch-color-input--radius-xl"),
    ] {
        let m = Mounted::new(ColorInput {
            radius: radius.into(),
            ..Default::default()
        });
        assert!(
            m.has_class(class),
            "radius {radius:?} must render {class}; got {:?}",
            m.root_classes()
        );
    }
}

/// Radius has no default class, unlike size: an unset or unrecognised value
/// emits none and leaves the base rule's `--rinch-radius-sm` standing. That is
/// the idiom `DropdownMenu`, `Modal` and `Card` already use, and it is why an
/// unset `radius` cannot change the current rendering.
#[test]
fn an_unset_or_unrecognised_radius_emits_no_radius_class() {
    for radius in ["", "nonsense", "12px"] {
        let m = Mounted::new(ColorInput {
            radius: radius.into(),
            ..Default::default()
        });
        assert!(
            !m.root_classes()
                .iter()
                .any(|c| c.starts_with("rinch-color-input--radius-")),
            "radius {radius:?} must emit no radius class; got {:?}",
            m.root_classes()
        );
    }
}

// ------------------------------------------------- dismissal (unchanged)

/// The only way to dismiss the dropdown is to click the field again. Pinned
/// here because it is easy to mistake for a gap rather than the design.
///
/// `ColorInput` has no click-outside dismissal: it mounts no backdrop and
/// registers no outside-click handler. It declared a `close_on_click_outside`
/// prop that read as if it did — that prop is removed in this change and the
/// behaviour it names is filed as its own piece of work, because adding it is a
/// new interaction rather than the wiring-up of an existing one.
#[test]
fn the_field_toggles_the_dropdown_and_is_the_only_way_to_dismiss_it() {
    let m = Mounted::new(ColorInput::default());

    m.click("rinch-color-input__input-group");
    assert!(m.is_open());
    m.click("rinch-color-input__input-group");
    assert!(!m.is_open());
}

// --------------------------------------------------- the opened effect

/// Opening the dropdown rebuilds the root's class string from scratch. It must
/// rebuild it from the *full* base, not from the bare `"rinch-color-input"` it
/// used to be built from — otherwise opening the dropdown would silently strip
/// the size and radius classes and resize the field mid-interaction.
#[test]
fn opening_the_dropdown_preserves_the_size_and_radius_classes() {
    let m = Mounted::new(ColorInput {
        size: "lg".into(),
        radius: "xl".into(),
        ..Default::default()
    });

    m.click("rinch-color-input__input-group");
    assert!(m.is_open(), "sanity: it opened");

    let classes = m.root_classes();
    for expected in ["rinch-color-input--lg", "rinch-color-input--radius-xl"] {
        assert!(
            classes.iter().any(|c| c == expected),
            "{expected} must survive opening the dropdown; got {classes:?}"
        );
    }
}

/// The same for the error and disabled modifiers, which moved into
/// `class_string` with the new ones.
#[test]
fn opening_the_dropdown_preserves_the_error_and_disabled_classes() {
    let m = Mounted::new(ColorInput {
        error: "bad colour".into(),
        disabled: true,
        ..Default::default()
    });

    m.click("rinch-color-input__input-group");
    let classes = m.root_classes();
    for expected in [
        "rinch-color-input--error",
        "rinch-color-input--disabled",
        "rinch-color-input--opened",
    ] {
        assert!(
            classes.iter().any(|c| c == expected),
            "{expected} must survive opening the dropdown; got {classes:?}"
        );
    }
}

// -------------------------------------- the classes reach the stylesheet

/// Every class the component emits for these props must be backed by a rule
/// that actually *declares something*, in the sheet the app ships.
///
/// This is the assertion that pins #263 shut. The bug was never that
/// `class_string` looked wrong — it was that the props reached nothing that
/// renders. A test reading only the class string is satisfied by a component
/// emitting `rinch-color-input--lg` into a stylesheet that has never heard of
/// it, which is the same defect wearing a different hat.
///
/// It checks declarations rather than the mere presence of the selector,
/// because presence is too weak. A class can be *named* by a rule that gives it
/// nothing — a selector list it appears in for an unrelated reason, or a rule
/// whose one declaration is later removed — and a presence-only check stays
/// green through that. Asserting the declaration that makes the prop visible
/// (`height` for a size step, `border-radius` for a radius step) is what ties
/// the class to an actual effect.
#[test]
fn every_emitted_size_and_radius_class_is_backed_by_a_real_rule() {
    let css = strip_comments(&rinch_components::styles::generate_all_component_styles());

    let mut sizes: Vec<String> = Vec::new();
    let mut radii: Vec<String> = Vec::new();
    for step in ["xs", "sm", "md", "lg", "xl"] {
        let m = Mounted::new(ColorInput {
            size: step.into(),
            radius: step.into(),
            ..Default::default()
        });
        for c in m.root_classes() {
            if c.starts_with("rinch-color-input--radius-") {
                radii.push(c);
            } else if c.starts_with("rinch-color-input--") && c != "rinch-color-input--opened" {
                sizes.push(c);
            }
        }
    }
    sizes.sort();
    sizes.dedup();
    radii.sort();
    radii.dedup();

    // If either of these drops the loop stopped producing classes and every
    // assertion below would pass vacuously.
    assert_eq!(sizes.len(), 5, "expected 5 size classes; got {sizes:?}");
    assert_eq!(radii.len(), 5, "expected 5 radius classes; got {radii:?}");

    for class in &sizes {
        let decls = declarations_for(&css, &format!(".{class} .rinch-color-input__input"));
        assert!(
            decls.iter().any(|d| d.starts_with("height:")),
            "`{class}` is emitted by ColorInput but sets no height in the shipped \
             stylesheet — the prop would reach the DOM and still do nothing, \
             which is the #263 defect. Declarations found: {decls:?}"
        );
    }

    for class in &radii {
        let decls = declarations_for(&css, &format!(".{class} .rinch-color-input__input-group"));
        assert!(
            decls.iter().any(|d| d.starts_with("border-radius:")),
            "`{class}` is emitted by ColorInput but sets no border-radius in the \
             shipped stylesheet. Declarations found: {decls:?}"
        );
    }
}

/// Every declaration in every rule whose selector list contains exactly
/// `selector`, whitespace-normalized and lowercased.
fn declarations_for(css: &str, selector: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = css;
    while let Some(open) = rest.find('{') {
        let prelude = rest[..open].rsplit('}').next().unwrap_or("").trim();
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        let body = &rest[open + 1..open + close];
        let matches = prelude
            .split(',')
            .any(|s| s.split_whitespace().collect::<Vec<_>>().join(" ") == selector);
        if matches {
            found.extend(
                body.split(';')
                    .map(|d| {
                        d.split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                            .to_lowercase()
                    })
                    .filter(|d| !d.is_empty()),
            );
        }
        rest = &rest[open + close + 1..];
    }
    found
}

/// Strip `/* ... */` comments before parsing.
///
/// Load-bearing, and verified so: with this reduced to the identity the test
/// fails, because the comment introducing the size rules is swept into the
/// prelude of the rule beneath it and stops the selector matching.
fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find("*/") {
            Some(end) => rest = &rest[start + 2 + end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}
