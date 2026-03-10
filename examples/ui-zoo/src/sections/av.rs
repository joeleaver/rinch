//! AV section - Audio/Video device controls (Zoom-style).
//!
//! Demonstrates rinch-av integration: real camera preview via RenderSurface,
//! live microphone level, device enumeration, and Zoom-like toolbar controls.

use rinch::prelude::*;
use rinch_av::DeviceInfo;
use rinch_av::audio::{
    AudioInputConfig, audio_input_devices, audio_output_devices, open_audio_input,
};
use rinch_av::camera::{CameraConfig, camera_devices, open_camera};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// State for the AV section.
#[derive(Clone)]
pub struct AvSectionState {
    // Toggles
    pub mic_muted: Signal<bool>,
    pub camera_off: Signal<bool>,
    pub screen_sharing: Signal<bool>,
    pub hand_raised: Signal<bool>,

    // Audio level (0.0 - 1.0) — updated from atomic on each frame
    pub audio_level: Signal<f32>,

    // Device selection
    pub audio_settings_open: Signal<bool>,
    pub video_settings_open: Signal<bool>,

    // Status
    pub call_active: Signal<bool>,
    pub call_duration: Signal<u32>,

    // Device lists (populated from real hardware)
    pub mic_devices: Signal<Vec<DeviceInfo>>,
    pub speaker_devices: Signal<Vec<DeviceInfo>>,
    pub camera_devices: Signal<Vec<DeviceInfo>>,

    // Selected device names
    pub selected_mic: Signal<String>,
    pub selected_speaker: Signal<String>,
    pub selected_camera: Signal<String>,

    // Volume controls (0-100)
    pub input_volume: Signal<f64>,
    pub output_volume: Signal<f64>,

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
        screen_sharing: Signal::new(false),
        hand_raised: Signal::new(false),
        audio_level: Signal::new(0.0),
        audio_settings_open: Signal::new(false),
        video_settings_open: Signal::new(false),
        call_active: Signal::new(false),
        call_duration: Signal::new(0),
        mic_devices: Signal::new(mics),
        speaker_devices: Signal::new(speakers),
        camera_devices: Signal::new(cameras),
        selected_mic: Signal::new(default_mic_name),
        selected_speaker: Signal::new(default_speaker_name),
        selected_camera: Signal::new(default_camera_name),
        input_volume: Signal::new(75.0),
        output_volume: Signal::new(80.0),
        error_msg: Signal::new(String::new()),
    });
}

#[component]
pub fn av_section() -> NodeHandle {
    let state = use_store::<AvSectionState>();

    let (
        mic_muted,
        camera_off,
        screen_sharing,
        hand_raised,
        audio_level,
        audio_settings_open,
        video_settings_open,
        call_active,
        call_duration,
        selected_mic,
        selected_speaker,
        selected_camera,
        input_volume,
        output_volume,
        error_msg,
    ) = (
        state.mic_muted,
        state.camera_off,
        state.screen_sharing,
        state.hand_raised,
        state.audio_level,
        state.audio_settings_open,
        state.video_settings_open,
        state.call_active,
        state.call_duration,
        state.selected_mic,
        state.selected_speaker,
        state.selected_camera,
        state.input_volume,
        state.output_volume,
        state.error_msg,
    );

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

    // Open audio input — cpal runs its own audio thread, leak handle to keep alive
    {
        let config = AudioInputConfig {
            sample_rate: 48000,
            channels: 1,
            buffer_size: 1024,
        };
        match open_audio_input(config, move |samples: &[f32]| {
            let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
            let level = (rms * 5.0).min(1.0);
            mic_level_writer.store(level.to_bits(), Ordering::Relaxed);
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

    // Poll the atomic audio level into the signal using the camera surface's
    // render callback (fires every frame when camera is active)
    let mic_level_reader = mic_level_atomic.clone();
    camera_surface.set_render_callback(move |_writer, _w, _h| {
        let bits = mic_level_reader.load(Ordering::Relaxed);
        let level = f32::from_bits(bits);
        // Only update signal if value changed meaningfully (avoid unnecessary repaints)
        let current = audio_level.get();
        if (level - current).abs() > 0.005 {
            audio_level.set(level);
        }
    });

    // Clone for use in video settings modal (rsx moves the handle into closures)
    let camera_surface2 = camera_surface.clone();

    rsx! {
        Fragment {
            // Header
            Title { order: 1, "Audio / Video" }
            Text { color: "dimmed", size: "lg",
                "Camera, microphone, and speaker controls powered by rinch-av. Zoom-style toolbar with device settings."
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

                    // Camera view — RenderSurface always visible at fixed size
                    div { style: "position: relative; width: 100%; height: 400px; background: var(--rinch-color-dark-7, #1a1b1e);",

                        // Always-visible RenderSurface (camera frames render here)
                        RenderSurface {
                            surface: Some(camera_surface.clone()),
                            style: "width: 100%; height: 100%;",
                        }

                        // Camera off overlay (floats on top when camera is off)
                        if camera_off.get() {
                            div { style: "position: absolute; top: 0; left: 0; width: 100%; height: 100%; background: var(--rinch-color-dark-7, #1a1b1e); display: flex; align-items: center; justify-content: center; z-index: 1;",
                                div { style: "display: flex; flex-direction: column; align-items: center; gap: 12px;",
                                    div { style: "width: 80px; height: 80px; border-radius: 50%; background: var(--rinch-color-dark-5, #373A40); display: flex; align-items: center; justify-content: center;",
                                        Text { size: "xl", weight: "bold", color: "dimmed", "JD" }
                                    }
                                    Text { size: "sm", color: "dimmed", "Camera is off" }
                                }
                            }
                        }

                        // Name tag (bottom-left)
                        div { style: "position: absolute; bottom: 12px; left: 12px; background: rgba(0,0,0,0.6); padding: 4px 10px; border-radius: 6px; display: flex; align-items: center; gap: 6px;",
                            Text { size: "xs", style: "color: white;", "You" }
                            if mic_muted.get() {
                                div { style: "width: 14px; height: 14px; color: #ff6b6b;",
                                    {render_tabler_icon(__scope, TablerIcon::MicrophoneOff, TablerIconStyle::Outline)}
                                }
                            }
                        }

                        // Duration overlay (top-right) when call active
                        if call_active.get() {
                            div { style: "position: absolute; top: 12px; right: 12px; background: rgba(0,0,0,0.6); padding: 4px 10px; border-radius: 6px;",
                                Text { size: "xs", style: "color: white;",
                                    {|| {
                                        let secs = call_duration.get();
                                        format!("{:02}:{:02}", secs / 60, secs % 60)
                                    }}
                                }
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
                            "Mute",
                            "Unmute",
                        )}

                        // Camera toggle
                        {toolbar_button(
                            __scope,
                            TablerIcon::Video,
                            TablerIcon::VideoOff,
                            camera_off,
                            "red",
                            "Stop Video",
                            "Start Video",
                        )}

                        // Screen share toggle
                        ActionIcon {
                            size: "xl",
                            radius: "xl",
                            variant: {|| if screen_sharing.get() { "filled" } else { "default" }},
                            color: {|| if screen_sharing.get() { "green" } else { "" }},
                            onclick: move || screen_sharing.update(|v| *v = !*v),
                            {render_tabler_icon(__scope, TablerIcon::ScreenShare, TablerIconStyle::Outline)}
                        }

                        // Raise hand
                        ActionIcon {
                            size: "xl",
                            radius: "xl",
                            variant: {|| if hand_raised.get() { "filled" } else { "default" }},
                            color: {|| if hand_raised.get() { "yellow" } else { "" }},
                            onclick: move || hand_raised.update(|v| *v = !*v),
                            {render_tabler_icon(__scope, TablerIcon::HandStop, TablerIconStyle::Outline)}
                        }

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

                        // End/Start call
                        ActionIcon {
                            size: "xl",
                            radius: "xl",
                            variant: "filled",
                            color: {|| if call_active.get() { "red" } else { "green" }},
                            onclick: move || {
                                call_active.update(|v| *v = !*v);
                                if !call_active.get() {
                                    call_duration.set(0);
                                }
                            },
                            if call_active.get() {
                                {render_tabler_icon(__scope, TablerIcon::PhoneOff, TablerIconStyle::Outline)}
                            } else {
                                {render_tabler_icon(__scope, TablerIcon::Phone, TablerIconStyle::Outline)}
                            }
                        }
                    }
                }

                // ── Sidebar: Status + Audio Meter ──
                Stack { gap: "md", style: "width: 280px; flex-shrink: 0;",

                    // Status card
                    Paper { p: "md", radius: "md", with_border: true,
                        Stack { gap: "sm",
                            Text { weight: "600", "Status" }
                            Divider {}
                            Group { gap: "xs",
                                Badge {
                                    color: {|| if call_active.get() { "green" } else { "gray" }},
                                    {|| if call_active.get() { "In Call" } else { "Not Connected" }}
                                }
                                if hand_raised.get() {
                                    Badge { color: "yellow", "Hand Raised" }
                                }
                                if screen_sharing.get() {
                                    Badge { color: "green", "Sharing" }
                                }
                            }
                        }
                    }

                    // Audio level meter (live from microphone)
                    Paper { p: "md", radius: "md", with_border: true,
                        Stack { gap: "sm",
                            Group { justify: "space-between",
                                Text { weight: "600", "Microphone Level" }
                                Badge {
                                    color: {|| if mic_muted.get() { "red" } else if mic_active.get() { "green" } else { "gray" }},
                                    variant: "light",
                                    {|| if mic_muted.get() { "Muted" } else if mic_active.get() { "Active" } else { "Unavailable" }}
                                }
                            }

                            // Level bar
                            {audio_level_bar(__scope, audio_level, mic_muted)}
                        }
                    }

                    // Device info card
                    Paper { p: "md", radius: "md", with_border: true,
                        Stack { gap: "sm",
                            Text { weight: "600", "Devices" }
                            Divider {}

                            // Microphone
                            Group { gap: "xs",
                                div { style: "width: 16px; height: 16px; color: var(--rinch-color-dimmed);",
                                    {render_tabler_icon(__scope, TablerIcon::Microphone, TablerIconStyle::Outline)}
                                }
                                Text { size: "sm", {|| {
                                    let name = selected_mic.get();
                                    if name.is_empty() { "No microphone found".to_string() } else { name }
                                }} }
                            }

                            // Speaker
                            Group { gap: "xs",
                                div { style: "width: 16px; height: 16px; color: var(--rinch-color-dimmed);",
                                    {render_tabler_icon(__scope, TablerIcon::Volume, TablerIconStyle::Outline)}
                                }
                                Text { size: "sm", {|| {
                                    let name = selected_speaker.get();
                                    if name.is_empty() { "No speaker found".to_string() } else { name }
                                }} }
                            }

                            // Camera
                            Group { gap: "xs",
                                div { style: "width: 16px; height: 16px; color: var(--rinch-color-dimmed);",
                                    {render_tabler_icon(__scope, TablerIcon::Video, TablerIconStyle::Outline)}
                                }
                                Text { size: "sm", {|| {
                                    let name = selected_camera.get();
                                    if name.is_empty() { "No camera found".to_string() } else { name }
                                }} }
                            }

                            Space { h: "xs" }
                            Button {
                                variant: "light",
                                size: "xs",
                                full_width: true,
                                onclick: move || audio_settings_open.set(true),
                                "Audio Settings"
                            }
                            Button {
                                variant: "light",
                                size: "xs",
                                full_width: true,
                                onclick: move || video_settings_open.set(true),
                                "Video Settings"
                            }
                        }
                    }

                    // Volume controls
                    Paper { p: "md", radius: "md", with_border: true,
                        Stack { gap: "sm",
                            Text { weight: "600", "Volume" }
                            Divider {}

                            // Input volume
                            Group { gap: "xs",
                                div { style: "width: 16px; height: 16px; color: var(--rinch-color-dimmed);",
                                    {render_tabler_icon(__scope, TablerIcon::Microphone, TablerIconStyle::Outline)}
                                }
                                Text { size: "sm", "Input" }
                            }
                            Slider {
                                value_signal: Some(input_volume),
                                onchange: move |v: f64| input_volume.set(v),
                                min: 0.0,
                                max: 100.0,
                            }

                            Space { h: "xs" }

                            // Output volume
                            Group { gap: "xs",
                                div { style: "width: 16px; height: 16px; color: var(--rinch-color-dimmed);",
                                    {render_tabler_icon(__scope, TablerIcon::Volume, TablerIconStyle::Outline)}
                                }
                                Text { size: "sm", "Output" }
                            }
                            Slider {
                                value_signal: Some(output_volume),
                                onchange: move |v: f64| output_volume.set(v),
                                min: 0.0,
                                max: 100.0,
                            }
                        }
                    }
                }
            }

            // ── Settings modals ──

            // Audio Settings Modal
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

                    Divider {}

                    // Noise suppression toggle
                    Group { justify: "space-between",
                        Stack { gap: "0",
                            Text { size: "sm", "Noise Suppression" }
                            Text { size: "xs", color: "dimmed", "Reduce background noise" }
                        }
                        Switch {
                            checked_fn: move || true,
                            onchange: move || {},
                        }
                    }

                    // Echo cancellation toggle
                    Group { justify: "space-between",
                        Stack { gap: "0",
                            Text { size: "sm", "Echo Cancellation" }
                            Text { size: "xs", color: "dimmed", "Prevent audio feedback" }
                        }
                        Switch {
                            checked_fn: move || true,
                            onchange: move || {},
                        }
                    }
                }

                Space { h: "lg" }
                Group { justify: "flex-end",
                    Button { onclick: move || audio_settings_open.set(false), "Done" }
                }
            }

            // Video Settings Modal
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

                    Divider {}

                    // HD video toggle
                    Group { justify: "space-between",
                        Stack { gap: "0",
                            Text { size: "sm", "HD Video" }
                            Text { size: "xs", color: "dimmed", "Send 720p video (uses more bandwidth)" }
                        }
                        Switch {
                            checked_fn: move || true,
                            onchange: move || {},
                        }
                    }

                    // Mirror video toggle
                    Group { justify: "space-between",
                        Stack { gap: "0",
                            Text { size: "sm", "Mirror My Video" }
                            Text { size: "xs", color: "dimmed", "Flip camera horizontally" }
                        }
                        Switch {
                            checked_fn: move || true,
                            onchange: move || {},
                        }
                    }
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
    _label_on: &'static str,
    _label_off: &'static str,
) -> NodeHandle {
    rsx! {
        ActionIcon {
            size: "xl",
            radius: "xl",
            variant: {|| if toggled.get() { "filled" } else { "default" }},
            color: {|| if toggled.get() { active_color } else { "" }},
            onclick: move || toggled.update(|v| *v = !*v),
            if toggled.get() {
                {render_tabler_icon(__scope, icon_off, TablerIconStyle::Outline)}
            } else {
                {render_tabler_icon(__scope, icon_on, TablerIconStyle::Outline)}
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
