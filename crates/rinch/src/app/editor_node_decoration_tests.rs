//! `Decoration::Node` on a live desktop editor: an app's rule on the class (and
//! on the next block, through `+`) restyles and lays out again when only the
//! decorations change, the caret and the selection highlight follow, and an
//! incremental frame leaves nothing behind.

use super::*;
use rinch_editor_core::decoration::{Decoration, DecorationSet};
use rinch_editor_core::{EditorState, Plugin, PluginKey, Pos, Selection};

const VP: (u32, u32) = (800, 600);

/// The first block, with whatever class the test asks for.
#[derive(Default)]
struct First {
    class: RefCell<Option<&'static str>>,
}

impl Plugin for First {
    fn key(&self) -> PluginKey {
        PluginKey("test.node-decoration")
    }
    fn decorations(&self, state: &EditorState) -> DecorationSet {
        let end = Pos(state.doc.child(0).node_size());
        DecorationSet::new(
            self.class
                .borrow()
                .map(|c| Decoration::node(Pos(0), end, c))
                .into_iter()
                .collect(),
        )
    }
}

struct Page {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
    first: Rc<First>,
}

const CSS: &str = "[data-pm-editor] p { margin: 0; line-height: 24px; } \
    [data-pm-editor] p.shaded { background-color: rgb(200, 200, 0); } \
    [data-pm-editor] p.spaced + p { margin-top: 40px; } \
    [data-pm-editor] p.folded + p { display: none; }";

fn page() -> Page {
    let slot: Rc<RefCell<Option<crate::editor::EditorHandle>>> = Rc::default();
    let first = Rc::new(First::default());
    let (slot_in, plugin) = (slot.clone(), first.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let style = scope.create_element("style");
        style.append_child(&scope.create_text(CSS));
        root.append_child(&style);
        let (container, handle) = crate::editor::mount_editor(scope);
        assert!(handle.add_plugin(plugin.clone()));
        handle.load_html("<p>heading</p><p>body text</p><p>after it</p>");
        container.set_attribute(
            "style",
            "width: 400px; height: 300px; font-size: 16px; font-family: sans-serif",
        );
        root.append_child(&container);
        *slot_in.borrow_mut() = Some(handle);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let handle = slot.borrow_mut().take().unwrap();
    // The selection highlight is drawn in the focused editor only.
    handle.focus();
    let mut page = Page { app, handle, first };
    page.settle();
    page
}

impl Page {
    /// Change the decoration with a transaction that changes nothing else, and
    /// run a frame as the shell does.
    fn decorate(&mut self, class: Option<&'static str>) {
        *self.first.class.borrow_mut() = class;
        assert!(self.handle.update(|st| Some(st.tr())));
        self.settle();
    }

    /// Run the loop's idle turns, drawing each frame it asks for, so the
    /// editor's owed overlay pass runs.
    fn settle(&mut self) {
        for _ in 0..3 {
            let actions = self.app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
            if actions.contains(&AppAction::RequestRedraw) {
                if self.app.has_pending_layout() {
                    self.app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
                }
                let _ = self.app.build_pixels(1.0, VP, false);
            }
        }
    }

    fn y(&self, pos: usize) -> f32 {
        self.app
            .editor_caret_point(&self.handle, Pos(pos))
            .unwrap()
            .1
    }

    /// The painted `y` of each selection highlight rectangle (each is placed by
    /// a transform, so its layout box says nothing about where it is).
    fn washes(&self) -> Vec<f32> {
        let mut doc = self.app.doc.as_ref().unwrap().borrow_mut();
        doc.query_selector_all("[data-pm-selection]")
            .into_iter()
            .filter_map(|id| doc.bounding_rect(id))
            .map(|b| b.y)
            .collect()
    }
}

// "heading" is 0..9, "body text" 9..20, "after it" 20..30.
const BODY: usize = 10;
const AFTER: usize = 21;

#[test]
fn a_rule_through_the_next_sibling_restyles_when_only_the_decoration_changes() {
    let mut p = page();
    let (body, after) = (p.y(BODY), p.y(AFTER));
    for _ in 0..2 {
        p.decorate(Some("spaced"));
        assert_eq!((p.y(BODY), p.y(AFTER)), (body + 40.0, after + 40.0));
        p.decorate(Some("folded"));
        assert_eq!(p.y(AFTER), body, "the hidden block takes no room");
        p.decorate(None);
        assert_eq!((p.y(BODY), p.y(AFTER)), (body, after));
    }
}

#[test]
fn the_selection_highlight_follows_a_decoration_that_moves_the_text() {
    let mut p = page();
    p.handle
        .set_selection(Selection::text(Pos(AFTER), Pos(AFTER + 5)));
    p.settle();
    let before = p.washes();
    assert_eq!(before.len(), 1, "positive control: one highlight");
    p.decorate(Some("spaced"));
    assert_eq!(p.washes(), [before[0] + 40.0]);
    p.decorate(None);
    assert_eq!(p.washes(), before);
}

#[cfg(software_shell)]
fn frame(app: &mut RinchApp) -> Vec<u8> {
    app.scene_dirty = true;
    app.has_previous_frame = false;
    app.build_pixels(1.0, VP, false).0.to_vec()
}

/// An incremental frame after the class goes (or comes) equals a full one.
#[cfg(software_shell)]
#[test]
fn an_incremental_frame_after_a_decoration_change_equals_a_full_one() {
    let mut p = page();
    let plain = frame(&mut p.app);
    for class in [Some("shaded"), Some("spaced"), Some("folded"), None] {
        p.decorate(class);
        p.app.scene_dirty = true;
        let incremental = p.app.build_pixels(1.0, VP, false).0.to_vec();
        let full = frame(&mut p.app);
        assert!(incremental == full, "{class:?}: stale pixels");
        assert_eq!(full == plain, class.is_none(), "{class:?}");
    }
}
