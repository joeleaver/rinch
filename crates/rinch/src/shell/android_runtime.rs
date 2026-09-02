//! Android runtime using android-activity (NativeActivity), presenting
//! straight into the `ANativeWindow` the Activity owns.
//!
//! Bypasses winit entirely for direct control over the Android Activity
//! lifecycle, touch input, and surface management. Uses the same
//! platform-agnostic [`RinchApp`] as the desktop shell.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use android_activity::InputStatus;
use android_activity::input::{KeyAction, KeyMapChar, MotionAction};
use android_activity::{AndroidApp, MainEvent, PollEvent};

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_core::events;
use rinch_platform::{AppAction, ImeEvent, KeyCode, Modifiers, PlatformEvent};

use crate::app::RinchApp;
use crate::shell::android_frame;
use crate::shell::android_ime::{ImeAction, ImeComposition};
use crate::shell::touch_gesture::{TouchAction, TouchGesture};

// ── Cross-thread dispatch ────────────────────────────────────────────────────

static REDRAW_PENDING: AtomicBool = AtomicBool::new(false);

/// Queue a cross-thread closure and ask for a frame.
///
/// The queue itself lives in `rinch-core` so every host shares one
/// ([`rinch_core::queue_main_callback`], issue #172); what this shell adds is
/// the redraw request that gets the loop back around to drain it.
fn dispatch_to_main_thread(f: Box<dyn FnOnce() + Send>) {
    rinch_core::queue_main_callback(f);
    REDRAW_PENDING.store(true, Ordering::Release);
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

    // The device outlives the window; the swapchain does not. That asymmetry
    // is the whole lifecycle design of the GPU path and the reason these are
    // two bindings rather than one — see [`GpuContext`] for what each half
    // dies of. On the software path there is nothing to keep, so there is only
    // `surface`.
    #[cfg(feature = "android-gpu")]
    let mut gpu: Option<GpuContext> = None;
    let mut surface: Option<WindowSurface> = None;
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
    // A window-focus change seen inside `poll_events`, dispatched after it
    // returns (`app` is not reachable from the callback). `None` = no change
    // this turn; the app starts focused, so nothing to send until Android says
    // otherwise (issue #147).
    let mut window_focus_change: Option<bool> = None;

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

                        // The old surface is dropped *first*, and on its own
                        // line, because `surface = <new>` would build the new
                        // one while the old was still alive. On the software
                        // path that is merely wasteful; on the GPU path two
                        // swapchains on one `ANativeWindow` is
                        // `VK_ERROR_NATIVE_WINDOW_IN_USE_KHR`, and the window
                        // stays un-drawable for the rest of the process.
                        // `InitWindow` twice with no `TerminateWindow` between
                        // is not the documented sequence, but the sequence a
                        // phone actually delivers is not something this file
                        // gets to assume.
                        surface = None;

                        #[cfg(not(feature = "android-gpu"))]
                        {
                            surface = Some(SoftSurface::new(&native_window, w, h));
                        }
                        #[cfg(feature = "android-gpu")]
                        {
                            surface = GpuContext::attach(&mut gpu, &native_window, w, h);
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
                    // Everything that can touch the window dies here, inside
                    // the callback, before `poll_events` returns.
                    //
                    // That timing is not a nicety. `android-activity` delivers
                    // this event from `onNativeWindowDestroyed`, and the
                    // Android thread that raised it is *blocked* until this
                    // loop has acknowledged it — so a swapchain dropped here
                    // is a swapchain destroyed while the `ANativeWindow` is
                    // still alive, which is the only order Vulkan accepts. A
                    // swapchain dropped one iteration later would be destroyed
                    // against a window the system had already freed.
                    //
                    // And it is the *only* way to present, which is what makes
                    // "render into a dead surface" unreachable rather than
                    // unlikely: the sole `GpuSurface` in the process is this
                    // binding, the paint block below cannot run without it,
                    // and this line empties it before the paint block is
                    // reached. There is no second handle to go stale.
                    //
                    // `gpu` — the device, the queue and vello's compiled
                    // pipelines — deliberately survives. Pressing Home
                    // destroys the window and nothing else; rebuilding the
                    // renderer on the way back would cost the better part of a
                    // second of black screen for a device Vulkan never
                    // invalidated.
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
                // Android's answer to `PlatformEvent::WindowFocus` (issue
                // #147): the activity keeps its in-document focus claim across
                // a focus loss — a notification shade, a permission dialog —
                // and the focused widget is told, then told again when focus
                // returns. Distinct from `Resume`/`Pause`, which are lifecycle:
                // an activity can be resumed but unfocused.
                MainEvent::GainedFocus => {
                    window_focus_change = Some(true);
                }
                MainEvent::LostFocus => {
                    window_focus_change = Some(false);
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

        // Window focus, before input: a key that arrives in the same turn as
        // the regain belongs to the refocused window. Dispatched *above* the
        // no-surface bail below for the same reason the composition flush is:
        // `MainEvent::LostFocus` and the `TerminateWindow` that drops the
        // surface arrive together when the activity goes to the background, so
        // deferring the loss to the next frame that has a surface would defer
        // it past the regain — and this slot holds only the latest value, so
        // the `GainedFocus` would overwrite it and the focused widget would
        // never hear `on_focus_lost` at all. `handle_event` needs no surface.
        if let Some(focused) = window_focus_change.take() {
            let actions = app.handle_event(
                PlatformEvent::WindowFocus(focused),
                physical_size,
                scale_factor,
            );
            process_actions(&actions, &mut running);
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
            Instant::now(),
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
        rinch_core::drain_main_callbacks();
        rinch_core::reactive::drain_polls();

        // The frame clock. Every time-driven thing `RinchApp` owns — CSS
        // transitions, CSS animations, the dirty state the input handlers
        // leave for it to batch — advances here and nowhere else, and this
        // loop polls with a 16ms timeout so the clock runs at ~60Hz. Before
        // the surface is presented, not after: what it resolves has to be in
        // the pixels this iteration hands to `present_pixels`, and the redraw
        // it asks for has to be visible to the swap below.
        let frame = android_frame::pump_frame(&mut app, physical_size, scale_factor);
        process_actions(&frame.actions, &mut running);
        if !running {
            break;
        }

        // Check if app has pending layout (signal changes create pending updates)
        let has_momentum = gesture.has_momentum();
        let redraw = REDRAW_PENDING.swap(false, Ordering::AcqRel);
        // `pending_layout` also covers a finished image decode, which has
        // dirtied no node (`RinchApp::has_pending_images`) — without that this
        // loop spins past the decode forever and the `<img>` paints as a blank
        // card. Found on a moto g stylus showing a rasterised PDF page out of
        // app-private storage: the page was on disk, the box was the right
        // size, and the pixels only arrived when something else happened to
        // make the screen redraw.
        let pending = frame.pending_layout;
        let needs_paint = redraw || pending || has_momentum || frame.needs_paint;

        if needs_paint && mounted {
            if pending {
                app.resolve_and_repaint(logical_size.0 as f32, logical_size.1 as f32);
            }

            #[cfg(feature = "android-gpu")]
            if let (Some(ctx), Some(s)) = (&mut gpu, &mut surface) {
                let scene = app.build_scene(scale_factor, logical_size);
                s.present_scene(&mut ctx.renderer, scene);
            }

            #[cfg(not(feature = "android-gpu"))]
            if let Some(ref mut s) = surface {
                let (pixels, w, h) = app.build_pixels(scale_factor, logical_size, false);
                s.present_pixels(pixels, w, h);
            }
        }
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
                        // A soft-keyboard commit is synthesized text — no key
                        // produced it, so there is no key value to report
                        // (`None` = unknown, per the field's contract).
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

/// The motion actions the recogniser reacts to. Everything else — pointer index
/// churn from a second finger, button events from a mouse — is `Other`, which it
/// ignores.
fn touch_action(action: MotionAction) -> TouchAction {
    match action {
        MotionAction::Down => TouchAction::Down,
        MotionAction::Move => TouchAction::Move,
        MotionAction::Up => TouchAction::Up,
        MotionAction::Cancel => TouchAction::Cancel,
        MotionAction::HoverMove => TouchAction::HoverMove,
        _ => TouchAction::Other,
    }
}

/// `now` is sampled once per loop iteration by the caller and threaded through,
/// so every event in one drain — and the timers that run after it — agree on
/// what time it is.
fn collect_input_events(
    android_app: &AndroidApp,
    gesture: &mut TouchGesture,
    scale_factor: f64,
    now: Instant,
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
                        gesture.process(touch_action(motion.action()), x, y, now, &mut events);
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

                            // The key-character-map char doubles as the logical
                            // key value: it is the layout-produced, case-accurate
                            // (`meta_state` includes Shift) character — exactly
                            // what `KeyboardEvent.key` spells for a printable
                            // key. Android hands us no DOM-style *name* for the
                            // rest (Enter, arrows, CapsLock), so those stay
                            // `None` and resolve through the physical `key`.
                            if let Some(key_code) = map_android_keycode(key.key_code()) {
                                events.push(PlatformEvent::KeyDown {
                                    key: key_code,
                                    logical_key: text.clone(),
                                    text,
                                    modifiers,
                                });
                            } else if let Some(text) = text {
                                events.push(PlatformEvent::KeyDown {
                                    key: KeyCode::Other,
                                    logical_key: Some(text.clone()),
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

    // A press that has been held still long enough is a context menu. This sits
    // beside the momentum tick because both are clocks the finger is not driving,
    // and the loop's 16ms poll is what turns them.
    gesture.tick_long_press(now, &mut events);

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

// ── The window surface ───────────────────────────────────────────────────────

/// Whatever this build presents through, named once so the loop above can hold
/// a single binding and the lifecycle can be reasoned about in one place.
///
/// The two implementations are not variants of a runtime choice — they are two
/// different builds. A `--features android-gpu` binary has no `SoftSurface`
/// compiled into it at all, and the shipping binary has no wgpu. That matters
/// here for a reason beyond code size: both presenters take the *same*
/// `ANativeWindow`, and Android lets a window be connected to exactly one
/// producer API at a time. `ANativeWindow_lock` connects it to the CPU
/// producer; creating a Vulkan swapchain connects it to
/// `NATIVE_WINDOW_API_EGL`. Building both and picking one at runtime would
/// mean the loser of that race silently failing every frame, so the choice is
/// made by the compiler and only one of them exists.
#[cfg(not(feature = "android-gpu"))]
type WindowSurface = SoftSurface;
#[cfg(feature = "android-gpu")]
type WindowSurface = GpuSurface;

/// The screen, and the one place a finished frame becomes pixels on it.
///
/// This writes straight into the buffer `ANativeWindow_lock` hands back. It
/// used to go through `softbuffer`, which is the right abstraction on the
/// desktop — one API over X11, Wayland, Win32 and the rest — and the wrong one
/// here, because Android is the platform where softbuffer cannot map the
/// window's memory into the caller's hands. Its Android backend keeps a shadow
/// `Vec<Pixel>` the size of the screen, allocates and zeroes a fresh one on
/// every `next_buffer()`, lets the caller fill *that*, and then copies it a
/// byte at a time into the locked window buffer on `present()`. Every frame
/// therefore paid for a 10.6 MB allocation and two full-screen copies where
/// one would do.
///
/// Card K27 measured the three phases on a moto g stylus 5G at 1080x2460,
/// with a temporary `log::info!` probe around each — the technique K24 used —
/// before and after this rewrite:
///
/// ```text
///                     acquire   fill   swap    total
/// through softbuffer      4.1    2.9    5.2     12.2
/// into the window         0.5    2.0    0.6      3.1
/// ```
///
/// `acquire` is the allocate-and-zero, `fill` is this function's own copy, and
/// `swap` is handing the buffer to the compositor. So only about a quarter of
/// the ~12ms was the copy this file controls; the rest was the shadow buffer
/// existing at all. That mattered more than it sounds: on a
/// screen that is scrolling with momentum the painter returns its cached
/// pixmap untouched and nothing is redrawn, so those 12ms *were* the entire
/// frame — 298 of the 318 frames in K27's library-scroll trace repainted
/// nothing and cost 13.3ms each anyway. After the rewrite the same trace's
/// unchanged frames cost 6.4ms, and the loop presents about 30% more of them
/// in the same wall-clock window — the residue is `ANativeWindow_lock`
/// blocking for a free buffer, which is the display's back-pressure and not
/// work.
///
/// Writing into the window directly gives up nothing here. The one thing a
/// shadow buffer buys is the ability to leave pixels alone between frames,
/// and this shell never does: `build_pixels` hands back a complete frame every
/// time, so every pixel of the window is written every time regardless of what
/// the swap chain had in it before.
#[cfg(not(feature = "android-gpu"))]
struct SoftSurface {
    native: ndk::native_window::NativeWindow,
    width: u32,
    height: u32,
}

#[cfg(not(feature = "android-gpu"))]
impl SoftSurface {
    /// Infallible: nothing here can fail in a way that leaves the shell better
    /// off skipping the frame. A refused `set_buffers_geometry` is logged and
    /// survived — `present_pixels` re-checks the format of the buffer it is
    /// actually handed, which is the authoritative answer anyway.
    fn new(window: &ndk::native_window::NativeWindow, width: u32, height: u32) -> Self {
        let surface = Self {
            native: window.clone(),
            width: width.max(1),
            height: height.max(1),
        };
        surface.configure();
        surface
    }

    /// Ask the window for buffers of the size and format this shell paints.
    ///
    /// `R8G8B8X8_UNORM` and not `R8G8B8A8_UNORM`: the frame is opaque, the
    /// fourth byte is never read, and asking for an alpha channel would invite
    /// the compositor to blend a surface that has nothing behind it.
    fn configure(&self) {
        use ndk::hardware_buffer_format::HardwareBufferFormat;
        if let Err(e) = self.native.set_buffers_geometry(
            self.width as i32,
            self.height as i32,
            Some(HardwareBufferFormat::R8G8B8X8_UNORM),
        ) {
            log::error!("present: set_buffers_geometry failed: {e}");
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.configure();
    }

    fn present_pixels(&mut self, pixels: &[u8], width: u32, height: u32) {
        use ndk::hardware_buffer_format::HardwareBufferFormat;

        if width != self.width || height != self.height {
            self.resize(width, height);
        }

        let mut guard = match self.native.lock(None) {
            Ok(g) => g,
            Err(_) => return,
        };

        // **Every byte of this buffer has to be written before we return.**
        //
        // `ANativeWindow_unlockAndPost` has no cancel — dropping the guard
        // posts whatever the buffer holds — and `lock(None)` declares the whole
        // buffer dirty, so Android copies nothing forward into it from the last
        // frame. A bail that simply returns therefore does not drop a frame, it
        // posts an uninitialised one. Where the pixels cannot be written, black
        // is written instead.
        //
        // The format check is the one softbuffer used to do for us
        // (`next_buffer` refused anything but 32-bit RGBA/RGBX). It cannot be
        // folded into `lines()` returning `None`: the Android default is
        // `R5G6B5` 16bpp, which has a perfectly good `bytes_per_pixel` of 2, so
        // a window whose `set_buffers_geometry` was refused hands back lines
        // half the expected width and RGBA8 bytes poured into them are a
        // garbled screen, not a wrong one.
        let format = guard.format();
        let format_ok = matches!(
            format,
            HardwareBufferFormat::R8G8B8A8_UNORM | HardwareBufferFormat::R8G8B8X8_UNORM
        );
        if !format_ok {
            log::error!("present: window buffer is {format:?}, not 32-bit RGBA/RGBX");
        }

        // `None` only for a format with no byte size at all, which leaves no
        // safe way to reach the bytes — the one case where the unwritten buffer
        // goes out as it is.
        let Some(lines) = guard.lines() else {
            log::error!("present: window buffer format {format:?} has no byte size");
            return;
        };

        let src_stride = width as usize * 4;
        for (y, line) in lines.enumerate() {
            let src = y * src_stride;
            if !format_ok || src + src_stride > pixels.len() {
                line.fill(std::mem::MaybeUninit::new(0));
                continue;
            }
            let n = line.len().min(src_stride);
            // SAFETY: `MaybeUninit<u8>` has the same layout as `u8`, so an
            // initialised `&[u8]` is a valid `&[MaybeUninit<u8>]` to read
            // from. This is the transmute `MaybeUninit::copy_from_slice`
            // performs, written out because that method is not yet stable on
            // this toolchain.
            let src_uninit = unsafe {
                std::slice::from_raw_parts(
                    pixels[src..src + n]
                        .as_ptr()
                        .cast::<std::mem::MaybeUninit<u8>>(),
                    n,
                )
            };
            line[..n].copy_from_slice(src_uninit);
            // A window wider than the frame leaves a tail on every line; it is
            // posted along with the rest, so it cannot be left uninitialised.
            line[n..].fill(std::mem::MaybeUninit::new(0));
        }
        // Dropping the guard unlocks the buffer and posts it.
    }
}

// ── GPU surface (wgpu + vello) ──────────────────────────────────────────

/// The device, and everything standing on it that outlives any one window.
///
/// **This struct exists because of when things die.** A Vulkan device, and the
/// twenty-odd compute pipelines vello compiles against it, belong to the
/// process: nothing Android does to an Activity invalidates them. A swapchain
/// belongs to the `ANativeWindow`, and Android destroys that window every time
/// the Activity is stopped — every press of Home, every task switch, every
/// screen rotation on some devices. Those are different lifetimes, so they are
/// different structs, and the loop holds one binding for each.
///
/// The version of this file that read the frame back to the CPU had no
/// swapchain and therefore no reason to make the distinction: it kept device
/// and target in one `GpuSurface` and dropped the lot on `TerminateWindow`.
/// Doing that now would recompile vello's pipeline set on every resume, and
/// the logcat timestamps say what that costs. On a moto g stylus 5G, cold:
///
/// ```text
/// 18:28:44.113  vello: initialising shader modules in parallel using 6 threads
/// 18:28:45.350  GPU: Adreno (TM) 619 (Vulkan)          <- 1,237 ms later
/// ```
///
/// and then, pressing Home and returning twice:
///
/// ```text
/// 18:32:12.476  InitWindow: 1080x2460
/// 18:32:12.484  GPU surface: formats=[...]             <- 8 ms
/// 18:32:17.620  InitWindow: 1080x2460
/// 18:32:17.624  GPU surface: formats=[...]             <- 4 ms
/// ```
///
/// Those two lines are a whole swapchain rebuild, and no `GPU: Adreno` line
/// appears between them, which is the observable proof that the device was
/// reused. One struct instead of two would have made every resume a
/// 1.2-second black screen in exchange for throwing away a device Vulkan
/// never invalidated.
/// Set by the device-lost callback, cleared by [`GpuContext::attach`]. The
/// callback is a plain `fn` with no access to the loop's bindings, so the flag
/// is how it reaches them.
#[cfg(feature = "android-gpu")]
static DEVICE_LOST: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "android-gpu")]
struct GpuContext {
    /// Kept because a surface must be created by the same instance that will
    /// present it, and the surfaces are created long after this struct is —
    /// once per window, for as many windows as the Activity is given.
    instance: wgpu::Instance,
    /// Kept because `get_capabilities` is a question about an (adapter,
    /// surface) *pair*: the formats and present modes a window supports have
    /// to be asked again for each new window, not cached from the first.
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: vello::Renderer,
}

#[cfg(feature = "android-gpu")]
impl GpuContext {
    /// Give the window a swapchain, bringing the device up if this is the
    /// first window the process has seen.
    ///
    /// Called from `InitWindow` and nowhere else. The `slot` argument is the
    /// loop's `gpu` binding: on the first call it is filled in and on every
    /// later one it is reused, so a resume costs a swapchain and not a device.
    ///
    /// Reused, that is, unless Vulkan took it away. Caching the device across
    /// windows is what makes a resume cheap, and it is also the one way this
    /// shell could hold a *dead* device for ever: the version of this file that
    /// built a device per window recovered from a device loss on the next
    /// `InitWindow` without anyone noticing it was a recovery. `DEVICE_LOST`
    /// keeps that property — the callback below can only log (it is handed no
    /// state), so it records the loss and the next window throws the context
    /// away and builds a fresh one.
    fn attach(
        slot: &mut Option<GpuContext>,
        window: &ndk::native_window::NativeWindow,
        width: u32,
        height: u32,
    ) -> Option<GpuSurface> {
        if DEVICE_LOST.swap(false, Ordering::AcqRel) && slot.is_some() {
            log::warn!("GPU: device was lost; rebuilding the context for this window");
            *slot = None;
        }
        if slot.is_none() {
            *slot = Some(GpuContext::new(window)?);
        }
        let ctx = slot.as_ref()?;

        let surface = match ctx.instance.create_surface(AndroidWindow {
            native: window.clone(),
        }) {
            Ok(s) => s,
            Err(e) => {
                log::error!("GPU: create_surface failed: {e}");
                return None;
            }
        };

        // The device was chosen for the *first* window this process saw. A
        // later window is a different `ANativeWindow` and, in principle, a
        // different `VkSurfaceKHR` with its own presentation-support answer.
        // In practice every Android surface on a device is supported by the
        // same queue family, so this has never been seen to fail — but
        // presenting to a surface the adapter cannot present to is undefined
        // behaviour, not an error, and the check costs one call at window
        // creation.
        if !ctx.adapter.is_surface_supported(&surface) {
            log::error!("GPU: adapter cannot present to this window; no GPU frames");
            return None;
        }

        GpuSurface::new(ctx, surface, width, height)
    }

    /// Bring up the instance, adapter, device and renderer.
    ///
    /// Takes the window because the adapter has to be chosen *for* a surface:
    /// `request_adapter`'s `compatible_surface` is what makes wgpu pick a
    /// queue family that can actually present, and a device whose queue cannot
    /// present is a device that can draw the frame and not show it. The
    /// surface built here is a probe — it is dropped at the end of this
    /// function and the real one is built by [`attach`](Self::attach) from the
    /// instance this leaves behind.
    ///
    /// **The probe must not outlive this function.**
    /// `vkCreateAndroidSurfaceKHR` is where the `ANativeWindow` is connected to
    /// `NATIVE_WINDOW_API_EGL` — libvulkan calls `native_window_api_connect`
    /// there, not at swapchain creation, and returns
    /// `VK_ERROR_NATIVE_WINDOW_IN_USE_KHR` for a second connect. So the probe
    /// and the real surface can only ever exist one at a time, and this works
    /// because `probe` is a local that drops before `attach` creates the other
    /// one. Do not hoist it into the returned struct.
    fn new(window: &ndk::native_window::NativeWindow) -> Option<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            flags: wgpu::InstanceFlags::empty(),
            backend_options: wgpu::BackendOptions::from_env_or_default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        });

        let probe = match instance.create_surface(AndroidWindow {
            native: window.clone(),
        }) {
            Ok(s) => s,
            Err(e) => {
                log::error!("GPU: probe surface failed: {e}");
                return None;
            }
        };

        let adapter =
            match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&probe),
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
            DEVICE_LOST.store(true, Ordering::Release);
        });

        // `use_cpu: false`, which is the whole point of having a device.
        //
        // It read `true` until card K27 measured it. Vello's `use_cpu` swaps
        // its compute pipeline for CPU implementations of the same stages —
        // a debugging aid — so this feature initialised Vulkan, allocated a
        // GPU texture, and then rasterised on the same four cores the
        // software painter uses. On a moto g stylus 5G at 1080x2460 that cost
        // 31ms a frame on the library list and 56ms on a screen with a
        // rasterised PDF page; with the flag corrected the same frames took
        // **2.3ms and 5.0ms**. Nothing else about the path changed.
        let renderer = vello::Renderer::new(
            &device,
            vello::RendererOptions {
                antialiasing_support: vello::AaSupport::area_only(),
                use_cpu: false,
                num_init_threads: None,
                pipeline_cache: None,
            },
        )
        .map_err(|e| log::error!("GPU: vello renderer failed: {e}"))
        .ok()?;

        let info = adapter.get_info();
        log::info!(
            "GPU: {} ({:?}), driver {}",
            info.name,
            info.backend,
            info.driver_info
        );

        Some(Self {
            instance,
            adapter,
            device,
            queue,
            renderer,
        })
    }
}

/// One window's swapchain, and the texture vello is allowed to draw into.
///
/// **Why there is an intermediate texture at all.** Vello is a compute
/// rasteriser: its final stage writes finished pixels into a *storage* image
/// from a compute shader. A swapchain image is a colour attachment, and on
/// Android — as on most drivers — it is not usable as a storage image, so
/// vello cannot be pointed at it. `render_to_texture` therefore draws into
/// `target`, an `Rgba8Unorm` storage texture the size of the window, and a
/// full-screen triangle then samples that into the swapchain image in the
/// format the display asked for. Vello's own `util` module does exactly this
/// and its documentation is explicit that the alternative — handing vello the
/// swapchain texture when the driver happens to allow it — "should generally
/// be avoided, as some GPUs assume that you will not be rendering to the
/// surface using a compute pipeline, and optimise accordingly".
///
/// The blit costs one texture read and one write per pixel, entirely on the
/// GPU. What it replaces is the readback this path used to do: a
/// `copy_texture_to_buffer` into mappable memory, a blocking wait for it, and
/// a row-by-row CPU copy to strip wgpu's 256-byte row padding — 50 to 69ms per
/// frame on this device, against a rasterisation that took 2 to 5ms. Card K27
/// measured that; card K35 is this rewrite, and measured it again the same
/// way — a `log::info!` per frame through `adb logcat`, medians over a few
/// hundred frames, on the same three screens of the same app on the same moto
/// g stylus 5G at 1080x2460, release build.
///
/// ```text
///           resolve  scene   vello  readback  unpack  present   total
/// K27, reading the frame back:
/// library       0.0    0.0     2.3      51.6    16.5     12.2    84.0
/// sheet         0.0    2.4     1.8      44.8    14.7     12.1    74.8
/// pdf           0.0    0.0     5.0      64.0    16.6     12.6    96.9
///
/// K35, presenting the swapchain:
/// library       0.0    0.0     2.4         -       -      1.6     4.1
/// sheet         0.0    1.2     2.5         -       -     15.3    18.8
/// pdf           0.0    0.0     5.6         -       -      1.6     7.4
/// ```
///
/// The `readback` and `unpack` columns are gone, which was the entire point,
/// and `vello` is unchanged — 2.4 against 2.3, 5.6 against 5.0 — which is the
/// evidence that nothing about the *rasterisation* was disturbed. A frame on
/// the library list costs 4.1ms where it cost 84.0.
///
/// **Read the `present` column carefully; it is not all work.** It is the
/// acquire, the blit and the `vkQueuePresentKHR` together, and under Fifo the
/// first and last of those block on the display. Broken out, the library
/// frame is `acquire 0.1 / blit 1.1 / present 0.4` and the sheet frame is
/// `acquire 0.1 / blit 1.1 / present 14.1`. The blit is a millisecond on all
/// three screens; the sheet's extra 14ms is vsync back-pressure, and it
/// appears there and not on the other two because the sheet is the only one of
/// the three that rebuilds its scene every frame (`scene 1.2`) and so actually
/// gives the compositor something new often enough to be throttled.
///
/// **What is left is not this file.** Frame-to-frame, the same traces measure
/// 20.4 / 36.0 / 24.0 ms — 4 to 17ms more than the paint costs. That residue is
/// the `poll_events(Some(16ms))` timeout at the top of the loop, which blocks
/// for its full 16ms whenever no input is arriving, and it is paid by the
/// software path identically. Presenting the swapchain has moved the ceiling
/// from "the readback" to "the loop's own clock", which is a different card.
#[cfg(feature = "android-gpu")]
struct GpuSurface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    /// Vello's canvas: `Rgba8Unorm`, `STORAGE_BINDING` so vello may write it,
    /// `TEXTURE_BINDING` so the blit may read it.
    target: wgpu::Texture,
    target_view: wgpu::TextureView,
    blitter: wgpu::util::TextureBlitter,
    /// Clones of the context's device and queue — both are `Arc` handles, so
    /// this costs a refcount and buys `resize` and `present_scene` the ability
    /// to work without being handed the whole context. The renderer is *not*
    /// cloned, because vello's `Renderer` is a `&mut` resource with a frame's
    /// worth of scratch buffers in it and there must only ever be one.
    device: wgpu::Device,
    queue: wgpu::Queue,
    width: u32,
    height: u32,
}

#[cfg(feature = "android-gpu")]
impl GpuSurface {
    fn new(
        ctx: &GpuContext,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
    ) -> Option<Self> {
        let width = width.max(1);
        let height = height.max(1);
        let caps = surface.get_capabilities(&ctx.adapter);

        log::info!(
            "GPU surface: formats={:?} present={:?} alpha={:?}",
            caps.formats,
            caps.present_modes,
            caps.alpha_modes
        );

        // **Format negotiation, and the one wrong answer.**
        //
        // Vello writes an `Rgba8Unorm` texture whose bytes are already
        // sRGB-encoded — that is what a CSS colour is. The blit samples that
        // texture (a `Unorm` read hands the shader the stored value untouched)
        // and writes the result to the swapchain. So the swapchain must also
        // be a plain `Unorm` format: an `*UnormSrgb` attachment would apply a
        // linear-to-sRGB encode on the way out, on values that are already
        // encoded, and the whole app would come out washed out and pale. That
        // failure is uniform and subtle enough to be mistaken for a theme bug,
        // which is why it is spelled out here rather than left to the reader
        // of a `matches!`.
        //
        // Between the two acceptable formats there is nothing to choose:
        // `Bgra8Unorm` is what Android hands out and the byte swap is free in
        // the blit's fragment shader, since the sampler yields components by
        // name and the attachment stores them by its own order.
        let format = match caps.formats.iter().copied().find(|f| {
            matches!(
                f,
                wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm
            )
        }) {
            Some(f) => f,
            None => {
                log::error!(
                    "GPU: window offers no non-sRGB 8-bit format ({:?}); no GPU frames",
                    caps.formats
                );
                return None;
            }
        };

        // Opaque if the window will take it, matching the software path's
        // `R8G8B8X8_UNORM` request and for the same reason: this shell paints
        // every pixel of a full-screen frame, there is nothing behind it, and
        // asking the compositor to blend a surface with no alpha to respect is
        // work nobody wants done. `Auto` is the fallback and lets wgpu pick
        // whatever the window does support.
        let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
            wgpu::CompositeAlphaMode::Opaque
        } else {
            wgpu::CompositeAlphaMode::Auto
        };

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            // **Fifo, which means this loop no longer decides when to
            // present.**
            //
            // Fifo is the one present mode Vulkan guarantees exists, and it is
            // vsync: `get_current_texture` blocks until the display has
            // released an image. That is a real change in who paces the frame
            // loop, and it is the change that was already true. The software
            // path's `ANativeWindow_lock` blocks for a free buffer in exactly
            // the same way — K27's note that "what is left is
            // `ANativeWindow_lock` waiting for a free buffer, which is the
            // display's back-pressure rather than work" describes Fifo by
            // another name. The loop's own 16ms `poll_events` timeout was
            // always an approximation of the same 60Hz; now something
            // authoritative enforces it.
            //
            // The cost is that a timing probe around the acquire measures
            // *waiting*, not work, and a reader of those numbers who forgets
            // it will conclude the present is expensive. Mailbox would hide
            // the wait by rendering frames nobody sees, which is a worse thing
            // to do to a phone battery than to a benchmark.
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
        };
        surface.configure(&ctx.device, &config);

        let (target, target_view) = Self::make_target(&ctx.device, width, height);

        Some(Self {
            surface,
            config,
            target,
            target_view,
            blitter: wgpu::util::TextureBlitter::new(&ctx.device, format),
            device: ctx.device.clone(),
            queue: ctx.queue.clone(),
            width,
            height,
        })
    }

    fn make_target(
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("rinch-vello-target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    fn resize(&mut self, width: u32, height: u32) {
        let (w, h) = (width.max(1), height.max(1));
        if (w, h) == (self.width, self.height) {
            return;
        }
        self.width = w;
        self.height = h;
        self.config.width = w;
        self.config.height = h;
        // Both halves, in this order. Reconfiguring the surface retires the
        // old swapchain images; the intermediate has to grow to match or the
        // blit would stretch a window-sized frame across a differently sized
        // window, which on a rotation is not a subtle artefact.
        self.surface.configure(&self.device, &self.config);
        let (target, view) = Self::make_target(&self.device, w, h);
        self.target = target;
        self.target_view = view;
    }

    fn reconfigure(&mut self) {
        self.surface.configure(&self.device, &self.config);
    }

    /// Take the next swapchain image, rebuilding the swapchain if the window
    /// has moved on without telling us.
    ///
    /// `Outdated` and `Lost` are not errors here, they are how a compositor
    /// says the window changed — a rotation, a fold, an insets change, a
    /// resize the Activity has not yet delivered as `WindowResized`. The
    /// answer to both is the same: reconfigure and ask once more.
    ///
    /// The retry is bounded at one, and deliberately does not try to work out
    /// the window's new size for itself. If the window really did change
    /// shape, this surface is still configured at the old one until
    /// `WindowResized` reaches the loop above and calls `resize` — a frame or
    /// two of the compositor scaling a slightly-wrong image, which is
    /// invisible, against a spin here that would not be. Dropping a frame is
    /// this function's failure mode; looping is not.
    ///
    /// **`suboptimal` is deliberately not one of the conditions retried on.**
    /// wgpu never reports it on Android: `wgpu-hal`'s Vulkan `acquire_texture`
    /// maps `VK_SUBOPTIMAL_KHR` to success under `target_os = "android"`,
    /// because wgpu creates every swapchain with `preTransform = IDENTITY` and
    /// libvulkan then returns `VK_SUBOPTIMAL_KHR` *persistently* for any
    /// display orientation that is not the identity one. Retrying on it would
    /// be dead code as written, and live code that rebuilt the whole swapchain
    /// and paid two vsync waits on **every** frame in landscape.
    fn acquire(&mut self) -> Option<wgpu::SurfaceTexture> {
        for attempt in 0..2 {
            match self.surface.get_current_texture() {
                Ok(frame) => return Some(frame),
                Err(e @ (wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost))
                    if attempt == 0 =>
                {
                    log::info!("GPU: swapchain {e:?}, reconfiguring");
                    self.reconfigure();
                }
                Err(e) => {
                    log::error!("GPU: acquire failed: {e:?}");
                    return None;
                }
            }
        }
        None
    }

    /// Rasterise the scene and put it on the screen, with no trip through
    /// system memory.
    ///
    /// The order — rasterise, *then* acquire — is deliberate. Under Fifo the
    /// acquire blocks until the display releases an image, so submitting
    /// vello's compute work first gives the GPU something to do during that
    /// wait instead of starting it afterwards. It also means an acquire that
    /// fails costs only the frame, not the scene: the intermediate texture
    /// still holds a valid, correctly sized frame, and the next iteration
    /// draws over it.
    fn present_scene(&mut self, renderer: &mut vello::Renderer, scene: &vello::Scene) {
        if let Err(e) = renderer.render_to_texture(
            &self.device,
            &self.queue,
            scene,
            &self.target_view,
            &vello::RenderParams {
                base_color: peniko::Color::from_rgba8(255, 255, 255, 255),
                width: self.width,
                height: self.height,
                antialiasing_method: vello::AaConfig::Area,
            },
        ) {
            log::error!("GPU render failed: {e}");
            return;
        }

        let Some(frame) = self.acquire() else {
            return;
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("rinch-blit"),
            });
        self.blitter
            .copy(&self.device, &mut encoder, &self.target_view, &view);
        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
    }
}

#[cfg(feature = "android-gpu")]
impl Drop for GpuSurface {
    /// Wait for the GPU to be finished with the swapchain before the swapchain
    /// stops existing.
    ///
    /// This runs before any field is dropped, which is the point. The last
    /// frame this surface presented may still be in the compositor's hands and
    /// its command buffer still executing; `wgpu` will destroy the
    /// `VkSwapchainKHR` when the `Surface` field goes, and destroying a
    /// swapchain with work outstanding against its images is undefined
    /// behaviour. Waiting here — inside `TerminateWindow`, while Android's own
    /// thread is still blocked waiting for this loop to acknowledge the
    /// event — is the one moment where the wait is both safe and free of
    /// anything to race against.
    fn drop(&mut self) {
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
    }
}

/// The `ANativeWindow`, wearing the two traits wgpu needs to make a surface
/// from it.
///
/// `ndk` implements `HasWindowHandle` for `NativeWindow` but not
/// `HasDisplayHandle` — there is no display object on Android to point at —
/// so the pair has to be assembled here. The `native` field is a *clone*,
/// which on `NativeWindow` is an `ANativeWindow_acquire`: wgpu takes ownership
/// of this value and keeps it for the life of the `Surface`, so the window
/// cannot be freed out from under a live swapchain even if the Activity
/// releases its own reference first. That is the second lock on the lifecycle,
/// under the one in `TerminateWindow`.
#[cfg(feature = "android-gpu")]
struct AndroidWindow {
    native: ndk::native_window::NativeWindow,
}

#[cfg(feature = "android-gpu")]
impl raw_window_handle::HasWindowHandle for AndroidWindow {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        self.native.window_handle()
    }
}

#[cfg(feature = "android-gpu")]
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
