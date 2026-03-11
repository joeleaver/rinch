//! AV section - Audio/Video device controls.
//!
//! Demonstrates rinch-av integration: real camera preview via RenderSurface,
//! live microphone level, mic test (record & playback), and device selection.

use rinch::prelude::*;
use rinch_av::DeviceInfo;
use rinch_av::audio::{
    AudioInputConfig, AudioOutputConfig, audio_input_devices, audio_output_devices,
    open_audio_input, open_audio_output,
};
use rinch_av::camera::{CameraConfig, camera_devices, open_camera};
use rinch_screen_capture::{CaptureConfig, ScreenCapture};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// State for the AV section.
#[derive(Clone)]
pub struct AvSectionState {
    // Toggles
    pub mic_muted: Signal<bool>,
    pub camera_off: Signal<bool>,

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
    // Enumerate real devices
    let mics = audio_input_devices().unwrap_or_default();
    let speakers = audio_output_devices().unwrap_or_default();
    let cameras = camera_devices().unwrap_or_default();

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
        camera_off: Signal::new(true), // Start with camera off
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
    let camera_off = state.camera_off;
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

    // ── Camera preview via RenderSurface ──
    let camera_surface = create_render_surface();
    let camera_writer = camera_surface.writer();

    // Find the default camera device ID for opening
    let camera_list = state.camera_devices.get();
    let default_camera_id = camera_list
        .iter()
        .find(|d| d.is_default)
        .or(camera_list.first())
        .map(|d| d.id.clone());

    // Start camera capture on a background thread (open_camera blocks until init).
    // The thread keeps the CameraCapture handle alive — dropping it stops capture.
    std::thread::Builder::new()
        .name("rinch-av-cam-init".into())
        .spawn(move || {
            let config = CameraConfig {
                width: 640,
                height: 480,
                fps: 30,
                format: None,
            };
            let result = if let Some(ref device_id) = default_camera_id {
                rinch_av::camera::open_camera_on(device_id, config, move |frame| {
                    camera_writer.submit_frame(&frame.data, frame.width, frame.height);
                })
            } else {
                open_camera(config, move |frame| {
                    camera_writer.submit_frame(&frame.data, frame.width, frame.height);
                })
            };
            match result {
                Ok(_capture) => {
                    // Keep thread alive so CameraCapture stays alive (stops on drop)
                    loop {
                        std::thread::sleep(std::time::Duration::from_secs(60));
                    }
                }
                Err(e) => {
                    eprintln!("Camera open failed: {}", e);
                }
            }
        })
        .ok();

    // ── Microphone level via audio input ──
    // Use atomic for lock-free communication from audio thread to main thread
    let mic_level_atomic = Arc::new(AtomicU32::new(0));
    let mic_level_writer = mic_level_atomic.clone();
    let mic_active = Signal::new(false);

    // Shared ring buffer for mic test recording (5 seconds at 48kHz mono)
    let max_samples = 48000 * 5;
    let recording_buf: Arc<std::sync::Mutex<Vec<f32>>> =
        Arc::new(std::sync::Mutex::new(Vec::with_capacity(max_samples)));
    let recording_flag = Arc::new(AtomicBool::new(false));

    // Open audio input — cpal runs its own audio thread, leak handle to keep alive
    {
        let recording_buf_writer = recording_buf.clone();
        let recording_flag_reader = recording_flag.clone();
        let config = AudioInputConfig {
            sample_rate: 48000,
            channels: 1,
            buffer_size: 1024,
        };
        match open_audio_input(config, move |samples: &[f32]| {
            // Compute RMS level
            let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
            let level = (rms * 5.0).min(1.0);
            mic_level_writer.store(level.to_bits(), Ordering::Relaxed);

            // Record samples if mic test is active
            if recording_flag_reader.load(Ordering::Relaxed)
                && let Ok(mut buf) = recording_buf_writer.try_lock()
            {
                let remaining = max_samples.saturating_sub(buf.len());
                if remaining > 0 {
                    let take = remaining.min(samples.len());
                    buf.extend_from_slice(&samples[..take]);
                }
            }
        }) {
            Ok(stream) => {
                mic_active.set(true);
                std::mem::forget(stream);
            }
            Err(e) => {
                eprintln!("Audio input open failed: {}", e);
            }
        }
    }

    // Poll the atomic audio level into the signal from a background thread.
    // Signal::send() automatically dispatches to the main thread.
    {
        let mic_level_reader = mic_level_atomic.clone();
        std::thread::Builder::new()
            .name("mic-level-poll".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    let bits = mic_level_reader.load(Ordering::Relaxed);
                    let level = f32::from_bits(bits);
                    audio_level.send(level);
                }
            })
            .ok();
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
                    div { style: "position: relative; width: 100%; height: 400px; background: var(--rinch-color-dark-7, #1a1b1e);",

                        RenderSurface {
                            surface: Some(camera_surface.clone()),
                            style: "width: 100%; height: 100%;",
                        }

                        // Camera off overlay
                        if camera_off.get() {
                            div { style: "position: absolute; top: 0; left: 0; width: 100%; height: 100%; background: var(--rinch-color-dark-7, #1a1b1e); display: flex; align-items: center; justify-content: center; z-index: 1;",
                                div { style: "display: flex; flex-direction: column; align-items: center; gap: 12px;",
                                    div { style: "width: 80px; height: 80px; border-radius: 50%; background: var(--rinch-color-dark-5, #373A40); display: flex; align-items: center; justify-content: center;",
                                        div { style: "width: 36px; height: 36px; color: var(--rinch-color-dimmed);",
                                            {render_tabler_icon(__scope, TablerIcon::VideoOff, TablerIconStyle::Outline)}
                                        }
                                    }
                                    Text { size: "sm", color: "dimmed", "Camera is off" }
                                }
                            }
                        }

                        // Mute indicator overlay (bottom-left)
                        if mic_muted.get() {
                            div { style: "position: absolute; bottom: 12px; left: 12px; background: rgba(0,0,0,0.6); padding: 4px 10px; border-radius: 6px; display: flex; align-items: center; gap: 6px;",
                                div { style: "width: 14px; height: 14px; color: #ff6b6b;",
                                    {render_tabler_icon(__scope, TablerIcon::MicrophoneOff, TablerIconStyle::Outline)}
                                }
                                Text { size: "xs", style: "color: white;", "Muted" }
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

                        // Camera toggle
                        {toolbar_button(
                            __scope,
                            TablerIcon::Video,
                            TablerIcon::VideoOff,
                            camera_off,
                            "red",
                        )}

                        // Divider
                        div { style: "width: 1px; background: var(--rinch-color-dark-4, #ced4da); margin: 4px 4px;" }

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
                                    color: {|| if mic_muted.get() { "red" } else if mic_active.get() { "green" } else { "gray" }},
                                    variant: "light",
                                    {|| if mic_muted.get() { "Muted" } else if mic_active.get() { "Active" } else { "Unavailable" }}
                                }
                            }

                            // Level bar
                            {audio_level_bar(__scope, audio_level, mic_muted)}

                            Space { h: "xs" }

                            // Mic test
                            Text { size: "sm", weight: "500", "Mic Test" }
                            Text { size: "xs", color: "dimmed",
                                "Record a short clip, then play it back to check your mic."
                            }
                            {mic_test_controls(__scope, start_recording.clone(), stop_recording.clone(), play_recording.clone(), mic_test_recording, mic_test_playing, mic_test_has_clip)}
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
                        if camera_off.get() {
                            div { style: "width: 100%; height: 100%; display: flex; align-items: center; justify-content: center;",
                                Text { color: "dimmed", "Camera is off — toggle camera to preview" }
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
    let btn = __scope.create_element("button");
    btn.set_attribute("type", "button");
    let handler_id = __scope.register_handler(move || toggled.update(|v| *v = !*v));
    btn.set_attribute("data-rid", &handler_id.0.to_string());

    let on_icon = render_tabler_icon(__scope, icon_on, TablerIconStyle::Outline);
    let off_icon = render_tabler_icon(__scope, icon_off, TablerIconStyle::Outline);
    btn.append_child(&on_icon);
    btn.append_child(&off_icon);

    __scope.create_effect({
        let btn = btn.clone();
        let on_icon = on_icon.clone();
        let off_icon = off_icon.clone();
        let active_color = active_color.to_string();
        move || {
            if toggled.get() {
                btn.set_attribute(
                    "class",
                    "rinch-action-icon rinch-action-icon--filled rinch-action-icon--xl rinch-action-icon--radius-xl",
                );
                btn.set_attribute("style", &format!(
                    "--rinch-action-icon-color: var(--rinch-color-{}-6)", active_color
                ));
                on_icon.set_attribute("style", "display: none");
                off_icon.set_attribute("style", "");
            } else {
                btn.set_attribute("class",
                    "rinch-action-icon rinch-action-icon--default rinch-action-icon--xl rinch-action-icon--radius-xl"
                );
                btn.set_attribute("style", "");
                on_icon.set_attribute("style", "");
                off_icon.set_attribute("style", "display: none");
            }
        }
    });

    btn
}

// ── Helper: mic test record/play controls ──
// Extracted to its own function so Callback values are passed as owned args
// (avoids FnOnce issues from reactive `if` blocks capturing non-Copy closures).

fn mic_test_controls(
    __scope: &mut RenderScope,
    start_recording: Callback,
    stop_recording: Callback,
    play_recording: Callback,
    recording: Signal<bool>,
    playing: Signal<bool>,
    has_clip: Signal<bool>,
) -> NodeHandle {
    // Dispatch record/stop/play based on state — all in one onclick per button
    // to avoid reactive component re-render + FnOnce issues with Callback.
    let record_btn = __scope.create_element("button");
    record_btn.set_attribute("class", "rinch-Button rinch-Button--light rinch-Button--xs");
    let record_text = __scope.create_text("Record");
    record_btn.append_child(&record_text);

    // Reactive text + style for record button
    __scope.create_effect({
        let btn = record_btn.clone();
        let text = record_text.clone();
        move || {
            if recording.get() {
                btn.set_attribute(
                    "class",
                    "rinch-Button rinch-Button--filled rinch-Button--xs rinch-Button--red",
                );
                text.set_text("Stop");
            } else {
                btn.set_attribute("class", "rinch-Button rinch-Button--light rinch-Button--xs");
                text.set_text("Record");
            }
            if playing.get() {
                btn.set_attribute("data-disabled", "true");
            } else {
                btn.remove_attribute("data-disabled");
            }
        }
    });

    // Record button click handler
    let handler_id = __scope.register_handler({
        let start = start_recording;
        let stop = stop_recording;
        move || {
            if recording.get() {
                stop.invoke();
            } else {
                start.invoke();
            }
        }
    });
    record_btn.set_attribute("data-rid", &handler_id.0.to_string());

    // Play button
    let play_btn = __scope.create_element("button");
    play_btn.set_attribute("class", "rinch-Button rinch-Button--light rinch-Button--xs");
    let play_text = __scope.create_text("Play");
    play_btn.append_child(&play_text);

    __scope.create_effect({
        let btn = play_btn.clone();
        let text = play_text.clone();
        move || {
            if playing.get() {
                text.set_text("Playing…");
                btn.set_attribute("data-disabled", "true");
            } else {
                text.set_text("Play");
                if !has_clip.get() || recording.get() {
                    btn.set_attribute("data-disabled", "true");
                } else {
                    btn.remove_attribute("data-disabled");
                }
            }
        }
    });

    let play_handler_id = __scope.register_handler(move || play_recording.invoke());
    play_btn.set_attribute("data-rid", &play_handler_id.0.to_string());

    // Recording status text
    let status = rsx! {
        if recording.get() {
            Text { size: "xs", color: "red", "Recording… (max 5s)" }
        }
    };

    rsx! {
        Fragment {
            Group { gap: "sm",
                {record_btn}
                {play_btn}
            }
            {status}
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
        div { style: "height: 8px; border-radius: 4px; background: var(--rinch-color-dark-4, #dee2e6); overflow: hidden;",
            div {
                style: {|| {
                    let pct = if muted.get() { 0.0 } else { level.get() * 100.0 };
                    let color = if pct > 80.0 { "#ff6b6b" } else if pct > 50.0 { "#fcc419" } else { "#51cf66" };
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
    let container = __scope.create_element("div");
    container.set_attribute("style", "display: flex; flex-direction: column; gap: 4px;");

    let device_list = devices.get();
    if device_list.is_empty() {
        let empty = rsx! {
            Text { size: "sm", color: "dimmed", style: "padding: 8px 12px;",
                "No devices found"
            }
        };
        container.append_child(&empty);
        return container;
    }

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
                            "padding: 8px 12px; border-radius: 6px; border: 1px solid var(--rinch-primary-color); background: var(--rinch-primary-color-light, rgba(34, 139, 230, 0.1)); cursor: pointer; display: flex; align-items: center; gap: 8px;"
                        } else {
                            "padding: 8px 12px; border-radius: 6px; border: 1px solid var(--rinch-color-dark-4, #dee2e6); cursor: pointer; display: flex; align-items: center; gap: 8px;"
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
                                "width: 16px; height: 16px; border-radius: 50%; border: 2px solid var(--rinch-color-dark-4, #ced4da);"
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

            std::thread::spawn(
                move || match ScreenCapture::start(CaptureConfig::default()) {
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
                },
            );
        }
    });

    rsx! {
        Paper { p: "0", radius: "lg", with_border: true,
            style: "overflow: hidden;",

            // Screen capture preview
            div { style: "position: relative; width: 100%; height: 400px; background: var(--rinch-color-dark-7, #1a1b1e);",
                RenderSurface {
                    surface: Some(screen_surface),
                    style: "width: 100%; height: 100%;",
                }

                // Placeholder overlay when not sharing
                if !sharing.get() {
                    div { style: "position: absolute; inset: 0; display: flex; align-items: center; justify-content: center;",
                        div { style: "display: flex; flex-direction: column; align-items: center; gap: 12px;",
                            div { style: "width: 80px; height: 80px; border-radius: 50%; background: var(--rinch-color-dark-5, #373A40); display: flex; align-items: center; justify-content: center;",
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
