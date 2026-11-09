//! Requesting a GPU adapter, and a device with the features and limits required by the shaders.
//!
//! Both the interactive and headless modes need a device that can run [`super::path_tracer::PathTracer`], so they request it through this module.
//! They differ only in the adapter: the interactive mode needs one compatible with the window's surface, while the headless mode has no surface.

/// Request an adapter that can present to `compatible_surface`, if one is given.
#[cfg_attr(
    target_arch = "wasm32",
    expect(
        clippy::future_not_send,
        reason = "WebGPU futures run on the browser thread"
    )
)]
pub(super) async fn request_adapter(
    instance: &wgpu::Instance,
    compatible_surface: Option<&wgpu::Surface<'_>>,
) -> anyhow::Result<wgpu::Adapter> {
    Ok(instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        })
        .await?)
}

/// Request a device and queue from `adapter` with the features and limits required by the shaders.
#[cfg_attr(
    target_arch = "wasm32",
    expect(
        clippy::future_not_send,
        reason = "WebGPU futures run on the browser thread"
    )
)]
pub(super) async fn request_device(
    adapter: &wgpu::Adapter,
) -> anyhow::Result<(wgpu::Device, wgpu::Queue)> {
    let supported_limits = adapter.limits(); // get the maximum limits the physical hardware supports

    log::info!(
        "Using adapter: {}\nMax storage buffer binding size: {} MiB\nMax texture dimension 2D: {}",
        adapter.get_info().name,
        supported_limits.max_storage_buffer_binding_size as f32 / 1024.0 / 1024.0,
        supported_limits.max_texture_dimension_2d
    );

    #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
    let required_features = (adapter.features()
        & wgpu_profiler::GpuProfiler::ALL_WGPU_TIMER_FEATURES) // for wgpu_profiler
        | wgpu::Features::FLOAT32_FILTERABLE; // highly supported: https://web3dsurvey.com/webgpu/features/float32-filterable
    #[cfg(not(all(feature = "testing", not(target_arch = "wasm32"))))]
    let required_features = wgpu::Features::FLOAT32_FILTERABLE;

    anyhow::ensure!(
        adapter.features().contains(required_features),
        "GPU does not support the required features: {:?}",
        required_features - adapter.features()
    );
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("Device"),
            required_features,
            required_limits: wgpu::Limits {
                max_storage_buffer_binding_size: supported_limits.max_storage_buffer_binding_size, // the default is 128 MiB, which is too small for millions of triangles
                max_buffer_size: supported_limits.max_buffer_size,
                ..wgpu::Limits::default()
            },
            experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        })
        .await?;

    Ok((device, queue))
}
