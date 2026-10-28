//! AV section - Audio/Video device controls.
//!
//! Demonstrates rinch-av integration: camera preview via RenderSurface, live
//! microphone level, mic test (record & playback), and device selection.
//!
//! Opening the page opens no device: the camera and the microphone start
//! from their buttons (#1916). `tests/av_opens_no_device.rs` pins that.

use rinch::prelude::*;
use rinch_av::DeviceInfo;
use rinch_av::audio::{
    AudioInputConfig, AudioInputStream, AudioOutputConfig, audio_input_devices,
    audio_output_devices, open_audio_input, open_audio_output,
};
use rinch_av::camera::{CameraCapture, CameraConfig, camera_devices, open_camera, open_camera_on};
use rinch_screen_capture::{CaptureConfig, ScreenCapture};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// The camera while it is open. `generation` moves on at every start and
/// stop, so a capture that finishes opening after a stop (or after a newer
/// start) is dropped, which closes it.
#[derive(Clone, Default)]
struct CameraSlot {
    held: Arc<Mutex<Option<CameraCapture>>>,
    generation: Arc<AtomicU64>,
}

impl CameraSlot {
    fn close(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        let capture = self.held.lock().unwrap().take();
        drop(capture);
    }
}

/// State for the AV section.
#[derive(Clone)]
pub struct AvSectionState {
    // Toggles
    pub mic_muted: Signal<bool>,

    // Audio level (0.0 - 1.0) — updated from atomic on each frame
    pub audio_level: Signal<f32>,

    // Mic test state
    pub mic_test_recording: Signal<bool>,
    pub mic_test_playing: Signal<bool>,
    pub mic_test_has_clip: Signal<bool>,

    // Settings modals
    pub audio_settings_open: Signal<bool>,
    pub video_settings_open: Signal<bool>,

    // Device lists (populated from real hardware)
    pub mic_devices: Signal<Vec<DeviceInfo>>,
    pub speaker_devices: Signal<Vec<DeviceInfo>>,
    pub camera_devices: Signal<Vec<DeviceInfo>>,

    // Selected device names
    pub selected_mic: Signal<String>,
    pub selected_speaker: Signal<String>,
    pub selected_camera: Signal<String>,

    // Error message for device failures
    pub error_msg: Signal<String>,
}

pub fn init_av_state() {
    // Enumerate real devices. Listing opens nothing.
    init_av_state_with(
        audio_input_devices().unwrap_or_default(),
        audio_output_devices().unwrap_or_default(),
        camera_devices().unwrap_or_default(),
    );
}

/// [`init_av_state`] with the device lists given (tests list none).
pub fn init_av_state_with(
    mics: Vec<DeviceInfo>,
    speakers: Vec<DeviceInfo>,
    cameras: Vec<DeviceInfo>,
) {
    // Deduplicate devices by name (nokhwa/cpal can return the same device multiple times)
    fn dedup_devices(devices: Vec<DeviceInfo>) -> Vec<DeviceInfo> {
        let mut seen = std::collections::HashSet::new();
        devices
            .into_iter()
            .filter(|d| seen.insert(d.name.clone()))
            .collect()
    }

    let mics = dedup_devices(mics);
    let speakers = dedup_devices(speakers);
    let cameras = dedup_devices(cameras);

    // Extract default names before moving vecs into signals
    let default_mic_name = mics
        .iter()
        .find(|d| d.is_default)
        .or(mics.first())
        .map(|d| d.name.clone())
        .unwrap_or_default();
    let default_speaker_name = speakers
        .iter()
        .find(|d| d.is_default)
        .or(speakers.first())
        .map(|d| d.name.clone())
        .unwrap_or_default();
    let default_camera_name = cameras
        .iter()
        .find(|d| d.is_default)
        .or(cameras.first())
        .map(|d| d.name.clone())
        .unwrap_or_default();

    create_store(AvSectionState {
        mic_muted: Signal::new(false),
        audio_level: Signal::new(0.0),
        mic_test_recording: Signal::new(false),
        mic_test_playing: Signal::new(false),
        mic_test_has_clip: Signal::new(false),
        audio_settings_open: Signal::new(false),
        video_settings_open: Signal::new(false),
        mic_devices: Signal::new(mics),
        speaker_devices: Signal::new(speakers),
        camera_devices: Signal::new(cameras),
        selected_mic: Signal::new(default_mic_name),
        selected_speaker: Signal::new(default_speaker_name),
        selected_camera: Signal::new(default_camera_name),
        error_msg: Signal::new(String::new()),
    });
}

#[component]
pub fn av_section() -> NodeHandle {
    let state = use_store::<AvSectionState>();

    let mic_muted = state.mic_muted;
    let audio_level = state.audio_level;
    let mic_test_recording = state.mic_test_recording;
    let mic_test_playing = state.mic_test_playing;
    let mic_test_has_clip = state.mic_test_has_clip;
    let audio_settings_open = state.audio_settings_open;
    let video_settings_open = state.video_settings_open;
    let selected_mic = state.selected_mic;
    let selected_speaker = state.selected_speaker;
    let selected_camera = state.selected_camera;
    let error_msg = state.error_msg;

    // Nothing here opens a device while the page renders (#1916): the camera
    // opens from "Start camera" (or the toolbar's camera button) and the
    // microphone from "Start microphone", and both close from the same
    // buttons and when the section unmounts. Listing devices (init_av_state)
    // opens nothing.

    // ── Camera preview via RenderSurface ──
    let camera_surface = create_render_surface();
    let camera_on = Signal::new(false);
    let camera = CameraSlot::default();

    let start_camera = {
        let camera = camera.clone();
        let surface = camera_surface.clone();
        let camera_devices = state.camera_devices;
        Callback::new(move || {
            let generation = camera.generation.fetch_add(1, Ordering::SeqCst) + 1;
            camera_on.set(true);
            error_msg.set(String::new());
            let wanted = selected_camera.get();
            let device = camera_devices
                .get()
                .into_iter()
                .find(|d| d.name == wanted)
                .map(|d| d.id);
            let writer = surface.writer();
            let camera = camera.clone();
            // open_camera blocks until the device is up, so on desktop it runs
            // on a thread of its own; the web's returns at once.
            let open = move || {
                let config = CameraConfig {
                    width: 640,
                    height: 480,
                    fps: 30,
                    format: None,
                };
                let on_frame = move |frame: rinch_av::camera::VideoFrame| {
                    writer.submit_frame(&frame.data, frame.width, frame.height);
                };
                let result = match device {
                    Some(id) => open_camera_on(&id, config, on_frame),
                    None => open_camera(config, on_frame),
                };
                let current = camera.generation.load(Ordering::SeqCst) == generation;
                match result {
                    // Stopped while it was opening: dropping it closes it.
                    Ok(capture) if current => {
                        *camera.held.lock().unwrap() = Some(capture);
                    }
                    Ok(_) => {}
                    Err(e) if current => {
                        camera_on.send(false);
                        error_msg.send(format!("Camera: {e}"));
                    }
                    Err(_) => {}
                }
            };
            #[cfg(not(target_arch = "wasm32"))]
            {
                let _ = std::thread::Builder::new()
                    .name("rinch-av-cam-init".into())
                    .spawn(open);
            }
            #[cfg(target_arch = "wasm32")]
            open();
        })
    };
    let stop_camera = {
        let camera = camera.clone();
        Callback::new(move || {
            camera.close();
            camera_on.set(false);
        })
    };
    let toggle_camera = {
        let (start, stop) = (start_camera.clone(), stop_camera.clone());
        Callback::new(move || {
            if camera_on.get() {
                stop.invoke()
            } else {
                start.invoke()
            }
        })
    };

    // ── Microphone level via audio input ──
    // Use atomic for lock-free communication from audio thread to main thread
    let mic_level_atomic = Arc::new(AtomicU32::new(0));
    let mic_on = Signal::new(false);
    let mic: Rc<RefCell<Option<AudioInputStream>>> = Rc::default();

    // Shared ring buffer for mic test recording (5 seconds at 48kHz mono)
    let max_samples = 48000 * 5;
    let recording_buf: Arc<std::sync::Mutex<Vec<f32>>> =
        Arc::new(std::sync::Mutex::new(Vec::with_capacity(max_samples)));
    let recording_flag = Arc::new(AtomicBool::new(false));

    let start_mic = {
        let mic = mic.clone();
        let mic_level_writer = mic_level_atomic.clone();
        let recording_buf_writer = recording_buf.clone();
        let recording_flag_reader = recording_flag.clone();
        Callback::new(move || {
            let config = AudioInputConfig {
                sample_rate: 48000,
                channels: 1,
                buffer_size: 1024,
            };
            let level = mic_level_writer.clone();
            let buf = recording_buf_writer.clone();
            let flag = recording_flag_reader.clone();
            match open_audio_input(config, move |samples: &[f32]| {
                // Compute RMS level
                let rms =
                    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
                level.store((rms * 5.0).min(1.0).to_bits(), Ordering::Relaxed);

                // Record samples if mic test is active
                if flag.load(Ordering::Relaxed)
                    && let Ok(mut buf) = buf.try_lock()
                {
                    let remaining = max_samples.saturating_sub(buf.len());
                    if remaining > 0 {
                        let take = remaining.min(samples.len());
                        buf.extend_from_slice(&samples[..take]);
                    }
                }
            }) {
                Ok(stream) => {
                    *mic.borrow_mut() = Some(stream);
                    mic_on.set(true);
                    error_msg.set(String::new());
                }
                Err(e) => error_msg.set(format!("Microphone: {e}")),
            }
        })
    };
    let stop_mic = {
        let mic = mic.clone();
        let level = mic_level_atomic.clone();
        Callback::new(move || {
            mic.borrow_mut().take();
            level.store(0, Ordering::Relaxed);
            mic_on.set(false);
        })
    };
    let toggle_mic = Callback::new(move || {
        if mic_on.get() {
            stop_mic.invoke()
        } else {
            start_mic.invoke()
        }
    });

    // Release both devices when the section goes away.
    {
        let (camera, mic) = (camera.clone(), mic.clone());
        __scope.on_cleanup(move || {
            camera.close();
            mic.borrow_mut().take();
        });
    }

    // Poll the atomic audio level into the signal, on the UI thread. Armed
    // during the render, so it stops when the section unmounts.
    {
        let mic_level_reader = mic_level_atomic.clone();
        let last = std::cell::Cell::new(u32::MAX);
        set_interval(50, move || {
            let bits = mic_level_reader.load(Ordering::Relaxed);
            if bits != last.replace(bits) {
                audio_level.set(f32::from_bits(bits));
            }
        });
    }

    // Mic test: start/stop recording, then play back
    let recording_buf_for_record = recording_buf.clone();
    let recording_flag_for_record = recording_flag.clone();
    let recording_buf_for_play = recording_buf.clone();

    let start_recording = {
        let recording_buf = recording_buf_for_record.clone();
        let recording_flag = recording_flag_for_record.clone();
        Callback::new(move || {
            // Clear previous recording
            if let Ok(mut buf) = recording_buf.lock() {
                buf.clear();
            }
            recording_flag.store(true, Ordering::Relaxed);
            mic_test_recording.set(true);
            mic_test_has_clip.set(false);

            // Auto-stop after 5 seconds
            let flag = recording_flag.clone();
            std::thread::Builder::new()
                .name("mic-test-timer".into())
                .spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    flag.store(false, Ordering::Relaxed);
                    mic_test_recording.send(false);
                    mic_test_has_clip.send(true);
                })
                .ok();
        })
    };

    let stop_recording = {
        let flag = recording_flag.clone();
        Callback::new(move || {
            flag.store(false, Ordering::Relaxed);
            mic_test_recording.set(false);
            mic_test_has_clip.set(true);
        })
    };

    let play_recording = {
        let recording_buf = recording_buf_for_play.clone();
        Callback::new(move || {
            let samples = if let Ok(buf) = recording_buf.lock() {
                buf.clone()
            } else {
                return;
            };
            if samples.is_empty() {
                return;
            }

            mic_test_playing.set(true);

            std::thread::Builder::new()
                .name("mic-test-playback".into())
                .spawn(move || {
                    let mut pos = 0usize;
                    let config = AudioOutputConfig {
                        sample_rate: 48000,
                        channels: 1,
                        buffer_size: 1024,
                    };
                    let total = samples.len();
                    let done = Arc::new(AtomicBool::new(false));
                    let done_writer = done.clone();
                    match open_audio_output(config, move |buf: &mut [f32]| {
                        for s in buf.iter_mut() {
                            if pos < total {
                                *s = samples[pos];
                                pos += 1;
                            } else {
                                *s = 0.0;
                                done_writer.store(true, Ordering::Relaxed);
                            }
                        }
                    }) {
                        Ok(_stream) => {
                            // Wait for playback to finish
                            while !done.load(Ordering::Relaxed) {
                                std::thread::sleep(std::time::Duration::from_millis(50));
                            }
                            // Small tail to flush audio buffer
                            std::thread::sleep(std::time::Duration::from_millis(100));
                            mic_test_playing.send(false);
                        }
                        Err(e) => {
                            eprintln!("Playback failed: {}", e);
                            mic_test_playing.send(false);
                        }
                    }
                })
                .ok();
        })
    };

    // Clone for use in video settings modal
    let camera_surface2 = camera_surface.clone();

    rsx! {
        Fragment {
            // Header
            Title { order: 1, "Audio / Video" }
            Text { color: "dimmed", size: "lg",
                "Camera preview, microphone controls, and device selection — powered by rinch-av."
            }
            Space { h: "xl" }

            // Error banner
            if !error_msg.get().is_empty() {
                Alert {
                    color: "red",
                    title: "Device Error",
                    {|| error_msg.get()}
                }
            }

            // ── Main layout: camera preview + sidebar ──
            div { style: "display: flex; gap: 24px; align-items: flex-start;",

                // Camera preview area
                Paper { p: "0", radius: "lg", with_border: true,
                    style: "flex: 1; overflow: hidden; min-width: 0;",

                    // Camera view
                    div { style: "position: relative; width: 100%; height: 400px; background: var(--rinch-color-body);",

                        RenderSurface {
                            surface: Some(camera_surface.clone()),
                            style: "width: 100%; height: 100%;",
                        }

                        // Camera off overlay
                        if !camera_on.get() {
                            div { style: "position: absolute; top: 0; left: 0; width: 100%; height: 100%; background: var(--rinch-color-body); display: flex; align-items: center; justify-content: center;",
                                div { style: "display: flex; flex-direction: column; align-items: center; gap: 12px;",
                                    div { style: "width: 80px; height: 80px; border-radius: 50%; background: var(--rinch-color-default); display: flex; align-items: center; justify-content: center;",
                                        div { style: "width: 36px; height: 36px; color: var(--rinch-color-dimmed);",
                                            {render_tabler_icon(__scope, TablerIcon::VideoOff, TablerIconStyle::Outline)}
                                        }
                                    }
                                    Text { size: "sm", color: "dimmed", "Camera is off: Start camera below" }
                                }
                            }
                        }

                        // Mute indicator overlay (bottom-left)
                        if mic_muted.get() {
                            div { style: "position: absolute; bottom: 12px; left: 12px; background: rgba(0,0,0,0.6); padding: 4px 10px; border-radius: 6px; display: flex; align-items: center; gap: 6px;",
                                div { style: "width: 14px; height: 14px; color: var(--rinch-color-red-6);",
                                    {render_tabler_icon(__scope, TablerIcon::MicrophoneOff, TablerIconStyle::Outline)}
                                }
                                Text { size: "xs", style: "color: var(--rinch-color-white);", "Muted" }
                            }
                        }
                    }

                    // ── Toolbar ──
                    div { style: "padding: 12px 16px; display: flex; justify-content: center; gap: 8px; background: var(--rinch-color-body);",
                        // Mic toggle
                        {toolbar_button(
                            __scope,
                            TablerIcon::Microphone,
                            TablerIcon::MicrophoneOff,
                            mic_muted,
                            "red",
                        )}

                        // Camera on/off: the only thing that opens the camera
                        Button {
                            variant: {|| if camera_on.get() { "light" } else { "filled" }},
                            color: {|| if camera_on.get() { "red" } else { "blue" }},
                            onclick: toggle_camera.clone(),
                            {render_tabler_icon(__scope, TablerIcon::Video, TablerIconStyle::Outline)}
                            {|| if camera_on.get() { " Stop camera" } else { " Start camera" }}
                        }

                        // Divider
                        div { style: "width: 1px; background: var(--rinch-color-border); margin: 4px 4px;" }

                        // Audio settings
                        ActionIcon {
                            size: "xl",
                            radius: "xl",
                            variant: "default",
                            onclick: move || audio_settings_open.set(true),
                            {render_tabler_icon(__scope, TablerIcon::Settings, TablerIconStyle::Outline)}
                        }

                        // Video settings
                        ActionIcon {
                            size: "xl",
                            radius: "xl",
                            variant: "default",
                            onclick: move || video_settings_open.set(true),
                            {render_tabler_icon(__scope, TablerIcon::Camera, TablerIconStyle::Outline)}
                        }
                    }
                }

                // ── Sidebar ──
                Stack { gap: "md", style: "width: 280px; flex-shrink: 0;",

                    // Microphone level meter (live from microphone)
                    Paper { p: "md", radius: "md", with_border: true,
                        Stack { gap: "sm",
                            Group { justify: "space-between",
                                Text { weight: "600", "Microphone" }
                                Badge {
                                    color: {|| if mic_muted.get() { "red" } else if mic_on.get() { "green" } else { "gray" }},
                                    variant: "light",
                                    {|| if mic_muted.get() { "Muted" } else if mic_on.get() { "Active" } else { "Off" }}
                                }
                            }

                            // The only thing that opens the microphone
                            Button {
                                size: "xs",
                                variant: {|| if mic_on.get() { "light" } else { "filled" }},
                                color: {|| if mic_on.get() { "red" } else { "blue" }},
                                onclick: toggle_mic.clone(),
                                {|| if mic_on.get() { "Stop microphone" } else { "Start microphone" }}
                            }

                            // Level bar
                            {audio_level_bar(__scope, audio_level, mic_muted)}

                            Space { h: "xs" }

                            // Mic test
                            Text { size: "sm", weight: "500", "Mic Test" }
                            Text { size: "xs", color: "dimmed",
                                "Start the microphone, record a short clip, then play it back."
                            }
                            {mic_test_controls(__scope, start_recording.clone(), stop_recording.clone(), play_recording.clone(), mic_on, mic_test_recording, mic_test_playing, mic_test_has_clip)}
                        }
                    }

                }
            }

            // ── Screen Sharing ──
            Space { h: "xl" }
            Title { order: 2, "Screen Sharing" }
            Text { color: "dimmed", size: "lg",
                "Capture your screen or a window via PipeWire + XDG Desktop Portal."
            }
            Space { h: "md" }

            ScreenShareDemo {}

            Space { h: "xl" }

            // ── Audio Settings Modal ──
            Modal {
                opened_fn: move || audio_settings_open.get(),
                onclose: move || audio_settings_open.set(false),
                title: "Audio Settings",
                size: "md",

                Stack { gap: "md",
                    // Microphone selection
                    Text { weight: "600", size: "sm", "Microphone" }
                    {device_selector(__scope, selected_mic, state.mic_devices)}

                    // Test mic level
                    Text { size: "xs", color: "dimmed", "Speak to test your microphone:" }
                    {audio_level_bar(__scope, audio_level, mic_muted)}

                    Divider {}

                    // Speaker selection
                    Text { weight: "600", size: "sm", "Speaker" }
                    {device_selector(__scope, selected_speaker, state.speaker_devices)}
                }

                Space { h: "lg" }
                Group { justify: "flex-end",
                    Button { onclick: move || audio_settings_open.set(false), "Done" }
                }
            }

            // ── Video Settings Modal ──
            Modal {
                opened_fn: move || video_settings_open.get(),
                onclose: move || video_settings_open.set(false),
                title: "Video Settings",
                size: "md",

                Stack { gap: "md",
                    // Camera preview in settings
                    div { style: "height: 280px; border-radius: 8px; overflow: hidden; background: var(--rinch-color-dark-7, #1a1b1e);",
                        if !camera_on.get() {
                            div { style: "width: 100%; height: 100%; display: flex; align-items: center; justify-content: center;",
                                Text { color: "dimmed", "Camera is off: start it to preview" }
                            }
                        } else {
                            RenderSurface {
                                surface: Some(camera_surface2.clone()),
                                style: "width: 100%; height: 100%;",
                            }
                        }
                    }

                    // Camera selection
                    Text { weight: "600", size: "sm", "Camera" }
                    {device_selector(__scope, selected_camera, state.camera_devices)}
                }

                Space { h: "lg" }
                Group { justify: "flex-end",
                    Button { onclick: move || video_settings_open.set(false), "Done" }
                }
            }
        }
    }
}

// ── Helper: toolbar toggle button ──

fn toolbar_button(
    __scope: &mut RenderScope,
    icon_on: TablerIcon,
    icon_off: TablerIcon,
    toggled: Signal<bool>,
    active_color: &'static str,
) -> NodeHandle {
    let on_icon = render_tabler_icon(__scope, icon_on, TablerIconStyle::Outline);
    let off_icon = render_tabler_icon(__scope, icon_off, TablerIconStyle::Outline);

    // Reactive icon visibility via effects on the pre-rendered icon handles
    let on_clone = on_icon.clone();
    let off_clone = off_icon.clone();
    __scope.create_effect(move || {
        if toggled.get() {
            on_clone.set_attribute("style", "display: none");
            off_clone.set_attribute("style", "");
        } else {
            on_clone.set_attribute("style", "");
            off_clone.set_attribute("style", "display: none");
        }
    });

    rsx! {
        button {
            r#type: "button",
            class: {|| if toggled.get() {
                "rinch-action-icon rinch-action-icon--filled rinch-action-icon--xl rinch-action-icon--radius-xl"
            } else {
                "rinch-action-icon rinch-action-icon--default rinch-action-icon--xl rinch-action-icon--radius-xl"
            }},
            style: {
                let active_color = active_color.to_string();
                move || if toggled.get() {
                    format!("--rinch-action-icon-color: var(--rinch-color-{}-6)", active_color)
                } else {
                    String::new()
                }
            },
            onclick: move || toggled.update(|v| *v = !*v),
            {on_icon}
            {off_icon}
        }
    }
}

// ── Helper: mic test record/play controls ──
// Extracted to its own function so Callback values are passed as owned args
// (avoids FnOnce issues from reactive `if` blocks capturing non-Copy closures).

#[allow(clippy::too_many_arguments)] // the signals it reads, each by name
fn mic_test_controls(
    __scope: &mut RenderScope,
    start_recording: Callback,
    stop_recording: Callback,
    play_recording: Callback,
    mic_on: Signal<bool>,
    recording: Signal<bool>,
    playing: Signal<bool>,
    has_clip: Signal<bool>,
) -> NodeHandle {
    // Build record button with reactive class/text via effects on handles
    let record_btn = rsx! {
        button {
            class: {|| if recording.get() {
                "rinch-Button rinch-Button--filled rinch-Button--xs rinch-Button--red"
            } else {
                "rinch-Button rinch-Button--light rinch-Button--xs"
            }},
            onclick: {
                let start = start_recording;
                let stop = stop_recording;
                move || {
                    if recording.get() {
                        stop.invoke();
                    } else if mic_on.get() {
                        start.invoke();
                    }
                }
            },
            {|| if recording.get() { "Stop" } else { "Record" }}
        }
    };

    // Reactive disabled state for record button
    let record_clone = record_btn.clone();
    __scope.create_effect(move || {
        if playing.get() || !mic_on.get() {
            record_clone.set_attribute("data-disabled", "true");
        } else {
            record_clone.remove_attribute("data-disabled");
        }
    });

    // Build play button with reactive class/text via effects on handles
    let play_btn = rsx! {
        button {
            class: "rinch-Button rinch-Button--light rinch-Button--xs",
            onclick: move || play_recording.invoke(),
            {|| if playing.get() { "Playing\u{2026}" } else { "Play" }}
        }
    };

    // Reactive disabled state for play button
    let play_clone = play_btn.clone();
    __scope.create_effect(move || {
        if playing.get() || !has_clip.get() || recording.get() {
            play_clone.set_attribute("data-disabled", "true");
        } else {
            play_clone.remove_attribute("data-disabled");
        }
    });

    rsx! {
        Fragment {
            Group { gap: "sm",
                {record_btn}
                {play_btn}
            }
            if recording.get() {
                Text { size: "xs", color: "red", "Recording\u{2026} (max 5s)" }
            }
        }
    }
}

// ── Helper: audio level bar ──

fn audio_level_bar(
    __scope: &mut RenderScope,
    level: Signal<f32>,
    muted: Signal<bool>,
) -> NodeHandle {
    rsx! {
        div { style: "height: 8px; border-radius: 4px; background: var(--rinch-color-border); overflow: hidden;",
            div {
                style: {|| {
                    let pct = if muted.get() { 0.0 } else { level.get() * 100.0 };
                    let color = if pct > 80.0 { "var(--rinch-color-red-6)" } else if pct > 50.0 { "var(--rinch-color-yellow-6)" } else { "var(--rinch-color-green-6)" };
                    format!(
                        "height: 100%; width: {pct}%; background: {color}; border-radius: 4px; transition: width 0.1s ease;",
                    )
                }},
            }
        }
    }
}

// ── Helper: device selector from real DeviceInfo list ──

fn device_selector(
    __scope: &mut RenderScope,
    selected: Signal<String>,
    devices: Signal<Vec<DeviceInfo>>,
) -> NodeHandle {
    let device_list = devices.get();
    if device_list.is_empty() {
        return rsx! {
            div { style: "display: flex; flex-direction: column; gap: 4px;",
                Text { size: "sm", color: "dimmed", style: "padding: 8px 12px;",
                    "No devices found"
                }
            }
        };
    }

    let container = rsx! {
        div { style: "display: flex; flex-direction: column; gap: 4px;" }
    };

    for device in device_list {
        let dev_name = device.name.clone();
        let dev_name2 = dev_name.clone();
        let dev_name3 = dev_name.clone();
        let dev_name_display = dev_name.clone();

        let item = rsx! {
            div {
                style: {
                    let d = dev_name.clone();
                    move || {
                        if selected.get() == d {
                            "padding: 8px 12px; border-radius: 6px; border: 1px solid var(--rinch-primary-color); background: var(--rinch-primary-color-light); cursor: pointer; display: flex; align-items: center; gap: 8px;"
                        } else {
                            "padding: 8px 12px; border-radius: 6px; border: 1px solid var(--rinch-color-border); cursor: pointer; display: flex; align-items: center; gap: 8px;"
                        }
                    }
                },
                onclick: {
                    let d = dev_name2.clone();
                    move || selected.set(d.clone())
                },
                div {
                    style: {
                        let d = dev_name3.clone();
                        move || {
                            let is_sel = selected.get() == d;
                            if is_sel {
                                "width: 16px; height: 16px; border-radius: 50%; border: 2px solid var(--rinch-primary-color); background: var(--rinch-primary-color);"
                            } else {
                                "width: 16px; height: 16px; border-radius: 50%; border: 2px solid var(--rinch-color-border);"
                            }
                        }
                    },
                }
                span { {dev_name_display.as_str()} }
            }
        };
        container.append_child(&item);
    }

    container
}

// ── Screen Sharing Demo ─────────────────────────────────────────────────────

#[component]
fn ScreenShareDemo() -> NodeHandle {
    let sharing = Signal::new(false);
    let status = Signal::new(String::new());
    let screen_surface = create_render_surface();

    // Hold the capture handle + pipe so they stay alive while sharing.
    // Use Arc<Mutex> so the background thread can store them.
    let capture_handle: Arc<std::sync::Mutex<Option<ScreenCapture>>> =
        Arc::new(std::sync::Mutex::new(None));
    let pipe_handle: Arc<std::sync::Mutex<Option<rinch_av::VideoPipe>>> =
        Arc::new(std::sync::Mutex::new(None));

    let writer = screen_surface.writer();
    let capture_ref = capture_handle.clone();
    let pipe_ref = pipe_handle.clone();

    let toggle = Callback::new(move || {
        if sharing.get() {
            // Stop: drop capture + pipe
            pipe_ref.lock().unwrap().take();
            capture_ref.lock().unwrap().take();
            sharing.set(false);
            status.set("Stopped".into());
        } else {
            status.set("Opening screen picker...".into());

            // ScreenCapture::start blocks for the portal dialog, so run on
            // a background thread. Once we have the capture, pipe frames to
            // the RenderSurface writer from the capture thread.
            let sharing_sig = sharing;
            let status_sig = status;
            let writer = writer.clone();
            let capture_ref = capture_ref.clone();
            let pipe_ref = pipe_ref.clone();

            // On WASM, ScreenCapture::start() handles async internally,
            // so no thread spawn needed. On desktop, use a thread to avoid
            // blocking the UI while the portal dialog is open.
            let start_capture = move || match ScreenCapture::start(CaptureConfig::default()) {
                Ok(capture) => {
                    let pipe = capture.video_track().pipe_to(move |frame| {
                        writer.submit_frame(&frame.data, frame.width, frame.height);
                    });
                    *pipe_ref.lock().unwrap() = Some(pipe);
                    *capture_ref.lock().unwrap() = Some(capture);
                    sharing_sig.send(true);
                    status_sig.send("Sharing".into());
                }
                Err(rinch_screen_capture::CaptureError::Cancelled) => {
                    status_sig.send("Cancelled".into());
                }
                Err(e) => {
                    status_sig.send(format!("Error: {e}"));
                }
            };
            #[cfg(not(target_arch = "wasm32"))]
            std::thread::spawn(start_capture);
            #[cfg(target_arch = "wasm32")]
            start_capture();
        }
    });

    rsx! {
        Paper { p: "0", radius: "lg", with_border: true,
            style: "overflow: hidden;",

            // Screen capture preview
            div { style: "position: relative; width: 100%; height: 400px; background: var(--rinch-color-body);",
                RenderSurface {
                    surface: Some(screen_surface),
                    style: "width: 100%; height: 100%;",
                }

                // Placeholder overlay when not sharing
                if !sharing.get() {
                    div { style: "position: absolute; inset: 0; display: flex; align-items: center; justify-content: center;",
                        div { style: "display: flex; flex-direction: column; align-items: center; gap: 12px;",
                            div { style: "width: 80px; height: 80px; border-radius: 50%; background: var(--rinch-color-default); display: flex; align-items: center; justify-content: center;",
                                div { style: "width: 36px; height: 36px; color: var(--rinch-color-dimmed);",
                                    {render_tabler_icon(__scope, TablerIcon::ScreenShare, TablerIconStyle::Outline)}
                                }
                            }
                            Text { size: "sm", color: "dimmed", "Click below to share your screen" }
                        }
                    }
                }
            }

            // Toolbar
            div { style: "padding: 12px 16px; display: flex; align-items: center; gap: 12px; background: var(--rinch-color-body);",
                Button {
                    variant: {|| if sharing.get() { "filled" } else { "light" }},
                    color: {|| if sharing.get() { "red" } else { "blue" }},
                    onclick: toggle.clone(),
                    {render_tabler_icon(__scope, TablerIcon::ScreenShare, TablerIconStyle::Outline)}
                    {|| if sharing.get() { " Stop Sharing" } else { " Share Screen" }}
                }

                Text { size: "sm", color: "dimmed", {|| status.get()} }
            }
        }
    }
}
