//! `rinch::font::register_font` in the harness (#3042): the faces reach the
//! test's window and the windows it opened, as the shell's turn gives them
//! to every open window.

use super::TestApp;

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
}
