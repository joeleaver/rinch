//! The pure rules of popups (#2552): the chain's close order, where keys go,
//! what a press closes, the anchor rectangle and the reading of a placement.

use super::chain;
use super::*;
use rinch_core::element::PopupKind::{Menu, Tooltip};

/// The main window 1 holds a menu 5; on it a submenu 7 and a tooltip 8; on
/// the submenu a submenu 9. A popup 6 owned by the main window but anchored
/// in the menu (parent 5) is above the menu too.
fn links() -> Vec<(u64, u64, PopupKind)> {
    vec![
        (5, 1, Menu),
        (6, 5, Menu),
        (7, 5, Menu),
        (8, 5, Tooltip),
        (9, 7, Menu),
    ]
}

#[test]
fn a_window_closes_every_popup_above_it_topmost_first() {
    let l = links();
    // Newest first: 9 before 7 (it is on 7), the menu's popups before it.
    assert_eq!(chain::close_order(1, &l), vec![9, 8, 7, 6, 5]);
    assert_eq!(chain::close_order(5, &l), vec![9, 8, 7, 6]);
    assert_eq!(chain::close_order(7, &l), vec![9]);
    assert!(chain::close_order(9, &l).is_empty());
    // Each popup comes before the one it is on: never a parent first.
    let order = chain::close_order(1, &l);
    for &(id, parent, _) in &l {
        if let Some(p) = order.iter().position(|&x| x == parent) {
            let c = order.iter().position(|&x| x == id).unwrap();
            assert!(c < p, "{id} closes before its parent {parent}");
        }
    }
}

#[test]
fn the_parent_relation_orders_the_close_not_the_owner() {
    // 6 is owned by the main window (opened in its code) yet anchored in
    // the menu: it is above the menu, so it closes when the menu does.
    let l = links();
    assert!(chain::close_order(5, &l).contains(&6));
    assert_eq!(chain::root(6, &l), 1);
}

#[test]
fn keys_go_to_the_topmost_menu_of_the_chain() {
    let l = links();
    // From the panel, from the menu, from the submenu: the newest menu.
    for at in [1, 5, 7, 9] {
        assert_eq!(chain::key_target(at, &l), Some(9), "from {at}");
    }
    // A tooltip is never the target, even the newest popup.
    let tip_on_top = vec![(5, 1, Menu), (8, 5, Tooltip)];
    assert_eq!(chain::key_target(1, &tip_on_top), Some(5));
    // A window of another chain keeps its keys.
    assert_eq!(chain::key_target(2, &l), None);
    // Only a tooltip: no menu, nothing forwarded.
    assert_eq!(chain::key_target(1, &[(3, 1, Tooltip)]), None);
}

#[test]
fn a_press_closes_the_menus_above_it_and_no_others() {
    let l = links();
    // In the panel: every menu.
    let mut all = chain::press_closes(1, &l);
    all.sort_unstable();
    assert_eq!(all, vec![5, 6, 7, 9]);
    // In the first menu: what is above it.
    let mut above = chain::press_closes(5, &l);
    above.sort_unstable();
    assert_eq!(above, vec![6, 7, 9]);
    // In the topmost: nothing of its own chain below it, only the
    // sibling submenu 6 that is not under it.
    assert_eq!(chain::press_closes(9, &l), vec![6]);
    assert!(chain::press_closes(9, &[(5, 1, Menu), (7, 5, Menu), (9, 7, Menu)]).is_empty());
    // In the submenu 7: its sibling 6 and the 9 above it, nothing below.
    assert_eq!(chain::press_closes(7, &l), vec![6, 9]);
    // A tooltip is never closed by a press.
    assert!(!chain::press_closes(1, &l).contains(&8));
}

#[test]
fn the_anchor_rect_is_zoomed_kept_in_the_parent_and_rounded_outward() {
    // Off the integer grid at zoom 1.5: 10.2..40.6 x 3.3..27.9 CSS px.
    let r = anchor_rect((10.2, 3.3, 30.4, 24.6), 1.5, (1920.0, 40.0));
    // x 15.3 -> 15, right 60.9 -> 61; y 4.95 -> 4, bottom 41.85 clamped 40.
    assert_eq!(r, [15, 4, 46, 36]);
    // Outside the parent on the left and the top: clamped in, 1x1 at least.
    assert_eq!(
        anchor_rect((-50.0, -5.0, 10.0, 2.0), 1.0, (100.0, 40.0)),
        [0, 0, 1, 1]
    );
    // Past the right edge: kept inside, still at least one px wide.
    assert_eq!(
        anchor_rect((150.0, 10.0, 20.0, 10.0), 1.0, (100.0, 40.0)),
        [99, 10, 1, 10]
    );
}

#[test]
fn a_placement_reads_a_flip_on_its_sides_axis() {
    let anchor = [100, 1040, 40, 40]; // a button in a bottom bar
    // Asked below, put above (flipped): the popup ends at the anchor's top.
    let p = placed_from(
        anchor,
        Placement::BOTTOM_START,
        (100.0, 840.0),
        (200.0, 200.0),
    );
    assert!(p.flipped_y && !p.flipped_x);
    // Asked above and put above: no flip.
    let p = placed_from(anchor, Placement::TOP_START, (100.0, 840.0), (200.0, 200.0));
    assert!(!p.flipped_y);
    // A submenu asked to the right, put to the left of its row.
    let p = placed_from(
        [300, 50, 200, 24],
        Placement::RIGHT_START,
        (100.0, 50.0),
        (200.0, 300.0),
    );
    assert!(p.flipped_x && !p.flipped_y);
    // Slid along the bar (x moved), not flipped.
    let p = placed_from(anchor, Placement::TOP_START, (20.0, 840.0), (200.0, 200.0));
    assert!(!p.flipped_x && !p.flipped_y);
    assert_eq!((p.x, p.y, p.width, p.height), (20.0, 840.0, 200.0, 200.0));
}

#[test]
fn the_default_adjust_flips_and_slides_on_both_axes() {
    let a = Adjust::default();
    assert!(a.contains(Adjust::FLIP_X | Adjust::FLIP_Y | Adjust::SLIDE_X | Adjust::SLIDE_Y));
    assert!(!a.contains(Adjust::RESIZE_X) && !a.contains(Adjust::RESIZE_Y));
}

#[test]
fn a_tooltip_takes_no_pointer_input_and_a_popup_is_never_a_layer_surface() {
    let tip = window_props_for(&PopupProps {
        kind: Tooltip,
        ..PopupProps::default()
    });
    assert_eq!(tip.input_region, InputRegion::None);
    assert_eq!(tip.wayland_layer_shell, Some(false));
    assert!(tip.size_to_content.is_some());
    let menu = window_props_for(&PopupProps {
        max_size: Some((300.5, 400.0)),
        ..PopupProps::default()
    });
    assert_eq!(menu.input_region, InputRegion::All);
    let sizing = menu.size_to_content.unwrap();
    assert_eq!(
        (sizing.max_width, sizing.max_height),
        (Some(301), Some(400))
    );
}

#[test]
fn only_the_first_menu_of_a_chain_may_raise_its_parents_keyboard() {
    // The menu 5 being made is registered already: it does not count.
    assert!(chain::may_raise_keyboard(5, 1, &[(5, 1, Menu)]));
    // A tooltip open on the bar does not stop it.
    assert!(chain::may_raise_keyboard(
        5,
        1,
        &[(3, 1, Tooltip), (5, 1, Menu)]
    ));
    // Another menu of the chain holds a grab: never.
    assert!(!chain::may_raise_keyboard(
        6,
        1,
        &[(5, 1, Menu), (6, 1, Menu)]
    ));
    // A submenu (its parent is a popup): never.
    assert!(!chain::may_raise_keyboard(
        7,
        5,
        &[(5, 1, Menu), (7, 5, Menu)]
    ));
}

#[test]
fn a_node_across_anchor_spans_the_parents_cross_axis() {
    let parent = (1280.0, 40.0);
    // Above or below: the bar's whole height under the button's columns.
    assert_eq!(across([8, 8, 62, 24], Side::Top, parent), [8, 0, 62, 40]);
    assert_eq!(across([8, 8, 62, 24], Side::Bottom, parent), [8, 0, 62, 40]);
    // Beside: the whole width beside the row.
    assert_eq!(
        across([8, 8, 62, 24], Side::Right, (200.0, 300.0)),
        [0, 8, 200, 24]
    );
    // A fractional parent rounds outward.
    assert_eq!(
        across([1, 2, 3, 4], Side::Top, (100.0, 39.4)),
        [1, 0, 3, 40]
    );
}
