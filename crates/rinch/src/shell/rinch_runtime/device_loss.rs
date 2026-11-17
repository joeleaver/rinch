//! Replacing the shared GPU device after a loss (#1788): the event loop's
//! half of `shell::gpu_device`.

use super::*;
use crate::shell::gpu_device::{self, GPU};

impl RinchRuntime {
    /// Replace the shared GPU device when it is due (lost, or an app given
    /// `App::external_gpu` handed over another), then tell the app's
    /// listeners of a device that changed since they were last told. Called
    /// at every wake and before every redraw, so a loss is dealt with before
    /// the next frame is drawn on the dead device.
    pub(super) fn settle_gpu_device(&mut self) {
        GPU.poll_published();
        if GPU.take_due() {
            self.replace_gpu_device();
        }
        if let Some(handle) = GPU.take_announcement() {
            gpu_device::announce(handle.as_ref());
        }
    }

    /// Rebuild every window's renderer on a new device. Every renderer goes
    /// first (each holds its surface, Vello's renderer, the compositors and
    /// the split slices, all on the old device), the shown windows' and the
    /// ones hidden windows keep for their next show (#2545) alike, then the
    /// device itself and the surfaces' GPU targets and registered textures;
    /// then each shown window that presented on the GPU builds its renderer
    /// as at startup, the first making (or adopting) the next device, which
    /// it publishes. A window whose renderer cannot start presents with
    /// software where its choice allows, as at startup.
    ///
    /// Every frame on the old device is gone, as a frame Vello lost is
    /// (#2071), and each window draws again the way it does for that
    /// ([`ShellWindow::redraw_lost_frame`]): a shown window redraws, in full;
    /// a hidden window the app prepared (#2633) is prepared again, on the new
    /// device, so its show still finds a drawn frame; any other hidden
    /// window builds its renderer at its next show. A window that presents
    /// with software lost nothing and does nothing. The windows' frames still
    /// out to Vello went with their renderers, so the lost-frame look
    /// (`look_at_frames`, #2781) has nothing of the old device left to ask.
    fn replace_gpu_device(&mut self) {
        // The shown windows that presented on the GPU, and every window
        // that had a frame on the old device (those and the hidden ones that
        // kept a renderer).
        let (mut on_gpu, mut lost_frames) = (Vec::new(), Vec::new());
        for role in self.roles() {
            let w = self.w(role);
            let parked = w.parked_renderer.take().is_some();
            let shown = w.renderer.take().is_some();
            if shown {
                on_gpu.push(role);
            }
            if shown || parked {
                lost_frames.push(role);
            }
        }
        GPU.retire();
        crate::render_surface::forget_gpu_device();

        for &role in &on_gpu {
            let w = self.w(role);
            let Some(window) = w.window.as_ref().map(|w| w.window.clone()) else {
                continue;
            };
            if role == WindowRole::DevTools {
                // DevTools presents as it was opened: with `WgpuRenderer::try_new`.
                let size = window.surface_size();
                w.renderer = WgpuRenderer::try_new(&*window, size.width.max(1), size.height.max(1))
                    .map_err(|e| {
                        tracing::warn!(
                            "DevTools: GPU renderer unavailable after the device loss ({e}); \
                                 using software"
                        )
                    })
                    .ok();
            } else {
                let transparent = w.gpu_surface_transparent();
                let (renderer, built) = w.start_gpu_renderer(&*window, transparent);
                w.note_renderer_built(built);
                w.renderer_started(renderer);
            }
        }
        for role in lost_frames {
            let w = self.w(role);
            // Nothing of the old frames survives: the next one is full.
            w.app.mark_scene_dirty();
            w.redraw_lost_frame();
        }
        tracing::info!(
            "rinch: GPU device replaced (generation {})",
            crate::shell::desktop::gpu_generation()
        );
    }
}
