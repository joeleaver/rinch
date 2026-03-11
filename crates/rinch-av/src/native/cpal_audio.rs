//! cpal audio backend — device enumeration, stream creation, format negotiation.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, Device, Host, SampleFormat, SampleRate, StreamConfig};

use crate::device::{DeviceId, DeviceInfo, DeviceKind};
use crate::error::AvError;

/// Get the default cpal host.
fn host() -> Host {
    cpal::default_host()
}

/// Map a cpal error into an `AvError`.
fn map_device_err(e: cpal::DevicesError) -> AvError {
    AvError::Backend(format!("device enumeration failed: {e}"))
}

fn map_stream_err(e: cpal::BuildStreamError) -> AvError {
    match e {
        cpal::BuildStreamError::DeviceNotAvailable => {
            AvError::DeviceNotFound("device not available".into())
        }
        cpal::BuildStreamError::StreamConfigNotSupported => {
            AvError::FormatUnsupported("stream config not supported".into())
        }
        cpal::BuildStreamError::InvalidArgument => AvError::Backend("invalid argument".into()),
        cpal::BuildStreamError::StreamIdOverflow => AvError::Backend("stream ID overflow".into()),
        _ => AvError::Backend(format!("build stream error: {e}")),
    }
}

fn map_play_err(e: cpal::PlayStreamError) -> AvError {
    AvError::Backend(format!("play stream error: {e}"))
}

fn map_default_err(e: cpal::DefaultStreamConfigError) -> AvError {
    match e {
        cpal::DefaultStreamConfigError::DeviceNotAvailable => {
            AvError::DeviceNotFound("device not available".into())
        }
        _ => AvError::Backend(format!("default config error: {e}")),
    }
}

/// Build a device name suitable for use as a `DeviceId`.
fn device_id(device: &Device) -> String {
    device.name().unwrap_or_else(|_| "unknown".into())
}

// ── Friendly Device Names (Linux) ──
//
// On Linux, cpal's ALSA backend exposes raw device names like "hw:CARD=Webcam,DEV=0",
// "plughw:CARD=Webcam,DEV=0", "dsnoop:CARD=Webcam,DEV=0" etc. — dozens of entries
// per physical device. When PulseAudio/PipeWire is running (indicated by a "pulse"
// ALSA device), we collapse these into one "System Default" + one entry per physical
// sound card with friendly names from /proc/asound/cards.
//
// On macOS/Windows, cpal's CoreAudio/WASAPI backends already provide friendly names,
// so `make_friendly` is a no-op pass-through.

/// Filter raw cpal device list to user-friendly entries.
#[cfg(target_os = "linux")]
fn make_friendly(raw: Vec<DeviceInfo>, kind: DeviceKind) -> Vec<DeviceInfo> {
    let has_pulse = raw.iter().any(|d| d.id.0 == "pulse");
    if !has_pulse {
        return raw;
    }

    let card_names = linux_card_friendly_names();
    let mut result = Vec::new();

    // "System Default" entry using the pulse device
    result.push(DeviceInfo {
        id: DeviceId("pulse".into()),
        name: "System Default".into(),
        kind,
        is_default: true,
    });

    // One entry per physical card, using `default:CARD=X` as the device ID
    // (most compatible ALSA device variant — handles sample rate conversion)
    let mut seen_cards = std::collections::HashSet::new();
    for device in &raw {
        if !device.id.0.starts_with("default:CARD=") {
            continue;
        }
        let Some(card_id) = linux_extract_card_id(&device.id.0) else {
            continue;
        };
        if !seen_cards.insert(card_id.to_string()) {
            continue;
        }
        let friendly_name = card_names
            .get(card_id)
            .cloned()
            .unwrap_or_else(|| card_id.to_string());
        result.push(DeviceInfo {
            id: device.id.clone(),
            name: friendly_name,
            kind,
            is_default: false,
        });
    }

    result
}

/// On non-Linux platforms, cpal already provides friendly device names.
#[cfg(not(target_os = "linux"))]
fn make_friendly(raw: Vec<DeviceInfo>, _kind: DeviceKind) -> Vec<DeviceInfo> {
    raw
}

/// Parse `/proc/asound/cards` to map ALSA card IDs to friendly names.
/// Returns a map of card_id (e.g. "Webcam") → friendly name (e.g. "NexiGo N60 FHD Webcam").
#[cfg(target_os = "linux")]
fn linux_card_friendly_names() -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let Ok(contents) = std::fs::read_to_string("/proc/asound/cards") else {
        return map;
    };
    // Format: " 1 [Webcam         ]: USB-Audio - NexiGo N60 FHD Webcam"
    for line in contents.lines() {
        let line = line.trim();
        if let Some(bracket_start) = line.find('[')
            && let Some(bracket_end) = line.find(']')
        {
            let card_id = line[bracket_start + 1..bracket_end].trim().to_string();
            if let Some(dash_pos) = line.find(" - ") {
                let friendly = line[dash_pos + 3..].trim().to_string();
                if !friendly.is_empty() {
                    map.insert(card_id, friendly);
                }
            }
        }
    }
    map
}

/// Extract the ALSA card ID from a device name like "default:CARD=Webcam".
#[cfg(target_os = "linux")]
fn linux_extract_card_id(alsa_name: &str) -> Option<&str> {
    let idx = alsa_name.find("CARD=")?;
    let rest = &alsa_name[idx + 5..];
    Some(rest.split(',').next().unwrap_or(rest))
}

// ── Device Enumeration ──

/// Enumerate output devices with friendly names.
pub(crate) fn enumerate_output_devices() -> Result<Vec<DeviceInfo>, AvError> {
    let host = host();
    let default_name = host.default_output_device().and_then(|d| d.name().ok());

    let devices = host.output_devices().map_err(map_device_err)?;
    let mut raw = Vec::new();
    for device in devices {
        let name = device.name().unwrap_or_else(|_| "Unknown".into());
        let id = device_id(&device);
        let is_default = default_name.as_deref() == Some(&name);
        raw.push(DeviceInfo {
            id: DeviceId(id),
            name,
            kind: DeviceKind::AudioOutput,
            is_default,
        });
    }

    Ok(make_friendly(raw, DeviceKind::AudioOutput))
}

/// Enumerate input devices with friendly names.
pub(crate) fn enumerate_input_devices() -> Result<Vec<DeviceInfo>, AvError> {
    let host = host();
    let default_name = host.default_input_device().and_then(|d| d.name().ok());

    let devices = host.input_devices().map_err(map_device_err)?;
    let mut raw = Vec::new();
    for device in devices {
        let name = device.name().unwrap_or_else(|_| "Unknown".into());
        let id = device_id(&device);
        let is_default = default_name.as_deref() == Some(&name);
        raw.push(DeviceInfo {
            id: DeviceId(id),
            name,
            kind: DeviceKind::AudioInput,
            is_default,
        });
    }

    Ok(make_friendly(raw, DeviceKind::AudioInput))
}

// ── Device Lookup ──

fn find_output_device(device_id: &DeviceId) -> Result<Device, AvError> {
    let host = host();
    let devices = host.output_devices().map_err(map_device_err)?;
    for device in devices {
        if self::device_id(&device) == device_id.0 {
            return Ok(device);
        }
    }
    Err(AvError::DeviceNotFound(device_id.0.clone()))
}

fn find_input_device(device_id: &DeviceId) -> Result<Device, AvError> {
    let host = host();
    let devices = host.input_devices().map_err(map_device_err)?;
    for device in devices {
        if self::device_id(&device) == device_id.0 {
            return Ok(device);
        }
    }
    Err(AvError::DeviceNotFound(device_id.0.clone()))
}

// ── Stream Config Negotiation ──

/// Build a `StreamConfig` from our config, validating against device capabilities.
fn negotiate_output_config(
    device: &Device,
    sample_rate: u32,
    channels: u16,
    buffer_size: u32,
) -> Result<(StreamConfig, SampleFormat), AvError> {
    // Try to get the default config first to check sample format support
    let default_config = device.default_output_config().map_err(map_default_err)?;

    let sample_format = default_config.sample_format();

    let buf_size = negotiate_buffer_size(&default_config.config(), buffer_size);

    let config = StreamConfig {
        channels,
        sample_rate: SampleRate(sample_rate),
        buffer_size: buf_size,
    };

    Ok((config, sample_format))
}

fn negotiate_input_config(
    device: &Device,
    sample_rate: u32,
    channels: u16,
    buffer_size: u32,
) -> Result<(StreamConfig, SampleFormat), AvError> {
    let default_config = device.default_input_config().map_err(map_default_err)?;

    let sample_format = default_config.sample_format();

    let buf_size = negotiate_buffer_size(&default_config.config(), buffer_size);

    let config = StreamConfig {
        channels,
        sample_rate: SampleRate(sample_rate),
        buffer_size: buf_size,
    };

    Ok((config, sample_format))
}

fn negotiate_buffer_size(_default_config: &StreamConfig, requested: u32) -> BufferSize {
    // cpal on many backends doesn't support fixed buffer sizes well,
    // so we default to letting the backend choose unless the user
    // explicitly requested something.
    if requested > 0 {
        BufferSize::Fixed(requested)
    } else {
        BufferSize::Default
    }
}

// ── Output Stream ──

/// Open an output stream on the given device.
///
/// Returns the cpal stream (must be kept alive) and the resolved config.
pub(crate) fn open_output_stream(
    device: &Device,
    sample_rate: u32,
    channels: u16,
    buffer_size: u32,
    mut fill: impl FnMut(&mut [f32]) + Send + 'static,
) -> Result<cpal::Stream, AvError> {
    let (config, sample_format) =
        negotiate_output_config(device, sample_rate, channels, buffer_size)?;

    let err_callback = |err: cpal::StreamError| {
        tracing::error!("audio output stream error: {err}");
    };

    let stream = match sample_format {
        SampleFormat::F32 => device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    fill(data);
                },
                err_callback,
                None,
            )
            .map_err(map_stream_err)?,
        SampleFormat::I16 => device
            .build_output_stream(
                &config,
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    // Convert: call fill with f32 buffer, then convert to i16
                    let mut f32_buf = vec![0.0f32; data.len()];
                    fill(&mut f32_buf);
                    for (out, &sample) in data.iter_mut().zip(f32_buf.iter()) {
                        *out = (sample * i16::MAX as f32) as i16;
                    }
                },
                err_callback,
                None,
            )
            .map_err(map_stream_err)?,
        SampleFormat::U16 => device
            .build_output_stream(
                &config,
                move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                    let mut f32_buf = vec![0.0f32; data.len()];
                    fill(&mut f32_buf);
                    for (out, &sample) in data.iter_mut().zip(f32_buf.iter()) {
                        // f32 [-1, 1] -> u16 [0, 65535]
                        *out = ((sample * 0.5 + 0.5) * u16::MAX as f32) as u16;
                    }
                },
                err_callback,
                None,
            )
            .map_err(map_stream_err)?,
        _ => {
            return Err(AvError::FormatUnsupported(format!(
                "unsupported sample format: {sample_format:?}"
            )));
        }
    };

    stream.play().map_err(map_play_err)?;
    Ok(stream)
}

// ── Input Stream ──

/// Open an input stream on the given device.
pub(crate) fn open_input_stream(
    device: &Device,
    sample_rate: u32,
    channels: u16,
    buffer_size: u32,
    mut on_data: impl FnMut(&[f32]) + Send + 'static,
) -> Result<cpal::Stream, AvError> {
    let (config, sample_format) =
        negotiate_input_config(device, sample_rate, channels, buffer_size)?;

    let err_callback = |err: cpal::StreamError| {
        tracing::error!("audio input stream error: {err}");
    };

    let stream = match sample_format {
        SampleFormat::F32 => device
            .build_input_stream(
                &config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    on_data(data);
                },
                err_callback,
                None,
            )
            .map_err(map_stream_err)?,
        SampleFormat::I16 => device
            .build_input_stream(
                &config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let f32_buf: Vec<f32> =
                        data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                    on_data(&f32_buf);
                },
                err_callback,
                None,
            )
            .map_err(map_stream_err)?,
        SampleFormat::U16 => device
            .build_input_stream(
                &config,
                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                    let f32_buf: Vec<f32> = data
                        .iter()
                        .map(|&s| (s as f32 / u16::MAX as f32) * 2.0 - 1.0)
                        .collect();
                    on_data(&f32_buf);
                },
                err_callback,
                None,
            )
            .map_err(map_stream_err)?,
        _ => {
            return Err(AvError::FormatUnsupported(format!(
                "unsupported sample format: {sample_format:?}"
            )));
        }
    };

    stream.play().map_err(map_play_err)?;
    Ok(stream)
}

/// Get the default output device.
///
/// On Linux with PipeWire, the ALSA "default" device may be unavailable
/// even though cpal returns it. We verify the device can provide a config
/// before accepting it, and fall back to the "pulse" ALSA plugin otherwise.
pub(crate) fn default_output_device() -> Result<Device, AvError> {
    let host = host();

    // Try the system default first, but verify it actually works
    if let Some(device) = host.default_output_device()
        && device.default_output_config().is_ok()
    {
        return Ok(device);
    }

    // Fallback: try "pulse" device (works on PipeWire systems via pipewire-alsa)
    if let Ok(devices) = host.output_devices() {
        for device in devices {
            if let Ok(name) = device.name()
                && name == "pulse"
                && device.default_output_config().is_ok()
            {
                return Ok(device);
            }
        }
    }

    // Last resort: first available output device that can provide a config
    if let Ok(devices) = host.output_devices() {
        for device in devices {
            if device.default_output_config().is_ok() {
                return Ok(device);
            }
        }
    }

    Err(AvError::DeviceNotFound("no default output device".into()))
}

/// Get the default input device.
///
/// On Linux with PipeWire, the ALSA "default" device may be unavailable
/// even though cpal returns it. We verify the device can provide a config
/// before accepting it, and fall back to the "pulse" ALSA plugin otherwise.
pub(crate) fn default_input_device() -> Result<Device, AvError> {
    let host = host();

    // Try the system default first, but verify it actually works
    if let Some(device) = host.default_input_device()
        && device.default_input_config().is_ok()
    {
        return Ok(device);
    }

    // Fallback: try "pulse" device (works on PipeWire systems via pipewire-alsa)
    if let Ok(devices) = host.input_devices() {
        for device in devices {
            if let Ok(name) = device.name()
                && name == "pulse"
                && device.default_input_config().is_ok()
            {
                return Ok(device);
            }
        }
    }

    // Last resort: first available input device that can provide a config
    if let Ok(devices) = host.input_devices() {
        for device in devices {
            if device.default_input_config().is_ok() {
                return Ok(device);
            }
        }
    }

    Err(AvError::DeviceNotFound("no default input device".into()))
}

/// Find a specific output device by ID.
pub(crate) fn get_output_device(id: &DeviceId) -> Result<Device, AvError> {
    find_output_device(id)
}

/// Find a specific input device by ID.
pub(crate) fn get_input_device(id: &DeviceId) -> Result<Device, AvError> {
    find_input_device(id)
}
