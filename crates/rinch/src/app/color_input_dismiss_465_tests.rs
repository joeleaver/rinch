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
//!   z 1001   .rinch-color-input__input-group   (while open)
//!   z 1000   .rinch-color-input__dropdown
//!   z  999   .rinch-color-input__backdrop      position: fixed
//! ```
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
//! `absolute` spelling stops covering.

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
fn find_by_class(app: &RinchApp, class: &str) -> usize {
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

fn has_class(app: &RinchApp, node_id: usize, class: &str) -> bool {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .get(node_id)
        .and_then(|n| n.attributes.get("class"))
        .is_some_and(|c| c.split_whitespace().any(|one| one == class))
}

fn box_of(app: &RinchApp, node_id: usize) -> (f32, f32, f32, f32) {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let (x, y, w, h) = painted_element_box(&d.tree, node_id);
    (x, y, x + w, y + h)
}

fn covers(b: (f32, f32, f32, f32), x: f32, y: f32) -> bool {
    x >= b.0 && x < b.2 && y >= b.1 && y < b.3
}

fn centre(app: &RinchApp, node_id: usize) -> (f32, f32) {
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
    shell: usize,
    /// Every colour the input reported, so "the tap reached the picker" is an
    /// observation rather than an inference from "it stayed open".
    picks: Rc<RefCell<Vec<String>>>,
}

impl Field {
    fn root(&self) -> usize {
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
    let shell_id: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));

    let picks_in = picks.clone();
    let shell_in = shell_id.clone();
    let app = mount(move |scope: &mut RenderScope| {
        let shell = scope.create_element("div");
        shell.set_attribute("style", SHELL_SMALL);
        shell_in.set(Some(shell.node_id().0));

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
/// "outside the field" would shrink to "inside this 400x300 panel". The sample
/// point is deliberately outside the shell: a point inside it is the fixed point
/// where both spellings agree, and `a_tap_elsewhere_on_the_page_dismisses`
/// already covers that.
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

/// The off switch, measured through the same path. Without it the suite passes
/// against a fix that ignores the prop and dismisses unconditionally, which is a
/// different bug with the same headline.
#[test]
fn close_on_click_outside_false_leaves_an_outside_tap_inert() {
    let mut f = mount_field(false);
    f.open();

    let (x, y) = IN_SHELL_OUTSIDE_FIELD;
    tap(&mut f.app, x, y);

    assert!(
        f.is_open(),
        "close_on_click_outside: false must leave the dropdown open"
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
