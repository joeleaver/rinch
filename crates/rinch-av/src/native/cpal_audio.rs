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

// ── Device Enumeration ──

/// Enumerate output devices.
pub(crate) fn enumerate_output_devices() -> Result<Vec<DeviceInfo>, AvError> {
    let host = host();
    let default_name = host.default_output_device().and_then(|d| d.name().ok());

    let devices = host.output_devices().map_err(map_device_err)?;
    let mut result = Vec::new();
    for device in devices {
        let name = device.name().unwrap_or_else(|_| "Unknown".into());
        let id = device_id(&device);
        let is_default = default_name.as_deref() == Some(&name);
        result.push(DeviceInfo {
            id: DeviceId(id),
            name,
            kind: DeviceKind::AudioOutput,
            is_default,
        });
    }
    Ok(result)
}

/// Enumerate input devices.
pub(crate) fn enumerate_input_devices() -> Result<Vec<DeviceInfo>, AvError> {
    let host = host();
    let default_name = host.default_input_device().and_then(|d| d.name().ok());

    let devices = host.input_devices().map_err(map_device_err)?;
    let mut result = Vec::new();
    for device in devices {
        let name = device.name().unwrap_or_else(|_| "Unknown".into());
        let id = device_id(&device);
        let is_default = default_name.as_deref() == Some(&name);
        result.push(DeviceInfo {
            id: DeviceId(id),
            name,
            kind: DeviceKind::AudioInput,
            is_default,
        });
    }
    Ok(result)
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
pub(crate) fn default_output_device() -> Result<Device, AvError> {
    host()
        .default_output_device()
        .ok_or_else(|| AvError::DeviceNotFound("no default output device".into()))
}

/// Get the default input device.
pub(crate) fn default_input_device() -> Result<Device, AvError> {
    host()
        .default_input_device()
        .ok_or_else(|| AvError::DeviceNotFound("no default input device".into()))
}

/// Find a specific output device by ID.
pub(crate) fn get_output_device(id: &DeviceId) -> Result<Device, AvError> {
    find_output_device(id)
}

/// Find a specific input device by ID.
pub(crate) fn get_input_device(id: &DeviceId) -> Result<Device, AvError> {
    find_input_device(id)
}
