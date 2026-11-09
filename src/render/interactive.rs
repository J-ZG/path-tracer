//! The interactive mode: a window where the user explores and edits the world while samples accumulate.
//!
//! [`State`] uses a [`PathTracer`] whose tile covers the whole window, tracing one sample and displaying the tile to the window's surface every frame.
//! On native, the headless mode is the other user of [`PathTracer`], rendering an image to a file instead.

/// Supported file extensions for the scene.
const SCENE_EXTS: &[&str] = &["glb"];
/// Supported file extensions for the skybox.
const SKYBOX_EXTS: &[&str] = &["exr", "hdr", "jpeg", "jpg", "png", "webp"];

use super::path_tracer::PathTracer;
use super::ui;
use super::world::World;

/// State of the interactive mode: the window's surface, the UI, the editable world, and the GPU resources for tracing and displaying it.
pub(crate) struct State {
    /// Represents the window or canvas that will be rendered onto.
    surface: wgpu::Surface<'static>,
    /// Describes the configuration of the surface, including format, size, etc. Needed for configuring the surface and recreating it on resize.
    surface_config: wgpu::SurfaceConfiguration,
    /// Represents a logical connection to the GPU and is used to create all other GPU resources.
    device: wgpu::Device,
    /// Used to execute recorded [`wgpu::CommandBuffer`] objects.
    queue: wgpu::Queue,
    /// Traces paths of [`Self::world`] for the whole window, and displays the result.
    path_tracer: PathTracer,

    /// Manages world data.
    world: World,

    /// Tracks the current mode the user is in, either in cursor free or cursor grabbed mode.
    user_mode: UserMode,

    /// The time of the most recent call to [`State::update()`], used for scaling movement speed and calculating FPS.
    ///
    /// Using [`web_time::Instant`] which is a cross-platform abstraction over [`std::time::Instant`] that works on both native and web targets.
    last_instant: web_time::Instant,

    /// Whether to show the UI overlay.
    show_ui: bool,

    /// Whether a frame has been presented, so that the loading screen in [`index.html`](https://github.com/j-zg/path-tracer/blob/main/index.html) can be dismissed once.
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

    /// Receiver for user-given files.
    file_receiver: std::sync::mpsc::Receiver<UploadedFile>,
    /// Sender for user-given files.
    file_sender: std::sync::mpsc::Sender<UploadedFile>,
    /// What to display in the UI regarding file uploads.
    file_upload_status: String,

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

/// Holds the byte data for an uploaded scene or skybox.
enum UploadedFile {
    Scene(Vec<u8>),
    Skybox(Vec<u8>),
}

impl State {
    /// Create a new [`State`].
    #[expect(clippy::future_not_send)]
    pub(crate) async fn new(
        window: std::sync::Arc<winit::window::Window>,
        display_handle: winit::event_loop::OwnedDisplayHandle,
    ) -> anyhow::Result<Self> {
        let egui_context = egui::Context::default();
        // egui-winit uses the operating system to choose Cmd or Ctrl for shortcuts
        #[cfg(target_arch = "wasm32")]
        if let Some(window) = web_sys::window()
            && let Ok(user_agent) = window.navigator().user_agent()
        {
            egui_context.set_os(egui::os::OperatingSystem::from_user_agent(&user_agent));
        }
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
            color_space: wgpu::SurfaceColorSpace::Auto,
        };

        let adapter = super::gpu::request_adapter(&instance, Some(&surface)).await?;

        // see which TextureFormat's are supported by the surface
        // Bgra8Unorm and Bgra8UnormSrgb should be guaranteed, but on web, Bgra8UnormSrgb isn't supported it seems
        // for example, on Windows native, [Bgra8UnormSrgb, Rgba8UnormSrgb, Bgra8Unorm, Rgba8Unorm, Rgba16Float, Rgb10a2Unorm] are supported, but with Chrome or Firefox, only [Bgra8Unorm, Rgba8Unorm, Rgba16Float] are supported
        #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
        log::info!(
            "Surface formats: {:?}",
            surface.get_capabilities(&adapter).formats
        );

        let (device, queue) = super::gpu::request_device(&adapter).await?;

        // update() only works when the surface is configured
        // update() is often called right after new(), and resize() isn't called unless a resize happens, so configuring surface here is necessary
        surface.configure(&device, &surface_config);

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

        // before getting world and path_tracer, we need to get bytes for the default scene and skybox first
        // on both native and wasm, loading files is at runtime, which allows files to change without having to rebuild the WASM
        let glb_path = super::DEFAULT_SCENE;
        let skybox_path = super::DEFAULT_SKYBOX;
        let glb_bytes;
        let skybox_bytes;
        #[cfg(not(target_arch = "wasm32"))]
        {
            glb_bytes = std::fs::read(glb_path)?;
            skybox_bytes = std::fs::read(skybox_path)?;
        }
        #[cfg(target_arch = "wasm32")]
        {
            async fn fetch_bytes(url: &str) -> anyhow::Result<Vec<u8>> {
                let response = wasm_bindgen_futures::JsFuture::from(
                    web_sys::window()
                        .ok_or_else(|| anyhow::anyhow!("get web_sys window failed"))?
                        .fetch_with_str(url),
                )
                .await
                .map_err(|e| anyhow::anyhow!("error fetching {url}: {e:?}"))?;
                let response = wasm_bindgen::JsCast::dyn_into::<web_sys::Response>(response)
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "failed to cast resolved fetch response to web_sys::Response: {e:?}"
                        )
                    })?;

                if !response.ok() {
                    anyhow::bail!(
                        "network error: HTTP {} {}",
                        response.status(),
                        response.status_text()
                    );
                }

                let u8_array = js_sys::Uint8Array::new(
                    &wasm_bindgen_futures::JsFuture::from(
                        response
                            .array_buffer()
                            .map_err(|e| anyhow::anyhow!("response.array_buffer error: {e:?}"))?,
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("JsFuture::from(array_buffer) error: {e:?}"))?,
                );
                let mut bytes = vec![0u8; u8_array.length() as usize];
                u8_array.copy_to(&mut bytes[..]);
                Ok(bytes)
            }

            glb_bytes = fetch_bytes(glb_path).await?;
            skybox_bytes = fetch_bytes(skybox_path).await?;
        }

        let mut world = World::new(
            &glb_bytes,
            &skybox_bytes,
            surface_config.width as f32 / surface_config.height as f32,
        )?;
        world.edit_skybox_average_luminance(super::world::DEFAULT_SKYBOX_LUMINANCE);
        let path_tracer = PathTracer::new(
            &device,
            &queue,
            world.take_updates(),
            [surface_config.width, surface_config.height],
            surface_config.format,
        )?;

        let (file_sender, file_receiver) = std::sync::mpsc::channel();

        Ok(Self {
            surface,
            surface_config,
            device,
            queue,
            path_tracer,

            world,

            user_mode: UserMode::CursorFree,

            last_instant: web_time::Instant::now(),

            show_ui: true,

            #[cfg(target_arch = "wasm32")]
            presented_first_frame: false,

            egui_context,
            egui_winit_state,
            egui_wgpu_renderer,

            #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
            profiler,

            file_receiver,
            file_sender,
            file_upload_status: String::new(),

            window,
        })
    }

    /// Use [`rfd`] to allow file picking on both native and web.
    ///
    /// `is_scene` being `true` means picking a scene, `false` means picking a skybox.
    // called only in update()
    fn spawn_file_picker(file_sender: std::sync::mpsc::Sender<UploadedFile>, is_scene: bool) {
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::spawn(move || {
            let dialog = rfd::FileDialog::new();
            let dialog = if is_scene {
                dialog.add_filter("Scene", SCENE_EXTS)
            } else {
                dialog.add_filter("Skybox", SKYBOX_EXTS)
            };
            if let Some(path) = dialog.pick_file()
                && let Ok(bytes) = std::fs::read(path)
            {
                let _ = file_sender.send(if is_scene {
                    UploadedFile::Scene(bytes)
                } else {
                    UploadedFile::Skybox(bytes)
                });
            }
        });
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(async move {
            let dialog = rfd::AsyncFileDialog::new();
            let dialog = if is_scene {
                dialog.add_filter("Scene", SCENE_EXTS)
            } else {
                dialog.add_filter("Skybox", SKYBOX_EXTS)
            };
            if let Some(file) = dialog.pick_file().await {
                let bytes = file.read().await;
                let _ = file_sender.send(if is_scene {
                    UploadedFile::Scene(bytes)
                } else {
                    UploadedFile::Skybox(bytes)
                });
            }
        });
    }

    /// Handle window resizes by reconfiguring the surface,
    /// resizing the tile of [`Self::path_tracer`] to the new surface size,
    /// and changing the camera aspect ratio to match the new surface size.
    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
        log::info!("Resize {width}x{height}");

        if width > 0 && height > 0 {
            if let Err(error) =
                self.path_tracer
                    .set_tile(&self.device, [width, height], [0, 0], [width, height])
            {
                log::error!("Failed to resize the path tracer: {error:#}");
                return;
            }
            self.surface_config.width = width;
            self.surface_config.height = height;
            self.surface.configure(&self.device, &self.surface_config);

            self.world
                .edit_camera(|camera| camera.resize((width as f32) / (height as f32)));

            // on initial window creation on MacOS, and sometimes on initial web page load, even though resize is called, render isn't called afterwards
            // so, force a render call here
            self.window.request_redraw();
        }
    }

    /// Called every frame in a loop to update the application state and perform rendering.
    pub(crate) fn update(&mut self) {
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => {
                let dt = self.last_instant.elapsed().as_secs_f32();
                self.last_instant = web_time::Instant::now();

                let raw_input = self.egui_winit_state.take_egui_input(&self.window);

                // allow the user to drag and drop a scene or skybox
                // TODO: doesn't work on wasm32 at all, even when browser default drag and drop behavior is disabled; probably requires complex fix
                for file in &raw_input.dropped_files {
                    let Some(ext) = file.path().extension().and_then(|e| e.to_str()) else {
                        continue;
                    };
                    let ext = ext.to_lowercase();
                    let is_scene = SCENE_EXTS.contains(&ext.as_str());
                    if !is_scene && !SKYBOX_EXTS.contains(&ext.as_str()) {
                        continue;
                    }
                    let sender = self.file_sender.clone();
                    let send_bytes = move |result: Result<Vec<u8>, String>| match result {
                        Ok(bytes) => {
                            let upload = if is_scene {
                                UploadedFile::Scene(bytes)
                            } else {
                                UploadedFile::Skybox(bytes)
                            };
                            let _ = sender.send(upload);
                        }
                        Err(error) => log::error!("Failed to read dropped file: {error}"),
                    };
                    #[cfg(not(target_arch = "wasm32"))]
                    send_bytes(file.bytes());
                    #[cfg(target_arch = "wasm32")]
                    {
                        let file = std::sync::Arc::clone(file);
                        wasm_bindgen_futures::spawn_local(async move {
                            send_bytes(file.bytes_async().await);
                        });
                    }
                }

                let is_hovering = !raw_input.hovered_files.is_empty();
                let mut max_bounces = self.path_tracer.max_bounces();
                let mut full_output = self.egui_context.run_ui(raw_input, |root_ui| {
                    if self.show_ui {
                        for action in ui::run_base(
                            root_ui,
                            &mut self.world,
                            &mut max_bounces,
                            dt,
                            self.path_tracer.sample_count(),
                            &self.file_upload_status,
                            is_hovering,
                        ) {
                            match action {
                                ui::BaseUiAction::UploadScene => {
                                    self.file_upload_status = "Picking scene...".to_string();
                                    Self::spawn_file_picker(self.file_sender.clone(), true);
                                }
                                ui::BaseUiAction::UploadSkybox => {
                                    self.file_upload_status = "Picking skybox...".to_string();
                                    Self::spawn_file_picker(self.file_sender.clone(), false);
                                }
                            }
                        }

                        if let Some(material) = self.world.selected_material() {
                            ui::run_modify_material(root_ui, &mut self.world, &material);
                        }
                    }
                });

                // handle when a file is uploaded
                // TODO: this makes the entire app freeze if the scene or skybox is large; maybe do this on a background thread somehow?
                while let Ok(file) = self.file_receiver.try_recv() {
                    match file {
                        UploadedFile::Scene(bytes) => {
                            if self.world.replace_scene(&bytes).is_ok() {
                                self.file_upload_status = "Successfully loaded scene".to_string();
                            } else {
                                log::warn!("Failed to load scene");
                                self.file_upload_status = "Failed to load scene".to_string();
                            }
                        }
                        UploadedFile::Skybox(bytes) => {
                            if self.world.replace_skybox(&bytes).is_ok() {
                                self.file_upload_status = "Successfully loaded skybox".to_string();
                            } else {
                                log::warn!("Failed to load skybox");
                                self.file_upload_status = "Failed to load skybox".to_string();
                            }
                        }
                    }
                }

                match &self.user_mode {
                    UserMode::CursorGrabbed(pressed_keys) => {
                        self.world
                            .edit_camera(|camera| camera.translate(pressed_keys, dt, dt));
                    }
                    UserMode::CursorFree => {}
                }

                self.path_tracer.set_max_bounces(max_bounces);
                self.path_tracer.apply_updates(
                    &self.device,
                    &self.queue,
                    self.world.take_updates(),
                );

                // tesselate full_output.shapes into a vec of triangles
                let paint_jobs = self
                    .egui_context
                    .tessellate(full_output.shapes, self.egui_context.pixels_per_point());

                let mut platform_output = full_output.platform_output;
                if matches!(self.user_mode, UserMode::CursorGrabbed(_)) {
                    platform_output.cursor_icon = egui::CursorIcon::None; // disallow egui from making cursor visible when cursor is grabbed, even if egui requests a visible cursor (for example, when hovering over a text input field)
                }
                // egui-winit's wasm clipboard only stores text inside the app
                // write copy/cut output to the browser clipboard
                #[cfg(target_arch = "wasm32")]
                for command in &platform_output.commands {
                    if let egui::OutputCommand::CopyText(text) = command
                        && let Some(window) = web_sys::window()
                    {
                        let clipboard = window.navigator().clipboard();
                        if clipboard.is_undefined() {
                            log::warn!("Browser clipboard is unavailable; use HTTPS or localhost");
                            continue;
                        }
                        let write = clipboard.write_text(text);
                        wasm_bindgen_futures::spawn_local(async move {
                            if let Err(error) = wasm_bindgen_futures::JsFuture::from(write).await {
                                log::warn!(
                                    "Failed to copy text to the browser clipboard: {error:?}"
                                );
                            }
                        });
                    }
                }
                // mainly for this app, just updates cursor icon and clipboard
                self.egui_winit_state
                    .handle_platform_output(&self.window, platform_output);

                // must be called before egui_wgpu_renderer.render()
                for (id, image_deltas) in &full_output.textures_delta.set {
                    for image_delta in image_deltas {
                        self.egui_wgpu_renderer.update_texture(
                            &self.device,
                            &self.queue,
                            *id,
                            image_delta,
                        );
                    }
                }

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

                // trace pass

                // for wgpu_profiler, not using the auto-closing profiler.scope() -> Scope<'a, Recorder> because scope.scoped_render_pass() returns OwningScope<'_, RenderPass<'_>>, not the normal RenderPass
                // same for scope.scoped_compute_pass(), returning OwningScope<'_, ComputePass<'_>>
                // egui's render() requires a normal RenderPass, not OwningScope
                // so, manually manage a GpuProfilerQuery here
                #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
                let outer_query = self
                    .profiler
                    .begin_query("Trace and display scope", &mut encoder);

                #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
                let compute_query = self
                    .profiler
                    .begin_pass_query("Trace pass scoped", &mut encoder)
                    .with_parent(Some(&outer_query)); // make compute_query a child of outer_query

                #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
                let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Trace pass"),
                    timestamp_writes: compute_query.compute_pass_timestamp_writes(),
                });

                #[cfg(not(all(feature = "testing", not(target_arch = "wasm32"))))]
                let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Trace pass"),
                    timestamp_writes: None,
                });

                self.path_tracer.trace(&self.queue, &mut compute_pass, 1);

                drop(compute_pass); // avoid multiple &mut to encoder

                // for manually managing a GpuProfilerQuery, this is necessary
                #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
                self.profiler.end_query(&mut encoder, compute_query);

                // display pass

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
                    label: Some("Display pass"),
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
                    .begin_pass_query("Display pass scoped", &mut encoder)
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

                self.path_tracer.display(&mut render_pass);

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

                self.queue.present(frame);

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

    /// Handle mouse movement events to rotate the camera when the cursor is locked.
    pub(crate) fn mouse_move_event(&mut self, delta: (f64, f64)) {
        if matches!(&self.user_mode, UserMode::CursorGrabbed(_)) {
            self.world
                .edit_camera(|camera| camera.rotate(delta.0 as f32, delta.1 as f32));
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

    /// Returns whether the cursor is currently grabbed by the application.
    pub(crate) const fn is_cursor_grabbed(&self) -> bool {
        matches!(self.user_mode, UserMode::CursorGrabbed(_))
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
