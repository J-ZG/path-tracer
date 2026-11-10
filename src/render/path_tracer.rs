//! The path tracer shared by the interactive and headless modes.
//!
//! [`PathTracer`] traces samples of a world into a tile and displays the tile onto a render target.
//! The interactive mode ([`super::interactive`]) uses one tile covering the window and displays it to the window's surface render target every frame.
//! The headless mode, native only, moves a tile across the image and displays each finished tile to a texture that is read back and saved.

use super::{world, world_resources::WorldResources};

/// Traces paths of a [`world::World`] uploaded to the GPU into a tile, and displays the tile.
///
/// A tile is a rectangular region of the image being produced, set with [`Self::set_tile()`].
/// Samples accumulate in the tile until accumulation is reset.
pub(super) struct PathTracer {
    /// GPU resources associated with the world; contains bind group 1 of the trace pipeline.
    world_resources: WorldResources,
    /// Parameters of the most recently traced sample.
    parameters: GpuTraceParameters,
    /// Width and height of the storage texture and accumulation buffer in pixels.
    tile_size: [u32; 2],
    /// Holds one [`GpuTraceParameters`] per sample traced in a single [`Self::trace()`] call, each at a dynamic offset that is a multiple of [`Self::parameters_stride`].
    parameters_buffer: wgpu::Buffer,
    /// Distance in bytes between consecutive [`GpuTraceParameters`] in [`Self::parameters_buffer`].
    parameters_stride: u32,

    /// Bind group 0 of the trace pipeline, containing the storage texture, accumulation buffer, and trace parameters of the tile.
    ///
    /// Bind group 1 is instead the bind group in [`Self::world_resources`].
    tile_bind_group: wgpu::BindGroup,
    /// Bind group 0 (the only bind group) of the display pipeline, containing just the storage texture.
    display_bind_group: wgpu::BindGroup,
    /// Layout of [`Self::tile_bind_group`], needed to recreate it when the tile size changes.
    tile_bind_group_layout: wgpu::BindGroupLayout,
    /// Layout of [`Self::display_bind_group`], needed to recreate it when the tile size changes.
    display_bind_group_layout: wgpu::BindGroupLayout,

    /// Runs `trace.slang` (compiled to WGSL by `build.rs`), which traces one sample per pixel into the storage texture and accumulation buffer.
    trace_pipeline: wgpu::ComputePipeline,
    /// Runs `display.wgsl`, which tone maps the storage texture onto a render target.
    display_pipeline: wgpu::RenderPipeline,
}

/// Parameters of one sample to be traced.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuTraceParameters {
    /// Width and height of the full image in pixels.
    image_size: [u32; 2],
    /// Position of the tile's top-left pixel within the full image.
    tile_origin: [u32; 2],
    /// Maximum number of bounces to compute after the first surface hit.
    max_bounces: u32,
    /// Number of samples accumulated in the tile, including this sample; always at least `1` in the shader.
    sample_count: u32,

    // Slang's std140 uniform layout rounds the struct size up to a multiple of 16 bytes
    pad: [u32; 2],
}

impl PathTracer {
    /// Default maximum number of bounces.
    pub(super) const DEFAULT_MAX_BOUNCES: u32 = 10;
    /// Upper limit of the maximum number of bounces.
    pub(super) const MAX_BOUNCES_LIMIT: u32 = 16;

    /// Maximum number of samples that a single [`Self::trace()`] call traces.
    const MAX_SAMPLES_PER_TRACE: u32 = 16;

    /// Create a [`PathTracer`] for a tile of `tile_size` that covers the full image, displaying to render targets of `display_format`.
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        updates: world::WorldUpdates,
        tile_size: [u32; 2],
        display_format: wgpu::TextureFormat,
    ) -> anyhow::Result<Self> {
        Self::validate_tile_size(device, tile_size)?;

        let parameters_size = std::mem::size_of::<GpuTraceParameters>() as u32;
        let parameters_stride =
            parameters_size.next_multiple_of(device.limits().min_uniform_buffer_offset_alignment);
        let parameters_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Trace parameters"),
            size: u64::from(parameters_stride * Self::MAX_SAMPLES_PER_TRACE),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let tile_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Tile bind group layout"),
                entries: &[
                    // storage texture
                    wgpu::BindGroupLayoutEntry {
                        binding: 0, // matches with @binding(0)
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: wgpu::TextureFormat::Rgba32Float,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    // accumulation buffer
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // trace parameters, at a dynamic offset selecting the sample
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: true,
                            min_binding_size: std::num::NonZeroU64::new(u64::from(parameters_size)),
                        },
                        count: None,
                    },
                ],
            });

        let world_bind_group_layout = WorldResources::layout(device);
        let world_resources =
            WorldResources::new(device, queue, world_bind_group_layout.clone(), updates)?;

        let display_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Display bind group layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        // TextureFormat::Rgba32Float is not filterable without FLOAT32_FILTERABLE feature, also meaning that a sampler cannot be used
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                }],
            });

        // while WorldResources::new() already created the world bind group, we need to create the remaining 2 bind groups:
        let (tile_bind_group, display_bind_group) = Self::create_bind_groups(
            device,
            &tile_bind_group_layout,
            &display_bind_group_layout,
            tile_size,
            &parameters_buffer,
        );

        let trace_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Trace pipeline"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Trace pipeline layout"),
                    bind_group_layouts: &[
                        Some(&tile_bind_group_layout),
                        Some(&world_bind_group_layout),
                    ],
                    immediate_size: 0,
                }),
            ),
            // SAFETY: trace.slang only indexes buffers with indices that the Rust side keeps in bounds,
            // its BVH traversal stacks are large enough for the BVH depth, and all of its loops terminate;
            // the runtime checks are skipped since they meaningfully slow down the shader
            module: &unsafe {
                device.create_shader_module_trusted(
                    wgpu::ShaderModuleDescriptor {
                        label: Some("trace.slang"),
                        source: wgpu::ShaderSource::Wgsl(
                            include_str!(concat!(env!("OUT_DIR"), "/trace.wgsl")).into(),
                        ),
                    },
                    wgpu::ShaderRuntimeChecks::unchecked(),
                )
            },
            entry_point: Some("compute_main"), // function name in trace.slang
            compilation_options: Default::default(),
            cache: None,
        });

        let display_shader = device.create_shader_module(wgpu::include_wgsl!("display.wgsl"));
        let display_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Display pipeline"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Display pipeline layout"),
                    bind_group_layouts: &[Some(&display_bind_group_layout)],
                    immediate_size: 0,
                }),
            ),
            vertex: wgpu::VertexState {
                module: &display_shader,
                entry_point: Some("vs_main"), // function name in display.wgsl
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList, // for ex: Vertices 0 1 2 3 4 5 create two triangles 0 1 2 and 3 4 5
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw, // ccw are front-face; right-handed coordinate system
                cull_mode: Some(wgpu::Face::Back),
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &display_shader,
                entry_point: Some("fs_main"), // function name in display.wgsl
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: display_format, // linear gamma
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent::REPLACE,
                        alpha: wgpu::BlendComponent::REPLACE,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        Ok(Self {
            world_resources,
            parameters: GpuTraceParameters {
                image_size: tile_size,
                tile_origin: [0, 0],
                max_bounces: Self::DEFAULT_MAX_BOUNCES,
                sample_count: 0,
                pad: [0; 2],
            },
            tile_size,
            parameters_buffer,
            parameters_stride,
            tile_bind_group,
            display_bind_group,
            tile_bind_group_layout,
            display_bind_group_layout,
            trace_pipeline,
            display_pipeline,
        })
    }

    /// Number of samples accumulated in the tile.
    pub(super) const fn sample_count(&self) -> u32 {
        self.parameters.sample_count
    }

    /// Maximum number of bounces after the first surface hit.
    pub(super) const fn max_bounces(&self) -> u32 {
        self.parameters.max_bounces
    }

    /// Set the maximum number of bounces, resetting accumulation if it changes.
    pub(super) const fn set_max_bounces(&mut self, max_bounces: u32) {
        if self.parameters.max_bounces != max_bounces {
            self.parameters.max_bounces = max_bounces;
            self.parameters.sample_count = 0;
        }
    }

    /// Apply updates given a [`world::WorldUpdates`], resetting accumulation when requested.
    pub(super) fn apply_updates(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        updates: world::WorldUpdates,
    ) {
        if updates.reset_accumulation {
            self.parameters.sample_count = 0;
        }
        self.world_resources.apply_updates(device, queue, updates);
    }

    /// Set the tile to trace: its `origin` and `size` within an image of `image_size`, all in pixels. Resets accumulation.
    ///
    /// Recreates the storage texture and accumulation buffer if `size` changes.
    pub(super) fn set_tile(
        &mut self,
        device: &wgpu::Device,
        image_size: [u32; 2],
        origin: [u32; 2],
        size: [u32; 2],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            origin[0] < image_size[0] && origin[1] < image_size[1],
            "tile origin must be within the image"
        );
        if size != self.tile_size {
            Self::validate_tile_size(device, size)?;
            (self.tile_bind_group, self.display_bind_group) = Self::create_bind_groups(
                device,
                &self.tile_bind_group_layout,
                &self.display_bind_group_layout,
                size,
                &self.parameters_buffer,
            );
            self.tile_size = size;
        }
        self.parameters.image_size = image_size;
        self.parameters.tile_origin = origin;
        self.parameters.sample_count = 0;
        Ok(())
    }

    /// Encode up to `sample_count` samples (at most [`Self::MAX_SAMPLES_PER_TRACE`]) of the tile into `pass`, returning the number of samples encoded.
    pub(super) fn trace(
        &mut self,
        queue: &wgpu::Queue,
        pass: &mut wgpu::ComputePass<'_>,
        sample_count: u32,
    ) -> u32 {
        let sample_count = sample_count.min(Self::MAX_SAMPLES_PER_TRACE);

        // each sample gets its own parameters, since a single write_buffer() is visible to all dispatches of a submission
        let stride = self.parameters_stride as usize;
        let mut parameters = vec![0; stride * sample_count as usize];
        for sample_parameters in parameters.chunks_exact_mut(stride) {
            self.parameters.sample_count += 1;
            sample_parameters[..std::mem::size_of::<GpuTraceParameters>()]
                .copy_from_slice(bytemuck::bytes_of(&self.parameters));
        }
        queue.write_buffer(&self.parameters_buffer, 0, &parameters);

        // only the part of the tile within the image is traced
        let width =
            self.tile_size[0].min(self.parameters.image_size[0] - self.parameters.tile_origin[0]);
        let height =
            self.tile_size[1].min(self.parameters.image_size[1] - self.parameters.tile_origin[1]);

        pass.set_pipeline(&self.trace_pipeline);
        pass.set_bind_group(1, &self.world_resources.bind_group, &[]);
        for sample in 0..sample_count {
            pass.set_bind_group(
                0, // matches with @group(0)
                &self.tile_bind_group,
                &[sample * self.parameters_stride],
            );
            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
        }
        sample_count
    }

    /// Encode drawing the accumulated tile, tone mapped, over the whole render target of `pass`.
    pub(super) fn display(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.display_pipeline);
        pass.set_bind_group(0, &self.display_bind_group, &[]);
        pass.draw(0..3, 0..1); // draw a full-screen triangle
    }

    /// Ensure that the storage texture and accumulation buffer of a tile of `tile_size` fit within the GPU limits.
    fn validate_tile_size(device: &wgpu::Device, tile_size: [u32; 2]) -> anyhow::Result<()> {
        let limits = device.limits();
        anyhow::ensure!(
            tile_size[0] > 0
                && tile_size[1] > 0
                && tile_size[0] <= limits.max_texture_dimension_2d
                && tile_size[1] <= limits.max_texture_dimension_2d,
            "tile size {}x{} must be positive and at most {} pixels in each dimension",
            tile_size[0],
            tile_size[1],
            limits.max_texture_dimension_2d
        );
        let accumulation_size = u64::from(tile_size[0]) * u64::from(tile_size[1]) * 16; // 16 bytes per pixel for vec4<f32>
        anyhow::ensure!(
            accumulation_size <= limits.max_storage_buffer_binding_size
                && accumulation_size <= limits.max_buffer_size,
            "accumulation buffer of tile size {}x{} exceeds GPU limits",
            tile_size[0],
            tile_size[1]
        );
        Ok(())
    }

    /// Create the storage texture and accumulation buffer for a tile of `tile_size`, returning the tile and display bind groups containing them, respectively.
    fn create_bind_groups(
        device: &wgpu::Device,
        tile_layout: &wgpu::BindGroupLayout,
        display_layout: &wgpu::BindGroupLayout,
        tile_size: [u32; 2],
        parameters_buffer: &wgpu::Buffer,
    ) -> (wgpu::BindGroup, wgpu::BindGroup) {
        let storage_texture_view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Storage texture"),
                size: wgpu::Extent3d {
                    width: tile_size[0],
                    height: tile_size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1, // the different "sizes" of the image for rendering at different distances; not necessary here since the texture is full screen and doesn't need to be rendered at different sizes
                sample_count: 1,    // multisampling for anti-aliasing (MSAA); not necessary here
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Float,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[wgpu::TextureFormat::Rgba32Float],
            })
            .create_view(&wgpu::TextureViewDescriptor {
                label: Some("Storage texture view"),
                format: Some(wgpu::TextureFormat::Rgba32Float),
                dimension: Some(wgpu::TextureViewDimension::D2),
                aspect: wgpu::TextureAspect::All,
                base_mip_level: 0,
                mip_level_count: Some(1),
                ..Default::default()
            });

        let accumulation_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Accumulation"), // this is actually used as a texture, but since textures cannot be both read and write, a storage buffer is used instead
            size: u64::from(tile_size[0]) * u64::from(tile_size[1]) * 16, // 16 bytes per pixel for vec4<f32>
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });

        #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
        log::info!("Accumulation buffer size: {}", accumulation_buffer.size());

        (
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Tile bind group"),
                layout: tile_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&storage_texture_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: accumulation_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        // a dynamic offset selects the parameters of one sample
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: parameters_buffer,
                            offset: 0,
                            size: std::num::NonZeroU64::new(
                                std::mem::size_of::<GpuTraceParameters>() as u64,
                            ),
                        }),
                    },
                ],
            }),
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Display bind group"),
                layout: display_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0, // matches with @binding(0)
                    resource: wgpu::BindingResource::TextureView(&storage_texture_view),
                }],
            }),
        )
    }
}
