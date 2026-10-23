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
//! Not seen: a box moved by a `:hover` rule without resizing, and the frames in
//! the middle of a transition or animation that moves (not resizes) a box —
//! the rect is refreshed when it ends.

use std::cell::RefCell;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// One watched element: the document and node id its signals are registered
/// under, and the element itself.
struct Watched {
    doc_key: u64,
    node_id: u64,
    el: web_sys::Element,
}

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
    watched: Vec<Watched>,
    installed: Option<Installed>,
    frame_pending: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Start refreshing the bounds signals registered for `node_id` of the
/// document `doc_key` (called by `WebDocument::watch_bounds`, under the
/// document borrow: nothing here runs app code).
pub(crate) fn watch(doc_key: u64, node_id: u64, el: web_sys::Element) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.installed.is_none() {
            s.installed = install();
        }
        let known = s
            .watched
            .iter()
            .any(|w| w.doc_key == doc_key && w.node_id == node_id);
        if !known {
            if let Some(i) = &s.installed {
                i.resize.observe(&el);
            }
            s.watched.push(Watched {
                doc_key,
                node_id,
                el,
            });
        }
    });
    schedule();
}

/// Forget everything `doc_key` watched (its document is being dropped).
pub(crate) fn forget_document(doc_key: u64) {
    let _ = STATE.try_with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            let State {
                watched, installed, ..
            } = &mut *s;
            watched.retain(|w| {
                let keep = w.doc_key != doc_key;
                if !keep && let Some(i) = installed {
                    i.resize.unobserve(&w.el);
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
    let measured: Vec<(u64, Vec<(u64, Rect)>)> = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.frame_pending = false;
        let State {
            watched, installed, ..
        } = &mut *s;
        // Drop the elements whose signals are all gone.
        let mut live: Vec<(u64, Vec<u64>)> = Vec::new();
        watched.retain(|w| {
            let ids = match live.iter().position(|(k, _)| *k == w.doc_key) {
                Some(i) => &live[i].1,
                None => {
                    live.push((
                        w.doc_key,
                        rinch_core::reactive::registered_bounds_nodes(w.doc_key),
                    ));
                    &live.last().unwrap().1
                }
            };
            let keep = ids.contains(&w.node_id);
            if !keep && let Some(i) = installed {
                i.resize.unobserve(&w.el);
            }
            keep
        });
        if watched.is_empty() {
            uninstall(installed.take());
            return Vec::new();
        }
        let mut out: Vec<(u64, Vec<(u64, Rect)>)> = Vec::new();
        for w in watched.iter() {
            if !w.el.is_connected() {
                continue;
            }
            let r = w.el.get_bounding_client_rect();
            let rect = (r.x() as f32, r.y() as f32, r.width() as f32, r.height() as f32);
            match out.iter_mut().find(|(k, _)| *k == w.doc_key) {
                Some((_, v)) => v.push((w.node_id, rect)),
                None => out.push((w.doc_key, vec![(w.node_id, rect)])),
            }
        }
        out
    });
    for (doc_key, rects) in measured {
        rinch_core::reactive::update_bounds_signals(doc_key, |node_id| {
            rects
                .iter()
                .find(|(id, _)| *id == node_id)
                .map(|(_, r)| *r)
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
