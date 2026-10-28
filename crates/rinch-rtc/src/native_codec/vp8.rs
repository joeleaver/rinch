//! VP8 through the system's libvpx (the `media-vp8` feature; M2, amended
//! 2026-10-28: opt-in, so a default `media` build needs no libvpx).

use std::os::raw::{c_int, c_uint, c_ulong};

use super::{I420, VideoDecoder, VideoEncoder};

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

pub(super) struct Vp8Encoder {
    ctx: VpxCtx,
    cfg: vpx_sys::vpx_codec_enc_cfg_t,
    w: u32,
    h: u32,
    /// One frame's duration at the track's rate, in the 1 ms timebase:
    /// libvpx's rate control spends bitrate × duration on a frame.
    frame_ms: u64,
    /// Pictures given to libvpx so far: its pts is this count of frame
    /// periods, not the wall clock.
    frames: u64,
}

// SAFETY: `cfg` is plain configuration (its pointer fields are libvpx's
// unused two-pass buffers, null); the context is `Send` as VpxCtx says.
unsafe impl Send for Vp8Encoder {}

impl Vp8Encoder {
    pub(super) fn new(w: u32, h: u32, fps: u32, bitrate: u32) -> Result<Self, String> {
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
            let mut ctx = VpxCtx(ctx.assume_init());
            // Speed over quality for real time.
            let _ = vpx_sys::vpx_codec_control_(
                &mut ctx.0,
                vpx_sys::vp8e_enc_control_id::VP8E_SET_CPUUSED as c_int,
                8 as c_int,
            );
            // A keyframe may take at most 9 frames' worth of bits, WebRTC's
            // own setting (`MaxIntraTarget`: half a 600 ms buffer at 30
            // fps): a keyframe of a hard picture otherwise ate seconds of
            // budget.
            let _ = vpx_sys::vpx_codec_control_(
                &mut ctx.0,
                vpx_sys::vp8e_enc_control_id::VP8E_SET_MAX_INTRA_BITRATE_PCT as c_int,
                (30 * fps).max(300) as c_uint,
            );
            Ok(Self {
                ctx,
                cfg,
                w,
                h,
                frame_ms: u64::from((1000 / fps.max(1)).max(1)),
                frames: 0,
            })
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
            // One frame period, also after frames the pump's send budget
            // skipped: told the real gap (a second or more on a hard
            // picture), libvpx spent it all on the next frame (180 KB of
            // noise at 300 kbps). It was told 1 ms before #1947's second
            // review.
            let duration = self.frame_ms as c_ulong;
            // A synthetic pts, one frame period apart. Given the wall clock
            // (`ts_ms`), the gaps the pump's send budget leaves read to
            // libvpx as a falling frame rate, so it made each frame larger,
            // which the budget skipped more of: VP8 on camera-like content
            // fell to 4 fps at 300 kbps and never recovered after the cap
            // was lifted (#1947's third review). The RTP timestamps keep
            // the real time (the pump's `duration`).
            let _ = ts_ms;
            let pts = (self.frames * self.frame_ms) as i64;
            self.frames += 1;
            let flags: vpx_sys::vpx_enc_frame_flags_t = if key {
                vpx_sys::VPX_EFLAG_FORCE_KF as _
            } else {
                0
            };
            vpx_ok(
                vpx_sys::vpx_codec_encode(
                    &mut self.ctx.0,
                    &img,
                    pts,
                    duration,
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

    fn set_bitrate(&mut self, bps: u32) -> Result<(), String> {
        self.cfg.rc_target_bitrate = (bps / 1000).max(30);
        // SAFETY: a live context and a configuration libvpx filled.
        unsafe {
            vpx_ok(
                vpx_sys::vpx_codec_enc_config_set(&mut self.ctx.0, &self.cfg),
                "bitrate",
            )
        }
    }
}

pub(super) struct Vp8Decoder {
    ctx: VpxCtx,
}

impl Vp8Decoder {
    pub(super) fn new() -> Result<Self, String> {
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

/// I420 planes (with strides) to RGBA8, BT.601 limited range.
#[allow(clippy::too_many_arguments)]
fn i420_to_rgba(
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
