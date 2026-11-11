//! Asking the compositor for an xdg-activation token, and handing its answer
//! back (#2501): see [`crate::activation`].

use winit::platform::startup_notify::WindowExtStartupNotify;

use super::*;
use crate::activation::{ActivationTokenError, answer};
use crate::shell::activation_tokens;

impl RinchRuntime {
    /// Ask for the token `request` waits for, for `role`'s window. Only on
    /// Wayland: winit's X11 backend answers with a startup-notification ID,
    /// a different mechanism this API does not offer.
    pub(super) fn request_activation_token(
        &mut self,
        role: WindowRole,
        request: rinch_core::MainCallbackId,
        event_loop: &dyn ActiveEventLoop,
    ) {
        if Backend::of(event_loop) != Backend::Wayland {
            answer(request, Err(ActivationTokenError::Unsupported));
            return;
        }
        let w = self.w(role);
        let Some(window) = &w.window else {
            // Hidden when it asked, or between the asking and this turn.
            answer(request, Err(ActivationTokenError::WindowClosed));
            return;
        };
        match window.raw().request_activation_token() {
            Ok(serial) => activation_tokens::requested(serial, w.facts_key, request),
            // No `xdg_activation_v1` global.
            Err(error) => {
                tracing::debug!("activation token unavailable: {error}");
                answer(request, Err(ActivationTokenError::Unsupported));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activation::{ActivationTokenResult, request_activation_token};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn component(scope: &mut RenderScope) -> NodeHandle {
        scope.create_element("div")
    }

    /// The answers a test's requests were given.
    type Answers = Rc<RefCell<Vec<ActivationTokenResult>>>;

    /// A callback that records its answer in `answers`.
    fn recorder(answers: &Answers) -> impl FnOnce(ActivationTokenResult) + 'static {
        let answers = Rc::clone(answers);
        move |result| answers.borrow_mut().push(result)
    }

    fn key_down() -> PlatformEvent {
        PlatformEvent::KeyDown {
            key: KeyCode::Enter,
            logical_key: None,
            text: None,
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Fresh,
        }
    }

    /// How many token requests reached the shell since the last call.
    fn requests_queued() -> usize {
        NATIVE_EVENT_QUEUE
            .lock()
            .unwrap()
            .drain()
            .iter()
            .filter(|(_, event)| matches!(event, RinchNativeEvent::RequestActivationToken(_)))
            .count()
    }

    /// A runtime whose page has, at (40, 40) and 100 by 40, an element whose
    /// `attribute` handler asks for a token; with `draggable` it is a drag
    /// source, whose click comes on the release.
    fn asking_element(attribute: &'static str, draggable: bool, answers: &Answers) -> RinchRuntime {
        let answers = Rc::clone(answers);
        let mut runtime = RinchRuntime::new(
            "t",
            800,
            600,
            move |scope: &mut RenderScope| {
                let root = scope.create_element("div");
                root.set_attribute("style", "position: relative; width: 800px; height: 600px");
                let element = scope.create_element("div");
                element.set_attribute(
                    "style",
                    "position: absolute; left: 40px; top: 40px; width: 100px; height: 40px",
                );
                if draggable {
                    element.set_attribute("draggable", "true");
                }
                let ask = scope.register_handler(move || {
                    request_activation_token(recorder(&answers));
                });
                element.set_attribute(attribute, &ask.0.to_string());
                root.append_child(&element);
                root
            },
            &[],
        );
        runtime.main.app.mount_component(800.0, 600.0);
        runtime.main.app.resolve_and_repaint(800.0, 600.0);
        let _ = requests_queued();
        runtime
    }

    fn send(runtime: &mut RinchRuntime, event: PlatformEvent) {
        let _ = runtime.main.app.handle_event(event, (800, 600), 1.0);
    }

    const ON_IT: (f32, f32) = (61.0, 57.0);

    #[test]
    fn a_drag_source_s_click_on_the_release_is_a_request() {
        let answers = Answers::default();
        let mut runtime = asking_element("data-rid", true, &answers);
        let (x, y) = ON_IT;
        let button = rinch_platform::MouseButton::Left;
        send(&mut runtime, PlatformEvent::MouseDown { x, y, button });
        assert_eq!(requests_queued(), 0, "a pending drag clicks on the release");
        send(&mut runtime, PlatformEvent::MouseUp { x, y, button });
        assert_eq!(requests_queued(), 1, "the release's click asked");
        rinch_core::drain_main_callbacks();
        assert!(
            answers.borrow().is_empty(),
            "not refused: it waits for the compositor"
        );
    }

    #[test]
    fn a_move_handler_is_refused() {
        let answers = Answers::default();
        let mut runtime = asking_element("data-onmousemove", false, &answers);
        let (x, y) = ON_IT;
        send(&mut runtime, PlatformEvent::MouseMove { x, y });
        assert_eq!(requests_queued(), 0);
        rinch_core::drain_main_callbacks();
        assert_eq!(*answers.borrow(), [Err(ActivationTokenError::NotFromInput)]);
    }

    #[test]
    fn outside_a_user_action_it_is_refused_once_in_a_later_turn() {
        let runtime = RinchRuntime::new("t", 800, 600, component, &[]);
        let answers = Answers::default();
        request_activation_token(recorder(&answers));
        assert!(answers.borrow().is_empty(), "never inside the request");
        rinch_core::drain_main_callbacks();
        assert_eq!(*answers.borrow(), [Err(ActivationTokenError::NotFromInput)]);
        rinch_core::drain_main_callbacks();
        assert_eq!(answers.borrow().len(), 1, "exactly once");
        drop(runtime);
    }

    #[test]
    fn inside_a_key_press_it_reaches_the_shell_as_the_asking_document_s_request() {
        let mut runtime = RinchRuntime::new("t", 800, 600, component, &[]);
        runtime.main.app.mount_component(800.0, 600.0);
        let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
        let answers = Answers::default();
        {
            let _doc = rinch_core::push_dispatching_doc(runtime.main.app.doc_key());
            let _action = crate::app::user_input::enter(&key_down());
            request_activation_token(recorder(&answers));
        }
        let queued = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
        let requests: Vec<_> = queued
            .iter()
            .filter(|(_, event)| matches!(event, RinchNativeEvent::RequestActivationToken(_)))
            .collect();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, Some(runtime.main.app.doc_key()));
        rinch_core::drain_main_callbacks();
        assert!(answers.borrow().is_empty(), "waits for the compositor");
    }

    #[test]
    fn a_component_unmounted_before_the_answer_is_never_called() {
        let runtime = RinchRuntime::new("t", 800, 600, component, &[]);
        let answers = Answers::default();
        let owner = rinch_core::reactive::Scope::new();
        owner.run(|| request_activation_token(recorder(&answers)));
        owner.dispose();
        rinch_core::drain_main_callbacks();
        assert!(answers.borrow().is_empty());
        drop(runtime);
    }
}
