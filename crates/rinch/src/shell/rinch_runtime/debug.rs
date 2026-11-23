//! The debug server's commands that need the renderer.

use super::*;

impl ShellWindow {
    /// Capture a screenshot from whichever renderer is active.
    #[cfg(feature = "debug")]
    fn capture_screenshot_impl(&mut self) -> DebugResult {
        #[cfg(feature = "gpu-renderer")]
        if let Some(renderer) = &self.renderer {
            return match renderer.capture_screenshot() {
                Ok((w, h, rgba)) => {
                    let png_bytes = screenshot::encode_png(&rgba, w, h);
                    DebugResult::Bytes {
                        data: base64::engine::general_purpose::STANDARD.encode(&png_bytes),
                    }
                }
                Err(e) => DebugResult::Error {
                    message: format!("Screenshot capture failed: {}", e),
                },
            };
        }
        let scale = self.scale_factor();
        let size = self.logical_size();
        // The frame `paint()` just presented, read back without repainting —
        // a repaint here would have no surface or video frame maps (#364).
        let (pixels, w, h) = self.app.screenshot_pixels(scale, size);
        let png_bytes = screenshot::encode_png(pixels, w, h);
        DebugResult::Bytes {
            data: base64::engine::general_purpose::STANDARD.encode(&png_bytes),
        }
    }
}

impl RinchRuntime {
    /// The main window made ready for a debug screenshot painted inside the
    /// drain (#2829): what the turn staged for its layer surface so far is
    /// sent first ([`Self::flush_layer_turn`]), so a `SetWindowSize` before
    /// the screenshot in the same drain is the size it is painted at. That
    /// part of the turn is one commit, and the frame's commit follows it;
    /// what the drain's later events stage goes out when it ends.
    #[cfg(any(feature = "debug", test))]
    pub(super) fn ready_for_screenshot(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.flush_layer_turn(WindowRole::Main, event_loop);
    }

    /// Handle debug commands that require the renderer (e.g., screenshots).
    /// Returns true if the event loop should exit.
    #[cfg(feature = "debug")]
    pub(super) fn handle_debug_commands_with_renderer(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
    ) -> bool {
        let Some(rx) = self.main.app.debug_cmd_rx.take() else {
            return false;
        };

        let mut should_exit = false;

        // Process commands as they arrive. After answering command N, wait a
        // short beat for a pipelined follow-up — a fast client sends command
        // N+1 the moment it reads response N, but by then the paint N induced
        // is usually already dispatched, so without this window N+1 would
        // stall behind that full paint (#153). Batching here lets the whole
        // burst ride a single paint. The window is bounded so a spamming
        // client cannot starve painting.
        let batch_start = std::time::Instant::now();
        let mut next = rx.0.try_recv().ok();
        while let Some(cmd) = next {
            let response = match &cmd.kind {
                // A window waiting for its renderer after a device loss
                // (#1788) has none to draw with: not painted with software.
                #[cfg(feature = "gpu-renderer")]
                rinch_debug::DebugCommandKind::Screenshot if self.awaits_gpu(WindowRole::Main) => {
                    DebugResult::Error {
                        message: "the window waits for a new GPU device after a device loss"
                            .to_string(),
                    }
                }
                rinch_debug::DebugCommandKind::Screenshot => {
                    // Screenshot needs the renderer -- handle it here in the shell
                    self.ready_for_screenshot(event_loop);
                    if let Err(e) = self.paint() {
                        DebugResult::Error {
                            message: format!("Paint failed: {}", e),
                        }
                    } else {
                        self.main.capture_screenshot_impl()
                    }
                }
                _ => {
                    // All other commands are handled by the platform-agnostic RinchApp
                    let mut actions = Vec::new();
                    let scale = self.main.scale_factor();
                    let size = self.main.window_size();
                    // We need to take the kind out of cmd, but cmd.kind is borrowed above.
                    // Re-dispatch through execute_debug_command on the app.
                    let result = self.main.app.execute_debug_command(
                        cmd.kind.clone(),
                        &mut actions,
                        scale,
                        size,
                    );
                    // Process any actions generated
                    for action in actions {
                        match action {
                            AppAction::Exit => {
                                should_exit = true;
                            }
                            AppAction::RequestRedraw => {
                                if let Some(w) = &self.main.window {
                                    w.request_redraw();
                                }
                            }
                            AppAction::DragWindow => {
                                if let Some(w) = &self.main.window {
                                    let _ = w.drag_window();
                                }
                            }
                            _ => {}
                        }
                    }
                    result
                }
            };
            let _ = cmd.response_tx.send(response);

            next = if batch_start.elapsed() < std::time::Duration::from_millis(25) {
                rx.0.recv_timeout(std::time::Duration::from_millis(2)).ok()
            } else {
                rx.0.try_recv().ok()
            };
        }

        self.main.app.debug_cmd_rx = Some(rx);
        should_exit
    }
}
