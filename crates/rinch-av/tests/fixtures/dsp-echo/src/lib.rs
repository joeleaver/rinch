//! Test processor: message [0, level] sets the level (level/100) rendered on every channel and
//! replies [0, level]; message [1] replies [1, quanta(u32 le), rate(u32 le), channels]; message [2]
//! panics (a trapping processor).
use rinch_dsp::DspProcessor;
struct Echo { level: f32, quanta: u32, rate: u32, channels: u16, replies: Vec<Vec<u8>> }
impl DspProcessor for Echo {
    fn message(&mut self, b: &[u8]) {
        match b.first() {
            Some(0) => { self.level = f32::from(b[1]) / 100.0; self.replies.push(vec![0, b[1]]); }
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
    fn render(&mut self, out: &mut [&mut [f32]]) {
        self.quanta += 1;
        for ch in out.iter_mut() { ch.fill(self.level); }
    }
    fn reply(&mut self) -> Option<Vec<u8>> {
        if self.replies.is_empty() { None } else { Some(self.replies.remove(0)) }
    }
}
rinch_dsp::export_dsp!(|rate: u32, channels: u16| Echo { level: 0.0, quanta: 0, rate, channels, replies: Vec::new() });
