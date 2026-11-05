//! GPU textures the host owns, shown by elements of an embedded tree (issue
//! #2082), re-exported through [`crate::embed`].
//!
//! A game draws a picture into a texture every frame — a minimap, a camera
//! monitor, a portrait from a second camera — on the device it shares with
//! rinch, and an element of its in-game UI shows it. The host registers the
//! texture with the [`RinchContext`](crate::embed::RinchContext) under a name
//! ([`register_texture`](crate::embed::RinchContext::register_texture)); an
//! element with `data-host-texture="<name>"` (the [`HostTexture`] component
//! renders one) draws it as its content, laid out, clipped and transformed as
//! any element is; and [`RinchOverlayRenderer::render`](crate::embed::RinchOverlayRenderer::render)
//! hands the texture to Vello, which copies it into its image atlas on the
//! GPU. Nothing is read back to the CPU and the tree is not rebuilt.
//!
//! **What a new frame costs.** A texture is drawn in the scene as an image
//! that names it, so the scene does not change when the texture's contents
//! do: each render re-copies the texture into Vello's atlas (one GPU copy of
//! its size; [`TextureRefresh::OnChange`] skips it for a texture that has not
//! changed). Registering another texture of the **same size** under the same
//! name — the double-buffered swap — points the same image at it: still no
//! repaint. Only a new name, a new size, another alpha mode, or removing a
//! name repaints the scene, once.
//!
//! **Formats.** `Rgba8Unorm` with `COPY_SRC` is copied as it is. Every other
//! accepted format (`Rgba8UnormSrgb`, `Bgra8Unorm`, `Bgra8UnormSrgb`, and
//! `Rgba8Unorm` without `COPY_SRC`) needs `TEXTURE_BINDING`, and is converted
//! by one render pass per refresh into an `Rgba8Unorm` texture the renderer
//! owns (its size, 4 bytes a texel): the bytes are kept (an sRGB format is
//! decoded and encoded again), only their order changes. Vello treats the
//! bytes as sRGB-encoded colour, as it does every image's.

use std::collections::HashMap;

use rinch_core::dom::{NodeHandle, RenderScope};
use vello::peniko::{ImageAlphaType, ImageData};
use wgpu::{Device, Queue, Texture, TextureFormat, TextureUsages};

/// How a host texture's colour values carry alpha.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TextureAlpha {
    /// Colour multiplied by alpha: what a GPU renderer's blending usually
    /// leaves in a render target, and a WebGL canvas's default
    /// (`premultipliedAlpha: true`). For an opaque texture the two are the
    /// same.
    #[default]
    Premultiplied,
    /// Colour not multiplied by alpha.
    Straight,
}

/// When the renderer copies a host texture's contents again.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TextureRefresh {
    /// At every render: the host may have drawn into it since the last one.
    #[default]
    EveryRender,
    /// Only when the host says it changed
    /// ([`texture_changed`](crate::embed::RinchContext::texture_changed)) or
    /// registers it again: for a picture that changes now and then.
    OnChange,
}

/// How a host texture is drawn: see [`RinchContext::register_texture_with`](crate::embed::RinchContext::register_texture_with).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct HostTextureOptions {
    /// How its colour carries alpha. Default [`TextureAlpha::Premultiplied`].
    pub alpha: TextureAlpha,
    /// When it is copied again. Default [`TextureRefresh::EveryRender`].
    pub refresh: TextureRefresh,
}

impl HostTextureOptions {
    /// The defaults: premultiplied, copied at every render.
    pub fn new() -> Self {
        Self::default()
    }

    /// How the texture's colour carries alpha.
    pub fn alpha(mut self, alpha: TextureAlpha) -> Self {
        self.alpha = alpha;
        self
    }

    /// When the texture is copied again.
    pub fn refresh(mut self, refresh: TextureRefresh) -> Self {
        self.refresh = refresh;
        self
    }
}

/// Why [`RinchContext::register_texture`](crate::embed::RinchContext::register_texture)
/// refused a texture. Nothing was registered, and a texture already under the
/// name stays.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HostTextureError {
    /// Not one of `Rgba8Unorm`, `Rgba8UnormSrgb`, `Bgra8Unorm`,
    /// `Bgra8UnormSrgb`. Convert other formats (a float HDR target, say)
    /// into one of these yourself: how to tone-map them is the host's call.
    UnsupportedFormat(TextureFormat),
    /// Not a single-layer, single-sample 2D texture.
    UnsupportedShape,
    /// The texture lacks the usage its format needs: `COPY_SRC` (or
    /// `TEXTURE_BINDING`) for `Rgba8Unorm`, `TEXTURE_BINDING` for the
    /// others.
    MissingUsage(TextureUsages),
}

impl std::fmt::Display for HostTextureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedFormat(fmt) => write!(
                f,
                "host texture format {fmt:?} is not supported (Rgba8Unorm, Rgba8UnormSrgb, \
                 Bgra8Unorm, Bgra8UnormSrgb are)"
            ),
            Self::UnsupportedShape => {
                f.write_str("a host texture must be a single-layer, single-sample 2D texture")
            }
            Self::MissingUsage(u) => write!(f, "the host texture needs the usage {u:?}"),
        }
    }
}

impl std::error::Error for HostTextureError {}

/// How a texture reaches Vello's atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
    /// Copied as it is (`Rgba8Unorm` with `COPY_SRC`).
    Direct,
    /// Through the conversion pass, written through a view of this format.
    Convert(TextureFormat),
}

fn route(texture: &Texture) -> Result<Route, HostTextureError> {
    if texture.dimension() != wgpu::TextureDimension::D2
        || texture.depth_or_array_layers() != 1
        || texture.sample_count() != 1
    {
        return Err(HostTextureError::UnsupportedShape);
    }
    let usage = texture.usage();
    let sampled = |target| {
        if usage.contains(TextureUsages::TEXTURE_BINDING) {
            Ok(Route::Convert(target))
        } else {
            Err(HostTextureError::MissingUsage(
                TextureUsages::TEXTURE_BINDING,
            ))
        }
    };
    match texture.format() {
        TextureFormat::Rgba8Unorm if usage.contains(TextureUsages::COPY_SRC) => Ok(Route::Direct),
        TextureFormat::Rgba8Unorm | TextureFormat::Bgra8Unorm => sampled(TextureFormat::Rgba8Unorm)
            .map_err(|_| {
                HostTextureError::MissingUsage(if texture.format() == TextureFormat::Rgba8Unorm {
                    TextureUsages::COPY_SRC
                } else {
                    TextureUsages::TEXTURE_BINDING
                })
            }),
        TextureFormat::Rgba8UnormSrgb | TextureFormat::Bgra8UnormSrgb => {
            sampled(TextureFormat::Rgba8UnormSrgb)
        }
        other => Err(HostTextureError::UnsupportedFormat(other)),
    }
}

/// One texture a context holds, by name.
struct Entry {
    texture: Texture,
    options: HostTextureOptions,
    route: Route,
    /// Bumped by [`HostTextures::changed`] and by registering again: a
    /// renderer holding an older one copies the texture again.
    version: u64,
}

/// The textures a [`RinchContext`](crate::embed::RinchContext) holds, by name.
#[derive(Default)]
pub(crate) struct HostTextures {
    by_name: HashMap<String, Entry>,
    next_version: u64,
}

impl HostTextures {
    pub(crate) fn register(
        &mut self,
        name: String,
        texture: &Texture,
        options: HostTextureOptions,
    ) -> Result<(), HostTextureError> {
        let route = route(texture)?;
        self.next_version += 1;
        self.by_name.insert(
            name,
            Entry {
                texture: texture.clone(),
                options,
                route,
                version: self.next_version,
            },
        );
        Ok(())
    }

    pub(crate) fn unregister(&mut self, name: &str) -> bool {
        self.by_name.remove(name).is_some()
    }

    pub(crate) fn changed(&mut self, name: &str) -> bool {
        self.next_version += 1;
        let v = self.next_version;
        self.by_name.get_mut(name).map(|e| e.version = v).is_some()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

/// One texture registered with a renderer.
struct Registered {
    /// The host's texture this image shows now.
    source: Texture,
    /// The image the scene draws (its blob id is the registration's).
    image: ImageData,
    /// For [`Route::Convert`]: the texture the pass writes, which Vello
    /// copies.
    converted: Option<Texture>,
    route: Route,
    /// The entry's version last copied.
    version: u64,
}

/// What a Vello renderer holds of a context's host textures: the image each
/// name is drawn as, registered with that renderer. One per
/// `vello::Renderer`. [`RinchOverlayRenderer`](crate::embed::RinchOverlayRenderer)
/// keeps one; a host that renders [`RinchContext::scene_with_textures`](crate::embed::RinchContext::scene_with_textures)
/// with its own renderer keeps one beside it.
#[derive(Default)]
pub struct HostTextureImages {
    by_name: HashMap<String, Registered>,
    converter: Option<Converter>,
    /// How many textures were registered with the renderer, ever.
    registrations: u64,
}

impl std::fmt::Debug for HostTextureImages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostTextureImages")
            .field("names", &self.by_name.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl HostTextureImages {
    /// None registered yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many textures this has registered with its renderer so far (a
    /// re-registration counts again). A test hook, not a stable API.
    #[doc(hidden)]
    pub fn registration_count(&self) -> u64 {
        self.registrations
    }

    /// Bring `renderer`'s registrations up to `textures` and copy (or
    /// convert) what is due this render. Returns each name's image and
    /// whether any image the scene draws changed (a name came or went, or a
    /// texture changed size or alpha): the scene must then be painted again.
    pub(crate) fn sync(
        &mut self,
        device: &Device,
        queue: &Queue,
        renderer: &mut vello::Renderer,
        textures: &HostTextures,
    ) -> (HashMap<String, ImageData>, bool) {
        let mut changed = false;
        self.by_name.retain(|name, reg| {
            let keep = textures.by_name.contains_key(name);
            if !keep {
                renderer.unregister_texture(reg.image.clone());
                changed = true;
            }
            keep
        });
        let mut encoder: Option<wgpu::CommandEncoder> = None;
        for (name, entry) in &textures.by_name {
            let (w, h) = (entry.texture.width(), entry.texture.height());
            let alpha = match entry.options.alpha {
                TextureAlpha::Premultiplied => ImageAlphaType::AlphaPremultiplied,
                TextureAlpha::Straight => ImageAlphaType::Alpha,
            };
            let reuse = self.by_name.get(name).is_some_and(|r| {
                r.image.width == w
                    && r.image.height == h
                    && r.image.alpha_type == alpha
                    && r.route == entry.route
            });
            if !reuse {
                if let Some(old) = self.by_name.remove(name) {
                    renderer.unregister_texture(old.image);
                }
                let converted = match entry.route {
                    Route::Direct => None,
                    Route::Convert(_) => Some(converted_texture(device, w, h)),
                };
                self.registrations += 1;
                let mut image =
                    renderer.register_texture(converted.clone().unwrap_or(entry.texture.clone()));
                image.alpha_type = alpha;
                self.by_name.insert(
                    name.clone(),
                    Registered {
                        source: entry.texture.clone(),
                        image,
                        converted,
                        route: entry.route,
                        // Copy it below.
                        version: 0,
                    },
                );
                changed = true;
            }
            let reg = self.by_name.get_mut(name).expect("inserted above");
            let swapped = reg.source != entry.texture;
            let due = swapped
                || reg.version != entry.version
                || entry.options.refresh == TextureRefresh::EveryRender;
            if !due {
                continue;
            }
            reg.version = entry.version;
            if swapped {
                reg.source = entry.texture.clone();
            }
            match (entry.route, &reg.converted) {
                (Route::Convert(target_format), Some(target)) => {
                    let converter = self.converter.get_or_insert_with(|| Converter::new(device));
                    let encoder = encoder.get_or_insert_with(|| {
                        device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("rinch host texture conversion"),
                        })
                    });
                    converter.convert(device, encoder, &entry.texture, target, target_format);
                    renderer.mark_override_image_dirty(&reg.image);
                }
                _ if swapped => {
                    // The same image, now copied from the new texture: the
                    // scene that draws it is unchanged.
                    renderer.override_image(
                        &reg.image,
                        Some(wgpu::TexelCopyTextureInfoBase {
                            texture: entry.texture.clone(),
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        }),
                    );
                }
                _ => renderer.mark_override_image_dirty(&reg.image),
            }
        }
        if let Some(encoder) = encoder {
            queue.submit(Some(encoder.finish()));
        }
        let images = self
            .by_name
            .iter()
            .map(|(name, reg)| (name.clone(), reg.image.clone()))
            .collect();
        (images, changed)
    }

    /// Unregister every texture from `renderer`, for a context that has
    /// none any more.
    pub(crate) fn clear(&mut self, renderer: &mut vello::Renderer) -> bool {
        let had = !self.by_name.is_empty();
        for (_, reg) in self.by_name.drain() {
            renderer.unregister_texture(reg.image);
        }
        had
    }
}

/// The `Rgba8Unorm` texture a converted host texture is written into, which
/// Vello copies from. Viewable as `Rgba8UnormSrgb` too, for an sRGB source.
fn converted_texture(device: &Device, width: u32, height: u32) -> Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("rinch host texture (converted)"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
        view_formats: &[TextureFormat::Rgba8UnormSrgb],
    })
}

const CONVERT_WGSL: &str = r#"
@group(0) @binding(0) var src: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // One triangle covering the target.
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    // Texel for texel: the target is the source's size.
    return textureLoad(src, vec2<i32>(pos.xy), 0);
}
"#;

/// The conversion pass: texel for texel, from any accepted format into the
/// `Rgba8Unorm` texture Vello copies.
struct Converter {
    layout: wgpu::BindGroupLayout,
    /// Writing `Rgba8Unorm` (the bytes as loaded) and `Rgba8UnormSrgb` (an
    /// sRGB source, decoded by the load and encoded again by the write).
    unorm: wgpu::RenderPipeline,
    srgb: wgpu::RenderPipeline,
}

impl Converter {
    fn new(device: &Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rinch host texture conversion"),
            source: wgpu::ShaderSource::Wgsl(CONVERT_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rinch host texture conversion"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rinch host texture conversion"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("rinch host texture conversion"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        Self {
            unorm: pipeline(TextureFormat::Rgba8Unorm),
            srgb: pipeline(TextureFormat::Rgba8UnormSrgb),
            layout,
        }
    }

    fn convert(
        &self,
        device: &Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &Texture,
        target: &Texture,
        target_format: TextureFormat,
    ) {
        let src = source.create_view(&wgpu::TextureViewDescriptor {
            mip_level_count: Some(1),
            ..Default::default()
        });
        let dst = target.create_view(&wgpu::TextureViewDescriptor {
            format: Some(target_format),
            ..Default::default()
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rinch host texture conversion"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&src),
            }],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("rinch host texture conversion"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &dst,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(if target_format == TextureFormat::Rgba8UnormSrgb {
            &self.srgb
        } else {
            &self.unorm
        });
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// An element that shows the host texture registered under `name` (#2082):
/// a `div` with `data-host-texture="<name>"`. See [`HostTexture`].
pub fn host_texture(__scope: &mut RenderScope, name: &str) -> NodeHandle {
    let div = __scope.create_element("div");
    div.set_attribute("class", "rinch-host-texture");
    div.set_attribute("data-host-texture", name);
    div
}

/// Shows a GPU texture the host registered with the context under `name`
/// ([`RinchContext::register_texture`](crate::embed::RinchContext::register_texture)),
/// updated every frame without a tree rebuild (#2082).
///
/// It is a `div`: sized by CSS (it has no size of its own; give it a width
/// and height, or a flex share), with the texture as its content, in its
/// content box, fitted by `object-fit` (`fill`, stretched, unless you set
/// `contain`, `cover`, `none` or `scale-down`), inside its border and
/// rounded by its `border-radius`, under its children. Its background shows
/// where the texture does not reach, and wherever no texture is registered
/// under the name. Any element with a `data-host-texture` attribute does the
/// same.
///
/// Drawn by [`RinchOverlayRenderer::render`](crate::embed::RinchOverlayRenderer::render)
/// and [`RinchContext::scene_with_textures`](crate::embed::RinchContext::scene_with_textures);
/// [`RinchContext::scene`](crate::embed::RinchContext::scene) draws none (the
/// renderer that will render that scene is the host's alone).
///
/// ```ignore
/// rsx! {
///     div { class: "hud",
///         HostTexture { name: "minimap", style: "width: 200px; height: 200px; border-radius: 8px;" }
///     }
/// }
/// ```
#[derive(Debug, Default)]
pub struct HostTexture {
    /// The name the texture is registered under.
    pub name: String,
}

impl rinch_core::Component for HostTexture {
    fn render(&self, scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        host_texture(scope, &self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_default_to_premultiplied_and_every_render() {
        let o = HostTextureOptions::new();
        assert_eq!(o.alpha, TextureAlpha::Premultiplied);
        assert_eq!(o.refresh, TextureRefresh::EveryRender);
        let o = o
            .alpha(TextureAlpha::Straight)
            .refresh(TextureRefresh::OnChange);
        assert_eq!(
            (o.alpha, o.refresh),
            (TextureAlpha::Straight, TextureRefresh::OnChange)
        );
    }
}
