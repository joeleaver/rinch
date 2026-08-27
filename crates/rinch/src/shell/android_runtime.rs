//! Android runtime using android-activity (NativeActivity) + softbuffer.
//!
//! Bypasses winit entirely for direct control over the Android Activity
//! lifecycle, touch input, and surface management. Uses the same
//! platform-agnostic [`RinchApp`] as the desktop shell.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use android_activity::InputStatus;
use android_activity::input::{KeyAction, KeyMapChar, MotionAction};
use android_activity::{AndroidApp, MainEvent, PollEvent};

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_core::events;
use rinch_platform::{AppAction, ImeEvent, KeyCode, Modifiers, MouseButton, PlatformEvent};

use crate::app::RinchApp;
use crate::shell::android_ime::{ImeAction, ImeComposition};

// ── Cross-thread dispatch ────────────────────────────────────────────────────

static MAIN_QUEUE: Mutex<Vec<Box<dyn FnOnce() + Send>>> = Mutex::new(Vec::new());
static REDRAW_PENDING: AtomicBool = AtomicBool::new(false);

fn dispatch_to_main_thread(f: Box<dyn FnOnce() + Send>) {
    MAIN_QUEUE.lock().unwrap().push(f);
    REDRAW_PENDING.store(true, Ordering::Release);
}

fn drain_main_queue() {
    let callbacks: Vec<Box<dyn FnOnce() + Send>> = MAIN_QUEUE.lock().unwrap().drain(..).collect();
    for cb in callbacks {
        cb();
    }
}

// ── Entry points ─────────────────────────────────────────────────────────────

/// Run a rinch application on Android with default theme.
///
/// Call this from your `android_main(app)` entry point:
///
/// ```ignore
/// use android_activity::AndroidApp;
///
/// #[no_mangle]
/// fn android_main(app: AndroidApp) {
///     rinch::shell::android_runtime::run(app, "My App", 0, 0, my_component);
/// }
/// ```
pub fn run<F>(android_app: AndroidApp, _title: &str, _width: u32, _height: u32, component: F)
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    #[cfg(feature = "theme")]
    {
        crate::setup_theme_css(&ThemeProviderProps::default());
    }

    let app = RinchApp::new(component);
    run_loop(android_app, app);
}

/// Run a rinch application on Android with theme configuration.
pub fn run_with_theme<F>(
    android_app: AndroidApp,
    _title: &str,
    _width: u32,
    _height: u32,
    component: F,
    theme: ThemeProviderProps,
) where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    crate::setup_theme_css(&theme);
    let app = RinchApp::new(component);
    run_loop(android_app, app);
}

// ── Main loop ────────────────────────────────────────────────────────────────

fn run_loop(android_app: AndroidApp, mut app: RinchApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("rinch"),
    );

    rinch_core::register_main_thread();
    rinch_core::set_cross_thread_dispatcher(dispatch_to_main_thread);
    rinch_core::set_on_signal_change(|| {
        REDRAW_PENDING.store(true, Ordering::Release);
    });

    rinch_android::init(&android_app);

    // #[cfg(feature = "android-gpu")]
    // gpu_diagnostic::run_tests();

    events::clear_handlers();
    rinch_core::clear_context();

    #[cfg(feature = "android-gpu")]
    let mut gpu_surface: Option<GpuSurface> = None;
    let mut surface: Option<SoftSurface> = None;
    let mut mounted = false;
    let mut physical_size = (0u32, 0u32);
    let mut scale_factor = 1.0f64;
    let mut running = true;
    let mut gesture = TouchGesture::new();
    let mut combining_accent: Option<char> = None;
    let mut keyboard_visible = false;
    // The kind of Enter key the keyboard is currently showing. Mirrors what
    // the Java side was last told, so the input session is only restarted when
    // the focused field's kind actually changes — see `ime::set_multiline`.
    let mut keyboard_multiline = false;
    // The IME's composing region, and the field it belongs to. See
    // `shell::android_ime` for why the region is mirrored on this side at all.
    let mut composition = ImeComposition::new();
    let mut composing_field: Option<usize> = None;
    let mut backgrounded = false;

    while running {
        android_app.poll_events(Some(Duration::from_millis(16)), |event| match event {
            PollEvent::Main(main_event) => match main_event {
                MainEvent::InitWindow { .. } => {
                    if let Some(native_window) = android_app.native_window() {
                        let w = native_window.width() as u32;
                        let h = native_window.height() as u32;
                        physical_size = (w, h);

                        let dpi = rinch_android::display::density_dpi().unwrap_or_else(|| {
                            let config = android_app.config();
                            config.density().unwrap_or(160) as i32
                        }) as f64;
                        scale_factor = dpi / 160.0;

                        log::info!(
                            "InitWindow: {}x{} physical, density={dpi}, scale={scale_factor:.2}",
                            w,
                            h
                        );

                        surface = SoftSurface::new(&native_window, w, h);
                        #[cfg(feature = "android-gpu")]
                        {
                            gpu_surface = GpuSurface::new(w, h);
                        }

                        if !mounted {
                            let (lw, lh) = rinch_platform::to_logical((w, h), scale_factor);
                            app.set_text_scale(scale_factor as f32);
                            app.mount_component(lw as f32, lh as f32);
                            mounted = true;
                        }

                        REDRAW_PENDING.store(true, Ordering::Release);
                    }
                }
                MainEvent::TerminateWindow { .. } => {
                    #[cfg(feature = "android-gpu")]
                    {
                        gpu_surface = None;
                    }
                    surface = None;
                }
                MainEvent::WindowResized { .. } => {
                    if let Some(native_window) = android_app.native_window() {
                        let w = native_window.width() as u32;
                        let h = native_window.height() as u32;
                        physical_size = (w, h);

                        let (lw, lh) = rinch_platform::to_logical((w, h), scale_factor);

                        // The `Resized` payload is the *logical* viewport layout
                        // is resolved at; `handle_event`'s `window_size` is the
                        // *physical* surface size it derives that viewport from
                        // (see `RinchApp::layout_viewport`). Passing the logical
                        // size for both divided by the scale factor twice.
                        let actions = app.handle_event(
                            PlatformEvent::Resized {
                                width: lw,
                                height: lh,
                            },
                            physical_size,
                            scale_factor,
                        );
                        process_actions(&actions, &mut running);

                        if let Some(ref mut s) = surface {
                            s.resize(w, h);
                        }
                        #[cfg(feature = "android-gpu")]
                        if let Some(ref mut s) = gpu_surface {
                            s.resize(w, h);
                        }

                        REDRAW_PENDING.store(true, Ordering::Release);
                    }
                }
                MainEvent::Resume { .. } => {
                    rinch_android::lifecycle::notify_resumed();
                }
                MainEvent::Pause => {
                    rinch_android::lifecycle::notify_paused();
                    // Flushed below, where the viewport this loop dispatches
                    // against is in scope.
                    backgrounded = true;
                }
                MainEvent::Destroy => {
                    running = false;
                }
                _ => {}
            },
            PollEvent::Wake => {}
            _ => {}
        });

        if !running {
            break;
        }

        // No surface — wait for InitWindow. The composition is flushed first:
        // `MainEvent::Pause` and the `TerminateWindow` that drops the surface
        // can arrive in one `poll_events`, and skipping the flush here would
        // throw away the very half-typed word it exists to save — and leave
        // `backgrounded` latched until the next frame that has a surface,
        // which is after the resume.
        if surface.is_none() {
            if std::mem::take(&mut backgrounded) {
                for action in composition.finish_composing_text() {
                    apply_ime_action(&mut app, action, physical_size, scale_factor, &mut running);
                }
            }
            continue;
        }

        // Process touch / key input — must drain after poll_events returns
        let logical_size = rinch_platform::to_logical(physical_size, scale_factor);
        let input_events = collect_input_events(
            &android_app,
            &mut gesture,
            scale_factor,
            &mut combining_accent,
        );
        for event in &input_events {
            let actions = app.handle_event(event.clone(), physical_size, scale_factor);
            process_actions(&actions, &mut running);
        }

        // What Enter means in the focused field, pushed before the keyboard is
        // raised so the session it opens already draws the right key. A
        // `<textarea>` wants a newline key; everything else wants what it had.
        // Android only reads this when a session starts, and rinch's own
        // field-to-field moves are invisible to it, so the change has to be
        // announced here or not at all.
        let needs_multiline = app.focused_input_is_multiline();
        if needs_multiline != keyboard_multiline
            && rinch_android::ime::set_multiline(needs_multiline)
        {
            // Advance the mirror only for a push that landed: a dropped one
            // would leave it claiming a kind the keyboard was never told
            // about, and nothing else ever pushes this. A failure retries on
            // the next turn of the loop.
            keyboard_multiline = needs_multiline;
        }

        // Show/hide soft keyboard based on input focus
        let needs_keyboard = app.has_focused_input() || app.has_focused_contenteditable();
        if needs_keyboard != keyboard_visible {
            keyboard_visible = needs_keyboard;
            if needs_keyboard {
                rinch_android::ime::show_keyboard();
            } else {
                rinch_android::ime::hide_keyboard();
            }
        }

        // What the IME did since the last frame, in the order it did it. Taken
        // *before* the focus check below, which needs to be able to throw the
        // batch away: every call in it was made against the field that was
        // focused when the keyboard made it.
        let mut updates = rinch_android::ime::drain_updates();

        // The focused field changed while the IME was composing into the old
        // one. The focus arbiter has already committed that composition into
        // the field that lost focus (`set_focus_target_deferred`'s
        // compositionend-before-blur), so nothing is inserted here — but the
        // keyboard still believes it is composing, and would deliver the same
        // word again into the field that just gained focus. Restarting its
        // input session is the only way to say so; Android cannot see the move
        // itself, because one `RinchInputView` holds focus throughout.
        let focused_field = app.focused_input_node();
        if focused_field != composing_field {
            composing_field = focused_field;
            if composition.abandon() {
                // Emptying the mirror is not enough on its own: the touch that
                // moved focus was dispatched earlier in *this* iteration, so
                // anything the keyboard queued before it is still in hand and
                // would now be applied to the field that just gained focus — a
                // `setComposingText` re-opening the region there, and the
                // `finishComposingText` that `restart_input` provokes then
                // inserting the same word a second time, into the wrong field.
                // The whole batch belongs to the session that just ended.
                updates.clear();
                rinch_android::ime::restart_input();
            }
        }

        // `android_ime` turns the `InputConnection` call stream into the
        // actions below; it holds the composing region and every rule about
        // when one ends.
        for update in updates {
            let actions = match update {
                rinch_android::ime::ImeUpdate::SetComposingText {
                    text,
                    new_cursor_position,
                } => composition.set_composing_text(text, new_cursor_position),
                rinch_android::ime::ImeUpdate::FinishComposingText => {
                    composition.finish_composing_text()
                }
                rinch_android::ime::ImeUpdate::CommitText(text) => composition.commit_text(text),
                rinch_android::ime::ImeUpdate::DeleteSurroundingText { before, after } => {
                    composition.delete_surrounding_text(before, after)
                }
            };
            for action in actions {
                apply_ime_action(&mut app, action, physical_size, scale_factor, &mut running);
            }
        }

        // Backgrounding ends the IME session. Android finishes the composition
        // on the connection when it does, but this process is on its way to
        // being frozen and may not be scheduled to see it — so the composition
        // is committed here instead, and a half-typed word survives the way it
        // would in an `EditText`, where the composing characters are already
        // real text in the buffer. Emptying the mirror makes the framework's
        // own `finishComposingText` a no-op rather than a second insertion.
        if std::mem::take(&mut backgrounded) {
            for action in composition.finish_composing_text() {
                apply_ime_action(&mut app, action, physical_size, scale_factor, &mut running);
            }
        }

        // Drain activity result callbacks (file picker, etc.)
        rinch_android::callback::drain_activity_results();
        rinch_android::callback::drain_permission_results();

        // Drain sensor, location, and lifecycle updates
        rinch_android::sensors::drain_sensor_events();
        rinch_android::location::drain_location();
        rinch_android::lifecycle::drain_lifecycle();

        // Drain cross-thread callbacks
        drain_main_queue();
        rinch_core::reactive::drain_polls();

        // Check if app has pending layout (signal changes create pending updates)
        let has_momentum = gesture.velocity_x.abs() > MOMENTUM_MIN_VELOCITY
            || gesture.velocity_y.abs() > MOMENTUM_MIN_VELOCITY;
        let redraw = REDRAW_PENDING.swap(false, Ordering::AcqRel);
        let pending = app.has_pending_layout();
        let needs_paint = redraw || pending || has_momentum;

        if needs_paint && mounted {
            if pending {
                app.resolve_and_repaint(logical_size.0 as f32, logical_size.1 as f32);
            }

            #[cfg(feature = "android-gpu")]
            if let (Some(gpu), Some(soft)) = (&mut gpu_surface, &mut surface) {
                let scene = app.build_scene(scale_factor, logical_size);
                let (pixels, w, h) = gpu.render_to_pixels(scene);
                soft.present_pixels(pixels, w, h);
            }

            #[cfg(not(feature = "android-gpu"))]
            if let Some(ref mut s) = surface {
                let (pixels, w, h) = app.build_pixels(scale_factor, logical_size, false);
                s.present_pixels(pixels, w, h);
            }
        }
    }
}

// ── Touch gesture recognizer ─────────────────────────────────────────────────

const SCROLL_THRESHOLD: f32 = 8.0;
const MOMENTUM_FRICTION: f32 = 0.95;
const MOMENTUM_MIN_VELOCITY: f32 = 0.5;

enum TouchState {
    Idle,
    /// Finger down, hasn't moved past threshold yet.
    Pending {
        x: f32,
        y: f32,
    },
    /// Finger is dragging — emit scroll events.
    Scrolling {
        last_x: f32,
        last_y: f32,
    },
}

struct TouchGesture {
    state: TouchState,
    /// Velocity for momentum scrolling (pixels per frame).
    velocity_x: f32,
    velocity_y: f32,
    /// Where to send scroll events (the initial touch point).
    scroll_origin: (f32, f32),
}

impl TouchGesture {
    fn new() -> Self {
        Self {
            state: TouchState::Idle,
            velocity_x: 0.0,
            velocity_y: 0.0,
            scroll_origin: (0.0, 0.0),
        }
    }

    fn process(&mut self, action: MotionAction, x: f32, y: f32, events: &mut Vec<PlatformEvent>) {
        match action {
            MotionAction::Down => {
                self.velocity_x = 0.0;
                self.velocity_y = 0.0;
                self.scroll_origin = (x, y);
                self.state = TouchState::Pending { x, y };
                events.push(PlatformEvent::MouseMove { x, y });
            }
            MotionAction::Move => {
                match self.state {
                    TouchState::Pending {
                        x: start_x,
                        y: start_y,
                    } => {
                        let dx = x - start_x;
                        let dy = y - start_y;
                        if dx.abs() > SCROLL_THRESHOLD || dy.abs() > SCROLL_THRESHOLD {
                            // Crossed threshold — switch to scrolling
                            self.state = TouchState::Scrolling {
                                last_x: x,
                                last_y: y,
                            };
                        }
                    }
                    TouchState::Scrolling { last_x, last_y } => {
                        let delta_x = (x - last_x) as f64;
                        let delta_y = (y - last_y) as f64;
                        self.velocity_x = (x - last_x) * 0.8 + self.velocity_x * 0.2;
                        self.velocity_y = (y - last_y) * 0.8 + self.velocity_y * 0.2;
                        self.state = TouchState::Scrolling {
                            last_x: x,
                            last_y: y,
                        };
                        let (ox, oy) = self.scroll_origin;
                        events.push(PlatformEvent::MouseWheel {
                            x: ox,
                            y: oy,
                            delta_x,
                            delta_y,
                        });
                    }
                    TouchState::Idle => {
                        events.push(PlatformEvent::MouseMove { x, y });
                    }
                }
            }
            MotionAction::Up => {
                match self.state {
                    TouchState::Pending { x, y } => {
                        // Didn't exceed threshold — this was a tap
                        events.push(PlatformEvent::MouseDown {
                            x,
                            y,
                            button: MouseButton::Left,
                        });
                        events.push(PlatformEvent::MouseUp {
                            x,
                            y,
                            button: MouseButton::Left,
                        });
                    }
                    TouchState::Scrolling { .. } => {
                        // End of scroll drag — momentum will be applied in tick()
                    }
                    TouchState::Idle => {}
                }
                self.state = TouchState::Idle;
            }
            MotionAction::Cancel => {
                self.state = TouchState::Idle;
                self.velocity_x = 0.0;
                self.velocity_y = 0.0;
            }
            MotionAction::HoverMove => {
                events.push(PlatformEvent::MouseMove { x, y });
            }
            MotionAction::ButtonPress | MotionAction::ButtonRelease => {}
            _ => {}
        }
    }

    /// Generate momentum scroll events. Returns true if still animating.
    fn tick_momentum(&mut self, events: &mut Vec<PlatformEvent>) -> bool {
        if matches!(self.state, TouchState::Scrolling { .. }) {
            // Still touching — don't apply momentum
            return false;
        }
        if self.velocity_x.abs() < MOMENTUM_MIN_VELOCITY
            && self.velocity_y.abs() < MOMENTUM_MIN_VELOCITY
        {
            self.velocity_x = 0.0;
            self.velocity_y = 0.0;
            return false;
        }

        let (ox, oy) = self.scroll_origin;
        events.push(PlatformEvent::MouseWheel {
            x: ox,
            y: oy,
            delta_x: self.velocity_x as f64,
            delta_y: self.velocity_y as f64,
        });

        self.velocity_x *= MOMENTUM_FRICTION;
        self.velocity_y *= MOMENTUM_FRICTION;
        true
    }
}

// ── IME ──────────────────────────────────────────────────────────────────────

/// Apply one [`ImeAction`] from the composition translator.
///
/// A commit is still applied as per-character `KeyDown`s rather than as
/// `ImeEvent::Commit`. That path is the one that has been watched working on a
/// device, and composition does not need it changed: what a preedit requires is
/// that the composition be *cleared before* the text that replaces it arrives,
/// which the action list already orders. Switching commits to one
/// `ImeEvent::Commit` (one edit, better undo grouping) remains worth doing and
/// remains a separate change — it alters what every keystroke on Android does,
/// including the ones that never compose.
///
/// `window_size` is the **physical** surface size, the unit `handle_event`
/// takes — it derives the logical layout viewport from it itself
/// (`RinchApp::layout_viewport`). Handing it the logical size here would divide
/// by the scale factor twice and lay the page out that much too narrow on every
/// IME keystroke.
fn apply_ime_action(
    app: &mut RinchApp,
    action: ImeAction,
    window_size: (u32, u32),
    scale_factor: f64,
    running: &mut bool,
) {
    // What the focused field actually holds, as a bound on the delete loops
    // below. Deleting past either end of the buffer is a no-op per character,
    // and `deleteSurroundingText(Integer.MAX_VALUE, 0)` — a documented way for
    // an IME to clear a field — would otherwise spin two billion no-op edit
    // commands and wedge the frame.
    let field_len = if matches!(action, ImeAction::Delete { .. }) {
        app.focused_input_value.chars().count()
    } else {
        0
    };
    let mut dispatch = |event: PlatformEvent, running: &mut bool| {
        let actions = app.handle_event(event, window_size, scale_factor);
        process_actions(&actions, running);
    };
    match action {
        ImeAction::Preedit { text, cursor } => {
            dispatch(
                PlatformEvent::Ime(ImeEvent::Preedit { text, cursor }),
                running,
            );
        }
        ImeAction::Insert(text) => {
            for ch in text.chars() {
                dispatch(
                    PlatformEvent::KeyDown {
                        key: KeyCode::Other,
                        logical_key: None,
                        text: Some(ch.to_string()),
                        modifiers: Modifiers::default(),
                    },
                    running,
                );
            }
        }
        ImeAction::Delete { before, after } => {
            for _ in 0..before.min(field_len) {
                dispatch(
                    PlatformEvent::KeyDown {
                        key: KeyCode::Backspace,
                        logical_key: None,
                        text: None,
                        modifiers: Modifiers::default(),
                    },
                    running,
                );
            }
            for _ in 0..after.min(field_len) {
                dispatch(
                    PlatformEvent::KeyDown {
                        key: KeyCode::Delete,
                        logical_key: None,
                        text: None,
                        modifiers: Modifiers::default(),
                    },
                    running,
                );
            }
        }
    }
}

// ── Input translation ────────────────────────────────────────────────────────

fn collect_input_events(
    android_app: &AndroidApp,
    gesture: &mut TouchGesture,
    scale_factor: f64,
    combining_accent: &mut Option<char>,
) -> Vec<PlatformEvent> {
    let mut events = Vec::new();

    match android_app.input_events_iter() {
        Ok(mut iter) => {
            while iter.next(|input_event| {
                use android_activity::input::InputEvent;
                match input_event {
                    InputEvent::MotionEvent(motion) => {
                        let ptr = motion.pointer_at_index(0);
                        let x = (ptr.x() as f64 / scale_factor) as f32;
                        let y = (ptr.y() as f64 / scale_factor) as f32;
                        gesture.process(motion.action(), x, y, &mut events);
                    }
                    InputEvent::KeyEvent(key) => {
                        let meta = key.meta_state();
                        let modifiers = Modifiers {
                            ctrl: meta.ctrl_on(),
                            shift: meta.shift_on(),
                            alt: meta.alt_on(),
                            meta: false,
                        };

                        if key.action() == KeyAction::Down {
                            let text = android_app
                                .device_key_character_map(key.device_id())
                                .ok()
                                .and_then(|map| match map.get(key.key_code(), key.meta_state()) {
                                    Ok(KeyMapChar::Unicode(ch)) => {
                                        if let Some(accent) = combining_accent.take() {
                                            match map.get_dead_char(accent, ch) {
                                                Ok(Some(combined)) => Some(combined.to_string()),
                                                _ => Some(ch.to_string()),
                                            }
                                        } else {
                                            Some(ch.to_string())
                                        }
                                    }
                                    Ok(KeyMapChar::CombiningAccent(accent)) => {
                                        *combining_accent = Some(accent);
                                        None
                                    }
                                    _ => None,
                                });

                            if let Some(key_code) = map_android_keycode(key.key_code()) {
                                events.push(PlatformEvent::KeyDown {
                                    key: key_code,
                                    logical_key: None,
                                    text,
                                    modifiers,
                                });
                            } else if let Some(text) = text {
                                events.push(PlatformEvent::KeyDown {
                                    key: KeyCode::Other,
                                    logical_key: None,
                                    text: Some(text),
                                    modifiers,
                                });
                            }
                        }
                    }
                    _ => {}
                }
                InputStatus::Handled
            }) {}
        }
        Err(e) => {
            log::warn!("input_events_iter failed: {e:?}");
        }
    }

    // Apply momentum scrolling
    gesture.tick_momentum(&mut events);

    events
}

fn map_android_keycode(keycode: android_activity::input::Keycode) -> Option<KeyCode> {
    use android_activity::input::Keycode as AK;
    match keycode {
        AK::DpadUp => Some(KeyCode::ArrowUp),
        AK::DpadDown => Some(KeyCode::ArrowDown),
        AK::DpadLeft => Some(KeyCode::ArrowLeft),
        AK::DpadRight => Some(KeyCode::ArrowRight),
        AK::MoveHome => Some(KeyCode::Home),
        AK::MoveEnd => Some(KeyCode::End),
        AK::PageUp => Some(KeyCode::PageUp),
        AK::PageDown => Some(KeyCode::PageDown),
        AK::Enter | AK::NumpadEnter => Some(KeyCode::Enter),
        AK::Tab => Some(KeyCode::Tab),
        AK::Escape | AK::Back => Some(KeyCode::Escape),
        AK::Del => Some(KeyCode::Backspace),
        AK::ForwardDel => Some(KeyCode::Delete),
        AK::Space => Some(KeyCode::Space),
        AK::A => Some(KeyCode::KeyA),
        AK::B => Some(KeyCode::KeyB),
        AK::C => Some(KeyCode::KeyC),
        AK::D => Some(KeyCode::KeyD),
        AK::E => Some(KeyCode::KeyE),
        AK::F => Some(KeyCode::KeyF),
        AK::G => Some(KeyCode::KeyG),
        AK::H => Some(KeyCode::KeyH),
        AK::I => Some(KeyCode::KeyI),
        AK::J => Some(KeyCode::KeyJ),
        AK::K => Some(KeyCode::KeyK),
        AK::L => Some(KeyCode::KeyL),
        AK::M => Some(KeyCode::KeyM),
        AK::N => Some(KeyCode::KeyN),
        AK::O => Some(KeyCode::KeyO),
        AK::P => Some(KeyCode::KeyP),
        AK::Q => Some(KeyCode::KeyQ),
        AK::R => Some(KeyCode::KeyR),
        AK::S => Some(KeyCode::KeyS),
        AK::T => Some(KeyCode::KeyT),
        AK::U => Some(KeyCode::KeyU),
        AK::V => Some(KeyCode::KeyV),
        AK::W => Some(KeyCode::KeyW),
        AK::X => Some(KeyCode::KeyX),
        AK::Y => Some(KeyCode::KeyY),
        AK::Z => Some(KeyCode::KeyZ),
        AK::Keycode0 => Some(KeyCode::Digit0),
        AK::Keycode1 => Some(KeyCode::Digit1),
        AK::Keycode2 => Some(KeyCode::Digit2),
        AK::Keycode3 => Some(KeyCode::Digit3),
        AK::Keycode4 => Some(KeyCode::Digit4),
        AK::Keycode5 => Some(KeyCode::Digit5),
        AK::Keycode6 => Some(KeyCode::Digit6),
        AK::Keycode7 => Some(KeyCode::Digit7),
        AK::Keycode8 => Some(KeyCode::Digit8),
        AK::Keycode9 => Some(KeyCode::Digit9),
        AK::ShiftLeft => Some(KeyCode::ShiftLeft),
        AK::ShiftRight => Some(KeyCode::ShiftRight),
        AK::CtrlLeft => Some(KeyCode::ControlLeft),
        AK::CtrlRight => Some(KeyCode::ControlRight),
        AK::AltLeft => Some(KeyCode::AltLeft),
        AK::AltRight => Some(KeyCode::AltRight),
        _ => None,
    }
}

// ── Softbuffer surface wrapper ───────────────────────────────────────────────

/// Wrapper around `NativeWindow` that provides both `HasWindowHandle` and
/// `HasDisplayHandle` traits required by softbuffer.
struct AndroidWindow {
    native: ndk::native_window::NativeWindow,
}

impl raw_window_handle::HasWindowHandle for AndroidWindow {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        // Delegate to ndk's impl
        self.native.window_handle()
    }
}

impl raw_window_handle::HasDisplayHandle for AndroidWindow {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        Ok(unsafe {
            raw_window_handle::DisplayHandle::borrow_raw(
                raw_window_handle::RawDisplayHandle::Android(
                    raw_window_handle::AndroidDisplayHandle::new(),
                ),
            )
        })
    }
}

struct SoftSurface {
    surface: softbuffer::Surface<std::sync::Arc<AndroidWindow>, std::sync::Arc<AndroidWindow>>,
    width: u32,
    height: u32,
}

impl SoftSurface {
    fn new(window: &ndk::native_window::NativeWindow, width: u32, height: u32) -> Option<Self> {
        use std::sync::Arc;

        let wrapper = Arc::new(AndroidWindow {
            native: window.clone(),
        });
        let width = width.max(1);
        let height = height.max(1);

        let context = softbuffer::Context::new(wrapper.clone()).ok()?;
        let mut surface = softbuffer::Surface::new(&context, wrapper).ok()?;

        use std::num::NonZeroU32;
        surface
            .configure(
                NonZeroU32::new(width).unwrap(),
                NonZeroU32::new(height).unwrap(),
                softbuffer::AlphaMode::Opaque,
            )
            .ok()?;

        Some(Self {
            surface,
            width,
            height,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        self.width = width;
        self.height = height;

        use std::num::NonZeroU32;
        let _ = self.surface.configure(
            NonZeroU32::new(width).unwrap(),
            NonZeroU32::new(height).unwrap(),
            softbuffer::AlphaMode::Opaque,
        );
    }

    fn present_pixels(&mut self, pixels: &[u8], width: u32, height: u32) {
        if width != self.width || height != self.height {
            self.resize(width, height);
        }

        let mut buffer = match self.surface.next_buffer() {
            Ok(b) => b,
            Err(_) => return,
        };

        let w = width as usize;
        let src_stride = w * 4;
        for (y, row) in buffer.pixel_rows().enumerate() {
            let src_offset = y * src_stride;
            for x in 0..w.min(row.len()) {
                let base = src_offset + x * 4;
                if base + 3 < pixels.len() {
                    row[x] = softbuffer::Pixel::new_rgb(
                        pixels[base],
                        pixels[base + 1],
                        pixels[base + 2],
                    );
                }
            }
        }

        let _ = buffer.present();
    }
}

// ── GPU surface (wgpu + vello) ──────────────────────────────────────────

#[cfg(feature = "android-gpu")]
struct GpuSurface {
    renderer: vello::Renderer,
    render_texture: wgpu::Texture,
    readback_buffer: wgpu::Buffer,
    device: wgpu::Device,
    queue: wgpu::Queue,
    width: u32,
    height: u32,
}

#[cfg(feature = "android-gpu")]
impl GpuSurface {
    fn new(width: u32, height: u32) -> Option<Self> {
        let width = width.max(1);
        let height = height.max(1);

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            flags: wgpu::InstanceFlags::empty(),
            backend_options: wgpu::BackendOptions::from_env_or_default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        });

        let adapter =
            match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })) {
                Ok(a) => a,
                Err(e) => {
                    log::error!("GPU: adapter failed: {e}");
                    return None;
                }
            };

        let experimental = wgpu::Features::EXPERIMENTAL_RAY_QUERY
            | wgpu::Features::EXPERIMENTAL_MESH_SHADER
            | wgpu::Features::EXPERIMENTAL_RAY_HIT_VERTEX_RETURN
            | wgpu::Features::EXPERIMENTAL_MESH_SHADER_MULTIVIEW
            | wgpu::Features::EXPERIMENTAL_PASSTHROUGH_SHADERS;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("rinch-android-gpu"),
            required_features: adapter.features() - experimental,
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::default(),
            experimental_features: wgpu::ExperimentalFeatures::default(),
        }))
        .map_err(|e| log::error!("GPU: device failed: {e}"))
        .ok()?;

        device.set_device_lost_callback(|reason, msg| {
            log::error!("GPU DEVICE LOST: reason={reason:?} msg={msg}");
        });

        let render_texture = Self::make_texture(&device, width, height);
        let readback_buffer = Self::make_buffer(&device, width, height);

        let mut renderer = vello::Renderer::new(
            &device,
            vello::RendererOptions {
                antialiasing_support: vello::AaSupport::area_only(),
                use_cpu: true,
                num_init_threads: None,
                pipeline_cache: None,
            },
        )
        .ok()?;

        log::info!("GPU: {width}x{height} {:?}", adapter.get_info().backend);

        Some(Self {
            renderer,
            render_texture,
            readback_buffer,
            device,
            queue,
            width,
            height,
        })
    }

    fn make_texture(device: &wgpu::Device, w: u32, h: u32) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    fn make_buffer(device: &wgpu::Device, w: u32, h: u32) -> wgpu::Buffer {
        let row = (w * 4 + 255) & !255;
        device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        let (w, h) = (width.max(1), height.max(1));
        self.width = w;
        self.height = h;
        self.render_texture = Self::make_texture(&self.device, w, h);
        self.readback_buffer = Self::make_buffer(&self.device, w, h);
    }

    fn render_to_pixels(&mut self, scene: &vello::Scene) -> (&[u8], u32, u32) {
        let view = self
            .render_texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        if let Err(e) = self.renderer.render_to_texture(
            &self.device,
            &self.queue,
            scene,
            &view,
            &vello::RenderParams {
                base_color: peniko::Color::from_rgba8(255, 255, 255, 255),
                width: self.width,
                height: self.height,
                antialiasing_method: vello::AaConfig::Area,
            },
        ) {
            log::error!("GPU render failed: {e}");
        }

        let row = (self.width * 4 + 255) & !255;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_texture_to_buffer(
            self.render_texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(std::iter::once(encoder.finish()));

        let slice = self.readback_buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });

        // Return the mapped data directly — caller must use it before we unmap
        // We can't return a reference to mapped data across the unmap boundary,
        // so we copy row-by-row to strip padding and return owned data via a static buffer.
        static PIXEL_BUF: Mutex<Vec<u8>> = Mutex::new(Vec::new());
        let data = slice.get_mapped_range();
        let stride = (self.width * 4) as usize;
        let mut buf = PIXEL_BUF.lock().unwrap();
        buf.resize(stride * self.height as usize, 0);
        for y in 0..self.height as usize {
            let src = y * row as usize;
            let dst = y * stride;
            buf[dst..dst + stride].copy_from_slice(&data[src..src + stride]);
        }
        drop(data);
        self.readback_buffer.unmap();

        let ptr = buf.as_ptr();
        let len = buf.len();
        drop(buf);
        // SAFETY: the static Mutex ensures the buffer lives long enough;
        // caller uses it immediately in present_pixels before next frame.
        let pixels = unsafe { std::slice::from_raw_parts(ptr, len) };
        (pixels, self.width, self.height)
    }
}

// ── GPU diagnostic tests ────────────────────────────────────────────────

#[cfg(feature = "android-gpu")]
mod gpu_diagnostic {
    pub fn run_tests() {
        log::info!("=== GPU DIAGNOSTIC TESTS ===");

        // 1. Create device
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });

        let adapter =
            match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })) {
                Ok(a) => a,
                Err(e) => {
                    log::error!("TEST: No adapter: {e}");
                    return;
                }
            };

        let info = adapter.get_info();
        log::info!("TEST: Adapter: {} ({:?})", info.name, info.backend);
        log::info!("TEST: Driver: {}", info.driver_info);

        let experimental = wgpu::Features::EXPERIMENTAL_RAY_QUERY
            | wgpu::Features::EXPERIMENTAL_MESH_SHADER
            | wgpu::Features::EXPERIMENTAL_RAY_HIT_VERTEX_RETURN
            | wgpu::Features::EXPERIMENTAL_MESH_SHADER_MULTIVIEW
            | wgpu::Features::EXPERIMENTAL_PASSTHROUGH_SHADERS;
        let features = adapter.features() - experimental;

        let (device, queue) =
            match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("gpu-test"),
                required_features: features,
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::default(),
                experimental_features: wgpu::ExperimentalFeatures::default(),
            })) {
                Ok(dq) => dq,
                Err(e) => {
                    log::error!("TEST: Device creation failed: {e}");
                    return;
                }
            };
        log::info!("TEST 1 PASS: Device created");

        // 2. Test basic compute shader (write to storage buffer)
        test_compute_buffer(&device, &queue);

        // 3. Test storage texture write (what Vello does)
        test_storage_texture(&device, &queue, 256, 256);

        // 4. Test storage texture at larger sizes
        test_storage_texture(&device, &queue, 1008, 2244);

        // 5. Test Vello renderer creation
        test_vello_renderer(&device);

        // 6. Test Vello render to small texture
        test_vello_render(&device, &queue, 256, 256);

        // 7. Test Vello render to full-size texture
        test_vello_render(&device, &queue, 1008, 2244);

        log::info!("=== GPU DIAGNOSTIC TESTS COMPLETE ===");
    }

    fn test_compute_buffer(device: &wgpu::Device, queue: &wgpu::Queue) {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("test compute"),
            source: wgpu::ShaderSource::Wgsl(
                r"
                @group(0) @binding(0) var<storage, read_write> output: array<u32>;
                @compute @workgroup_size(64)
                fn main(@builtin(global_invocation_id) id: vec3<u32>) {
                    output[id.x] = id.x + 42u;
                }
            "
                .into(),
            ),
        });

        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256 * 4,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("test pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: None,
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(4, 1, 1);
        }
        queue.submit(std::iter::once(encoder.finish()));

        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        let data = slice.get_mapped_range();
        let vals: &[u32] = bytemuck::cast_slice(&data);
        if vals[0] == 42 && vals[1] == 43 {
            log::info!(
                "TEST 2 PASS: Compute shader works (vals[0]={}, vals[1]={})",
                vals[0],
                vals[1]
            );
        } else {
            log::error!("TEST 2 FAIL: Unexpected values: {:?}", &vals[..4]);
        }
        drop(data);
        buffer.unmap();
    }

    fn test_storage_texture(device: &wgpu::Device, queue: &wgpu::Queue, w: u32, h: u32) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("test storage texture"),
            source: wgpu::ShaderSource::Wgsl(
                r"
                @group(0) @binding(0) var output: texture_storage_2d<rgba8unorm, write>;
                @compute @workgroup_size(8, 8)
                fn main(@builtin(global_invocation_id) id: vec3<u32>) {
                    let dims = textureDimensions(output);
                    if id.x < dims.x && id.y < dims.y {
                        textureStore(output, id.xy, vec4<f32>(1.0, 0.0, 0.0, 1.0));
                    }
                }
            "
                .into(),
            ),
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("test storage tex pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: None,
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups((w + 7) / 8, (h + 7) / 8, 1);
        }
        queue.submit(std::iter::once(encoder.finish()));
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        log::info!("TEST 3 PASS: Storage texture {w}x{h} compute completed");
    }

    fn test_vello_renderer(device: &wgpu::Device) {
        match vello::Renderer::new(
            device,
            vello::RendererOptions {
                antialiasing_support: vello::AaSupport::area_only(),
                use_cpu: false,
                num_init_threads: None,
                pipeline_cache: None,
            },
        ) {
            Ok(_) => log::info!("TEST 5 PASS: Vello GPU renderer created"),
            Err(e) => log::error!("TEST 5 FAIL: Vello renderer creation failed: {e}"),
        }
    }

    fn test_vello_render(device: &wgpu::Device, queue: &wgpu::Queue, w: u32, h: u32) {
        let mut renderer = match vello::Renderer::new(
            device,
            vello::RendererOptions {
                antialiasing_support: vello::AaSupport::area_only(),
                use_cpu: false,
                num_init_threads: None,
                pipeline_cache: None,
            },
        ) {
            Ok(r) => r,
            Err(e) => {
                log::error!("TEST 6/7 SKIP: Can't create renderer: {e}");
                return;
            }
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create a scene with some content
        let mut scene = vello::Scene::new();
        use vello::kurbo::{Affine, Rect};
        use vello::peniko::{Brush, Color, Fill};
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &Brush::Solid(Color::from_rgba8(255, 0, 0, 255)),
            None,
            &Rect::new(0.0, 0.0, w as f64, h as f64),
        );

        match renderer.render_to_texture(
            device,
            queue,
            &scene,
            &view,
            &vello::RenderParams {
                base_color: peniko::Color::from_rgba8(255, 255, 255, 255),
                width: w,
                height: h,
                antialiasing_method: vello::AaConfig::Area,
            },
        ) {
            Ok(_) => {
                log::info!("TEST 6/7: Vello submit OK for {w}x{h}, polling...");
                match device.poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: None,
                }) {
                    Ok(_) => log::info!("TEST 6/7 PASS: poll OK for {w}x{h}"),
                    Err(e) => log::error!("TEST 6/7 FAIL: poll failed for {w}x{h}: {e}"),
                }
            }
            Err(e) => log::error!("TEST 6/7 FAIL: Vello render_to_texture {w}x{h} failed: {e}"),
        }
    }
}

// ── Action processing ────────────────────────────────────────────────────────

fn process_actions(actions: &[AppAction], running: &mut bool) {
    for action in actions {
        match action {
            AppAction::RequestRedraw => {
                REDRAW_PENDING.store(true, Ordering::Release);
            }
            AppAction::Exit => {
                *running = false;
            }
            AppAction::SetMinimized(_)
            | AppAction::SetMaximized(_)
            | AppAction::SetVisible(_)
            | AppAction::DragWindow
            | AppAction::DragResizeWindow(_)
            | AppAction::SetCursor(_)
            | AppAction::ToggleDevTools
            | AppAction::ToggleInspectMode => {}
        }
    }
}
