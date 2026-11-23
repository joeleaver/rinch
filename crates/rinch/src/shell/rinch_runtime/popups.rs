//! Popup windows (#2552): the shell's half. `rinch::popup` opens them and
//! keeps the chain (the parent relation) in the window registry; here they
//! are made against their parent, closed topmost first, and given the
//! input a menu chain takes.
//!
//! - **Made** ([`RinchRuntime::open_pending_popup`]): a content-sized app,
//!   mounted at its parent's scale with the parent's theme, measured, then
//!   made as a winit `WindowType::Popup` with the parent's raw handle, a
//!   positioner from its placement against the anchor
//!   (`shell::popup_positioner`), a grab for a menu (`with_active`), at the
//!   measured size. A 0x0 measurement is refused.
//! - **One menu chain at a time.** A menu whose parent is not the topmost
//!   menu (or the foot of the chain) closes the menus in its way
//!   ([`Dismissal::Replaced`]) and is made a turn later, once they have gone
//!   (the fork has no close-and-reopen in one loop iteration).
//! - **Closed** ([`RinchRuntime::close_popups_above`]): before any window
//!   closes or hides, every popup above it closes, topmost first, told
//!   [`Dismissal::ParentClosed`]. `popup_done` (`CloseRequested` on a popup)
//!   closes with no veto, told [`Dismissal::System`].
//! - **Input**: a press outside the popups above it closes them
//!   ([`Dismissal::PressOutside`]) and is delivered; keys reaching any
//!   window of a menu chain go to its topmost menu; the parent's
//!   `Focused(false)` while its chain holds the keyboard is ignored.
//! - **Placement**: `Moved` and `SurfaceResized` update
//!   [`WindowHandle::placement`](crate::WindowHandle::placement); a moved
//!   anchor re-places the popup (`request_popup_reposition`).

use super::*;
use crate::popup::{self, chain};
use crate::window_handle;
use rinch_core::element::{Dismissal, LayerKeyboard, PopupKind, PopupPlaced};

/// How long a menu with `keyboard_via_parent` waits for its parent's
/// keyboard before it is made without (ms).
const KEYBOARD_WAIT_MS: u32 = 300;

/// The shell's popup state beside its windows.
#[derive(Default)]
pub(super) struct PopupState {
    /// A popup closed in this loop iteration: a new menu waits for the next
    /// (winit cannot close and open a popup in one iteration).
    closed_this_turn: bool,
    /// Popups waiting for the next iteration (see above), by id.
    deferred: Vec<u64>,
    /// Menus waiting for their parent layer surface to get the keyboard
    /// (`keyboard_via_parent`): the parent's id and the popup.
    pub(super) awaiting_keyboard: Vec<(u64, window_handle::Pending)>,
    /// Layer surfaces whose keyboard interactivity a chain raised, by window
    /// id, with what to restore when the chain closes.
    pub(super) raised: Vec<(u64, LayerKeyboard)>,
    /// Windows whose `Focused(false)` was held while their chain had a menu
    /// open (KWin moving the keyboard to the menu), by id: told once the
    /// chain has closed, unless the keyboard came back first.
    pub(super) held_blur: Vec<u64>,
}

impl RinchRuntime {
    /// The role of window `id`: the main window or another, while it has an
    /// OS window. `None` for one not made, closed, or `0`.
    pub(super) fn role_of_window_id(&self, id: u64) -> Option<WindowRole> {
        if id != 0 && id == self.main_handle && !self.main_closed {
            return Some(WindowRole::Main);
        }
        self.others
            .iter()
            .any(|w| w.handle == id)
            .then_some(WindowRole::Other(id))
    }

    /// The window id of `role` (`None` for DevTools).
    pub(super) fn window_id_of(&self, role: WindowRole) -> Option<u64> {
        match role {
            WindowRole::Main => Some(self.main_handle),
            WindowRole::Other(id) => Some(id),
            WindowRole::DevTools => None,
        }
    }

    /// Make popup `id` (its `OpenWindow`), if it is still waiting.
    pub(super) fn open_pending_popup(&mut self, id: u64, event_loop: &dyn ActiveEventLoop) {
        // A menu still waiting for its parent's keyboard when the wait ran
        // out (a seat with no keyboard never sends one): made now, keys or
        // not.
        if let Some(at) = self
            .popups
            .awaiting_keyboard
            .iter()
            .position(|(_, p)| p.id == id)
        {
            let (parent, pending) = self.popups.awaiting_keyboard.remove(at);
            tracing::debug!("popup {id}: its parent's keyboard did not arrive; made without");
            match self.role_of_window_id(parent) {
                Some(role) => self.make_popup(pending, parent, role, event_loop),
                None => Self::popup_failed(id, "its parent window closed"),
            }
            // A menu that could not be made leaves no parent raised.
            self.restore_raised_keyboards();
            return;
        }
        let Some((parent, kind, via_parent)) = window_handle::with_popup(id, |p| {
            (p.parent, p.props.kind, p.props.keyboard_via_parent)
        }) else {
            return;
        };
        // Made already, or closed: an `OpenWindow` that came late (the
        // keyboard wait's) asks nothing.
        if !window_handle::is_pending(id) {
            return;
        }
        if kind == PopupKind::Menu {
            let links = window_handle::popups();
            let in_the_way: Vec<u64> = chain::press_closes(parent, &links)
                .into_iter()
                .filter(|&m| m != id)
                .collect();
            for m in in_the_way {
                window_handle::note_dismissal(m, Dismissal::Replaced);
                self.close_other(m);
            }
        }
        if self.popups.closed_this_turn {
            if !self.popups.deferred.contains(&id) {
                self.popups.deferred.push(id);
            }
            return;
        }
        let Some(pending) = window_handle::take_pending(id) else {
            return;
        };
        let Some(parent_role) = self.role_of_window_id(parent) else {
            Self::popup_failed(id, "its parent window is not open");
            return;
        };
        if self.w(parent_role).window.is_none() {
            Self::popup_failed(id, "its parent window is hidden");
            return;
        }
        // The keyboard through the parent layer surface (wlroots): raised,
        // and the popup made once the keyboard has arrived. Never while a
        // grab is held: only for the first menu of a chain.
        if kind == PopupKind::Menu
            && via_parent
            && self.raise_parent_keyboard(id, parent, parent_role)
        {
            self.popups.awaiting_keyboard.push((parent, pending));
            // Not forever: a seat with no keyboard never gives it.
            rinch_core::reactive::unowned(|| {
                rinch_core::set_timeout(KEYBOARD_WAIT_MS, move || {
                    send_native_event_for(EventFor::Window(id), RinchNativeEvent::OpenWindow(id));
                })
            });
            return;
        }
        self.make_popup(pending, parent, parent_role, event_loop);
    }

    /// Popup `id` could not be made: it closes, told why.
    fn popup_failed(id: u64, why: &str) {
        tracing::warn!("a popup could not open: {why}");
        window_handle::note_dismissal(id, Dismissal::Failed(why.to_string()));
        crate::window_perf::closed(id);
        window_handle::note_closed(id);
    }

    /// Raise the parent layer surface's keyboard interactivity to
    /// `exclusive` for a menu with `keyboard_via_parent`. Answers whether the
    /// popup is to wait for the keyboard: not when the parent is no layer
    /// surface, has the keyboard already, or a menu of its chain is open.
    fn raise_parent_keyboard(&mut self, id: u64, parent: u64, parent_role: WindowRole) -> bool {
        if !chain::may_raise_keyboard(id, parent, &window_handle::popups()) {
            return false;
        }
        let w = self.w(parent_role);
        if w.layer.is_none() || w.app.window_focused {
            return false;
        }
        let had = w.layer_spec(false).surface.keyboard;
        if had == LayerKeyboard::Exclusive {
            return false;
        }
        if !set_layer_keyboard(w, LayerKeyboard::Exclusive) {
            return false;
        }
        tracing::debug!("popups: window {parent}'s keyboard raised from {had:?}");
        if !self.popups.raised.iter().any(|r| r.0 == parent) {
            self.popups.raised.push((parent, had));
        }
        true
    }

    /// The parent `parent` got the keyboard: the menus waiting for it are
    /// made now.
    pub(super) fn parent_got_keyboard(&mut self, parent: u64, event_loop: &dyn ActiveEventLoop) {
        tracing::debug!("popups: window {parent} has the keyboard");
        let (ready, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.popups.awaiting_keyboard)
            .into_iter()
            .partition(|(p, _)| *p == parent);
        self.popups.awaiting_keyboard = rest;
        for (_, pending) in ready {
            let id = pending.id;
            match self.role_of_window_id(parent) {
                Some(role) => self.make_popup(pending, parent, role, event_loop),
                None => Self::popup_failed(id, "its parent window closed"),
            }
        }
        self.restore_raised_keyboards();
    }

    /// Give the layer surfaces whose chains closed the keyboard
    /// interactivity they had: the compositor gives the keyboard back to
    /// the window that had it (sway and labwc do).
    pub(super) fn restore_raised_keyboards(&mut self) {
        let links = window_handle::popups();
        let raised = std::mem::take(&mut self.popups.raised);
        for (parent, had) in raised {
            let waiting = self
                .popups
                .awaiting_keyboard
                .iter()
                .any(|(p, _)| *p == parent);
            if chain::topmost_menu(parent, &links).is_some() || waiting {
                self.popups.raised.push((parent, had));
                continue;
            }
            if let Some(role) = self.role_of_window_id(parent) {
                tracing::debug!("popups: window {parent}'s keyboard back to {had:?}");
                set_layer_keyboard(self.w(role), had);
            }
        }
    }

    /// Make `pending`, a popup on `parent` (`parent_role`'s window).
    fn make_popup(
        &mut self,
        mut pending: window_handle::Pending,
        parent: u64,
        parent_role: WindowRole,
        event_loop: &dyn ActiveEventLoop,
    ) {
        let id = pending.id;
        let Some((props, asked_at)) =
            window_handle::with_popup(id, |p| (p.props.clone(), p.asked_at))
        else {
            return;
        };
        let parent_window = match self.w(parent_role).window.as_ref() {
            Some(w) => w.window.clone(),
            None => return Self::popup_failed(id, "its parent window is hidden"),
        };
        // Up to the parent's output when the app set no largest size.
        if props.max_size.is_none() {
            if let Some((w, h)) = output_size_of(&*parent_window) {
                let zoom = crate::zoom::current();
                if let Some(sizing) = pending.props.size_to_content.as_mut() {
                    sizing.max_width = Some((w / zoom).max(1.0) as u32);
                    sizing.max_height = Some((h / zoom).max(1.0) as u32);
                }
            }
        }
        let mut window = Self::new_other_window(pending, &self.fonts);
        // The parent's theme (a document of the shell follows the process's
        // theme; one with its own CSS passes it on).
        #[cfg(feature = "theme")]
        window
            .app
            .set_owned_theme_css(self.w(parent_role).app.owned_theme_css.clone());
        // At the parent's scale from the first frame, fractional included.
        let scale = self.w(parent_role).app.device_pixel_ratio;
        window_handle::mounting(id, || window.mount_before_window(scale));
        window_handle::note_mounted(id, window.app.doc_key());
        window.register_perf(false);
        crate::window_perf::popup_opened(id, props.kind, parent);
        let renders_gpu = self.w(parent_role).has_gpu_renderer();
        match window.create_popup_window(event_loop, &*parent_window, &props, renders_gpu) {
            Ok(rect) => {
                window_handle::with_popup(id, |p| p.sent_anchor = Some(rect));
            }
            Err(why) => {
                tracing::warn!("a popup could not open: {why}");
                window_handle::note_dismissal(id, Dismissal::Failed(why));
                Self::close_made(window);
                return;
            }
        }
        window.popup_first_frame = Some(asked_at);
        self.others.push(window);
        let role = WindowRole::Other(id);
        // A menu has the keyboard (forwarded to it on wlroots): its
        // document is focused from the start.
        if props.kind == PopupKind::Menu {
            self.dispatch_event(role, PlatformEvent::WindowFocus(true), event_loop);
        }
        self.about_to_wait_for(role, event_loop);
    }

    /// Close every popup above window `id`, topmost first, each told
    /// [`Dismissal::ParentClosed`] unless it already has a cause: what goes
    /// before any window closes or hides.
    pub(super) fn close_popups_above(&mut self, id: u64) {
        let links = window_handle::popups();
        for child in chain::close_order(id, &links) {
            window_handle::note_dismissal(child, Dismissal::ParentClosed);
            self.close_one_other(child);
        }
        // Menus waiting for this window's keyboard never open.
        let (gone, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.popups.awaiting_keyboard)
            .into_iter()
            .partition(|(p, _)| *p == id);
        self.popups.awaiting_keyboard = rest;
        for (_, pending) in gone {
            window_handle::note_dismissal(pending.id, Dismissal::ParentClosed);
            crate::window_perf::closed(pending.id);
            window_handle::note_closed(pending.id);
        }
    }

    /// A popup was closed: note it for this iteration, and give back a
    /// raised keyboard whose chain is gone.
    pub(super) fn note_popup_closed(&mut self) {
        self.popups.closed_this_turn = true;
        self.restore_raised_keyboards();
    }

    /// The end of a loop iteration: the popups that waited for it are asked
    /// for again, made in the next.
    pub(super) fn popups_end_of_turn(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.popups.closed_this_turn = false;
        // A blur held while a menu chain was open, told now that it has
        // closed and the keyboard did not come back.
        let held = std::mem::take(&mut self.popups.held_blur);
        for id in held {
            let Some(role) = self.role_of_window_id(id) else {
                continue;
            };
            if self.in_menu_chain(role) {
                self.popups.held_blur.push(id);
            } else {
                self.window_event_for(role, event_loop, WindowEvent::Focused(false));
            }
        }
        for id in std::mem::take(&mut self.popups.deferred) {
            send_native_event_for(EventFor::Window(id), RinchNativeEvent::OpenWindow(id));
        }
    }

    /// A press reached `role`'s window: the menus it closes are closed at
    /// the end of the handler, after the press is delivered.
    pub(super) fn light_dismiss_press(&mut self, role: WindowRole) {
        let Some(id) = self.window_id_of(role) else {
            return;
        };
        let point = self.w(role).app.cursor_pos;
        for menu in popup::light_dismiss(id, point) {
            self.close_soon(WindowRole::Other(menu));
        }
    }

    /// Where an input event for `role` goes: a key, IME or modifier event
    /// reaching any window of a menu chain goes to its topmost menu.
    pub(super) fn input_role(&mut self, role: WindowRole, event: &WindowEvent) -> WindowRole {
        let keyboard = matches!(
            event,
            WindowEvent::KeyboardInput { .. }
                | WindowEvent::Ime(_)
                | WindowEvent::ModifiersChanged(_)
        );
        if !keyboard {
            return role;
        }
        let Some(id) = self.window_id_of(role) else {
            return role;
        };
        let links = window_handle::popups();
        let Some(target) = chain::key_target(id, &links) else {
            return role;
        };
        if target == id || !self.others.iter().any(|w| w.handle == target) {
            return role;
        }
        // The window it reached keeps the modifiers too, for after the chain.
        if let WindowEvent::ModifiersChanged(m) = event {
            self.w(role).modifiers = m.state();
        }
        WindowRole::Other(target)
    }

    /// `role`'s window lost the keyboard: held, answering `true`, while its
    /// chain has a menu open (KWin moving the keyboard to the menu), and
    /// told when the chain closes ([`Self::popups_end_of_turn`]); a gain
    /// forgets a held loss.
    pub(super) fn hold_focus_change(&mut self, role: WindowRole, focused: bool) -> bool {
        let Some(id) = self.window_id_of(role) else {
            return false;
        };
        if focused {
            self.popups.held_blur.retain(|&h| h != id);
            return false;
        }
        if !self.in_menu_chain(role) {
            return false;
        }
        if !self.popups.held_blur.contains(&id) {
            self.popups.held_blur.push(id);
        }
        true
    }

    /// Whether `role`'s window is in a chain with a menu open.
    pub(super) fn in_menu_chain(&self, role: WindowRole) -> bool {
        let Some(id) = self.window_id_of(role) else {
            return false;
        };
        chain::key_target(id, &window_handle::popups()).is_some()
    }

    /// The system dismissed `role`'s popup (`popup_done`): it closes with no
    /// veto. Answers whether it was a popup.
    pub(super) fn popup_done(&mut self, role: WindowRole) -> bool {
        let WindowRole::Other(id) = role else {
            return false;
        };
        if window_handle::with_popup(id, |_| ()).is_none() {
            return false;
        }
        window_handle::note_dismissal(id, Dismissal::System);
        self.close_soon(role);
        true
    }

    /// `role`'s popup moved or was resized: its placement as the system
    /// made it.
    pub(super) fn popup_placed(&mut self, role: WindowRole, moved: Option<(i32, i32)>) {
        let WindowRole::Other(id) = role else {
            return;
        };
        let Some((anchor, placement, signal)) =
            window_handle::with_popup(id, |p| (p.sent_anchor, p.props.placement, p.placed))
        else {
            return;
        };
        let (Some(anchor), Some(signal)) = (anchor, signal) else {
            return;
        };
        let w = self.w(role);
        let Some(window) = w.window.as_ref() else {
            return;
        };
        let os = window.os_scale_factor();
        let size = window.raw().surface_size();
        let size = (f64::from(size.width) / os, f64::from(size.height) / os);
        let old = rinch_core::reactive::untracked(|| signal.get());
        let pos = match (moved, old) {
            (Some((x, y)), _) => (f64::from(x) / os, f64::from(y) / os),
            (None, Some(old)) => (old.x, old.y),
            (None, None) => return,
        };
        let placed: PopupPlaced = popup::placed_from(anchor, placement, pos, size);
        if old != Some(placed) {
            signal.set(Some(placed));
        }
    }

    /// Each turn, a popup anchored to an element that moved is placed again
    /// against where it is now.
    pub(super) fn follow_popup_anchors(&mut self) {
        for (id, parent, _) in window_handle::popups() {
            let Some(role) = self.role_of_window_id(id) else {
                continue;
            };
            let Some(parent_role) = self.role_of_window_id(parent) else {
                continue;
            };
            let Some(parent_size) = self.w(parent_role).window.as_ref().map(|w| {
                let s = w.raw().surface_size();
                let os = w.os_scale_factor();
                (f64::from(s.width) / os, f64::from(s.height) / os)
            }) else {
                continue;
            };
            let Some((props, sent)) =
                window_handle::with_popup(id, |p| (p.props.clone(), p.sent_anchor))
            else {
                continue;
            };
            let Some(rect) = popup::anchor_rect_for(&props, crate::zoom::current(), parent_size)
            else {
                continue;
            };
            if sent == Some(rect) {
                continue;
            }
            let positioner = crate::shell::popup_positioner::positioner(
                props.placement,
                props.adjust,
                rect,
                props.offset,
            );
            let Some(window) = self.w(role).window.as_ref() else {
                continue;
            };
            if reposition(window.raw(), positioner) {
                window_handle::with_popup(id, |p| p.sent_anchor = Some(rect));
            }
        }
    }
}

impl ShellWindow {
    /// Whether this window presents on the GPU: a popup on it does too.
    pub(super) fn has_gpu_renderer(&self) -> bool {
        #[cfg(feature = "gpu-renderer")]
        {
            self.renderer.is_some()
        }
        #[cfg(not(feature = "gpu-renderer"))]
        {
            false
        }
    }

    /// Make this popup's OS window on `parent`, its document mounted and
    /// measured: answers the anchor rectangle it was placed against.
    pub(super) fn create_popup_window(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        parent: &dyn winit::window::Window,
        props: &crate::popup::PopupProps,
        renders_gpu: bool,
    ) -> Result<[i32; 4], String> {
        let zoom = crate::zoom::current();
        let os = parent.scale_factor();
        let parent_size = parent.surface_size();
        let parent_logical = (
            f64::from(parent_size.width) / os,
            f64::from(parent_size.height) / os,
        );
        let rect = crate::popup::anchor_rect_for(props, zoom, parent_logical)
            .ok_or("its anchor is not laid out")?;
        let (width, height) = self
            .start_surface_negotiation()
            .ok_or("its content could not be measured")?;
        if width == 0 || height == 0 {
            return Err(format!("its content measured {width}x{height}"));
        }
        let positioner = crate::shell::popup_positioner::positioner(
            props.placement,
            props.adjust,
            rect,
            props.offset,
        );
        let raw = parent
            .rwh_06_window_handle()
            .window_handle()
            .map_err(|e| e.to_string())?
            .as_raw();
        let attrs = WindowAttributes::default()
            .with_title(&self.title)
            .with_window_type(winit::window::WindowType::Popup)
            .with_positioner(positioner)
            .with_surface_size(LogicalSize::new(
                f64::from(width) * zoom,
                f64::from(height) * zoom,
            ))
            .with_active(props.kind == PopupKind::Menu)
            .with_decorations(false)
            .with_resizable(false)
            .with_transparent(props.transparent);
        // SAFETY: the parent window outlives the popup: the shell closes
        // every popup above a window before that window goes.
        let attrs = unsafe { attrs.with_parent_window(Some(raw)) };
        let window = event_loop.create_window(attrs).map_err(|e| e.to_string())?;
        self.shown_once = true;

        // The app's renderer: the GPU when its parent presents there.
        #[cfg(feature = "gpu-renderer")]
        let mut renderer_built = false;
        #[cfg(feature = "gpu-renderer")]
        if renders_gpu {
            let transparent = self.gpu_surface_transparent();
            let (renderer, built) = self.start_gpu_renderer(&*window, transparent);
            renderer_built = built;
            self.renderer_started(renderer);
        }
        #[cfg(not(feature = "gpu-renderer"))]
        let _ = renders_gpu;

        let size = window.surface_size();
        let scale = crate::zoom::apply(window.scale_factor());
        let logical = to_logical((size.width, size.height), scale);
        self.app
            .set_surface_size((size.width.max(1), size.height.max(1)));
        let physical = (size.width, size.height);
        let _ = self
            .app
            .handle_event(PlatformEvent::ScaleFactorChanged(scale), physical, scale);
        self.app
            .set_granted_size(logical.0 as f32, logical.1 as f32);
        let _ = self.app.handle_event(
            PlatformEvent::Resized {
                width: logical.0,
                height: logical.1,
            },
            physical,
            scale,
        );
        #[cfg(feature = "gpu-renderer")]
        self.note_renderer_built(renderer_built);
        self.facts_key = Some(self.app.doc_key());
        self.window = Some(WinitWindow::new(window));
        let window_arc = self.window.as_ref().unwrap().window.clone();
        crate::render_surface::set_redraw_callback(
            self.facts_key,
            Arc::new(move || {
                window_arc.request_redraw();
            }),
        );
        // A tooltip takes no pointer input: its empty region goes with its
        // first frame.
        self.input_region = self
            .app
            .window_props
            .as_ref()
            .map_or(Default::default(), |p| p.input_region);
        self.send_input_region(false);
        self.window.as_ref().unwrap().request_redraw();
        Ok(rect)
    }

    /// A popup's first frame was presented: under `RINCH_PERF`, how long
    /// after the app asked for it.
    pub(super) fn note_popup_frame(&mut self) {
        if let Some(asked) = self.popup_first_frame.take() {
            crate::window_perf::popup_first_frame(self.handle, asked.elapsed());
        }
    }
}

/// The logical size of the output `window` is on, as `rinch::outputs` lists
/// it, else as winit's monitor reports it.
fn output_size_of(window: &dyn winit::window::Window) -> Option<(f64, f64)> {
    let monitor = window.current_monitor()?;
    let id = crate::outputs::OutputId::from_raw(monitor.native_id());
    let outputs = rinch_core::reactive::untracked(crate::outputs::outputs);
    if let Some(o) = outputs.iter().find(|o| o.id == id) {
        return Some(o.size);
    }
    let mode = monitor.current_video_mode()?;
    let scale = monitor.scale_factor();
    Some((
        f64::from(mode.size().width) / scale,
        f64::from(mode.size().height) / scale,
    ))
}

/// Set `window`'s layer keyboard interactivity (Wayland). Answers whether it
/// could be set.
fn set_layer_keyboard(window: &ShellWindow, keyboard: LayerKeyboard) -> bool {
    #[cfg(target_os = "linux")]
    if let Some(w) = &window.window {
        use winit::platform::wayland::WindowExtWayland;
        w.raw()
            .set_layer_shell_keyboard_interactivity(crate::shell::layer_shell::winit_keyboard(
                keyboard,
            ));
        return true;
    }
    let _ = (window, keyboard);
    false
}

/// Re-place the popup `window` with `positioner`. Answers whether it was
/// asked.
fn reposition(
    window: &dyn winit::window::Window,
    positioner: winit::window::WindowPositioner,
) -> bool {
    #[cfg(target_os = "linux")]
    {
        use winit::platform::wayland::WindowExtWayland;
        match window.request_popup_reposition(Some(positioner), None) {
            Ok(_) => return true,
            Err(e) => tracing::debug!("a popup could not be placed again: {e}"),
        }
    }
    let _ = (window, positioner);
    false
}
