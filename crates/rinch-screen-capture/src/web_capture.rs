//! WASM screen capture via `getDisplayMedia()`.
//!
//! The browser handles the picker dialog and capture. We read frames from the
//! MediaStream using a hidden `<video>` element + `<canvas>` pixel readback.
//!
//! Because `getDisplayMedia()` is async and `ScreenCapture::start()` is sync,
//! we return immediately and run the async setup in `spawn_local`. The
//! `VideoTrack` starts receiving frames once the user picks a source.

use std::cell::Cell;
use std::rc::Rc;

use js_sys::Reflect;
use rinch_av::VideoTrack;
use rinch_av::camera::VideoFrame;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::{CaptureConfig, CaptureError};

/// Handle to a browser screen capture session.
///
/// Dropping this stops all MediaStream tracks and cancels the rAF loop.
pub(crate) struct WebCapture {
    /// Shared flag — set to `true` to stop the rAF loop and clean up.
    stop: Rc<Cell<bool>>,
    /// The MediaStream, kept alive so we can stop tracks on drop.
    stream: Rc<Cell<Option<web_sys::MediaStream>>>,
    /// Hidden DOM elements to clean up on drop.
    video: Rc<Cell<Option<web_sys::HtmlVideoElement>>>,
    canvas: Rc<Cell<Option<web_sys::HtmlCanvasElement>>>,
}

impl WebCapture {
    /// Start screen capture.
    ///
    /// Returns immediately. The actual `getDisplayMedia()` call and frame loop
    /// run asynchronously via `spawn_local`. If the user cancels or an error
    /// occurs, the `VideoTrack` simply never receives frames.
    pub fn start(track: &VideoTrack, config: &CaptureConfig) -> Result<Self, CaptureError> {
        let stop = Rc::new(Cell::new(false));
        let stream_holder: Rc<Cell<Option<web_sys::MediaStream>>> = Rc::new(Cell::new(None));
        let video_holder: Rc<Cell<Option<web_sys::HtmlVideoElement>>> = Rc::new(Cell::new(None));
        let canvas_holder: Rc<Cell<Option<web_sys::HtmlCanvasElement>>> = Rc::new(Cell::new(None));

        let sink = track.frame_sink();
        let max_fps = config.max_fps;
        let show_cursor = config.show_cursor;

        let stop_clone = stop.clone();
        let stream_clone = stream_holder.clone();
        let video_clone = video_holder.clone();
        let canvas_clone = canvas_holder.clone();

        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) = run_capture(
                sink,
                max_fps,
                show_cursor,
                stop_clone,
                stream_clone,
                video_clone,
                canvas_clone,
            )
            .await
            {
                tracing::warn!("screen capture error: {e}");
            }
        });

        Ok(Self {
            stop,
            stream: stream_holder,
            video: video_holder,
            canvas: canvas_holder,
        })
    }
}

impl Drop for WebCapture {
    fn drop(&mut self) {
        // Signal the rAF loop to stop.
        self.stop.set(true);

        // Stop all MediaStream tracks.
        if let Some(stream) = self.stream.take() {
            let tracks = stream.get_tracks();
            for i in 0..tracks.length() {
                if let Some(track) = tracks.get(i).dyn_ref::<web_sys::MediaStreamTrack>() {
                    track.stop();
                }
            }
        }

        // Remove hidden DOM elements.
        if let Some(video) = self.video.take() {
            video.remove();
        }
        if let Some(canvas) = self.canvas.take() {
            canvas.remove();
        }
    }
}

/// Run the async capture pipeline.
async fn run_capture(
    sink: crate::FrameSink,
    max_fps: u32,
    show_cursor: bool,
    stop: Rc<Cell<bool>>,
    stream_holder: Rc<Cell<Option<web_sys::MediaStream>>>,
    video_holder: Rc<Cell<Option<web_sys::HtmlVideoElement>>>,
    canvas_holder: Rc<Cell<Option<web_sys::HtmlCanvasElement>>>,
) -> Result<(), String> {
    // 1. Call getDisplayMedia().
    let window = web_sys::window().ok_or("no global window")?;
    let navigator = window.navigator();
    let media_devices = navigator
        .media_devices()
        .map_err(|e| format!("mediaDevices: {e:?}"))?;

    // Build constraints: { video: { cursor: "always"/"never" } }
    let constraints = web_sys::DisplayMediaStreamConstraints::new();
    let video_constraints = js_sys::Object::new();
    let cursor_value = if show_cursor { "always" } else { "never" };
    Reflect::set(
        &video_constraints,
        &JsValue::from_str("cursor"),
        &JsValue::from_str(cursor_value),
    )
    .map_err(|e| format!("set cursor constraint: {e:?}"))?;
    constraints.set_video(&video_constraints.into());

    let stream_promise = media_devices
        .get_display_media_with_constraints(&constraints)
        .map_err(|e| format!("getDisplayMedia: {e:?}"))?;

    let stream_js = JsFuture::from(stream_promise).await.map_err(|e| {
        // User cancel shows up as a NotAllowedError DOMException.
        let msg = format!("{e:?}");
        if msg.contains("NotAllowedError") || msg.contains("Permission denied") {
            "cancelled".to_string()
        } else {
            format!("getDisplayMedia failed: {msg}")
        }
    })?;

    let stream: web_sys::MediaStream = stream_js
        .dyn_into()
        .map_err(|_| "getDisplayMedia did not return a MediaStream")?;

    // Store stream so Drop can stop tracks.
    stream_holder.set(Some(stream.clone()));

    // 2. Create hidden <video> element and attach the stream.
    let document = window.document().ok_or("no document")?;

    let video: web_sys::HtmlVideoElement = document
        .create_element("video")
        .map_err(|e| format!("create video: {e:?}"))?
        .dyn_into()
        .map_err(|_| "not an HtmlVideoElement")?;

    video
        .set_attribute("playsinline", "")
        .map_err(|e| format!("{e:?}"))?;
    video.set_autoplay(true);
    video.set_muted(true);
    // Hide it off-screen.
    video
        .style()
        .set_property("position", "fixed")
        .map_err(|e| format!("{e:?}"))?;
    video
        .style()
        .set_property("left", "-9999px")
        .map_err(|e| format!("{e:?}"))?;
    video
        .style()
        .set_property("top", "-9999px")
        .map_err(|e| format!("{e:?}"))?;
    video
        .style()
        .set_property("width", "1px")
        .map_err(|e| format!("{e:?}"))?;
    video
        .style()
        .set_property("height", "1px")
        .map_err(|e| format!("{e:?}"))?;

    document
        .body()
        .ok_or("no body")?
        .append_child(&video)
        .map_err(|e| format!("append video: {e:?}"))?;

    video.set_src_object(Some(&stream));

    // Wait for the video to start playing so we know dimensions.
    let play_promise = video.play().map_err(|e| format!("video.play: {e:?}"))?;
    JsFuture::from(play_promise)
        .await
        .map_err(|e| format!("video play failed: {e:?}"))?;

    video_holder.set(Some(video.clone()));

    // 3. Create hidden <canvas> for pixel readback.
    let canvas: web_sys::HtmlCanvasElement = document
        .create_element("canvas")
        .map_err(|e| format!("create canvas: {e:?}"))?
        .dyn_into()
        .map_err(|_| "not an HtmlCanvasElement")?;

    canvas
        .style()
        .set_property("display", "none")
        .map_err(|e| format!("{e:?}"))?;

    document
        .body()
        .ok_or("no body")?
        .append_child(&canvas)
        .map_err(|e| format!("append canvas: {e:?}"))?;

    canvas_holder.set(Some(canvas.clone()));

    let ctx: web_sys::CanvasRenderingContext2d = canvas
        .get_context("2d")
        .map_err(|e| format!("getContext 2d: {e:?}"))?
        .ok_or("canvas 2d context is None")?
        .dyn_into()
        .map_err(|_| "not a CanvasRenderingContext2d")?;

    // 4. rAF loop: draw video → read pixels → push frame.
    let min_frame_interval_ms = if max_fps > 0 {
        1000.0 / max_fps as f64
    } else {
        0.0
    };
    let last_frame_time = Rc::new(Cell::new(0.0_f64));
    let perf = window.performance().ok_or("no performance object")?;

    // Listen for the stream ending (user clicks "Stop sharing" in browser UI).
    let stop_on_end = stop.clone();
    let track_list = stream.get_video_tracks();
    if track_list.length() > 0 {
        if let Some(first_track) = track_list.get(0).dyn_ref::<web_sys::MediaStreamTrack>() {
            let onended = Closure::<dyn FnMut()>::new(move || {
                stop_on_end.set(true);
            });
            first_track.set_onended(Some(onended.as_ref().unchecked_ref()));
            onended.forget(); // leak — cleaned up when track/page is destroyed
        }
    }

    // Start the rAF loop using a recursive closure pattern.
    run_raf_loop(
        video,
        canvas,
        ctx,
        sink,
        stop,
        last_frame_time,
        min_frame_interval_ms,
        perf,
    );

    Ok(())
}

/// Kick off the requestAnimationFrame loop.
fn run_raf_loop(
    video: web_sys::HtmlVideoElement,
    canvas: web_sys::HtmlCanvasElement,
    ctx: web_sys::CanvasRenderingContext2d,
    sink: crate::FrameSink,
    stop: Rc<Cell<bool>>,
    last_frame_time: Rc<Cell<f64>>,
    min_interval_ms: f64,
    perf: web_sys::Performance,
) {
    // Shared closure reference for self-scheduling.
    let closure: Rc<Cell<Option<Closure<dyn FnMut()>>>> = Rc::new(Cell::new(None));
    let closure_clone = closure.clone();

    let cb = Closure::<dyn FnMut()>::new(move || {
        if stop.get() {
            // Drop the closure to break the cycle.
            closure_clone.set(None);
            return;
        }

        let now = perf.now();

        // FPS throttling.
        if min_interval_ms > 0.0 && (now - last_frame_time.get()) < min_interval_ms {
            // Schedule next frame and skip this one.
            schedule_raf(&closure_clone);
            return;
        }
        last_frame_time.set(now);

        let vw = video.video_width();
        let vh = video.video_height();

        if vw > 0 && vh > 0 {
            // Resize canvas if needed.
            if canvas.width() != vw || canvas.height() != vh {
                canvas.set_width(vw);
                canvas.set_height(vh);
            }

            // Draw video frame to canvas.
            if ctx
                .draw_image_with_html_video_element(&video, 0.0, 0.0)
                .is_ok()
            {
                // Read pixels.
                if let Ok(image_data) = ctx.get_image_data(0.0, 0.0, vw as f64, vh as f64) {
                    let data = image_data.data();
                    let frame = VideoFrame {
                        data: data.0,
                        width: vw,
                        height: vh,
                        timestamp_us: (now * 1000.0) as u64,
                    };
                    // Push to the shared sink (same mechanism as PipeWire capture).
                    if let Ok(mut guard) = sink.lock() {
                        *guard = Some(frame);
                    }
                }
            }
        }

        // Schedule next frame.
        schedule_raf(&closure_clone);
    });

    // Store and schedule the first frame.
    closure.set(Some(cb));
    schedule_raf(&closure);
}

/// Schedule the closure for the next requestAnimationFrame.
fn schedule_raf(closure_cell: &Rc<Cell<Option<Closure<dyn FnMut()>>>>) {
    // We need to temporarily take the closure out to get a reference to it.
    let cb = closure_cell.take();
    if let Some(ref closure) = cb {
        if let Some(window) = web_sys::window() {
            let _ = window.request_animation_frame(closure.as_ref().unchecked_ref());
        }
    }
    // Put it back.
    closure_cell.set(cb);
}
