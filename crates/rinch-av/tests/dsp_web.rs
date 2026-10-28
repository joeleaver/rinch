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
            ..Default::default()
        },
        // `make` is never called on the web, and the processor type has no bounds there.
        |_, _| (),
        ECHO,
    )
    .expect("open_dsp_output");
    out.resume();
    out
}

#[wasm_bindgen_test]
async fn a_message_sent_before_load_arrives_first_and_replies_come_back_in_order() {
    let out = open(44_100, 2);
    // Sent synchronously after open: the worklet cannot be running yet.
    assert!(out.send(&[0, 50]));
    assert!(out.send(&[0, 25]));
    assert!(out.send(&[1]));
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
    assert!(out.send(&[1]));
    let r = replies_within(&out, 1, 3000).await;
    let (q0, _, ch) = status(&r[0]);
    assert_eq!(ch, 1);
    sleep_ms(1000).await;
    assert!(out.send(&[1]));
    let r = replies_within(&out, 1, 3000).await;
    let (q1, _, _) = status(&r[0]);
    // 48 000 / 128 = 375 quanta a second; allow a loaded headless host its slack.
    let per_s = q1 - q0;
    assert!((250..=500).contains(&per_s), "{per_s} quanta in ~1 s");
}

#[wasm_bindgen_test]
async fn dropping_before_the_worklet_starts_is_quiet() {
    let out = open(48_000, 2);
    assert!(out.send(&[0, 10]));
    drop(out);
    sleep_ms(500).await;
    // Reaching here without a panic is the assertion (`start` holds only a Weak).
}

async fn error_within(out: &DspOutput, ms: i32) -> Option<String> {
    let mut waited = 0;
    while out.error().is_none() && waited < ms {
        sleep_ms(20).await;
        waited += 20;
    }
    out.error()
}

fn open_module(module: &[u8]) -> DspOutput {
    let out = open_dsp_output(DspOutputConfig::default(), |_, _| (), module)
        .expect("open_dsp_output starts loading");
    out.resume();
    out
}

/// A module the browser cannot compile is the output's error, not just a log line (the review's
/// `finding_a_bad_module_fails_silently` saw `Ok` and silence at the PR head). Sending is then
/// refused.
#[wasm_bindgen_test]
async fn a_bad_module_is_reported() {
    let out = open_module(b"not wasm");
    let err = error_within(&out, 3000).await.expect("an error");
    assert!(err.contains("did not start"), "{err}");
    assert!(!out.send(&[1]), "a failed output refuses messages");
    assert!(out.replies().is_empty());
}

/// A module without the ABI's exports (here: none at all) is refused, naming what is missing.
#[wasm_bindgen_test]
async fn a_module_without_the_abi_is_refused() {
    let out = open_module(b"\0asm\x01\0\0\0");
    let err = error_within(&out, 3000).await.expect("an error");
    assert!(err.contains("rinch_dsp_abi"), "{err}");
}

/// A module built for another ABI version is refused by the worklet, which says which.
#[wasm_bindgen_test]
async fn a_module_of_another_abi_version_is_refused() {
    let out = open_module(&other_abi_module(99));
    let err = error_within(&out, 3000).await.expect("an error");
    assert!(err.contains("ABI 99"), "{err}");
    // Positive control: the same module claiming this ABI passes the version check and runs
    // (silently: its init does nothing and its render points at zeroed memory), so no error.
    let ok = open_module(&other_abi_module(rinch_dsp::ABI_VERSION));
    assert!(error_within(&ok, 1000).await.is_none(), "{:?}", ok.error());
}

/// A minimal module with every export the host calls: `rinch_dsp_abi` returns `abi`, `reply`
/// returns -1 (none), the rest do nothing or return 0. Hand-assembled.
fn other_abi_module(abi: u32) -> Vec<u8> {
    fn leb_i32(mut v: i32, out: &mut Vec<u8>) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if (v == 0 && b & 0x40 == 0) || (v == -1 && b & 0x40 != 0) {
                out.push(b);
                return;
            }
            out.push(b | 0x80);
        }
    }
    fn section(id: u8, body: Vec<u8>, out: &mut Vec<u8>) {
        out.push(id);
        out.push(body.len() as u8); // every section here is under 128 bytes
        out.extend(body);
    }
    let mut m = b"\0asm\x01\0\0\0".to_vec();
    // Types: 0 () -> i32, 1 (f32, i32) -> (), 2 (i32) -> i32, 3 (i32) -> ().
    section(
        1,
        vec![
            4, 0x60, 0, 1, 0x7f, 0x60, 2, 0x7d, 0x7f, 0, 0x60, 1, 0x7f, 1, 0x7f, 0x60, 1, 0x7f, 0,
        ],
        &mut m,
    );
    // Functions: abi, init, alloc, message, render, reply, reply_ptr.
    section(3, vec![7, 0, 1, 2, 3, 2, 0, 0], &mut m);
    // One page of memory.
    section(5, vec![1, 0, 1], &mut m);
    let names = [
        "rinch_dsp_abi",
        "rinch_dsp_init",
        "rinch_dsp_alloc",
        "rinch_dsp_message",
        "rinch_dsp_render",
        "rinch_dsp_reply",
        "rinch_dsp_reply_ptr",
    ];
    let mut exports = vec![8];
    for (i, n) in names.iter().enumerate() {
        exports.push(n.len() as u8);
        exports.extend(n.as_bytes());
        exports.extend([0, i as u8]);
    }
    exports.push(6);
    exports.extend(b"memory");
    exports.extend([2, 0]);
    let mut e = vec![];
    // The export section is longer than 127 bytes: write its size as two LEB bytes.
    e.push(7);
    let len = exports.len();
    e.extend([(len as u8 & 0x7f) | 0x80, (len >> 7) as u8]);
    e.extend(exports);
    m.extend(e);
    let body = |consts: Option<i32>| {
        let mut b = vec![0];
        if let Some(c) = consts {
            b.push(0x41);
            leb_i32(c, &mut b);
        }
        b.push(0x0b);
        b
    };
    let bodies = [
        body(Some(abi as i32)),
        body(None),
        body(Some(0)),
        body(None),
        body(Some(0)),
        body(Some(-1)),
        body(Some(0)),
    ];
    let mut code = vec![7];
    for b in bodies {
        code.push(b.len() as u8);
        code.extend(b);
    }
    section(10, code, &mut m);
    m
}

/// A processor that traps (a Rust panic: `unreachable` in the module) is the output's error, and
/// the output stops (the review's `finding_a_trapping_processor_goes_silent` saw only silence at
/// the PR head).
#[wasm_bindgen_test]
async fn a_trapping_processor_is_reported() {
    let out = open(48_000, 2);
    assert!(out.send(&[1]));
    assert_eq!(replies_within(&out, 1, 3000).await.len(), 1);
    assert!(out.error().is_none());
    assert!(out.send(&[2])); // panics inside the module
    let err = error_within(&out, 3000)
        .await
        .expect("the trap is reported");
    assert!(err.contains("RuntimeError"), "{err}");
    assert!(!out.send(&[1]), "a failed output refuses messages");
    assert!(replies_within(&out, 1, 500).await.is_empty());
}

/// Messages past `message_capacity` are refused while the worklet cannot take them (here: before
/// it has loaded, so nothing is applied yet), and replies past `reply_capacity` are dropped and
/// counted.
#[wasm_bindgen_test]
async fn the_queues_are_bounded() {
    let out = open_dsp_output(
        DspOutputConfig {
            // Two 6-byte messages fit, a third does not.
            message_capacity: 12,
            // One 6-byte reply fits ([0, n] + header), a second does not.
            reply_capacity: 8,
            ..Default::default()
        },
        |_, _| (),
        ECHO,
    )
    .unwrap();
    out.resume();
    assert!(out.send(&[0, 10]));
    assert!(out.send(&[0, 20]));
    assert!(!out.send(&[0, 30]), "the message budget is spent");
    // Once applied, the budget comes back and the replies arrive: only the first fits.
    let mut waited = 0;
    while out.dropped_replies() == 0 && waited < 3000 {
        sleep_ms(20).await;
        waited += 20;
    }
    assert_eq!(out.dropped_replies(), 1);
    assert_eq!(out.replies(), vec![vec![0, 10]]);
    let mut waited = 0;
    while !out.send(&[0, 40]) {
        assert!(waited < 3000, "the budget never came back");
        sleep_ms(20).await;
        waited += 20;
    }
    assert_eq!(replies_within(&out, 1, 3000).await, vec![vec![0, 40]]);
}
