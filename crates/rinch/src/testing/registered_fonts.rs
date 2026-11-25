//! `rinch::font::register_font` in the harness (#3042): the faces reach the
//! test's window and the windows it opened, as the shell's turn gives them
//! to every open window.

use super::TestApp;
use crate::font::AppFont;

/// The fonts a `TestApp` is mounted with: the builder's, then the faces
/// `register_font` queued before the mount, as `RinchRuntime::with_app`
/// takes them; a queued face identical to one already there is skipped.
pub(super) fn with_queued(mut fonts: Vec<AppFont>) -> Vec<AppFont> {
    crate::shell::rinch_runtime::take_new_fonts(&mut fonts);
    fonts
}

impl TestApp {
    /// Register the faces `rinch::font::register_font` queued with the
    /// window and every open child, and keep them for children opened later
    /// (`RinchRuntime::register_queued_fonts`). A face identical to one
    /// already registered is skipped.
    pub(super) fn register_queued_fonts(&mut self) {
        let fonts = crate::shell::rinch_runtime::take_new_fonts(&mut self.fonts);
        if fonts.is_empty() {
            return;
        }
        self.app.register_app_fonts(&fonts);
        for child in &mut self.children {
            child.register_app_fonts(&fonts);
        }
    }
}

#[cfg(test)]
mod tests {
    use rinch_core::dom::{NodeHandle, RenderScope};
    use rinch_core::element::WindowProps;

    use crate::font::AppFont;
    use crate::testing::TestApp;

    const FACE: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

    fn component(scope: &mut RenderScope) -> NodeHandle {
        scope.create_element("div")
    }

    /// The generics of each face `app` registered, in order.
    fn registered(app: &crate::app::RinchApp) -> Vec<parley::fontique::GenericFamily> {
        app.pending_fonts
            .iter()
            .map(|f| f.font.generics[0])
            .collect()
    }

    /// A `TestApp` behaves as the app does: the face a component registers
    /// reaches its window and the window it opened, once a turn has served
    /// the request, and a child opened afterwards is made with it. Before
    /// the fix the bytes stayed queued and no document ever had them.
    #[test]
    fn a_test_app_registers_the_face_with_every_window() {
        let mut app = TestApp::mount(component);
        crate::open_window(
            WindowProps {
                visible: false,
                ..Default::default()
            },
            component,
        );
        app.settle();
        assert_eq!(app.children.len(), 1, "the child window is open");
        let before = app.app.pending_fonts.len();

        crate::font::register_font(AppFont::monospace(FACE));
        assert_eq!(
            app.app.pending_fonts.len(),
            before,
            "nothing is registered until a turn serves the request"
        );
        app.settle();
        let mono = parley::fontique::GenericFamily::Monospace;
        assert_eq!(registered(&app.app)[before..], [mono]);
        assert_eq!(registered(app.children[0].app()), [mono]);

        // Again, identical: skipped by both windows.
        crate::font::register_font(AppFont::monospace(FACE));
        app.settle();
        assert_eq!(registered(&app.app)[before..], [mono]);
        assert_eq!(registered(app.children[0].app()), [mono]);

        // A window opened afterwards is made with it.
        crate::open_window(
            WindowProps {
                visible: false,
                ..Default::default()
            },
            component,
        );
        app.settle();
        assert_eq!(app.children.len(), 2);
        assert_eq!(registered(app.children[1].app()), [mono]);
    }

    /// A face registered by a child window's own code is the app's: the
    /// test's window and every child get it. Before the fix the event was
    /// routed to the child as its own request, which has none for it, and
    /// silently dropped, so no window ever had the face.
    #[test]
    fn a_face_from_a_childs_document_reaches_every_window() {
        let mut app = TestApp::mount(component);
        crate::open_window(
            WindowProps {
                visible: false,
                ..Default::default()
            },
            component,
        );
        app.settle();
        let before = app.app.pending_fonts.len();
        {
            let _doc = rinch_core::push_dispatching_doc(app.children[0].doc());
            crate::font::register_font(AppFont::monospace(FACE));
        }
        app.settle();
        let mono = parley::fontique::GenericFamily::Monospace;
        assert_eq!(registered(&app.app)[before..], [mono]);
        assert_eq!(registered(app.children[0].app()), [mono]);
    }

    /// A face registered before the `TestApp` is mounted reaches its window,
    /// after the builder's own fonts, as a face registered in an app's
    /// `main` reaches its first window (`RinchRuntime::with_app`).
    #[test]
    fn a_face_registered_before_the_mount_reaches_the_window() {
        // No `TestApp` takes the event yet: it goes to the process's queue,
        // which tests counting its events hold this lock to read.
        let _lock = crate::app::RERENDER_EVENTS_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        rinch_core::mark_host_thread();
        crate::font::register_font(AppFont::monospace(FACE));
        let app = TestApp::builder(component)
            .fonts(&[AppFont::serif(FACE)])
            .mount();
        crate::shell::rinch_runtime::drain_native_queue_for_test();
        let serif = parley::fontique::GenericFamily::Serif;
        let mono = parley::fontique::GenericFamily::Monospace;
        let registered = registered(&app.app);
        assert!(
            registered.ends_with(&[serif, mono]),
            "the builder's, then the queued face: {registered:?}"
        );
    }
}
