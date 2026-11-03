//! GPU resources associated with [`super::world`].

use super::world;

// TODO: finish docs

/// Contains all GPU resources associated with [`super::world`].
pub(super) struct GpuWorld {
    pub(super) bind_group: wgpu::BindGroup,
    layout: wgpu::BindGroupLayout,
    bindings: WorldBindings,
}

/// Contains bindings in binding order.
struct WorldBindings {
    scene_geometries: wgpu::Buffer,
    scene_bvh_nodes: wgpu::Buffer,
    scene_attributes: wgpu::Buffer,
    scene_materials: wgpu::Buffer,
    scene_srgb_texture_atlases: wgpu::TextureView,
    scene_linear_texture_atlases: wgpu::TextureView,
    atlas_sampler: wgpu::Sampler,
    camera: wgpu::Buffer,
    /// Shared handle to the buffer controlled by the caller of this module; cloning does not copy GPU data.
    frame_count: wgpu::Buffer,
    skybox_texture: wgpu::TextureView,
    skybox_uniforms: wgpu::Buffer,
    skybox_sampler: wgpu::Sampler,
    scene_uniforms: wgpu::Buffer,
    scene_alias_table: wgpu::Buffer,
    skybox_alias_table: wgpu::Buffer,
}

/// Temporary construction bundle for reuse during initial upload and scene replacement.
struct SceneResources {
    geometries: wgpu::Buffer,
    attributes: wgpu::Buffer,
    materials: wgpu::Buffer,
    bvh_nodes: wgpu::Buffer,
    alias_table: wgpu::Buffer,
    srgb_texture_atlases: wgpu::TextureView,
    linear_texture_atlases: wgpu::TextureView,
}

/// Temporary construction bundle for reuse during initial upload and skybox replacement.
struct SkyboxResources {
    rgba: wgpu::TextureView,
    alias_table: wgpu::Buffer,
}

/// Private helper function to create a [`wgpu::Buffer`].
fn buffer<T: bytemuck::Pod>(
    device: &wgpu::Device,
    label: &str,
    data: &[T],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    if data.is_empty() {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: std::mem::size_of::<T>() as u64,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    } else {
        use wgpu::util::DeviceExt;
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(data),
            usage: usage | wgpu::BufferUsages::COPY_DST,
        })
    }
}

impl SceneResources {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, scene: &world::SceneUpload) -> Self {
        // handle texture atlases
        let srgb_texture_atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sRGB texture atlas array"),
            size: wgpu::Extent3d {
                width: world::ATLAS_SIZE,
                height: world::ATLAS_SIZE,
                depth_or_array_layers: scene.srgb_texture_atlases.len() as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let linear_texture_atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Linear texture atlas array"),
            size: wgpu::Extent3d {
                width: world::ATLAS_SIZE,
                height: world::ATLAS_SIZE,
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
                    texture: &srgb_texture_atlas,
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
                    bytes_per_row: Some(world::ATLAS_SIZE * 4),
                    rows_per_image: Some(world::ATLAS_SIZE),
                },
                wgpu::Extent3d {
                    width: world::ATLAS_SIZE,
                    height: world::ATLAS_SIZE,
                    depth_or_array_layers: 1,
                },
            );
        }
        // upload each linear atlas layer
        for (layer_idx, atlas_data) in scene.linear_texture_atlases.iter().enumerate() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &linear_texture_atlas,
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
                    bytes_per_row: Some(world::ATLAS_SIZE * 4),
                    rows_per_image: Some(world::ATLAS_SIZE),
                },
                wgpu::Extent3d {
                    width: world::ATLAS_SIZE,
                    height: world::ATLAS_SIZE,
                    depth_or_array_layers: 1,
                },
            );
        }

        let alias_table = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Scene aliases"),
            size: (scene.geometries.len() * std::mem::size_of::<world::GpuSceneAliasEntry>())
                as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            geometries: buffer(
                device,
                "Geometries",
                scene.geometries,
                wgpu::BufferUsages::STORAGE,
            ),
            attributes: buffer(
                device,
                "Attributes",
                scene.attributes,
                wgpu::BufferUsages::STORAGE,
            ),
            materials: buffer(
                device,
                "Materials",
                scene.materials,
                wgpu::BufferUsages::STORAGE,
            ),
            bvh_nodes: buffer(device, "BVH", scene.bvh_nodes, wgpu::BufferUsages::STORAGE),
            alias_table,
            srgb_texture_atlases: srgb_texture_atlas.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            }),
            linear_texture_atlases: linear_texture_atlas.create_view(
                &wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2Array),
                    ..Default::default()
                },
            ),
        }
    }
}

impl SkyboxResources {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, skybox: &world::SkyboxUpload) -> Self {
        // handle skybox texture
        let skybox_texture_size = wgpu::Extent3d {
            width: skybox.uniforms.width,
            height: skybox.uniforms.height,
            depth_or_array_layers: 1,
        };
        let skybox_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Skybox texture"),
            size: skybox_texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2, // 2D equirectangular
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &skybox_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(skybox.rgba),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(skybox.uniforms.width * 16), // 4 channels * 4 bytes (f32)
                rows_per_image: Some(skybox.uniforms.height),
            },
            skybox_texture_size,
        );

        Self {
            rgba: skybox_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            alias_table: buffer(
                device,
                "Skybox aliases",
                skybox.alias_table,
                wgpu::BufferUsages::STORAGE,
            ),
        }
    }
}

impl GpuWorld {
    pub(super) fn layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        // group 1 layout for compute shader: world data

        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Compute bind group 1 layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0, // geometries
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1, // bvh nodes
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2, // attributes
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
                            world::GpuCamera,
                        >()
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
                    binding: 9, // skybox texture
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true }, // feature FLOAT32_FILTERABLE is enabled, so this texture can be filtered
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 10, // skybox uniforms
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: std::num::NonZeroU64::new(16),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 11, // skybox sampler
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 12, // scene uniforms
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 13, // scene alias table
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 14, // skybox alias table
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        })
    }
}

impl WorldBindings {
    #[expect(clippy::needless_pass_by_value)] // intentionally consume the updates
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame_count: &wgpu::Buffer,
        updates: world::WorldUpdates,
    ) -> anyhow::Result<Self> {
        use anyhow::Context;

        let scene = SceneResources::new(
            device,
            queue,
            updates
                .scene
                .as_ref()
                .context("initial scene upload missing")?,
        );
        let skybox = SkyboxResources::new(
            device,
            queue,
            updates
                .skybox
                .as_ref()
                .context("initial skybox upload missing")?,
        );

        let camera = buffer(
            device,
            "Camera",
            &[updates.camera.context("initial camera upload missing")?],
            wgpu::BufferUsages::UNIFORM,
        );
        let (scene_data, aliases) = updates
            .scene_lighting
            .context("initial scene lighting missing")?;
        if !aliases.is_empty() {
            queue.write_buffer(&scene.alias_table, 0, bytemuck::cast_slice(aliases));
        }
        let scene_uniforms = buffer(
            device,
            "Scene uniforms",
            &[scene_data],
            wgpu::BufferUsages::UNIFORM,
        );
        let skybox_uniforms = buffer(
            device,
            "Skybox uniforms",
            &[updates
                .skybox_uniforms
                .context("initial skybox uniforms missing")?],
            wgpu::BufferUsages::UNIFORM,
        );
        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Atlas sampler"),
            ..Default::default()
        });
        let skybox_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Skybox sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Ok(Self {
            scene_geometries: scene.geometries,
            scene_bvh_nodes: scene.bvh_nodes,
            scene_attributes: scene.attributes,
            scene_materials: scene.materials,
            scene_srgb_texture_atlases: scene.srgb_texture_atlases,
            scene_linear_texture_atlases: scene.linear_texture_atlases,
            atlas_sampler,
            camera,
            frame_count: frame_count.clone(),
            skybox_texture: skybox.rgba,
            skybox_uniforms,
            skybox_sampler,
            scene_uniforms,
            scene_alias_table: scene.alias_table,
            skybox_alias_table: skybox.alias_table,
        })
    }

    // returns a new bind group is needed
    fn apply_updates(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        updates: world::WorldUpdates,
    ) -> bool {
        let rebind = updates.scene.is_some() || updates.skybox.is_some();

        if let Some(scene) = updates.scene {
            let scene = SceneResources::new(device, queue, &scene);
            self.scene_geometries = scene.geometries;
            self.scene_bvh_nodes = scene.bvh_nodes;
            self.scene_attributes = scene.attributes;
            self.scene_materials = scene.materials;
            self.scene_srgb_texture_atlases = scene.srgb_texture_atlases;
            self.scene_linear_texture_atlases = scene.linear_texture_atlases;
            self.scene_alias_table = scene.alias_table;
        }
        if let Some(skybox) = updates.skybox {
            let skybox = SkyboxResources::new(device, queue, &skybox);
            self.skybox_texture = skybox.rgba;
            self.skybox_alias_table = skybox.alias_table;
        }
        if let Some(camera) = updates.camera {
            queue.write_buffer(&self.camera, 0, bytemuck::bytes_of(&camera));
        }
        for (index, material) in updates.materials {
            queue.write_buffer(
                &self.scene_materials,
                (index * std::mem::size_of::<world::GpuMaterial>()) as u64,
                bytemuck::bytes_of(&material),
            );
        }
        if let Some((uniforms, aliases)) = updates.scene_lighting {
            queue.write_buffer(&self.scene_uniforms, 0, bytemuck::bytes_of(&uniforms));
            if !aliases.is_empty() {
                queue.write_buffer(&self.scene_alias_table, 0, bytemuck::cast_slice(aliases));
            }
        }
        if let Some(uniforms) = updates.skybox_uniforms {
            queue.write_buffer(&self.skybox_uniforms, 0, bytemuck::bytes_of(&uniforms));
        }

        rebind
    }

    fn create_bind_group(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Compute bind group 1"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.scene_geometries.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.scene_bvh_nodes.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.scene_attributes.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.scene_materials.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&self.scene_srgb_texture_atlases),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(
                        &self.scene_linear_texture_atlases,
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(&self.atlas_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: self.camera.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: self.frame_count.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(&self.skybox_texture),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: self.skybox_uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: wgpu::BindingResource::Sampler(&self.skybox_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: self.scene_uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 13,
                    resource: self.scene_alias_table.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 14,
                    resource: self.skybox_alias_table.as_entire_binding(),
                },
            ],
        })
    }
}

impl GpuWorld {
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: wgpu::BindGroupLayout,
        frame_count: &wgpu::Buffer,
        updates: world::WorldUpdates,
    ) -> anyhow::Result<Self> {
        let bindings = WorldBindings::new(device, queue, frame_count, updates)?;
        Ok(Self {
            bind_group: bindings.create_bind_group(device, &layout),
            layout,
            bindings,
        })
    }

    /// Apply updates given a [`world::WorldUpdates`].
    pub(super) fn apply_updates(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        updates: world::WorldUpdates,
    ) {
        if self.bindings.apply_updates(device, queue, updates) {
            self.bind_group = self.bindings.create_bind_group(device, &self.layout);
        }
    }
}
