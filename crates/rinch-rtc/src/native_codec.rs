//! The desktop video codecs behind one encoder and one decoder trait:
//! H.264 through openh264, VP8 through the system's libvpx (M2, decided
//! 2026-10-27: VP8 so desktop video works with a Firefox that has no
//! OpenH264 plugin). Pictures travel as I420 between them.

use std::os::raw::{c_int, c_ulong};

/// A negotiated video codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum VideoCodec {
    H264,
    Vp8,
}

impl VideoCodec {
    pub(crate) fn from_mime(mime: &str) -> Option<Self> {
        if mime.eq_ignore_ascii_case("video/H264") {
            Some(Self::H264)
        } else if mime.eq_ignore_ascii_case("video/VP8") {
            Some(Self::Vp8)
        } else {
            None
        }
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

/// I420 planes (with strides) to RGBA8, BT.601 limited range.
#[allow(clippy::too_many_arguments)]
pub(crate) fn i420_to_rgba(
    y: &[u8],
    u: &[u8],
    v: &[u8],
    sy: usize,
    su: usize,
    sv: usize,
    w: usize,
    h: usize,
    out: &mut Vec<u8>,
) {
    out.resize(w * h * 4, 0);
    for row in 0..h {
        for col in 0..w {
            let yy = i32::from(y[row * sy + col]) - 16;
            let uu = i32::from(u[(row / 2) * su + col / 2]) - 128;
            let vv = i32::from(v[(row / 2) * sv + col / 2]) - 128;
            let c = 298 * yy;
            let r = (c + 409 * vv + 128) >> 8;
            let g = (c - 100 * uu - 208 * vv + 128) >> 8;
            let b = (c + 516 * uu + 128) >> 8;
            let i = (row * w + col) * 4;
            out[i] = r.clamp(0, 255) as u8;
            out[i + 1] = g.clamp(0, 255) as u8;
            out[i + 2] = b.clamp(0, 255) as u8;
            out[i + 3] = 255;
        }
    }
}

pub(crate) trait VideoEncoder: Send {
    /// Encode one picture; `key` forces a keyframe. An empty result is a
    /// frame the rate control skipped.
    fn encode(&mut self, pic: &I420, ts_ms: u64, key: bool) -> Result<Vec<u8>, String>;
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
    match codec {
        VideoCodec::H264 => H264Encoder::new(fps, bitrate).map(|e| Box::new(e) as Box<_>),
        VideoCodec::Vp8 => Vp8Encoder::new(w, h, fps, bitrate).map(|e| Box::new(e) as Box<_>),
    }
}

pub(crate) fn new_decoder(codec: VideoCodec) -> Result<Box<dyn VideoDecoder>, String> {
    match codec {
        VideoCodec::H264 => H264Decoder::new().map(|d| Box::new(d) as Box<_>),
        VideoCodec::Vp8 => Vp8Decoder::new().map(|d| Box::new(d) as Box<_>),
    }
}

// ================================================================== H.264

struct H264Encoder(openh264::encoder::Encoder);

impl H264Encoder {
    fn new(fps: u32, bitrate: u32) -> Result<Self, String> {
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
        openh264::encoder::Encoder::with_api_config(api, config)
            .map(Self)
            .map_err(|e| e.to_string())
    }
}

impl VideoEncoder for H264Encoder {
    fn encode(&mut self, pic: &I420, ts_ms: u64, key: bool) -> Result<Vec<u8>, String> {
        let yuv = openh264::formats::YUVBuffer::from_vec(pic.data.clone(), pic.w, pic.h);
        if key {
            self.0.force_intra_frame();
        }
        self.0
            .encode_at(&yuv, openh264::Timestamp::from_millis(ts_ms))
            .map(|b| b.to_vec())
            .map_err(|e| e.to_string())
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

// ==================================================================== VP8

/// A libvpx context; destroyed on drop.
struct VpxCtx(vpx_sys::vpx_codec_ctx_t);

// SAFETY: the context is used from one thread at a time (it moves with the
// encoder or decoder that owns it) and libvpx keeps no thread affinity.
unsafe impl Send for VpxCtx {}

impl Drop for VpxCtx {
    fn drop(&mut self) {
        // SAFETY: initialised by vpx_codec_{enc,dec}_init_ver before a
        // VpxCtx is made; destroyed once.
        unsafe {
            vpx_sys::vpx_codec_destroy(&mut self.0);
        }
    }
}

fn vpx_ok(e: vpx_sys::vpx_codec_err_t, what: &str) -> Result<(), String> {
    if e == vpx_sys::vpx_codec_err_t::VPX_CODEC_OK {
        Ok(())
    } else {
        Err(format!("libvpx {what}: {e:?}"))
    }
}

struct Vp8Encoder {
    ctx: VpxCtx,
    w: u32,
    h: u32,
}

impl Vp8Encoder {
    fn new(w: u32, h: u32, fps: u32, bitrate: u32) -> Result<Self, String> {
        // SAFETY: plain libvpx calls on zero-initialised C structs, each
        // result checked; the interface pointer is a static.
        unsafe {
            let iface = vpx_sys::vpx_codec_vp8_cx();
            if iface.is_null() {
                return Err("libvpx has no VP8 encoder".into());
            }
            // Filled by libvpx: zeroed is not a valid value of its enum
            // fields (bindgen makes them Rust enums).
            let mut cfg = std::mem::MaybeUninit::<vpx_sys::vpx_codec_enc_cfg_t>::uninit();
            vpx_ok(
                vpx_sys::vpx_codec_enc_config_default(iface, cfg.as_mut_ptr(), 0),
                "config",
            )?;
            let mut cfg = cfg.assume_init();
            cfg.g_w = w;
            cfg.g_h = h;
            cfg.g_timebase.num = 1;
            cfg.g_timebase.den = 1000;
            cfg.rc_target_bitrate = (bitrate / 1000).max(30);
            cfg.rc_end_usage = vpx_sys::vpx_rc_mode::VPX_CBR;
            cfg.rc_dropframe_thresh = 25;
            cfg.rc_min_quantizer = 2;
            cfg.rc_max_quantizer = 56;
            cfg.rc_undershoot_pct = 100;
            cfg.rc_overshoot_pct = 15;
            cfg.rc_buf_sz = 1000;
            cfg.rc_buf_initial_sz = 500;
            cfg.rc_buf_optimal_sz = 600;
            cfg.g_lag_in_frames = 0;
            cfg.g_error_resilient = vpx_sys::VPX_ERROR_RESILIENT_DEFAULT;
            cfg.g_threads = 2;
            // Keyframes on request (and the sender's periodic one), not
            // by libvpx's own schedule.
            cfg.kf_mode = vpx_sys::vpx_kf_mode::VPX_KF_DISABLED;
            let _ = fps;
            let mut ctx = std::mem::MaybeUninit::<vpx_sys::vpx_codec_ctx_t>::uninit();
            vpx_ok(
                vpx_sys::vpx_codec_enc_init_ver(
                    ctx.as_mut_ptr(),
                    iface,
                    &cfg,
                    0,
                    vpx_sys::VPX_ENCODER_ABI_VERSION as c_int,
                ),
                "encoder init",
            )?;
            let ctx = VpxCtx(ctx.assume_init());
            let mut ctx = ctx;
            // Speed over quality for real time.
            let _ = vpx_sys::vpx_codec_control_(
                &mut ctx.0,
                vpx_sys::vp8e_enc_control_id::VP8E_SET_CPUUSED as c_int,
                8 as c_int,
            );
            Ok(Self { ctx, w, h })
        }
    }
}

impl VideoEncoder for Vp8Encoder {
    fn encode(&mut self, pic: &I420, ts_ms: u64, key: bool) -> Result<Vec<u8>, String> {
        if pic.w as u32 != self.w || pic.h as u32 != self.h {
            return Err("VP8: the picture changed size".into());
        }
        // SAFETY: the image wraps `pic.data`, which outlives the encode
        // call; libvpx copies what it keeps. Output packets are read before
        // the next call on the context.
        unsafe {
            let mut img = std::mem::MaybeUninit::<vpx_sys::vpx_image_t>::uninit();
            let wrapped = vpx_sys::vpx_img_wrap(
                img.as_mut_ptr(),
                vpx_sys::vpx_img_fmt::VPX_IMG_FMT_I420,
                self.w,
                self.h,
                1,
                pic.data.as_ptr() as *mut _,
            );
            if wrapped.is_null() {
                return Err("VP8: could not wrap the picture".into());
            }
            let img = img.assume_init();
            let flags: vpx_sys::vpx_enc_frame_flags_t = if key {
                vpx_sys::VPX_EFLAG_FORCE_KF as _
            } else {
                0
            };
            vpx_ok(
                vpx_sys::vpx_codec_encode(
                    &mut self.ctx.0,
                    &img,
                    ts_ms as i64,
                    1,
                    flags,
                    vpx_sys::VPX_DL_REALTIME as c_ulong,
                ),
                "encode",
            )?;
            let mut out = Vec::new();
            let mut iter: vpx_sys::vpx_codec_iter_t = std::ptr::null();
            loop {
                let pkt = vpx_sys::vpx_codec_get_cx_data(&mut self.ctx.0, &mut iter);
                if pkt.is_null() {
                    break;
                }
                if (*pkt).kind == vpx_sys::vpx_codec_cx_pkt_kind::VPX_CODEC_CX_FRAME_PKT {
                    let f = (*pkt).data.frame;
                    out.extend_from_slice(std::slice::from_raw_parts(f.buf as *const u8, f.sz));
                }
            }
            Ok(out)
        }
    }
}

struct Vp8Decoder {
    ctx: VpxCtx,
}

impl Vp8Decoder {
    fn new() -> Result<Self, String> {
        // SAFETY: as for the encoder.
        unsafe {
            let iface = vpx_sys::vpx_codec_vp8_dx();
            if iface.is_null() {
                return Err("libvpx has no VP8 decoder".into());
            }
            let mut ctx = std::mem::MaybeUninit::<vpx_sys::vpx_codec_ctx_t>::uninit();
            vpx_ok(
                vpx_sys::vpx_codec_dec_init_ver(
                    ctx.as_mut_ptr(),
                    iface,
                    std::ptr::null(),
                    0,
                    vpx_sys::VPX_DECODER_ABI_VERSION as c_int,
                ),
                "decoder init",
            )?;
            Ok(Self {
                ctx: VpxCtx(ctx.assume_init()),
            })
        }
    }
}

impl VideoDecoder for Vp8Decoder {
    fn decode(&mut self, data: &[u8], out: &mut Vec<u8>) -> Result<Option<(u32, u32)>, String> {
        // SAFETY: `data` lives across the call; the image libvpx returns is
        // read before the next decode on this context.
        unsafe {
            vpx_ok(
                vpx_sys::vpx_codec_decode(
                    &mut self.ctx.0,
                    data.as_ptr(),
                    data.len() as _,
                    std::ptr::null_mut(),
                    0,
                ),
                "decode",
            )?;
            let mut iter: vpx_sys::vpx_codec_iter_t = std::ptr::null();
            let mut size = None;
            loop {
                let img = vpx_sys::vpx_codec_get_frame(&mut self.ctx.0, &mut iter);
                if img.is_null() {
                    break;
                }
                let img = &*img;
                let (w, h) = (img.d_w as usize, img.d_h as usize);
                let (sy, su, sv) = (
                    img.stride[0] as usize,
                    img.stride[1] as usize,
                    img.stride[2] as usize,
                );
                let y = std::slice::from_raw_parts(img.planes[0], sy * h);
                let u = std::slice::from_raw_parts(img.planes[1], su * h.div_ceil(2));
                let v = std::slice::from_raw_parts(img.planes[2], sv * h.div_ceil(2));
                i420_to_rgba(y, u, v, sy, su, sv, w, h, out);
                size = Some((w as u32, h as u32));
            }
            Ok(size)
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

    #[test]
    fn vp8_round_trips_a_moving_box() {
        round_trip(VideoCodec::Vp8);
    }

    #[test]
    fn h264_round_trips_a_moving_box() {
        round_trip(VideoCodec::H264);
    }
}
