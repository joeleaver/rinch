//! `NodeHandle::bounds_signal()` on the web (issue #1781).
//!
//! Desktop refreshes every registered bounds signal after each layout pass
//! (`RinchApp::refresh_bounds_signals`). The browser lays out on its own, so
//! rinch-web watches what can move a watched element and refreshes the signals
//! from `getBoundingClientRect()` in the next animation frame:
//!
//! - the element's own resize (a `ResizeObserver` over every watched element —
//!   this is also what sees a size a CSS animation or a `:hover` rule changes),
//! - any DOM write (a `MutationObserver` on the document: a sibling inserted
//!   above, a class flipped, text changed — each can move a box without
//!   resizing it),
//! - a scroll of anything (a capturing `scroll` listener on the document),
//! - a window resize, an image `load` (captured), the end of a transition or an
//!   animation, and `document.fonts` finishing a load.
//!
//! Every one of those only schedules one refresh per frame. The observers and
//! listeners exist only while something is watched: they are installed by the
//! first [`watch`] and removed by the refresh that finds no registered signal
//! left, so a page with no bounds signal pays nothing.
//!
//! Not seen, or seen late:
//! - a box moved by a `:hover` rule without resizing, and the frames in the
//!   middle of a transition or animation that moves (not resizes) a box — the
//!   rect is refreshed when it ends;
//! - a DOM write or a scroll inside a **shadow root** (an island mounted in
//!   one): the `MutationObserver` and the `scroll` listener are on `document`,
//!   which neither reaches. The element's own resize still is seen;
//! - a change reaches the signal in the frame **after** it happens (the
//!   observers report after layout, the refresh is the next frame), not in the
//!   frame that paints it, as desktop's post-layout refresh does.
//!
//! Cost while anything is watched: every DOM write anywhere in the page makes a
//! mutation record and asks for a refresh (one per frame however many).

use std::cell::RefCell;
use std::collections::HashMap;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// The observers and listeners, alive while anything is watched.
struct Installed {
    resize: web_sys::ResizeObserver,
    mutation: web_sys::MutationObserver,
    /// `(target, event, capture)` for each listener added with [`Self::wake`].
    listeners: Vec<(web_sys::EventTarget, &'static str, bool)>,
    /// Schedules a refresh; shared by the observers and every listener.
    wake: Closure<dyn FnMut()>,
}

#[derive(Default)]
struct State {
    /// The watched elements by `(doc_key, node_id)` — the key their signals
    /// are registered under.
    watched: HashMap<(u64, u64), web_sys::Element>,
    installed: Option<Installed>,
    frame_pending: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Start refreshing the bounds signals registered for `node_id` of the
/// document `doc_key` (called by `WebDocument::watch_bounds`, under the
/// document borrow: nothing here runs app code).
///
/// Called on every `bounds_signal()` call; a node already watched costs one
/// lookup and schedules nothing.
pub(crate) fn watch(doc_key: u64, node_id: u64, el: &web_sys::Element) {
    let new = STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.watched.contains_key(&(doc_key, node_id)) {
            return false;
        }
        if s.installed.is_none() {
            s.installed = install();
        }
        if let Some(i) = &s.installed {
            i.resize.observe(el);
        }
        s.watched.insert((doc_key, node_id), el.clone());
        true
    });
    if new {
        schedule();
    }
}

/// Forget everything `doc_key` watched (its document is being dropped).
pub(crate) fn forget_document(doc_key: u64) {
    let _ = STATE.try_with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            let State {
                watched, installed, ..
            } = &mut *s;
            watched.retain(|&(k, _), el| {
                let keep = k != doc_key;
                if !keep && let Some(i) = installed {
                    i.resize.unobserve(el);
                }
                keep
            });
            if watched.is_empty() {
                uninstall(installed.take());
            }
        }
    });
}

fn install() -> Option<Installed> {
    let window = web_sys::window()?;
    let document = window.document()?;
    let wake = Closure::<dyn FnMut()>::new(schedule);
    let resize = web_sys::ResizeObserver::new(wake.as_ref().unchecked_ref()).ok()?;
    let mutation = web_sys::MutationObserver::new(wake.as_ref().unchecked_ref()).ok()?;
    let init = web_sys::MutationObserverInit::new();
    init.set_subtree(true);
    init.set_child_list(true);
    init.set_attributes(true);
    init.set_character_data(true);
    let _ = mutation.observe_with_options(&document, &init);

    let mut listeners: Vec<(web_sys::EventTarget, &'static str, bool)> = vec![
        (window.clone().into(), "resize", false),
        (document.clone().into(), "scroll", true),
        (document.clone().into(), "load", true),
        (document.clone().into(), "transitionend", true),
        (document.clone().into(), "animationend", true),
    ];
    // `document.fonts` (a `FontFaceSet`), read without the web-sys feature.
    if let Ok(fonts) = js_sys::Reflect::get(&document, &JsValue::from_str("fonts"))
        && let Ok(fonts) = fonts.dyn_into::<web_sys::EventTarget>()
    {
        listeners.push((fonts, "loadingdone", false));
    }
    for (target, event, capture) in &listeners {
        let _ = target.add_event_listener_with_callback_and_bool(
            event,
            wake.as_ref().unchecked_ref(),
            *capture,
        );
    }
    Some(Installed {
        resize,
        mutation,
        listeners,
        wake,
    })
}

fn uninstall(installed: Option<Installed>) {
    let Some(i) = installed else { return };
    i.resize.disconnect();
    i.mutation.disconnect();
    for (target, event, capture) in &i.listeners {
        let _ = target.remove_event_listener_with_callback_and_bool(
            event,
            i.wake.as_ref().unchecked_ref(),
            *capture,
        );
    }
    // `i.wake` drops here; nothing refers to it any more.
}

/// Ask for one refresh at the next animation frame.
fn schedule() {
    let first = STATE.with(|s| {
        let mut s = s.borrow_mut();
        !std::mem::replace(&mut s.frame_pending, true)
    });
    if !first {
        return;
    }
    let cb = Closure::once_into_js(refresh);
    let asked = web_sys::window()
        .map(|w| w.request_animation_frame(cb.unchecked_ref()).is_ok())
        .unwrap_or(false);
    if !asked {
        STATE.with(|s| s.borrow_mut().frame_pending = false);
    }
}

/// `(x, y, width, height)`, the shape `update_bounds_signals` reads.
type Rect = (f32, f32, f32, f32);

/// Measure every watched element and publish, with no borrow held while the
/// signal writes run app code.
fn refresh() {
    let measured: HashMap<u64, HashMap<u64, Rect>> = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.frame_pending = false;
        let State {
            watched, installed, ..
        } = &mut *s;
        // Drop the elements whose signals are all gone.
        let mut live: HashMap<u64, std::collections::HashSet<u64>> = HashMap::new();
        watched.retain(|&(doc_key, node_id), el| {
            let keep = live
                .entry(doc_key)
                .or_insert_with(|| {
                    rinch_core::reactive::registered_bounds_nodes(doc_key)
                        .into_iter()
                        .collect()
                })
                .contains(&node_id);
            if !keep && let Some(i) = installed {
                i.resize.unobserve(el);
            }
            keep
        });
        if watched.is_empty() {
            uninstall(installed.take());
            return HashMap::new();
        }
        let mut out: HashMap<u64, HashMap<u64, Rect>> = HashMap::new();
        for (&(doc_key, node_id), el) in watched.iter() {
            if !el.is_connected() {
                continue;
            }
            let r = el.get_bounding_client_rect();
            let rect = (
                r.x() as f32,
                r.y() as f32,
                r.width() as f32,
                r.height() as f32,
            );
            out.entry(doc_key).or_default().insert(node_id, rect);
        }
        out
    });
    for (doc_key, rects) in measured {
        rinch_core::reactive::update_bounds_signals(doc_key, |node_id| {
            rects.get(&node_id).copied()
        });
    }
}

/// How many elements are watched (test-only).
#[doc(hidden)]
pub fn __bounds_watch_count() -> usize {
    STATE.with(|s| s.borrow().watched.len())
}

/// Whether the observers and listeners are installed (test-only).
#[doc(hidden)]
pub fn __bounds_watch_installed() -> bool {
    STATE.with(|s| s.borrow().installed.is_some())
}
