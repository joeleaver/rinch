//! #747, at the shell — which `Loader`s let the app go idle, and which do not.
//!
//! The rinch-dom half is
//! `crates/rinch-dom/tests/display_none_animation_tests.rs`, which asserts on
//! `active_animations` and on `tick_animations`. This file asserts the thing
//! those two decide, which is the reason the issue was filed: the desktop frame
//! clock's `AboutToWait` arm schedules another frame whenever
//! `tree.active_animations` holds a running (not paused, #763) animation
//! (`app/event_dispatch.rs`), so an
//! animation running on something nobody paints keeps a desktop app rendering
//! at full rate, indefinitely, with nothing on screen moving.
//!
//! A transition cannot do that — it has a declared duration and dies after it.
//! An `animation: … infinite` has no end, which is why
//! `reinsertion_transition_tests::a_detached_animation_stops_asking_for_frames`
//! exists for the *removed* case (#699) and why this exists for the hidden one.
//!
//! # `display: none` and `visibility: hidden` answer differently, and both are right
//!
//! The two hidden spellings are not interchangeable here, and the shipped
//! component library uses both, so both are pinned.
//!
//! A **`display: none`** panel is not being rendered: its `Loader` runs nothing
//! and the app sleeps. That is #747, and `a_loader_in_a_display_none_panel_lets_
//! the_app_go_idle` is its pin.
//!
//! A **`visibility: hidden`** panel *is* being rendered — it keeps its box, it
//! transitions, it is merely not painted — so its `Loader`'s animation stays
//! registered, and runs unless something pauses it. That is browser-correct
//! (measured in Chrome) and it is the same answer the transition rule beside it
//! gives, but running it is not free, and since **#751** it is reachable through
//! a shipped component: the closed `Drawer`'s root is `visibility: hidden` now,
//! precisely so its panel can transition. Measured here before #912, software
//! backend, 804x600, 20 idle frames:
//!
//! | closed-state spelling | `active_animations` | idle frames asking to redraw | ms per tick+paint |
//! |---|---|---|---|
//! | `display: none` | 0 | 0 / 20 | 0.001 |
//! | `visibility: hidden`, running (the `Drawer` before #912) | 1 | 20 / 20 | 2.53 |
//! | `visibility: hidden`, paused (the `Drawer` since #912) | 1 | 0 / 20 | — |
//! | open drawer, either spelling | 1 | 20 / 20 | 8.8 – 9.7 |
//!
//! Refusing an animation to a `visibility: hidden` box would put desktop at odds
//! with both the browser and the transition rule next to it, for one
//! component's benefit. The cure belongs to the component —
//! `animation-play-state: paused` on a closed overlay's subtree — which works
//! since **#763** (a paused animation asks for no frames) and which the
//! component library declares since **#912**: the closed `Drawer`, a closed
//! `Popover`'s dropdown and an unhovered `HoverCard`'s dropdown pause every
//! animation beneath them. The fixtures in the second half of this file pin
//! each, with the shipped stylesheet and nothing else.
//!
//! # Mutants, and what kills each
//!
//! Measured over `cargo test -p rinch-dom -p rinch --no-fail-fast`, each mutant
//! applied to the committed source and reverted afterwards. See the rinch-dom
//! file's table for the full set; these are the rows this file participates in.
//!
//! | mutant | killed by |
//! |---|---|
//! | the gate never refuses (i.e. `main`) | `a_loader_in_a_display_none_panel_lets_the_app_go_idle` and `opening_a_display_none_panel_starts_the_loader_spinning` |
//! | the gate reads the node's **own** display only | the same two — the `Loader` is not the hidden node, the panel two levels above its oval is |
//! | the gate checks the **immediate parent** instead of walking | the same two, for the same reason, and they are two of the three fixtures in the whole scope that kill it |
//! | `visibility: hidden` folded into "not rendered" | `a_loader_in_a_closed_drawer_is_paused_and_lets_the_app_idle` and the popover / hover-card fixtures (their "registered" preconditions) |
//! | #912's closed rules removed (`.rinch-drawer__root--hidden *` etc. → a selector that matches nothing) | `a_loader_in_a_closed_drawer_is_paused_and_lets_the_app_idle`, `an_app_declared_animation_in_a_closed_drawer_is_paused_too`, `a_loader_in_a_closed_popover_is_paused_until_it_opens`, `a_loader_in_an_unhovered_hover_card_is_paused_until_hovered` (each its own overlay) |
//! | a closed rule that pauses **always** (its `--hidden` / `:not(…)` dropped) | `opening_the_drawer_resumes_its_loader`, `a_loader_in_an_open_drawer_keeps_asking_for_frames`, and the popover / hover-card fixtures' reopen halves |
//! | the drawer rule without `!important` | `an_app_declared_animation_in_a_closed_drawer_is_paused_too`, alone — the shipped `.rinch-loader__oval` ties on specificity and is declared *before* the drawer sheet, so the `Loader` fixtures pass without it |
//!
//! `a_loader_in_an_open_drawer_keeps_asking_for_frames` kills none of them, by
//! design: it is the positive control every zero above rests on, and it passes
//! on `main` too.

// `rsx!` writes absolute `rinch::` paths, and this *is* the rinch crate.
use super::*;
use crate as rinch;
use rinch_components::{
    Drawer, HoverCard, HoverCardDropdown, HoverCardTarget, Loader, Popover, PopoverDropdown,
};
use rinch_core::{Component, Signal};
use rinch_macros::rsx;

/// The size the app is mounted at.
const MOUNT: (f32, f32) = (800.0, 600.0);
/// The size it settles at, and the size the frame clock is pumped at. It
/// differs from [`MOUNT`] by more than half a pixel — see `settle` for why that
/// was once necessary and no longer is.
const SETTLED: (f32, f32) = (804.0, 600.0);
const PHYSICAL: (u32, u32) = (804, 600);

/// The spinning element: the oval inside the `Loader`.
const OVAL: &str = "rinch-loader__oval";

fn node_with_class(app: &RinchApp, class: &str) -> usize {
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
    matches[0]
}

/// How many animations are registered on a node.
fn animations(app: &RinchApp, node: usize) -> usize {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .active_animations
        .get(&node)
        .map(|v| v.len())
        .unwrap_or(0)
}

/// Whether the node's one animation is paused.
fn is_paused(app: &RinchApp, node: usize) -> bool {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let running = d
        .tree
        .active_animations
        .get(&node)
        .expect("no animation is registered on this node");
    assert_eq!(running.len(), 1, "this helper assumes exactly one");
    running[0].play_state == rinch_dom::animation::AnimationPlayState::Paused
}

/// A node's box origin in window coordinates: its parent-relative `layout`
/// summed up the parent chain (nothing in these fixtures is scrolled or
/// transformed).
fn window_origin(app: &RinchApp, node: usize) -> (f32, f32) {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let (mut x, mut y) = (0.0, 0.0);
    let mut at = Some(node);
    while let Some(id) = at {
        let n = d.tree.get(id).unwrap();
        x += n.layout.x;
        y += n.layout.y;
        at = n.parent;
    }
    (x, y)
}

/// How many `animation` declarations the cascade found on the node. Zero would
/// mean this file is measuring a component that declares no animation at all,
/// which is the fixed point every "0 animations" assertion has to be kept off.
fn animation_specs(app: &RinchApp, node: usize) -> usize {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.get(node).unwrap().animation_specs.len()
}

fn display_of(app: &RinchApp, node: usize) -> rinch_dom::computed_style::DisplayValue {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.get(node).unwrap().computed_style.display
}

/// Load the shipped component stylesheet, settle, and get the animations
/// actually registered.
///
/// **The last step was written for two faults that are fixed now.** When #747
/// landed, `transitions_enabled` gated animation *starts* as well as transition
/// starts: the first layout cascades with the flag still `false`, and
/// `recompute_all_styles_full` — how a theme change and this harness install a
/// stylesheet — cleared `active_animations` and re-cascaded with it forced
/// `false`. So no animation was running after the first two steps, and the
/// resize to [`SETTLED`] (more than half a pixel, so `resolve_layout` drops
/// every cached style) was the re-cascade that started them. That was issue
/// **#762**; since it was fixed, the first two steps start them on their own.
/// Measured: with `SETTLED` set equal to `MOUNT`, so the last resolve re-cascades
/// nothing, all five fixtures here still pass. The resize is kept rather than
/// deleted because the mutant table above was measured through it.
///
/// It goes straight to the document because `RinchApp::resolve_and_repaint`
/// short-circuits on a clean tree, and the tree *is* clean by here: the resize is
/// the thing that dirties it, and the check runs first.
///
/// The pump in [`idle_frames_requesting_redraw`] runs at this same size, so no
/// frame it drives re-cascades anything — which would request a redraw of its
/// own and say nothing about animations.
fn settle(app: &mut RinchApp) {
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(MOUNT.0, MOUNT.1);
    {
        let doc = app.doc.as_ref().unwrap();
        doc.borrow_mut().resolve_layout(SETTLED.0, SETTLED.1);
    }
}

/// A `Loader` inside a `Drawer`, with the signal that opens the drawer.
fn mount_loader_in_drawer(opened_at_start: bool) -> (RinchApp, Signal<bool>) {
    let opened = Signal::new(opened_at_start);
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let loader = Loader::default().render(scope, &[]);
        let drawer = Drawer {
            opened_fn: Some(std::rc::Rc::new(move || opened.get())),
            position: "left".to_string(),
            ..Default::default()
        }
        .render(scope, &[loader]);
        root.append_child(&drawer);
        root
    });

    app.mount_component(MOUNT.0, MOUNT.1);
    settle(&mut app);
    (app, opened)
}

/// A `Loader` inside a plain panel whose `display` is toggled by a signal.
///
/// Not the `Drawer`: since #751 the `Drawer`'s closed root is
/// `visibility: hidden`, which is *rendered*. This is the hand-built panel that
/// keeps #747's own subject — a spinner that is genuinely not being rendered —
/// pinned at the shell.
fn mount_loader_in_panel(open_at_start: bool) -> (RinchApp, Signal<bool>) {
    let open = Signal::new(open_at_start);
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        rsx! {
            div {
                div {
                    class: "panel-747",
                    style: {move || if open.get() {
                        "display: block; width: 200px; height: 200px"
                    } else {
                        "display: none; width: 200px; height: 200px"
                    }},
                    {Loader::default().render(__scope, &[])}
                }
            }
        }
    });

    app.mount_component(MOUNT.0, MOUNT.1);
    settle(&mut app);
    (app, open)
}

/// Pump the frame clock `n` times, the way the desktop event loop does when
/// nothing else is happening, and report how many of those frames asked the
/// shell to redraw.
fn idle_frames_requesting_redraw(app: &mut RinchApp, n: usize) -> usize {
    let mut asked = 0;
    for _ in 0..n {
        let actions = app.handle_event(PlatformEvent::AboutToWait, PHYSICAL, 1.0);
        if actions.contains(&AppAction::RequestRedraw) {
            asked += 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(4));
    }
    asked
}

// ── The `display: none` half: #747's own subject ─────────────────────────────

/// The issue: a `Loader` in a panel that is **not being rendered** must let the
/// app go idle.
///
/// The `Loader` is not itself hidden — the panel two levels above its oval is —
/// so this is the ancestor case, and a gate reading the oval's own computed
/// `display` (which is `block`) would not see it.
#[test]
fn a_loader_in_a_display_none_panel_lets_the_app_go_idle() {
    let (mut app, _open) = mount_loader_in_panel(false);
    let oval = node_with_class(&app, OVAL);

    assert!(
        animation_specs(&app, oval) > 0,
        "precondition: the cascade did find the `Loader`'s animation, so a \
         zero below means it was refused rather than never seen"
    );
    assert_ne!(
        display_of(&app, oval),
        rinch_dom::computed_style::DisplayValue::None,
        "precondition: the oval's own display is not `none` — it is the panel \
         above it that is hidden"
    );
    assert_eq!(
        animations(&app, oval),
        0,
        "a spinner in a `display: none` panel is not being rendered, so it runs \
         nothing"
    );

    assert_eq!(
        idle_frames_requesting_redraw(&mut app, 4),
        0,
        "and the app is allowed to sleep — this is what the issue was filed \
         about, an app rendering at full rate with nothing on screen moving"
    );
}

/// The drop must not be one-way: opening the panel brings the spinner back, and
/// the frames with it.
#[test]
fn opening_a_display_none_panel_starts_the_loader_spinning() {
    let (mut app, open) = mount_loader_in_panel(false);
    let oval = node_with_class(&app, OVAL);
    assert_eq!(animations(&app, oval), 0, "precondition: closed and still");
    assert_eq!(
        idle_frames_requesting_redraw(&mut app, 2),
        0,
        "precondition: and idle"
    );

    open.set(true);
    app.resolve_and_repaint(SETTLED.0, SETTLED.1);

    assert_eq!(
        animations(&app, oval),
        1,
        "the spinner spins again once the panel is open"
    );
    assert_eq!(
        idle_frames_requesting_redraw(&mut app, 4),
        4,
        "and the frame clock is running again"
    );
}

// ── The `visibility: hidden` half: the shipped `Drawer`, after #751 ──────────

/// The control the file rests on: an **open** drawer's `Loader` keeps the frame
/// clock running, because it is genuinely on screen and genuinely moving.
///
/// Without this, "the panel asks for no frames" above would also pass against a
/// build where a `Loader` never animates at all, or where `AboutToWait` never
/// requests a redraw for any reason.
#[test]
fn a_loader_in_an_open_drawer_keeps_asking_for_frames() {
    let (mut app, _opened) = mount_loader_in_drawer(true);
    let oval = node_with_class(&app, OVAL);

    assert!(
        animation_specs(&app, oval) > 0,
        "precondition: the shipped `Loader` really does declare an animation"
    );
    assert_eq!(
        animations(&app, oval),
        1,
        "precondition: it is running on a rendered drawer"
    );

    assert_eq!(
        idle_frames_requesting_redraw(&mut app, 4),
        4,
        "a spinner nobody has hidden must be repainted every frame"
    );
}

/// A `Loader` in a **closed** `Drawer` is still registered — the drawer is
/// rendered — but it is **paused**, so the app idles (#912).
///
/// Since #751 the closed drawer's root is `visibility: hidden` rather than
/// `display: none`, so that its panel can transition. Such a box is being
/// rendered — it keeps its box, it is merely not painted — so #747's rule leaves
/// its animations alone, which is what a browser does and what the transition
/// rule beside it already said. Left there, an `animation: … infinite` has no
/// duration to expire and the app asked for a redraw on every idle frame,
/// forever. The drawer's closed rule now declares
/// `animation-play-state: paused` over its whole subtree, and a paused animation
/// asks for no frames (#763).
#[test]
fn a_loader_in_a_closed_drawer_is_paused_and_lets_the_app_idle() {
    let (mut app, _opened) = mount_loader_in_drawer(false);
    let oval = node_with_class(&app, OVAL);

    assert!(
        animation_specs(&app, oval) > 0,
        "precondition: the `Loader` declares an animation"
    );
    assert_ne!(
        display_of(&app, oval),
        rinch_dom::computed_style::DisplayValue::None,
        "precondition: nothing here is `display: none` — #751 made the closed \
         drawer `visibility: hidden`, which is rendered"
    );
    assert_eq!(
        animations(&app, oval),
        1,
        "a `visibility: hidden` ancestor does not drop an animation, in rinch \
         or in a browser"
    );
    assert!(is_paused(&app, oval), "but the drawer's closed rule pauses it");
    assert_eq!(
        idle_frames_requesting_redraw(&mut app, 4),
        0,
        "so a closed drawer holding a `Loader` lets the app sleep"
    );
}

/// Opening the drawer **resumes** its `Loader`: the same animation, running
/// again, with the frame clock back on.
///
/// The half that is easy to get wrong in the other direction: the oval was
/// rendered the whole time, so nothing dropped its animation and nothing may
/// mint it a new one — it only changes play state.
/// `paused_animation_frames_tests::a_loader_in_a_closed_drawer_idles_and_resumes_where_it_paused`
/// checks, off the fixed point, that the resumed spinner keeps the time it had
/// already spent rather than restarting from zero.
#[test]
fn opening_the_drawer_resumes_its_loader() {
    let (mut app, opened) = mount_loader_in_drawer(false);
    let oval = node_with_class(&app, OVAL);
    assert!(is_paused(&app, oval), "precondition: closed and paused");

    opened.set(true);
    app.resolve_and_repaint(SETTLED.0, SETTLED.1);

    assert_eq!(animations(&app, oval), 1, "still exactly one");
    assert!(!is_paused(&app, oval), "running again");
    assert_eq!(
        idle_frames_requesting_redraw(&mut app, 4),
        4,
        "and the frame clock is running again"
    );
}

/// Every registered animation in the document, as `(total, paused)`.
fn registered(app: &RinchApp) -> (usize, usize) {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let all: Vec<_> = d.tree.active_animations.values().flatten().collect();
    let paused = all
        .iter()
        .filter(|a| a.play_state == rinch_dom::animation::AnimationPlayState::Paused)
        .count();
    (all.len(), paused)
}

/// Content that is not a `Loader`, inside a `Drawer`, with an extra app
/// stylesheet loaded after the component one.
fn mount_in_drawer(
    opened_at_start: bool,
    app_css: &'static str,
    content: fn(&mut RenderScope) -> NodeHandle,
) -> (RinchApp, Signal<bool>) {
    let opened = Signal::new(opened_at_start);
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let inner = content(scope);
        let drawer = Drawer {
            opened_fn: Some(std::rc::Rc::new(move || opened.get())),
            position: "left".to_string(),
            ..Default::default()
        }
        .render(scope, &[inner]);
        root.append_child(&drawer);
        root
    });
    app.mount_component(MOUNT.0, MOUNT.1);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&format!(
            "{}\n{app_css}",
            rinch_components::generate_component_css()
        ));
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(MOUNT.0, MOUNT.1);
    {
        let doc = app.doc.as_ref().unwrap();
        doc.borrow_mut().resolve_layout(SETTLED.0, SETTLED.1);
    }
    (app, opened)
}

/// An animation the **app** declares, in a stylesheet loaded after the
/// component one. Its `animation` shorthand resets the play state and ties the
/// closed rule's `*` on specificity, so it is the declaration order that would
/// decide — and the app's comes last. The closed rule is `!important` so that
/// it still wins.
#[test]
fn an_app_declared_animation_in_a_closed_drawer_is_paused_too() {
    fn spinner(scope: &mut RenderScope) -> NodeHandle {
        let el = scope.create_element("div");
        el.set_attribute("class", "spin-912");
        el
    }
    let (mut app, opened) = mount_in_drawer(
        false,
        "@keyframes spin-912 { to { transform: rotate(360deg); } }
         .spin-912 { width: 20px; height: 20px; animation: spin-912 1s linear infinite; }",
        spinner,
    );
    assert_eq!(
        registered(&app),
        (1, 1),
        "the app's animation is registered, and paused by the closed rule"
    );
    assert_eq!(idle_frames_requesting_redraw(&mut app, 4), 0, "so the app idles");

    opened.set(true);
    app.resolve_and_repaint(SETTLED.0, SETTLED.1);
    assert_eq!(registered(&app), (1, 0), "opening resumes it");
}

// ── The other overlays that close with `visibility: hidden` ─────────────────

/// A `Loader` in a **closed** `Popover`'s dropdown: the dropdown is
/// `visibility: hidden`, rendered, and — since #912 — paused. Opening the
/// popover resumes it.
#[test]
fn a_loader_in_a_closed_popover_is_paused_until_it_opens() {
    let opened = Signal::new(false);
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let loader = Loader::default().render(scope, &[]);
        let dropdown = PopoverDropdown.render(scope, &[loader]);
        let popover = Popover {
            opened_fn: Some(std::rc::Rc::new(move || opened.get())),
            ..Default::default()
        }
        .render(scope, &[dropdown]);
        root.append_child(&popover);
        root
    });
    app.mount_component(MOUNT.0, MOUNT.1);
    settle(&mut app);
    let oval = node_with_class(&app, OVAL);

    assert_eq!(animations(&app, oval), 1, "precondition: registered");
    assert!(is_paused(&app, oval), "a closed popover pauses its content");
    assert_eq!(idle_frames_requesting_redraw(&mut app, 4), 0, "and idles");

    opened.set(true);
    app.resolve_and_repaint(SETTLED.0, SETTLED.1);
    assert!(!is_paused(&app, oval), "opening resumes it");
    // The dropdown's 150ms opacity fade asks for frames of its own; the
    // spinner goes on asking after it has finished.
    std::thread::sleep(std::time::Duration::from_millis(200));
    idle_frames_requesting_redraw(&mut app, 2);
    assert_eq!(
        idle_frames_requesting_redraw(&mut app, 4),
        4,
        "and the clock runs while it is on screen"
    );
}

/// A `Loader` in a `HoverCard`'s dropdown: hidden until the card is hovered,
/// which is the card's resting state — so without the pause an app holding one
/// never slept. Hovering the card resumes it.
#[test]
fn a_loader_in_an_unhovered_hover_card_is_paused_until_hovered() {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let label = scope.create_text("hover me");
        let target = HoverCardTarget.render(scope, &[label]);
        target.set_attribute("style", "width: 100px; height: 40px");
        let loader = Loader::default().render(scope, &[]);
        let dropdown = HoverCardDropdown.render(scope, &[loader]);
        let card = HoverCard::default().render(scope, &[target, dropdown]);
        root.append_child(&card);
        root
    });
    app.mount_component(MOUNT.0, MOUNT.1);
    settle(&mut app);
    let oval = node_with_class(&app, OVAL);

    assert_eq!(animations(&app, oval), 1, "precondition: registered");
    assert!(is_paused(&app, oval), "an unhovered card pauses its content");
    assert_eq!(idle_frames_requesting_redraw(&mut app, 4), 0, "and idles");

    let target = node_with_class(&app, "rinch-hover-card__target");
    let (x, y) = window_origin(&app, target);
    let (x, y) = (x + 10.0, y + 10.0);
    app.handle_event(PlatformEvent::MouseMove { x, y }, PHYSICAL, 1.0);
    app.resolve_and_repaint(SETTLED.0, SETTLED.1);
    assert!(!is_paused(&app, oval), "hovering the card resumes it");
}
