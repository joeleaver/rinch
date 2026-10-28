//! The desktop video codecs behind one encoder and one decoder trait:
//! H.264 through openh264, and with the `media-vp8` feature VP8 through the
//! system's libvpx (`vp8.rs`; M2, decided 2026-10-27 so desktop video works
//! with a Firefox that has no OpenH264 plugin, made opt-in 2026-10-28).
//! Pictures travel as I420 between them.

use std::os::raw::c_int;

#[cfg(feature = "media-vp8")]
mod vp8;

/// A negotiated video codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum VideoCodec {
    H264,
    /// Only with `media-vp8`: without it VP8 is neither offered nor
    /// accepted, and an SDP's `video/VP8` maps to no codec.
    #[cfg(feature = "media-vp8")]
    Vp8,
}

impl VideoCodec {
    /// Whether an encoded frame (VP8, or H.264 in Annex B) can be decoded
    /// on its own: a VP8 key frame, or an H.264 access unit with an IDR
    /// slice.
    pub(crate) fn is_keyframe(self, frame: &[u8]) -> bool {
        match self {
            // The frame tag's first bit: 0 for a key frame (RFC 6386 9.1).
            #[cfg(feature = "media-vp8")]
            Self::Vp8 => frame.first().is_some_and(|b| b & 1 == 0),
            Self::H264 => {
                let mut i = 0;
                while i + 3 < frame.len() {
                    if frame[i] == 0 && frame[i + 1] == 0 && frame[i + 2] == 1 {
                        if frame[i + 3] & 0x1f == 5 {
                            return true;
                        }
                        i += 3;
                    } else {
                        i += 1;
                    }
                }
                false
            }
        }
    }

    pub(crate) fn from_mime(mime: &str) -> Option<Self> {
        if mime.eq_ignore_ascii_case("video/H264") {
            return Some(Self::H264);
        }
        #[cfg(feature = "media-vp8")]
        if mime.eq_ignore_ascii_case("video/VP8") {
            return Some(Self::Vp8);
        }
        None
    }
}

/// A picture in I420, planes tightly packed (Y, then U, then V).
pub(crate) struct I420 {
    pub(crate) w: usize,
    pub(crate) h: usize,
    pub(crate) data: Vec<u8>,
}

impl I420 {
    /// From RGBA8 (even width and height).
    pub(crate) fn from_rgba(rgba: &[u8], w: usize, h: usize) -> Self {
        let yuv = openh264::formats::YUVBuffer::from_rgb_source(
            openh264::formats::RgbaSliceU8::new(rgba, (w, h)),
        );
        use openh264::formats::YUVSource;
        let mut data = Vec::with_capacity(w * h * 3 / 2);
        data.extend_from_slice(yuv.y());
        data.extend_from_slice(yuv.u());
        data.extend_from_slice(yuv.v());
        Self { w, h, data }
    }
}

pub(crate) trait VideoEncoder: Send {
    /// Encode one picture; `key` forces a keyframe. An empty result is a
    /// frame the rate control skipped.
    fn encode(&mut self, pic: &I420, ts_ms: u64, key: bool) -> Result<Vec<u8>, String>;
    /// A new target bitrate for the running encoder: no keyframe, no new
    /// encoder (a cap set mid-call must not cost a keyframe).
    fn set_bitrate(&mut self, bps: u32) -> Result<(), String>;
}

pub(crate) trait VideoDecoder: Send {
    /// Decode one access unit; RGBA into `out`, the size when a picture
    /// came out.
    fn decode(&mut self, data: &[u8], out: &mut Vec<u8>) -> Result<Option<(u32, u32)>, String>;
}

pub(crate) fn new_encoder(
    codec: VideoCodec,
    w: u32,
    h: u32,
    fps: u32,
    bitrate: u32,
) -> Result<Box<dyn VideoEncoder>, String> {
    // The picture size is VP8's to know up front; openh264 takes it from
    // each picture.
    #[cfg(not(feature = "media-vp8"))]
    let _ = (w, h);
    match codec {
        VideoCodec::H264 => H264Encoder::new(fps, bitrate).map(|e| Box::new(e) as Box<_>),
        #[cfg(feature = "media-vp8")]
        VideoCodec::Vp8 => vp8::Vp8Encoder::new(w, h, fps, bitrate).map(|e| Box::new(e) as Box<_>),
    }
}

pub(crate) fn new_decoder(codec: VideoCodec) -> Result<Box<dyn VideoDecoder>, String> {
    match codec {
        VideoCodec::H264 => H264Decoder::new().map(|d| Box::new(d) as Box<_>),
        #[cfg(feature = "media-vp8")]
        VideoCodec::Vp8 => vp8::Vp8Decoder::new().map(|d| Box::new(d) as Box<_>),
    }
}

// ================================================================== H.264

struct H264Encoder {
    enc: openh264::encoder::Encoder,
    fps: u32,
    /// openh264 initialises itself on the first picture; options can be
    /// set only after that.
    started: bool,
}

/// openh264's `SBitrateInfo` (`codec_app_def.h`), for
/// `ENCODER_OPTION_BITRATE`.
#[repr(C)]
struct SBitrateInfo {
    /// `LAYER_NUM`: `SPATIAL_LAYER_ALL` is 4.
    layer: c_int,
    bitrate: c_int,
}

/// `ENCODER_OPTION_BITRATE` in openh264's `ENCODER_OPTION` enum.
const ENCODER_OPTION_BITRATE: c_int = 5;

impl H264Encoder {
    fn new(fps: u32, bitrate: u32) -> Result<Self, String> {
        Ok(Self {
            enc: Self::make(fps, bitrate)?,
            fps,
            started: false,
        })
    }

    fn make(fps: u32, bitrate: u32) -> Result<openh264::encoder::Encoder, String> {
        let api = crate::native_media::h264_api().map_err(|e| e.to_string())?;
        // Bitrate mode with frame skipping: without skipping, openh264
        // cannot hold a bitrate (it warns so), and full-frame noise ran at
        // 17 Mbps under a 300 kbps cap.
        let config = openh264::encoder::EncoderConfig::new()
            .bitrate(openh264::encoder::BitRate::from_bps(bitrate))
            .max_frame_rate(openh264::encoder::FrameRate::from_hz(fps as f32))
            .usage_type(openh264::encoder::UsageType::CameraVideoRealTime)
            .rate_control_mode(openh264::encoder::RateControlMode::Bitrate)
            .profile(openh264::encoder::Profile::Baseline)
            .skip_frames(true);
        openh264::encoder::Encoder::with_api_config(api, config).map_err(|e| e.to_string())
    }
}

impl VideoEncoder for H264Encoder {
    fn encode(&mut self, pic: &I420, ts_ms: u64, key: bool) -> Result<Vec<u8>, String> {
        let yuv = openh264::formats::YUVBuffer::from_vec(pic.data.clone(), pic.w, pic.h);
        if key {
            self.enc.force_intra_frame();
        }
        let out = self
            .enc
            .encode_at(&yuv, openh264::Timestamp::from_millis(ts_ms))
            .map(|b| b.to_vec())
            .map_err(|e| e.to_string())?;
        self.started = true;
        Ok(out)
    }

    fn set_bitrate(&mut self, bps: u32) -> Result<(), String> {
        if !self.started {
            self.enc = Self::make(self.fps, bps)?;
            return Ok(());
        }
        let mut info = SBitrateInfo {
            layer: 4,
            bitrate: bps.min(i32::MAX as u32) as c_int,
        };
        // SAFETY: the encoder is initialised (a picture went through it),
        // and the option takes a pointer to an SBitrateInfo, which `info`
        // is laid out as; openh264 reads it during the call.
        let r = unsafe {
            self.enc
                .raw_api()
                .set_option(ENCODER_OPTION_BITRATE, (&raw mut info).cast())
        };
        if r == 0 {
            Ok(())
        } else {
            Err(format!("openh264: set bitrate: {r}"))
        }
    }
}

struct H264Decoder(openh264::decoder::Decoder);

impl H264Decoder {
    fn new() -> Result<Self, String> {
        let api = crate::native_media::h264_api().map_err(|e| e.to_string())?;
        openh264::decoder::Decoder::with_api_config(api, openh264::decoder::DecoderConfig::new())
            .map(Self)
            .map_err(|e| e.to_string())
    }
}

impl VideoDecoder for H264Decoder {
    fn decode(&mut self, data: &[u8], out: &mut Vec<u8>) -> Result<Option<(u32, u32)>, String> {
        match self.0.decode(data) {
            Ok(Some(yuv)) => {
                use openh264::formats::YUVSource;
                let (w, h) = yuv.dimensions();
                out.resize(w * h * 4, 0);
                yuv.write_rgba8(out);
                Ok(Some((w as u32, h as u32)))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box_frame(w: usize, h: usize, x: usize) -> Vec<u8> {
        let mut f = vec![0u8; w * h * 4];
        for (i, px) in f.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (cx, cy) = (i % w, i / w);
            let on = (x..x + 32).contains(&cx) && (16..48).contains(&cy);
            px.copy_from_slice(if on {
                &[255, 255, 255, 255]
            } else {
                &[0, 0, 0, 255]
            });
        }
        f
    }

    fn round_trip(codec: VideoCodec) {
        let (w, h) = (160usize, 96usize);
        let mut enc = new_encoder(codec, w as u32, h as u32, 30, 300_000).unwrap();
        let mut dec = new_decoder(codec).unwrap();
        let mut out = Vec::new();
        let mut shown = 0;
        for k in 0..30u64 {
            let pic = I420::from_rgba(&box_frame(w, h, (k as usize * 3) % 120), w, h);
            let bits = enc.encode(&pic, k * 33, k == 0).unwrap();
            if bits.is_empty() {
                continue;
            }
            if let Some((dw, dh)) = dec.decode(&bits, &mut out).unwrap() {
                assert_eq!((dw, dh), (w as u32, h as u32));
                // The box's middle is bright, a far corner dark.
                let x = (k as usize * 3) % 120 + 16;
                let mid = (32 * w + x) * 4;
                assert!(
                    out[mid] > 180,
                    "{codec:?} frame {k}: box at {x}: {}",
                    out[mid]
                );
                assert!(out[((h - 2) * w + w - 2) * 4] < 60);
                shown += 1;
            }
        }
        assert!(shown >= 25, "{codec:?}: {shown} of 30 frames decoded");
    }

    #[cfg(feature = "media-vp8")]
    #[test]
    fn vp8_round_trips_a_moving_box() {
        round_trip(VideoCodec::Vp8);
    }

    #[test]
    fn h264_round_trips_a_moving_box() {
        round_trip(VideoCodec::H264);
    }

    /// A busy but compressible picture: a gradient, a textured square
    /// moving across it, mild noise.
    fn medium(w: usize, h: usize, k: usize, seed: &mut u32) -> Vec<u8> {
        let mut f = vec![0u8; w * h * 4];
        for (i, px) in f.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (x, y) = (i % w, i / w);
            *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let n = ((*seed >> 24) as i32 & 15) - 8;
            let sq = (x + 100_000 - k * 5) % 200 < 100 && (y + 100_000 - k * 3) % 160 < 80;
            let t = if sq {
                (((x ^ y) & 8) * 28) as i32
            } else {
                ((x * 255 / w + y * 128 / h) / 2) as i32
            };
            let v = (t + n).clamp(0, 255) as u8;
            *px = [v, v / 2 + 60, 255 - v, 255];
        }
        f
    }

    /// `set_bitrate` reaches a running encoder: its output falls to the new
    /// rate without a new encoder (a cap set mid-call costs no keyframe).
    fn takes_a_new_bitrate(codec: VideoCodec) {
        let (w, h) = (640usize, 480usize);
        let mut enc = new_encoder(codec, w as u32, h as u32, 30, 1_500_000).unwrap();
        let mut seed = 1u32;
        let bytes = |enc: &mut Box<dyn VideoEncoder>, from: usize, n: usize, seed: &mut u32| {
            (from..from + n)
                .map(|k| {
                    let pic = I420::from_rgba(&medium(w, h, k, seed), w, h);
                    enc.encode(&pic, k as u64 * 33, k == 0).unwrap().len()
                })
                .sum::<usize>()
        };
        let high = bytes(&mut enc, 0, 90, &mut seed) * 8 / 3;
        enc.set_bitrate(200_000).unwrap();
        bytes(&mut enc, 90, 30, &mut seed);
        let low = bytes(&mut enc, 120, 90, &mut seed) * 8 / 3;
        assert!(
            low < 300_000 && low * 2 < high,
            "{codec:?}: {high} bps at 1.5 Mbps, then {low} bps at 200 kbps"
        );
    }

    #[cfg(feature = "media-vp8")]
    #[test]
    fn a_running_vp8_encoder_takes_a_new_bitrate() {
        takes_a_new_bitrate(VideoCodec::Vp8);
    }

    #[test]
    fn a_running_h264_encoder_takes_a_new_bitrate() {
        takes_a_new_bitrate(VideoCodec::H264);
    }
}
