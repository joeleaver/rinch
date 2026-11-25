//! The winit `ApplicationHandler`: the event loop's entry points into the
//! runtime.

use super::*;

impl ApplicationHandler for RinchRuntime {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
        // The outputs, before the window's first render reads them (#2414).
        crate::shell::outputs::publish(event_loop);
        // Popup windows: on Wayland (#2552).
        crate::popup::set_supported(
            crate::shell::overlay_window::Backend::of(event_loop)
                == crate::shell::overlay_window::Backend::Wayland,
        );
        // Whether the compositor paces Fifo itself (`wp_fifo_v1`), before
        // any window presents (#2669).
        #[cfg(feature = "gpu-renderer")]
        crate::shell::frame_pacing::note_compositor(event_loop);
        if self.start_surfaces(crate::shell::layer_shell::window_roles_of(event_loop)) {
            return;
        }
        // Mounted already means the window was hidden on purpose — started
        // hidden (#1520) or sent to the tray — and creating one here would
        // mount the root a second time; `show_window` is what brings it back.
        if creates_window_on_surfaces(self.main.window.is_some(), self.main.app.doc.is_some()) {
            self.create_window(event_loop);
        }
    }

    fn proxy_wake_up(&mut self, event_loop: &dyn ActiveEventLoop) {
        // A lost GPU device is replaced before anything draws (#1788).
        #[cfg(feature = "gpu-renderer")]
        self.settle_gpu_device();
        // Keep the pointer ahead of whatever the wake carries (an injected
        // press, a debug command that reads the hover state).
        for role in self.roles() {
            self.flush_pointer_move(role, event_loop);
        }
        let mut deferred_ran = Vec::new();
        drain_wake_queues(
            self,
            |rt| {
                // Drain main-thread callback queue.
                rinch_core::drain_main_callbacks();

                // Then the work other threads sent an app itself (issue #328):
                // a plain `<input>` paste whose clipboard read just answered.
                // Its wake is one of the callbacks drained above, so this is
                // the turn it asked for. It dirties the field it pastes into,
                // but a field with no layout change leaves
                // `has_pending_layout` false, so ask for the frame here.
                for role in rt.roles() {
                    if rt.w(role).app.run_deferred_work() > 0 {
                        deferred_ran.push(role);
                    }
                }
            },
            // Drain queued native events.
            |rt| rt.drain_native_events(event_loop),
        );

        // A frame for each window the drain left something to draw (#2608): a
        // cross-thread Signal::send()/update_send() drained above ran its
        // effects here, and what they changed on screen they changed in the
        // document (`RinchApp::wake_wants_frame`). A wake that changed nothing
        // there — window controls, debug commands, no-op callbacks, a signal
        // nothing draws — presents nothing.
        for role in self.roles() {
            let w = self.w(role);
            if w.app.wake_wants_frame(deferred_ran.contains(&role)) {
                if let Some(w) = &w.window {
                    w.request_redraw();
                }
            } else if let Some(renderer) = &mut w.soft_renderer {
                // A software window idle for its period gives its buffers
                // back (#2843); the timer that woke the loop for it is one of
                // the callbacks drained above. Not when a frame is coming:
                // it would only allocate them again.
                if renderer.service_idle_trim() {
                    crate::window_perf::trimmed(&w.app);
                }
            }
        }
        // Closed popups' renderers kept past their idle period go (#2552).
        #[cfg(feature = "gpu-renderer")]
        popups::trim_spare_renderers();
        self.settle_closes(event_loop);
    }

    fn monitors_changed(&mut self, event_loop: &dyn ActiveEventLoop) {
        // An output was added, removed or changed (#2414): publish the new
        // list, and draw what its readers changed.
        crate::shell::outputs::publish(event_loop);
        // A content-sized window's largest size follows its output (#2552).
        self.follow_outputs_in_every_window();
        for role in self.roles() {
            let w = self.w(role);
            if w.app.wake_wants_frame(false) {
                if let Some(w) = &w.window {
                    w.request_redraw();
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        // Found by its request's serial, not by `window_id`: winit sends it
        // even when the window that asked has been dropped, under that
        // window's id, which a newer window can have (#2501).
        if let WindowEvent::ActivationTokenDone { serial, token } = event {
            crate::shell::activation_tokens::done(serial, token.into_raw());
            return;
        }
        let Some(role) = self.role_of(window_id) else {
            return;
        };
        self.window_event_for(role, event_loop, event);
        self.settle_closes(event_loop);
    }

    fn device_event(
        &mut self,
        _event_loop: &dyn ActiveEventLoop,
        _device_id: Option<winit::event::DeviceId>,
        event: DeviceEvent,
    ) {
        let role = self.motion_role();
        self.w(role).note_device_event(&event);
    }

    fn about_to_wait(&mut self, event_loop: &dyn ActiveEventLoop) {
        // A rebuild after a device loss tries again at its time (#1788): the
        // wake it armed lands here.
        #[cfg(feature = "gpu-renderer")]
        self.settle_gpu_device();
        // The batch's raw motion and last pointer move, before anything is
        // laid out.
        self.flush_pointer_motion(event_loop);
        for role in self.roles() {
            self.w(role).restore_held_pointer_move();
            self.flush_pointer_move(role, event_loop);
        }
        self.keep_prepared();
        self.about_to_wait_inner(event_loop);
        // A popup whose anchor moved is placed again (#2552).
        self.follow_popup_anchors();
        self.settle_closes(event_loop);
        // Popups that waited for the next iteration are asked for again.
        self.popups_end_of_turn(event_loop);
    }
}
