//! Editor accessibility bridge — pushes the editor's [`accesskit::TreeUpdate`]
//! (derived purely in `rinch_editor_core::a11y`) to the platform assistive-tech
//! adapter, and routes AT [`accesskit::ActionRequest`]s back to the runtime.
//!
//! The derivation is portable; only the *adapter* is platform-specific. Linux uses
//! `accesskit_unix` (AT-SPI over D-Bus); other desktop platforms get a no-op bridge
//! until their hand-written adapters land (the published `accesskit_winit` binds
//! monolithic winit 0.30 and won't compile against the forked winit-core 0.31 —
//! design amendment A2). Compiled only under the `a11y` feature.
//!
//! ## Threading
//! `accesskit_unix` runs the AT-SPI handlers on its own async thread, so they must
//! be `Send`. The activation handler reads the latest tree from a shared
//! `Arc<Mutex<Option<TreeUpdate>>>` (AccessKit types are `Send`); action requests
//! are forwarded to the main thread through the runtime's `send_native_event`
//! queue (which also wakes the event loop), exactly like other off-thread events.

#[cfg(target_os = "linux")]
pub(crate) use unix::AccesskitBridge;

#[cfg(not(target_os = "linux"))]
pub(crate) use noop::AccesskitBridge;

#[cfg(target_os = "linux")]
mod unix {
    use std::sync::{Arc, Mutex};

    use accesskit::{
        ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler, TreeUpdate,
    };
    use rinch_editor_core::EditorState;
    use rinch_editor_core::a11y::build_tree_update;

    use crate::shell::rinch_runtime::{RinchNativeEvent, send_native_event};

    /// The latest tree, shared with the AT thread's activation handler.
    type SharedTree = Arc<Mutex<Option<TreeUpdate>>>;

    /// Returns the initial tree when the AT connects (on the adapter thread).
    struct Activation(SharedTree);
    impl ActivationHandler for Activation {
        fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
            self.0.lock().unwrap().clone()
        }
    }

    /// Forwards an AT action to the main thread (and wakes the event loop).
    struct Action;
    impl ActionHandler for Action {
        fn do_action(&mut self, request: ActionRequest) {
            send_native_event(RinchNativeEvent::A11yAction(request));
        }
    }

    struct Deactivation;
    impl DeactivationHandler for Deactivation {
        fn deactivate_accessibility(&mut self) {}
    }

    /// Owns the AT-SPI adapter and the shared tree. Lives on the main thread.
    pub(crate) struct AccesskitBridge {
        adapter: accesskit_unix::Adapter,
        shared: SharedTree,
    }

    impl AccesskitBridge {
        pub(crate) fn new() -> Self {
            let shared: SharedTree = Arc::new(Mutex::new(None));
            let adapter =
                accesskit_unix::Adapter::new(Activation(shared.clone()), Action, Deactivation);
            Self { adapter, shared }
        }

        /// Re-derive and push the editor's tree. A no-op for the AT when none is
        /// connected (`update_if_active` short-circuits), but always refreshes the
        /// shared snapshot so a *later* AT connection sees the current tree.
        pub(crate) fn update(&mut self, state: &EditorState) {
            let tree = build_tree_update(state);
            *self.shared.lock().unwrap() = Some(tree.clone());
            self.adapter.update_if_active(|| tree);
        }

        /// Tell the AT whether the window is focused.
        pub(crate) fn set_window_focused(&mut self, focused: bool) {
            self.adapter.update_window_focus_state(focused);
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod noop {
    use rinch_editor_core::EditorState;

    /// No-op accessibility bridge for platforms without a hand-written adapter yet
    /// (Windows/macOS — follow-up). Keeps the `a11y` feature buildable everywhere.
    pub(crate) struct AccesskitBridge;

    impl AccesskitBridge {
        pub(crate) fn new() -> Self {
            AccesskitBridge
        }
        pub(crate) fn update(&mut self, _state: &EditorState) {}
        pub(crate) fn set_window_focused(&mut self, _focused: bool) {}
    }
}
