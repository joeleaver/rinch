//! `open_dsp_output` on the web (#1963): the processor's raw module runs in an AudioWorklet.
//! The module is `tests/fixtures/dsp_echo.wasm`, built from `tests/fixtures/dsp-echo` with
//! `cargo build --release --target wasm32-unknown-unknown` (a `rinch_dsp::export_dsp!` cdylib).
//! Nothing is captured; the output plays a constant level (headless: no speaker).
#![cfg(target_arch = "wasm32")]

use rinch_av::audio::{DspOutput, DspOutputConfig, open_dsp_output};
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const ECHO: &[u8] = include_bytes!("fixtures/dsp_echo.wasm");

async fn sleep_ms(ms: i32) {
    let p = js_sys::Promise::new(&mut |resolve, _| {
        let _ = web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
    });
    let _ = JsFuture::from(p).await;
}

/// Polls `replies()` for up to `ms`, returning everything seen once `want` replies arrived.
async fn replies_within(out: &DspOutput, want: usize, ms: i32) -> Vec<Vec<u8>> {
    let mut all = Vec::new();
    let mut waited = 0;
    while all.len() < want && waited < ms {
        sleep_ms(20).await;
        waited += 20;
        all.extend(out.replies());
    }
    all
}

fn status(r: &[u8]) -> (u32, u32, u8) {
    assert_eq!(r[0], 1, "not a status reply: {r:?}");
    (
        u32::from_le_bytes(r[1..5].try_into().unwrap()),
        u32::from_le_bytes(r[5..9].try_into().unwrap()),
        r[9],
    )
}

fn open(rate: u32, channels: u16) -> DspOutput {
    let out = open_dsp_output(
        DspOutputConfig {
            sample_rate: rate,
            channels,
            buffer_size: 512,
        },
        |_, _| -> NoNative { unreachable!() },
        ECHO,
    )
    .expect("open_dsp_output");
    out.resume();
    out
}

/// `make` is never called on the web; a type to satisfy its bounds.
struct NoNative;
impl rinch_av::audio::DspProcessor for NoNative {
    fn message(&mut self, _: &[u8]) {}
    fn render(&mut self, _: &mut [&mut [f32]]) {}
}

#[wasm_bindgen_test]
async fn a_message_sent_before_load_arrives_first_and_replies_come_back_in_order() {
    let out = open(44_100, 2);
    // Sent synchronously after open: the worklet cannot be running yet.
    out.send(vec![0, 50]);
    out.send(vec![0, 25]);
    out.send(vec![1]);
    let r = replies_within(&out, 3, 3000).await;
    assert_eq!(r.len(), 3, "replies: {r:?}");
    assert_eq!(r[0], vec![0, 50]);
    assert_eq!(r[1], vec![0, 25]);
    let (_quanta, rate, channels) = status(&r[2]);
    assert_eq!(
        rate, 44_100,
        "the processor was built at the context's real rate"
    );
    assert_eq!(channels, 2);
    assert_eq!(out.config().sample_rate, 44_100);
}

#[wasm_bindgen_test]
async fn the_processor_renders_in_128_frame_quanta_in_real_time() {
    let out = open(48_000, 1);
    out.send(vec![1]);
    let r = replies_within(&out, 1, 3000).await;
    let (q0, _, ch) = status(&r[0]);
    assert_eq!(ch, 1);
    sleep_ms(1000).await;
    out.send(vec![1]);
    let r = replies_within(&out, 1, 3000).await;
    let (q1, _, _) = status(&r[0]);
    // 48 000 / 128 = 375 quanta a second; allow a loaded headless host its slack.
    let per_s = q1 - q0;
    assert!((250..=500).contains(&per_s), "{per_s} quanta in ~1 s");
}

#[wasm_bindgen_test]
async fn dropping_before_the_worklet_starts_is_quiet() {
    let out = open(48_000, 2);
    out.send(vec![0, 10]);
    drop(out);
    sleep_ms(500).await;
    // Reaching here without a panic is the assertion (`start` holds only a Weak).
}

/// FINDING (review #1964): a module the browser cannot compile is reported only to the log.
/// `open_dsp_output` returned `Ok`, and the output can never say it failed.
#[wasm_bindgen_test]
async fn finding_a_bad_module_fails_silently() {
    let out = open_dsp_output(
        DspOutputConfig::default(),
        |_, _| -> NoNative { unreachable!() },
        b"not wasm",
    );
    let out = out.expect("Ok today, though the module cannot load");
    out.send(vec![1]);
    let r = replies_within(&out, 1, 1500).await;
    assert!(r.is_empty());
}

/// FINDING (review #1964): a processor that traps (a Rust panic in `message`/`render`) kills the
/// worklet's `process()`; the node goes silent and nothing on the page hears of it
/// (no `processorerror` listener).
#[wasm_bindgen_test]
async fn finding_a_trapping_processor_goes_silent() {
    let out = open(48_000, 2);
    out.send(vec![1]);
    assert_eq!(replies_within(&out, 1, 3000).await.len(), 1);
    out.send(vec![2]); // panics inside the module
    sleep_ms(200).await;
    out.send(vec![1]);
    let r = replies_within(&out, 1, 1500).await;
    assert!(
        r.is_empty(),
        "the processor kept running after a trap: {r:?}"
    );
}
