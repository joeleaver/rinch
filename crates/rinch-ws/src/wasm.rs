//! Web (wasm32) WebSocket backend backed by `web_sys::WebSocket`.
//!
//! Everything runs on the single web thread (the UI thread), so incoming events
//! are dispatched to the callback registry directly — no
//! `rinch_core::run_on_main_thread` hop (its `Send` bound a JS-capturing closure
//! could not satisfy anyway).
//!
//! The JS event closures are kept alive by the [`Backend`] for as long as the
//! connection is open; [`Backend::close`] detaches them from the socket before
//! they are dropped so a late browser event can never invoke a freed closure.

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{BinaryType, CloseEvent, MessageEvent, WebSocket};

use crate::{WsClose, WsError, WsEvent, WsMessage};

/// The web half of a connection: the socket plus its live event closures.
pub(crate) struct Backend {
    ws: WebSocket,
    // Kept alive; dropped (after being detached in `close`) with the backend.
    _on_open: Closure<dyn FnMut(JsValue)>,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
    _on_close: Closure<dyn FnMut(CloseEvent)>,
    _on_error: Closure<dyn FnMut(JsValue)>,
}

pub(crate) fn connect(url: &str, id: u64) -> Result<Backend, WsError> {
    let ws = WebSocket::new(url).map_err(|e| WsError::InvalidUrl(js_err(&e)))?;
    ws.set_binary_type(BinaryType::Arraybuffer);

    let on_open = Closure::wrap(Box::new(move |_e: JsValue| {
        crate::dispatch(id, WsEvent::Open);
    }) as Box<dyn FnMut(JsValue)>);

    let on_message = Closure::wrap(Box::new(move |e: MessageEvent| {
        let data = e.data();
        if let Some(text) = data.as_string() {
            crate::dispatch(id, WsEvent::Message(WsMessage::Text(text)));
        } else if data.is_instance_of::<js_sys::ArrayBuffer>() {
            let bytes = js_sys::Uint8Array::new(&data).to_vec();
            crate::dispatch(id, WsEvent::Message(WsMessage::Binary(bytes)));
        }
        // Blob is never produced: binary_type is forced to ArrayBuffer above.
    }) as Box<dyn FnMut(MessageEvent)>);

    let on_close = Closure::wrap(Box::new(move |e: CloseEvent| {
        crate::dispatch(
            id,
            WsEvent::Close(WsClose {
                code: e.code(),
                reason: e.reason(),
            }),
        );
    }) as Box<dyn FnMut(CloseEvent)>);

    let on_error = Closure::wrap(Box::new(move |_e: JsValue| {
        // The browser `error` event carries no useful detail; a `close` with the
        // real code follows.
        crate::dispatch(
            id,
            WsEvent::Error(WsError::Protocol("websocket error".to_string())),
        );
    }) as Box<dyn FnMut(JsValue)>);

    ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    ws.set_onerror(Some(on_error.as_ref().unchecked_ref()));

    Ok(Backend {
        ws,
        _on_open: on_open,
        _on_message: on_message,
        _on_close: on_close,
        _on_error: on_error,
    })
}

impl Backend {
    pub(crate) fn send_text(&self, text: String) {
        let _ = self.ws.send_with_str(&text);
    }

    pub(crate) fn send_bytes(&self, bytes: Vec<u8>) {
        let _ = self.ws.send_with_u8_array(&bytes);
    }

    pub(crate) fn close(&self) {
        // Detach handlers first: the closures are about to be dropped with the
        // backend, and a browser `close` event must not fire into a freed closure.
        self.ws.set_onopen(None);
        self.ws.set_onmessage(None);
        self.ws.set_onclose(None);
        self.ws.set_onerror(None);
        let _ = self.ws.close();
    }
}

/// Render a `JsValue` error into a debug string.
fn js_err(e: &JsValue) -> String {
    format!("{e:?}")
}
