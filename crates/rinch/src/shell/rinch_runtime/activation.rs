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

    #[test]
    fn outside_a_press_it_is_refused_once_in_a_later_turn() {
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
    fn inside_a_press_it_reaches_the_shell_as_the_asking_document_s_request() {
        let mut runtime = RinchRuntime::new("t", 800, 600, component, &[]);
        runtime.main.app.mount_component(800.0, 600.0);
        let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
        let answers = Answers::default();
        {
            let _doc = rinch_core::push_dispatching_doc(runtime.main.app.doc_key());
            let _press = crate::app::user_input::enter(&key_down());
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
