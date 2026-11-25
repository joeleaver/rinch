//! One window's swapchain and vello's render target (`android-gpu`).

use super::gpu_context::GpuContext;
use crate::shell::android_frame;

/// One window's swapchain, and the texture vello is allowed to draw into.
///
/// **Why there is an intermediate texture at all.** Vello is a compute
/// rasteriser: its final stage writes finished pixels into a *storage* image
/// from a compute shader. A swapchain image is a colour attachment, and on
/// Android — as on most drivers — it is not usable as a storage image, so
/// vello cannot be pointed at it. `render_to_texture` therefore draws into
/// `target`, an `Rgba8Unorm` storage texture the size of the window, and a
/// full-screen triangle then samples that into the swapchain image in the
/// format the display asked for. Vello's own `util` module does exactly this
/// and its documentation is explicit that the alternative — handing vello the
/// swapchain texture when the driver happens to allow it — "should generally
/// be avoided, as some GPUs assume that you will not be rendering to the
/// surface using a compute pipeline, and optimise accordingly".
///
/// The blit costs one texture read and one write per pixel, entirely on the
/// GPU. What it replaces is the readback this path used to do: a
/// `copy_texture_to_buffer` into mappable memory, a blocking wait for it, and
/// a row-by-row CPU copy to strip wgpu's 256-byte row padding — 50 to 69ms per
/// frame on this device, against a rasterisation that took 2 to 5ms. Card K27
/// measured that; card K35 is this rewrite, and measured it again the same
/// way — a `log::info!` per frame through `adb logcat`, medians over a few
/// hundred frames, on the same three screens of the same app on the same moto
/// g stylus 5G at 1080x2460, release build.
///
/// ```text
///           resolve  scene   vello  readback  unpack  present   total
/// K27, reading the frame back:
/// library       0.0    0.0     2.3      51.6    16.5     12.2    84.0
/// sheet         0.0    2.4     1.8      44.8    14.7     12.1    74.8
/// pdf           0.0    0.0     5.0      64.0    16.6     12.6    96.9
///
/// K35, presenting the swapchain:
/// library       0.0    0.0     2.4         -       -      1.6     4.1
/// sheet         0.0    1.2     2.5         -       -     15.3    18.8
/// pdf           0.0    0.0     5.6         -       -      1.6     7.4
/// ```
///
/// The `readback` and `unpack` columns are gone, which was the entire point,
/// and `vello` is unchanged — 2.4 against 2.3, 5.6 against 5.0 — which is the
/// evidence that nothing about the *rasterisation* was disturbed. A frame on
/// the library list costs 4.1ms where it cost 84.0.
///
/// **Read the `present` column carefully; it is not all work.** It is the
/// acquire, the blit and the `vkQueuePresentKHR` together, and under Fifo the
/// first and last of those block on the display. Broken out, the library
/// frame is `acquire 0.1 / blit 1.1 / present 0.4` and the sheet frame is
/// `acquire 0.1 / blit 1.1 / present 14.1`. The blit is a millisecond on all
/// three screens; the sheet's extra 14ms is vsync back-pressure, and it
/// appears there and not on the other two because the sheet is the only one of
/// the three that rebuilds its scene every frame (`scene 1.2`) and so actually
/// gives the compositor something new often enough to be throttled.
///
/// **What is left is not this file.** Frame-to-frame, the same traces measure
/// 20.4 / 36.0 / 24.0 ms — 4 to 17ms more than the paint costs. That residue is
/// the `poll_events(Some(16ms))` timeout at the top of the loop, which blocks
/// for its full 16ms whenever no input is arriving, and it is paid by the
/// software path identically. Presenting the swapchain has moved the ceiling
/// from "the readback" to "the loop's own clock", which is a different card.
///
/// Card **K37** is that card, and it changes how the numbers above should be
/// read. The loop no longer sleeps between frames, so the wait it used to do in
/// `poll_events` now happens here instead, inside the present. Re-measured with
/// the acquire timed separately from the rest, on the library, the sheet and a
/// PDF page: the acquire is 0.08 / 0.07 / 0.06ms — the swapchain always has an
/// image ready — and everything after it (blit, submit, `frame.present()`) is
/// 12.5 / 31.1 / 4.9ms, which on the first two is the queue refusing another
/// frame rather than work appearing from nowhere. The PDF's 4.9 is the useful
/// control: its whole paint is 22.2ms, so seventeen of those milliseconds are
/// `build_scene` and `render_to_texture` and no amount of pacing will move
/// them. Frame-to-frame — the number that is about the user and not about this
/// file — went to 16.3 / 40.9 / 23.0. See `android_frame::poll_timeout` for
/// both halves of that table.
///
/// **Card K42 then asked why the library frame lands on every second vsync,
/// and the answer is not in this file at all.** It is worth reading before
/// changing anything below, because all four of the things a reader would
/// reach for first have now been measured on the handset: three of them move
/// nothing, and the fourth cannot be changed from this repository.
///
/// The frame that started K42 was `build_scene 3.43 / render_to_texture 2.04 /
/// acquire 0.07 / blit encode 0.04 / submit 0.66 / present 9.57`, and the
/// obvious reading of it — 6.17ms of work, 9.57ms blocked in the present, so
/// something about the swapchain is holding the caller — is wrong. Every one
/// of those numbers is CPU-side, and *all six of them together* are the cost
/// of writing a command buffer, not the cost of executing it. The rasterisation
/// itself is not in the table.
///
/// Timed with a `device.poll(PollType::Wait)` immediately after
/// `render_to_texture` submits and before the acquire — so the wait is on
/// vello's own submission with no swapchain image involved, and cannot be
/// confused for back-pressure — the moto g stylus 5G (Adreno 619, 1080x2460)
/// takes **17.8ms of GPU time to rasterise one frame of the library list**
/// (17.4 in the second run, the one broken down further below; the two runs
/// bracket it). Everything after that — the blit, the wait for the swapchain
/// image, `vkQueuePresentKHR` — is 1.6ms together. Against a 120Hz panel's
/// 8.33ms that is 2.1x over budget, and against the 17.2ms frame-to-frame the
/// compositor actually reports it means the pipeline is already running at
/// essentially 100% of its real bottleneck. There is no idle time in this
/// frame to reclaim. The 9.57ms attributed to `frame.present()` was the queue
/// declining to take another frame from a GPU that had not finished the last
/// one — accurate as a measurement, misleading as a diagnosis.
///
/// Measured the way K40 did rather than the way K39 did, because an
/// in-process probe is exactly what got this wrong the first time:
/// `dumpsys SurfaceFlinger --latency` on the app's own layer, polled through
/// the run and stitched (it is a 128-entry ring), over a scripted fling on the
/// library list. The stock Settings list flicked by the same script on the
/// same panel is the control — it is what "done" looks like on this handset.
///
/// ```text
///                                          p50     p95     p99    fps  missed
/// Settings, the control                   8.33    8.66   16.71  115.9    3.4%
/// this file as it stands                 16.67   24.99   25.29   58.1   51.6%
/// desired_maximum_frame_latency: 1       16.67   24.99   25.35   57.7   51.9%
/// desired_maximum_frame_latency: 3       16.67   24.99   25.31   57.6   52.0%
/// present_mode: Mailbox                  16.67   25.00   25.38   57.8   51.9%
/// AaConfig::Msaa8                        49.98   50.31   50.39   20.5   82.9%
/// AaConfig::Msaa16                       58.20   58.71   58.79   18.0   85.0%
/// vello at half scale, blit upscales      8.34   24.82   25.39  101.0   15.9%
/// ```
///
/// The first four rows are the same run to within noise, which is what a
/// saturated GPU looks like from every angle you can configure a swapchain
/// from. `desired_maximum_frame_latency` is the swapchain's image count in
/// disguise — wgpu passes `latency + 1` as `min_image_count` — so 1 and 3 are
/// two and four images, and neither a shallower queue nor a deeper one changes
/// a frame the GPU cannot finish in time. Mailbox is the same answer with a
/// worse battery: it would only help if the loop were being *paced* by the
/// display, and it is being paced by itself.
///
/// The last three rows are the ones that move, and they all move the same
/// quantity: pixels. Both MSAA modes are far worse than the analytic
/// `AaConfig::Area` this file already uses, which is now a measurement rather
/// than an assumption. And halving vello's render scale — a quarter of the
/// pixels, with the blit already present to resample them back up — takes the
/// median to the panel's own 8.33ms. Broken down at full resolution: an
/// **empty** scene still costs 4.19ms of GPU, which is half the 120Hz budget
/// spent before anything is drawn, and the real scene costs 17.4; at half
/// scale the same scene costs 8.06. So roughly 5ms of the frame is fixed and
/// 12ms is per-pixel work in vello's fine stage, and the road to 120fps on
/// this handset runs through one of those two numbers, not through anything
/// on this page.
///
/// **The blit is the one suspect that did cost something, and it cannot be
/// removed at this pin.** Its device-side cost — as opposed to the 0.04ms to
/// encode it — is most of the 1.6ms above: a full-screen read and write of
/// 2.66 million pixels. This window would allow it to go: `get_capabilities` answers
/// `usages=COPY_SRC | COPY_DST | TEXTURE_BINDING | STORAGE_BINDING |
/// RENDER_ATTACHMENT | STORAGE_ATOMIC` and offers `Rgba8Unorm`, which is
/// exactly what vello's fine stage wants to write. Configuring the surface
/// that way and handing vello the swapchain view compiles, runs, logs `vello
/// writes the swapchain image` — and then panics on the first frame with
/// `Device::create_bind_group: The adapter does not support write access for
/// storage textures of format Rgba8Unorm`, which is not true of this adapter.
/// It is true of wgpu: `wgpu-core`'s `present.rs` builds every surface
/// texture with `format_features.allowed_usages =
/// TextureUsages::RENDER_ATTACHMENT` and a `flags` set containing only the
/// two multisample bits, hard-coded, whatever the surface advertised. So no
/// swapchain image is bindable as a storage texture in wgpu 27 (nor 30) and vello can
/// never write one, on any driver. That is a wgpu change, not a rinch one,
/// and it is worth something like a millisecond and a half of a 17.4ms frame
/// if it ever lands — which is not the difference between 58fps and 120, but
/// is most of the difference between a p95 of 25ms and a p95 of 16.7.
#[cfg(feature = "android-gpu")]
pub(super) struct GpuSurface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    /// Vello's canvas: `Rgba8Unorm`, `STORAGE_BINDING` so vello may write it,
    /// `TEXTURE_BINDING` so the blit may read it.
    target: wgpu::Texture,
    target_view: wgpu::TextureView,
    blitter: wgpu::util::TextureBlitter,
    /// Clones of the context's device and queue — both are `Arc` handles, so
    /// this costs a refcount and buys `resize` and `present_scene` the ability
    /// to work without being handed the whole context. The renderer is *not*
    /// cloned, because vello's `Renderer` is a `&mut` resource with a frame's
    /// worth of scratch buffers in it and there must only ever be one.
    pub(super) device: wgpu::Device,
    queue: wgpu::Queue,
    width: u32,
    height: u32,
    /// Keeps a hard GPU fault from becoming ~120 `error` lines a second now
    /// that the loop retries it. See [`android_frame::PresentFaultLog`].
    faults: android_frame::PresentFaultLog,
}

#[cfg(feature = "android-gpu")]
impl GpuSurface {
    pub(super) fn new(
        ctx: &GpuContext,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
    ) -> Option<Self> {
        let width = width.max(1);
        let height = height.max(1);
        let caps = surface.get_capabilities(&ctx.adapter);

        log::info!(
            "GPU surface: formats={:?} present={:?} alpha={:?}",
            caps.formats,
            caps.present_modes,
            caps.alpha_modes
        );

        // **Format negotiation, and the one wrong answer.**
        //
        // Vello writes an `Rgba8Unorm` texture whose bytes are already
        // sRGB-encoded — that is what a CSS colour is. The blit samples that
        // texture (a `Unorm` read hands the shader the stored value untouched)
        // and writes the result to the swapchain. So the swapchain must also
        // be a plain `Unorm` format: an `*UnormSrgb` attachment would apply a
        // linear-to-sRGB encode on the way out, on values that are already
        // encoded, and the whole app would come out washed out and pale. That
        // failure is uniform and subtle enough to be mistaken for a theme bug,
        // which is why it is spelled out here rather than left to the reader
        // of a `matches!`.
        //
        // Between the two acceptable formats there is nothing to choose:
        // `Bgra8Unorm` is what Android hands out and the byte swap is free in
        // the blit's fragment shader, since the sampler yields components by
        // name and the attachment stores them by its own order.
        let format = match caps.formats.iter().copied().find(|f| {
            matches!(
                f,
                wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm
            )
        }) {
            Some(f) => f,
            None => {
                log::error!(
                    "GPU: window offers no non-sRGB 8-bit format ({:?}); no GPU frames",
                    caps.formats
                );
                return None;
            }
        };

        // Opaque if the window will take it, matching the software path's
        // `R8G8B8X8_UNORM` request and for the same reason: this shell paints
        // every pixel of a full-screen frame, there is nothing behind it, and
        // asking the compositor to blend a surface with no alpha to respect is
        // work nobody wants done. `Auto` is the fallback and lets wgpu pick
        // whatever the window does support.
        let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
            wgpu::CompositeAlphaMode::Opaque
        } else {
            wgpu::CompositeAlphaMode::Auto
        };

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            // **Fifo, which means this loop no longer decides when to
            // present.**
            //
            // Fifo is the one present mode Vulkan guarantees exists, and it is
            // vsync: `get_current_texture` blocks until the display has
            // released an image. That is a real change in who paces the frame
            // loop, and it is the change that was already true. The software
            // path's `ANativeWindow_lock` blocks for a free buffer in exactly
            // the same way — K27's note that "what is left is
            // `ANativeWindow_lock` waiting for a free buffer, which is the
            // display's back-pressure rather than work" describes Fifo by
            // another name. The loop's own 16ms `poll_events` timeout was
            // always an approximation of the same 60Hz; now something
            // authoritative enforces it. Card K37 then removed the
            // approximation: the loop asks the display what a frame costs and
            // waits out the remainder of it, so on the 120Hz panel this was
            // measured on the two clocks finally agree instead of the slower
            // guess winning.
            //
            // The cost is that a timing probe around the acquire measures
            // *waiting*, not work, and a reader of those numbers who forgets
            // it will conclude the present is expensive. Mailbox would hide
            // the wait by rendering frames nobody sees, which is a worse thing
            // to do to a phone battery than to a benchmark.
            //
            // Card K42 measured Mailbox here rather than leaving that as an
            // argument, because a number beats a principle: on the moto g
            // stylus 5G it is 57.8fps against Fifo's 58.1, with the same
            // percentiles to two decimal places. It buys nothing, because
            // this frame is not waiting for the display — it is waiting for
            // itself. Same for the frame latency below, which wgpu turns into
            // `min_image_count = latency + 1`: two images and four images both
            // measure 57.7 and 57.6. See the table on `GpuSurface`.
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&ctx.device, &config);

        let (target, target_view) = Self::make_target(&ctx.device, width, height);

        Some(Self {
            surface,
            config,
            target,
            target_view,
            blitter: wgpu::util::TextureBlitter::new(&ctx.device, format),
            device: ctx.device.clone(),
            queue: ctx.queue.clone(),
            width,
            height,
            faults: android_frame::PresentFaultLog::default(),
        })
    }

    fn make_target(
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("rinch-vello-target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    pub(super) fn resize(&mut self, width: u32, height: u32) {
        let (w, h) = (width.max(1), height.max(1));
        if (w, h) == (self.width, self.height) {
            return;
        }
        self.width = w;
        self.height = h;
        self.config.width = w;
        self.config.height = h;
        // Both halves, in this order. Reconfiguring the surface retires the
        // old swapchain images; the intermediate has to grow to match or the
        // blit would stretch a window-sized frame across a differently sized
        // window, which on a rotation is not a subtle artefact.
        self.surface.configure(&self.device, &self.config);
        let (target, view) = Self::make_target(&self.device, w, h);
        self.target = target;
        self.target_view = view;
    }

    fn reconfigure(&mut self) {
        self.surface.configure(&self.device, &self.config);
    }

    /// Take the next swapchain image, rebuilding the swapchain if the window
    /// has moved on without telling us.
    ///
    /// `Outdated` and `Lost` are not errors here, they are how a compositor
    /// says the window changed — a rotation, a fold, an insets change, a
    /// resize the Activity has not yet delivered as `WindowResized`. The
    /// answer to both is the same: reconfigure and ask once more.
    ///
    /// The retry is bounded at one, and deliberately does not try to work out
    /// the window's new size for itself. If the window really did change
    /// shape, this surface is still configured at the old one until
    /// `WindowResized` reaches the loop above and calls `resize` — a frame or
    /// two of the compositor scaling a slightly-wrong image, which is
    /// invisible, against a spin here that would not be. Dropping a frame is
    /// this function's failure mode; looping is not.
    ///
    /// **`suboptimal` is deliberately not one of the conditions retried on.**
    /// wgpu never reports it on Android: `wgpu-hal`'s Vulkan `acquire_texture`
    /// maps `VK_SUBOPTIMAL_KHR` to success under `target_os = "android"`,
    /// because wgpu creates every swapchain with `preTransform = IDENTITY` and
    /// libvulkan then returns `VK_SUBOPTIMAL_KHR` *persistently* for any
    /// display orientation that is not the identity one. Retrying on it would
    /// be dead code as written, and live code that rebuilt the whole swapchain
    /// and paid two vsync waits on **every** frame in landscape.
    fn acquire(&mut self) -> Option<wgpu::SurfaceTexture> {
        for attempt in 0..2 {
            match self.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame)
                | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => return Some(frame),
                e @ (wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost)
                    if attempt == 0 =>
                {
                    // Read rather than claim: this line sits in front of the
                    // `error` below on the same failed attempt, and must not
                    // spend the one report that fault is owed.
                    if self.faults.reported() {
                        log::debug!("GPU: swapchain {e:?}, reconfiguring");
                    } else {
                        log::info!("GPU: swapchain {e:?}, reconfiguring");
                    }
                    self.reconfigure();
                }
                e => {
                    if self.faults.failed() {
                        log::error!(
                            "GPU: acquire failed: {e:?}. The frame is owed, so the loop \
                             retries it once per display frame (#563); repeats are logged \
                             at debug until a present succeeds."
                        );
                    } else {
                        log::debug!("GPU: acquire failed: {e:?}");
                    }
                    return None;
                }
            }
        }
        None
    }

    /// Rasterise the scene and put it on the screen, with no trip through
    /// system memory.
    ///
    /// The order — rasterise, *then* acquire — is deliberate. Under Fifo the
    /// acquire is where the display's back-pressure was expected to land, so
    /// submitting vello's compute work first gives the GPU something to do
    /// during that wait instead of starting it afterwards. It also means an
    /// acquire that fails costs only the frame, not the scene: the
    /// intermediate texture still holds a valid, correctly sized frame, and
    /// the next iteration draws over it.
    ///
    /// The first of those reasons has never actually been observed to pay:
    /// K37 measured the acquire at 0.06-0.08ms and K42 at 0.07 with a 0.12
    /// maximum, on a frame that misses half the panel's refreshes. The
    /// swapchain always has an image ready, because the thing this loop is
    /// short of is not images — it is GPU. The ordering stays for the second
    /// reason, which is about correctness rather than speed.
    ///
    /// Returns whether the frame actually reached the swapchain. See
    /// [`SoftSurface::present_pixels`] for why the loop needs to be told:
    /// both early returns below are fast failures, and a loop that paced
    /// itself on "we tried to paint" rather than "we painted" would spin
    /// through them. (It does retry a frame that failed, one display frame
    /// later — issue #563, `android_frame::retry_owed_frame` — which is the
    /// bounded version of the same thing and needs this `bool` to tell the two
    /// apart. "One display frame" is true only because the loop restarts its
    /// frame clock as it arms; the subtraction alone would saturate to zero on
    /// any frame that overran, which on this path is most of them.)
    pub(super) fn present_scene(
        &mut self,
        renderer: &mut vello::Renderer,
        scene: &vello::Scene,
    ) -> bool {
        if let Err(e) = renderer.render_to_texture(
            &self.device,
            &self.queue,
            scene,
            &self.target_view,
            &vello::RenderParams {
                base_color: peniko::Color::from_rgba8(255, 255, 255, 255),
                width: self.width,
                height: self.height,
                antialiasing_method: vello::AaConfig::Area,
            },
        ) {
            if self.faults.failed() {
                log::error!(
                    "GPU render failed: {e}. The frame is owed, so the loop retries it \
                     once per display frame (#563); repeats are logged at debug until a \
                     present succeeds."
                );
            } else {
                log::debug!("GPU render failed: {e}");
            }
            return false;
        }

        let Some(frame) = self.acquire() else {
            return false;
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("rinch-blit"),
            });
        self.blitter
            .copy(&self.device, &mut encoder, &self.target_view, &view);
        self.queue.submit(std::iter::once(encoder.finish()));
        self.queue.present(frame);
        self.faults.presented();
        true
    }
}

#[cfg(feature = "android-gpu")]
impl Drop for GpuSurface {
    /// Wait for the GPU to be finished with the swapchain before the swapchain
    /// stops existing.
    ///
    /// This runs before any field is dropped, which is the point. The last
    /// frame this surface presented may still be in the compositor's hands and
    /// its command buffer still executing; `wgpu` will destroy the
    /// `VkSwapchainKHR` when the `Surface` field goes, and destroying a
    /// swapchain with work outstanding against its images is undefined
    /// behaviour. Waiting here — inside `TerminateWindow`, while Android's own
    /// thread is still blocked waiting for this loop to acknowledge the
    /// event — is the one moment where the wait is both safe and free of
    /// anything to race against.
    fn drop(&mut self) {
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
    }
}

/// The `ANativeWindow`, wearing the two traits wgpu needs to make a surface
/// from it.
///
/// `ndk` implements `HasWindowHandle` for `NativeWindow` but not
/// `HasDisplayHandle` — there is no display object on Android to point at —
/// so the pair has to be assembled here. The `native` field is a *clone*,
/// which on `NativeWindow` is an `ANativeWindow_acquire`: wgpu takes ownership
/// of this value and keeps it for the life of the `Surface`, so the window
/// cannot be freed out from under a live swapchain even if the Activity
/// releases its own reference first. That is the second lock on the lifecycle,
/// under the one in `TerminateWindow`.
#[cfg(feature = "android-gpu")]
pub(super) struct AndroidWindow {
    pub(super) native: ndk::native_window::NativeWindow,
}

#[cfg(feature = "android-gpu")]
impl raw_window_handle::HasWindowHandle for AndroidWindow {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        self.native.window_handle()
    }
}

#[cfg(feature = "android-gpu")]
impl raw_window_handle::HasDisplayHandle for AndroidWindow {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        Ok(unsafe {
            raw_window_handle::DisplayHandle::borrow_raw(
                raw_window_handle::RawDisplayHandle::Android(
                    raw_window_handle::AndroidDisplayHandle::new(),
                ),
            )
        })
    }
}
