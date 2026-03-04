//! HTMLVideoElement-based video player for WASM.
//!
//! On WASM, the browser handles all video decoding, rendering, and audio.
//! We find the `<video>` element created by [`VideoViewport`] via the
//! `data-video-player` attribute and wire DOM events to Signals.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::Ordering;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::HtmlVideoElement;

use rinch_core::Signal;

use crate::player::{PlaybackState, VideoPlayer, VideoPlayerBackend};

/// Web-based video player using HTMLVideoElement.
pub struct WebVideoPlayer {
    /// Viewport ID — matches the `data-video-player` attribute on the `<video>` element.
    viewport_id: Rc<RefCell<String>>,
    /// Cached reference to the video element.
    element: Rc<RefCell<Option<HtmlVideoElement>>>,
    /// Player signals — written to by event listeners.
    state: Signal<PlaybackState>,
    position: Signal<f64>,
    duration: Signal<f64>,
    volume: Signal<f32>,
    muted: Signal<bool>,
    buffered: Signal<f64>,
    playing: Signal<bool>,
    /// Keep closures alive for the lifetime of the player.
    _closures: Rc<RefCell<Vec<Closure<dyn FnMut(web_sys::Event)>>>>,
}

impl std::fmt::Debug for WebVideoPlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebVideoPlayer")
            .field("viewport_id", &*self.viewport_id.borrow())
            .finish_non_exhaustive()
    }
}

impl WebVideoPlayer {
    /// Try to find the `<video>` element in the DOM and cache it.
    fn ensure_element(&self) -> Option<HtmlVideoElement> {
        if let Some(ref el) = *self.element.borrow() {
            return Some(el.clone());
        }
        let vid = self.viewport_id.borrow();
        let document = web_sys::window()?.document()?;
        let selector = format!("[data-video-player=\"{vid}\"]");
        let el = document.query_selector(&selector).ok()??;
        let video: HtmlVideoElement = el.dyn_into().ok()?;
        *self.element.borrow_mut() = Some(video.clone());
        Some(video)
    }
}

impl VideoPlayerBackend for WebVideoPlayer {
    fn play(&self) {
        if let Some(el) = self.ensure_element() {
            let _ = el.play();
        }
    }

    fn pause(&self) {
        if let Some(el) = self.ensure_element() {
            el.pause().ok();
        }
    }

    fn seek(&self, seconds: f64) {
        if let Some(el) = self.ensure_element() {
            el.set_current_time(seconds);
        }
    }

    fn set_volume(&self, vol: f32) {
        if let Some(el) = self.ensure_element() {
            el.set_volume(vol as f64);
        }
    }

    fn set_muted(&self, muted: bool) {
        if let Some(el) = self.ensure_element() {
            el.set_muted(muted);
        }
    }

    fn set_source(&self, src: &str) {
        // The <video> element may not be in the DOM yet (VideoViewport renders
        // after set_source). Defer via microtask to let the DOM mount first.
        let src = src.to_string();
        let element_rc = self.element.clone();
        let viewport_id = self.viewport_id.borrow().clone();
        let closures_rc = self._closures.clone();
        let state = self.state;
        let position = self.position;
        let duration = self.duration;
        let volume = self.volume;
        let muted = self.muted;
        let buffered = self.buffered;
        let playing = self.playing;

        let closure = Closure::once(move || {
            let document = web_sys::window().unwrap().document().unwrap();
            let selector = format!("[data-video-player=\"{viewport_id}\"]");
            let Some(el) = document.query_selector(&selector).ok().flatten() else {
                tracing::warn!("VideoViewport not found for {viewport_id}");
                return;
            };
            let Ok(video) = el.dyn_into::<HtmlVideoElement>() else {
                return;
            };

            let target: &web_sys::EventTarget = video.as_ref();
            let mut cbs = closures_rc.borrow_mut();

            // loadedmetadata — duration becomes available
            {
                let v = video.clone();
                let cb = Closure::wrap(Box::new(move |_: web_sys::Event| {
                    duration.set(v.duration());
                    if state.get() == PlaybackState::Loading {
                        state.set(PlaybackState::Paused);
                    }
                }) as Box<dyn FnMut(_)>);
                target
                    .add_event_listener_with_callback("loadedmetadata", cb.as_ref().unchecked_ref())
                    .ok();
                cbs.push(cb);
            }
            // play
            {
                let cb = Closure::wrap(Box::new(move |_: web_sys::Event| {
                    playing.set(true);
                    state.set(PlaybackState::Playing);
                }) as Box<dyn FnMut(_)>);
                target
                    .add_event_listener_with_callback("play", cb.as_ref().unchecked_ref())
                    .ok();
                cbs.push(cb);
            }
            // pause
            {
                let cb = Closure::wrap(Box::new(move |_: web_sys::Event| {
                    playing.set(false);
                    state.set(PlaybackState::Paused);
                }) as Box<dyn FnMut(_)>);
                target
                    .add_event_listener_with_callback("pause", cb.as_ref().unchecked_ref())
                    .ok();
                cbs.push(cb);
            }
            // ended
            {
                let cb = Closure::wrap(Box::new(move |_: web_sys::Event| {
                    playing.set(false);
                    state.set(PlaybackState::Ended);
                }) as Box<dyn FnMut(_)>);
                target
                    .add_event_listener_with_callback("ended", cb.as_ref().unchecked_ref())
                    .ok();
                cbs.push(cb);
            }
            // timeupdate
            {
                let v = video.clone();
                let cb = Closure::wrap(Box::new(move |_: web_sys::Event| {
                    position.set(v.current_time());
                }) as Box<dyn FnMut(_)>);
                target
                    .add_event_listener_with_callback("timeupdate", cb.as_ref().unchecked_ref())
                    .ok();
                cbs.push(cb);
            }
            // progress (buffered data)
            {
                let v = video.clone();
                let cb = Closure::wrap(Box::new(move |_: web_sys::Event| {
                    let ranges = v.buffered();
                    let pos = position.get();
                    let mut buf_end = pos;
                    for i in 0..ranges.length() {
                        if let (Ok(start), Ok(end)) = (ranges.start(i), ranges.end(i)) {
                            if start <= pos && pos <= end {
                                buf_end = end;
                                break;
                            }
                        }
                    }
                    buffered.set(buf_end - pos);
                }) as Box<dyn FnMut(_)>);
                target
                    .add_event_listener_with_callback("progress", cb.as_ref().unchecked_ref())
                    .ok();
                cbs.push(cb);
            }
            // volumechange
            {
                let v = video.clone();
                let cb = Closure::wrap(Box::new(move |_: web_sys::Event| {
                    volume.set(v.volume() as f32);
                    muted.set(v.muted());
                }) as Box<dyn FnMut(_)>);
                target
                    .add_event_listener_with_callback("volumechange", cb.as_ref().unchecked_ref())
                    .ok();
                cbs.push(cb);
            }
            // error
            {
                let v = video.clone();
                let cb = Closure::wrap(Box::new(move |_: web_sys::Event| {
                    let msg = v
                        .error()
                        .map(|e| format!("Media error code {}", e.code()))
                        .unwrap_or_else(|| "Unknown error".into());
                    state.set(PlaybackState::Error(msg));
                }) as Box<dyn FnMut(_)>);
                target
                    .add_event_listener_with_callback("error", cb.as_ref().unchecked_ref())
                    .ok();
                cbs.push(cb);
            }

            video.set_src(&src);
            *element_rc.borrow_mut() = Some(video);
        });

        let window = web_sys::window().unwrap();
        window.queue_microtask(closure.as_ref().unchecked_ref());
        closure.forget();
    }

    fn cleanup(&self) {
        if let Some(el) = self.element.borrow().as_ref() {
            el.pause().ok();
            el.set_src("");
            el.load();
        }
    }
}

/// Create a web-based video player.
///
/// Creates shared signals between the backend and the player so DOM event
/// listeners update the player's signals directly.
pub fn create_web_player(src: &str) -> VideoPlayer {
    // Peek at the next player ID (VideoPlayer::new will allocate it)
    let next_id = crate::player::NEXT_PLAYER_ID.load(Ordering::Relaxed);
    let viewport_id = Rc::new(RefCell::new(format!("video-{next_id}")));

    // Shared signals — backend event listeners write, player UI reads
    let playing = Signal::new(false);
    let position = Signal::new(0.0);
    let duration = Signal::new(0.0);
    let volume = Signal::new(1.0);
    let muted_sig = Signal::new(false);
    let buffered = Signal::new(0.0);
    let state = Signal::new(PlaybackState::Idle);

    let backend = WebVideoPlayer {
        viewport_id,
        element: Rc::new(RefCell::new(None)),
        state,
        position,
        duration,
        volume,
        muted: muted_sig,
        buffered,
        playing,
        _closures: Rc::new(RefCell::new(Vec::new())),
    };

    let mut player = VideoPlayer::new(Box::new(backend));
    // Override player's signals with the shared ones
    player.playing = playing;
    player.position = position;
    player.duration = duration;
    player.volume = volume;
    player.muted = muted_sig;
    player.buffered = buffered;
    player.state = state;

    if !src.is_empty() {
        player.set_source(src);
    }
    player
}
