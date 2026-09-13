//! `z_index` on the five overlay components, end to end (issue #474).
//!
//! `Modal`, `Drawer`, `Popover`, `DropdownMenu` and `Notification` each declared
//! a `z_index` prop that nothing read — every one of them hard-coded its level
//! in CSS. The prop now publishes one inherited custom property on the
//! component's root, and the stylesheet spends it: the base layer reads it
//! directly, and a second layer that has to stay ordered against the first
//! (`Modal`/`Drawer`'s panel above their overlay, `DropdownMenu`'s backdrop below
//! its panel) derives its own level with a `calc()`. So one prop moves the whole
//! component and the internal ordering is preserved by construction rather than
//! by the caller remembering to offset two numbers.
//!
//! `rinch-components`' own `prop_wiring_474` fixtures pin the two halves —
//! the property is published, the sheet reads it. They cannot pin the join:
//! `MockDomDocument` has no CSS engine, so nothing there would notice if Stylo
//! stopped substituting a custom property into `z-index` (or into a `calc()` in
//! one). That substitution is the whole mechanism, and it is measured here,
//! through the real document, the real stylesheet and the real cascade.
//!
//! The fixtures deliberately ask for **517**: it is no component's default, is
//! adjacent to none of them, and makes a `+ 1`/`- 1` mix-up read as 518/516
//! rather than as some other component's number.

use super::*;

use rinch_components::{
    Drawer, DropdownMenu, DropdownMenuDropdown, DropdownMenuTarget, Modal, Notification, Popover,
    PopoverDropdown, PopoverTarget,
};
use rinch_core::{Callback, Component};

const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// The level the prop asks for, distinct from every default in play.
const ASKED: i32 = 517;

/// Mount one of each overlay, all five with the same `z_index`, under the real
/// component stylesheet.
fn mount(z_index: Option<i32>) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");

        // Modal: root (the full-viewport overlay) + panel.
        let modal = Modal {
            opened: true,
            z_index,
            ..Default::default()
        }
        .render(scope, &[]);
        root.append_child(&modal);

        // Drawer: same shape.
        let drawer = Drawer {
            opened: true,
            z_index,
            ..Default::default()
        }
        .render(scope, &[]);
        root.append_child(&drawer);

        // Popover: the dropdown is a separate component, so the level has to
        // reach it by inheritance from the popover's own root.
        let popover_target = PopoverTarget.render(scope, &[]);
        let popover_dropdown = PopoverDropdown.render(scope, &[]);
        let popover = Popover {
            z_index,
            ..Default::default()
        }
        .render(scope, &[popover_target, popover_dropdown]);
        root.append_child(&popover);

        // DropdownMenu: panel + the backdrop that catches outside clicks, which
        // only exists when there is an `on_close` to fire.
        let menu_props = DropdownMenu {
            opened: true,
            on_close: Some(Callback::new(|| {})),
            z_index,
            ..Default::default()
        };
        let menu_target = DropdownMenuTarget.render(scope, &[]);
        let menu_dropdown = DropdownMenuDropdown.render(scope, &[]);
        let menu = menu_props.render(scope, &[menu_target, menu_dropdown]);
        root.append_child(&menu);

        // Notification: one box, its own level.
        let notification = Notification {
            opened: true,
            z_index,
            ..Default::default()
        }
        .render(scope, &[]);
        root.append_child(&notification);

        root
    });

    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    app
}

/// The computed `z-index` of the one node carrying `class` exactly.
fn computed_z(app: &RinchApp, class: &str) -> Option<i32> {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let matches: Vec<usize> = d
        .tree
        .nodes
        .iter()
        .filter(|(_, n)| {
            n.attributes
                .get("class")
                .is_some_and(|c| c.split_whitespace().any(|one| one == class))
        })
        .map(|(id, _)| id)
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one node carrying `{class}`, found {}",
        matches.len()
    );
    d.tree
        .get(matches[0])
        .expect("the node is in the tree")
        .computed_style
        .z_index
}

/// Every layer, with the level it must compute to at `ASKED` and with no prop.
/// The pairs are the point: a prop that moved one layer of a pair and not the
/// other would put a backdrop over the panel it guards, or a panel under its own
/// overlay.
const LAYERS: &[(&str, i32, i32)] = &[
    // (class, with z_index: 517, with no z_index)
    ("rinch-modal__root", ASKED, 200),
    ("rinch-modal", ASKED + 1, 201),
    ("rinch-drawer__root", ASKED, 200),
    ("rinch-drawer", ASKED + 1, 201),
    ("rinch-popover__dropdown", ASKED, 100),
    ("rinch-dropdown-menu__dropdown", ASKED, 100),
    ("rinch-dropdown-menu__backdrop", ASKED - 1, 99),
    ("rinch-notification", ASKED, 300),
];

#[test]
fn z_index_moves_every_layer_of_its_overlay() {
    let app = mount(Some(ASKED));
    for (class, asked, _) in LAYERS {
        assert_eq!(
            computed_z(&app, class),
            Some(*asked),
            "`{class}` must compute to {asked} when the component is given \
             z_index: {ASKED}"
        );
    }
}

#[test]
fn an_unset_z_index_leaves_the_stylesheets_own_levels() {
    let app = mount(None);
    for (class, _, default) in LAYERS {
        assert_eq!(
            computed_z(&app, class),
            Some(*default),
            "`{class}` must keep its stylesheet level of {default} when no \
             z_index is given"
        );
    }
}

/// The orderings the pairs exist for, stated as orderings rather than as
/// numbers — this is what a future change to either half has to keep true.
#[test]
fn the_pairs_stay_ordered_at_every_level() {
    for z in [None, Some(ASKED), Some(3), Some(-4)] {
        let app = mount(z);
        let panel_over_overlay = [
            ("rinch-modal", "rinch-modal__root"),
            ("rinch-drawer", "rinch-drawer__root"),
            (
                "rinch-dropdown-menu__dropdown",
                "rinch-dropdown-menu__backdrop",
            ),
        ];
        for (over, under) in panel_over_overlay {
            let top = computed_z(&app, over).expect("a level");
            let bottom = computed_z(&app, under).expect("a level");
            assert!(
                top > bottom,
                "at z_index {z:?}, `{over}` ({top}) must stay above `{under}` \
                 ({bottom})"
            );
        }
    }
}
