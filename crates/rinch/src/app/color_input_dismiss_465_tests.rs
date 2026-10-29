//! `ColorInput`'s dropdown is dismissed by a click outside it and by Escape,
//! driven through the real event path (issue #465).
//!
//! **Why here and not in `rinch-components`.** That crate's harness mounts into
//! `MockDomDocument`, which has no CSS engine, no layout and no keyboard
//! dispatch: it can see that a backdrop element exists and nothing about which
//! of the backdrop, the panel and the field answers a tap. Every claim in this
//! file is about that ordering, so it has to be measured against the shipped
//! stylesheet with real boxes. `RinchApp::doc` is `pub(crate)`, so the fixture
//! lives inside this crate.
//!
//! The three boxes, and what decides between them:
//!
//! ```text
//!   z + 1   .rinch-color-input__input-group   (while open)
//!   z       .rinch-color-input__dropdown
//!   z - 1   .rinch-color-input__backdrop      position: fixed
//! ```
//!
//! with `z` = `--rinch-color-input-z-index`, 100 unless `ColorInput::z_index`
//! says otherwise (#801 — it was a literal 1000, which put the backdrop above
//! every `Modal`; the `#801` section at the end measures that).
//!
//! The field is *above* the backdrop deliberately, and that is the one place
//! this differs from `DropdownMenu`, whose trigger is a button: here the trigger
//! **is** the text input, and clicking into it to place a caret has to keep
//! working while the picker is open. `a_tap_on_the_text_field_while_open_still_focuses_it`
//! is that requirement, and it is the fixture a backdrop-over-everything
//! implementation fails.
//!
//! The backdrop is `position: fixed`, copied from `DropdownMenu` and `Popover`.
//! The long note above `.rinch-dropdown-menu__backdrop` explains why at length;
//! `a_tap_outside_the_clipping_shell_dismisses` is the measurement, and it sits
//! outside an `overflow: hidden` shell precisely because that is where the
//! `absolute` spelling is clipped away — its *box* reaches there either way.

use super::*;
use std::cell::{Cell, RefCell};

use rinch_components::Modal;
use rinch_components::color_input::ColorInput;
use rinch_core::{Callback, Component, InputCallback, Signal};

const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// A clipping shell narrower than the viewport — a sidebar, a table cell, a
/// form panel. "Outside the field" is somewhere a real click goes here, which
/// is exactly what an `absolute` backdrop would not reach.
///
/// Tall enough to hold the open picker (`0,40 → 218,440`, measured). At 300px
/// it clipped the swatch grid away and a tap aimed at a swatch reached nothing
/// — which looks exactly like the backdrop having swallowed it.
const SHELL_SMALL: &str = "position: relative; overflow: hidden; width: 400px; height: 500px";

/// A point inside the shell, outside the field and outside the open dropdown.
const IN_SHELL_OUTSIDE_FIELD: (f32, f32) = (300.0, 250.0);

/// A point outside that shell and inside the viewport.
const OUTSIDE_SHELL: (f32, f32) = (600.0, 250.0);

/// Mount `build` under the real theme **and** component stylesheets, so a
/// backdrop's `position`, `z-index` and `display` are the shipped ones.
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

fn tap(app: &mut RinchApp, x: f32, y: f32) {
    app.handle_event(
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
        (800, 600),
        1.0,
    );
    app.handle_event(
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        },
        (800, 600),
        1.0,
    );
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
}

/// One Escape keystroke: press and release, as a keyboard delivers it. Only the
/// press may dismiss.
fn escape(app: &mut RinchApp) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key: KeyCode::Escape,
            logical_key: None,
            text: None,
            modifiers: Modifiers::default(),
            // Escape never reaches the once-per-press activation arm, so the
            // fallback answer is the honest one here (issue #463).
            repeat: KeyRepeat::Unknown,
        },
        (800, 600),
        1.0,
    );
    app.handle_event(
        PlatformEvent::KeyUp {
            key: KeyCode::Escape,
            logical_key: None,
            modifiers: Modifiers::default(),
        },
        (800, 600),
        1.0,
    );
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
}

/// The one node carrying `class`, by name rather than by append order.
///
/// Iterated with `Slab::iter`, not `0..nodes.len()`: a slab's `len()` is its
/// *occupied* count, so a freed node anywhere makes an index range miss.
fn find_by_class(app: &RinchApp, class: &str) -> u64 {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .nodes
        .iter()
        .find(|(_, n)| {
            n.attributes
                .get("class")
                .is_some_and(|c| c.split_whitespace().any(|one| one == class))
        })
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no node carries the class {class}"))
}

fn has_class(app: &RinchApp, node_id: u64, class: &str) -> bool {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .get(node_id)
        .and_then(|n| n.attributes.get("class"))
        .is_some_and(|c| c.split_whitespace().any(|one| one == class))
}

fn box_of(app: &RinchApp, node_id: u64) -> (f32, f32, f32, f32) {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let (x, y, w, h) = painted_element_box(&d.tree, node_id);
    (x, y, x + w, y + h)
}

fn covers(b: (f32, f32, f32, f32), x: f32, y: f32) -> bool {
    x >= b.0 && x < b.2 && y >= b.1 && y < b.3
}

fn centre(app: &RinchApp, node_id: u64) -> (f32, f32) {
    let (x0, y0, x1, y1) = box_of(app, node_id);
    assert!(
        x1 > x0 && y1 > y0,
        "node {node_id} has no box to aim at: {:?}",
        (x0, y0, x1, y1)
    );
    ((x0 + x1) / 2.0, (y0 + y1) / 2.0)
}

struct Field {
    app: RinchApp,
    /// The clipping shell, so a test can prove a sample point is outside it
    /// rather than assert that from the style string.
    shell: u64,
    /// Every colour the input reported, so "the tap reached the picker" is an
    /// observation rather than an inference from "it stayed open".
    picks: Rc<RefCell<Vec<String>>>,
    /// Presses that reached the shell's own click handler — the page under
    /// the field. A backdrop swallows them; without one they arrive.
    shell_presses: Rc<Cell<usize>>,
}

impl Field {
    fn root(&self) -> u64 {
        find_by_class(&self.app, "rinch-color-input")
    }

    fn is_open(&self) -> bool {
        has_class(&self.app, self.root(), "rinch-color-input--opened")
    }

    /// Open the dropdown the way a user does: a tap on the preview swatch, in
    /// the input group but not on the text field, so the focus claim this makes
    /// is never mistaken for the one `a_tap_on_the_text_field_…` measures.
    fn open(&mut self) {
        let swatch = find_by_class(&self.app, "rinch-color-input__swatch-preview");
        let (x, y) = centre(&self.app, swatch);
        tap(&mut self.app, x, y);
        assert!(
            self.is_open(),
            "precondition: the field opened the dropdown"
        );
    }
}

/// Mount a `ColorInput` inside an `overflow: hidden` shell smaller than the
/// viewport, with one preset swatch in its picker.
fn mount_field(close_on_click_outside: bool) -> Field {
    let picks: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let shell_id: Rc<Cell<Option<u64>>> = Rc::new(Cell::new(None));

    let shell_presses: Rc<Cell<usize>> = Rc::new(Cell::new(0));

    let picks_in = picks.clone();
    let shell_in = shell_id.clone();
    let presses_in = shell_presses.clone();
    let app = mount(move |scope: &mut RenderScope| {
        let shell = scope.create_element("div");
        shell.set_attribute("style", SHELL_SMALL);
        shell_in.set(Some(shell.node_id().0));
        let id = scope.register_handler(move || presses_in.set(presses_in.get() + 1));
        shell.set_attribute("data-rid", &id.0.to_string());

        let field = ColorInput {
            value: "#000000".into(),
            swatches: vec!["#ff0000".into()],
            swatches_per_row: Some(1),
            close_on_click_outside,
            onchange: Some(InputCallback::new(move |v: String| {
                picks_in.borrow_mut().push(v)
            })),
            ..Default::default()
        }
        .render(scope, &[]);

        shell.append_child(&field);
        shell
    });

    Field {
        shell: shell_id.get().expect("the shell's node id"),
        app,
        picks,
        shell_presses,
    }
}

// ── outside clicks ───────────────────────────────────────────────────────────

/// The fault: with the picker open, a click anywhere else left it open.
#[test]
fn a_tap_elsewhere_on_the_page_dismisses() {
    let mut f = mount_field(true);
    f.open();

    // Beside the open dropdown, still inside the shell.
    let (x, y) = IN_SHELL_OUTSIDE_FIELD;
    let panel = box_of(&f.app, find_by_class(&f.app, "rinch-color-input__dropdown"));
    assert!(
        !covers(panel, x, y),
        "({x}, {y}) must miss the open panel {panel:?}, or this measures the \
         picker taking its own click"
    );
    tap(&mut f.app, x, y);

    assert!(!f.is_open(), "a tap outside the field must dismiss");
    assert_eq!(
        f.shell_presses.get(),
        0,
        "the backdrop takes the tap: the page under it does not see it"
    );
    assert!(
        f.picks.borrow().is_empty(),
        "and must not report a colour: {:?}",
        f.picks.borrow()
    );
}

/// **The `fixed` measurement.** A tap beyond the shell that clips the field
/// dismisses, because the backdrop is not clipped with it.
///
/// This is the fixture that fails against `position: absolute` — an absolute
/// box *is* clipped by an `overflow` ancestor in its containing-block chain, so
/// "outside the field" would shrink to "inside this 400x500 panel". Written back
/// over the shipped sheet, #317's exact spelling (`absolute` with ±100vh/±100vw
/// insets) leaves the backdrop's *box* covering `(-800, -600) → (1200, 636)` and
/// this tap still does not dismiss, which is the clip and nothing else.
///
/// The sample point is deliberately outside the shell: a point inside it is the
/// fixed point where both spellings agree — measured, the same mutant leaves
/// `a_tap_elsewhere_on_the_page_dismisses` green — so a fixture sitting there
/// says nothing about `fixed`.
#[test]
fn a_tap_outside_the_clipping_shell_dismisses() {
    let mut f = mount_field(true);
    f.open();

    let (x, y) = OUTSIDE_SHELL;
    let shell = box_of(&f.app, f.shell);
    assert!(
        !covers(shell, x, y),
        "({x}, {y}) must be outside the clipping shell {shell:?}, or an \
         absolute backdrop reaches it too and the fixture proves nothing"
    );
    let backdrop = box_of(&f.app, find_by_class(&f.app, "rinch-color-input__backdrop"));
    assert!(
        covers(backdrop, x, y),
        "({x}, {y}) must be inside the backdrop's own box {backdrop:?} — what \
         the tap turns on is the clip chain and the paint order, not the box"
    );

    tap(&mut f.app, x, y);
    assert!(!f.is_open());
}

/// The off switch, measured through the same path: with no backdrop the tap
/// reaches the page under it. Without this the suite passes against a fix that
/// ignores the prop and mounts the backdrop unconditionally.
///
/// The dropdown still closes, and that is #1572, not the backdrop: opening put
/// the keyboard on the text field, the tap on the page releases it, and focus
/// leaving the field closes the dropdown, as Mantine's `onBlur` does whatever
/// its popover's `closeOnClickOutside` says. A press that takes no focus (a
/// `data-nofocus` toolbar) leaves it open.
#[test]
fn close_on_click_outside_false_lets_an_outside_tap_through() {
    let mut f = mount_field(false);
    f.open();

    let (x, y) = IN_SHELL_OUTSIDE_FIELD;
    tap(&mut f.app, x, y);

    assert_eq!(
        f.shell_presses.get(),
        1,
        "close_on_click_outside: false mounts no backdrop, so the page gets the tap"
    );
    assert!(
        !f.is_open(),
        "and the tap moved the focus out of the field, which closes it (#1572)"
    );
}

// ── clicks that must NOT dismiss ─────────────────────────────────────────────

/// A tap on a swatch inside the dropdown picks that colour and leaves the
/// picker open.
///
/// The backdrop sits one z-level under the panel for exactly this. Both halves
/// are asserted: "still open" alone would pass against a backdrop that took the
/// tap and a dismissal that never ran.
#[test]
fn a_tap_on_a_swatch_inside_the_dropdown_picks_and_does_not_dismiss() {
    let mut f = mount_field(true);
    f.open();

    let swatch = {
        let grid = find_by_class(&f.app, "rinch-color-picker__swatches");
        let doc = f.app.doc.as_ref().unwrap();
        let d = doc.borrow();
        *d.tree
            .get(grid)
            .expect("the swatch grid is in the tree")
            .children
            .first()
            .expect("one preset swatch")
    };
    let (x, y) = centre(&f.app, swatch);

    tap(&mut f.app, x, y);

    assert_eq!(
        f.picks.borrow().len(),
        1,
        "the tap must reach the picker's swatch, not the backdrop: {:?}",
        f.picks.borrow()
    );
    assert!(
        f.is_open(),
        "and picking a colour must not dismiss the picker"
    );
}

/// **The field stays clickable while the picker is open.**
///
/// This is where `ColorInput` differs from `DropdownMenu`: the trigger *is* a
/// text input, so a click into it has to place a caret rather than be swallowed
/// by the dismissal. The claim is the focus arbiter's, not the class's — a
/// backdrop above the field would still close the dropdown (that is what a
/// backdrop does) and would leave the keyboard unclaimed, so `is_open` alone
/// cannot tell the two apart.
#[test]
fn a_tap_on_the_text_field_while_open_still_focuses_it() {
    let mut f = mount_field(true);
    f.open();

    let input = find_by_class(&f.app, "rinch-color-input__input");
    let (x, y) = centre(&f.app, input);
    tap(&mut f.app, x, y);

    assert_eq!(
        f.app.focus_target,
        FocusTarget::Input(input),
        "a tap on the text field must claim the keyboard for it, even with the \
         picker open — otherwise the caret cannot be placed"
    );
    assert!(
        !f.is_open(),
        "and the field's own handler still toggles the dropdown shut"
    );
}

// ── Escape ───────────────────────────────────────────────────────────────────

/// Escape closes the picker.
#[test]
fn escape_closes_the_dropdown() {
    let mut f = mount_field(true);
    f.open();

    escape(&mut f.app);

    assert!(!f.is_open(), "Escape must close the dropdown");
}

/// **Escape reaches the picker before the modal behind it, and only then the
/// modal.**
///
/// The dismiss stack is LIFO and the `ColorInput` inside the modal registered
/// last, so the first Escape is the picker's. The second closes the modal —
/// which is the half that says the input *stopped* consuming the key once its
/// dropdown was shut, rather than swallowing Escape for the rest of the
/// session.
#[test]
fn escape_closes_the_dropdown_before_the_modal_behind_it() {
    let modal_closes: Rc<Cell<usize>> = Rc::new(Cell::new(0));
    let picks: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));

    let closes_in = modal_closes.clone();
    let picks_in = picks.clone();
    let mut app = mount(move |scope: &mut RenderScope| {
        let open = Signal::new(true);
        let field = ColorInput {
            value: "#000000".into(),
            onchange: Some(InputCallback::new(move |v: String| {
                picks_in.borrow_mut().push(v)
            })),
            ..Default::default()
        }
        .render(scope, &[]);

        Modal {
            opened_fn: Some(Rc::new(move || open.get())),
            onclose: Some(Callback::new(move || closes_in.set(closes_in.get() + 1))),
            ..Default::default()
        }
        .render(scope, &[field])
    });

    // Open the picker.
    let swatch = find_by_class(&app, "rinch-color-input__swatch-preview");
    let (x, y) = centre(&app, swatch);
    tap(&mut app, x, y);
    let root = find_by_class(&app, "rinch-color-input");
    assert!(
        has_class(&app, root, "rinch-color-input--opened"),
        "precondition: the picker is open"
    );

    escape(&mut app);
    assert!(
        !has_class(&app, root, "rinch-color-input--opened"),
        "the first Escape closes the picker"
    );
    assert_eq!(
        modal_closes.get(),
        0,
        "and must not reach the modal behind it"
    );

    escape(&mut app);
    assert_eq!(
        modal_closes.get(),
        1,
        "the second Escape falls through to the modal — a closed picker does \
         not go on swallowing the key"
    );
}

// ── an open picker that is no longer rendered (issue #802) ───────────────────

/// A dismiss entry pushed under everything the fixture opens later, counting
/// the Escapes that reach it — "the key fell through" as an observation.
fn count_escapes_behind(app: &RinchApp) -> (Rc<Cell<usize>>, rinch_core::DismissHandle) {
    let reached: Rc<Cell<usize>> = Rc::new(Cell::new(0));
    let r = reached.clone();
    let key = app.doc.as_ref().unwrap().borrow().doc_key();
    let handle = rinch_core::push_dismiss_handler(key, move || {
        r.set(r.get() + 1);
        true
    });
    (reached, handle)
}

/// **An ancestor that stops rendering the open picker takes its Escape with
/// it.** `Modal`'s closed state is `display: none` and its subtree stays
/// mounted, so a picker that was open when the app closed the modal keeps its
/// open state and its entry. That entry used to consume the next Escape,
/// invisibly (#802); it declines now, so the key reaches what is behind.
#[test]
fn escape_passes_a_picker_its_closed_modal_no_longer_renders() {
    let modal_open: Rc<Cell<Option<Signal<bool>>>> = Rc::new(Cell::new(None));
    let open_in = modal_open.clone();
    let mut app = mount(move |scope: &mut RenderScope| {
        let open = Signal::new(true);
        open_in.set(Some(open));
        let field = ColorInput {
            value: "#000000".into(),
            ..Default::default()
        }
        .render(scope, &[]);
        Modal {
            opened_fn: Some(Rc::new(move || open.get())),
            onclose: Some(Callback::new(move || open.set(false))),
            ..Default::default()
        }
        .render(scope, &[field])
    });
    let (behind, _keep) = count_escapes_behind(&app);

    let swatch = find_by_class(&app, "rinch-color-input__swatch-preview");
    let (x, y) = centre(&app, swatch);
    tap(&mut app, x, y);
    let root = find_by_class(&app, "rinch-color-input");
    assert!(
        has_class(&app, root, "rinch-color-input--opened"),
        "precondition: the picker is open"
    );

    // The app closes the modal on its own — a "cancel", a network event — and
    // the frame is laid out before the next key arrives.
    modal_open.get().unwrap().set(false);
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    assert!(
        has_class(&app, root, "rinch-color-input--opened"),
        "precondition: closing the modal leaves the picker's own state open"
    );

    escape(&mut app);
    assert_eq!(
        behind.get(),
        1,
        "the Escape must reach what is behind the hidden picker, not be \
         swallowed by it"
    );
}

/// The same for `visibility: hidden` — the closed state of `Drawer` and
/// `Popover`, which keeps the box but shows nothing. Set directly on a wrapper
/// so the fixture does not depend on a 300ms transition.
#[test]
fn escape_passes_a_picker_under_a_visibility_hidden_ancestor() {
    let wrapper_id: Rc<Cell<Option<u64>>> = Rc::new(Cell::new(None));
    let wrapper_in = wrapper_id.clone();
    let mut app = mount(move |scope: &mut RenderScope| {
        let wrapper = scope.create_element("div");
        wrapper_in.set(Some(wrapper.node_id().0));
        let field = ColorInput {
            value: "#000000".into(),
            ..Default::default()
        }
        .render(scope, &[]);
        wrapper.append_child(&field);
        wrapper
    });
    let (behind, _keep) = count_escapes_behind(&app);

    let swatch = find_by_class(&app, "rinch-color-input__swatch-preview");
    let (x, y) = centre(&app, swatch);
    tap(&mut app, x, y);

    {
        let doc = app.doc.as_ref().unwrap();
        doc.borrow_mut().set_style(
            rinch_core::dom::NodeId(wrapper_id.get().unwrap()),
            "visibility",
            "hidden",
        );
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);

    escape(&mut app);
    assert_eq!(behind.get(), 1, "a hidden picker must not answer Escape");
}

/// The control: the same picker, rendered, still takes its Escape first. Without
/// it the two fixtures above pass against an entry that never consumes.
#[test]
fn escape_is_still_the_rendered_pickers() {
    let mut f = mount_field(true);
    let (behind, _keep) = count_escapes_behind(&f.app);
    f.open();
    escape(&mut f.app);
    assert!(!f.is_open(), "the picker answers the first Escape");
    assert_eq!(behind.get(), 0, "and the key goes no further");
}

// ── the backdrop's level against the library's other overlays (#801) ────────

/// What `mount_with_modal` hands a test.
struct WithModal {
    app: RinchApp,
    /// The modal's `opened_fn` source, so the modal can appear without a
    /// click — a background error dialog, a shortcut, a network event. A click
    /// that opened it would be eaten by the backdrop first and close the picker.
    show_modal: Signal<bool>,
    /// Presses of the button inside the modal's body.
    presses: Rc<Cell<usize>>,
}

/// A `ColorInput` beside (not inside) a `Modal` that starts closed, with one
/// button in the modal's body. `color_input_z` is the input's `z_index` prop.
fn mount_with_modal(color_input_z: Option<i32>) -> WithModal {
    let presses: Rc<Cell<usize>> = Rc::new(Cell::new(0));
    let show: Rc<Cell<Option<Signal<bool>>>> = Rc::new(Cell::new(None));

    let presses_in = presses.clone();
    let show_in = show.clone();
    let app = mount(move |scope: &mut RenderScope| {
        let open = Signal::new(false);
        show_in.set(Some(open));

        let page = scope.create_element("div");
        let field = ColorInput {
            value: "#000000".into(),
            z_index: color_input_z,
            ..Default::default()
        }
        .render(scope, &[]);
        page.append_child(&field);

        let button = scope.create_element("button");
        button.set_attribute("class", "probe-801-button");
        button.set_attribute("style", "width: 120px; height: 30px");
        let id = scope.register_handler(move || presses_in.set(presses_in.get() + 1));
        button.set_attribute("data-rid", &id.0.to_string());

        let modal = Modal {
            title: "Dialog".into(),
            opened_fn: Some(Rc::new(move || open.get())),
            ..Default::default()
        }
        .render(scope, &[button]);
        page.append_child(&modal);
        page
    });

    WithModal {
        app,
        show_modal: show.get().expect("the modal's signal"),
        presses,
    }
}

impl WithModal {
    fn picker_open(&self) -> bool {
        let root = find_by_class(&self.app, "rinch-color-input");
        has_class(&self.app, root, "rinch-color-input--opened")
    }

    fn open_picker(&mut self) {
        let swatch = find_by_class(&self.app, "rinch-color-input__swatch-preview");
        let (x, y) = centre(&self.app, swatch);
        tap(&mut self.app, x, y);
        assert!(self.picker_open(), "precondition: the picker is open");
    }

    /// The modal appears with no click, while the picker is still open.
    fn show_modal(&mut self) {
        self.show_modal.set(true);
        self.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
        let modal = find_by_class(&self.app, "rinch-modal");
        let b = box_of(&self.app, modal);
        assert!(
            b.2 > b.0 && b.3 > b.1,
            "precondition: the modal is laid out, got {b:?}"
        );
    }

    /// Whether the topmost node at `(x, y)` is `class` or inside it.
    fn hit_is_inside(&self, x: f32, y: f32, class: &str) -> bool {
        let doc = self.app.doc.as_ref().unwrap();
        let d = doc.borrow();
        let mut at = hit_testing::hit_test(&d.tree, x, y);
        while let Some(id) = at {
            let node = d.tree.get(id).expect("a hit node is in the tree");
            if node
                .attributes
                .get("class")
                .is_some_and(|c| c.split_whitespace().any(|one| one == class))
            {
                return true;
            }
            at = node.parent;
        }
        false
    }
}

/// **The fault (#801):** a `Modal` that appears over an open picker is
/// click-blocked — the picker's backdrop, at `z-index: 999`, sat above the
/// modal's 200/201 and took the first press anywhere on the dialog.
///
/// With the backdrop at the library's level (`var(--rinch-color-input-z-index,
/// 100) - 1`, the shape `DropdownMenu` and `Popover` use) the press reaches
/// the dialog's own button. Since #1572 that press also closes the picker,
/// because it moves the focus out of the field (Mantine's `onBlur`); the
/// picker is still open while the modal is merely shown over it.
#[test]
fn a_modal_that_opens_over_an_open_picker_takes_its_own_press() {
    let mut w = mount_with_modal(None);
    w.open_picker();
    w.show_modal();

    let title = find_by_class(&w.app, "rinch-modal__title");
    let (tx, ty) = centre(&w.app, title);
    assert!(
        w.hit_is_inside(tx, ty, "rinch-modal__title"),
        "the modal's title must be the topmost box at its own centre, not the \
         picker's backdrop"
    );

    // The panel's level too, not only the backdrop's: where the open picker
    // and the modal overlap, the modal is on top. At the old literal 1000 the
    // picker panel painted over the dialog and took presses there.
    let panel = box_of(&w.app, find_by_class(&w.app, "rinch-color-input__dropdown"));
    let modal = box_of(&w.app, find_by_class(&w.app, "rinch-modal"));
    let overlap = (
        panel.0.max(modal.0),
        panel.1.max(modal.1),
        panel.2.min(modal.2),
        panel.3.min(modal.3),
    );
    assert!(
        overlap.2 > overlap.0 && overlap.3 > overlap.1,
        "precondition: the open picker {panel:?} and the modal {modal:?} overlap"
    );
    let (ox, oy) = ((overlap.0 + overlap.2) / 2.0, (overlap.1 + overlap.3) / 2.0);
    assert!(
        w.hit_is_inside(ox, oy, "rinch-modal"),
        "at ({ox}, {oy}), inside both, the modal must be the topmost box"
    );

    let button = find_by_class(&w.app, "probe-801-button");
    let (bx, by) = centre(&w.app, button);
    tap(&mut w.app, bx, by);
    assert_eq!(
        w.presses.get(),
        1,
        "the first press on the modal's button must run it — the backdrop \
         must not swallow it"
    );
    // The press focused the modal's button, so the focus left the field and
    // the picker closed (#1572) — a focus leave, not the backdrop: the press
    // count above is what says the backdrop did not take it.
    assert!(
        !w.picker_open(),
        "the press moved the focus out of the field, which closes the picker"
    );
}

/// The picker's own outside-press dismissal is untouched by the lower level:
/// with no other overlay up, a press elsewhere on the page still closes it.
/// (Off the shell's fixed point: this page has no clipping shell at all, and
/// the sample point is the far corner, nowhere near the field.)
#[test]
fn with_the_modal_closed_an_outside_press_still_dismisses_the_picker() {
    let mut w = mount_with_modal(None);
    w.open_picker();

    tap(&mut w.app, 700.0, 500.0);
    assert!(
        !w.picker_open(),
        "a press elsewhere must dismiss the picker"
    );
}

/// **`z_index` moves all three levels together.** Raised past the modal's
/// 200/201, the picker's backdrop outranks the modal again — the old order,
/// now an explicit choice — and the press on the dialog is the backdrop's: it
/// dismisses the picker and does not run the modal's button.
///
/// This is the fixture that kills a prop that is read and never published, or
/// a backdrop rule that keeps a literal level instead of deriving from the
/// custom property.
#[test]
fn a_raised_z_index_puts_the_picker_back_above_the_modal() {
    let mut w = mount_with_modal(Some(500));
    w.open_picker();
    w.show_modal();

    let button = find_by_class(&w.app, "probe-801-button");
    let (bx, by) = centre(&w.app, button);
    assert!(
        w.hit_is_inside(bx, by, "rinch-color-input__backdrop"),
        "at z_index 500 the backdrop (499) is above the modal (200/201)"
    );
    tap(&mut w.app, bx, by);
    assert_eq!(w.presses.get(), 0, "the backdrop took the press");
    assert!(!w.picker_open(), "and dismissed the picker");
}

/// **Inside a `Modal` the three levels still order** — field over panel over
/// backdrop — because the modal panel is a stacking context and they are
/// compared inside it (#545). A swatch tap picks, a tap elsewhere in the
/// dialog dismisses.
#[test]
fn inside_a_modal_the_picker_still_orders_above_its_backdrop() {
    let picks: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let picks_in = picks.clone();
    let mut app = mount(move |scope: &mut RenderScope| {
        let open = Signal::new(true);
        let field = ColorInput {
            value: "#000000".into(),
            swatches: vec!["#ff0000".into()],
            swatches_per_row: Some(1),
            onchange: Some(InputCallback::new(move |v: String| {
                picks_in.borrow_mut().push(v)
            })),
            ..Default::default()
        }
        .render(scope, &[]);
        // Tall enough to hold the open picker (`0,40 → 218,440` under the
        // field, as in the shell above): the modal is `overflow-y: auto`, and
        // a picker that hangs past its bottom is clipped there — in a browser
        // too — while the `fixed` backdrop is not, so a swatch below the
        // modal's edge would be answered by the backdrop whatever the levels.
        let content = scope.create_element("div");
        content.set_attribute("style", "height: 440px");
        content.append_child(&field);
        Modal {
            title: "Dialog".into(),
            opened_fn: Some(Rc::new(move || open.get())),
            ..Default::default()
        }
        .render(scope, &[content])
    });

    let preview = find_by_class(&app, "rinch-color-input__swatch-preview");
    let (x, y) = centre(&app, preview);
    tap(&mut app, x, y);
    let root = find_by_class(&app, "rinch-color-input");
    assert!(has_class(&app, root, "rinch-color-input--opened"));

    let swatch = {
        let grid = find_by_class(&app, "rinch-color-picker__swatches");
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        *d.tree.get(grid).unwrap().children.first().unwrap()
    };
    let (sx, sy) = centre(&app, swatch);
    tap(&mut app, sx, sy);
    assert_eq!(picks.borrow().len(), 1, "the swatch took the tap");
    assert!(has_class(&app, root, "rinch-color-input--opened"));

    let title = find_by_class(&app, "rinch-modal__title");
    let (tx, ty) = centre(&app, title);
    tap(&mut app, tx, ty);
    assert!(
        !has_class(&app, root, "rinch-color-input--opened"),
        "a tap on the dialog's title, outside the picker, dismisses it"
    );
}

// ── #1572: focus leaving the field, and the window blurring ────────────────────
//
// Mantine closes the dropdown in the input's `onBlur`, which a window blur also
// fires. Each fixture below starts with the keyboard on the text field (opened
// by a tap on the field itself, which is what a user does to type), so "focus
// left" is a real transition out of the `ColorInput`, not a claim that was never
// made.

/// A `ColorInput` between two buttons, so Tab has somewhere to go on either
/// side, and a programmatic focus has a target outside the field.
struct Flanked {
    field: Field,
    after: NodeHandle,
}

fn mount_flanked() -> Flanked {
    let picks: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let shell_id: Rc<Cell<Option<u64>>> = Rc::new(Cell::new(None));
    let after_slot: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));

    let picks_in = picks.clone();
    let shell_in = shell_id.clone();
    let after_in = after_slot.clone();
    let app = mount(move |scope: &mut RenderScope| {
        let shell = scope.create_element("div");
        shell.set_attribute("style", SHELL_SMALL);
        shell_in.set(Some(shell.node_id().0));

        let before = scope.create_element("button");
        before.set_attribute("class", "before-1572");
        before.append_child(&scope.create_text("before"));
        shell.append_child(&before);

        let field = ColorInput {
            value: "#000000".into(),
            swatches: vec!["#ff0000".into()],
            swatches_per_row: Some(1),
            onchange: Some(InputCallback::new(move |v: String| {
                picks_in.borrow_mut().push(v)
            })),
            ..Default::default()
        }
        .render(scope, &[]);
        shell.append_child(&field);

        let after = scope.create_element("button");
        after.set_attribute("class", "after-1572");
        after.append_child(&scope.create_text("after"));
        shell.append_child(&after);
        *after_in.borrow_mut() = Some(after);
        shell
    });

    let after = after_slot.borrow().clone().expect("the after button");
    Flanked {
        field: Field {
            shell: shell_id.get().expect("the shell's node id"),
            app,
            picks,
            shell_presses: Rc::new(Cell::new(0)),
        },
        after,
    }
}

impl Flanked {
    fn input(&self) -> u64 {
        find_by_class(&self.field.app, "rinch-color-input__input")
    }

    /// Open the dropdown by a tap on the text field, which also gives it the
    /// keyboard.
    fn open_from_the_field(&mut self) {
        let input = self.input();
        let (x, y) = centre(&self.field.app, input);
        tap(&mut self.field.app, x, y);
        assert!(self.field.is_open(), "precondition: the tap opened it");
        assert_eq!(
            self.field.app.focus_target,
            FocusTarget::Input(input),
            "precondition: the field holds the keyboard"
        );
    }

    fn key(&mut self, key: KeyCode) {
        self.field.app.handle_event(
            PlatformEvent::KeyDown {
                key,
                logical_key: None,
                text: None,
                modifiers: Modifiers::default(),
                repeat: KeyRepeat::Unknown,
            },
            (800, 600),
            1.0,
        );
        self.field.app.handle_event(
            PlatformEvent::KeyUp {
                key,
                logical_key: None,
                modifiers: Modifiers::default(),
            },
            (800, 600),
            1.0,
        );
        self.field.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    }

    fn window_focus(&mut self, focused: bool) {
        self.field
            .app
            .handle_event(PlatformEvent::WindowFocus(focused), (800, 600), 1.0);
        self.field.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    }

    fn after_id(&self) -> u64 {
        self.after.node_id().0
    }
}

/// Tab from the field to the button after it closes the dropdown.
#[test]
fn tab_away_from_the_field_closes_the_dropdown_1572() {
    let mut f = mount_flanked();
    f.open_from_the_field();

    f.key(KeyCode::Tab);

    assert_eq!(
        f.field.app.focus_target,
        FocusTarget::Node(f.after_id()),
        "precondition: Tab moved the keyboard to the next button"
    );
    assert!(
        !f.field.is_open(),
        "focus leaving the field closes the dropdown"
    );
    assert!(f.field.picks.borrow().is_empty(), "and reports no colour");
}

/// A programmatic focus elsewhere closes it too.
#[test]
fn a_programmatic_focus_elsewhere_closes_the_dropdown_1572() {
    let mut f = mount_flanked();
    f.open_from_the_field();

    f.after.focus();
    f.field.app.drain_focus_requests(true);
    f.field.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);

    assert_eq!(
        f.field.app.focus_target,
        FocusTarget::Node(f.after_id()),
        "precondition: the request moved the keyboard"
    );
    assert!(
        !f.field.is_open(),
        "focus leaving the field closes the dropdown"
    );
}

/// The window losing the keyboard closes it, with the field keeping its claim
/// (notify-and-retain, #147/#226); the window coming back reopens nothing.
#[test]
fn a_window_blur_closes_the_dropdown_1572() {
    let mut f = mount_flanked();
    f.open_from_the_field();
    let input = f.input();

    f.window_focus(false);
    assert!(!f.field.is_open(), "a window blur closes the dropdown");
    assert_eq!(
        f.field.app.focus_target,
        FocusTarget::Input(input),
        "and the field keeps its claim"
    );

    f.window_focus(true);
    assert!(!f.field.is_open(), "coming back does not reopen it");
}

/// Picking from inside the dropdown keeps it open and keeps the keyboard on the
/// field: a press in the panel takes the click but not the focus (Mantine's
/// `preventDefault` on the dropdown's mousedown), so nothing left the field.
#[test]
fn a_swatch_tap_keeps_the_dropdown_open_and_the_field_focused_1572() {
    let mut f = mount_flanked();
    f.open_from_the_field();
    let input = f.input();

    let swatch = {
        let grid = find_by_class(&f.field.app, "rinch-color-picker__swatches");
        let doc = f.field.app.doc.as_ref().unwrap();
        let d = doc.borrow();
        *d.tree.get(grid).unwrap().children.first().unwrap()
    };
    let (x, y) = centre(&f.field.app, swatch);
    tap(&mut f.field.app, x, y);

    assert_eq!(
        f.field.picks.borrow().len(),
        1,
        "the tap reached the swatch"
    );
    assert!(f.field.is_open(), "and the dropdown stays open");
    assert_eq!(
        f.field.app.focus_target,
        FocusTarget::Input(input),
        "with the keyboard still on the field"
    );

    // A press on the picker's saturation panel likewise.
    let panel = find_by_class(&f.field.app, "rinch-color-picker__saturation");
    let (x, y) = centre(&f.field.app, panel);
    tap(&mut f.field.app, x, y);
    assert!(f.field.is_open(), "a press on the panel keeps it open");
    assert_eq!(f.field.app.focus_target, FocusTarget::Input(input));

    // Escape still closes it.
    escape(&mut f.field.app);
    assert!(!f.field.is_open(), "Escape still closes the dropdown");
}

/// A tap on the preview swatch while open still toggles it shut, and does not
/// move the keyboard off the field (which would close it, then the toggle would
/// reopen it).
#[test]
fn a_preview_tap_while_open_closes_the_dropdown_1572() {
    let mut f = mount_flanked();
    f.open_from_the_field();
    let input = f.input();

    let preview = find_by_class(&f.field.app, "rinch-color-input__swatch-preview");
    let (x, y) = centre(&f.field.app, preview);
    tap(&mut f.field.app, x, y);

    assert!(
        !f.field.is_open(),
        "the preview's tap toggles the dropdown shut"
    );
    assert_eq!(f.field.app.focus_target, FocusTarget::Input(input));
}

/// Opening from the preview swatch puts the keyboard on the field, as a tap on
/// a Mantine input's section does; so Tab from there leaves the field and
/// closes the dropdown rather than landing on the field itself.
#[test]
fn opening_from_the_preview_focuses_the_field_1572() {
    let mut f = mount_flanked();
    let before = find_by_class(&f.field.app, "before-1572");
    let (x, y) = centre(&f.field.app, before);
    tap(&mut f.field.app, x, y);
    assert_eq!(
        f.field.app.focus_target,
        FocusTarget::Node(before),
        "precondition: the button before the field holds the keyboard"
    );

    f.field.open();
    f.field.app.drain_focus_requests(true);
    f.field.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let input = f.input();
    assert_eq!(
        f.field.app.focus_target,
        FocusTarget::Input(input),
        "opening moves the keyboard to the field"
    );
    assert!(f.field.is_open(), "and the dropdown is open");

    f.key(KeyCode::Tab);
    assert_eq!(f.field.app.focus_target, FocusTarget::Node(f.after_id()));
    assert!(!f.field.is_open(), "Tab away closes it");
}

/// A focus move from the field into the panel is not a leave: the entry is
/// owned by the `ColorInput`'s root, which holds both. The picker ships no
/// focusable control (its own hex input is off inside a `ColorInput`), so one
/// is made here — a swatch given a `tabindex` — and focused by code. An entry
/// owned by the text field alone would close the dropdown on that move.
#[test]
fn a_focus_move_into_the_panel_keeps_the_dropdown_open_1572() {
    let mut f = mount_flanked();
    f.open_from_the_field();

    let swatch = {
        let grid = find_by_class(&f.field.app, "rinch-color-picker__swatches");
        let doc = f.field.app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        let id = *d.tree.get(grid).unwrap().children.first().unwrap();
        rinch_core::dom::DomDocument::set_attribute(
            &mut *d,
            rinch_core::dom::NodeId(id),
            "tabindex",
            "0",
        );
        id
    };
    f.field
        .app
        .resolve_and_repaint(VIEWPORT.0 + 1.0, VIEWPORT.1);
    {
        let doc = f.field.app.doc.as_ref().unwrap();
        rinch_core::dom::DomDocument::focus_element(
            &mut *doc.borrow_mut(),
            rinch_core::dom::NodeId(swatch),
        );
    }
    f.field.app.drain_focus_requests(true);
    f.field.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);

    assert_eq!(
        f.field.app.focus_target,
        FocusTarget::Node(swatch),
        "precondition: the swatch in the panel holds the keyboard"
    );
    assert!(
        f.field.is_open(),
        "a focus move inside the ColorInput keeps it open"
    );

    // And leaving from there still closes it.
    f.key(KeyCode::Tab);
    assert_eq!(f.field.app.focus_target, FocusTarget::Node(f.after_id()));
    assert!(!f.field.is_open(), "Tab out of the panel closes it");
}
