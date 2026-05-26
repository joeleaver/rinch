//! Android runtime using android-activity (GameActivity) + softbuffer.
//!
//! Bypasses winit entirely for direct control over the Android Activity
//! lifecycle, touch input, and surface management. Uses the same
//! platform-agnostic [`RinchApp`] as the desktop shell.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use android_activity::InputStatus;
use android_activity::input::{KeyAction, MotionAction};
use android_activity::{AndroidApp, MainEvent, PollEvent};

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_core::events;
use rinch_platform::{AppAction, KeyCode, Modifiers, MouseButton, PlatformEvent};

use crate::app::RinchApp;

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

    events::clear_handlers();
    rinch_core::clear_context();

    let mut surface: Option<SoftSurface> = None;
    let mut mounted = false;
    let mut window_size = (0u32, 0u32);
    let mut scale_factor = 1.0f64;
    let mut running = true;
    let mut gesture = TouchGesture::new();

    while running {
        android_app.poll_events(Some(Duration::from_millis(16)), |event| {
            match event {
                PollEvent::Main(main_event) => match main_event {
                    MainEvent::InitWindow { .. } => {
                        if let Some(native_window) = android_app.native_window() {
                            let w = native_window.width() as u32;
                            let h = native_window.height() as u32;
                            window_size = (w, h);

                            let config = android_app.config();
                            let dpi = config.density().unwrap_or(160) as f64;
                            scale_factor = dpi / 160.0;

                            surface = SoftSurface::new(&native_window, w, h);

                            if !mounted {
                                // Mount at physical pixel size — we use scale 1.0
                                // for both rendering and input coordinates
                                app.mount_component(w as f32, h as f32);
                                mounted = true;
                            }

                            REDRAW_PENDING.store(true, Ordering::Release);
                        }
                    }
                    MainEvent::TerminateWindow { .. } => {
                        surface = None;
                    }
                    MainEvent::WindowResized { .. } => {
                        if let Some(native_window) = android_app.native_window() {
                            let w = native_window.width() as u32;
                            let h = native_window.height() as u32;
                            window_size = (w, h);

                            let actions = app.handle_event(
                                PlatformEvent::Resized {
                                    width: w,
                                    height: h,
                                },
                                window_size,
                                scale_factor,
                            );
                            process_actions(&actions, &mut running);

                            if let Some(ref mut s) = surface {
                                s.resize(w, h);
                            }

                            REDRAW_PENDING.store(true, Ordering::Release);
                        }
                    }
                    MainEvent::Destroy => {
                        running = false;
                    }
                    _ => {}
                },
                PollEvent::Wake => {}
                _ => {}
            }
        });

        if !running {
            break;
        }

        // No surface — wait for InitWindow
        if surface.is_none() {
            continue;
        }

        // Process touch / key input — must drain after poll_events returns
        let input_events = collect_input_events(&android_app, &mut gesture);
        for event in &input_events {
            let actions = app.handle_event(event.clone(), window_size, scale_factor);
            process_actions(&actions, &mut running);
        }

        // Drain cross-thread callbacks
        drain_main_queue();
        rinch_core::reactive::drain_polls();

        // Check if app has pending layout (signal changes create pending updates)
        let has_momentum = gesture.velocity_x.abs() > MOMENTUM_MIN_VELOCITY
            || gesture.velocity_y.abs() > MOMENTUM_MIN_VELOCITY;
        let needs_paint = REDRAW_PENDING.swap(false, Ordering::AcqRel)
            || app.has_pending_layout()
            || has_momentum;

        if needs_paint && mounted {
            if let Some(ref mut s) = surface {
                // Resolve any pending layout changes from signal updates
                if app.has_pending_layout() {
                    app.resolve_and_repaint(window_size.0 as f32, window_size.1 as f32);
                }
                let (pixels, w, h) = app.build_pixels(1.0, window_size, false);
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

// ── Input translation ────────────────────────────────────────────────────────

fn collect_input_events(
    android_app: &AndroidApp,
    gesture: &mut TouchGesture,
) -> Vec<PlatformEvent> {
    let mut events = Vec::new();

    match android_app.input_events_iter() {
        Ok(mut iter) => {
            while iter.next(|input_event| {
                use android_activity::input::InputEvent;
                match input_event {
                    InputEvent::MotionEvent(motion) => {
                        let ptr = motion.pointer_at_index(0);
                        gesture.process(motion.action(), ptr.x(), ptr.y(), &mut events);
                    }
                    InputEvent::KeyEvent(key) => {
                        if key.action() == KeyAction::Down {
                            if let Some(key_code) = map_android_keycode(key.key_code()) {
                                events.push(PlatformEvent::KeyDown {
                                    key: key_code,
                                    text: None,
                                    modifiers: Modifiers::default(),
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
        AK::Enter | AK::NumpadEnter => Some(KeyCode::Enter),
        AK::Tab => Some(KeyCode::Tab),
        AK::Escape | AK::Back => Some(KeyCode::Escape),
        AK::Del => Some(KeyCode::Backspace),
        AK::ForwardDel => Some(KeyCode::Delete),
        AK::Space => Some(KeyCode::Space),
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
