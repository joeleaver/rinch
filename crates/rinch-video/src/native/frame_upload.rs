//! CPU pixel data → wgpu texture upload.
//!
//! Manages reusable wgpu textures for uploading decoded video frames.
//! Each video layer gets its own texture slot so multiple players can
//! be composited in the same render pass without overwriting each other.

use wgpu::{
    Device, Extent3d, Queue, Texture, TextureDescriptor, TextureDimension, TextureFormat,
    TextureUsages,
};

/// A single texture slot (reused across frames if dimensions match).
struct TextureSlot {
    texture: Texture,
    width: u32,
    height: u32,
}

/// Manages wgpu textures for video frame uploads.
///
/// Maintains a pool of textures, one per video layer index.
/// Each slot's texture is reused across frames. If the video resolution
/// changes, the texture is recreated.
pub struct FrameUploader {
    slots: Vec<TextureSlot>,
}

impl Default for FrameUploader {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameUploader {
    /// Create a new frame uploader (no textures allocated yet).
    pub fn new() -> Self {
        Self { slots: Vec::new() }
    }

    /// Upload RGBA pixel data to a GPU texture at the given slot index.
    ///
    /// Each slot maintains its own texture so multiple video layers can
    /// coexist without overwriting each other. Creates or resizes the
    /// texture if dimensions changed.
    ///
    /// Returns a reference to the texture at that slot.
    pub fn upload(
        &mut self,
        index: usize,
        device: &Device,
        queue: &Queue,
        pixels: &[u8],
        width: u32,
        height: u32,
    ) -> &Texture {
        // Grow the slot pool if needed
        while self.slots.len() <= index {
            // Create a 1x1 placeholder (will be resized on first real upload)
            self.slots.push(TextureSlot {
                texture: Self::create_texture(device, 1, 1),
                width: 1,
                height: 1,
            });
        }

        let slot = &mut self.slots[index];

        // Recreate texture if dimensions changed
        if slot.width != width || slot.height != height {
            slot.texture = Self::create_texture(device, width, height);
            slot.width = width;
            slot.height = height;
        }

        // Write pixel data to the texture
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &slot.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        &slot.texture
    }

    fn create_texture(device: &Device, width: u32, height: u32) -> Texture {
        device.create_texture(&TextureDescriptor {
            label: Some("rinch video frame"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }
}
