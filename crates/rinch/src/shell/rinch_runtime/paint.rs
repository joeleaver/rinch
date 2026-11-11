//! Painting the main window, in software and on the GPU.

use super::*;

impl RinchRuntime {
    /// Paint the current scene to the window.
    pub(super) fn paint(&mut self) -> Result<(), String> {
        // A paint inside the drain (a debug screenshot) presents a frame,
        // which commits: the layer changes staged so far go before it, in
        // their one commit (#2464).
        self.main.flush_layer_changes();
        // Update inspect highlight rect from DevTools store before painting
        self.update_inspect_highlight();

        let paint_start = std::time::Instant::now();

        let result = {
            #[cfg(feature = "gpu")]
            {
                if self.main.renderer.is_some() {
                    self.main.paint_gpu()
                } else {
                    self.main.paint_software()
                }
            }
            #[cfg(not(feature = "gpu"))]
            {
                self.main.paint_software()
            }
        };

        // Track frame timing for DevTools Performance panel
        if let Some(last) = self.main.last_paint_time {
            let frame_ms = paint_start.duration_since(last).as_secs_f64() * 1000.0;
            if self.main.frame_times.len() >= 60 {
                self.main.frame_times.pop_front();
            }
            self.main.frame_times.push_back(frame_ms);

            if let Some(store) = &self.devtools_store {
                if store.visible.get() {
                    let paint_ms = paint_start.elapsed().as_secs_f64() * 1000.0;
                    store.frame_time_ms.set(paint_ms);

                    // Average FPS over recent frames
                    if !self.main.frame_times.is_empty() {
                        let avg_ms: f64 = self.main.frame_times.iter().sum::<f64>()
                            / self.main.frame_times.len() as f64;
                        if avg_ms > 0.0 {
                            store.fps.set(1000.0 / avg_ms);
                        }
                    }
                }
            }
        }
        self.main.last_paint_time = Some(paint_start);

        // One redraw is one performance frame (`rinch_dom::perf`).
        self.main.app.end_perf_frame();

        result
    }
}

impl ShellWindow {
    pub(super) fn paint_software(&mut self) -> Result<(), String> {
        let Some(window) = &self.window else {
            return Ok(());
        };

        // Ensure pending cross-thread closures are processed and layout
        // is resolved before painting (same as paint_gpu).
        rinch_core::drain_main_callbacks();
        rinch_core::reactive::drain_polls();
        let scale = window.scale_factor();
        // Layout and paint work in logical (CSS) pixels; `inner_size` is
        // physical (see `to_logical`).
        let size = window.logical_size();
        if self.app.has_pending_layout() {
            self.app.resolve_and_repaint(size.0 as f32, size.1 as f32);
        }

        let transparent = self.app.is_transparent();
        let s = scale as f32;

        // Update layout sizes for render surfaces. Scans the DOM for
        // data-render-surface attributes rather than relying on the registry,
        // so surfaces survive reactive scope rebuilds (where unregister runs
        // after re-mount, temporarily removing the surface from the registry).
        // The window's surfaces: what this paint drives and collects (#2416).
        let surfaces = self.app.window_surface_ids(s);
        crate::render_surface::note_window_surfaces(self.facts_key, &surfaces);

        // Invoke per-frame render callbacks before collecting frames.
        crate::render_surface::invoke_window_render_callbacks(&surfaces);

        // Collect surface pixel data and set it for inline painting.
        // Surfaces are painted inline during paint_document() (like <img> elements).
        let surface_pixels = crate::render_surface::collect_surface_pixels_by_id(&surfaces);
        if !surface_pixels.is_empty() {
            // Mark scene dirty so build_pixels() actually repaints — the
            // damage is the surface nodes, marked just below.
            self.app.request_repaint();
            // ...and mark the surface nodes themselves, for the same reason the
            // video viewports are marked below: inline painting is subject to
            // the dirty-region cache, and a small dirty region elsewhere would
            // otherwise prune the surface's subtree and freeze its last frame.
            // This has to happen here, at collect time — the collector above
            // has just cleared `needs_redraw`, so nothing downstream can still
            // tell which surfaces delivered a frame.
            let ids: Vec<usize> = surface_pixels.keys().copied().collect();
            self.app.mark_surface_nodes_paint_dirty(&ids);
            rinch_dom::paint::set_surface_pixels(Some(surface_pixels));
        }

        // Also update layout sizes for video/GameViewport surfaces that use the
        // viewport_name path (data-viewport attribute).
        let reg_names = crate::render_surface::registered_viewport_names();
        for viewport_name in &reg_names {
            if let Some(viewport) = self.app.viewport_rect(viewport_name) {
                let phys_w = (viewport.2 * s) as u32;
                let phys_h = (viewport.3 * s) as u32;
                crate::render_surface::update_layout_size(viewport_name, phys_w, phys_h);
                crate::render_surface::update_layout_position(
                    viewport_name,
                    viewport.0,
                    viewport.1,
                );
            }
        }

        // Video and `GameViewport` frames paint **inline**, during paint, at
        // the viewport node's own z-order (issues #358, #361) — the software
        // backend has no compositor path. That is what lets a drawer, a modal,
        // a dropdown or a game's HUD above one survive: ordinary paint order
        // does the occluding, with no occlusion tracking anywhere. They used to
        // be blitted over the finished pixels, which destroyed all of those.
        //
        // Only a `GameViewport` still punches a hole in its ancestors'
        // backgrounds, and only while it has a frame (#186); its letterbox is
        // that hole. Video punches none and `rinch-dom` aspect-fits it over an
        // opaque black fill, which is #354's letterbox bars on this backend.
        //
        // Inline painting is subject to the dirty-region cache, so the viewport
        // nodes are marked as the damage — see `install_viewport_frames`.
        self.app
            .install_viewport_frames(crate::render_surface::collect_viewport_frames_by_name(
                &surfaces,
            ));

        // Build the scene — surfaces paint inline at their layout positions
        let (_base, w, h) = self.app.build_pixels(scale, size, transparent);

        RinchApp::clear_viewport_frames();
        rinch_dom::paint::set_surface_pixels(None);

        let pixels = self
            .app
            .skia_painter
            .as_ref()
            .map(|p| p.pixels())
            .unwrap_or(&[]);

        // Lazily create or get the softbuffer renderer
        if self.soft_renderer.is_none() {
            self.soft_renderer = Some(crate::shell::softbuffer_renderer::SoftbufferRenderer::new(
                window.window.clone(),
                w,
                h,
                transparent,
            ));
        }

        let present_start = std::time::Instant::now();
        if let Some(renderer) = &mut self.soft_renderer {
            renderer.present_pixels(pixels, w, h);
        }
        self.app.perf_add(
            rinch_dom::perf::Counter::TimePresentNs,
            present_start.elapsed().as_nanos() as u64,
        );

        Ok(())
    }

    #[cfg(feature = "gpu")]
    pub(super) fn paint_gpu(&mut self) -> Result<(), String> {
        let Some(renderer) = &mut self.renderer else {
            return Ok(());
        };
        let Some(window) = &self.window else {
            return Ok(());
        };

        // Ensure pending cross-thread closures (run_on_main_thread) are
        // processed and layout is resolved before painting. Without this,
        // continuous RedrawRequested from render surfaces can paint with
        // stale layout if the ReRender event from signal changes hasn't
        // been processed yet.
        rinch_core::drain_main_callbacks();
        rinch_core::reactive::drain_polls();
        let scale = window.scale_factor();
        // Layout and paint work in logical (CSS) pixels; `inner_size` is
        // physical (see `to_logical`).
        let size = window.logical_size();
        if self.app.has_pending_layout() {
            self.app.resolve_and_repaint(size.0 as f32, size.1 as f32);
        }

        let transparent = self.app.is_transparent();
        let s = scale as f32;

        // Update layout sizes for all render surfaces. DOM-based scan
        // so surfaces survive reactive scope rebuilds. The window's surfaces:
        // what this paint drives and collects (#2416).
        let surfaces = self.app.window_surface_ids(s);
        crate::render_surface::note_window_surfaces(self.facts_key, &surfaces);

        // Also update layout sizes for video/GameViewport surfaces that still
        // use the viewport_name path (data-viewport attribute).
        let reg_names = crate::render_surface::registered_viewport_names();
        for viewport_name in &reg_names {
            if let Some(viewport) = self.app.viewport_rect(viewport_name) {
                let phys_w = (viewport.2 * s) as u32;
                let phys_h = (viewport.3 * s) as u32;
                crate::render_surface::update_layout_size(viewport_name, phys_w, phys_h);
                crate::render_surface::update_layout_position(
                    viewport_name,
                    viewport.0,
                    viewport.1,
                );
            }
        }

        // Invoke per-frame render callbacks before collecting frames.
        crate::render_surface::invoke_window_render_callbacks(&surfaces);

        // Read back GPU textures to CPU for inline-paint RenderSurface components.
        // This must happen before collect_surface_pixels_by_id().
        crate::render_surface::readback_gpu_textures(&surfaces);

        // Collect CPU surface pixel data for inline painting (RenderSurface path).
        // After readback, this includes both pure-CPU and readback-GPU surfaces.
        let surface_pixels = crate::render_surface::collect_surface_pixels_by_id(&surfaces);
        if !surface_pixels.is_empty() {
            // The surface nodes are the damage, as on the software path: a
            // partial frame elsewhere would otherwise prune them (#1718).
            self.app.request_repaint();
            let ids: Vec<usize> = surface_pixels.keys().copied().collect();
            self.app.mark_surface_nodes_paint_dirty(&ids);
            rinch_dom::paint::set_surface_pixels(Some(surface_pixels));
        }

        // `RenderSurface` GPU frames (#1768): each surface's target texture,
        // registered with this window's Vello renderer and drawn in the scene
        // as the surface element's content. A surface that drew this paint is
        // the damage. Always synced, so a surface that stopped drawing (or
        // left) has its texture unregistered.
        let gpu_frames = crate::render_surface::collect_gpu_surface_frames(&surfaces);
        let fresh: Vec<usize> = gpu_frames
            .iter()
            .filter(|(_, _, fresh)| *fresh)
            .map(|(id, ..)| *id)
            .collect();
        let gpu_images = renderer
            .sync_surface_images(gpu_frames.into_iter().map(|(id, texture, _)| (id, texture)));
        // A surface the window composites split (#1865) is not damage: its new
        // frame is composited from the kept slices, with no repaint.
        self.app.mark_fresh_surfaces(
            &fresh,
            &renderer.split_surface_ids(),
            &renderer.split_stale_ids(),
        );
        if !gpu_images.is_empty() {
            rinch_dom::paint::set_surface_gpu_images(Some(gpu_images));
        }

        // `GameViewport` surfaces still use the compositor/hole-punch path.
        // Track which viewport names have compositor layers so we can set
        // ACTIVE_VIEWPORTS to prevent hole-punching for inline-painted surfaces.
        let all_layers: Vec<rinch_platform::CompositeLayer> = Vec::new();
        let mut gpu_layers = Vec::new();
        let mut compositor_viewport_names: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        // A viewport that says it is not ready (`data-viewport-ready`) gets no
        // layer (#348): paint cuts it no hole and its node paints its own
        // background, and a layer under a node whose background is transparent
        // would show its last frame where the software backend shows none.
        let mut skipped_unready = false;

        // `GameViewport`s as split base layers (#1866): what paint offers to
        // the painter, the GPU textures and the CPU frames the compositor
        // reads. The layers above are gathered too, for a frame that falls
        // back to them.
        let try_base = self.base_fallback.try_split();
        let mut base_viewports: std::collections::HashMap<String, rinch_dom::paint::BaseViewport> =
            std::collections::HashMap::new();
        let mut base_views = std::collections::HashMap::new();
        let mut base_pending: Vec<crate::split_frame::PendingFrame> = Vec::new();
        // The CPU frames as the layered path's layers would be built from them.
        #[allow(clippy::type_complexity)]
        let mut cpu_layers: Vec<(
            std::sync::Arc<Vec<u8>>,
            u32,
            u32,
            (f32, f32, f32, f32),
            (f32, f32, f32, f32),
            [f32; 4],
            Option<(f32, f32, f32, f32)>,
        )> = Vec::new();

        // Collect compositor-path surface frames (video, GameViewport — not RenderSurface)
        {
            let surface_frames = crate::render_surface::collect_surface_frames_shared(&surfaces);
            for (id, viewport_name, pixels, surf_w, surf_h, fresh) in surface_frames {
                if !self.app.viewport_ready(&viewport_name) {
                    skipped_unready = true;
                    continue;
                }
                if let Some((viewport, radii)) = self.app.viewport_rect_with_radius(&viewport_name)
                {
                    compositor_viewport_names.insert(viewport_name.clone());
                    if surf_w > 0 && surf_h > 0 {
                        base_viewports.insert(
                            viewport_name.clone(),
                            rinch_dom::paint::BaseViewport {
                                surface_id: id,
                                extents: (surf_w, surf_h),
                                letterbox: true,
                            },
                        );
                        // Shared, not copied; uploaded only when new, or
                        // when the window holds no texture for it.
                        base_pending.push(crate::shell::base_fallback::pending_base_frame(
                            id,
                            &pixels,
                            (surf_w, surf_h),
                            fresh,
                        ));
                    }
                    // The whole box — what paint punched the hole over.
                    let box_viewport = (
                        viewport.0 * s,
                        viewport.1 * s,
                        viewport.2 * s,
                        viewport.3 * s,
                    );
                    // Letterbox: fit source within viewport preserving aspect ratio.
                    let viewport = {
                        let (vx, vy, vw, vh) = box_viewport;
                        let src_aspect = surf_w as f32 / surf_h.max(1) as f32;
                        let vp_aspect = vw / vh.max(1.0);
                        if (src_aspect - vp_aspect).abs() < 0.001 {
                            box_viewport
                        } else if src_aspect > vp_aspect {
                            let fit_h = vw / src_aspect;
                            let offset_y = (vh - fit_h) / 2.0;
                            (vx, vy + offset_y, vw, fit_h)
                        } else {
                            let fit_w = vh * src_aspect;
                            let offset_x = (vw - fit_w) / 2.0;
                            (vx + offset_x, vy, fit_w, vh)
                        }
                    };
                    let border_radius = [radii[0] * s, radii[1] * s, radii[2] * s, radii[3] * s];
                    let clip_rect = self
                        .app
                        .viewport_clip_rect(&viewport_name)
                        .map(|cr| (cr.0 * s, cr.1 * s, cr.2 * s, cr.3 * s));
                    // Black backdrop under the frame, covering the whole box, so
                    // the letterbox bars are black rather than see-through
                    // (issue #354). Pushed first: the compositor draws layers in
                    // order. Its radii are corrected to the corners the box
                    // actually shares with its clip, or a viewport that is only
                    // the top half of a rounded card would have its *middle*
                    // rounded away, leaving a see-through notch.
                    //
                    // Built only for the layered path: the split path reads the
                    // shared frame, and a copy per frame is not free.
                    cpu_layers.push((
                        pixels,
                        surf_w,
                        surf_h,
                        box_viewport,
                        viewport,
                        border_radius,
                        clip_rect,
                    ));
                }
            }
        }

        // Extract GPU texture sources for compositor — only non-inline surfaces
        // (video, GameViewport). Inline RenderSurface GPU textures are read back
        // to CPU above and painted inline.
        {
            let texture_sources = crate::render_surface::collect_texture_sources(&surfaces);
            for (surface_id, viewport_name, is_inline, tex_source_arc) in texture_sources {
                // Skip inline surfaces — they're already read back to CPU pixels
                if is_inline {
                    continue;
                }

                if let Some((viewport, radii)) = self.app.viewport_rect_with_radius(&viewport_name)
                {
                    let phys_w = (viewport.2 * s) as u32;
                    let phys_h = (viewport.3 * s) as u32;
                    crate::render_surface::update_layout_size_by_id(surface_id, phys_w, phys_h);

                    let viewport = (
                        viewport.0 * s,
                        viewport.1 * s,
                        viewport.2 * s,
                        viewport.3 * s,
                    );
                    let border_radius = [radii[0] * s, radii[1] * s, radii[2] * s, radii[3] * s];
                    let clip_rect = self
                        .app
                        .viewport_clip_rect(&viewport_name)
                        .map(|cr| (cr.0 * s, cr.1 * s, cr.2 * s, cr.3 * s));
                    if !self.app.viewport_ready(&viewport_name) {
                        // Sized above, so the app goes on rendering at the
                        // right size; only the layer is withheld.
                        skipped_unready = true;
                        continue;
                    }
                    // A hole only once there is a texture to fill it, on both
                    // paths (#1883 review): before, a viewport whose source
                    // was cleared punched with nothing behind it.
                    if let Some(ref ts) = *tex_source_arc.lock().unwrap() {
                        compositor_viewport_names.insert(viewport_name.clone());
                        base_viewports.insert(
                            viewport_name.clone(),
                            rinch_dom::paint::BaseViewport {
                                surface_id,
                                extents: (ts.width, ts.height),
                                letterbox: false,
                            },
                        );
                        base_views.insert(surface_id, ts.view.clone());
                        gpu_layers.push(crate::shell::desktop::GpuTextureLayer {
                            view: ts.view.clone(),
                            viewport,
                            border_radius,
                            clip_rect,
                        });
                    }
                }
            }
        }

        // Set or clear composite layers on the renderer (`GameViewport` CPU
        // frames and GPU textures only). Video is drawn in the scene since
        // #1768, so the retention a loaded video needed between its frames
        // (the last cycle's layers kept, with their holes) is gone with it.
        //
        // On the split path (#1866) the viewports are base layers and the
        // renderer holds no layers: the UI is rendered over the window's base
        // and the compositor replaces it inside each viewport with its game.
        let use_base = try_base && !base_viewports.is_empty();
        let layered = move || {
            let mut all_layers = all_layers;
            for (pixels, w, h, box_viewport, viewport, border_radius, clip_rect) in cpu_layers {
                all_layers.push(black_backdrop_layer(
                    box_viewport,
                    radii_at_shared_corners(box_viewport, clip_rect, border_radius),
                    clip_rect,
                ));
                all_layers.push(rinch_platform::CompositeLayer {
                    pixels: pixels.as_ref().clone(),
                    width: w,
                    height: h,
                    viewport,
                    border_radius,
                    clip_rect,
                });
            }
            (all_layers, gpu_layers)
        };
        let mut fallback_layers = Some(layered);
        if use_base {
            if renderer.has_composite_layers() {
                renderer.set_composite_layers(vec![]);
                renderer.set_gpu_layers(vec![]);
            }
            // A kept slice made for a frame of another size has a stale
            // image transform: the viewport is painted, and split, again.
            let stale: Vec<&str> = base_viewports
                .iter()
                .filter(|(_, b)| {
                    renderer
                        .split
                        .kept_extents(b.surface_id)
                        .is_some_and(|e| e != b.extents)
                })
                .map(|(name, _)| name.as_str())
                .collect();
            if !stale.is_empty() {
                self.app.request_repaint();
                self.app.mark_viewport_nodes_paint_dirty(&stale);
            }
        } else if let Some((all_layers, gpu_layers)) = fallback_layers.take().map(|f| f())
            && (!all_layers.is_empty() || !gpu_layers.is_empty() || skipped_unready)
        {
            renderer.set_composite_layers(all_layers);
            renderer.set_gpu_layers(gpu_layers);
        } else if renderer.has_composite_layers() {
            renderer.set_composite_layers(vec![]);
            renderer.set_gpu_layers(vec![]);
        }

        // Set active viewports so hole-punching only applies to compositor
        // surfaces (`GameViewport`), not inline-painted ones.
        //
        // Installed unconditionally, empty set included. Skipping the call when
        // nothing has a layer left `ACTIVE_VIEWPORTS` at `None`, which
        // `viewport_punches` reads as "no filter — punch everything": the
        // gate was inoperative in exactly the case it exists for, a viewport
        // with no frame behind it (issue #186).
        //
        // Through the app rather than straight into paint: the set is an input
        // to the scene, so a set that differs from the last paint's has to
        // repaint the viewports it differs in (issue #349). `build_scene`
        // otherwise returns the cached scene, with last paint's holes in it —
        // a hole left behind by a surface that unregistered, or none yet for
        // one that has just delivered its first frame.
        //
        // Video frames go in with them: painted inline, in the scene (#1768).
        // Split GPU surfaces and video (#1865) while no `GameViewport` layer is
        // under the UI: the base-layer path and the split path do not compose
        // yet (a base layer under split slices is the design's next step).
        let has_layers = !use_base && (renderer.has_composite_layers() || skipped_unready);
        let mut viewport_frames = crate::render_surface::collect_viewport_frames_by_name(&surfaces);
        let mut split_uploads = crate::split_frame::prepare_split_viewports(
            &mut viewport_frames,
            &mut renderer.split,
            !has_layers,
        );
        self.app
            .install_gpu_viewport_frames(viewport_frames, Some(compositor_viewport_names));
        if use_base {
            split_uploads.extend(base_pending);
        }
        renderer.split.set_base_views(if use_base {
            base_views
        } else {
            Default::default()
        });

        // Build the frame from the document — CPU surfaces paint inline via
        // draw_image(). Only what its damage reaches is encoded and rendered
        // while the renderer's UI texture holds the last frame (#1718). A
        // refused base split builds it again, in full, layered (R1).
        let target = renderer.ui_invalid_reason(transparent);
        let mut fell_back = false;
        let split_change = self.base_fallback.build(
            &mut self.app,
            scale,
            size,
            use_base.then_some(base_viewports),
            !has_layers,
            target,
            || fell_back = true,
        );
        if fell_back {
            if let Some((all_layers, gpu_layers)) = fallback_layers.take().map(|f| f()) {
                renderer.set_composite_layers(all_layers);
                renderer.set_gpu_layers(gpu_layers);
            }
            renderer.split.set_base_views(Default::default());
            split_uploads.clear();
        }
        let frame = self.app.last_gpu_frame();
        let has_layers = renderer.has_composite_layers();

        RinchApp::clear_viewport_frames();
        rinch_dom::paint::set_surface_pixels(None);
        rinch_dom::paint::set_surface_gpu_images(None);
        rinch_dom::paint::set_viewport_split_ids(None);

        // Render to screen
        let present_start = std::time::Instant::now();
        let before: Vec<usize> = renderer.split_surface_ids();
        if let Some(change) = split_change {
            renderer.apply_split(change)?;
        }
        if has_layers {
            renderer.split.clear();
        }
        // After the change: only the videos this frame keeps split upload
        // their frame (#1906 review: a hidden or refused one uploaded too).
        let uploads = {
            let renderer = &mut *renderer;
            renderer
                .split
                .upload_split_frames(&renderer.device, &renderer.queue, split_uploads)
        };

        #[cfg(debug_assertions)]
        {
            let after = renderer.split_surface_ids();
            for id in after.iter().filter(|id| !before.contains(id)) {
                tracing::debug!("rinch: render surface {id} composited split (#1865)");
            }
            for id in before.iter().filter(|id| !after.contains(id)) {
                tracing::debug!("rinch: render surface {id} drawn in the scene (#1865)");
            }
        }
        #[cfg(not(debug_assertions))]
        let _ = before;
        renderer.paint_frame(frame, transparent)?;
        self.app
            .perf_add(rinch_dom::perf::Counter::SplitFrameUploads, uploads);
        self.app.perf_add(
            rinch_dom::perf::Counter::TimePresentNs,
            present_start.elapsed().as_nanos() as u64,
        );

        Ok(())
    }

    /// Get the current window size, in physical pixels.
    ///
    /// This is what the OS surface and the renderer are sized in, and what
    /// pointer coordinates arrive in — `RinchApp::handle_event` divides it by
    /// the scale factor itself. Layout and paint want [`Self::logical_size`].
    ///
    /// Before the window exists there is no surface, so the fallback is the
    /// requested size, which `create_window` passes as a `LogicalSize`; the
    /// scale factor is 1x until then, so the two agree.
    pub(super) fn window_size(&self) -> (u32, u32) {
        self.window
            .as_ref()
            .map(|w| w.inner_size())
            .unwrap_or((self.width, self.height))
    }

    /// Get the current window size in logical (CSS) pixels.
    ///
    /// Only the software debug-screenshot path needs this; the paint paths
    /// derive it from the `window` they already hold.
    #[cfg(feature = "debug")]
    pub(super) fn logical_size(&self) -> (u32, u32) {
        // `self.width`/`self.height` are already the requested *logical* size
        // (`create_window` hands them to winit as a `LogicalSize`), so the
        // no-window fallback must not divide them again.
        self.window
            .as_ref()
            .map(|w| w.logical_size())
            .unwrap_or((self.width.max(1), self.height.max(1)))
    }

    /// Get the current scale factor.
    pub(super) fn scale_factor(&self) -> f64 {
        self.window
            .as_ref()
            .map(|w| w.scale_factor())
            .unwrap_or(1.0)
    }
}

// ── DevTools data extraction helpers ─────────────────────────────────────────

// ── Compositor letterbox backdrop ────────────────────────────────

/// Keep only the radii of corners where the viewport's box actually meets the
/// corner of what clips it, zeroing the rest.
///
/// `RinchApp::viewport_rect_with_radius` reports the **clipping ancestor's**
/// radii, but the viewport is usually only part of that ancestor — in the UI
/// Zoo demo, a video filling the top of a card whose lower half is the controls
/// bar. Applying those radii to the viewport's own box then rounds an edge that
/// runs through the middle of the card, carving a see-through notch out of a
/// hole nothing else fills: exactly the defect this is meant to close. A corner
/// is only a real corner where both of the viewport's edges coincide with the
/// clip's.
///
/// With no clip rect there is no ancestor box to compare against, so the radii
/// are taken as given — the viewport is then the rounded box itself.
///
/// All rects are `(x, y, w, h)` in physical pixels; radii are `[tl, tr, br, bl]`.
#[cfg(any(feature = "gpu", test))]
fn radii_at_shared_corners(
    box_rect: (f32, f32, f32, f32),
    clip_rect: Option<(f32, f32, f32, f32)>,
    radii: [f32; 4],
) -> [f32; 4] {
    let Some((cx, cy, cw, ch)) = clip_rect else {
        return radii;
    };
    // Both rects are truncated from the same layout values, so allow a pixel
    // of slop rather than demanding bit-exact edges.
    let near = |a: f32, b: f32| (a - b).abs() <= 1.0;
    let (bx, by, bw, bh) = box_rect;
    let (left, top) = (near(bx, cx), near(by, cy));
    let (right, bottom) = (near(bx + bw, cx + cw), near(by + bh, cy + ch));
    [
        if left && top { radii[0] } else { 0.0 },
        if right && top { radii[1] } else { 0.0 },
        if right && bottom { radii[2] } else { 0.0 },
        if left && bottom { radii[3] } else { 0.0 },
    ]
}

/// The opaque-black layer that goes **under** a video frame's own layer,
/// covering the viewport's whole box.
///
/// Paint punches its hole over the entire viewport box, but the frame is
/// aspect-fitted inside it, so a source whose aspect ratio differs from the
/// box's leaves letterbox/pillarbox bars that no layer covers — see-through to
/// the desktop on a transparent window (issue #354). This layer covers them,
/// black, which is what a browser paints for `<video>`.
///
/// It has to be the **compositor** that paints the black, not the viewport
/// element: this backend draws these layers *under* the Vello UI, so an opaque
/// element background would hide the frame outright. Pushing a backdrop layer
/// instead needs no change to [`rinch_platform::CompositeLayer`] or the
/// compositor shader — layers are drawn in order, and this one carries the same
/// `border_radius` and `clip_rect` as the frame's layer, so it picks up the
/// viewport's rounded corners and its ancestors' clipping for free.
///
/// The source is a single black texel stretched over `viewport`; every sample
/// of a 1×1 texture is that texel, whatever the filter mode. `Queue::write_texture`
/// places no row-alignment requirement on the upload.
///
/// Compiled under `test` as well as `gpu` so its geometry stays covered by the
/// default `cargo test`, which builds the software backend and cannot reach
/// `paint_gpu` at all.
#[cfg(any(feature = "gpu", test))]
fn black_backdrop_layer(
    viewport: (f32, f32, f32, f32),
    border_radius: [f32; 4],
    clip_rect: Option<(f32, f32, f32, f32)>,
) -> rinch_platform::CompositeLayer {
    rinch_platform::CompositeLayer {
        pixels: vec![0, 0, 0, 255],
        width: 1,
        height: 1,
        viewport,
        border_radius,
        clip_rect,
    }
}

#[cfg(test)]
mod viewport_tests {
    use super::*;

    /// A window that reports a physical surface size and a scale factor.
    struct FakeWindow {
        surface: (u32, u32),
        scale: f64,
    }

    impl PlatformWindow for FakeWindow {
        fn inner_size(&self) -> (u32, u32) {
            self.surface
        }
        fn scale_factor(&self) -> f64 {
            self.scale
        }
        fn request_redraw(&self) {}
        fn set_minimized(&self, _minimized: bool) {}
        fn set_maximized(&self, _maximized: bool) {}
        fn set_visible(&self, _visible: bool) {}
        fn is_maximized(&self) -> bool {
            false
        }
        fn drag_window(&self) -> Result<(), String> {
            Ok(())
        }
        fn set_title(&self, _title: &str) {}
    }

    /// The desktop shell used to hand the document `PlatformWindow::inner_size`
    /// — the *physical* surface size — as its layout viewport, while paint
    /// multiplies every layout coordinate by the same window's scale factor.
    /// On a 1.25x display that laid the page out 1.25x too wide and then drew
    /// it 1.25x larger again, so the rightmost fifth fell outside the surface:
    /// a `flex: 1` child's trailing siblings (a row's confidence dots, the box
    /// after a growing one) were laid out correctly and simply never appeared.
    ///
    /// `run("probe", 460, 300, ..)` on a 1.25x display gets a 575x975 surface;
    /// the viewport is 460 CSS pixels wide, not 575.
    #[test]
    fn layout_viewport_is_logical_not_physical() {
        let window = FakeWindow {
            surface: (575, 975),
            scale: 1.25,
        };
        assert_eq!(
            window.logical_size(),
            (460, 780),
            "layout must get the logical viewport, not the 575x975 surface size"
        );
    }

    /// The page must fit the surface once paint has scaled it — the invariant
    /// the physical-size viewport broke.
    #[test]
    fn scaled_layout_viewport_fits_the_surface() {
        for &(surface, scale) in &[
            ((575u32, 975u32), 1.25f64),
            ((786, 1704), 2.0),
            ((491, 1065), 1.25),
        ] {
            let window = FakeWindow { surface, scale };
            let (vw, vh) = window.logical_size();
            let painted = (vw as f64 * scale, vh as f64 * scale);
            assert!(
                (painted.0 - surface.0 as f64).abs() <= 1.0
                    && (painted.1 - surface.1 as f64).abs() <= 1.0,
                "viewport {vw}x{vh} painted at {scale}x is {painted:?}, \
                 which does not fill the {surface:?} surface"
            );
        }
    }

    /// A 1x display is unaffected, and a nonsensical scale factor must not
    /// collapse the viewport to zero.
    #[test]
    fn layout_viewport_degenerate_scales() {
        assert_eq!(
            FakeWindow {
                surface: (800, 600),
                scale: 1.0
            }
            .logical_size(),
            (800, 600)
        );
        assert_eq!(
            FakeWindow {
                surface: (800, 600),
                scale: 0.0
            }
            .logical_size(),
            (800, 600),
            "a zero scale factor must fall back to 1x, not divide by zero"
        );
        assert_eq!(
            FakeWindow {
                surface: (1, 1),
                scale: 4.0
            }
            .logical_size(),
            (1, 1),
            "the viewport must never round down to zero"
        );
        for bad in [f64::NAN, f64::INFINITY, -2.0] {
            assert_eq!(
                FakeWindow {
                    surface: (800, 600),
                    scale: bad
                }
                .logical_size(),
                (800, 600),
                "a {bad} scale factor must fall back to 1x"
            );
        }
    }

    /// The viewport the shell lays out at and the one `RinchApp::handle_event`
    /// re-lays out at on every `ReRender` / `AboutToWait` must be the *same*
    /// number. They are derived on opposite sides of the shell boundary — the
    /// shell from the window, the app from the physical size it was handed — so
    /// a divergence here silently reintroduces the oversized layout one frame
    /// after mount.
    #[test]
    fn app_and_shell_agree_on_the_layout_viewport() {
        for &(surface, scale) in &[
            ((575u32, 975u32), 1.25f64),
            ((577, 977), 1.25),
            ((786, 1704), 2.0),
            ((800, 600), 1.0),
        ] {
            let window = FakeWindow { surface, scale };
            let (sw, sh) = window.logical_size();
            let (aw, ah) = RinchApp::layout_viewport(surface, scale);
            assert_eq!(
                (sw as f32, sh as f32),
                (aw, ah),
                "shell laid out {surface:?}@{scale}x at {sw}x{sh} but the app \
                 re-lays it out at {aw}x{ah}"
            );
        }
    }
}

// ── #354: the GPU compositor covers the whole punched box ────────────────
//
// Paint cuts the hole over a viewport's entire layout box; the compositor
// aspect-fits the frame inside it. Whatever the compositor does not draw in
// that box is background-removed *and* frame-free — see-through to the desktop
// on a transparent window. The GPU backend therefore draws a black backdrop
// layer under the frame, the way a browser paints `<video>` bars, from the
// compositor rather than from the element (this backend draws the video *under*
// the UI, so an opaque element background would hide the frame outright).

#[cfg(test)]
mod letterbox_backdrop {
    use super::*;

    /// The GPU compositor draws `composite_layers` in order, so the backdrop
    /// must cover the **whole box** (not the fitted rect) and carry the frame
    /// layer's radii and clip, or the bars are unpainted or the corners square.
    ///
    /// `paint_gpu` itself needs a device and a window, so this covers the one
    /// part of it that is pure geometry. It is compiled in every configuration
    /// (see `black_backdrop_layer`'s `cfg`), so the default `cargo test` — which
    /// builds the software backend — still guards the GPU path's shape.
    #[test]
    fn gpu_backdrop_layer_covers_the_whole_box_with_black() {
        let box_viewport = (10.0, 20.0, 200.0, 100.0);
        let radii = [8.0, 8.0, 8.0, 8.0];
        let clip = Some((0.0, 0.0, 400.0, 300.0));
        let layer = black_backdrop_layer(box_viewport, radii, clip);

        assert_eq!(
            layer.viewport, box_viewport,
            "the backdrop covers the punched box, not the fitted rect (#354)"
        );
        assert_eq!(layer.border_radius, radii, "and the viewport's corners");
        assert_eq!(layer.clip_rect, clip, "and its ancestors' clipping");
        assert_eq!(
            (layer.width, layer.height),
            (1, 1),
            "one texel is enough — every sample of a 1x1 texture is that texel"
        );
        assert_eq!(
            layer.pixels,
            vec![0, 0, 0, 255],
            "opaque black, so the bars are covered rather than see-through"
        );
    }

    /// The radii reported for a viewport belong to its *clipping ancestor*, so
    /// they only describe corners the two rects share. The UI Zoo shape — a
    /// video filling the top of a card whose lower half is the controls bar —
    /// is the one that bites: rounding the viewport's bottom edge carves a
    /// notch out of the middle of the card that nothing then fills.
    #[test]
    fn only_corners_shared_with_the_clip_keep_their_radius() {
        let card = Some((0.0, 0.0, 200.0, 300.0));
        // The video occupies the card's top 100px: top corners are real, the
        // bottom edge runs through the middle of the card.
        assert_eq!(
            radii_at_shared_corners((0.0, 0.0, 200.0, 100.0), card, [8.0; 4]),
            [8.0, 8.0, 0.0, 0.0],
            "the bottom corners are not corners — rounding them would cut a \
             see-through notch mid-card (#354)"
        );
        // A viewport filling its clip keeps every corner.
        assert_eq!(
            radii_at_shared_corners((0.0, 0.0, 200.0, 300.0), card, [8.0; 4]),
            [8.0; 4]
        );
        // Floating in the middle of the clip: no corner is shared.
        assert_eq!(
            radii_at_shared_corners((50.0, 50.0, 100.0, 100.0), card, [8.0; 4]),
            [0.0; 4]
        );
        // No clip rect: nothing to compare against, so take the radii as given.
        assert_eq!(
            radii_at_shared_corners((0.0, 0.0, 200.0, 100.0), None, [8.0; 4]),
            [8.0; 4]
        );
        // A pixel of slop, because both rects are truncated from the same
        // layout floats.
        assert_eq!(
            radii_at_shared_corners((1.0, 0.0, 199.0, 300.0), card, [8.0; 4]),
            [8.0; 4]
        );
    }
}
