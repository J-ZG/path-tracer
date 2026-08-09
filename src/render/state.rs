//! Define the [`State`] struct which encapsulates all the GPU resources, scene data, and application state needed for rendering.
//!
//! Includes methods for initializing these resources and handling events from the window.

/// The name of the glTF file to load from the [`assets/glb/`](https://github.com/0dpe/path-tracer/tree/main/assets/glb) directory to render.
///
/// Can be exported from Blender.
/// ![Export glTF from Blender 1](https://raw.githubusercontent.com/0dpe/path-tracer/main/doc/blender_export_gltf_1.jpg)
/// ![Export glTF from Blender 2](https://raw.githubusercontent.com/0dpe/path-tracer/main/doc/blender_export_gltf_2.jpg)
/// Be sure to check `Include > Custom Properties` and `Mesh > Apply Modifiers`.
const GLTF: &str = "box";

use anyhow::Context;

use super::scene;
use super::ui;

/// Encapsulates all the GPU resources, scene data, and application state.
pub struct State {
    /// Represents the window or canvas that will be rendered onto.
    surface: wgpu::Surface<'static>,
    /// Describes the configuration of the surface, including format, size, etc. Needed for configuring the surface and recreating it on resize.
    surface_config: wgpu::SurfaceConfiguration,
    /// Represents a logical connection to the GPU and is used to create all other GPU resources.
    device: wgpu::Device,
    /// Used to execute recorded [`wgpu::CommandBuffer`] objects.
    queue: wgpu::Queue,

    /// Bind group 0 for the compute shader, containing the storage texture and accumulation buffer that are written to by the compute shader.
    compute_0_bind_group: wgpu::BindGroup,
    /// Bind group 0 for the render shader, containing just the storage texture read from by the fragment shader.
    render_0_bind_group: wgpu::BindGroup,
    /// Bind group layout for [`State::compute_0_bind_group`], needed here to recreate the bind group on window resizes.
    compute_0_bind_group_layout: wgpu::BindGroupLayout,
    /// Bind group layout for [`State::render_0_bind_group`], needed here to recreate the bind group on window resizes.
    render_0_bind_group_layout: wgpu::BindGroupLayout,

    /// Bind group 1 for the compute shader, containing mostly scene data. This bind group is separated from group 0 because this group never needs to be recreated during runtime (hence, its layout also isn't stored in this struct).
    compute_1_bind_group: wgpu::BindGroup,
    /// Buffer for camera data. Written to when the camera moves.
    camera_buffer: wgpu::Buffer,
    /// Buffer for frame count value used by the shader for accumulation and RNG seeding. Written to every frame with [`State::frame_count`].
    frame_count_buffer: wgpu::Buffer,
    /// Buffer for material data. Written to when the user modifies a material through the UI.
    materials_buffer: wgpu::Buffer,

    /// Runs the compute shader.
    compute_pipeline: wgpu::ComputePipeline,
    /// Runs the vertex and fragment shaders.
    render_pipeline: wgpu::RenderPipeline,

    /// Contains all data about the 3D scene.
    scene: scene::Scene,

    /// Tracks the current mode the user is in, either in cursor free or cursor grabbed mode.
    user_mode: UserMode,

    /// The time of the most recent call to [`State::update()`], used for scaling movement speed and calculating FPS.
    ///
    /// Using [`web_time::Instant`] which is a cross-platform abstraction over [`std::time::Instant`] that works on both native and web targets.
    last_instant: web_time::Instant,

    /// Tracks the number of frames rendered since the last camera movement or window resize.
    ///
    /// Used for accumulation in the compute shader, where the shader can use this count to determine how many frames have been accumulated so far and adjust its calculations accordingly.
    /// Note that when the shader gets this value (via [`wgpu::Queue::write_buffer()`]), it's always at least `1`.
    frame_count: u32,

    /// The index of the material the camera is currently pointed at. Used for determining which material the UI targets.
    selected_material_index: Option<usize>, // even though this is only used in update(), it's still stored in the struct because this value should be remembered across multiple update() calls to avoid having to run get material index even when camera_dirty is false

    /// Whether to show the UI overlay.
    show_ui: bool,

    /// Whether a frame has been presented, so that the loading screen in [`index.html`](https://github.com/0dpe/path-tracer/blob/main/index.html) can be dismissed once.
    #[cfg(target_arch = "wasm32")]
    presented_first_frame: bool,

    /// Context for the UI.
    egui_context: egui::Context,
    /// Lets the window communicate with the UI.
    pub(crate) egui_winit_state: egui_winit::State,
    /// Renders the UI.
    egui_wgpu_renderer: egui_wgpu::Renderer,

    /// Used for performance analysis.
    #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
    profiler: wgpu_profiler::GpuProfiler,

    /// The window.
    pub(crate) window: std::sync::Arc<winit::window::Window>,
}

/// Mode the user is currently in, either cursor free or cursor grabbed.
///
/// When cursor is grabbed, currently pressed keys are tracked.
enum UserMode {
    CursorFree,
    CursorGrabbed(std::collections::HashSet<winit::keyboard::KeyCode>),
}

impl State {
    /// Create a new [`State`].
    #[allow(clippy::future_not_send)]
    pub(crate) async fn new(
        window: std::sync::Arc<winit::window::Window>,
        display_handle: winit::event_loop::OwnedDisplayHandle,
    ) -> anyhow::Result<Self> {
        let egui_context = egui::Context::default();
        egui_context.set_theme(egui::Theme::Dark);
        egui_context.set_style_of(egui_context.theme(), ui::style());
        egui_extras::install_image_loaders(&egui_context);

        let egui_winit_state = egui_winit::State::new(
            egui_context.clone(),
            egui_context.viewport_id(),
            &display_handle,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(8192), // 8192 is the default max_texture_dimension_2d
        );

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
                // TODO: 30
                // apply_limit_buckets: false,
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

        let egui_wgpu_renderer = egui_wgpu::Renderer::new(
            &device,
            wgpu::TextureFormat::Bgra8Unorm,
            egui_wgpu::RendererOptions::default(),
        );

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
            format: wgpu::TextureFormat::Bgra8Unorm, // Bgra8Unorm should be a guaranteed supported format
            // sometimes on web initial page load, the canvas can have window.inner_size().width of 0
            // 0 length or width causes surface.configure() to panic
            // so, .max(1) makes sure that width and height are never less than 1
            // this is a standard solution: https://github.com/gfx-rs/wgpu/blob/04129aadaffb7fc72627102531633342f7cf90db/examples/features/src/hello_triangle/mod.rs#L66
            width: window.inner_size().width.max(1),
            height: window.inner_size().height.max(1),
            present_mode: wgpu::PresentMode::AutoNoVsync, // no vsync allows the program to run as fast as possible, giving better visual results when the GPU can calculate faster than the monitor refresh rate
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![wgpu::TextureFormat::Bgra8Unorm],
            // TODO: 30
            // color_space: wgpu::SurfaceColorSpace::Auto, 
        };

        // update() only works when the surface is configured
        // update() is often called right after new(), and resize() isn't called unless a resize happens, so configuring surface here is necessary
        surface.configure(&device, &surface_config);

        // group 0 for compute shader: storage texture and accumulation buffer
        let compute_0_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Compute bind group 0 layout"),
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
                ],
            });

        // group 0 for fragment shader: just storage texture
        let render_0_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Render bind group 0 layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        // TextureFormat::Rgba32Float is not filterable, also meaning that a sampler cannot be used
                        // if sampling is really needed, consider enabling FLOAT32_FILTERABLE feature, which is a native + web feature
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                }],
            });

        // group 1 for compute shader: scene data
        let compute_1_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Compute bind group 1 layout"),
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
                        binding: 4, // sRGB texture atlas array
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 5, // linear texture atlas array
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 6, // atlas sampler
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 7, // camera
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: std::num::NonZeroU64::new(std::mem::size_of::<
                                scene::GpuCamera,
                            >(
                            )
                                as u64), // not necessary, but is an optimization; allows wgpu to skip per-draw validation
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 8, // frame count
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: std::num::NonZeroU64::new(4),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 10, // skybox sampler
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
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
                    bind_group_layouts: &[
                        Some(&compute_0_bind_group_layout),
                        Some(&compute_1_bind_group_layout),
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
                    bind_group_layouts: &[Some(&render_0_bind_group_layout)],
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
        let mut scene = scene::Scene::new(&format!("assets/glb/{GLTF}.glb")).await?;
        scene
            .camera
            .translate_and_rotate_to(Some(glam::Vec3::new(0.0, 0.0, 7.0)), None, None);

        // for dragon.glb:
        // scene.camera.translate_and_rotate_to(Some(glam::Vec3::new(-4.0, 0.2, 0.0)), Some(-90.0_f32.to_radians()), None);

        // create texture arrays from atlases
        let texture_atlas_srgb = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sRGB texture atlas array"),
            size: wgpu::Extent3d {
                width: scene::ATLAS_SIZE,
                height: scene::ATLAS_SIZE,
                depth_or_array_layers: scene.srgb_texture_atlases.len() as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let texture_atlas_linear = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("linear texture atlas array"),
            size: wgpu::Extent3d {
                width: scene::ATLAS_SIZE,
                height: scene::ATLAS_SIZE,
                depth_or_array_layers: scene.linear_texture_atlases.len() as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        // upload each sRGB atlas layer
        for (layer_idx, atlas_data) in scene.srgb_texture_atlases.iter().enumerate() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture_atlas_srgb,
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
                    bytes_per_row: Some(scene::ATLAS_SIZE * 4),
                    rows_per_image: Some(scene::ATLAS_SIZE),
                },
                wgpu::Extent3d {
                    width: scene::ATLAS_SIZE,
                    height: scene::ATLAS_SIZE,
                    depth_or_array_layers: 1,
                },
            );
        }

        // upload each linear atlas layer
        for (layer_idx, atlas_data) in scene.linear_texture_atlases.iter().enumerate() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture_atlas_linear,
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
                    bytes_per_row: Some(scene::ATLAS_SIZE * 4),
                    rows_per_image: Some(scene::ATLAS_SIZE),
                },
                wgpu::Extent3d {
                    width: scene::ATLAS_SIZE,
                    height: scene::ATLAS_SIZE,
                    depth_or_array_layers: 1,
                },
            );
        }

        // setup for group 0
        let (compute_0_bind_group, render_0_bind_group) = Self::create_0_bind_groups(
            &device,
            &compute_0_bind_group_layout,
            &render_0_bind_group_layout,
            &(surface_config.width, surface_config.height),
        );

        use wgpu::util::DeviceExt; // for create_buffer_init

        // only camera, frame_count, and material buffers stored in the struct since they're the only ones being updated when the program is running (they have wgpu::BufferUsages::COPY_DST)

        let (gpu_camera, selected_material_index) = scene
            .takeout_gpu_camera()
            .context("camera isn't dirty on scene load")?;

        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Camera"),
            contents: bytemuck::bytes_of(&gpu_camera),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let frame_count_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Frame count"),
            contents: bytemuck::bytes_of(&1u32),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let materials_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Materials"),
            contents: bytemuck::cast_slice(&scene.materials),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

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

        let compute_1_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Compute bind group 1"),
            layout: &compute_1_bind_group_layout,
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
                    resource: materials_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&texture_atlas_srgb.create_view(
                        &wgpu::TextureViewDescriptor {
                            label: Some("sRGB texture atlas view"),
                            format: Some(wgpu::TextureFormat::Rgba8UnormSrgb),
                            dimension: Some(wgpu::TextureViewDimension::D2Array),
                            aspect: wgpu::TextureAspect::All,
                            ..Default::default()
                        },
                    )),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(
                        &texture_atlas_linear.create_view(&wgpu::TextureViewDescriptor {
                            label: Some("Linear texture atlas view"),
                            format: Some(wgpu::TextureFormat::Rgba8Unorm),
                            dimension: Some(wgpu::TextureViewDimension::D2Array),
                            aspect: wgpu::TextureAspect::All,
                            ..Default::default()
                        }),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(&device.create_sampler(
                        &wgpu::SamplerDescriptor {
                            label: Some("Atlas sampler"),
                            address_mode_u: wgpu::AddressMode::ClampToEdge,
                            address_mode_v: wgpu::AddressMode::ClampToEdge,
                            address_mode_w: wgpu::AddressMode::ClampToEdge,
                            mag_filter: wgpu::FilterMode::Nearest,
                            min_filter: wgpu::FilterMode::Nearest,
                            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                            ..Default::default()
                        },
                    )),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: frame_count_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::Sampler(&device.create_sampler(
                        &wgpu::SamplerDescriptor {
                            label: Some("Skybox sampler"),
                            address_mode_u: wgpu::AddressMode::ClampToEdge,
                            address_mode_v: wgpu::AddressMode::ClampToEdge,
                            address_mode_w: wgpu::AddressMode::ClampToEdge,
                            mag_filter: wgpu::FilterMode::Linear,
                            min_filter: wgpu::FilterMode::Linear,
                            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                            ..Default::default()
                        },
                    )),
                },
            ],
        });

        Ok(Self {
            surface,
            surface_config,
            device,
            queue,

            compute_0_bind_group,
            render_0_bind_group,
            compute_0_bind_group_layout,
            render_0_bind_group_layout,

            compute_1_bind_group,
            camera_buffer,
            frame_count_buffer,
            materials_buffer,

            compute_pipeline,
            render_pipeline,

            scene,

            user_mode: UserMode::CursorFree,

            last_instant: web_time::Instant::now(),

            frame_count: 0,

            selected_material_index,

            show_ui: true,

            #[cfg(target_arch = "wasm32")]
            presented_first_frame: false,

            egui_context,
            egui_winit_state,
            egui_wgpu_renderer,

            #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
            profiler,

            window,
        })
    }

    /// Create or recreate the storage texture, accumulation buffer, and their corresponding bind groups for both compute and render pipelines.
    /// Called on window resizes since the surface size changes.
    /// Returns the compute and render bind groups, respectively, as a tuple.
    fn create_0_bind_groups(
        device: &wgpu::Device,
        compute_layout: &wgpu::BindGroupLayout,
        render_layout: &wgpu::BindGroupLayout,
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

        (
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Compute bind group 0"),
                layout: compute_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&storage_texture_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Buffer(
                            device
                                .create_buffer(&wgpu::BufferDescriptor {
                                    label: Some("Accumulation buffer"), // this is actually used as a texture, but since textures cannot be both read and write, a storage buffer is used instead
                                    size: wgpu::BufferAddress::from(
                                        texture_size.0 * texture_size.1 * 16, // 16 bytes per pixel for vec4<f32>
                                    ),
                                    usage: wgpu::BufferUsages::STORAGE,
                                    mapped_at_creation: false,
                                })
                                .as_entire_buffer_binding(),
                        ),
                    },
                ],
            }),
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Render bind group 0"),
                layout: render_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0, // matches with @binding(0)
                    resource: wgpu::BindingResource::TextureView(&storage_texture_view),
                }],
            }),
        )
    }

    /// Handle window resizes by reconfiguring the surface,
    /// recreating the storage texture and accumulation buffer,
    /// recreating their bind groups [`State::compute_0_bind_group`] and [`State::render_0_bind_group`],
    /// and changing the camera aspect ratio to match the new surface size.
    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        // log::info!("Resize {width}x{height}");

        if width > 0 && height > 0 {
            self.surface_config.width = width;
            self.surface_config.height = height;
            self.surface.configure(&self.device, &self.surface_config);

            // recreating bind group 0 is necessary for a resize since storage texture size must match the surface size
            (self.compute_0_bind_group, self.render_0_bind_group) = Self::create_0_bind_groups(
                &self.device,
                &self.compute_0_bind_group_layout,
                &self.render_0_bind_group_layout,
                &(width, height),
            );

            self.scene.camera.resize((width as f32) / (height as f32));

            // on initial window creation on MacOS, and sometimes on initial web page load, even though resize is called, render isn't called afterwards
            // so, force a render call here
            self.window.request_redraw();
        }
    }

    /// Returns whether the cursor is currently grabbed by the application.
    pub(crate) const fn is_cursor_grabbed(&self) -> bool {
        matches!(self.user_mode, UserMode::CursorGrabbed(_))
    }

    /// Called every frame in a loop to update the application state and perform rendering.
    pub(crate) fn update(&mut self) {
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => {
                let dt = self.last_instant.elapsed().as_secs_f32();
                self.last_instant = web_time::Instant::now();

                let mut ui_changed_mat_index = None;

                let mut full_output = self.egui_context.run_ui(
                    self.egui_winit_state.take_egui_input(&self.window),
                    |root_ui| {
                        if self.show_ui {
                            ui_changed_mat_index = ui::run(
                                root_ui,
                                &mut self.scene,
                                self.selected_material_index,
                                dt,
                                self.frame_count,
                            );
                        }
                    },
                );

                if let Some(mat_index) = ui_changed_mat_index {
                    self.queue.write_buffer(
                        &self.materials_buffer,
                        (mat_index * std::mem::size_of::<scene::GpuMaterial>())
                            as wgpu::BufferAddress,
                        bytemuck::bytes_of(&self.scene.materials[mat_index]),
                    );

                    self.frame_count = 0;
                }

                match &self.user_mode {
                    UserMode::CursorGrabbed(pressed_keys) => {
                        self.scene.camera.translate(pressed_keys, dt, dt);
                    }
                    UserMode::CursorFree => {}
                }

                if let Some((gpu_camera, selected_material_index)) = self.scene.takeout_gpu_camera()
                {
                    self.selected_material_index = selected_material_index;

                    self.queue.write_buffer(
                        &self.camera_buffer,
                        0,
                        bytemuck::bytes_of(&gpu_camera),
                    );

                    self.frame_count = 0;
                }

                self.frame_count += 1; // note that frame_count is incremented every frame, after any reset to 0

                self.queue.write_buffer(
                    // write_buffer happens after the increment
                    &self.frame_count_buffer,
                    0,
                    bytemuck::bytes_of(&(self.frame_count)),
                ); // so for in the compute shader, frame_count is always at least 1
                // only when frame_count > 1 will the compute shader write to the accumulation buffer (the initial frame needs to overwrite the accumulation buffer instead of accumulating with a nonexistent previous frame)

                // tesselate full_output.shapes into a vec of triangles
                let paint_jobs = self
                    .egui_context
                    .tessellate(full_output.shapes, self.egui_context.pixels_per_point());

                let mut platform_output = full_output.platform_output;
                if matches!(self.user_mode, UserMode::CursorGrabbed(_)) {
                    platform_output.cursor_icon = egui::CursorIcon::None; // disallow egui from making cursor visible when cursor is grabbed, even if egui requests a visible cursor (for example, when hovering over a text input field)
                }
                // mainly for this app, just updates cursor icon and clipboard
                self.egui_winit_state
                    .handle_platform_output(&self.window, platform_output);

                // must be called before egui_wgpu_renderer.render()
                for (id, image_delta) in &full_output.textures_delta.set {
                    self.egui_wgpu_renderer.update_texture(
                        &self.device,
                        &self.queue,
                        *id,
                        image_delta,
                    );
                }
                // TODO: 30
                // for (id, image_deltas) in &full_output.textures_delta.set {
                //     for image_delta in image_deltas {
                //         self.egui_wgpu_renderer.update_texture(
                //             &self.device,
                //             &self.queue,
                //             *id,
                //             image_delta,
                //         );
                //     }
                // }

                // used for both egui_wgpu_renderer.update_buffers() and egui_wgpu_renderer.render()
                let screen_descriptor = egui_wgpu::ScreenDescriptor {
                    size_in_pixels: [self.surface_config.width, self.surface_config.height],
                    pixels_per_point: self.egui_context.pixels_per_point(),
                };

                // encodes a series of GPU instructions (RenderPasses, ComputePasses, etc.)
                let mut encoder =
                    self.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("Encoder"),
                        });

                // must be called before egui_wgpu_renderer.render()
                self.egui_wgpu_renderer.update_buffers(
                    &self.device,
                    &self.queue,
                    &mut encoder,
                    &paint_jobs,
                    &screen_descriptor,
                );

                // compute pass

                // for wgpu_profiler, not using the auto-closing profiler.scope() -> Scope<'a, Recorder> because scope.scoped_render_pass() returns OwningScope<'_, RenderPass<'_>>, not the normal RenderPass
                // same for scope.scoped_compute_pass(), returning OwningScope<'_, ComputePass<'_>>
                // egui's render() requires a normal RenderPass, not OwningScope
                // so, manually manage a GpuProfilerQuery here
                #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
                let outer_query = self
                    .profiler
                    .begin_query("Compute and render scope", &mut encoder);

                #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
                let compute_query = self
                    .profiler
                    .begin_pass_query("Compute pass scoped", &mut encoder)
                    .with_parent(Some(&outer_query)); // make compute_query a child of outer_query

                #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
                let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Compute pass"),
                    timestamp_writes: compute_query.compute_pass_timestamp_writes(),
                });

                #[cfg(not(all(feature = "testing", not(target_arch = "wasm32"))))]
                let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Compute pass"),
                    timestamp_writes: None,
                });

                compute_pass.set_pipeline(&self.compute_pipeline);
                compute_pass.set_bind_group(0, &self.compute_0_bind_group, &[]); // the u32 passed here, which is 0, matches with @group(0)
                compute_pass.set_bind_group(1, &self.compute_1_bind_group, &[]);
                compute_pass.dispatch_workgroups(
                    self.surface_config.width.div_ceil(8),
                    self.surface_config.height.div_ceil(8),
                    1,
                );

                drop(compute_pass); // avoid multiple &mut to encoder

                // for manually managing a GpuProfilerQuery, this is necessary
                #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
                self.profiler.end_query(&mut encoder, compute_query);

                // render pass

                // create a reusable frame texture view to avoid creating it redundantly across the two render passes
                let frame_view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("Current frame surface texture view"),
                    format: Some(self.surface_config.format), // linear gamma
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    aspect: wgpu::TextureAspect::All,
                    base_mip_level: 0,
                    mip_level_count: Some(1),
                    ..Default::default()
                });

                let render_pass_descriptor = wgpu::RenderPassDescriptor {
                    label: Some("Render pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &frame_view,
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
                let render_query = self
                    .profiler
                    .begin_pass_query("Render pass scoped", &mut encoder)
                    .with_parent(Some(&outer_query));

                #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
                let mut render_pass = encoder
                    .begin_render_pass(&wgpu::RenderPassDescriptor {
                        timestamp_writes: render_query.render_pass_timestamp_writes(),
                        ..render_pass_descriptor
                    })
                    .forget_lifetime();

                #[cfg(not(all(feature = "testing", not(target_arch = "wasm32"))))]
                let mut render_pass = encoder
                    .begin_render_pass(&render_pass_descriptor)
                    .forget_lifetime();

                render_pass.set_pipeline(&self.render_pipeline);
                render_pass.set_bind_group(0, &self.render_0_bind_group, &[]);
                render_pass.draw(0..3, 0..1); // draw a triangle (full-screen triangle)

                // egui render pass, using the same render_pass RenderPass<'_>
                self.egui_wgpu_renderer
                    .render(&mut render_pass, &paint_jobs, &screen_descriptor);

                // freed after render(); will still run if this was not called, but might cause memory leak
                for id in &full_output.textures_delta.free {
                    self.egui_wgpu_renderer.free_texture(id);
                }
                full_output.textures_delta.clear();

                drop(render_pass); // will cause runtime panic if this wasn't called before profiler.end_query(&mut encoder, outer_query)

                #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
                {
                    self.profiler.end_query(&mut encoder, render_query);
                    self.profiler.end_query(&mut encoder, outer_query);
                    self.profiler.resolve_queries(&mut encoder);
                }

                self.queue.submit([encoder.finish()]);

                // although everything seems to work without pre_present_notify(), this is encouraged by winit docs
                // might only matter on Wayland
                self.window.pre_present_notify();

                frame.present();
                // TODO: 30 
                // self.queue.present(frame);

                // the first proper frame is presented, so there is something on the canvas on the web,
                // so the loading screen in index.html can fade out
                #[cfg(target_arch = "wasm32")]
                if !self.presented_first_frame {
                    self.presented_first_frame = true;
                    crate::logger::send_js_event("presented_first_frame", "First frame presented");
                }

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
    pub(crate) fn key_event(&mut self, key_event: &winit::event::KeyEvent) {
        // ignore when the OS or browser generates multiple presses and releases while a key is held down
        if !key_event.repeat
            && let winit::keyboard::PhysicalKey::Code(code) = key_event.physical_key
        {
            if key_event.physical_key == winit::keyboard::KeyCode::F1
                && key_event.state == winit::event::ElementState::Released
            {
                self.show_ui ^= true;
            }

            match &mut self.user_mode {
                UserMode::CursorGrabbed(pressed_keys) => match key_event.state {
                    winit::event::ElementState::Pressed => {
                        pressed_keys.insert(code);
                        // log::info!("Key pressed: {code:?}");
                    }
                    winit::event::ElementState::Released => {
                        pressed_keys.remove(&code);
                        // log::info!("Key released: {code:?}");
                        if key_event.physical_key == winit::keyboard::KeyCode::Escape {
                            self.release_cursor_grab();
                        }
                    }
                },
                UserMode::CursorFree => {}
            }
        }
    }

    /// Handle mouse movement events to rotate the camera when the cursor is locked, and reset the frame count for accumulation when the camera moves.
    pub(crate) const fn mouse_move_event(&mut self, delta: (f64, f64)) {
        if matches!(&self.user_mode, UserMode::CursorGrabbed(_)) {
            self.scene.camera.rotate(delta.0 as f32, delta.1 as f32);
        }
    }

    /// Handle mouse button input events to toggle cursor grab mode when the left mouse button is released.
    pub(crate) fn mouse_button_event(
        &mut self,
        state: winit::event::ElementState,
        button: winit::event::MouseButton,
    ) {
        if state == winit::event::ElementState::Released
            && button == winit::event::MouseButton::Left
        {
            // on wasm, set_cursor_grab() and set_cursor_visible() seem to work most of the time
            // also, on wasm, the escape key is sometimes not received by winit since the browser thinks the webpage is in full screen, so the browser handles the escape
            // however, clicking works on wasm, so left click can be used to exit
            if matches!(self.user_mode, UserMode::CursorGrabbed(_)) {
                self.release_cursor_grab();
            } else if let Err(e) = self
                .window
                .set_cursor_grab(winit::window::CursorGrabMode::Locked)
            {
                log::error!("Cursor not grabbed: {e}");
            } else {
                self.user_mode = UserMode::CursorGrabbed(std::collections::HashSet::new());
                self.window.set_cursor_visible(false);
            }
        }
    }

    /// Release the cursor from the window, setting cursor grab mode to none.
    pub(crate) fn release_cursor_grab(&mut self) {
        if let Err(e) = self
            .window
            .set_cursor_grab(winit::window::CursorGrabMode::None)
        {
            log::error!("Cursor not released: {e}");
        } else {
            self.window.set_cursor_visible(true);
            self.user_mode = UserMode::CursorFree;
        }
    }
}
