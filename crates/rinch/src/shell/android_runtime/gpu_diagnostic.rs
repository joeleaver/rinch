//! A bring-up diagnostic, run by hand (`android-gpu`, #372).

pub fn run_tests() {
    log::info!("=== GPU DIAGNOSTIC TESTS ===");

    // 1. Create device
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });

    let adapter = match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    })) {
        Ok(a) => a,
        Err(e) => {
            log::error!("TEST: No adapter: {e}");
            return;
        }
    };

    let info = adapter.get_info();
    log::info!("TEST: Adapter: {} ({:?})", info.name, info.backend);
    log::info!("TEST: Driver: {}", info.driver_info);

    // Every adapter feature but the experimental ones (which need
    // `experimental_features` opted in) and passthrough shaders (an
    // experimental feature until wgpu 28 renamed it; still excluded as
    // before). wgpu 30 adds experimental features the old hand-written
    // list did not name, so take wgpu's own mask (#1955).
    let experimental =
        wgpu::Features::all_experimental_mask() | wgpu::Features::PASSTHROUGH_SHADERS;
    let features = adapter.features() - experimental;

    let (device, queue) =
        match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("gpu-test"),
            required_features: features,
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::default(),
            experimental_features: wgpu::ExperimentalFeatures::default(),
        })) {
            Ok(dq) => dq,
            Err(e) => {
                log::error!("TEST: Device creation failed: {e}");
                return;
            }
        };
    log::info!("TEST 1 PASS: Device created");

    // 2. Test basic compute shader (write to storage buffer)
    test_compute_buffer(&device, &queue);

    // 3. Test storage texture write (what Vello does)
    test_storage_texture(&device, &queue, 256, 256);

    // 4. Test storage texture at larger sizes
    test_storage_texture(&device, &queue, 1008, 2244);

    // 5. Test Vello renderer creation
    test_vello_renderer(&device);

    // 6. Test Vello render to small texture
    test_vello_render(&device, &queue, 256, 256);

    // 7. Test Vello render to full-size texture
    test_vello_render(&device, &queue, 1008, 2244);

    log::info!("=== GPU DIAGNOSTIC TESTS COMPLETE ===");
}

fn test_compute_buffer(device: &wgpu::Device, queue: &wgpu::Queue) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("test compute"),
        source: wgpu::ShaderSource::Wgsl(
            r"
            @group(0) @binding(0) var<storage, read_write> output: array<u32>;
            @compute @workgroup_size(64)
            fn main(@builtin(global_invocation_id) id: vec3<u32>) {
                output[id.x] = id.x + 42u;
            }
        "
            .into(),
        ),
    });

    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 256 * 4,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&bind_group_layout)],
        immediate_size: 0,
    });

    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("test pipeline"),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    });

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: None,
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(4, 1, 1);
    }
    queue.submit(std::iter::once(encoder.finish()));

    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });
    let data = slice.get_mapped_range().expect("the buffer is mapped");
    let vals: &[u32] = bytemuck::cast_slice(&data);
    if vals[0] == 42 && vals[1] == 43 {
        log::info!(
            "TEST 2 PASS: Compute shader works (vals[0]={}, vals[1]={})",
            vals[0],
            vals[1]
        );
    } else {
        log::error!("TEST 2 FAIL: Unexpected values: {:?}", &vals[..4]);
    }
    drop(data);
    buffer.unmap();
}

fn test_storage_texture(device: &wgpu::Device, queue: &wgpu::Queue, w: u32, h: u32) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("test storage texture"),
        source: wgpu::ShaderSource::Wgsl(
            r"
            @group(0) @binding(0) var output: texture_storage_2d<rgba8unorm, write>;
            @compute @workgroup_size(8, 8)
            fn main(@builtin(global_invocation_id) id: vec3<u32>) {
                let dims = textureDimensions(output);
                if id.x < dims.x && id.y < dims.y {
                    textureStore(output, id.xy, vec4<f32>(1.0, 0.0, 0.0, 1.0));
                }
            }
        "
            .into(),
        ),
    });

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: wgpu::TextureFormat::Rgba8Unorm,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        }],
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&bind_group_layout)],
        immediate_size: 0,
    });

    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("test storage tex pipeline"),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(&view),
        }],
    });

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: None,
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
    }
    queue.submit(std::iter::once(encoder.finish()));
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });
    log::info!("TEST 3 PASS: Storage texture {w}x{h} compute completed");
}

fn test_vello_renderer(device: &wgpu::Device) {
    match vello::Renderer::new(
        device,
        vello::RendererOptions {
            antialiasing_support: vello::AaSupport::area_only(),
            use_cpu: false,
            num_init_threads: None,
            pipeline_cache: None,
        },
    ) {
        Ok(_) => log::info!("TEST 5 PASS: Vello GPU renderer created"),
        Err(e) => log::error!("TEST 5 FAIL: Vello renderer creation failed: {e}"),
    }
}

fn test_vello_render(device: &wgpu::Device, queue: &wgpu::Queue, w: u32, h: u32) {
    let mut renderer = match vello::Renderer::new(
        device,
        vello::RendererOptions {
            antialiasing_support: vello::AaSupport::area_only(),
            use_cpu: false,
            num_init_threads: None,
            pipeline_cache: None,
        },
    ) {
        Ok(r) => r,
        Err(e) => {
            log::error!("TEST 6/7 SKIP: Can't create renderer: {e}");
            return;
        }
    };

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    // Create a scene with some content
    let mut scene = vello::Scene::new();
    use vello::kurbo::{Affine, Rect};
    use vello::peniko::{Brush, Color, Fill};
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        &Brush::Solid(Color::from_rgba8(255, 0, 0, 255)),
        None,
        &Rect::new(0.0, 0.0, w as f64, h as f64),
    );

    match renderer.render_to_texture(
        device,
        queue,
        &scene,
        &view,
        &vello::RenderParams {
            base_color: peniko::Color::from_rgba8(255, 255, 255, 255),
            width: w,
            height: h,
            antialiasing_method: vello::AaConfig::Area,
        },
    ) {
        Ok(_) => {
            log::info!("TEST 6/7: Vello submit OK for {w}x{h}, polling...");
            match device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            }) {
                Ok(_) => log::info!("TEST 6/7 PASS: poll OK for {w}x{h}"),
                Err(e) => log::error!("TEST 6/7 FAIL: poll failed for {w}x{h}: {e}"),
            }
        }
        Err(e) => log::error!("TEST 6/7 FAIL: Vello render_to_texture {w}x{h} failed: {e}"),
    }
}
