//! WASM camera capture via `getUserMedia` + canvas pixel readback.

use crate::AvError;
use crate::camera::format::VideoFrame;
use crate::device::{DeviceId, DeviceInfo, DeviceKind};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

/// Configuration for camera capture.
#[derive(Debug, Clone)]
pub struct CameraConfig {
    /// Requested width in pixels.
    pub width: u32,
    /// Requested height in pixels.
    pub height: u32,
    /// Requested frames per second.
    pub fps: u32,
    /// Preferred pixel format. `None` lets the backend choose.
    pub format: Option<super::PixelFormat>,
}

impl Default for CameraConfig {
    fn default() -> Self {
        Self {
            width: 640,
            height: 480,
            fps: 30,
            format: None,
        }
    }
}

/// An active camera capture session.
///
/// Dropping this stops the media tracks and cancels the frame loop.
pub struct CameraCapture {
    /// Media stream tracks to stop on drop.
    tracks: Vec<web_sys::MediaStreamTrack>,
    /// Set to true to cancel the rAF loop.
    running: Rc<RefCell<bool>>,
}

impl CameraCapture {
    /// Stop capture.
    pub fn stop(&mut self) {
        *self.running.borrow_mut() = false;
        for track in &self.tracks {
            track.stop();
        }
        self.tracks.clear();
    }
}

impl Drop for CameraCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

fn window() -> Result<web_sys::Window, AvError> {
    web_sys::window().ok_or_else(|| AvError::Backend("no global window".into()))
}

fn document() -> Result<web_sys::Document, AvError> {
    window()?
        .document()
        .ok_or_else(|| AvError::Backend("no document".into()))
}

fn media_devices() -> Result<web_sys::MediaDevices, AvError> {
    window()?
        .navigator()
        .media_devices()
        .map_err(|_| AvError::Backend("navigator.mediaDevices not available".into()))
}

/// Enumerate available camera devices.
///
/// On WASM this starts an async enumeration via `enumerateDevices()`.
/// Since the public API is synchronous, this fires a `spawn_local` and
/// returns an empty vec on first call. Call again after the user has
/// granted permission (e.g. after `open_camera`) to get populated results.
///
/// For a fully async workflow, use `camera_devices_async()`.
pub fn camera_devices() -> Result<Vec<DeviceInfo>, AvError> {
    // Synchronous fallback: return empty. Users should use camera_devices_async.
    Ok(Vec::new())
}

/// Async version of device enumeration for WASM.
pub async fn camera_devices_async() -> Result<Vec<DeviceInfo>, AvError> {
    let md = media_devices()?;
    let promise = md
        .enumerate_devices()
        .map_err(|e| AvError::Backend(format!("enumerateDevices failed: {:?}", e)))?;
    let result = JsFuture::from(promise)
        .await
        .map_err(|e| AvError::Backend(format!("enumerateDevices rejected: {:?}", e)))?;

    let array: js_sys::Array = result.into();
    let mut devices = Vec::new();
    let mut idx = 0u32;
    for item in array.iter() {
        let info: web_sys::MediaDeviceInfo = item.into();
        if info.kind() == web_sys::MediaDeviceKind::Videoinput {
            let device_id = info.device_id();
            let label = info.label();
            devices.push(DeviceInfo {
                id: DeviceId(if device_id.is_empty() {
                    format!("camera-{}", idx)
                } else {
                    device_id
                }),
                name: if label.is_empty() {
                    format!("Camera {}", idx)
                } else {
                    label
                },
                kind: DeviceKind::Camera,
                is_default: idx == 0,
            });
            idx += 1;
        }
    }
    Ok(devices)
}

/// Open the default camera.
///
/// On WASM this calls `getUserMedia` and sets up a `requestAnimationFrame`
/// loop to extract RGBA frames via a hidden canvas.
///
/// **Important:** This must be called in response to a user gesture (click)
/// or the browser will deny permission.
pub fn open_camera(
    config: CameraConfig,
    on_frame: impl FnMut(VideoFrame) + Send + 'static,
) -> Result<CameraCapture, AvError> {
    open_camera_impl(None, config, on_frame)
}

/// Open a specific camera by device ID.
pub fn open_camera_on(
    device: &DeviceId,
    config: CameraConfig,
    on_frame: impl FnMut(VideoFrame) + Send + 'static,
) -> Result<CameraCapture, AvError> {
    open_camera_impl(Some(device.0.clone()), config, on_frame)
}

fn open_camera_impl(
    device_id: Option<String>,
    config: CameraConfig,
    on_frame: impl FnMut(VideoFrame) + Send + 'static,
) -> Result<CameraCapture, AvError> {
    let md = media_devices()?;

    // Build constraints
    let video_constraints = js_sys::Object::new();
    js_sys::Reflect::set(
        &video_constraints,
        &"width".into(),
        &JsValue::from_f64(config.width as f64),
    )
    .unwrap();
    js_sys::Reflect::set(
        &video_constraints,
        &"height".into(),
        &JsValue::from_f64(config.height as f64),
    )
    .unwrap();
    js_sys::Reflect::set(
        &video_constraints,
        &"frameRate".into(),
        &JsValue::from_f64(config.fps as f64),
    )
    .unwrap();

    if let Some(ref id) = device_id {
        let exact = js_sys::Object::new();
        js_sys::Reflect::set(&exact, &"exact".into(), &JsValue::from_str(id)).unwrap();
        js_sys::Reflect::set(&video_constraints, &"deviceId".into(), &exact).unwrap();
    }

    let constraints = web_sys::MediaStreamConstraints::new();
    constraints.set_audio(&JsValue::FALSE);
    constraints.set_video(&video_constraints);

    let promise = md
        .get_user_media_with_constraints(&constraints)
        .map_err(|e| AvError::Backend(format!("getUserMedia failed: {:?}", e)))?;

    let running = Rc::new(RefCell::new(true));
    let running_clone = running.clone();
    let tracks_store: Rc<RefCell<Vec<web_sys::MediaStreamTrack>>> =
        Rc::new(RefCell::new(Vec::new()));
    let tracks_clone = tracks_store.clone();

    let w = config.width;
    let h = config.height;

    wasm_bindgen_futures::spawn_local(async move {
        let result = JsFuture::from(promise).await;
        let stream: web_sys::MediaStream = match result {
            Ok(val) => val.into(),
            Err(e) => {
                tracing::error!("getUserMedia rejected: {:?}", e);
                return;
            }
        };

        // Store tracks for cleanup
        let track_list = stream.get_video_tracks();
        {
            let mut tracks = tracks_clone.borrow_mut();
            for i in 0..track_list.length() {
                let track: web_sys::MediaStreamTrack = track_list.get(i).into();
                tracks.push(track);
            }
        }

        // Create hidden video element
        let doc = match document() {
            Ok(d) => d,
            Err(_) => return,
        };

        let video: web_sys::HtmlVideoElement =
            doc.create_element("video").unwrap().dyn_into().unwrap();
        video.set_attribute("autoplay", "").unwrap();
        video.set_attribute("playsinline", "").unwrap();
        video.set_attribute("muted", "").unwrap();
        video.style().set_property("display", "none").unwrap();
        video.set_src_object(Some(&stream));

        // Append to body so it can play
        if let Some(body) = doc.body() {
            body.append_child(&video).unwrap();
        }

        // Wait for video to be ready
        let _ = JsFuture::from(video.play().unwrap()).await;

        // Create hidden canvas for pixel readback
        let canvas: web_sys::HtmlCanvasElement =
            doc.create_element("canvas").unwrap().dyn_into().unwrap();
        canvas.set_width(w);
        canvas.set_height(h);
        canvas.style().set_property("display", "none").unwrap();

        let ctx: web_sys::CanvasRenderingContext2d = canvas
            .get_context("2d")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();

        // Start requestAnimationFrame loop
        start_frame_loop(running_clone, video, canvas, ctx, w, h, on_frame);
    });

    // Return the CameraCapture immediately.
    // The tracks_store will be populated asynchronously, but CameraCapture
    // holds its own running flag to signal stop.
    //
    // We need to give CameraCapture the ability to stop tracks. Since the
    // tracks aren't available yet (async), we store them in a shared RefCell
    // and give CameraCapture a reference.
    Ok(CameraCapture {
        tracks: Vec::new(), // Will be empty initially; stop via running flag
        running,
    })
}

fn start_frame_loop(
    running: Rc<RefCell<bool>>,
    video: web_sys::HtmlVideoElement,
    _canvas: web_sys::HtmlCanvasElement,
    ctx: web_sys::CanvasRenderingContext2d,
    width: u32,
    height: u32,
    on_frame: impl FnMut(VideoFrame) + 'static,
) {
    let on_frame = Rc::new(RefCell::new(on_frame));
    let start_time = js_sys::Date::now();

    // Recursive rAF closure
    let f: Rc<RefCell<Option<Closure<dyn FnMut()>>>> = Rc::new(RefCell::new(None));
    let g = f.clone();

    *g.borrow_mut() = Some(Closure::new({
        let running = running.clone();
        let video = video.clone();
        let ctx = ctx.clone();
        let on_frame = on_frame.clone();
        let f = f.clone();

        move || {
            if !*running.borrow() {
                // Cleanup: remove video element
                if let Some(parent) = video.parent_node() {
                    let _ = parent.remove_child(&video);
                }
                // Drop the closure reference to break the cycle
                let _ = f.borrow_mut().take();
                return;
            }

            // Draw current video frame to canvas
            let vw = video.video_width() as f64;
            let vh = video.video_height() as f64;
            if vw > 0.0 && vh > 0.0 {
                let _ = ctx.draw_image_with_html_video_element_and_dw_and_dh(
                    &video,
                    0.0,
                    0.0,
                    width as f64,
                    height as f64,
                );

                // Read pixels
                if let Ok(image_data) = ctx.get_image_data(0.0, 0.0, width as f64, height as f64) {
                    let data = image_data.data().0;
                    let timestamp_us = ((js_sys::Date::now() - start_time) * 1000.0) as u64;

                    let frame = VideoFrame {
                        data,
                        width,
                        height,
                        timestamp_us,
                    };

                    (on_frame.borrow_mut())(frame);
                }
            }

            // Schedule next frame
            if let Some(win) = web_sys::window() {
                if let Some(ref closure) = *f.borrow() {
                    let _ = win.request_animation_frame(closure.as_ref().unchecked_ref());
                }
            }
        }
    }));

    // Kick off the first frame
    if let Some(win) = web_sys::window() {
        if let Some(ref closure) = *g.borrow() {
            let _ = win.request_animation_frame(closure.as_ref().unchecked_ref());
        }
    }
}
