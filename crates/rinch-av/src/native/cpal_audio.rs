//! cpal audio backend — device enumeration, stream creation, format negotiation.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, Device, Host, SampleFormat, StreamConfig, SupportedStreamConfig,
    SupportedStreamConfigRange,
};

use crate::device::{DeviceId, DeviceInfo, DeviceKind};
use crate::error::AvError;

/// Get the default cpal host.
fn host() -> Host {
    cpal::default_host()
}

/// Map a cpal error into an `AvError`.
fn map_err(e: cpal::Error) -> AvError {
    use cpal::ErrorKind as K;
    match e.kind() {
        K::DeviceNotAvailable | K::HostUnavailable => AvError::DeviceNotFound(e.to_string()),
        K::DeviceBusy => AvError::DeviceInUse,
        K::PermissionDenied => AvError::PermissionDenied,
        K::UnsupportedConfig => AvError::FormatUnsupported(e.to_string()),
        _ => AvError::Backend(e.to_string()),
    }
}

/// The backend's own device id (an ALSA PCM name such as `pulse` or
/// `default:CARD=Webcam`), used as our `DeviceId`.
fn device_id(device: &Device) -> String {
    device
        .id()
        .map(|id| id.id().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

/// The device's display name.
fn device_name(device: &Device) -> String {
    device
        .description()
        .map(|d| d.name().to_string())
        .unwrap_or_else(|_| "Unknown".into())
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
    let default_id = host.default_output_device().map(|d| device_id(&d));

    let devices = host.output_devices().map_err(map_err)?;
    let mut raw = Vec::new();
    for device in devices {
        let name = device_name(&device);
        let id = device_id(&device);
        let is_default = default_id.as_deref() == Some(&id);
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
    let default_id = host.default_input_device().map(|d| device_id(&d));

    let devices = host.input_devices().map_err(map_err)?;
    let mut raw = Vec::new();
    for device in devices {
        let name = device_name(&device);
        let id = device_id(&device);
        let is_default = default_id.as_deref() == Some(&id);
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
    let devices = host.output_devices().map_err(map_err)?;
    for device in devices {
        if self::device_id(&device) == device_id.0 {
            return Ok(device);
        }
    }
    Err(AvError::DeviceNotFound(device_id.0.clone()))
}

fn find_input_device(device_id: &DeviceId) -> Result<Device, AvError> {
    let host = host();
    let devices = host.input_devices().map_err(map_err)?;
    for device in devices {
        if self::device_id(&device) == device_id.0 {
            return Ok(device);
        }
    }
    Err(AvError::DeviceNotFound(device_id.0.clone()))
}

// ── Stream Config Negotiation ──

/// The format and layout a stream was actually opened with: what the
/// callbacks' samples really are. Interleaved f32 at the API, whatever
/// `sample_format` the device runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Negotiated {
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: SampleFormat,
}

/// The sample formats the callbacks convert to and from f32, best first.
fn format_rank(f: SampleFormat) -> Option<u8> {
    match f {
        SampleFormat::F32 => Some(0),
        SampleFormat::I16 => Some(1),
        SampleFormat::U16 => Some(2),
        _ => None,
    }
}

/// Pick what to open the device with, from what it supports.
///
/// Prefers, in order: the requested channel count; a range that contains
/// the requested rate (otherwise the nearest rate the range allows); a
/// format the callbacks convert, f32 first. With no usable range, the
/// device's default config if its format is convertible.
///
/// The result is what the stream really runs at, which the caller reports
/// (it used to report the *requested* rate and channels whatever the device
/// did, issue #1916 / design record rtc-media.md 3.5).
pub(crate) fn select_config(
    ranges: &[SupportedStreamConfigRange],
    default: Option<&SupportedStreamConfig>,
    want_rate: u32,
    want_channels: u16,
) -> Result<Negotiated, AvError> {
    let best = ranges
        .iter()
        .filter_map(|r| format_rank(r.sample_format()).map(|rank| (r, rank)))
        .map(|(r, rank)| {
            let rate = want_rate.clamp(r.min_sample_rate(), r.max_sample_rate());
            let key = (
                r.channels() != want_channels,
                r.channels().abs_diff(want_channels),
                rate.abs_diff(want_rate),
                rank,
            );
            (key, r, rate)
        })
        .min_by_key(|(key, _, _)| *key);
    if let Some((_, r, rate)) = best {
        return Ok(Negotiated {
            sample_rate: rate,
            channels: r.channels(),
            sample_format: r.sample_format(),
        });
    }
    match default {
        Some(d) if format_rank(d.sample_format()).is_some() => Ok(Negotiated {
            sample_rate: d.sample_rate(),
            channels: d.channels(),
            sample_format: d.sample_format(),
        }),
        Some(d) => Err(AvError::FormatUnsupported(format!(
            "unsupported sample format: {:?}",
            d.sample_format()
        ))),
        None => Err(AvError::FormatUnsupported(
            "the device reports no stream configuration".into(),
        )),
    }
}

fn negotiate(
    ranges: Result<Vec<SupportedStreamConfigRange>, cpal::Error>,
    default: Result<SupportedStreamConfig, cpal::Error>,
    sample_rate: u32,
    channels: u16,
    buffer_size: u32,
) -> Result<(StreamConfig, Negotiated), AvError> {
    let ranges = ranges.unwrap_or_default();
    let default = default.ok();
    if ranges.is_empty() && default.is_none() {
        return Err(AvError::DeviceNotFound("device not available".into()));
    }
    let n = select_config(&ranges, default.as_ref(), sample_rate, channels)?;
    let config = StreamConfig {
        channels: n.channels,
        sample_rate: n.sample_rate,
        buffer_size: negotiate_buffer_size(buffer_size),
    };
    Ok((config, n))
}

fn negotiate_buffer_size(requested: u32) -> BufferSize {
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
pub(crate) fn open_output_stream<F: FnMut(&mut [f32]) + Send + 'static>(
    device: &Device,
    sample_rate: u32,
    channels: u16,
    buffer_size: u32,
    make_fill: impl FnOnce(Negotiated) -> F,
) -> Result<(cpal::Stream, Negotiated), AvError> {
    let (config, negotiated) = negotiate(
        device
            .supported_output_configs()
            .map(|it| it.collect::<Vec<_>>()),
        device.default_output_config(),
        sample_rate,
        channels,
        buffer_size,
    )?;
    let sample_format = negotiated.sample_format;
    // Built now that the stream's real format is known.
    let mut fill = make_fill(negotiated);

    let err_callback = |err: cpal::Error| {
        tracing::error!("audio output stream error: {err}");
    };

    let mut f32_buf: Vec<f32> = Vec::new();
    let stream = match sample_format {
        SampleFormat::F32 => device
            .build_output_stream(
                config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    fill(data);
                },
                err_callback,
                None,
            )
            .map_err(map_err)?,
        SampleFormat::I16 => device
            .build_output_stream(
                config,
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    // Convert: call fill with an f32 buffer (reused across
                    // callbacks), then convert to i16.
                    f32_buf.resize(data.len(), 0.0);
                    fill(&mut f32_buf);
                    for (out, &sample) in data.iter_mut().zip(f32_buf.iter()) {
                        *out = (sample * i16::MAX as f32) as i16;
                    }
                },
                err_callback,
                None,
            )
            .map_err(map_err)?,
        SampleFormat::U16 => device
            .build_output_stream(
                config,
                move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                    f32_buf.resize(data.len(), 0.0);
                    fill(&mut f32_buf);
                    for (out, &sample) in data.iter_mut().zip(f32_buf.iter()) {
                        // f32 [-1, 1] -> u16 [0, 65535]
                        *out = ((sample * 0.5 + 0.5) * u16::MAX as f32) as u16;
                    }
                },
                err_callback,
                None,
            )
            .map_err(map_err)?,
        _ => {
            return Err(AvError::FormatUnsupported(format!(
                "unsupported sample format: {sample_format:?}"
            )));
        }
    };

    stream.play().map_err(map_err)?;
    Ok((stream, negotiated))
}

// ── Input Stream ──

/// Open an input stream on the given device.
pub(crate) fn open_input_stream<F: FnMut(&[f32]) + Send + 'static>(
    device: &Device,
    sample_rate: u32,
    channels: u16,
    buffer_size: u32,
    make_on_data: impl FnOnce(Negotiated) -> F,
) -> Result<(cpal::Stream, Negotiated), AvError> {
    let (config, negotiated) = negotiate(
        device
            .supported_input_configs()
            .map(|it| it.collect::<Vec<_>>()),
        device.default_input_config(),
        sample_rate,
        channels,
        buffer_size,
    )?;
    let sample_format = negotiated.sample_format;
    // Built now that the stream's real format is known.
    let mut on_data = make_on_data(negotiated);

    let err_callback = |err: cpal::Error| {
        tracing::error!("audio input stream error: {err}");
    };

    let mut f32_buf: Vec<f32> = Vec::new();
    let stream = match sample_format {
        SampleFormat::F32 => device
            .build_input_stream(
                config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    on_data(data);
                },
                err_callback,
                None,
            )
            .map_err(map_err)?,
        SampleFormat::I16 => device
            .build_input_stream(
                config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    f32_buf.clear();
                    f32_buf.extend(data.iter().map(|&s| s as f32 / i16::MAX as f32));
                    on_data(&f32_buf);
                },
                err_callback,
                None,
            )
            .map_err(map_err)?,
        SampleFormat::U16 => device
            .build_input_stream(
                config,
                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                    f32_buf.clear();
                    f32_buf.extend(
                        data.iter()
                            .map(|&s| (s as f32 / u16::MAX as f32) * 2.0 - 1.0),
                    );
                    on_data(&f32_buf);
                },
                err_callback,
                None,
            )
            .map_err(map_err)?,
        _ => {
            return Err(AvError::FormatUnsupported(format!(
                "unsupported sample format: {sample_format:?}"
            )));
        }
    };

    stream.play().map_err(map_err)?;
    Ok((stream, negotiated))
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
            if device_id(&device) == "pulse" && device.default_output_config().is_ok() {
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
            if device_id(&device) == "pulse" && device.default_input_config().is_ok() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use cpal::SupportedBufferSize;

    fn range(ch: u16, min: u32, max: u32, f: SampleFormat) -> SupportedStreamConfigRange {
        SupportedStreamConfigRange::new(ch, min, max, SupportedBufferSize::Unknown, f)
    }

    #[test]
    fn the_requested_format_when_the_device_supports_it() {
        let ranges = [
            range(2, 44_100, 48_000, SampleFormat::I16),
            range(1, 8_000, 96_000, SampleFormat::F32),
        ];
        let n = select_config(&ranges, None, 48_000, 1).unwrap();
        assert_eq!(
            n,
            Negotiated {
                sample_rate: 48_000,
                channels: 1,
                sample_format: SampleFormat::F32
            }
        );
    }

    #[test]
    fn what_the_device_runs_when_it_cannot_do_what_was_asked() {
        // A stereo-only, 44.1 kHz-only device asked for 48 kHz mono: the
        // stream really runs 44.1 kHz stereo, and that is what is reported.
        let ranges = [range(2, 44_100, 44_100, SampleFormat::I16)];
        let n = select_config(&ranges, None, 48_000, 1).unwrap();
        assert_eq!((n.sample_rate, n.channels), (44_100, 2));
        assert_eq!(n.sample_format, SampleFormat::I16);
    }

    #[test]
    fn the_nearest_rate_inside_a_range_and_f32_first() {
        let ranges = [
            range(1, 16_000, 32_000, SampleFormat::U16),
            range(1, 16_000, 32_000, SampleFormat::F32),
            range(1, 16_000, 32_000, SampleFormat::I16),
        ];
        let n = select_config(&ranges, None, 48_000, 1).unwrap();
        assert_eq!(
            (n.sample_rate, n.sample_format),
            (32_000, SampleFormat::F32)
        );
    }

    #[test]
    fn unconvertible_formats_fall_back_to_a_convertible_default_or_fail() {
        let ranges = [range(1, 48_000, 48_000, SampleFormat::I32)];
        assert!(matches!(
            select_config(&ranges, None, 48_000, 1),
            Err(AvError::FormatUnsupported(_))
        ));
        let default =
            SupportedStreamConfig::new(2, 48_000, SupportedBufferSize::Unknown, SampleFormat::F32);
        let n = select_config(&ranges, Some(&default), 22_050, 1).unwrap();
        assert_eq!((n.sample_rate, n.channels), (48_000, 2));
    }
}
