//! Desktop's half of #2993: Enter, Shift+Enter and Mod+Enter in a code block,
//! pressed as keys through the app's key dispatch (the editor's keymap path).
//! Enter and Shift+Enter type a `"\n"`, the block grows a line and the caret
//! is drawn at the new line's start; Mod+Enter leaves the block for a
//! paragraph after it. Before the fix Enter split the block in two and
//! Shift+Enter did nothing.

use super::*;
use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::{Pos, Selection};

const VP: (u32, u32) = (800, 600);
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

fn idle(app: &mut RinchApp) {
    for _ in 0..3 {
        let actions = app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
        if actions.contains(&AppAction::RequestRedraw) {
            if app.has_pending_layout() {
                app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
            }
            let _ = app.build_pixels(1.0, VP, false);
        }
    }
}

struct Page {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
}

/// `html` in a 400px editor, Inter 16px / 24px, focused and settled.
fn page(html: &str) -> Page {
    let handle = crate::editor::create_editor();
    assert!(handle.load_html(html));
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let editor = handle_in.mount(scope);
        editor.set_attribute(
            "style",
            "width: 400px; height: 400px; font-size: 16px; line-height: 24px; \
             font-family: sans-serif",
        );
        root.append_child(&editor);
        root
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let mut page = Page { app, handle };
    page.handle.focus();
    idle(&mut page.app);
    page
}

impl Page {
    /// Press and release Enter with `shift` and the platform's primary
    /// modifier (`primary`: Ctrl, Cmd on macOS) held.
    fn enter(&mut self, shift: bool, primary: bool) {
        let mac = cfg!(target_os = "macos");
        let modifiers = Modifiers {
            shift,
            ctrl: primary && !mac,
            meta: primary && mac,
            ..Default::default()
        };
        let key = KeyCode::Enter;
        self.app.handle_event(
            PlatformEvent::KeyDown {
                key,
                logical_key: None,
                text: None,
                modifiers,
                repeat: KeyRepeat::Fresh,
            },
            VP,
            1.0,
        );
        self.app.handle_event(
            PlatformEvent::KeyUp {
                key,
                logical_key: None,
                modifiers,
            },
            VP,
            1.0,
        );
        idle(&mut self.app);
    }

    fn html(&self) -> String {
        node_to_html(&self.handle.doc())
    }

    /// The layout-local caret at document position `pos`, through the model's
    /// caret address (what the caret overlay is placed from).
    fn caret(&self, pos: usize) -> (f32, f32) {
        let (block, byte) = self.handle.caret_address(Pos(pos)).unwrap();
        let doc = self.app.doc.as_ref().unwrap().borrow();
        doc.query_caret_position(block, byte).expect("laid out")
    }
}

#[test]
fn enter_and_shift_enter_in_a_code_block_start_a_new_line_in_it() {
    for shift in [false, true] {
        let mut p = page("<pre>abcd</pre>");
        p.handle.set_selection(Selection::cursor(Pos(3)));
        let line_start = p.caret(1);
        let before_break = p.caret(3);
        assert_eq!(
            before_break.1, line_start.1,
            "positive control: `ab|cd` is on the first line"
        );
        p.enter(shift, false);
        assert_eq!(p.html(), "<pre>ab\ncd</pre>", "shift {shift}");
        assert_eq!(
            p.handle.selection(),
            Selection::cursor(Pos(4)),
            "shift {shift}"
        );
        // The caret after the newline is at the start of the second line.
        let (x, y) = p.caret(4);
        assert_eq!(x, line_start.0, "shift {shift}: at the line's start");
        assert!(y > line_start.1, "shift {shift}: on a line below, {y}");
    }
}

#[test]
fn mod_enter_in_a_code_block_leaves_it_for_a_paragraph() {
    let mut p = page("<pre>abcd</pre><p>x</p>");
    p.handle.set_selection(Selection::cursor(Pos(3)));
    p.enter(false, true);
    assert_eq!(p.html(), "<pre>abcd</pre><p></p><p>x</p>");
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(7)));
}

/// #2993's review, desktop: Ctrl/Cmd+Enter in a paragraph, where `exitCode`
/// does not apply, is the app's to use — the editor's `on_key` and the
/// keyboard interceptor hear it and the document does not change. Desktop
/// offers both before the keymap (the browser's order for the capture
/// listener), and `dispatch_key` now answers `None` for the inapplicable
/// binding, so the editor does not claim it either.
#[test]
fn mod_enter_outside_a_code_block_is_left_to_the_app() {
    use std::cell::RefCell;
    use std::rc::Rc;
    let mut p = page("<p>ab</p><pre>cd</pre>");
    let on_key = Rc::new(RefCell::new(Vec::<(String, bool)>::new()));
    let k1 = on_key.clone();
    p.handle.on_key(move |k| {
        k1.borrow_mut().push((k.key.to_string(), k.primary));
        false
    });
    let intercepted = Rc::new(RefCell::new(Vec::<(String, bool)>::new()));
    let i1 = intercepted.clone();
    rinch_core::set_keyboard_interceptor(move |k| {
        // The release reaches the interceptor too; the press is the one the
        // keymap could have taken.
        if k.kind == rinch_core::KeyEventKind::Down {
            i1.borrow_mut().push((k.key.clone(), k.ctrl || k.meta));
        }
        false
    });
    p.handle.set_selection(Selection::cursor(Pos(2)));
    p.enter(false, true);
    rinch_core::clear_keyboard_interceptor();
    assert_eq!(p.html(), "<p>ab</p><pre>cd</pre>", "nothing changed");
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(2)));
    assert_eq!(*on_key.borrow(), [("Enter".to_string(), true)]);
    assert_eq!(*intercepted.borrow(), [("Enter".to_string(), true)]);
    assert_eq!(
        p.handle
            .dispatch_key(rinch_editor_core::KeyBinding::parse("Mod-Enter").unwrap()),
        None,
        "the binding is not the editor's here"
    );
}
