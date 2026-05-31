//! Define the [`State`] struct which encapsulates all the GPU resources, scene data, and application state needed for rendering.
//!
//! Includes methods for initializing these resources and handling events from the window.

/// The name of the glTF file to load from the [`assets/`](https://github.com/0dpe/path-tracer/tree/main/assets) directory to render.
///
/// Can be exported from Blender.
/// ![Export glTF from Blender 1](https://raw.githubusercontent.com/0dpe/path-tracer/main/doc/blender_export_gltf_1.jpg)
/// ![Export glTF from Blender 2](https://raw.githubusercontent.com/0dpe/path-tracer/main/doc/blender_export_gltf_2.jpg)
pub const GLTF: &str = "cornell_custom";

use super::scene;

/// Encapsulates all the GPU resources, scene data, and applaction state.
pub struct State {
    /// The surface represents the platform-specific window or canvas that will be rendered onto.
    surface: wgpu::Surface<'static>,
    /// Describes the configuration of the surface, including format, size, etc. Needed for configuring the surface and recreating it on resize.
    surface_config: wgpu::SurfaceConfiguration,
    /// Represents a logical connection to the GPU and is used to create all other GPU resources.
    device: wgpu::Device,
    /// Used to execute recorded [`wgpu::CommandBuffer`] objects.
    queue: wgpu::Queue,

    /// Defines how the shader will sample from textures; used in both compute and render pipelines since they have the same filtering requirements.
    sampler: wgpu::Sampler,

    // group 0 (dynamic)
    /// Bind group for the compute shader, containing the storage texture and accumulation buffer that are written to by the compute shader.
    compute_texture_bind_group: wgpu::BindGroup,
    /// Bind group for the render shader, containing the storage texture and sampler that are read from by the fragment shader.
    render_texture_bind_group: wgpu::BindGroup,
    /// Bind group layout for [`Self::compute_texture_bind_group`], needed in [`State`] to recreate the bind group on window resizes.
    compute_texture_bind_group_layout: wgpu::BindGroupLayout,
    /// Bind group layout for [`Self::render_texture_bind_group`], needed in [`State`] to recreate the bind group on window resizes.
    render_texture_bind_group_layout: wgpu::BindGroupLayout,

    // group 1 (static)
    /// Bind group for the compute shader, containing the scene data. This bind group is static since the scene data doesn't change at runtime.
    compute_scene_bind_group: wgpu::BindGroup,
    /// Buffer for the camera data, which is updated every frame. This is a separate buffer from the scene data buffers since it's updated at a different frequency.
    camera_buffer: wgpu::Buffer,

    /// Runs the compute shader to perform path tracing calculations and write to the storage texture.
    compute_pipeline: wgpu::ComputePipeline,
    /// Runs the vertex and fragment shaders to render a full-screen triangle that samples from the storage texture and outputs to the surface.
    render_pipeline: wgpu::RenderPipeline,

    /// Contains all data about the 3D scene.
    scene: scene::Scene,

    /// Set of currently pressed keys, used for camera movement.
    pressed_keys: std::collections::HashSet<winit::keyboard::KeyCode>,
    /// Tracks the current cursor grab mode, used for controlling whether the cursor is captured for camera rotation or not.
    cursor_grab: winit::window::CursorGrabMode,

    /// The time of the last update, used for calculating delta time (dt) for movement speed scaling and tracking FPS.
    /// Using [`web_time::Instant`] which is a cross-platform abstraction over [`std::time::Instant`] that works on both native and web targets.
    last_instant: web_time::Instant,

    /// FPS tracking.
    fps_timer: f32,
    /// FPS tracking.
    fps_counter: u32,
    /// FPS tracking.
    current_fps: u32, // the stored FPS value; not necessary to be stored in the struct right now, but for display FPS in the UI, having it here would make that possible

    /// Tracks the number of frames rendered since the last camera movement or window resize.
    /// Used for accumulation in the compute shader, where the shader can use this count to determine how many frames have been accumulated so far and adjust its calculations accordingly.
    frame_count: u32,

    /// Used for performance analysis.
    #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
    profiler: wgpu_profiler::GpuProfiler,

    /// The winit window.
    pub window: std::sync::Arc<winit::window::Window>,
}

/// Create or recreate the storage texture, accumulation buffer, and their corresponding bind groups for both compute and render pipelines.
/// Called on window resizes since the surface size changes.
/// Returns the compute and render bind groups, respectively, as a tuple.
// private helper function, not a method inside the impl because new() calls it
// called in both State's new() and resize()
fn create_texture_bind_groups(
    device: &wgpu::Device,
    sampler: &wgpu::Sampler,
    compute_texture_layout: &wgpu::BindGroupLayout,
    render_texture_layout: &wgpu::BindGroupLayout,
    texture_size: &(u32, u32),
) -> (wgpu::BindGroup, wgpu::BindGroup) {
    let storage_texture_view = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("Storage texture"),
            size: wgpu::Extent3d {
                width: texture_size.0,
                height: texture_size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1, // the different "sizes" of the image for rendering at different distances; not necessary here since the texture is full screen and doesn't need to be rendered at different sizes
            sample_count: 1,    // multisampling for anti-aliasing (MSAA); not necessary here
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float, // linear gamma
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[wgpu::TextureFormat::Rgba16Float],
        })
        .create_view(&wgpu::TextureViewDescriptor {
            label: Some("Storage texture view"),
            format: Some(wgpu::TextureFormat::Rgba16Float),
            dimension: Some(wgpu::TextureViewDimension::D2),
            aspect: wgpu::TextureAspect::All,
            base_mip_level: 0,
            mip_level_count: Some(1),
            ..Default::default()
        });

    // create the accumulation buffer (16 bytes per pixel for vec4<f32>)
    // this is actually used as a texture, but since textures cannot be both read and write, a storage buffer is used instead
    let accumulation_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Accumulation buffer"),
        size: wgpu::BufferAddress::from(texture_size.0 * texture_size.1 * 16),
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });

    (
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Compute texture bind group"),
            layout: compute_texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0, // matches with @binding(0)
                    resource: wgpu::BindingResource::TextureView(&storage_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1, // matches with @binding(1)
                    resource: wgpu::BindingResource::Buffer(
                        accumulation_buffer.as_entire_buffer_binding(),
                    ),
                },
            ],
        }),
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Render texture bind group"),
            layout: render_texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0, // matches with @binding(0)
                    resource: wgpu::BindingResource::TextureView(&storage_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1, // matches with @binding(1)
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        }),
    )
}

impl State {
    /// Asynchronously create a new [`State`] by initializing all GPU resources, loading the scene, and preparing the bind groups and pipelines.
    pub async fn new(
        window: std::sync::Arc<winit::window::Window>,
        display_handle: winit::event_loop::OwnedDisplayHandle,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        log::info!("Called: new");

        // create instance, the context for all other wgpu objects
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            #[cfg(not(target_arch = "wasm32"))]
            backends: wgpu::Backends::PRIMARY, // Vulkan, Metal, DX12, WebGPU (no WebGL)
            #[cfg(target_arch = "wasm32")]
            backends: wgpu::Backends::BROWSER_WEBGPU, // just WebGPU
            ..wgpu::InstanceDescriptor::new_with_display_handle(Box::new(display_handle))
        });

        // create surface, which targets the given winit window
        let surface = instance.create_surface(std::sync::Arc::clone(&window))?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await?;
        log::info!("Using adapter: {}", adapter.get_info().name); // doesn't log the name on web for some reason, but logs fine on native
        let supported_limits = adapter.limits(); // get the maximum limits the physical hardware supports
        log::info!(
            "Max storage buffer binding size: {} MiB",
            supported_limits.max_storage_buffer_binding_size as f32 / 1024.0 / 1024.0
        );
        // log::info!(
        //     "Max texture dimension 2D: {}",
        //     supported_limits.max_texture_dimension_2d
        // );

        #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
        let required_features =
            adapter.features() & wgpu_profiler::GpuProfiler::ALL_WGPU_TIMER_FEATURES; // for wgpu_profiler
        #[cfg(not(all(feature = "testing", not(target_arch = "wasm32"))))]
        let required_features = wgpu::Features::empty();

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Device"),
                required_features,
                required_limits: wgpu::Limits {
                    max_storage_buffer_binding_size: supported_limits
                        .max_storage_buffer_binding_size, // the default is 128 MiB, which is too small for millions of triangles
                    max_buffer_size: supported_limits.max_buffer_size,
                    ..wgpu::Limits::default()
                },
                experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await?;

        #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
        tracy_client::Client::start();

        #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
        let profiler = wgpu_profiler::GpuProfiler::new_with_tracy_client(
            wgpu_profiler::GpuProfilerSettings::default(),
            adapter.get_info().backend,
            &device,
            &queue,
        )?;

        // see which TextureFormat's are supported by the surface
        // Bgra8Unorm and Bgra8UnormSrgb should be guaranteed, but on web, Bgra8UnormSrgb isn't supported it seems
        // for example, on Windows native, [Bgra8UnormSrgb, Rgba8UnormSrgb, Bgra8Unorm, Rgba8Unorm, Rgba16Float, Rgb10a2Unorm] are supported, but with Chrome or Firefox, only [Bgra8Unorm, Rgba8Unorm, Rgba16Float] are supported
        log::info!(
            "Surface formats: {:?}",
            surface.get_capabilities(&adapter).formats
        );

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: wgpu::TextureFormat::Bgra8Unorm, // linear gamma. Bgra8Unorm should be a guaranteed supported format
            // sometimes on web initial page load, the canvas can have window.inner_size().width of 0
            // 0 length or width causes surface.configure() to panic
            // so, .max(1) makes sure that width and height are never less than 1
            width: window.inner_size().width.max(1),
            height: window.inner_size().height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![wgpu::TextureFormat::Bgra8Unorm], // linear gamma
        };

        // render() only works when the surface is configured
        // render() is often called right after new(), and resize() isn't called unless a resize happens, so configuring surface here is necessary
        surface.configure(&device, &surface_config);

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        // group 0: the storage texture for compute shader and the accumulation buffer
        let compute_texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Compute texture bind group layout"),
                entries: &[
                    // storage texture
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,                              // matches with @binding(0)
                        visibility: wgpu::ShaderStages::COMPUTE, // which stages can see this binding
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: wgpu::TextureFormat::Rgba16Float, // linear gamma
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
                ],
            });

        // group 0: the storage texture and sampler for render (fragment) shader
        let render_texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Render texture bind group layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,                               // matches with @binding(0)
                        visibility: wgpu::ShaderStages::FRAGMENT, // which stages can see this binding
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,                               // matches with @binding(1)
                        visibility: wgpu::ShaderStages::FRAGMENT, // which stages can see this binding
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        // group 1: the scene data buffers for compute shader
        let compute_scene_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Scene bind group layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0, // triangle geometry
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1, // bvh
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2, // triangle attributes
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3, // materials
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4, // texture atlas array
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 5, // atlas sampler
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 6, // camera
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: std::num::NonZeroU64::new(std::mem::size_of::<
                                scene::GpuCamera,
                            >(
                            )
                                as u64), // not necessary, but is an optimizaton; allows wgpu to skip per-draw validation
                        },
                        count: None,
                    },
                ],
            });

        // create a shader module from vertex_fragment.wgsl
        // used for vertex and fragment
        let vertex_fragment_shader =
            device.create_shader_module(wgpu::include_wgsl!("vertex_fragment.wgsl"));

        // configure compute and render pipelines with the bind group layouts and the shader module
        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Compute pipeline"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Compute pipeline layout"),
                    // takes both group 0 and 1
                    bind_group_layouts: &[
                        Some(&compute_texture_bind_group_layout),
                        Some(&compute_scene_bind_group_layout),
                    ],
                    immediate_size: 0,
                }),
            ),
            module: &device.create_shader_module(wgpu::include_wgsl!("trace_paths.wgsl")), // the compute shader is in a separate file
            entry_point: Some("compute_main"), // function name in the wgsl file
            compilation_options: Default::default(),
            cache: None,
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render pipeline"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Render pipeline layout"),
                    // only needs group 0
                    bind_group_layouts: &[Some(&render_texture_bind_group_layout)],
                    immediate_size: 0,
                }),
            ),
            vertex: wgpu::VertexState {
                module: &vertex_fragment_shader,
                entry_point: Some("vs_main"), // function name in vertex_fragment.wgsl
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
                module: &vertex_fragment_shader,
                entry_point: Some("fs_main"), // function name in vertex_fragment.wgsl
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_config.format, // linear gamma
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

        // load and parse a glTF 2.0 file
        let mut scene = scene::Scene::new(&format!("assets/{GLTF}.glb")).await?;
        scene.move_camera_to(glam::Vec3::new(0.0, 0.0, 6.0));

        // for dragon.glb:
        // scene.move_camera_to(glam::Vec3::new(-4.0, 0.2, 0.0));
        // scene.rotate_camera_to(-std::f32::consts::FRAC_PI_2, 0.0);

        const ATLAS_SIZE: u32 = scene::ATLAS_SIZE as u32;

        // create texture array from atlases
        let texture_atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Texture atlas array"),
            size: wgpu::Extent3d {
                width: ATLAS_SIZE,
                height: ATLAS_SIZE,
                depth_or_array_layers: scene.texture_atlases.len() as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        // upload each atlas layer
        for (layer_idx, atlas_data) in scene.texture_atlases.iter().enumerate() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture_atlas,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: layer_idx as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                atlas_data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ATLAS_SIZE * 4),
                    rows_per_image: Some(ATLAS_SIZE),
                },
                wgpu::Extent3d {
                    width: ATLAS_SIZE,
                    height: ATLAS_SIZE,
                    depth_or_array_layers: 1,
                },
            );
        }

        let gpu_camera = scene.prepare_gpu_camera(0);

        use wgpu::util::DeviceExt; // for create_buffer_init

        // only this buffer (not geometry, bvh, etc.) is stored in the struct since it's the only one that's being updated when the program is running
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Camera buffer"),
            contents: bytemuck::bytes_of(&gpu_camera),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // buffer setup for group 1
        // local helper function to initialize storage buffers
        fn create_storage_buffer<T: bytemuck::Pod>(
            device: &wgpu::Device,
            label: &str,
            data: &[T],
        ) -> wgpu::Buffer {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(data),
                usage: wgpu::BufferUsages::STORAGE,
            })
        }
        let compute_scene_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Compute scene bind group"),
            layout: &compute_scene_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: create_storage_buffer(&device, "Geometry", &scene.geometries)
                        .as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: create_storage_buffer(&device, "BVH", &scene.bvh_nodes)
                        .as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: create_storage_buffer(&device, "Attributes", &scene.attributes)
                        .as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: create_storage_buffer(&device, "Materials", &scene.materials)
                        .as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&texture_atlas.create_view(
                        &wgpu::TextureViewDescriptor {
                            label: Some("Texture atlas view"),
                            format: Some(wgpu::TextureFormat::Rgba8Unorm),
                            dimension: Some(wgpu::TextureViewDimension::D2Array),
                            aspect: wgpu::TextureAspect::All,
                            ..Default::default()
                        },
                    )),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&sampler), // the same sampler can be used for both compute and render bind groups since they have the same filtering requirements
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: camera_buffer.as_entire_binding(),
                },
            ],
        });

        // texture setup for group 0
        let (compute_texture_bind_group, render_texture_bind_group) = create_texture_bind_groups(
            &device,
            &sampler,
            &compute_texture_bind_group_layout,
            &render_texture_bind_group_layout,
            &(surface_config.width, surface_config.height),
        );

        Ok(Self {
            surface,
            surface_config,
            device,
            queue,

            sampler,

            compute_texture_bind_group,
            render_texture_bind_group,
            compute_texture_bind_group_layout,
            render_texture_bind_group_layout,

            camera_buffer,
            compute_scene_bind_group,

            compute_pipeline,
            render_pipeline,

            scene,

            pressed_keys: std::collections::HashSet::new(),
            cursor_grab: winit::window::CursorGrabMode::None,

            last_instant: web_time::Instant::now(),

            fps_timer: 0.0,
            fps_counter: 0,
            current_fps: 0,

            frame_count: 0,

            #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
            profiler,

            window,
        })
    }

    /// Handle window resizes by reconfiguring the surface and recreating the storage texture, accumulation buffer, and their corresponding bind groups to match the new surface size.
    /// Changes the camera aspect ratio and resets the frame count for accumulation.
    pub fn resize(&mut self, width: u32, height: u32) {
        log::info!("Called: resize {width}x{height}");

        if width > 0 && height > 0 {
            self.surface_config.width = width;
            self.surface_config.height = height;
            self.surface.configure(&self.device, &self.surface_config);

            // recreate texture group with size of new storage texture matching new surface size
            // recreating bind group 0 is necessary for a resize since storage texture size must match the surface size
            (
                self.compute_texture_bind_group,
                self.render_texture_bind_group,
            ) = create_texture_bind_groups(
                &self.device,
                &self.sampler,
                &self.compute_texture_bind_group_layout,
                &self.render_texture_bind_group_layout,
                &(width, height),
            );

            self.scene
                .resize_camera_aspect_ratio(width as f32, height as f32);
            self.frame_count = 0;

            // on initial window creation on MacOS, and sometimes on initial web page load, even though resize is called, render isn't called afterwards
            // so, force a render call here
            self.window.request_redraw();
        }
    }

    /// Called every frame in a loop to update the application state and perform rendering.
    pub fn update(&mut self) {
        let dt = self.last_instant.elapsed().as_secs_f32();
        self.last_instant = web_time::Instant::now();

        self.fps_timer += dt;
        self.fps_counter += 1;
        if self.fps_timer >= 1.0 {
            self.current_fps = self.fps_counter;

            log::info!("FPS: {}", self.current_fps);

            self.fps_timer -= 1.0; // -= 1.0 instead of = 0.0 prevents drifting over time
            self.fps_counter = 0;
        }

        if self.scene.move_camera(&self.pressed_keys, dt, dt) {
            self.frame_count = 0; // reset frame count for accumulation when the camera moves
        }

        self.frame_count += 1; // note that frame_count is incremented every frame no matter if any camera movement has ocurred
        // so only when frame_count > 1 will the compute shader write to the accumulation buffer (the initial frame needs to overwrite the accumulation buffer instead of accumulating with a nonexistant previous frame)

        // update camera buffer every frame since even if the camera isn't moving, the frame count is still increasing for accumulation
        self.queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::bytes_of(&self.scene.prepare_gpu_camera(self.frame_count)),
        );

        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => {
                // encoder can record RenderPasses, ComputePasses, and transfer operations between driver-managed resources like Buffers and Textures
                // when finished recording, CommandEncoder::finish is called to obtain a CommandBuffer which is submitted for execution
                let mut encoder =
                    self.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("Encoder"),
                        });

                {
                    // this is in a code block because the &mut to encoder needs to be dropped before profiler.resolve_queries()
                    #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
                    let mut scope = self
                        .profiler
                        .scope("Compute and render scope", &mut encoder);

                    // compute pass
                    {
                        // this is in a code block because begin_compute_pass() takes a &mut to encoder
                        #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
                        let mut compute_pass = scope.scoped_compute_pass("Compute pass scoped");

                        #[cfg(not(all(feature = "testing", not(target_arch = "wasm32"))))]
                        let mut compute_pass =
                            encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                                label: Some("Compute pass"),
                                timestamp_writes: None,
                            });

                        compute_pass.set_pipeline(&self.compute_pipeline);
                        compute_pass.set_bind_group(0, &self.compute_texture_bind_group, &[]); // the u32 passed here, which is 0, matches with @group(0)
                        compute_pass.set_bind_group(1, &self.compute_scene_bind_group, &[]);

                        let workgroup_size = 8; // matches with @compute @workgroup_size(8, 8, 1)
                        let workgroup_count_x = self.surface_config.width.div_ceil(workgroup_size); // make sure that the entire texture is covered by 8x8 workgroups, since texture size should always equal surface_config size
                        let workgroup_count_y = self.surface_config.height.div_ceil(workgroup_size);
                        compute_pass.dispatch_workgroups(workgroup_count_x, workgroup_count_y, 1);
                    }

                    // render pass
                    {
                        // this is in a code block because begin_compute_pass() takes a &mut to encoder
                        let render_pass_descriptor = wgpu::RenderPassDescriptor {
                            label: Some("Render pass"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &frame.texture.create_view(&wgpu::TextureViewDescriptor {
                                    label: Some("Current frame surface texture view"),
                                    format: Some(self.surface_config.format), // linear gamma
                                    dimension: Some(wgpu::TextureViewDimension::D2),
                                    aspect: wgpu::TextureAspect::All,
                                    base_mip_level: 0,
                                    mip_level_count: Some(1),
                                    ..Default::default()
                                }),
                                depth_slice: None, // only useful for 3D textures
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Load,
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            depth_stencil_attachment: None,
                            timestamp_writes: None,
                            occlusion_query_set: None,
                            multiview_mask: None,
                        };

                        #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
                        let mut render_pass =
                            scope.scoped_render_pass("Render pass scoped", render_pass_descriptor);

                        #[cfg(not(all(feature = "testing", not(target_arch = "wasm32"))))]
                        let mut render_pass = encoder.begin_render_pass(&render_pass_descriptor);

                        render_pass.set_pipeline(&self.render_pipeline);
                        render_pass.set_bind_group(0, &self.render_texture_bind_group, &[]);
                        render_pass.draw(0..3, 0..1); // draw a triangle
                    }
                }

                #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
                self.profiler.resolve_queries(&mut encoder);

                self.queue.submit([encoder.finish()]);

                // although everything seems to work without pre_present_notify(), this is encouraged by winit docs
                // might only matter on Wayland
                self.window.pre_present_notify();

                frame.present();

                #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
                {
                    if let Err(e) = self.profiler.end_frame() {
                        log::error!("Failed to end profiler frame: {e:?}");
                    }
                    self.profiler
                        .process_finished_frame(self.queue.get_timestamp_period()); // needed for giving tracy statistics
                }
            }

            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {}
            wgpu::CurrentSurfaceTexture::Outdated
            | wgpu::CurrentSurfaceTexture::Suboptimal(_)
            | wgpu::CurrentSurfaceTexture::Lost => {
                // on Windows, fast resizes can cause Outdated error
                let size = self.window.inner_size();
                self.resize(size.width, size.height);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                log::error!("Validation error in get_current_texture");
            }
        }

        #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
        tracy_client::frame_mark();

        // this will trigger RedrawRequested event, which is a call to self.update() again, which creates a loop at the vsync rate of the monitor
        self.window.request_redraw();
    }

    /// Handle keyboard input events to update the set of currently pressed keys and toggle cursor grab mode when the Escape key is released.
    pub fn key_event(&mut self, key_event: &winit::event::KeyEvent) {
        // ignore when the OS or browser generates multiple presses and releases while a key is held down
        if !key_event.repeat
            && self.cursor_grab == winit::window::CursorGrabMode::Locked
            && let winit::keyboard::PhysicalKey::Code(code) = key_event.physical_key
        {
            match key_event.state {
                winit::event::ElementState::Pressed => {
                    self.pressed_keys.insert(code);
                    // log::info!("Key pressed: {code:?}");
                }
                winit::event::ElementState::Released => {
                    self.pressed_keys.remove(&code);
                    // log::info!("Key released: {code:?}");
                    if key_event.physical_key == winit::keyboard::KeyCode::Escape {
                        self.cycle_cursor_grab();
                    }
                }
            }
        }
    }

    /// Handle mouse movement events to rotate the camera when the cursor is locked, and reset the frame count for accumulation when the camera moves.
    pub fn mouse_move_event(&mut self, delta: (f64, f64)) {
        if self.cursor_grab == winit::window::CursorGrabMode::Locked {
            let (dx, dy) = (delta.0 as f32, delta.1 as f32);

            self.scene.rotate_camera(dx, dy, 0.003, 0.003);
            self.frame_count = 0; // reset frame count for accumulation when the camera moves
        }
    }

    /// Handle mouse button input events to toggle cursor grab mode when the left mouse button is released.
    pub fn mouse_button_event(
        &mut self,
        state: winit::event::ElementState,
        button: winit::event::MouseButton,
    ) {
        if state == winit::event::ElementState::Released
            && button == winit::event::MouseButton::Left
        {
            self.cycle_cursor_grab();
        }
    }

    /// Toggle cursor grab mode between locked and none.
    // on wasm, set_cursor_grab() and set_cursor_visible() seem to work most of the time
    // also, on wasm, the escape key is sometimes not received by winit since the browser thinks the webpage is in full screen, so the browser handles the escape
    // however, clicking works on wasm, so left click can be used to exit
    fn cycle_cursor_grab(&mut self) {
        log::info!("Called: cycle_cursor_grab");

        match self.cursor_grab {
            winit::window::CursorGrabMode::None => {
                if let Err(e) = self
                    .window
                    .set_cursor_grab(winit::window::CursorGrabMode::Locked)
                {
                    log::error!("Cursor not grabbed: {e}");
                } else {
                    self.window.set_cursor_visible(false);
                    self.cursor_grab = winit::window::CursorGrabMode::Locked;
                }
            }
            winit::window::CursorGrabMode::Locked | winit::window::CursorGrabMode::Confined => {
                if let Err(e) = self
                    .window
                    .set_cursor_grab(winit::window::CursorGrabMode::None)
                {
                    log::error!("Cursor not released: {e}");
                } else {
                    self.pressed_keys.clear();
                    self.window.set_cursor_visible(true);
                    self.cursor_grab = winit::window::CursorGrabMode::None;
                }
            }
        }
    }
}
