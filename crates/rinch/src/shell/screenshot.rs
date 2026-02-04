//! Screenshot capture via wgpu texture readback.

use wgpu::{
    BufferDescriptor, BufferUsages, CommandEncoderDescriptor, Device, Extent3d, Queue, Texture,
    TextureFormat,
};

/// Capture a screenshot from a GPU texture, returning raw RGBA bytes.
///
/// The caller is responsible for encoding to PNG.
pub fn capture_texture_rgba(
    device: &Device,
    queue: &Queue,
    texture: &Texture,
    width: u32,
    height: u32,
    format: TextureFormat,
) -> Result<Vec<u8>, String> {
    let bytes_per_pixel = 4u32;
    // wgpu requires rows to be aligned to 256 bytes
    let unpadded_row = width * bytes_per_pixel;
    let padded_row = (unpadded_row + 255) & !255;
    let buffer_size = (padded_row * height) as u64;

    let staging_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("screenshot staging buffer"),
        size: buffer_size,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("screenshot encoder"),
    });

    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging_buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row),
                rows_per_image: Some(height),
            },
        },
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );

    queue.submit(Some(encoder.finish()));

    // Map the buffer and read back
    let slice = staging_buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| format!("GPU poll failed: {:?}", e))?;
    rx.recv()
        .map_err(|e| format!("Channel recv failed: {}", e))?
        .map_err(|e| format!("Buffer mapping failed: {:?}", e))?;

    let data = slice.get_mapped_range();

    // Remove row padding and handle format conversion
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        let offset = (row * padded_row) as usize;
        let row_data = &data[offset..offset + (unpadded_row as usize)];

        match format {
            TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb => {
                // BGRA -> RGBA
                for pixel in row_data.chunks_exact(4) {
                    rgba.push(pixel[2]); // R
                    rgba.push(pixel[1]); // G
                    rgba.push(pixel[0]); // B
                    rgba.push(pixel[3]); // A
                }
            }
            _ => {
                // Assume RGBA
                rgba.extend_from_slice(row_data);
            }
        }
    }

    drop(data);
    staging_buffer.unmap();

    Ok(rgba)
}

/// Encode raw RGBA pixel data to PNG bytes.
pub fn encode_png(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut png_data = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_data, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("Failed to write PNG header");
        writer
            .write_image_data(rgba)
            .expect("Failed to write PNG data");
    }
    png_data
}
