//! Linux screen capture via XDG Desktop Portal + PipeWire.
//!
//! Flow:
//! 1. ashpd opens the ScreenCast portal dialog (user picks a screen/window)
//! 2. Portal returns a PipeWire node ID + file descriptor
//! 3. pipewire-rs connects to the node and streams frames
//! 4. Frames are converted to RGBA and pushed to the VideoTrack

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rinch_av::camera::VideoFrame;
use tracing::{debug, warn};

use crate::{CaptureConfig, CaptureError, FrameSink};

/// Persistent tokio runtime for portal D-Bus calls.
///
/// ashpd caches the zbus D-Bus connection in a process-global `OnceLock`.
/// If we create throwaway runtimes, the connection's async tasks (signal dispatch,
/// keepalive) stop when the runtime drops, and the second portal call hangs because
/// D-Bus response signals are never delivered.
///
/// A multi-threaded runtime keeps the event loop alive between `block_on` calls,
/// so the cached D-Bus connection stays healthy.
fn portal_runtime() -> &'static tokio::runtime::Runtime {
    use std::sync::OnceLock;
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("failed to create portal tokio runtime")
    })
}

/// Handle to a running PipeWire screen capture.
pub(crate) struct PipeWireCapture {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for PipeWireCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }
}

/// Information from the portal dialog.
struct PortalResult {
    pw_fd: std::os::fd::OwnedFd,
    node_id: u32,
    /// Send on this to close the portal session from any thread.
    /// The actual close runs on the portal runtime using ashpd's D-Bus connection.
    session_close: tokio::sync::oneshot::Sender<()>,
}

impl PipeWireCapture {
    pub fn start(sink: FrameSink, config: &CaptureConfig) -> Result<Self, CaptureError> {
        // Step 1: Run the portal dialog (needs a tokio runtime for ashpd).
        let portal = run_portal_dialog(config)?;

        debug!("portal: node_id={}, fd={:?}", portal.node_id, portal.pw_fd);

        // Step 2: Spawn a PipeWire capture thread.
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();
        let max_fps = config.max_fps;

        let thread = std::thread::Builder::new()
            .name("rinch-screen-capture".into())
            .spawn(move || {
                if let Err(e) = run_pipewire_capture(portal, sink, max_fps, &stop_clone) {
                    warn!("PipeWire capture error: {e}");
                }
                debug!("screen capture thread exiting");
            })
            .map_err(|e| CaptureError::Platform(format!("spawn thread: {e}")))?;

        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}

/// Open the XDG ScreenCast portal dialog. Blocks until the user selects a source.
fn run_portal_dialog(config: &CaptureConfig) -> Result<PortalResult, CaptureError> {
    use ashpd::desktop::PersistMode;
    use ashpd::desktop::screencast::{CursorMode, Screencast, SourceType};

    // Use the persistent runtime so zbus reuses its D-Bus connection across calls.
    let rt = portal_runtime();

    rt.block_on(async {
        let proxy = Screencast::new()
            .await
            .map_err(|e| CaptureError::Platform(format!("screencast proxy: {e}")))?;

        let session = proxy
            .create_session()
            .await
            .map_err(|e| CaptureError::Platform(format!("create session: {e}")))?;

        let cursor_mode = if config.show_cursor {
            CursorMode::Embedded
        } else {
            CursorMode::Hidden
        };

        proxy
            .select_sources(
                &session,
                cursor_mode,
                SourceType::Monitor | SourceType::Window,
                false,
                None,
                PersistMode::DoNot,
            )
            .await
            .map_err(|e| CaptureError::Platform(format!("select sources: {e}")))?
            .response()
            .map_err(|e| CaptureError::Platform(format!("select sources response: {e}")))?;

        let response = proxy
            .start(&session, None)
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("cancelled") || msg.contains("Cancelled") {
                    CaptureError::Cancelled
                } else {
                    CaptureError::Platform(format!("start: {e}"))
                }
            })?
            .response()
            .map_err(|e| CaptureError::Platform(format!("response: {e}")))?;

        let streams = response.streams();
        if streams.is_empty() {
            // No streams selected — close session immediately and bail.
            let _ = session.close().await;
            return Err(CaptureError::Cancelled);
        }

        let stream = &streams[0];
        let node_id = stream.pipe_wire_node_id();

        let pw_fd = proxy
            .open_pipe_wire_remote(&session)
            .await
            .map_err(|e| CaptureError::Platform(format!("pipewire fd: {e}")))?;

        // The session must stay alive while PipeWire streams frames.
        // Spawn a task on the portal runtime that holds the Session and waits
        // for a close signal. This uses ashpd's cached D-Bus connection
        // (same one that created the session) so the portal accepts the Close.
        let (close_tx, close_rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            let _ = close_rx.await;
            debug!("closing portal session");
            let _ = session.close().await;
        });

        Ok(PortalResult {
            pw_fd,
            node_id,
            session_close: close_tx,
        })
    })
}

/// Connect to PipeWire and stream frames to the VideoTrack.
fn run_pipewire_capture(
    portal: PortalResult,
    sink: FrameSink,
    _max_fps: u32,
    stop: &AtomicBool,
) -> Result<(), String> {
    use pipewire as pw;
    use pw::spa::param::format::{FormatProperties, MediaSubtype, MediaType};
    use pw::spa::param::video::VideoFormat;
    use pw::spa::pod::serialize::PodSerializer;
    use pw::spa::pod::{ChoiceValue, Object, Property, PropertyFlags, Value};
    use pw::spa::utils::{Choice, ChoiceEnum, ChoiceFlags, Id, SpaTypes};

    let PortalResult {
        pw_fd,
        node_id,
        session_close,
    } = portal;

    pw::init();

    let mainloop = pw::main_loop::MainLoop::new(None).map_err(|e| format!("MainLoop::new: {e}"))?;

    let context = pw::context::Context::new(&mainloop).map_err(|e| format!("Context::new: {e}"))?;

    let core = context
        .connect_fd(pw_fd, None)
        .map_err(|e| format!("connect_fd: {e}"))?;

    // Build format parameters for the stream.
    let obj = Object {
        type_: SpaTypes::ObjectParamFormat.as_raw(),
        id: pw::spa::param::ParamType::EnumFormat.as_raw(),
        properties: vec![
            Property {
                key: FormatProperties::MediaType.as_raw(),
                flags: PropertyFlags::empty(),
                value: Value::Id(Id(MediaType::Video.as_raw())),
            },
            Property {
                key: FormatProperties::MediaSubtype.as_raw(),
                flags: PropertyFlags::empty(),
                value: Value::Id(Id(MediaSubtype::Raw.as_raw())),
            },
            Property {
                key: FormatProperties::VideoFormat.as_raw(),
                flags: PropertyFlags::empty(),
                value: Value::Choice(ChoiceValue::Id(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Enum {
                        default: Id(VideoFormat::BGRx.as_raw()),
                        alternatives: vec![
                            Id(VideoFormat::BGRx.as_raw()),
                            Id(VideoFormat::RGBA.as_raw()),
                            Id(VideoFormat::RGBx.as_raw()),
                        ],
                    },
                ))),
            },
        ],
    };

    // Serialize the pod object into a buffer.
    let value = Value::Object(obj);
    let mut params_buf = vec![0u8; 1024];
    let (written, _) = PodSerializer::serialize(std::io::Cursor::new(&mut params_buf[..]), &value)
        .map_err(|e| format!("pod serialize: {e:?}"))?;
    let pod_bytes_len = written.position() as usize;

    // Create the stream.
    let stream = pw::stream::Stream::new(
        &core,
        "rinch-screen-capture",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )
    .map_err(|e| format!("Stream::new: {e}"))?;

    // Track negotiated format.
    let format_info = Arc::new(std::sync::Mutex::new(FormatInfo::default()));
    let format_info_clone = format_info.clone();

    // Set up frame callback.
    let capture_start = std::time::Instant::now();

    let _listener = stream
        .add_local_listener_with_user_data(())
        .param_changed(move |_, _, id, pod| {
            if id != pw::spa::param::ParamType::Format.as_raw() {
                return;
            }
            if let Some(pod) = pod
                && let Ok(info) = parse_video_format(pod)
            {
                debug!(
                    "negotiated format: {}x{} {:?}",
                    info.width, info.height, info.format
                );
                *format_info_clone.lock().unwrap() = info;
            }
        })
        .process(move |stream, _| {
            if let Some(mut buffer) = stream.dequeue_buffer() {
                let datas = buffer.datas_mut();
                if datas.is_empty() {
                    return;
                }

                let data = &mut datas[0];
                let chunk = data.chunk();
                let size = chunk.size() as usize;
                if size == 0 {
                    return;
                }

                if let Some(raw) = data.data() {
                    let info = format_info.lock().unwrap().clone();
                    if info.width == 0 || info.height == 0 {
                        return;
                    }

                    let rgba = convert_to_rgba(&raw[..size], &info);
                    if !rgba.is_empty() {
                        let frame = VideoFrame {
                            data: rgba,
                            width: info.width,
                            height: info.height,
                            timestamp_us: capture_start.elapsed().as_micros() as u64,
                        };
                        *sink.lock().unwrap() = Some(frame);
                    }
                }
            }
        })
        .register()
        .map_err(|e| format!("listener register: {e}"))?;

    // Connect the stream to the portal's PipeWire node.
    let pod_ref = pw::spa::pod::Pod::from_bytes(&params_buf[..pod_bytes_len])
        .ok_or("failed to create Pod from serialized bytes")?;

    stream
        .connect(
            pw::spa::utils::Direction::Input,
            Some(node_id),
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut [pod_ref],
        )
        .map_err(|e| format!("stream connect: {e}"))?;

    debug!("PipeWire stream connected, starting capture loop");

    // Run the main loop until stopped.
    let mainloop_weak = mainloop.downgrade();
    let stop_ref = stop as *const AtomicBool;

    // Check stop flag periodically via a timer.
    let timer = mainloop.loop_().add_timer(move |_| {
        let stop = unsafe { &*stop_ref };
        if stop.load(Ordering::Relaxed)
            && let Some(ml) = mainloop_weak.upgrade()
        {
            ml.quit();
        }
    });

    timer.update_timer(
        Some(std::time::Duration::from_millis(100)),
        Some(std::time::Duration::from_millis(100)),
    );

    mainloop.run();

    debug!("PipeWire main loop exited, signaling session close");

    // Signal the portal runtime to close the session via ashpd's D-Bus connection.
    let _ = session_close.send(());

    Ok(())
}

/// Negotiated video format info.
#[derive(Debug, Clone, Default)]
struct FormatInfo {
    width: u32,
    height: u32,
    format: NegotiatedFormat,
}

#[derive(Debug, Clone, Default)]
#[allow(clippy::upper_case_acronyms)]
enum NegotiatedFormat {
    #[default]
    Unknown,
    BGRx,
    RGBA,
    RGBx,
}

/// Parse the negotiated video format from a SPA pod.
fn parse_video_format(pod: &pipewire::spa::pod::Pod) -> Result<FormatInfo, String> {
    use pipewire::spa::param::format::FormatProperties;
    use pipewire::spa::param::video::VideoFormat;
    use pipewire::spa::pod::Value;
    use pipewire::spa::pod::deserialize::PodDeserializer;

    let mut info = FormatInfo::default();

    if let Ok((_, Value::Object(obj))) = PodDeserializer::deserialize_from::<Value>(pod.as_bytes())
    {
        for prop in &obj.properties {
            match prop.key {
                k if k == FormatProperties::VideoFormat.as_raw() => {
                    if let Value::Id(id) = &prop.value {
                        info.format = match id.0 {
                            x if x == VideoFormat::BGRx.as_raw() => NegotiatedFormat::BGRx,
                            x if x == VideoFormat::RGBA.as_raw() => NegotiatedFormat::RGBA,
                            x if x == VideoFormat::RGBx.as_raw() => NegotiatedFormat::RGBx,
                            _ => NegotiatedFormat::Unknown,
                        };
                    }
                }
                k if k == FormatProperties::VideoSize.as_raw() => {
                    if let Value::Rectangle(r) = &prop.value {
                        info.width = r.width;
                        info.height = r.height;
                    }
                }
                _ => {}
            }
        }
    }

    Ok(info)
}

/// Convert raw pixel data to RGBA based on the negotiated format.
fn convert_to_rgba(raw: &[u8], info: &FormatInfo) -> Vec<u8> {
    let pixel_count = (info.width * info.height) as usize;
    let expected = pixel_count * 4;

    match info.format {
        NegotiatedFormat::RGBA => {
            if raw.len() >= expected {
                raw[..expected].to_vec()
            } else {
                Vec::new()
            }
        }
        NegotiatedFormat::BGRx | NegotiatedFormat::Unknown => {
            // BGRx → RGBA: swap B↔R, set A=255.
            if raw.len() < expected {
                return Vec::new();
            }
            let mut rgba = vec![0u8; expected];
            for i in 0..pixel_count {
                let src = i * 4;
                let dst = i * 4;
                rgba[dst] = raw[src + 2]; // R ← B
                rgba[dst + 1] = raw[src + 1]; // G
                rgba[dst + 2] = raw[src]; // B ← R
                rgba[dst + 3] = 255; // A (BGRx has padding, not alpha)
            }
            rgba
        }
        NegotiatedFormat::RGBx => {
            // RGBx → RGBA: just set A=255.
            if raw.len() < expected {
                return Vec::new();
            }
            let mut rgba = raw[..expected].to_vec();
            for i in 0..pixel_count {
                rgba[i * 4 + 3] = 255;
            }
            rgba
        }
    }
}
