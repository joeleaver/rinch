//! The software presenter: frames written straight into the `ANativeWindow`.

use crate::shell::android_frame;

/// The screen, and the one place a finished frame becomes pixels on it.
///
/// This writes straight into the buffer `ANativeWindow_lock` hands back. It
/// used to go through `softbuffer`, which is the right abstraction on the
/// desktop — one API over X11, Wayland, Win32 and the rest — and the wrong one
/// here, because Android is the platform where softbuffer cannot map the
/// window's memory into the caller's hands. Its Android backend keeps a shadow
/// `Vec<Pixel>` the size of the screen, allocates and zeroes a fresh one on
/// every `next_buffer()`, lets the caller fill *that*, and then copies it a
/// byte at a time into the locked window buffer on `present()`. Every frame
/// therefore paid for a 10.6 MB allocation and two full-screen copies where
/// one would do.
///
/// Card K27 measured the three phases on a moto g stylus 5G at 1080x2460,
/// with a temporary `log::info!` probe around each — the technique K24 used —
/// before and after this rewrite:
///
/// ```text
///                     acquire   fill   swap    total
/// through softbuffer      4.1    2.9    5.2     12.2
/// into the window         0.5    2.0    0.6      3.1
/// ```
///
/// `acquire` is the allocate-and-zero, `fill` is this function's own copy, and
/// `swap` is handing the buffer to the compositor. So only about a quarter of
/// the ~12ms was the copy this file controls; the rest was the shadow buffer
/// existing at all. That mattered more than it sounds: on a
/// screen that is scrolling with momentum the painter returns its cached
/// pixmap untouched and nothing is redrawn, so those 12ms *were* the entire
/// frame — 298 of the 318 frames in K27's library-scroll trace repainted
/// nothing and cost 13.3ms each anyway. After the rewrite the same trace's
/// unchanged frames cost 6.4ms, and the loop presents about 30% more of them
/// in the same wall-clock window — the residue is `ANativeWindow_lock`
/// blocking for a free buffer, which is the display's back-pressure and not
/// work.
///
/// Writing into the window directly gives up nothing here. The one thing a
/// shadow buffer buys is the ability to leave pixels alone between frames,
/// and this shell never does: `build_pixels` hands back a complete frame every
/// time, so every pixel of the window is written every time regardless of what
/// the swap chain had in it before.
#[cfg(not(feature = "android-gpu"))]
pub(super) struct SoftSurface {
    native: ndk::native_window::NativeWindow,
    width: u32,
    height: u32,
    /// Keeps a permanent window-format fault from becoming ~120 `error` lines
    /// a second now that the loop retries it. See
    /// [`android_frame::PresentFaultLog`].
    faults: android_frame::PresentFaultLog,
}

#[cfg(not(feature = "android-gpu"))]
impl SoftSurface {
    /// Infallible: nothing here can fail in a way that leaves the shell better
    /// off skipping the frame. A refused `set_buffers_geometry` is logged and
    /// survived — `present_pixels` re-checks the format of the buffer it is
    /// actually handed, which is the authoritative answer anyway.
    pub(super) fn new(window: &ndk::native_window::NativeWindow, width: u32, height: u32) -> Self {
        let surface = Self {
            native: window.clone(),
            width: width.max(1),
            height: height.max(1),
            faults: android_frame::PresentFaultLog::default(),
        };
        surface.configure();
        surface
    }

    /// Ask the window for buffers of the size and format this shell paints.
    ///
    /// `R8G8B8X8_UNORM` and not `R8G8B8A8_UNORM`: the frame is opaque, the
    /// fourth byte is never read, and asking for an alpha channel would invite
    /// the compositor to blend a surface that has nothing behind it.
    fn configure(&self) {
        use ndk::hardware_buffer_format::HardwareBufferFormat;
        if let Err(e) = self.native.set_buffers_geometry(
            self.width as i32,
            self.height as i32,
            Some(HardwareBufferFormat::R8G8B8X8_UNORM),
        ) {
            log::error!("present: set_buffers_geometry failed: {e}");
        }
    }

    pub(super) fn resize(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.configure();
    }

    /// Returns whether the frame actually reached the window.
    ///
    /// The loop paces itself on that answer (see
    /// `android_frame::poll_timeout`), and the two early returns below are the
    /// reason it is a `bool` rather than the loop assuming a paint attempt is
    /// a paint: `lock` blocks for a free buffer, which is the display's own
    /// back-pressure and the thing that makes a busy loop impossible — but
    /// when it *fails* it returns at once, and a loop that treated that as a
    /// presented frame would come straight back and fail again as fast as the
    /// CPU allowed.
    ///
    /// Since issue #563 the loop *does* come back for a failed frame — it was
    /// owed one and nothing else would have asked — but it arms
    /// `REDRAW_PENDING` **and restarts its frame clock** to do it, so the retry
    /// costs a whole display frame of sleep. Both halves are needed: without
    /// the clock reset a paint that already overran the frame leaves
    /// `frame_interval - spent` saturated at zero and the retry is immediate,
    /// which is the spin this `bool` exists to prevent, arriving by a different
    /// road. The distinction it draws is untouched and is still the one that
    /// matters: retrying on the panel's clock rather than on the CPU's. See
    /// `android_frame::retry_owed_frame`.
    ///
    /// The `error` below is reported once per fault rather than once per
    /// attempt, because the retry means there are now ~120 attempts a second
    /// and this particular fault — a window whose buffer format has no byte
    /// size — does not heal. See `android_frame::PresentFaultLog`.
    pub(super) fn present_pixels(&mut self, pixels: &[u8], width: u32, height: u32) -> bool {
        use ndk::hardware_buffer_format::HardwareBufferFormat;

        if width != self.width || height != self.height {
            self.resize(width, height);
        }

        let mut guard = match self.native.lock(None) {
            Ok(g) => g,
            Err(_) => return false,
        };

        // **Every byte of this buffer has to be written before we return.**
        //
        // `ANativeWindow_unlockAndPost` has no cancel — dropping the guard
        // posts whatever the buffer holds — and `lock(None)` declares the whole
        // buffer dirty, so Android copies nothing forward into it from the last
        // frame. A bail that simply returns therefore does not drop a frame, it
        // posts an uninitialised one. Where the pixels cannot be written, black
        // is written instead.
        //
        // The format check is the one softbuffer used to do for us
        // (`next_buffer` refused anything but 32-bit RGBA/RGBX). It cannot be
        // folded into `lines()` returning `None`: the Android default is
        // `R5G6B5` 16bpp, which has a perfectly good `bytes_per_pixel` of 2, so
        // a window whose `set_buffers_geometry` was refused hands back lines
        // half the expected width and RGBA8 bytes poured into them are a
        // garbled screen, not a wrong one.
        let format = guard.format();
        let format_ok = matches!(
            format,
            HardwareBufferFormat::R8G8B8A8_UNORM | HardwareBufferFormat::R8G8B8X8_UNORM
        );
        if !format_ok {
            log::error!("present: window buffer is {format:?}, not 32-bit RGBA/RGBX");
        }

        // `None` only for a format with no byte size at all, which leaves no
        // safe way to reach the bytes — the one case where the unwritten buffer
        // goes out as it is.
        let Some(lines) = guard.lines() else {
            if self.faults.failed() {
                log::error!(
                    "present: window buffer format {format:?} has no byte size. \
                     The frame is owed, so the loop retries it once per display \
                     frame (#563); repeats are logged at debug until a present \
                     succeeds."
                );
            } else {
                log::debug!("present: window buffer format {format:?} has no byte size");
            }
            return false;
        };

        let src_stride = width as usize * 4;
        for (y, line) in lines.enumerate() {
            let src = y * src_stride;
            if !format_ok || src + src_stride > pixels.len() {
                line.fill(std::mem::MaybeUninit::new(0));
                continue;
            }
            let n = line.len().min(src_stride);
            // SAFETY: `MaybeUninit<u8>` has the same layout as `u8`, so an
            // initialised `&[u8]` is a valid `&[MaybeUninit<u8>]` to read
            // from. This is the transmute `MaybeUninit::copy_from_slice`
            // performs, written out because that method is not yet stable on
            // this toolchain.
            let src_uninit = unsafe {
                std::slice::from_raw_parts(
                    pixels[src..src + n]
                        .as_ptr()
                        .cast::<std::mem::MaybeUninit<u8>>(),
                    n,
                )
            };
            line[..n].copy_from_slice(src_uninit);
            // A window wider than the frame leaves a tail on every line; it is
            // posted along with the rest, so it cannot be left uninitialised.
            line[n..].fill(std::mem::MaybeUninit::new(0));
        }
        // Dropping the guard unlocks the buffer and posts it.
        self.faults.presented();
        true
    }
}
