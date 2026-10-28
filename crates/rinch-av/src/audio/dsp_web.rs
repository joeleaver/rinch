//! The web half of [`super::dsp`]: the processor's raw wasm module runs inside an `AudioWorklet`.
//!
//! The page compiles the module (`WebAssembly.compile`) and hands the compiled `WebAssembly.Module`
//! to the worklet in `processorOptions`; the processor instantiates it synchronously with no imports
//! and calls the [`rinch_dsp`] exports once per 128-frame quantum. Messages go in over the node's
//! `MessagePort` and are applied at the top of `process()`; replies come back the same way. No
//! `SharedArrayBuffer`, so the page needs no cross-origin isolation.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::{Rc, Weak};

use js_sys::{Array, Object, Reflect, Uint8Array};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AudioContext, AudioContextOptions, AudioWorkletNode, AudioWorkletNodeOptions, MessageEvent,
};

use super::dsp::DspOutputConfig;
use crate::error::AvError;

/// The worklet side. `{NAME}` is replaced per output, since a processor name registers once per
/// context.
const PROCESSOR: &str = r#"
class RinchDsp extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const o = options.processorOptions;
    this.ex = new WebAssembly.Instance(o.module, {}).exports;
    this.ch = o.channels;
    this.ex.rinch_dsp_init(sampleRate, this.ch);
    this.inbox = [];
    this.port.onmessage = (e) => this.inbox.push(e.data);
  }
  process(inputs, outputs) {
    const ex = this.ex;
    for (const m of this.inbox) {
      const p = ex.rinch_dsp_alloc(m.length);
      new Uint8Array(ex.memory.buffer, p, m.length).set(m);
      ex.rinch_dsp_message(m.length);
    }
    this.inbox.length = 0;
    const out = outputs[0];
    const n = out[0].length;
    const p = ex.rinch_dsp_render(n);
    const samples = new Float32Array(ex.memory.buffer, p, this.ch * n);
    for (let c = 0; c < out.length; c++) {
      const from = Math.min(c, this.ch - 1) * n;
      out[c].set(samples.subarray(from, from + n));
    }
    let len;
    while ((len = ex.rinch_dsp_reply()) > 0) {
      this.port.postMessage(new Uint8Array(ex.memory.buffer, ex.rinch_dsp_reply_ptr(), len).slice());
    }
    return true;
  }
}
registerProcessor("{NAME}", RinchDsp);
"#;

thread_local! {
    static NEXT: Cell<u32> = const { Cell::new(0) };
}

/// The port's reply listener, kept alive as long as the output.
type OnMessage = Closure<dyn FnMut(MessageEvent)>;

struct Shared {
    ctx: AudioContext,
    node: RefCell<Option<AudioWorkletNode>>,
    /// Messages sent before the worklet was running, delivered in order once it is.
    pending: RefCell<Vec<Vec<u8>>>,
    replies: RefCell<VecDeque<Vec<u8>>>,
    on_message: RefCell<Option<OnMessage>>,
}

pub(super) struct Web {
    shared: Rc<Shared>,
}

impl Web {
    pub(super) fn send(&self, bytes: Vec<u8>) {
        match self
            .shared
            .node
            .borrow()
            .as_ref()
            .and_then(|n| n.port().ok())
        {
            Some(port) => {
                let _ = port.post_message(&Uint8Array::from(bytes.as_slice()));
            }
            None => self.shared.pending.borrow_mut().push(bytes),
        }
    }

    pub(super) fn replies(&self) -> Vec<Vec<u8>> {
        self.shared.replies.borrow_mut().drain(..).collect()
    }

    pub(super) fn resume(&self) {
        let _ = self.shared.ctx.resume();
    }
}

impl Drop for Web {
    fn drop(&mut self) {
        if let Some(node) = self.shared.node.borrow_mut().take() {
            let _ = node.disconnect();
            if let Ok(port) = node.port() {
                port.set_onmessage(None);
            }
        }
        let _ = self.shared.ctx.close();
    }
}

fn js_err(e: &JsValue) -> String {
    e.as_string()
        .or_else(|| {
            e.dyn_ref::<js_sys::Error>()
                .map(|e| String::from(e.message()))
        })
        .unwrap_or_else(|| format!("{e:?}"))
}

pub(super) fn open(
    config: DspOutputConfig,
    module: &[u8],
) -> Result<(Web, DspOutputConfig), AvError> {
    let opts = AudioContextOptions::new();
    opts.set_sample_rate(config.sample_rate as f32);
    let ctx = AudioContext::new_with_context_options(&opts)
        .map_err(|e| AvError::Backend(format!("Failed to create AudioContext: {}", js_err(&e))))?;
    let real = DspOutputConfig {
        sample_rate: ctx.sample_rate() as u32,
        ..config
    };
    let shared = Rc::new(Shared {
        ctx,
        node: RefCell::new(None),
        pending: RefCell::new(Vec::new()),
        replies: RefCell::new(VecDeque::new()),
        on_message: RefCell::new(None),
    });
    let bytes = Uint8Array::from(module);
    let channels = config.channels.max(1);
    wasm_bindgen_futures::spawn_local(start(Rc::downgrade(&shared), bytes, channels));
    Ok((Web { shared }, real))
}

/// Load the processor script and the module, then start the node. Gives up quietly if the output
/// was dropped meanwhile; says why in the log if the browser refuses.
async fn start(shared: Weak<Shared>, bytes: Uint8Array, channels: u16) {
    if let Err(e) = try_start(&shared, bytes, channels).await {
        tracing::error!("rinch-av: the audio worklet did not start: {}", js_err(&e));
    }
}

async fn try_start(shared: &Weak<Shared>, bytes: Uint8Array, channels: u16) -> Result<(), JsValue> {
    let name = NEXT.with(|n| {
        let v = n.get();
        n.set(v + 1);
        format!("rinch-dsp-{v}")
    });
    let Some(ctx) = shared.upgrade().map(|s| s.ctx.clone()) else {
        return Ok(());
    };
    let source = PROCESSOR.replace("{NAME}", &name);
    let parts: Array = [JsValue::from_str(&source)].into_iter().collect();
    let bag = web_sys::BlobPropertyBag::new();
    bag.set_type("application/javascript");
    let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &bag)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)?;
    let loaded = JsFuture::from(ctx.audio_worklet()?.add_module(&url)?).await;
    let _ = web_sys::Url::revoke_object_url(&url);
    loaded?;
    let module = JsFuture::from(js_sys::WebAssembly::compile(&bytes)).await?;
    let Some(shared) = shared.upgrade() else {
        return Ok(());
    };

    let processor_options = Object::new();
    Reflect::set(&processor_options, &"module".into(), &module)?;
    Reflect::set(
        &processor_options,
        &"channels".into(),
        &JsValue::from(u32::from(channels)),
    )?;
    let options = AudioWorkletNodeOptions::new();
    options.set_number_of_inputs(0);
    options.set_number_of_outputs(1);
    options.set_output_channel_count(
        &[JsValue::from(u32::from(channels))]
            .into_iter()
            .collect::<Array>(),
    );
    options.set_processor_options(Some(&processor_options));
    let node = AudioWorkletNode::new_with_options(&shared.ctx, &name, &options)?;
    node.connect_with_audio_node(&shared.ctx.destination())?;

    let weak = Rc::downgrade(&shared);
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
        if let (Some(s), Ok(data)) = (weak.upgrade(), e.data().dyn_into::<Uint8Array>()) {
            s.replies.borrow_mut().push_back(data.to_vec());
        }
    });
    let port = node.port()?;
    port.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    for m in shared.pending.borrow_mut().drain(..) {
        port.post_message(&Uint8Array::from(m.as_slice()))?;
    }
    *shared.on_message.borrow_mut() = Some(on_message);
    *shared.node.borrow_mut() = Some(node);
    Ok(())
}
