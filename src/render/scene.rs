//! Load and format a 3D scene stored in a .glb glTF 2.0 file for passing to the GPU.

// it's all pub instead of pub(crate) in this module because criterion, an external crate, is used to benchmark stuff in this module

use anyhow::Context;

/// The size of each texture atlas in pixels.
///
/// 8192 is the default value of [`wgpu::Limits::max_texture_dimension_2d`].
/// Since passing an array of images with different sizes isn't supported universally,
/// images are packed into atlases which are larger square images that store multiple smaller images.
/// Then, an array of atlases are passed to the GPU without problems.
pub const ATLAS_SIZE: u32 = 8192;
// TODO: could be sized dynamically: after atlas construction, if there was only one atlas, shrink the atlas until it fits; there are multiple atlases, just use 8192

// TODO: fix transparency in textures; parsing is done correctly, but shader is not correct. also, in gltf::material, parse alpha_mode and alpha_cutoff to do this properly
// TODO: add vertex colors
// TODO: update wgpu to 30 once egui supports it
// TODO: allow user to use their own glb files

/// Contains all the data loaded from a glTF file and formatted for the GPU.
/// The coordinate system is: +X to the right of the screen, +Y up the screen, and -Z into the screen.
pub struct Scene {
    // geometries[i] and attributes[i] together represent a triangle at index i
    // hence, only GpuTriangleAttribute contains material index (geometries doesn't contain any index)
    pub geometries: Vec<GpuTriangleGeometry>,
    pub attributes: Vec<GpuTriangleAttribute>,
    pub materials: Vec<GpuMaterial>,
    pub bvh_nodes: Vec<GpuBvhNode>,
    /// The array of sRGB texture atlases. Each texture atlas is a RGBA8 image (stored as a [`Vec<u8>`]) that contains multiple smaller sRGB textures packed together.
    ///
    /// For locating a specific texture, the `*_tex_layer` fields in [`GpuMaterial`] are indices into this array (or the [`Scene::linear_texture_atlases`] array)
    /// of texture atlases. Then, the `*_uv` fields should be used to access the correct portion of the texture atlas.
    pub srgb_texture_atlases: Vec<Vec<u8>>,
    /// The array of linear texture atlases. Each texture atlas is a RGBA8 image (stored as a [`Vec<u8>`]) that contains multiple smaller linear textures packed together.
    ///
    /// For locating a specific texture, the `*_tex_layer` fields in [`GpuMaterial`] are indices into this array (or the [`Scene::srgb_texture_atlases`] array)
    /// of texture atlases. Then, the `*_uv` fields should be used to access the correct portion of the texture atlas.
    pub linear_texture_atlases: Vec<Vec<u8>>,
    pub camera: Camera,
    /// The initial scene data, which are used to reset the scene to its original state. Private to disallow modifications from outside this module; get with [`Scene::initial()`].
    initial: Initial,
}

/// Stores scene data that [`Scene::new()`] originally generated, before any other code modifies the public fields of these structs.
pub struct Initial {
    pub materials: Vec<GpuMaterial>,
    pub camera: Camera,
}

/// Used for keeping track of camera data internally; not passed to the GPU.
///
/// See [`GpuCamera`] for the struct that is passed to the GPU, which is converted from [`Camera`] with [`Scene::takeout_gpu_camera()`].
#[derive(Clone)]
pub struct Camera {
    /// Camera position X, Y, Z in world space.
    position: glam::Vec3,
    /// Yaw in radians.
    yaw: f32,
    /// Pitch in radians. Should be in [-π/2, π/2].
    pitch: f32,
    // focus distance is not needed since this is a pinhole camera; focus distance is implicitly 1
    /// Vertical field of view in radians. Should be in [0, π).
    fov_y: f32,
    /// Width ÷ height of the image plane.
    aspect_ratio: f32,
    /// Movement speeds for horizontal and vertical movement, respectively, in units per second.
    pub movement_speeds: [f32; 2], // these are pub because changing them doesn't cause camera to be dirty, so they can be changed freely
    /// Mouse sensitivity for rotating the camera for horizontal and vertical rotations, respectively.
    pub rotation_speeds: [f32; 2],
    /// Whether the camera has been modified. Any method that changes [`Camera`] sets this to `true`, while [`Scene::takeout_gpu_camera()`] sets this to `false` if it was `true`.
    dirty: bool,
}

/// Contains the positions of the three vertices of a triangle.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)] // Clone and Copy are required by Pod
#[cfg_attr(
    all(feature = "testing", not(target_arch = "wasm32")),
    derive(serde::Serialize)
)]
pub struct GpuTriangleGeometry {
    // bytemuck::Pod requires alignment without implicit padding
    // although glam::Vec3A is 16 byte aligned, it has padding
    // WebGPU requirements: https://www.w3.org/TR/WGSL/#alignment-and-size
    p0: glam::Vec3,
    p1: glam::Vec3,
    p2: glam::Vec3,
}

/// Contains the material index and vertex attributes of a triangle.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[cfg_attr(
    all(feature = "testing", not(target_arch = "wasm32")),
    derive(serde::Serialize)
)]
pub struct GpuTriangleAttribute {
    /// Points to the index of a [`GpuMaterial`] in [`Scene::materials`]; indicates which material this triangle uses.
    // meshes or primitives are not passed to the GPU; instead, individual triangles themselves are passed; each triangle thus has a material index
    index: u32,
    // note that these are vertex normals, separate from the normal texture or scale factor in material
    // vertex normals define the base shape, while the normal texture and scale add finer details on top of the base shape
    /// Normal vector at vertex 0. Defaults to a generated flat normal.
    n0: glam::Vec3,
    /// Normal vector at vertex 1. Defaults to a generated flat normal.
    n1: glam::Vec3,
    /// Normal vector at vertex 2. Defaults to a generated flat normal.
    n2: glam::Vec3,
    /// UV vector at vertex 0. Defaults to `(0.0, 0.0)`.
    uv0: glam::Vec2,
    /// UV vector at vertex 1. Defaults to `(1.0, 0.0)`.
    uv1: glam::Vec2,
    /// UV vector at vertex 2. Defaults to `(0.0, 1.0)`.
    uv2: glam::Vec2,
    #[expect(clippy::doc_markdown)]
    /// Tangent vector at vertex 0. Defaults to generation with MikkTSpace algorithm.
    t0: glam::Vec4,
    #[expect(clippy::doc_markdown)]
    /// Tangent vector at vertex 1. Defaults to generation with MikkTSpace algorithm.
    t1: glam::Vec4,
    #[expect(clippy::doc_markdown)]
    /// Tangent vector at vertex 2. Defaults to generation with MikkTSpace algorithm.
    t2: glam::Vec4,
}

#[expect(clippy::doc_markdown)]
/// Contains parameters for one complete material.
///
/// All data are parsed from the given .glb file. Some additional information about parameters' relationships to [OpenPBR](https://academysoftwarefoundation.github.io/OpenPBR/#parameterreference) specification are documented below.
///
/// Parameters with `OpenPBR_*` field names are all parsed from [material.extras](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#_material_extras) and are intended to be used as parameters OpenPBR specify. The defaults and ranges for these parameters follow OpenPBR specification, and are enforced for values in this struct. To ensure successful parsing of these parameters, the JSON key must match (case sensitive) the field name in this struct.
///
/// The `*_tex_layer` and `*_uv` fields are used, for example in WGSL, like so: `textureSampleLevel(srgb_texture_atlas, atlas_sampler, fma(interpolated_uv, base_color_uv.zw, base_color_uv.xy), base_color_tex_layer, 0.0)`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[cfg_attr(
    all(feature = "testing", not(target_arch = "wasm32")),
    derive(serde::Serialize)
)]
#[expect(non_snake_case)]
pub struct GpuMaterial {
    /// Base color in linear RGBA. Linear multiplier to the base color texture; if no texture, the base color is just this color.
    ///
    /// Parsed from [material.pbrMetallicRoughness.baseColorTexture](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#_material_pbrmetallicroughness_basecolorfactor).
    ///
    /// Defaults to `(1.0, 1.0, 1.0, 1.0)`; RGBA in [0, 1].
    ///
    /// Intended OpenPBR counterpart identifier: `base_color`. OpenPBR uses only RGB; default is `(0.8, 0.8, 0.8)`; RGB in [0, 1].
    pub base_color_factor: glam::Vec4,
    /// Defaults to `1.0`; in [0, 1].
    pub OpenPBR_base_weight: f32,
    /// Linear multiplier to the metallic value in the metallic-roughness texture; if no texture, this is the metallic value.
    ///
    /// Parsed from [material.pbrMetallicRoughness.metallicFactor](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#_material_pbrmetallicroughness_metallicfactor).
    ///
    /// Defaults to `1.0`; in [0, 1].
    ///
    /// Intended OpenPBR counterpart identifier: `base_metalness`. OpenPBR's default is `0.0`; in [0, 1].
    pub metallic_factor: f32,
    /// Linear multiplier to the roughness value in the metallic-roughness texture; if no texture, this is the roughness value.
    ///
    /// Parsed from [material.pbrMetallicRoughness.roughnessFactor](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#_material_pbrmetallicroughness_roughnessfactor).
    ///
    /// Defaults to `1.0`; in [0, 1].
    ///
    /// Intended OpenPBR counterpart identifier: `base_diffuse_roughness`. OpenPBR's default is `0.0`; in [0, 1].
    pub roughness_factor: f32,
    /// Defaults to `0.0`; in [0, 1].
    pub OpenPBR_emission_weight: f32,
    /// Emissive color in linear RGB and emissive strength in the 4th value. RGB is a linear multiplier to the emissive texture; if no texture, the emissive color is just this color.
    ///
    /// RGB parsed from [material.emissiveFactor](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#_material_emissivefactor) and strength from [KHR_materials_emissive_strength](https://github.com/KhronosGroup/glTF/tree/main/extensions/2.0/Khronos/KHR_materials_emissive_strength).
    ///
    /// Defaults to `(0.0, 0.0, 0.0, 1.0)`; RGB in [0, 1].
    ///
    /// Intended OpenPBR counterpart identifiers: `emission_color` and `emission_luminance`. OpenPBR's defaults are `(1.0, 1.0, 1.0)` and `1000`; RGB in [0, ∞) and luminance in [0, ∞). Luminance has units of nits.
    pub emissive_color_and_strength: glam::Vec4,
    /// Specular color in linear RGB and weight of specular in the 4th value. RGB is a linear multiplier to the specular color texture; if no texture, the specular color is just this color.
    ///
    /// Parsed from [KHR_materials_specular](https://github.com/KhronosGroup/glTF/tree/main/extensions/2.0/Khronos/KHR_materials_specular).
    ///
    /// Defaults to `(1.0, 1.0, 1.0, 1.0)`; RGB is uncapped.
    ///
    /// Intended OpenPBR counterpart identifiers: `specular_color` and `specular_weight`. OpenPBR's defaults are `(1.0, 1.0, 1.0)` and `1.0`; RGB in [0, 1] and weight in [0, ∞).
    pub specular_color_and_factor: glam::Vec4,
    /// Defaults to `0.3`; in [0, 1].
    pub OpenPBR_specular_roughness: f32,
    /// Defaults to `0.0`; in [0, 1].
    pub OpenPBR_specular_roughness_anisotropy: f32,
    /// Defaults to `0.0`; in [0, 1].
    pub OpenPBR_specular_haze: f32,
    /// Defaults to `0.3`; in [0, 1].
    pub OpenPBR_specular_haze_spread: f32,
    /// Index of refraction.
    ///
    /// Parsed from [KHR_materials_ior](https://github.com/KhronosGroup/glTF/tree/main/extensions/2.0/Khronos/KHR_materials_ior).
    ///
    /// Defaults to `1.5`; in [1, ∞).
    ///
    /// Intended OpenPBR counterpart identifier: `specular_ior`. OpenPBR's default is `1.5`; in (0, ∞).
    pub ior: f32,
    /// Defaults to `0.0`; in [0, 1].
    pub OpenPBR_specular_retroreflectivity: f32,
    /// Linear scale factor for the normal texture; if no texture, should be ignored.
    ///
    /// Parsed from [material.normalTextureInfo.scale](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#_material_normaltextureinfo_scale).
    ///
    /// Defaults to `1.0`.
    ///
    /// No intended OpenPBR counterpart.
    pub normal_scale: f32,

    pad: [f32; 3],

    /// Index into [`Scene::srgb_texture_atlases`] for the texture atlas containing the sRGB base color texture; `-1` if no texture.
    base_color_tex_layer: i32,
    /// Index into [`Scene::linear_texture_atlases`] for the texture atlas containing the linear normal texture; `-1` if no texture.
    normal_tex_layer: i32,
    /// Index into [`Scene::linear_texture_atlases`] for the texture atlas containing the linear metallic-roughness texture; `-1` if no texture.
    metallic_roughness_tex_layer: i32,
    /// Index into [`Scene::srgb_texture_atlases`] for the texture atlas containing the sRGB emissive texture; `-1` if no texture.
    emissive_tex_layer: i32,
    /// Index into [`Scene::srgb_texture_atlases`] for the texture atlas containing the sRGB specular color texture; `-1` if no texture.
    specular_color_tex_layer: i32,
    /// Index into [`Scene::linear_texture_atlases`] for the texture atlas containing the linear specular factor texture; `-1` if no texture.
    specular_factor_tex_layer: i32,

    /// Offset (x, y) and scale (x, y) for the base color texture in an atlas; if no texture, are all zeros.
    base_color_uv: glam::Vec4,
    /// Offset (x, y) and scale (x, y) for the normal texture in an atlas; if no texture, are all zeros.
    normal_uv: glam::Vec4,
    /// Offset (x, y) and scale (x, y) for the metallic-roughness texture in an atlas; if no texture, are all zeros.
    metallic_roughness_uv: glam::Vec4,
    /// Offset (x, y) and scale (x, y) for the emissive texture in an atlas; if no texture, are all zeros.
    emissive_uv: glam::Vec4,
    /// Offset (x, y) and scale (x, y) for the specular color texture in an atlas; if no texture, are all zeros.
    specular_color_uv: glam::Vec4,
    /// Offset (x, y) and scale (x, y) for the specular factor texture in an atlas; if no texture, are all zeros.
    specular_factor_uv: glam::Vec4,
}

/// A node in the BVH tree.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuBvhNode {
    /// Axis-aligned bounding box's minimum of this node.
    aabb_min: glam::Vec3,
    /// If this node is an internal node, this value is the index of the left child node in [`Scene::bvh_nodes`].
    /// If this node is a leaf ([`Self::tri_count`] is not `0`), this value is the starting offset into [`Scene::geometries`] for the triangles contained in this leaf.
    left_first: u32,
    /// Axis-aligned bounding box's maximum of this node.
    aabb_max: glam::Vec3,
    /// If this node is a leaf, this value is the number of triangles contained in this leaf.
    /// If this node is an internal node, this value is `0` and [`Self::left_first`] is the index in [`Scene::bvh_nodes`] of this node's left child.
    tri_count: u32,
}

/// Contains camera data formatted for the GPU; obtained with [`Scene::takeout_gpu_camera()`].
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
/* although the 4th value of each Vec4 is unused, if having Vec3 with matching WGSL:
struct GpuCamera {
    data: array<f32, 12>,
};
does not work for UNIFORM buffers
Error: Global variable [5] 'camera' is invalid: Alignment requirements for address space Uniform are not met by [10]The array stride 4 is not a multiple of the required alignment 16naga(15)
Even putting #[repr(C, align(16))] on this struct does not work
*/
pub struct GpuCamera {
    /// Camera position in world space.
    position: glam::Vec4,
    /// Lower-left pixel coordinate of image plane in world space minus camera position.
    lower_left_corner_minus_position: glam::Vec4,
    /// Vector that spans the full X of image plane in world space.
    horizontal: glam::Vec4,
    /// Vector that spans the full Y of image plane in world space.
    vertical: glam::Vec4,
}

/// Internal BVH helper struct.
#[derive(Clone, Copy)]
struct Aabb {
    min: glam::Vec3,
    max: glam::Vec3,
}
impl Aabb {
    const fn new() -> Self {
        Self {
            min: glam::Vec3::splat(f32::INFINITY),
            max: glam::Vec3::splat(f32::NEG_INFINITY),
        }
    }
    fn grow(&mut self, p: glam::Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }
    #[expect(clippy::use_self)]
    fn union(&mut self, other: &Aabb) {
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
    }
    fn area(&self) -> f32 {
        let e = (self.max - self.min).max(glam::Vec3::ZERO);
        e.z.mul_add(e.x, e.x.mul_add(e.y, e.y * e.z)) // e.x * e.y + e.y * e.z + e.z * e.x, but using mul_add for better precision and performance
    }
}
/// Internal BVH helper struct.
struct TriangleInfo {
    aabb: Aabb,
    centroid: glam::Vec3,
}

/// Adapter to use [`bevy_mikktspace::Geometry`] for tangent generation if needed.
struct MikkTSpaceGeometry<'a> {
    indices: &'a [u32],
    positions: &'a [glam::Vec3],
    normals: &'a [glam::Vec3],
    tex_coords: &'a [glam::Vec2],
    tangents: &'a mut [glam::Vec4],
}
impl bevy_mikktspace::Geometry for MikkTSpaceGeometry<'_> {
    fn num_faces(&self) -> usize {
        self.indices.len() / 3
    }
    fn num_vertices_of_face(&self, _face: usize) -> usize {
        3
    }
    fn position(&self, face: usize, vert: usize) -> [f32; 3] {
        self.positions[self.indices[face * 3 + vert] as usize].into()
    }
    fn normal(&self, face: usize, vert: usize) -> [f32; 3] {
        self.normals[self.indices[face * 3 + vert] as usize].into()
    }
    fn tex_coord(&self, face: usize, vert: usize) -> [f32; 2] {
        self.tex_coords[self.indices[face * 3 + vert] as usize].into()
    }
    fn set_tangent(
        &mut self,
        tangent_space: Option<bevy_mikktspace::TangentSpace>,
        face: usize,
        vert: usize,
    ) {
        let tangent = tangent_space.unwrap_or_default().tangent_encoded();
        self.tangents[face * 3 + vert] = glam::Vec4::from(tangent);
    }
}

impl Scene {
    /// Load a glTF file from the given path and format it for the GPU.
    #[allow(clippy::unused_async)] // async is used on web for getting response for loading the glb file
    #[allow(clippy::future_not_send)]
    pub async fn new(path: &str) -> anyhow::Result<Self> {
        // on both native and wasm, loading is at runtime
        // this allows the glTF file to change without having to rebuild the WASM
        #[allow(unused_mut)]
        let mut bytes;

        #[cfg(not(target_arch = "wasm32"))]
        {
            bytes = std::fs::read(path)?;
        }

        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsCast; // trait for dyn_into()

            let response: web_sys::Response = wasm_bindgen_futures::JsFuture::from(
                web_sys::window()
                    .context("get web_sys window failed")?
                    .fetch_with_str(path),
            )
            .await
            .map_err(|e| anyhow::anyhow!("error fetching {path}: {e:?}"))?
            .dyn_into()
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
            bytes = vec![0u8; u8_array.length() as usize];
            u8_array.copy_to(&mut bytes[..]);
        }

        let (document, buffers, images) = gltf::import_slice(bytes)?;

        let mut geometries = Vec::new();
        let mut attributes = Vec::new();
        let mut materials = Vec::new();
        let mut tri_infos = Vec::new();

        // see enabled extensions in Cargo.toml
        // TODO: add KHR_texture_transform
        document.extensions_used().for_each(|s| {
            log::info!("glTF includes extension: {s}");
        });

        // collect indices of images designated as sRGB per glTF spec
        // only base color, emissive, and specular color textures are sRGB in glTF
        let mut srgb_images = std::collections::HashSet::new();
        for material in document.materials() {
            if let Some(tex) = material.pbr_metallic_roughness().base_color_texture() {
                srgb_images.insert(tex.texture().source().index());
            }
            if let Some(tex) = material.emissive_texture() {
                srgb_images.insert(tex.texture().source().index());
            }
            if let Some(specular) = material.specular()
                && let Some(tex) = specular.specular_color_texture()
            {
                srgb_images.insert(tex.texture().source().index());
            }
        }

        struct AtlasLayer {
            allocator: guillotiere::AtlasAllocator,
            pixels: Vec<u8>,
        }
        // closure for making a new AtlasLayer
        let new_layer = || AtlasLayer {
            allocator: guillotiere::AtlasAllocator::new(guillotiere::size2(
                ATLAS_SIZE as i32,
                ATLAS_SIZE as i32,
            )),
            pixels: vec![0; (ATLAS_SIZE * ATLAS_SIZE * 4) as usize],
        };

        // both arrays of atlases have at least 1 atlas (even if there are no sRGB textures or no linear textures),
        // so we call wgpu device.create_texture() with depth_or_array_layers of 0
        let mut srgb_atlases = vec![new_layer()];
        let mut linear_atlases = vec![new_layer()];

        let mut srgb_uvs = std::collections::HashMap::new();
        let mut linear_uvs = std::collections::HashMap::new();

        let mut rgba = Vec::new(); // declare once to save performance

        // loop through each image
        for (img_idx, image) in images.iter().enumerate() {
            rgba.clear();

            #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
            log::info!("Converting image with format {:?} to RGBA8", image.format);

            // convert image into RGBA8
            // most textures from most models are 8 bit; higher bit depth textures are uncommon
            use gltf::image::Format;
            match image.format {
                Format::R8 => rgba.extend(image.pixels.iter().flat_map(|&r| [r, r, r, 255])),
                Format::R8G8 => rgba.extend(
                    image
                        .pixels
                        .chunks_exact(2)
                        .flat_map(|rg| [rg[0], rg[1], 0, 255]),
                ),
                Format::R8G8B8 => rgba.extend(
                    image
                        .pixels
                        .chunks_exact(3)
                        .flat_map(|rgb| [rgb[0], rgb[1], rgb[2], 255]),
                ),
                Format::R8G8B8A8 => rgba.extend(image.pixels.iter().copied()),

                // data is guaranteed to be little-endian in glTF, so from_le_bytes isn't needed
                Format::R16 => rgba.extend(
                    image
                        .pixels
                        .chunks_exact(2)
                        .flat_map(|r| [r[1], r[1], r[1], 255]),
                ),
                Format::R16G16 => rgba.extend(
                    image
                        .pixels
                        .chunks_exact(4)
                        .flat_map(|rg| [rg[1], rg[3], 0, 255]),
                ),
                Format::R16G16B16 => rgba.extend(
                    image
                        .pixels
                        .chunks_exact(6)
                        .flat_map(|rgb| [rgb[1], rgb[3], rgb[5], 255]),
                ),
                Format::R16G16B16A16 => rgba.extend(
                    image
                        .pixels
                        .chunks_exact(8)
                        .flat_map(|rgba| [rgba[1], rgba[3], rgba[5], rgba[7]]),
                ),
                _ => {
                    log::warn!(
                        "Unsupported image format {:?}, using fallback opaque white texture",
                        image.format
                    );
                    rgba.extend(vec![255; (image.width * image.height * 4) as usize]);
                }
            }

            let (atlases, uvs) = if srgb_images.contains(&img_idx) {
                (&mut srgb_atlases, &mut srgb_uvs)
            } else {
                (&mut linear_atlases, &mut linear_uvs)
            };

            // save parsed images for testing
            // #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
            // Self::save_rgba_image(
            //     &format!("parsed_image_{img_idx}.png"),
            //     &rgba,
            //     image.width,
            //     image.height,
            // );

            let size = guillotiere::size2(image.width.cast_signed(), image.height.cast_signed());
            let mut allocation = None;
            let mut layer_idx = 0;

            // try to find a layer that has enough space
            for (i, layer) in atlases.iter_mut().enumerate() {
                if let Some(alloc) = layer.allocator.allocate(size) {
                    allocation = Some(alloc);
                    layer_idx = i;
                    break;
                }
            }

            // if all atlases are full, create a new atlas
            if allocation.is_none() {
                let mut layer = new_layer();
                allocation = layer.allocator.allocate(size);
                layer_idx = atlases.len();
                atlases.push(layer);
            }

            let alloc = allocation.context("single image is larger than ATLAS_SIZE atlas")?;
            let rect = alloc.rectangle;

            // blit the image into the atlas buffer
            let layer = &mut atlases[layer_idx];
            for y in 0..image.height.cast_signed() {
                let src_start = (y * image.width.cast_signed() * 4) as usize;
                let src_end = src_start + (image.width.cast_signed() * 4) as usize;

                let dst_start =
                    ((rect.min.y + y) * ATLAS_SIZE as i32 * 4 + rect.min.x * 4) as usize;
                let dst_end = dst_start + (image.width.cast_signed() * 4) as usize;

                layer.pixels[dst_start..dst_end].copy_from_slice(&rgba[src_start..src_end]);
            }

            // calculate scale and offset in the 0.0 -> 1.0 range based on the atlas
            let uv_offset_scale = glam::Vec4::new(
                rect.min.x as f32 / ATLAS_SIZE as f32,   // offset x
                rect.min.y as f32 / ATLAS_SIZE as f32,   // offset y
                image.width as f32 / ATLAS_SIZE as f32,  // scale x
                image.height as f32 / ATLAS_SIZE as f32, // scale y
            );

            uvs.insert(img_idx, (layer_idx as i32, uv_offset_scale));
        }

        // collect root nodes to begin traversal
        let root_nodes = document.default_scene().map_or_else(
            || document.nodes().collect::<Vec<_>>(),
            |scene| scene.nodes().collect::<Vec<_>>(),
        );

        // initialize the stack for iterative node processing
        let mut stack = Vec::new();
        for node in root_nodes {
            stack.push((node, glam::Mat4::IDENTITY));
        }

        // used so that primitives with the same material can reuse the same GpuMaterial instead of storing duplicate materials in the Scene
        let mut material_cache = std::collections::HashMap::new();

        // temporary vecs for processing each primitive; declared once here to save on performance of repeated allocations; these are cleared and reused for each primitive
        let mut temp_positions = Vec::new();
        let mut temp_normals = Vec::new();
        let mut temp_tex_coords = Vec::new();
        let mut temp_indices = Vec::new();
        let mut temp_tangents = Vec::new();
        // used if tangents are calculated using mikktspace
        let mut computed_tangents = Vec::new();

        const DUMMY_UV: [glam::Vec2; 3] = [
            glam::Vec2::new(0.0, 0.0),
            glam::Vec2::new(1.0, 0.0),
            glam::Vec2::new(0.0, 1.0),
        ];

        // iteratively process all nodes with a stack
        while let Some((node, parent_mat)) = stack.pop() {
            // compute this node's transform matrix
            let local_mat = match node.transform() {
                gltf::scene::Transform::Matrix { matrix } => {
                    glam::Mat4::from_cols_array_2d(&matrix)
                }
                gltf::scene::Transform::Decomposed {
                    translation,
                    rotation,
                    scale,
                } => {
                    glam::Mat4::from_translation(glam::Vec3::from(translation))
                        * glam::Mat4::from_quat(glam::Quat::from_array(rotation))
                        * glam::Mat4::from_scale(glam::Vec3::from(scale))
                }
            };

            let model_mat = parent_mat * local_mat;

            // get the mesh data if this node references a mesh
            if let Some(mesh) = node.mesh() {
                // only invert/transpose the matrix if the node actually has geometry
                let normal_mat = model_mat.inverse().transpose();

                // primitives are the only useful data in a gltf::Mesh for this renderer
                for primitive in mesh.primitives() {
                    // only process the primitive if it has triangles
                    if primitive.mode() == gltf::mesh::Mode::Triangles {
                        // a reader can read the data of a primitive
                        let reader =
                            primitive.reader(|buffer| Some(&buffers[buffer.index()].0[..]));

                        temp_positions.clear();
                        if let Some(it) = reader.read_positions() {
                            temp_positions.extend(
                                it.map(|p| model_mat.transform_point3(glam::Vec3::from_array(p))),
                            );
                        } else {
                            continue; // skip if no vertex positions
                        }

                        temp_indices.clear();
                        if let Some(it) = reader.read_indices() {
                            temp_indices.extend(it.into_u32());
                        } else {
                            temp_indices.extend(0..temp_positions.len() as u32);
                        }

                        temp_tex_coords.clear();
                        if let Some(it) = reader.read_tex_coords(0) {
                            temp_tex_coords.extend(it.into_f32().map(glam::Vec2::from_array));
                        } else {
                            temp_tex_coords
                                .extend(DUMMY_UV.iter().cycle().take(temp_positions.len()));
                        }

                        temp_normals.clear();
                        temp_tangents.clear();

                        let use_computed_tangents;

                        if let Some(it) = reader.read_normals() {
                            temp_normals.extend(it.map(|n| {
                                normal_mat
                                    .transform_vector3(glam::Vec3::from_array(n))
                                    .normalize()
                            }));
                            if let Some(it) = reader.read_tangents() {
                                let mut has_bad_tangent = false;

                                for t in it {
                                    #[expect(clippy::float_cmp)]
                                    if t == [1.0, 0.0, 0.0, 1.0] {
                                        // a tangent of (1.0, 0.0, 0.0, 1.0) might be correct and intentional, but it's more often a placeholder tangent, so we don't use it
                                        has_bad_tangent = true;
                                        break;
                                    }
                                    let t_vec = glam::Vec4::from_array(t);
                                    temp_tangents.push(
                                        model_mat
                                            .transform_vector3(t_vec.truncate())
                                            .normalize()
                                            .extend(t_vec.w),
                                    );
                                }

                                if !has_bad_tangent {
                                    for chunk in temp_indices.chunks_exact(3) {
                                        let w0 = temp_tangents[chunk[0] as usize].w;
                                        let w1 = temp_tangents[chunk[1] as usize].w;
                                        let w2 = temp_tangents[chunk[2] as usize].w;

                                        #[expect(clippy::float_cmp)]
                                        if w0 != w1 || w0 != w2 {
                                            has_bad_tangent = true;
                                            break;
                                        }
                                    }
                                }

                                if has_bad_tangent {
                                    temp_tangents.clear();
                                    use_computed_tangents = true;
                                } else {
                                    use_computed_tangents = false;
                                }
                            } else {
                                use_computed_tangents = true;
                            }
                        } else {
                            // if normals are not given, generate flat normals
                            // don't even bother trying to parse tangents, since glTF states that if normals aren't given but tangents are, the given tangents should be ignored
                            // indicate to generate tangents later on
                            temp_normals.resize(temp_positions.len(), glam::Vec3::ZERO);
                            for chunk in temp_indices.chunks_exact(3) {
                                let [i0, i1, i2] =
                                    [chunk[0] as usize, chunk[1] as usize, chunk[2] as usize];
                                let n = (temp_positions[i1] - temp_positions[i0])
                                    .cross(temp_positions[i2] - temp_positions[i0])
                                    .normalize();
                                temp_normals[i0] = n;
                                temp_normals[i1] = n;
                                temp_normals[i2] = n;
                            }
                            use_computed_tangents = true;
                        }

                        computed_tangents.clear();
                        if use_computed_tangents {
                            // compute tangents with mikktspace
                            computed_tangents.resize(temp_indices.len(), glam::Vec4::ZERO);

                            bevy_mikktspace::generate_tangents(&mut MikkTSpaceGeometry {
                                indices: &temp_indices,
                                positions: &temp_positions,
                                normals: &temp_normals,
                                tex_coords: &temp_tex_coords,
                                tangents: &mut computed_tangents,
                            })?;

                            // the handedness is negated because of coordinate system nuances
                            // test these to verify that negation works:
                            // https://github.com/KhronosGroup/glTF-Sample-Assets/blob/main/Models/NormalTangentMirrorTest/README.md
                            // https://github.com/KhronosGroup/glTF-Sample-Assets/blob/main/Models/NormalTangentTest/README.md
                            for t in &mut computed_tangents {
                                t.w = -t.w;
                            }
                        }

                        let mat = primitive.material();

                        let material_index = if let Some(idx) = mat.index() {
                            // if idx is already in material_cache, return material_cache[idx]
                            // note that material_cache stores keys which match the original material index from the glTF file,
                            // and values which are the corresponding index in the Scene::materials vec for the GpuMaterial
                            // this allows primitives with the same material to reuse the same GpuMaterial in Scene instead of storing duplicate GpuMaterials
                            // if idx is not in material_cache, run the closure, which also stores the new index in material_cache since or_insert_with returns &mut material_cache[idx]
                            *material_cache.entry(idx).or_insert_with(|| {
                                let new_idx = materials.len() as u32;
                                materials.push(Self::parse_gpu_material(
                                    &mat,
                                    &srgb_uvs,
                                    &linear_uvs,
                                ));
                                new_idx
                            })
                        } else {
                            // if there's no index, caching is impossible, so just parse the material and push it to the materials vec without caching
                            let new_idx = materials.len() as u32;
                            materials.push(Self::parse_gpu_material(&mat, &srgb_uvs, &linear_uvs));
                            new_idx
                        };

                        for (tri, chunk) in temp_indices.chunks_exact(3).enumerate() {
                            let [i0, i1, i2] =
                                [chunk[0] as usize, chunk[1] as usize, chunk[2] as usize];

                            let p0 = temp_positions[i0];
                            let p1 = temp_positions[i1];
                            let p2 = temp_positions[i2];

                            let (t0, t1, t2) = if use_computed_tangents {
                                let base = tri * 3;
                                (
                                    computed_tangents[base],
                                    computed_tangents[base + 1],
                                    computed_tangents[base + 2],
                                )
                            } else {
                                (temp_tangents[i0], temp_tangents[i1], temp_tangents[i2])
                            };

                            // calculate AABB and centroid of this triangle for the BVH builder
                            tri_infos.push(TriangleInfo {
                                aabb: Aabb {
                                    min: p0.min(p1).min(p2),
                                    max: p0.max(p1).max(p2),
                                },
                                centroid: (p0 + p1 + p2) / 3.0,
                            });

                            geometries.push(GpuTriangleGeometry { p0, p1, p2 });

                            attributes.push(GpuTriangleAttribute {
                                index: material_index,
                                n0: temp_normals[i0],
                                n1: temp_normals[i1],
                                n2: temp_normals[i2],
                                uv0: temp_tex_coords[i0],
                                uv1: temp_tex_coords[i1],
                                uv2: temp_tex_coords[i2],
                                t0,
                                t1,
                                t2,
                            });
                        }
                    } else {
                        log::info!(
                            "Mesh mode is not triangles, skipping primitive with mode {:?}",
                            primitive.mode()
                        );
                    }
                }
            }

            // push all children to the stack to be processed next
            stack.extend(node.children().map(|child| (child, model_mat)));
        }

        let mut bvh_nodes = vec![GpuBvhNode {
            aabb_min: glam::Vec3::ZERO,
            left_first: 0,
            aabb_max: glam::Vec3::ZERO,
            tri_count: 0,
        }];

        log::info!(
            "Loaded scene with {} triangles, {} materials, {} linear and {} sRGB texture atlases",
            geometries.len(),
            materials.len(),
            linear_atlases.len(),
            srgb_atlases.len(),
        );

        let bvh_construction_start = web_time::Instant::now();

        // start recursive BVH construction
        if !tri_infos.is_empty() {
            let tri_count = tri_infos.len();
            Self::update_node_bounds(0, &mut bvh_nodes, &tri_infos, 0, tri_count);
            Self::subdivide(
                0,
                &mut bvh_nodes,
                &mut tri_infos,
                &mut geometries,
                &mut attributes,
                0,
                tri_count,
            );
        }

        log::info!(
            "Finished BVH construction in {:?} with {} nodes",
            bvh_construction_start.elapsed(),
            bvh_nodes.len()
        );

        let camera = Camera::new(bvh_nodes[0].aabb_min, bvh_nodes[0].aabb_max);

        let initial = Initial {
            materials: materials.clone(),
            camera: camera.clone(),
        };

        let scene = Self {
            geometries,
            attributes,
            materials,
            bvh_nodes,
            srgb_texture_atlases: srgb_atlases.into_iter().map(|l| l.pixels).collect(), // map out just the pixels from the AtlasLayer structs
            linear_texture_atlases: linear_atlases.into_iter().map(|l| l.pixels).collect(),
            camera,
            initial,
        };

        // save atlas images and scene data for testing
        #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
        {
            for (idx, pixels) in scene.srgb_texture_atlases.iter().enumerate() {
                Self::save_rgba_image(
                    &format!("srgb_atlas_{idx}.png"),
                    pixels,
                    ATLAS_SIZE,
                    ATLAS_SIZE,
                );
            }
            for (idx, pixels) in scene.linear_texture_atlases.iter().enumerate() {
                Self::save_rgba_image(
                    &format!("linear_atlas_{idx}.png"),
                    pixels,
                    ATLAS_SIZE,
                    ATLAS_SIZE,
                );
            }
            scene.save_scene_data_json();
        }

        Ok(scene)
    }

    /// Convert a glTF material into a [`GpuMaterial`] for passing to the GPU, using the provided mapping of image indices to atlas layers and UV offset/scale.
    // not a closure inside new() because using a function seems slightly faster
    fn parse_gpu_material(
        mat: &gltf::Material,
        srgb_image_uvs: &std::collections::HashMap<usize, (i32, glam::Vec4)>,
        linear_image_uvs: &std::collections::HashMap<usize, (i32, glam::Vec4)>,
    ) -> GpuMaterial {
        let get_srgb_layer_and_uv = |img_idx_opt: Option<usize>| -> (i32, glam::Vec4) {
            img_idx_opt
                .and_then(|idx| srgb_image_uvs.get(&idx).copied())
                .unwrap_or((-1, glam::Vec4::ZERO)) // return -1 layer and zero UV if no texture attached
        };
        let get_linear_layer_and_uv = |img_idx_opt: Option<usize>| -> (i32, glam::Vec4) {
            img_idx_opt
                .and_then(|idx| linear_image_uvs.get(&idx).copied())
                .unwrap_or((-1, glam::Vec4::ZERO))
        };

        let pbr = mat.pbr_metallic_roughness();

        let (base_color_tex_layer, base_color_uv) = get_srgb_layer_and_uv(
            pbr.base_color_texture()
                .map(|t| t.texture().source().index()),
        );

        let (metallic_roughness_tex_layer, metallic_roughness_uv) = get_linear_layer_and_uv(
            pbr.metallic_roughness_texture()
                .map(|t| t.texture().source().index()),
        );

        let norm_tex = mat.normal_texture();
        let (normal_tex_layer, normal_uv) =
            get_linear_layer_and_uv(norm_tex.as_ref().map(|t| t.texture().source().index()));

        let (emissive_tex_layer, emissive_uv) =
            get_srgb_layer_and_uv(mat.emissive_texture().map(|t| t.texture().source().index()));

        let specular = mat.specular();
        let (specular_factor_tex_layer, specular_factor_uv) = get_linear_layer_and_uv(
            specular
                .as_ref()
                .and_then(gltf::material::Specular::specular_texture)
                .map(|t| t.texture().source().index()),
        );
        let (specular_color_tex_layer, specular_color_uv) = get_srgb_layer_and_uv(
            specular
                .as_ref()
                .and_then(gltf::material::Specular::specular_color_texture)
                .map(|t| t.texture().source().index()),
        );

        #[expect(non_snake_case)]
        let mut OpenPBR_base_weight = 1.0;
        #[expect(non_snake_case)]
        let mut OpenPBR_emission_weight = 0.0;
        #[expect(non_snake_case)]
        let mut OpenPBR_specular_roughness = 0.3;
        #[expect(non_snake_case)]
        let mut OpenPBR_specular_roughness_anisotropy = 0.0;
        #[expect(non_snake_case)]
        let mut OpenPBR_specular_haze = 0.0;
        #[expect(non_snake_case)]
        let mut OpenPBR_specular_haze_spread = 0.3;
        #[expect(non_snake_case)]
        let mut OpenPBR_specular_retroreflectivity = 0.0;
        if let Some(extras) = mat.extras() {
            match serde_json::from_str::<serde_json::Value>(extras.get()) {
                Ok(serde_json::Value::Object(map)) => {
                    // for now, all these OpenPBR values from extras are f32 in [0, 1]
                    let apply_extra = |key: &str, val: &serde_json::Value, target: &mut f32| {
                        if let Some(v_f64) = val.as_f64() {
                            let v_f32 = v_f64 as f32;
                            if (0.0..=1.0).contains(&v_f32) {
                                *target = v_f32;
                            } else {
                                log::warn!("{key} value {v_f32} is out of range [0, 1]");
                            }
                        } else {
                            log::warn!("{key} value is not a float");
                        }
                    };

                    for (k, v) in map {
                        match k.as_str() {
                            "OpenPBR_base_weight" => apply_extra(&k, &v, &mut OpenPBR_base_weight),
                            "OpenPBR_emission_weight" => {
                                apply_extra(&k, &v, &mut OpenPBR_emission_weight);
                            }
                            "OpenPBR_specular_roughness" => {
                                apply_extra(&k, &v, &mut OpenPBR_specular_roughness);
                            }
                            "OpenPBR_specular_roughness_anisotropy" => {
                                apply_extra(&k, &v, &mut OpenPBR_specular_roughness_anisotropy);
                            }
                            "OpenPBR_specular_haze" => {
                                apply_extra(&k, &v, &mut OpenPBR_specular_haze);
                            }
                            "OpenPBR_specular_haze_spread" => {
                                apply_extra(&k, &v, &mut OpenPBR_specular_haze_spread);
                            }
                            "OpenPBR_specular_retroreflectivity" => {
                                apply_extra(&k, &v, &mut OpenPBR_specular_retroreflectivity);
                            }
                            _ => log::warn!("Unrecognized key in pbrMetallicRoughness.extras: {k}"),
                        }
                    }
                }
                Ok(_) => {
                    log::warn!("pbrMetallicRoughness.extras is not a JSON object");
                }
                Err(e) => {
                    log::warn!("Failed to parse pbrMetallicRoughness.extras: {e}");
                }
            }
        }

        GpuMaterial {
            base_color_factor: glam::Vec4::from(pbr.base_color_factor()),
            OpenPBR_base_weight,
            metallic_factor: pbr.metallic_factor(),
            roughness_factor: pbr.roughness_factor(),
            OpenPBR_emission_weight,
            emissive_color_and_strength: glam::Vec4::from((
                glam::Vec3::from(mat.emissive_factor()),
                mat.emissive_strength().unwrap_or(1.0),
            )),
            specular_color_and_factor: glam::Vec4::from((
                glam::Vec3::from(specular.as_ref().map_or(
                    [1.0, 1.0, 1.0],
                    gltf::material::Specular::specular_color_factor,
                )),
                specular.map_or(1.0, |s| s.specular_factor()),
            )),
            OpenPBR_specular_roughness,
            OpenPBR_specular_roughness_anisotropy,
            OpenPBR_specular_haze,
            OpenPBR_specular_haze_spread,
            ior: mat.ior().unwrap_or(1.5),
            OpenPBR_specular_retroreflectivity,
            normal_scale: norm_tex.map_or(1.0, |t| t.scale()),

            pad: [0.0; 3],

            base_color_tex_layer,
            normal_tex_layer,
            metallic_roughness_tex_layer,
            emissive_tex_layer,
            specular_factor_tex_layer,
            specular_color_tex_layer,

            base_color_uv,
            normal_uv,
            metallic_roughness_uv,
            emissive_uv,
            specular_factor_uv,
            specular_color_uv,
        }
    }

    /// Update the BVH node's bounding box based on the triangles it spans.
    fn update_node_bounds(
        node_idx: usize,
        nodes: &mut [GpuBvhNode],
        tri_infos: &[TriangleInfo],
        start: usize,
        end: usize,
    ) {
        let mut aabb = Aabb::new();
        for info in &tri_infos[start..end] {
            aabb.union(&info.aabb);
        }
        nodes[node_idx].aabb_min = aabb.min;
        nodes[node_idx].aabb_max = aabb.max;
    }

    /// Recursive binning SAH sub-divider.
    ///
    /// `node_idx` is the index of the current node in `nodes`, and `start` and `end` specify the range of triangles in `tri_infos`, `geometries`, and `attributes` that this node spans.
    fn subdivide(
        node_idx: usize,
        nodes: &mut Vec<GpuBvhNode>,
        tri_infos: &mut [TriangleInfo],
        geometries: &mut [GpuTriangleGeometry],
        attributes: &mut [GpuTriangleAttribute],
        start: usize,
        end: usize,
    ) {
        let tri_count = end - start;
        // if there are 2 or fewer triangles, make this node a leaf; otherwise, keep subdividing
        if tri_count <= 2 {
            nodes[node_idx].left_first = start as u32;
            nodes[node_idx].tri_count = tri_count as u32;
            return;
        }

        // split based on not the edges of triangles; calculate bounding box that encapsulates only the centroids
        let mut centroid_bounds = Aabb::new();
        for info in &tri_infos[start..end] {
            centroid_bounds.grow(info.centroid);
        }

        const BINS: usize = 8;
        let mut best_axis = 0;
        let mut best_split = 0;
        let mut best_cost = f32::MAX;

        for axis in 0..3 {
            // for axis in x, y, z
            let bounds_min = centroid_bounds.min[axis];
            let bounds_max = centroid_bounds.max[axis];
            #[expect(clippy::float_cmp)] // there should be no precision drift here
            if bounds_min == bounds_max {
                continue;
            } // all triangle centroids are overlapping on this axis

            let scale = BINS as f32 / (bounds_max - bounds_min);

            #[derive(Clone, Copy)]
            struct Bin {
                count: u32,
                bounds: Aabb,
            }
            let mut bins = [Bin {
                count: 0,
                bounds: Aabb::new(),
            }; BINS];

            for info in &tri_infos[start..end] {
                let centroid = info.centroid[axis];
                let mut bin_idx = ((centroid - bounds_min) * scale) as usize;
                bin_idx = bin_idx.min(BINS - 1);
                bins[bin_idx].count += 1;
                bins[bin_idx].bounds.union(&info.aabb);
            }

            let mut left_area = [0.0; BINS - 1];
            let mut left_count = [0; BINS - 1];
            let mut right_area = [0.0; BINS - 1];
            let mut right_count = [0; BINS - 1];

            let mut left_box = Aabb::new();
            let mut left_sum = 0;
            for i in 0..BINS - 1 {
                left_sum += bins[i].count;
                left_box.union(&bins[i].bounds);
                left_count[i] = left_sum;
                left_area[i] = left_box.area();
            }

            let mut right_box = Aabb::new();
            let mut right_sum = 0;
            for i in (1..BINS).rev() {
                right_sum += bins[i].count;
                right_box.union(&bins[i].bounds);
                right_count[i - 1] = right_sum;
                right_area[i - 1] = right_box.area();
            }

            for i in 0..BINS - 1 {
                let cost = (left_count[i] as f32)
                    .mul_add(left_area[i], right_count[i] as f32 * right_area[i]);
                if cost < best_cost {
                    best_cost = cost;
                    best_axis = axis;
                    best_split = i;
                }
            }
        }

        let node_area = {
            let e = nodes[node_idx].aabb_max - nodes[node_idx].aabb_min;
            e.z.mul_add(e.x, e.x.mul_add(e.y, e.y * e.z)) // omitting 2.0 coefficient
        };
        let leaf_cost = tri_count as f32 * node_area;

        // if making it a leaf is cheaper than the best SAH split, terminate here
        if best_cost >= leaf_cost {
            nodes[node_idx].left_first = start as u32;
            nodes[node_idx].tri_count = tri_count as u32;
            return;
        }

        let bounds_min = centroid_bounds.min[best_axis];
        let bounds_max = centroid_bounds.max[best_axis];
        let scale = BINS as f32 / (bounds_max - bounds_min);

        let mut left = start;
        let mut right = end - 1;

        while left <= right {
            let centroid = tri_infos[left].centroid[best_axis];
            let mut bin_idx = ((centroid - bounds_min) * scale) as usize;
            bin_idx = bin_idx.min(BINS - 1);

            if bin_idx <= best_split {
                left += 1;
            } else {
                tri_infos.swap(left, right);
                geometries.swap(left, right);
                attributes.swap(left, right);
                if right == 0 {
                    break;
                } // safe guard against underflow 
                right -= 1;
            }
        }

        let split_idx = left;

        // edge case: floats caused a weird partition leaving one side completely empty
        if split_idx == start || split_idx == end {
            nodes[node_idx].left_first = start as u32;
            nodes[node_idx].tri_count = tri_count as u32;
            return;
        }

        // create child nodes contiguously (left, right)
        let left_child_idx = nodes.len();
        nodes.push(GpuBvhNode {
            aabb_min: glam::Vec3::ZERO,
            left_first: 0,
            aabb_max: glam::Vec3::ZERO,
            tri_count: 0,
        });
        let right_child_idx = nodes.len();
        nodes.push(GpuBvhNode {
            aabb_min: glam::Vec3::ZERO,
            left_first: 0,
            aabb_max: glam::Vec3::ZERO,
            tri_count: 0,
        });

        nodes[node_idx].left_first = left_child_idx as u32;
        nodes[node_idx].tri_count = 0; // 0 signals non-leaf internal node

        Self::update_node_bounds(left_child_idx, nodes, tri_infos, start, split_idx);
        Self::update_node_bounds(right_child_idx, nodes, tri_infos, split_idx, end);

        Self::subdivide(
            left_child_idx,
            nodes,
            tri_infos,
            geometries,
            attributes,
            start,
            split_idx,
        );
        Self::subdivide(
            right_child_idx,
            nodes,
            tri_infos,
            geometries,
            attributes,
            split_idx,
            end,
        );
    }

    /// Get the scene data that [`Scene::new()`] originally generated.
    // there's a getter rather than a direct pub field access because we want to make sure that Initial isn't modified outside of Scene
    pub const fn initial(&self) -> &Initial {
        &self.initial
    }

    /// Get the [`GpuCamera`] struct for the GPU with the current [`Camera`] values and determine which triangle the camera is directly pointed at, returning that triangle's material's index in [`Scene::materials`].
    ///
    /// Returns [`None`] and does no calculations if camera wasn't dirty. Otherwise, calculates fields in [`GpuCamera`], then sets camera dirty to false.
    // not a method in impl Camera because this method also needs read access to Scene stuff
    // takes &mut to self because it needs to set self.camera.dirty to false
    pub fn takeout_gpu_camera(&mut self) -> Option<(GpuCamera, Option<usize>)> {
        if !self.camera.dirty {
            return None;
        }

        // calculate GpuCamera fields

        let rotation_mat = glam::Mat3::from_rotation_y(self.camera.yaw)
            * glam::Mat3::from_rotation_x(self.camera.pitch);

        let image_plane_height = 2.0 * (self.camera.fov_y * 0.5).tan();

        let horizontal3 = rotation_mat.mul_vec3(glam::Vec3::X).normalize()
            * self.camera.aspect_ratio
            * image_plane_height;

        let vertical3 = rotation_mat.mul_vec3(glam::Vec3::NEG_Y).normalize() * image_plane_height;

        // create this value just for optimization for reuse when picking material
        let vertical3_plus_horizontal3_half = (vertical3 + horizontal3) * 0.5;

        let forward3 = rotation_mat.mul_vec3(glam::Vec3::NEG_Z).normalize();

        let lower_left_corner_minus_position3 = forward3 - vertical3_plus_horizontal3_half;

        // use GpuCamera fields to pick material
        // ray-triangle intersection below adopted from WGSL

        let dir = (lower_left_corner_minus_position3 + vertical3_plus_horizontal3_half).normalize();
        let dir_recip = dir.recip(); // division by zero can happen

        let d = dir.to_array();
        let abs_d = dir.abs().to_array();

        let kxy = usize::from(abs_d[0] <= abs_d[1]);
        let kz = if abs_d[kxy] > abs_d[2] { kxy } else { 2 };

        let kx_val = if kz == 2 { 0 } else { kz + 1 };
        let ky_val = if kx_val == 2 { 0 } else { kx_val + 1 };

        let is_neg = d[kz] < 0.0;
        let kx = if is_neg { ky_val } else { kx_val };
        let ky = if is_neg { kx_val } else { ky_val };

        let sz = dir_recip.to_array()[kz];
        let sx = -d[kx] * sz;
        let sy = -d[ky] * sz;

        let mut stack = [0u32; 256];
        let mut stack_ptr = 0;

        stack[stack_ptr] = 0;
        stack_ptr += 1;

        let mut closest_t = f32::MAX;
        let mut hit_index: Option<usize> = None;

        while stack_ptr > 0 {
            stack_ptr -= 1;
            let node = self.bvh_nodes[stack[stack_ptr] as usize];

            let t0 = (node.aabb_min - self.camera.position) * dir_recip;
            let t1 = (node.aabb_max - self.camera.position) * dir_recip;

            let t_near = t0.min(t1).max_element();
            let t_far = t0.max(t1).min_element();

            if !(t_near <= t_far && t_far > 0.0 && t_near < closest_t) {
                continue;
            }

            if node.tri_count > 0 {
                for i in 0..node.tri_count {
                    let tri_index = (node.left_first + i) as usize;
                    let tri = self.geometries[tri_index];

                    let a = (tri.p0 - self.camera.position).to_array();
                    let b = (tri.p1 - self.camera.position).to_array();
                    let c = (tri.p2 - self.camera.position).to_array();

                    let ax = sx * a[kz] + a[kx];
                    let ay = sy * a[kz] + a[ky];
                    let bx = sx * b[kz] + b[kx];
                    let by = sy * b[kz] + b[ky];
                    let cx = sx * c[kz] + c[kx];
                    let cy = sy * c[kz] + c[ky];

                    let u = cx * by - cy * bx;
                    let v = ax * cy - ay * cx;
                    let w = bx * ay - by * ax;

                    if u.min(v.min(w)) < 0.0 && u.max(v.max(w)) > 0.0 {
                        continue;
                    }

                    let det = u + v + w;
                    if det == 0.0 {
                        continue;
                    }

                    let t = (w * sz).mul_add(c[kz], (u * sz).mul_add(a[kz], v * sz * b[kz])) / det;

                    if t > 0.0 && t < closest_t {
                        closest_t = t;
                        hit_index = Some(tri_index);
                    }
                }
            } else {
                assert!(stack_ptr + 2 <= stack.len(), "BVH traversal stack overflow");

                stack[stack_ptr] = node.left_first + 1;
                stack_ptr += 1;
                stack[stack_ptr] = node.left_first;
                stack_ptr += 1;
            }
        }

        self.camera.dirty = false;

        Some((
            GpuCamera {
                position: glam::Vec4::from((self.camera.position, 0.0)),
                lower_left_corner_minus_position: glam::Vec4::from((
                    lower_left_corner_minus_position3,
                    0.0,
                )),
                horizontal: glam::Vec4::from((horizontal3, 0.0)),
                vertical: glam::Vec4::from((vertical3, 0.0)),
            },
            hit_index.map(|idx| self.attributes[idx].index as usize),
        ))
    }

    /// Save the scene data (geometries, attributes, materials) to a YAML file in a `debug_output/` directory at the crate root.
    ///
    /// This is only compiled when the `testing` feature is enabled and the target architecture is not wasm32, as file system access is not available in WebAssembly.
    #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
    #[allow(dead_code)]
    fn save_scene_data_json(&self) {
        #[derive(serde::Serialize)]
        struct IndexedGeometry {
            index: usize,
            #[serde(flatten)]
            geometry: GpuTriangleGeometry,
        }

        #[derive(serde::Serialize)]
        struct IndexedAttribute {
            index: usize,
            #[serde(flatten)]
            attribute: GpuTriangleAttribute,
        }

        #[derive(serde::Serialize)]
        struct IndexedMaterial {
            index: usize,
            #[serde(flatten)]
            material: GpuMaterial,
        }

        #[derive(serde::Serialize)]
        struct SceneData {
            geometries: Vec<IndexedGeometry>,
            attributes: Vec<IndexedAttribute>,
            materials: Vec<IndexedMaterial>,
        }

        let scene_data = SceneData {
            geometries: self
                .geometries
                .iter()
                .enumerate()
                .map(|(index, geometry)| IndexedGeometry {
                    index,
                    geometry: *geometry,
                })
                .collect(),
            attributes: self
                .attributes
                .iter()
                .enumerate()
                .map(|(index, attribute)| IndexedAttribute {
                    index,
                    attribute: *attribute,
                })
                .collect(),
            materials: self
                .materials
                .iter()
                .enumerate()
                .map(|(index, material)| IndexedMaterial {
                    index,
                    material: *material,
                })
                .collect(),
        };

        let mut output_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        output_path.push("debug_output");

        if let Err(e) = std::fs::create_dir_all(&output_path) {
            log::error!("Failed to create debug_output directory: {e}");
            return;
        }

        output_path.push("scene_data.yaml");

        match serde_saphyr::to_string(&scene_data) {
            Ok(yaml) => match std::fs::write(&output_path, yaml) {
                Ok(()) => {
                    log::info!(
                        "Saved scene data YAML to {} (geometries: {}, attributes: {}, materials: {})",
                        output_path.display(),
                        self.geometries.len(),
                        self.attributes.len(),
                        self.materials.len()
                    );
                }
                Err(e) => {
                    log::error!("Failed to write scene data YAML: {e}");
                }
            },
            Err(e) => {
                log::error!("Failed to serialize scene data to YAML: {e}");
            }
        }
    }

    /// Save a texture atlas as a PNG image in a `debug_output/` directory at the crate root.
    ///
    /// This is only compiled when the `testing` feature is enabled and the target architecture is not wasm32, as file system access is not available in WebAssembly.
    #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
    #[allow(dead_code)]
    fn save_rgba_image(filename: &str, pixels: &[u8], width: u32, height: u32) {
        // create debug_output directory in the crate root
        let mut output_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        output_path.push("debug_output");

        if let Err(e) = std::fs::create_dir_all(&output_path) {
            log::error!("Failed to create debug_output directory: {e}");
            return;
        }

        output_path.push(filename);

        match image::RgbaImage::from_raw(width, height, pixels.to_vec()) {
            Some(img) => match img.save(&output_path) {
                Ok(()) => {
                    log::info!("Saved debug image to {}", output_path.display());
                }
                Err(e) => {
                    log::error!("Failed to save image {}: {}", output_path.display(), e);
                }
            },
            None => {
                log::error!(
                    "Failed to create image from raw pixels: {}x{} with {} bytes",
                    width,
                    height,
                    pixels.len()
                );
            }
        }
    }
}

impl Camera {
    /// Calculate decent starting camera values from scene bounds.
    fn new(aabb_min: glam::Vec3, aabb_max: glam::Vec3) -> Self {
        let scene_center = (aabb_min + aabb_max) * 0.5;
        Self {
            position: glam::Vec3::new(
                scene_center.x,
                (aabb_max.y - aabb_min.y).mul_add(0.01, aabb_max.y), // position the camera slightly above the top of the scene
                scene_center.z,
            ),
            yaw: 0.0,
            pitch: 0.0,
            fov_y: std::f32::consts::FRAC_PI_2,
            aspect_ratio: 1.0,
            movement_speeds: [(aabb_max.x - aabb_min.x).max(aabb_max.z - aabb_min.z) * 0.3; 2],
            rotation_speeds: [0.003; 2],
            dirty: true,
        }
    }

    /// Move the camera based on currently pressed keys and distance multipliers (the amount of time elapsed since the last frame).
    pub fn translate(
        &mut self,
        pressed_keys: &std::collections::HashSet<winit::keyboard::KeyCode>,
        horizontal_distance: f32,
        vertical_distance: f32,
    ) {
        let w = pressed_keys.contains(&winit::keyboard::KeyCode::KeyW);
        let s = pressed_keys.contains(&winit::keyboard::KeyCode::KeyS);
        let a = pressed_keys.contains(&winit::keyboard::KeyCode::KeyA);
        let d = pressed_keys.contains(&winit::keyboard::KeyCode::KeyD);
        let space = pressed_keys.contains(&winit::keyboard::KeyCode::Space);
        let shift = pressed_keys.contains(&winit::keyboard::KeyCode::ShiftLeft);
        let q = pressed_keys.contains(&winit::keyboard::KeyCode::KeyQ);

        if !(w || s || a || d || space || shift) {
            return;
        }

        let speed_multiplier = if q { 4.0 } else { 1.0 };

        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();

        let forward_xz = glam::Vec3::new(-sin_yaw, 0.0, -cos_yaw);
        let right_xz = glam::Vec3::new(cos_yaw, 0.0, -sin_yaw);

        let forward_coeff = (if w { 1.0 } else { 0.0 }) - (if s { 1.0 } else { 0.0 });
        let right_coeff = (if d { 1.0 } else { 0.0 }) - (if a { 1.0 } else { 0.0 });

        let intent = forward_xz * forward_coeff + right_xz * right_coeff;

        let move_xz = if intent.length_squared() > f32::EPSILON {
            intent.normalize() * horizontal_distance * speed_multiplier * self.movement_speeds[0]
        } else {
            glam::Vec3::ZERO
        };

        let vert_dir = (if space { 1.0 } else { 0.0 }) - (if shift { 1.0 } else { 0.0 });
        let vertical_movement =
            vert_dir * vertical_distance * speed_multiplier * self.movement_speeds[1];

        // check if there's any actual movement
        // keys might be pressed but cancel each other out (e.g. W and S)
        // == 0.0 instead of <= f32::EPSILON because with tiny scenes, allow very slow movement speeds without it being considered as no movement
        if move_xz.length_squared() == 0.0 && vertical_movement.abs() == 0.0 {
            return;
        }

        self.position += move_xz + glam::Vec3::new(0.0, vertical_movement, 0.0);

        self.dirty = true;
    }

    /// Rotate the camera based on mouse movement deltas. `dx` and `dy` are the changes in mouse position.
    pub const fn rotate(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * self.rotation_speeds[0];
        self.pitch -= dy * self.rotation_speeds[1];

        self.pitch = self
            .pitch
            .clamp(-std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2);

        self.dirty = true;
    }

    /// Resize the camera's aspect ratio.
    pub const fn resize(&mut self, aspect_ratio: f32) {
        self.aspect_ratio = aspect_ratio;
        self.dirty = true;
    }

    /// Change the camera's vertical field of view in radians. Clamps to [0, π).
    pub const fn set_fov(&mut self, fov_y: f32) {
        self.fov_y = fov_y.clamp(0.0, std::f32::consts::PI - f32::EPSILON);
        self.dirty = true;
    }

    /// Get the vertical field of view.
    pub const fn fov_y(&self) -> f32 {
        self.fov_y
    }

    /// Move the camera to a specific world-space position and rotation; yaw and pitch in radians.
    pub fn translate_and_rotate_to(
        &mut self,
        position: Option<glam::Vec3>,
        yaw: Option<f32>,
        pitch: Option<f32>,
    ) {
        self.position = position.unwrap_or(self.position);
        self.yaw = yaw.unwrap_or(self.yaw);
        self.pitch = pitch.unwrap_or(self.pitch);
        self.dirty = true;
    }

    /// Get the position in world space.
    pub const fn position(&self) -> glam::Vec3 {
        self.position
    }

    /// Get the yaw in radians.
    pub const fn yaw(&self) -> f32 {
        self.yaw
    }

    /// Get the pitch in radians.
    pub const fn pitch(&self) -> f32 {
        self.pitch
    }
}
