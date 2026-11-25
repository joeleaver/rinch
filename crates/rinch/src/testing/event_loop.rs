//! The harness's event loop: what the shell does between input and the
//! next frame (window requests, content size, timers, layout, paint), and
//! the virtual clock that [`TestApp::settle`] and
//! [`TestApp::advance_time`] drive.

use super::*;

impl TestApp {
    // ── The event loop ──────────────────────────────────────────────────

    /// The window size the shell hands `handle_event`: physical pixels.
    ///
    /// Also the harness's stand-in for its surface's size, which it has no
    /// surface to read: what
    /// [`window_physical_size`](crate::window::window_physical_size) answers
    /// under a `TestApp`. The shell publishes the surface's own size instead.
    pub(super) fn physical_size(&self) -> (u32, u32) {
        (
            (self.size.0 as f64 * self.scale).round() as u32,
            (self.size.1 as f64 * self.scale).round() as u32,
        )
    }

    /// Hand `event` to the app as the shell's `dispatch_event` does, then run
    /// one turn of the loop.
    pub(super) fn send(&mut self, event: PlatformEvent) {
        let asked = self.dispatch(event);
        self.turn(asked);
        self.serve_window_requests();
    }

    /// Send the window gaining or losing the keyboard, unless it already
    /// has (or lacks) it.
    pub(super) fn set_window_focus(&mut self, focused: bool) {
        if self.focused != focused {
            self.focused = focused;
            self.send(PlatformEvent::WindowFocus(focused));
        }
    }

    /// The requests the app's code made of its window since this last ran:
    /// recorded, and the ones that change what a test sees applied. What an
    /// applied request runs may ask for more, which are served in the next
    /// round.
    pub(super) fn serve_window_requests(&mut self) {
        if self.serving {
            return;
        }
        self.serving = true;
        for _ in 0..MAX_REQUEST_ROUNDS {
            let events = std::mem::take(&mut *self.sink.events.borrow_mut());
            if events.is_empty() {
                break;
            }
            for (to, event) in events {
                if let RinchNativeEvent::OpenWindow(id) = event {
                    // A window the component opened: made after the turn
                    // that built it, as the shell makes it on a later wake.
                    self.sink.opened.borrow_mut().push((id, 0));
                    self.sink.opening.set(true);
                    let opened = children::Child::open(id, self);
                    self.sink.opening.set(false);
                    if let Some(child) = opened {
                        let doc = child.doc();
                        for entry in self.sink.opened.borrow_mut().iter_mut() {
                            if entry.0 == id {
                                entry.1 = doc;
                            }
                        }
                        self.children.push(child);
                        if let Some(child) = self.children.last_mut() {
                            child.turn();
                        }
                    }
                    continue;
                }
                if matches!(event, RinchNativeEvent::RegisterFonts) {
                    // `rinch::font::register_font`: the app's, whichever
                    // window's code sent it (#3042), as the shell serves it.
                    self.register_queued_fonts();
                    continue;
                }
                let (window, doc) = match to {
                    EventFor::Window(window) => (Some(window), None),
                    EventFor::Sender(doc) => (None, doc),
                };
                if let Some(at) = self.child_at(window, doc) {
                    // An open child's own request, through its handle or
                    // from its document's code: its own, never the app's.
                    if self.children[at].native(event) {
                        self.close_child(at);
                    }
                } else if let Some(window) = window {
                    // A window a `WindowHandle` names that is not open here
                    // (closed, or never made): recorded only.
                    if let Some(request) = WindowRequest::from_native(event) {
                        self.requests.push(WindowRequest::OtherWindow {
                            window,
                            request: Box::new(request),
                        });
                    }
                } else if let RinchNativeEvent::InjectedPlatformEvent(event) = event {
                    // `inject_platform_event`: the shell dispatches it.
                    self.send(event);
                } else if matches!(event, RinchNativeEvent::ApplyWindowTitle) {
                    // `rinch::window::set_window_title` in this app's
                    // document: the title it stored is the window's title
                    // request. The store is the process's, so only this
                    // document's titles are taken, never another test's.
                    let doc = self.sink.doc.get();
                    if let Some(title) = crate::window::take_stored_title(|d| d == Some(doc)) {
                        self.requests.push(WindowRequest::SetTitle(title));
                    }
                } else if let Some(request) = WindowRequest::from_native(event) {
                    self.requests.push(request.clone());
                    self.apply(request);
                }
            }
        }
        // Whatever became of the event that asked, as the shell's every turn.
        self.register_queued_fonts();
        self.serving = false;
    }

    /// Apply a window request to the test's window, as far as a test can see
    /// it.
    fn apply(&mut self, request: WindowRequest) {
        match request {
            WindowRequest::Show if !self.visible => {
                self.visible = true;
                // A new surface: a content-sized window measures afresh.
                if self.app.is_content_sized() {
                    self.restart_content_size();
                }
                self.paint();
                if !self.non_activating {
                    self.set_window_focus(true);
                }
            }
            WindowRequest::Hide if self.visible => {
                self.visible = false;
                // The shell drops a hidden window's renderer, its timer with
                // it.
                self.presentation = None;
                // A hidden window has no keyboard.
                self.set_window_focus(false);
            }
            // A hidden window is shown by it; a non-activating one never
            // takes the keyboard.
            WindowRequest::Focus | WindowRequest::ActivateWithToken(_) if !self.visible => {
                self.apply(WindowRequest::Show)
            }
            WindowRequest::Focus | WindowRequest::ActivateWithToken(_) if !self.non_activating => {
                self.set_window_focus(true)
            }
            WindowRequest::Prepare(_) if !self.visible => {
                if self.app.is_content_sized() {
                    self.restart_content_size();
                }
                self.paint_now();
            }
            WindowRequest::SetSize(width, height) if !self.app.is_content_sized() => {
                self.resize(width.round() as u32, height.round() as u32);
            }
            _ => {}
        }
    }

    /// A content-sized window's new negotiation (a show, a prepare): free
    /// again at its largest size, then the content's size there.
    fn restart_content_size(&mut self) {
        if let Some(max) = self.content_max() {
            self.app.restart_content_limit(max);
        }
        self.negotiate_content_size();
    }

    /// A content-sized window's largest size, as the shell's
    /// `ShellWindow::content_max` gives it: the app's maximum, else the size
    /// of the output it is on among those played
    /// ([`TestApp::set_outputs`]), else the props' size when none is.
    fn content_max(&self) -> Option<(f32, f32)> {
        content_max_of(&self.app)
    }

    /// The outputs changed: a content-sized window whose largest size
    /// changed starts a new negotiation, which the next turn's
    /// `follow_content_size` grants, as `ShellWindow::follow_outputs` does.
    pub(super) fn follow_outputs(&mut self) {
        let Some(max) = self.content_max() else {
            return;
        };
        if self.app.content_largest() != Some(max) {
            self.app.restart_content_limit(max);
            self.app.remeasure_content();
        }
    }

    /// Give a content-sized window its content's size, as the shell does when
    /// it makes the surface: measured, granted, and the window resized to it.
    pub(super) fn negotiate_content_size(&mut self) {
        if let Some((width, height)) = self.app.content_size() {
            self.grant(width, height);
        }
    }

    /// After a turn: a content-sized window whose content changed size asks
    /// for the new size, and is given it at once, as a layer surface or a
    /// plain toplevel is (`RinchRuntime::follow_content_size`). A hidden
    /// window asks nothing: its show measures.
    fn follow_content_size(&mut self) -> bool {
        if !self.visible || !self.app.is_content_sized() {
            return false;
        }
        if self.app.has_pending_layout() {
            self.layout();
        }
        match self.app.content_size_changed() {
            Some((width, height)) => {
                self.grant(width, height);
                true
            }
            None => false,
        }
    }

    /// The window given `width`x`height` logical pixels: the document's grant,
    /// then the `Resized` the shell sends for a `SurfaceResized`.
    fn grant(&mut self, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        self.app.set_granted_size(width as f32, height as f32);
        self.size = (width, height);
        self.dispatch(PlatformEvent::Resized { width, height });
    }

    /// `handle_event` alone, keeping what the app asked for.
    fn dispatch(&mut self, event: PlatformEvent) -> bool {
        // The shell sets the surface's size before the `Resized` its
        // `SurfaceResized` makes.
        if matches!(event, PlatformEvent::Resized { .. }) {
            self.app.set_surface_size(self.physical_size());
        }
        #[cfg(test)]
        if matches!(event, PlatformEvent::AboutToWait) {
            self.harness_work.about_to_waits += 1;
        }
        let actions = self
            .app
            .handle_event(event, self.physical_size(), self.scale);
        self.note(actions)
    }

    /// Lay the document out at the window's size and repaint what changed
    /// (`resolve_and_repaint`), as the shell does before a frame. Every
    /// layout the harness asks of the main window after mounting is asked
    /// here, where the in-crate test counts it.
    fn layout(&mut self) {
        self.app
            .resolve_and_repaint(self.size.0 as f32, self.size.1 as f32);
        #[cfg(test)]
        {
            self.harness_work.layouts += 1;
        }
    }

    /// Keep `actions`; answer whether they ask for a frame.
    pub(super) fn note(&mut self, actions: Vec<AppAction>) -> bool {
        let redraw = actions.contains(&AppAction::RequestRedraw);
        self.actions.extend(actions);
        redraw
    }

    /// One turn of the desktop loop after an event: the wake's queues
    /// (`proxy_wake_up`), `AboutToWait`, and the frame if anything asked for
    /// one — `redraw_asked` is whether the event itself did (the shell's
    /// `process_actions` requests that redraw before the loop turns). What the
    /// drain left to draw asks for the frame too, as `proxy_wake_up` decides
    /// it (`RinchApp::wake_wants_frame`: a signal that changed nothing on
    /// screen asks for none, #2608). Answers whether this turn painted.
    pub(super) fn turn(&mut self, redraw_asked: bool) -> bool {
        // The timers due now (their native thread queues them as main-thread
        // callbacks), the main-thread callbacks, then the work other threads
        // sent this app (#328) — `RinchRuntime::proxy_wake_up`'s order.
        self.fire_due_timers();
        rinch_core::drain_main_callbacks();
        let ran = self.app.run_deferred_work();
        let asked = self.dispatch(PlatformEvent::AboutToWait);
        let resized = self.follow_content_size();
        let painted = if (redraw_asked || asked || resized || self.app.wake_wants_frame(ran > 0))
            && self.visible
        {
            self.paint();
            true
        } else {
            // An idle window gives its buffers back (#2843), as
            // `proxy_wake_up` serves the trim when no frame is coming.
            if let Some(presentation) = &mut self.presentation {
                presentation.service_idle_trim(rinch_core::timer::now());
                if presentation.take_trimmed() {
                    crate::window_perf::trimmed(&self.app);
                }
            }
            false
        };
        // Each open child's part of the turn, short of painting (#2793).
        for child in &mut self.children {
            child.turn();
            #[cfg(test)]
            {
                self.child_turns += 1;
            }
        }
        painted
    }

    /// After input to a child window: a turn of the loop, then its
    /// requests.
    pub(super) fn after_child_input(&mut self) {
        self.turn(false);
        self.serve_window_requests();
    }

    /// Fire the timers due at the virtual clock's time now, armed before this
    /// call (one armed by them waits for the next turn, as on the desktop).
    fn fire_due_timers(&mut self) {
        let Some(now) = virtual_time::elapsed() else {
            return;
        };
        let armed_before = virtual_time::next_seq();
        while let Some(id) = virtual_time::take_due(now, armed_before) {
            rinch_core::timer::fire_timeout(id);
        }
    }

    /// One redraw, as `ShellWindow::paint_software` does it: the queues it
    /// drains first, the layout it resolves, the frame. Nothing while the
    /// window is hidden: a hidden window presents nothing.
    pub(super) fn paint(&mut self) {
        if self.visible {
            self.paint_now();
        }
    }

    /// [`Self::paint`], shown or not: a prepare paints the hidden window.
    fn paint_now(&mut self) {
        rinch_core::drain_main_callbacks();
        rinch_core::reactive::drain_polls();
        if self.app.has_pending_layout() {
            self.layout();
        }
        let transparent = self.app.is_transparent();
        let (_, w, h) = self.app.build_pixels(self.scale, self.size, transparent);
        // A shown window presents; a prepared one does not.
        if self.visible {
            self.present(w, h);
        }
        let _frame = self.app.end_perf_frame();
        #[cfg(test)]
        if let Some(frame) = _frame {
            self.painted.accumulate(&frame);
        }
        #[cfg(test)]
        if _frame.is_some() && crate::window_perf::capture::listening() {
            let doc = self.doc().borrow();
            let perf = &doc.tree.perf;
            crate::window_perf::capture::note(&match perf.label() {
                Some(label) => format!("[PERF] {label} frame {}", perf.frames()),
                None => format!("[PERF] frame {}", perf.frames()),
            });
        }
    }

    /// Present the frame just painted (`w` x `h`), as the shell's
    /// `paint_software_at` does: the presentation made at the first present,
    /// with its `[PERF] idle_trim=` line (#2917), its `[PERF] trimmed` line
    /// when this redraw presented nothing and served the idle trim instead,
    /// and its `[PERF] untrimmed` line when this present made its buffers
    /// again after a trim (#2852).
    fn present(&mut self, w: u32, h: u32) {
        let presentation = self.presentation.get_or_insert_with(|| {
            let period = self
                .app
                .window_props
                .as_ref()
                .map_or_else(Default::default, |p: &WindowProps| p.idle_trim)
                .period();
            let presentation = IdleTrimmed::new(Presenter::new(true, period, |_, _| NoSurface));
            crate::window_perf::idle_trim(&self.app, presentation.idle_period());
            presentation
        });
        let generation = self.app.pixels_generation();
        presentation.present_frame(&[], w, h, generation, rinch_core::timer::now());
        if presentation.take_trimmed() {
            crate::window_perf::trimmed(&self.app);
        }
        if presentation.take_untrimmed() {
            crate::window_perf::untrimmed(&self.app);
        }
    }

    /// Run turns of the loop until the app stops asking for frames — what
    /// an idle event loop does after the last input — or 16 turns have run (an
    /// animation that is still going asks every turn).
    ///
    /// Every input method already runs one turn; call this after a change made
    /// outside input (a signal written by the test) or to let a short
    /// transition finish.
    pub fn settle(&mut self) {
        for _ in 0..SETTLE_TURNS {
            let painted = self.turn(false);
            self.serve_window_requests();
            if !painted && !rinch_core::main_callbacks_pending() {
                return;
            }
        }
    }

    /// Move the virtual clock forward by `by`, firing every `set_timeout` and
    /// `set_interval` timer that comes due on the way: in deadline order
    /// (timers due together in the order they were armed), the clock at each
    /// one's deadline when it fires, and a turn of the loop after each
    /// deadline, so effects, layout and window requests are current before a
    /// later timer fires (the timers due at one instant fire in one turn). A
    /// timer one of them arms fires too if it comes due inside `by`. See
    /// [Time](super#time).
    ///
    /// Panics after 100 000 timers in one call: a timer re-arming itself
    /// with no delay never lets the clock move.
    pub fn advance_time(&mut self, by: Duration) {
        let Some(start) = virtual_time::elapsed() else {
            return;
        };
        let until = start + by;
        let mut fired = 0;
        while let Some(id) = virtual_time::take_due(until, u64::MAX) {
            fired += 1;
            assert!(
                fired <= MAX_TIMERS_PER_ADVANCE,
                "advance_time fired {MAX_TIMERS_PER_ADVANCE} timers without reaching \
                 its end: a timer is re-arming itself with no delay"
            );
            rinch_core::timer::fire_timeout(id);
            self.turn(false);
            self.serve_window_requests();
        }
        virtual_time::advance_to(until);
        self.turn(false);
        self.serve_window_requests();
    }

    /// The virtual clock's time: how far [`Self::advance_time`] has moved it
    /// since the mount (since the first mount, when another `TestApp` on this
    /// thread was alive then and shares its clock).
    pub fn elapsed(&self) -> Duration {
        virtual_time::elapsed().unwrap_or_default()
    }
}
