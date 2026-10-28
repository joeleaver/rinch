//! Test processor for `open_dsp_output` on the web. Renders `level + 0.25 × channel` on each
//! channel (level starts at 0.5, so channels are distinct without any message). Messages:
//! `[0, n]` sets level to n/100 and replies `[0, n]`; `[1]` replies
//! `[1, quanta(u32 le), rate(u32 le), channels]`; `[2]` panics (a trapping processor).
//!
//! Rebuild `../dsp_echo.wasm` after changing this or `rinch-dsp`'s ABI:
//!     cargo build --release --target wasm32-unknown-unknown \
//!         --manifest-path crates/rinch-av/tests/fixtures/dsp-echo/Cargo.toml
//!     cp crates/rinch-av/tests/fixtures/dsp-echo/target/wasm32-unknown-unknown/release/dsp_echo.wasm \
//!         crates/rinch-av/tests/fixtures/
use rinch_dsp::{DspProcessor, channels_mut};

struct Echo {
    level: f32,
    quanta: u32,
    rate: u32,
    channels: u16,
    replies: Vec<Vec<u8>>,
}

impl DspProcessor for Echo {
    fn message(&mut self, b: &[u8]) {
        match b.first() {
            Some(0) => {
                self.level = f32::from(b[1]) / 100.0;
                self.replies.push(vec![0, b[1]]);
            }
            Some(1) => {
                let mut r = vec![1];
                r.extend_from_slice(&self.quanta.to_le_bytes());
                r.extend_from_slice(&self.rate.to_le_bytes());
                r.push(self.channels as u8);
                self.replies.push(r);
            }
            Some(2) => panic!("boom"),
            _ => {}
        }
    }
    fn render(&mut self, out: &mut [f32], channels: usize) {
        self.quanta += 1;
        for (c, ch) in channels_mut(out, channels).enumerate() {
            ch.fill(self.level + 0.25 * c as f32);
        }
    }
    fn reply(&mut self, out: &mut Vec<u8>) -> bool {
        if self.replies.is_empty() {
            return false;
        }
        out.extend_from_slice(&self.replies.remove(0));
        true
    }
}

rinch_dsp::export_dsp!(|rate: u32, channels: u16| Echo {
    level: 0.5,
    quanta: 0,
    rate,
    channels,
    replies: Vec::new(),
});
