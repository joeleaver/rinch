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
    /// the split slices, all on the old device), then the device itself and
    /// the surfaces' GPU targets and registered textures; then each window
    /// that presented on the GPU builds its renderer as at startup, the first
    /// making (or adopting) the next device, which it publishes. A window
    /// whose renderer cannot start presents with software where its choice
    /// allows, as at startup. A hidden window's kept renderer is dropped: its
    /// next show builds one.
    fn replace_gpu_device(&mut self) {
        let roles: Vec<WindowRole> = self.roles().collect();
        let mut on_gpu = Vec::new();
        for &role in &roles {
            let w = self.w(role);
            w.parked_renderer = None;
            if w.renderer.take().is_some() {
                on_gpu.push(role);
            }
        }
        GPU.retire();
        crate::render_surface::forget_gpu_device();

        for role in on_gpu {
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
            // Nothing of the old frames survives: the next one is full.
            w.app.mark_scene_dirty();
            window.request_redraw();
        }
        tracing::info!(
            "rinch: GPU device replaced (generation {})",
            crate::shell::desktop::gpu_generation()
        );
    }
}
