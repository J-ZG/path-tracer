//! Load, format, and manage a 3D world from a glTF 2.0 .glb file.
//!
//! A .glb file can be exported from Blender.
//!
//! Be sure to check `Include > Custom Properties` and `Mesh > Apply Modifiers`.
//!
//! ![Export glTF from Blender 1](https://raw.githubusercontent.com/j-zg/path-tracer/main/doc/blender_export_gltf_1.jpg)
//! ![Export glTF from Blender 2](https://raw.githubusercontent.com/j-zg/path-tracer/main/doc/blender_export_gltf_2.jpg)

// the terminology is: world = scene + skybox

// some are pub instead of pub(super) or pub(crate) in this module because criterion, an external crate, is used to benchmark stuff in this module

use anyhow::Context;

/// Default average luminance of a skybox in nits, which is that of a clear daytime sky.
///
/// Skybox image files usually don't store physically based values, so the average luminance computed from an imported skybox is rarely meaningful.
/// Set a physically based value with [`World::edit_skybox_average_luminance()`] that suits the camera exposure.
/// This value suits the exposure from the Sunny 16 rule: aperture f/16, shutter speed 1/125 s, and ISO 100.
pub(super) const DEFAULT_SKYBOX_LUMINANCE: f32 = 8000.0; // EXPOSURE is currently set in trace.wgsl

/// The size of each texture atlas in pixels.
///
/// 8192 is the default value of [`wgpu::Limits::max_texture_dimension_2d`].
/// Since passing an array of images with different sizes isn't supported universally,
/// images are packed into atlases which are larger square images that store multiple smaller images.
/// Then, an array of atlases are passed to the GPU without problems.
pub(super) const ATLAS_SIZE: u32 = 8192;

/// [`GpuMaterial::alpha_mode`] where the surface always exists and the alpha of the base color is ignored.
const ALPHA_MODE_OPAQUE: u32 = 0;
/// [`GpuMaterial::alpha_mode`] where the surface exists only where the alpha of the base color is at least [`GpuMaterial::alpha_cutoff`].
const ALPHA_MODE_MASK: u32 = 1;
/// [`GpuMaterial::alpha_mode`] where the alpha of the base color is the probability that the surface exists.
const ALPHA_MODE_BLEND: u32 = 2;
// TODO: could be sized dynamically: after atlas construction, if there was only one atlas, shrink the atlas until it fits; there are multiple atlases, just use 8192

// TODO: add vertex colors
// TODO: there are lag spikes (not related to wgpu version 30), occurring at set amount of time elapsed after application first runs, which slows down frame rate by some percentage (not absolute change in frame rate)
// TODO: render at a smaller resolution by default (maybe by somehow running an auto benchmark before anything runs to determine the user's hardware capabilities), and allow user to adjust resolution, so the app doesn't crash the user's computer

// TODO: finish docs

// Rust structs use the Gpu prefix iff their bytes are uploaded to GPU buffers
// CPU containers and resource handles do not use the prefix

/// Contains world data and pending changes.
pub(super) struct World {
    scene: Scene,
    skybox: Skybox,
    scene_id: std::rc::Rc<()>,
    pending: PendingChanges,
    scene_uniforms: GpuSceneUniforms,
    scene_alias_table: Vec<GpuSceneAliasEntry>,
    selected_material: Option<usize>,
}

/// Used for internally keeping track of desired changes since the last [`World::take_updates()`] call.
#[derive(Default)]
struct PendingChanges {
    scene: Option<(Scene, std::rc::Rc<()>)>,
    skybox: Option<Skybox>,
    camera: Option<Camera>,
    skybox_average_luminance: Option<f32>,
    materials: std::collections::BTreeMap<usize, GpuMaterial>,
    full_upload: bool,
}

/// Updated data that should be applied (written to the GPU).
#[must_use]
pub(super) struct WorldUpdates<'a> {
    pub(super) scene: Option<SceneUpload<'a>>,
    pub(super) skybox: Option<SkyboxUpload<'a>>,
    pub(super) camera: Option<GpuCamera>,
    pub(super) materials: Vec<(usize, GpuMaterial)>,
    pub(super) scene_uniforms: Option<GpuSceneUniforms>,
    pub(super) scene_alias_table: Option<&'a [GpuSceneAliasEntry]>,
    pub(super) skybox_uniforms: Option<GpuSkyboxUniforms>,
    pub(super) reset_accumulation: bool,
}

/// Updated data coming from an entirely new scene that should be applied (written to the GPU).
pub(super) struct SceneUpload<'a> {
    pub(super) geometries: &'a [GpuTriangleGeometry],
    pub(super) attributes: &'a [GpuTriangleAttribute],
    pub(super) materials: &'a [GpuMaterial],
    pub(super) bvh_nodes: &'a [GpuBvhNode],
    pub(super) srgb_texture_atlases: &'a [Vec<u8>],
    pub(super) linear_texture_atlases: &'a [Vec<u8>],
}

/// Updated data coming from an entirely new skybox that should be applied (written to the GPU).
pub(super) struct SkyboxUpload<'a> {
    pub(super) rgba: &'a [f32],
    pub(super) uniforms: GpuSkyboxUniforms,
    pub(super) alias_table: &'a [GpuSkyboxAliasEntry],
}

/// Contains all the data loaded from a glTF 2.0 .glb file.
pub struct Scene {
    // geometries[i] and attributes[i] together represent a triangle at index i
    // hence, only GpuTriangleAttribute contains material index (GpuTriangleGeometry doesn't contain any index)
    geometries: Vec<GpuTriangleGeometry>,
    attributes: Vec<GpuTriangleAttribute>,
    materials: Vec<GpuMaterial>,
    bvh_nodes: Vec<GpuBvhNode>,
    /// The array of sRGB texture atlases. Each texture atlas is a RGBA8 image (stored as a [`Vec<u8>`]) that contains multiple smaller sRGB textures packed together.
    ///
    /// For locating a specific texture, the `*_tex_layer` fields in [`GpuMaterial`] are indices into this array (or the [`Scene::linear_texture_atlases`] array)
    /// of texture atlases. Then, the `*_uv` fields should be used to access the correct portion of the texture atlas.
    srgb_texture_atlases: Vec<Vec<u8>>,
    /// The array of linear texture atlases. Each texture atlas is a RGBA8 image (stored as a [`Vec<u8>`]) that contains multiple smaller linear textures packed together.
    ///
    /// For locating a specific texture, the `*_tex_layer` fields in [`GpuMaterial`] are indices into this array (or the [`Scene::srgb_texture_atlases`] array)
    /// of texture atlases. Then, the `*_uv` fields should be used to access the correct portion of the texture atlas.
    linear_texture_atlases: Vec<Vec<u8>>,
    camera: Camera,
    /// The initial scene data, for resetting editable data.
    initial: Initial,
}

/// Stores initial scene data, for resetting editable data.
struct Initial {
    materials: Vec<GpuMaterial>,
    camera: Camera,
}

/// Contains the positions of the three vertices of a triangle.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)] // Clone and Copy are required by Pod
#[cfg_attr(
    all(feature = "testing", not(target_arch = "wasm32")),
    derive(serde::Serialize)
)]
pub(super) struct GpuTriangleGeometry {
    // bytemuck::Pod requires alignment without implicit padding
    // although glam::Vec3A is 16 byte aligned, it has padding
    // WebGPU requirements: https://www.w3.org/TR/WGSL/#alignment-and-size
    v0: glam::Vec3,
    v1: glam::Vec3,
    v2: glam::Vec3,
}

/// Contains the material index and vertex attributes of a triangle.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[cfg_attr(
    all(feature = "testing", not(target_arch = "wasm32")),
    derive(serde::Serialize)
)]
pub(super) struct GpuTriangleAttribute {
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
pub(super) struct GpuMaterial {
    /// Base color in linear RGBA. Linear multiplier to the base color texture; if no texture, the base color is just this color.
    ///
    /// Parsed from [material.pbrMetallicRoughness.baseColorFactor](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#_material_pbrmetallicroughness_basecolorfactor).
    ///
    /// Defaults to `(1.0, 1.0, 1.0, 1.0)`; RGBA in [0, 1].
    ///
    /// Intended OpenPBR counterpart identifier: `base_color`. OpenPBR uses only RGB; default is `(0.8, 0.8, 0.8)`; RGB in [0, 1].
    pub(super) base_color_factor: glam::Vec4,
    /// Defaults to `1.0`; in [0, 1].
    pub(super) OpenPBR_base_weight: f32,
    /// Linear multiplier to the metallic value in the metallic-roughness texture; if no texture, this is the metallic value.
    ///
    /// Parsed from [material.pbrMetallicRoughness.metallicFactor](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#_material_pbrmetallicroughness_metallicfactor).
    ///
    /// Defaults to `1.0`; in [0, 1].
    ///
    /// Intended OpenPBR counterpart identifier: `base_metalness`. OpenPBR's default is `0.0`; in [0, 1].
    pub(super) metallic_factor: f32,
    /// Linear multiplier to the roughness value in the metallic-roughness texture; if no texture, this is the roughness value.
    ///
    /// Parsed from [material.pbrMetallicRoughness.roughnessFactor](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#_material_pbrmetallicroughness_roughnessfactor).
    ///
    /// Defaults to `1.0`; in [0, 1].
    ///
    /// Intended OpenPBR counterpart identifier: `base_diffuse_roughness`. OpenPBR's default is `0.0`; in [0, 1].
    pub(super) roughness_factor: f32,
    /// Defaults to `0.0`; in [0, 1].
    pub(super) OpenPBR_emission_weight: f32,
    /// Emission color in linear RGB and emission strength in the 4th value. RGB is a linear multiplier to the emission texture; if no texture, the emission color is just this color.
    ///
    /// RGB parsed from [material.emissiveFactor](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#_material_emissivefactor) and strength from [KHR_materials_emissive_strength](https://github.com/KhronosGroup/glTF/tree/main/extensions/2.0/Khronos/KHR_materials_emissive_strength).
    ///
    /// Defaults to `(0.0, 0.0, 0.0, 1.0)`; RGB in [0, 1].
    ///
    /// Intended OpenPBR counterpart identifiers: `emission_color` and `emission_luminance`. OpenPBR's defaults are `(1.0, 1.0, 1.0)` and `1000`; RGB in [0, ∞) and luminance in [0, ∞). Luminance has units of nits.
    pub(super) emission_color_and_strength: glam::Vec4,
    /// Specular color in linear RGB and weight of specular in the 4th value. RGB is a linear multiplier to the specular color texture; if no texture, the specular color is just this color.
    ///
    /// Parsed from [KHR_materials_specular](https://github.com/KhronosGroup/glTF/tree/main/extensions/2.0/Khronos/KHR_materials_specular).
    ///
    /// Defaults to `(1.0, 1.0, 1.0, 1.0)`; RGB is uncapped.
    ///
    /// Intended OpenPBR counterpart identifiers: `specular_color` and `specular_weight`. OpenPBR's defaults are `(1.0, 1.0, 1.0)` and `1.0`; RGB in [0, 1] and weight in [0, ∞).
    pub(super) specular_color_and_factor: glam::Vec4,
    /// Defaults to `0.3`; in [0, 1].
    pub(super) OpenPBR_specular_roughness: f32,
    /// Defaults to `0.0`; in [0, 1].
    pub(super) OpenPBR_specular_roughness_anisotropy: f32,
    /// Defaults to `0.0`; in [0, 1].
    pub(super) OpenPBR_specular_haze: f32,
    /// Defaults to `0.3`; in [0, 1].
    pub(super) OpenPBR_specular_haze_spread: f32,
    /// Index of refraction.
    ///
    /// Parsed from [KHR_materials_ior](https://github.com/KhronosGroup/glTF/tree/main/extensions/2.0/Khronos/KHR_materials_ior).
    ///
    /// Defaults to `1.5`; in [1, ∞).
    ///
    /// Intended OpenPBR counterpart identifier: `specular_ior`. OpenPBR's default is `1.5`; in (0, ∞).
    pub(super) ior: f32,
    /// Defaults to `0.0`; in [0, 1].
    pub(super) OpenPBR_specular_retroreflectivity: f32,
    /// Linear scale factor for the normal texture; if no texture, should be ignored.
    ///
    /// Parsed from [material.normalTextureInfo.scale](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#_material_normaltextureinfo_scale).
    ///
    /// Defaults to `1.0`.
    ///
    /// No intended OpenPBR counterpart.
    pub(super) normal_scale: f32,

    /// How the alpha of the base color determines whether the surface exists: [`ALPHA_MODE_OPAQUE`], [`ALPHA_MODE_MASK`], or [`ALPHA_MODE_BLEND`].
    ///
    /// Parsed from [material.alphaMode](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#_material_alphamode).
    alpha_mode: u32,
    /// The surface doesn't exist where the alpha of the base color is less than this value; only used with [`ALPHA_MODE_MASK`].
    ///
    /// Parsed from [material.alphaCutoff](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#_material_alphacutoff).
    ///
    /// Defaults to `0.5`; in [0, ∞).
    alpha_cutoff: f32,

    pad: f32,

    /// Index into [`Scene::srgb_texture_atlases`] for the texture atlas containing the sRGB base color texture; `-1` if no texture.
    base_color_tex_layer: i32,
    /// Index into [`Scene::linear_texture_atlases`] for the texture atlas containing the linear normal texture; `-1` if no texture.
    normal_tex_layer: i32,
    /// Index into [`Scene::linear_texture_atlases`] for the texture atlas containing the linear metallic-roughness texture; `-1` if no texture.
    metallic_roughness_tex_layer: i32,
    /// Index into [`Scene::srgb_texture_atlases`] for the texture atlas containing the sRGB emission texture; `-1` if no texture.
    emission_tex_layer: i32,
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
    /// Offset (x, y) and scale (x, y) for the emission texture in an atlas; if no texture, are all zeros.
    emission_uv: glam::Vec4,
    /// Offset (x, y) and scale (x, y) for the specular color texture in an atlas; if no texture, are all zeros.
    specular_color_uv: glam::Vec4,
    /// Offset (x, y) and scale (x, y) for the specular factor texture in an atlas; if no texture, are all zeros.
    specular_factor_uv: glam::Vec4,
}
pub(super) struct MaterialId {
    scene: std::rc::Rc<()>,
    index: usize,
}
impl MaterialId {
    pub(super) const fn index(&self) -> usize {
        self.index
    }
}

/// A node in the BVH tree.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct GpuBvhNode {
    /// Axis-aligned bounding box's minimum of this node.
    aabb_min: glam::Vec3,
    /// If this node is an internal node ([`Self::tri_count`] is `0`), this value is the index of the left child node in [`Scene::bvh_nodes`].
    /// The right child node's index is `left_first + 1`.
    /// If this node is a leaf, this value is the starting offset into [`Scene::geometries`] for the triangles contained in this leaf.
    left_first: u32,
    /// Axis-aligned bounding box's maximum of this node.
    aabb_max: glam::Vec3,
    /// If this node is an internal node, this value is `0`.
    /// If this node is a leaf, this value is the number of triangles contained in this leaf node.
    tri_count: u32,
}
impl GpuBvhNode {
    /// Create a new [`GpuBvhNode`] with zeros.
    const fn new() -> Self {
        Self {
            aabb_min: glam::Vec3::ZERO,
            left_first: 0,
            aabb_max: glam::Vec3::ZERO,
            tri_count: 0,
        }
    }

    /// Update the axis-aligned bounding box to make it encompass the given triangles.
    fn update_aabb(&mut self, triangles: &[BvhTriangle]) {
        let mut aabb = Aabb::new();
        for triangle in triangles {
            aabb.union(&triangle.aabb);
        }
        self.aabb_min = aabb.min;
        self.aabb_max = aabb.max;
    }
}

/// Used for keeping track of camera data internally; not passed to the GPU.
#[derive(Clone)]
pub(super) struct Camera {
    /// Camera position X, Y, Z in world space.
    position: glam::Vec3,
    /// Yaw in radians.
    yaw: f32,
    /// Pitch in radians. Should be in [-π/2, π/2].
    pitch: f32,
    // focus distance is not needed since this is a pinhole camera; focus distance is implicitly 1
    /// Vertical field of view in radians. Should be in [0, π).
    fov: f32,
    /// Width ÷ height of the image plane.
    aspect_ratio: f32,
    /// Movement speeds for horizontal and vertical movement, respectively, in units per second.
    pub(super) movement_speeds: [f32; 2],
    /// Mouse sensitivity for rotating the camera for horizontal and vertical rotations, respectively.
    pub(super) rotation_speeds: [f32; 2],
}

/// Contains camera data formatted for the GPU.
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
pub(super) struct GpuCamera {
    /// Camera position in world space.
    position: glam::Vec4,
    /// Lower-left pixel coordinate of image plane in world space minus camera position.
    lower_left_corner_minus_position: glam::Vec4,
    /// Vector that spans the full X of image plane in world space.
    horizontal: glam::Vec4,
    /// Vector that spans the full Y of image plane in world space.
    vertical: glam::Vec4,
}

/// BVH axis-aligned bounding box helper struct.
#[derive(Clone, Copy)]
struct Aabb {
    min: glam::Vec3,
    max: glam::Vec3,
}
impl Aabb {
    /// Creates a new [`Aabb`] with `min` set to positive infinity and `max` set to negative infinity.
    const fn new() -> Self {
        Self {
            min: glam::Vec3::splat(f32::INFINITY),
            max: glam::Vec3::splat(f32::NEG_INFINITY),
        }
    }
    fn union(&mut self, other: &Self) {
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
    }
    fn surface_area_half(&self) -> f32 {
        let e = self.max - self.min;
        e.z.mul_add(e.x, e.x.mul_add(e.y, e.y * e.z)) // e.x * e.y + e.y * e.z + e.z * e.x, but using mul_add for better precision and performance
        // omitting 2.0 coefficient, so this returns half of the surface area
    }
}
/// BVH triangle helper struct.
struct BvhTriangle {
    aabb: Aabb,
    centroid: glam::Vec3,
    original_index: usize,
}
/// BVH bin helper struct.
#[derive(Clone, Copy)]
struct BvhBin {
    aabb: Aabb,
    tri_count: u32,
}
/// The amount of bins to calculate during BVH construction.
const BVH_BINS: usize = 8;

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

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct GpuSceneUniforms {
    /// Estimated lumens, ignoring emission textures. Triangles emit on both sides.
    total_luminous_flux: f32,
    emissive_triangles_count: u32,
    /// Geometry and skybox category probabilities.
    light_probabilities: [f32; 2],
}
impl GpuSceneUniforms {
    fn update_light_probabilities(&mut self, skybox_total_luminous_flux: f32) {
        let geometry_flux = if self.emissive_triangles_count > 0 {
            self.total_luminous_flux
        } else {
            0.0
        };
        let scale = geometry_flux.max(skybox_total_luminous_flux);
        self.light_probabilities = if scale > 0.0 {
            let geometry = geometry_flux / scale;
            let skybox = skybox_total_luminous_flux / scale;
            [geometry / (geometry + skybox), skybox / (geometry + skybox)]
        } else {
            [0.0; 2]
        };
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct GpuSceneAliasEntry {
    /// Probability of accepting this slot rather than its alias.
    threshold: f32,
    /// Another slot in [`World::scene_alias_table`].
    alias_index: u32,
    triangle_index: u32,
    selection_probability: f32,
}

/// Contains skybox texture data formatted for the GPU.
struct Skybox {
    /// Normalized equirectangular texture.
    rgba: Vec<f32>,
    uniforms: GpuSkyboxUniforms,
    alias_table: Vec<GpuSkyboxAliasEntry>,
}

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct GpuSkyboxUniforms {
    /// Estimated lumens incident on the scene's bounding sphere.
    total_luminous_flux: f32,
    /// Solid-angle-weighted average luminance in nits; may be user-defined.
    average_luminance: f32,
    pub(super) width: u32,
    pub(super) height: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct GpuSkyboxAliasEntry {
    threshold: f32,
    alias_index: u32,
    selection_probability: f32,
}

/// Vose's alias method.
///
/// Returns a skybox alias table, but is also used to create the scene alias table via a simple mapping.
fn alias_table(weights: &[f64]) -> Vec<GpuSkyboxAliasEntry> {
    let total: f64 = weights.iter().sum();
    if total <= 0.0 {
        return Vec::new();
    }
    let mut entries: Vec<_> = weights
        .iter()
        .enumerate()
        .map(|(index, weight)| GpuSkyboxAliasEntry {
            threshold: 1.0,
            alias_index: index as u32,
            selection_probability: (weight / total) as f32,
        })
        .collect();
    let mut scaled: Vec<_> = weights
        .iter()
        .map(|weight| weight / total * weights.len() as f64)
        .collect();
    let (mut small, mut large): (Vec<_>, Vec<_>) =
        (0..weights.len()).partition(|&i| scaled[i] < 1.0);
    while let (Some(&low), Some(&high)) = (small.last(), large.last()) {
        small.pop();
        large.pop();
        entries[low].threshold = scaled[low].clamp(0.0, 1.0) as f32;
        entries[low].alias_index = high as u32;
        scaled[high] = (scaled[high] + scaled[low]) - 1.0;
        if scaled[high] < 1.0 {
            small.push(high);
        } else {
            large.push(high);
        }
    }
    entries
}

/// Get luminance via Rec. 709 coefficients.
fn luminance(rgb: glam::Vec3) -> f64 {
    let rgb = rgb.as_dvec3().max(glam::DVec3::ZERO);
    rgb.dot(glam::DVec3::new(0.2126, 0.7152, 0.0722))
}

/// Exact equirectangular row solid angle, omitting the common longitude factor.
fn row_weight(y: u32, height: u32) -> f64 {
    let step = std::f64::consts::PI / f64::from(height);
    (f64::from(y) * step).cos() - (f64::from(y + 1) * step).cos()
}

impl World {
    pub(super) fn new(
        scene_bytes: &[u8],
        skybox_bytes: &[u8],
        aspect_ratio: f32,
    ) -> anyhow::Result<Self> {
        let mut scene = Scene::new(scene_bytes)?;
        scene.camera.resize(aspect_ratio);
        scene.initial.camera.resize(aspect_ratio);
        Ok(Self {
            scene,
            skybox: Skybox::new(skybox_bytes)?,
            scene_id: std::rc::Rc::new(()),
            pending: PendingChanges {
                full_upload: true,
                ..Default::default()
            },
            scene_uniforms: GpuSceneUniforms::default(),
            scene_alias_table: Vec::new(),
            selected_material: None,
        })
    }

    pub(super) fn replace_scene(&mut self, bytes: &[u8]) -> anyhow::Result<()> {
        let mut scene = Scene::new(bytes)?;
        let aspect_ratio = self
            .pending
            .camera
            .as_ref()
            .unwrap_or_else(|| {
                self.pending
                    .scene
                    .as_ref()
                    .map_or(&self.scene.camera, |(s, _)| &s.camera)
            })
            .aspect_ratio;
        scene.camera.resize(aspect_ratio);
        scene.initial.camera.resize(aspect_ratio);
        self.pending.scene = Some((scene, std::rc::Rc::new(())));
        self.pending.camera = None;
        self.pending.materials.clear();
        Ok(())
    }

    pub(super) fn replace_skybox(&mut self, bytes: &[u8]) -> anyhow::Result<()> {
        let mut skybox = Skybox::new(bytes)?;
        skybox.uniforms.average_luminance = self
            .pending
            .skybox_average_luminance
            .unwrap_or(self.skybox.uniforms.average_luminance);
        self.pending.skybox = Some(skybox);
        Ok(())
    }

    /// Set the skybox average luminance in nits.
    pub(super) const fn edit_skybox_average_luminance(&mut self, luminance: f32) {
        self.pending.skybox_average_luminance = Some(luminance.max(0.0));
    }

    /// Get the skybox average luminance in nits.
    pub(super) const fn skybox_average_luminance(&self) -> f32 {
        self.skybox.uniforms.average_luminance
    }

    /// Get camera data.
    pub(super) const fn camera(&self) -> &Camera {
        &self.scene.camera
    }
    /// Get initial camera data.
    pub(super) const fn initial_camera(&self) -> &Camera {
        &self.scene.initial.camera
    }

    pub(super) fn edit_camera(&mut self, edit: impl FnOnce(&mut Camera)) {
        edit(self.pending.camera.get_or_insert_with(|| {
            self.pending
                .scene
                .as_ref()
                .map_or(&self.scene.camera, |(s, _)| &s.camera)
                .clone()
        }));
    }

    pub(super) fn selected_material(&self) -> Option<MaterialId> {
        self.selected_material.map(|index| MaterialId {
            scene: std::rc::Rc::clone(&self.scene_id),
            index,
        })
    }

    pub(super) fn material(&self, id: &MaterialId) -> Option<(&GpuMaterial, &GpuMaterial)> {
        if !std::rc::Rc::ptr_eq(&id.scene, &self.scene_id) {
            return None;
        }
        self.scene
            .materials
            .get(id.index)
            .zip(self.scene.initial.materials.get(id.index))
    }

    pub(super) fn edit_material(
        &mut self,
        id: &MaterialId,
        edit: impl FnOnce(&mut GpuMaterial),
    ) -> bool {
        let (scene, id_pending) = self
            .pending
            .scene
            .as_ref()
            .map_or((&self.scene, &self.scene_id), |(scene, id_pending)| {
                (scene, id_pending)
            });

        let Some(current_id) = (id.index < scene.materials.len()).then(|| MaterialId {
            scene: std::rc::Rc::clone(id_pending),
            index: id.index,
        }) else {
            return false;
        };
        if !std::rc::Rc::ptr_eq(&id.scene, &current_id.scene) {
            return false;
        }
        let scene = self
            .pending
            .scene
            .as_ref()
            .map_or(&self.scene, |(scene, _)| scene);
        let original = self
            .pending
            .materials
            .get(&id.index)
            .copied()
            .unwrap_or(scene.materials[id.index]);
        let mut candidate = original;
        edit(&mut candidate);

        let editable_size = std::mem::offset_of!(GpuMaterial, alpha_mode);
        let candidate_bytes = &bytemuck::bytes_of(&candidate)[..editable_size];
        if !bytemuck::cast_slice::<u8, f32>(candidate_bytes)
            .iter()
            .all(|value| value.is_finite())
        {
            return false;
        }
        let mut accepted = original;
        bytemuck::bytes_of_mut(&mut accepted)[..editable_size].copy_from_slice(candidate_bytes);
        if bytemuck::bytes_of(&accepted) == bytemuck::bytes_of(&scene.materials[id.index]) {
            self.pending.materials.remove(&id.index);
        } else {
            self.pending.materials.insert(id.index, accepted);
        }
        true
    }

    #[expect(clippy::float_cmp)]
    pub(super) fn take_updates(&mut self) -> WorldUpdates<'_> {
        let pending = std::mem::take(&mut self.pending);
        let scene_replaced = pending.scene.is_some() || pending.full_upload;
        let skybox_replaced = pending.skybox.is_some() || pending.full_upload;

        if let Some((scene, id)) = pending.scene {
            self.scene = scene;
            self.scene_id = id;
        }
        if let Some(skybox) = pending.skybox {
            self.skybox = skybox;
        }

        let mut camera_changed = scene_replaced;
        let mut selection_changed = scene_replaced;
        if let Some(camera) = pending.camera {
            selection_changed |= camera.position != self.scene.camera.position
                || camera.yaw != self.scene.camera.yaw
                || camera.pitch != self.scene.camera.pitch;
            camera_changed |= selection_changed
                || camera.fov != self.scene.camera.fov
                || camera.aspect_ratio != self.scene.camera.aspect_ratio;
            self.scene.camera = camera;
        }

        let mut luminance_changed = skybox_replaced;
        if let Some(luminance) = pending.skybox_average_luminance {
            luminance_changed |= luminance != self.skybox.uniforms.average_luminance;
            self.skybox.uniforms.average_luminance = luminance;
        }

        let mut lighting_changed = scene_replaced;
        let mut materials = Vec::new();
        for (index, material) in pending.materials {
            let old = &mut self.scene.materials[index];
            lighting_changed |= old.OpenPBR_emission_weight != material.OpenPBR_emission_weight
                || old.emission_color_and_strength != material.emission_color_and_strength;
            *old = material;
            if !scene_replaced {
                materials.push((index, material));
            }
        }
        if lighting_changed {
            (self.scene_uniforms, self.scene_alias_table) = self.scene.uniforms_and_alias_table();
        }

        if skybox_replaced {
            self.skybox.update_alias_table();
        }
        let skybox_uniforms_changed = scene_replaced || luminance_changed;
        if skybox_uniforms_changed {
            self.skybox
                .update_total_luminous_flux(self.scene.radius_squared());
        }
        let scene_uniforms_changed = lighting_changed || skybox_uniforms_changed;
        if scene_uniforms_changed {
            self.scene_uniforms
                .update_light_probabilities(self.skybox.uniforms.total_luminous_flux);
        }

        if selection_changed {
            let camera = &self.scene.camera;
            self.selected_material =
                self.scene
                    .pick_material(camera.position, camera.yaw, camera.pitch);
        }

        let reset_accumulation = scene_replaced
            || skybox_replaced
            || camera_changed
            || luminance_changed
            || !materials.is_empty();

        WorldUpdates {
            scene: scene_replaced.then(|| SceneUpload {
                geometries: &self.scene.geometries,
                attributes: &self.scene.attributes,
                materials: &self.scene.materials,
                bvh_nodes: &self.scene.bvh_nodes,
                srgb_texture_atlases: &self.scene.srgb_texture_atlases,
                linear_texture_atlases: &self.scene.linear_texture_atlases,
            }),
            skybox: skybox_replaced.then(|| SkyboxUpload {
                rgba: &self.skybox.rgba,
                uniforms: self.skybox.uniforms,
                alias_table: &self.skybox.alias_table,
            }),
            camera: camera_changed.then(|| self.scene.camera.gpu_camera()),
            materials,
            scene_uniforms: scene_uniforms_changed.then_some(self.scene_uniforms),
            scene_alias_table: lighting_changed.then_some(self.scene_alias_table.as_slice()),
            skybox_uniforms: skybox_uniforms_changed.then_some(self.skybox.uniforms),
            reset_accumulation,
        }
    }
}

impl Scene {
    /// Format .glb byte data for the GPU.
    // pub for criterion
    pub fn new(bytes: &[u8]) -> anyhow::Result<Self> {
        let (document, buffers, images) =
            gltf::import_slice(bytes).context("gltf::import_slice failed")?;

        // see enabled extensions in Cargo.toml
        // TODO: add KHR_texture_transform
        #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
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
        for (img_index, image) in images.iter().enumerate() {
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
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .flat_map(|&[r, g]| [r, g, 0, 255]),
                ),
                Format::R8G8B8 => rgba.extend(
                    image
                        .pixels
                        .as_chunks::<3>()
                        .0
                        .iter()
                        .flat_map(|&[r, g, b]| [r, g, b, 255]),
                ),
                Format::R8G8B8A8 => rgba.extend(image.pixels.iter().copied()),

                // data is guaranteed to be little-endian in glTF, so from_le_bytes isn't needed
                Format::R16 => rgba.extend(
                    image
                        .pixels
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .flat_map(|&[_, g]| [g, g, g, 255]),
                ),
                Format::R16G16 => rgba.extend(
                    image
                        .pixels
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .flat_map(|rg| [rg[1], rg[3], 0, 255]),
                ),
                Format::R16G16B16 => rgba.extend(
                    image
                        .pixels
                        .as_chunks::<6>()
                        .0
                        .iter()
                        .flat_map(|rgb| [rgb[1], rgb[3], rgb[5], 255]),
                ),
                Format::R16G16B16A16 => rgba.extend(
                    image
                        .pixels
                        .as_chunks::<8>()
                        .0
                        .iter()
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

            let (atlases, uvs) = if srgb_images.contains(&img_index) {
                (&mut srgb_atlases, &mut srgb_uvs)
            } else {
                (&mut linear_atlases, &mut linear_uvs)
            };

            // save parsed images for testing
            // #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
            // Self::save_rgba_image(
            //     &format!("parsed_image_{img_index}.png"),
            //     &rgba,
            //     image.width,
            //     image.height,
            // );

            let size = guillotiere::size2(image.width.cast_signed(), image.height.cast_signed());
            let mut allocation = None;
            let mut layer_index = 0;

            // try to find a layer that has enough space
            for (i, layer) in atlases.iter_mut().enumerate() {
                if let Some(alloc) = layer.allocator.allocate(size) {
                    allocation = Some(alloc);
                    layer_index = i;
                    break;
                }
            }

            // if all atlases are full, create a new atlas
            if allocation.is_none() {
                let mut layer = new_layer();
                allocation = layer.allocator.allocate(size);
                layer_index = atlases.len();
                atlases.push(layer);
            }

            let rect = allocation
                .context("single image is larger than ATLAS_SIZE atlas")?
                .rectangle;

            // blit the image into the atlas buffer
            let layer = &mut atlases[layer_index];
            for y in 0..image.height.cast_signed() {
                let src_start = (y * image.width.cast_signed() * 4) as usize;
                let src_end = src_start + (image.width.cast_signed() * 4) as usize;

                let dst_start =
                    ((rect.min.y + y) * ATLAS_SIZE as i32 * 4 + rect.min.x * 4) as usize;
                let dst_end = dst_start + (image.width.cast_signed() * 4) as usize;

                layer.pixels[dst_start..dst_end].copy_from_slice(&rgba[src_start..src_end]);
            }

            // calculate scale and offset based on the atlas
            let uv_offset_scale = glam::Vec4::new(
                rect.min.x as f32 / ATLAS_SIZE as f32,   // offset x
                rect.min.y as f32 / ATLAS_SIZE as f32,   // offset y
                image.width as f32 / ATLAS_SIZE as f32,  // scale x
                image.height as f32 / ATLAS_SIZE as f32, // scale y
            );

            uvs.insert(img_index, (layer_index as i32, uv_offset_scale));
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

        let mut geometries = Vec::new();
        let mut attributes = Vec::new();
        let mut materials = Vec::new();

        let mut bvh_triangles = Vec::new(); // contains all triangles in the scene, used for BVH construction

        // used so that primitives with the same material can reuse the same GpuMaterial instead of storing duplicate GpuMaterials in Scene
        let mut material_cache = std::collections::HashMap::new();

        // temporary vecs for processing each primitive; declared once here to save on performance of repeated allocations; these are cleared and reused for each primitive
        let mut temp_prim_positions = Vec::new();
        let mut temp_prim_normals = Vec::new();
        let mut temp_prim_tex_coords = Vec::new();
        let mut temp_prim_indices = Vec::new();
        let mut temp_prim_tangents = Vec::new();
        // used if tangents are calculated using mikktspace
        let mut temp_prim_computed_tangents = Vec::new();

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
                    if primitive.mode() != gltf::mesh::Mode::Triangles {
                        log::warn!(
                            "Mesh mode is not triangles, skipping primitive with mode {:?}",
                            primitive.mode()
                        );
                        continue;
                    }

                    // a reader can read the data of a primitive
                    let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()].0[..]));

                    temp_prim_positions.clear();
                    if let Some(it) = reader.read_positions() {
                        temp_prim_positions.extend(
                            it.map(|p| model_mat.transform_point3(glam::Vec3::from_array(p))),
                        );
                    } else {
                        log::warn!("Skipping primitive with no vertex positions");
                        continue;
                    }

                    temp_prim_indices.clear();
                    if let Some(it) = reader.read_indices() {
                        temp_prim_indices.extend(it.into_u32());
                    } else {
                        temp_prim_indices.extend(0..temp_prim_positions.len() as u32);
                    }

                    temp_prim_tex_coords.clear();
                    if let Some(it) = reader.read_tex_coords(0) {
                        temp_prim_tex_coords.extend(it.into_f32().map(glam::Vec2::from_array));
                    } else {
                        temp_prim_tex_coords
                            .extend(DUMMY_UV.iter().cycle().take(temp_prim_positions.len()));
                    }

                    temp_prim_normals.clear();
                    temp_prim_tangents.clear();

                    let mut use_computed_tangents = false;

                    if let Some(it) = reader.read_normals() {
                        temp_prim_normals.extend(it.map(|n| {
                            normal_mat
                                .transform_vector3(glam::Vec3::from_array(n))
                                .normalize()
                        }));
                        if let Some(it) = reader.read_tangents() {
                            for t in it {
                                #[expect(clippy::float_cmp)]
                                if t == [1.0, 0.0, 0.0, 1.0] {
                                    // a tangent of (1.0, 0.0, 0.0, 1.0) might be correct and intentional, but it's more often a placeholder tangent, so we don't use it
                                    use_computed_tangents = true;
                                    break;
                                }
                                let t_vec = glam::Vec4::from_array(t);
                                temp_prim_tangents.push(
                                    model_mat
                                        .transform_vector3(t_vec.truncate())
                                        .normalize()
                                        .extend(t_vec.w),
                                );
                            }

                            if !use_computed_tangents {
                                for chunk in temp_prim_indices.as_chunks::<3>().0 {
                                    let w0 = temp_prim_tangents[chunk[0] as usize].w;
                                    let w1 = temp_prim_tangents[chunk[1] as usize].w;
                                    let w2 = temp_prim_tangents[chunk[2] as usize].w;

                                    #[expect(clippy::float_cmp)]
                                    if w0 != w1 || w0 != w2 {
                                        use_computed_tangents = true;
                                        break;
                                    }
                                }
                            }
                        } else {
                            use_computed_tangents = true;
                        }
                    } else {
                        // if normals are not given, generate flat normals
                        // don't even bother trying to parse tangents, since glTF states that if normals aren't given but tangents are, the given tangents should be ignored
                        temp_prim_normals.resize(temp_prim_positions.len(), glam::Vec3::ZERO);
                        for chunk in temp_prim_indices.as_chunks::<3>().0 {
                            let [i0, i1, i2] =
                                [chunk[0] as usize, chunk[1] as usize, chunk[2] as usize];
                            let n = (temp_prim_positions[i1] - temp_prim_positions[i0])
                                .cross(temp_prim_positions[i2] - temp_prim_positions[i0])
                                .normalize();
                            temp_prim_normals[i0] = n;
                            temp_prim_normals[i1] = n;
                            temp_prim_normals[i2] = n;
                        }
                        use_computed_tangents = true;
                    }

                    if use_computed_tangents {
                        temp_prim_computed_tangents.clear();

                        // compute tangents with mikktspace
                        temp_prim_computed_tangents
                            .resize(temp_prim_indices.len(), glam::Vec4::ZERO);

                        bevy_mikktspace::generate_tangents(&mut MikkTSpaceGeometry {
                            indices: &temp_prim_indices,
                            positions: &temp_prim_positions,
                            normals: &temp_prim_normals,
                            tex_coords: &temp_prim_tex_coords,
                            tangents: &mut temp_prim_computed_tangents,
                        })?;

                        // the handedness is negated because of coordinate system nuances
                        // test these to verify that negation works:
                        // https://github.com/KhronosGroup/glTF-Sample-Assets/blob/main/Models/NormalTangentMirrorTest/README.md
                        // https://github.com/KhronosGroup/glTF-Sample-Assets/blob/main/Models/NormalTangentTest/README.md
                        for t in &mut temp_prim_computed_tangents {
                            t.w = -t.w;
                        }
                    }

                    let mat = primitive.material();

                    let material_index = if let Some(index) = mat.index() {
                        // if index is already in material_cache, return material_cache[index]
                        // note that material_cache stores keys which match the original material index from the glTF file,
                        // and values which are the corresponding index in the Scene::materials vec for the GpuMaterial
                        // this allows primitives with the same material to reuse the same GpuMaterial in Scene instead of storing duplicate GpuMaterials
                        // if index is not in material_cache, run the closure, which also stores the new index in material_cache since or_insert_with returns &mut material_cache[index]
                        *material_cache.entry(index).or_insert_with(|| {
                            let new_index = materials.len() as u32;
                            materials.push(GpuMaterial::new(&mat, &srgb_uvs, &linear_uvs));
                            new_index
                        })
                    } else {
                        // if there's no index, caching is impossible, so just parse the material and push it to the materials vec without caching
                        let new_index = materials.len() as u32;
                        materials.push(GpuMaterial::new(&mat, &srgb_uvs, &linear_uvs));
                        new_index
                    };

                    for (tri, chunk) in temp_prim_indices.as_chunks::<3>().0.iter().enumerate() {
                        let [i0, i1, i2] =
                            [chunk[0] as usize, chunk[1] as usize, chunk[2] as usize];

                        let v0 = temp_prim_positions[i0];
                        let v1 = temp_prim_positions[i1];
                        let v2 = temp_prim_positions[i2];

                        // calculate AABB and centroid of this triangle for the BVH builder later
                        bvh_triangles.push(BvhTriangle {
                            aabb: Aabb {
                                min: v0.min(v1).min(v2),
                                max: v0.max(v1).max(v2),
                            },
                            centroid: (v0 + v1 + v2) / 3.0,
                            original_index: geometries.len(),
                        });

                        geometries.push(GpuTriangleGeometry { v0, v1, v2 });

                        let (t0, t1, t2) = if use_computed_tangents {
                            let base = tri * 3;
                            (
                                temp_prim_computed_tangents[base],
                                temp_prim_computed_tangents[base + 1],
                                temp_prim_computed_tangents[base + 2],
                            )
                        } else {
                            (
                                temp_prim_tangents[i0],
                                temp_prim_tangents[i1],
                                temp_prim_tangents[i2],
                            )
                        };

                        attributes.push(GpuTriangleAttribute {
                            index: material_index,
                            n0: temp_prim_normals[i0],
                            n1: temp_prim_normals[i1],
                            n2: temp_prim_normals[i2],
                            uv0: temp_prim_tex_coords[i0],
                            uv1: temp_prim_tex_coords[i1],
                            uv2: temp_prim_tex_coords[i2],
                            t0,
                            t1,
                            t2,
                        });

                        // hence after this for loop, geometries[i], attributes[i], and bvh_triangles[i] all represent the same triangle at index i, as these three vecs are populated together in this same loop
                    }
                }
            }

            // push all children to the stack to be processed next
            stack.extend(node.children().map(|child| (child, model_mat)));
        }

        // create the root BVH node
        let mut bvh_nodes = vec![GpuBvhNode::new()];
        let triangle_count = bvh_triangles.len(); // the same as geometries.len() and attributes.len()
        anyhow::ensure!(triangle_count > 0, "scene has no triangles");
        bvh_nodes[0].update_aabb(&bvh_triangles[0..triangle_count]); // make the root node's aabb encompass all triangles of the scene
        // start recursive BVH construction
        Self::subdivide(0, &mut bvh_nodes, &mut bvh_triangles, 0, triangle_count);

        // reorder geometries and attributes to match bvh_triangles's ordering, so GpuBvhNode's left_first and triangle_count values can be used to index into them correctly
        geometries = bvh_triangles
            .iter()
            .map(|tri| geometries[tri.original_index]) // borrow checker is fine with this
            .collect();
        attributes = bvh_triangles
            .iter()
            .map(|tri| attributes[tri.original_index])
            .collect();

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
        // #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
        // {
        //     for (index, pixels) in scene.srgb_texture_atlases.iter().enumerate() {
        //         Self::save_rgba_image(
        //             &format!("srgb_atlas_{index}.png"),
        //             pixels,
        //             ATLAS_SIZE,
        //             ATLAS_SIZE,
        //         );
        //     }
        //     for (index, pixels) in scene.linear_texture_atlases.iter().enumerate() {
        //         Self::save_rgba_image(
        //             &format!("linear_atlas_{index}.png"),
        //             pixels,
        //             ATLAS_SIZE,
        //             ATLAS_SIZE,
        //         );
        //     }
        //     scene.save_scene_data_json();
        // }

        log::info!(
            "Loaded scene with {} triangles, {} materials, {} linear and {} sRGB texture atlases, {} BVH nodes",
            triangle_count,
            scene.materials.len(),
            scene.linear_texture_atlases.len(),
            scene.srgb_texture_atlases.len(),
            scene.bvh_nodes.len(),
        );

        #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
        {
            let mut max_bvh_depth = 0;
            let mut bvh_stack = vec![(0usize, 0usize)];
            while let Some((node_index, depth)) = bvh_stack.pop() {
                max_bvh_depth = max_bvh_depth.max(depth);

                let node = scene.bvh_nodes[node_index];
                if node.tri_count == 0 {
                    bvh_stack.push((node.left_first as usize, depth + 1));
                    bvh_stack.push(((node.left_first + 1) as usize, depth + 1));
                }
            }
            log::info!("BVH tree max depth: {max_bvh_depth}");
        }

        Ok(scene)
    }

    /// Recursive binned surface area heuristic subdivider.
    ///
    /// The current target [`GpuBvhNode`] is `nodes[node_index]`, and `bvh_triangles[start..end]` are the triangles this node spans.
    // TODO: look into BVH again. Spacial splits (SBVH)? Spatial-partitioning (like KD trees) is rarely used in modern path tracers, but are KD trees really worse?
    // Also, UBVH https://dcgi.fel.cvut.cz/en/publications/2025/kaceric-cgf-ubvh/, wide BVH, and better heuristic than SAH
    fn subdivide(
        node_index: usize,
        nodes: &mut Vec<GpuBvhNode>,
        bvh_triangles: &mut [BvhTriangle],
        start: usize,
        end: usize,
    ) {
        let tri_count = end - start;
        // if there are 2 or fewer triangles, make this node a leaf and terminate; otherwise, keep subdividing
        if tri_count <= 2 {
            nodes[node_index].left_first = start as u32;
            nodes[node_index].tri_count = tri_count as u32;
            return;
        }

        let mut centroids_aabb = Aabb::new();
        for tri in &bvh_triangles[start..end] {
            // grow the centroids_aabb to encompass tri.centroid
            centroids_aabb.min = centroids_aabb.min.min(tri.centroid);
            centroids_aabb.max = centroids_aabb.max.max(tri.centroid);
        }

        // find a good AABB split
        // the naive method is to sort all triangles along an axis and test splitting between every single triangle, is O(N logN)
        // instead, use binning to speed up to O(N)
        // rather than testing splits between every triangle, test only a set number of equally spaced splits
        // with 8 bins, we test 7 splits
        // see https://jacco.ompf2.com/2022/04/21/how-to-build-a-bvh-part-3-quick-builds/
        let mut best_axis = 0;
        // the index of the bin to split at (split at the "right" edge of this bin)
        // the largest this value can be is BVH_BINS - 2
        let mut best_split = 0;
        let mut best_cost = f32::MAX;
        // for axis in x, y, z
        for axis in 0..3 {
            let aabb_min = centroids_aabb.min[axis];
            let aabb_max = centroids_aabb.max[axis];

            #[expect(clippy::float_cmp)] // there should be no precision drift here
            if aabb_min == aabb_max {
                continue; // all triangle centroids are overlapping on this axis
            }

            let mut bins = [BvhBin {
                aabb: Aabb::new(),
                tri_count: 0,
            }; BVH_BINS];

            let scale = BVH_BINS as f32 / (aabb_max - aabb_min);
            for tri in &bvh_triangles[start..end] {
                // determine which bin this triangle belongs to based on this triangle's centroid
                let bin_index =
                    (((tri.centroid[axis] - aabb_min) * scale) as usize).min(BVH_BINS - 1);
                bins[bin_index].tri_count += 1;
                bins[bin_index].aabb.union(&tri.aabb); // the bin's aabb encompasses the entire triangle, not just its centroid
            }

            // find which of the BVH_BINS - 1 possible splits is the best:
            // with BVH_BINS = 8 total bins, would be to evaluate bin 0 and bins 1-7, bins 0-1 and bins 2-7, ..., bins 0-6 and bin 7

            // to evaluate any split, we need to know both the left and right bins' surface area and triangle count
            let mut left_area = [0.0; BVH_BINS - 1]; // [surface area of bin 0, of bins 0 and 1 unioned, ..., of all bins except BVH_BINS - 1 unioned]
            let mut left_count = [0; BVH_BINS - 1]; // [triangle count of bin 0, of bins 0 and 1, ..., of all bins except BVH_BINS - 1]
            let mut right_area = [0.0; BVH_BINS - 1]; // [surface area of all bins except 0 unioned, ..., of bins BVH_BINS - 1 and BVH_BINS - 2 unioned, of bin BVH_BINS - 1]
            let mut right_count = [0; BVH_BINS - 1]; // [triangle count of all bins except 0, ..., of bins BVH_BINS - 1 and BVH_BINS - 2, of bin BVH_BINS - 1]

            // left sweep to populate left_area and left_count
            let mut left_box = Aabb::new();
            let mut left_sum = 0;
            // with BVH_BINS = 8, is [0, 1, 2, 3, 4, 5, 6]
            for i in 0..BVH_BINS - 1 {
                left_sum += bins[i].tri_count;
                left_box.union(&bins[i].aabb);
                left_count[i] = left_sum;
                left_area[i] = left_box.surface_area_half();
            }
            // right sweep to populate right_area and right_count
            let mut right_box = Aabb::new();
            let mut right_sum = 0;
            // with BVH_BINS = 8, is [7, 6, 5, 4, 3, 2, 1]
            for i in (1..BVH_BINS).rev() {
                right_sum += bins[i].tri_count;
                right_box.union(&bins[i].aabb);
                right_count[i - 1] = right_sum;
                right_area[i - 1] = right_box.surface_area_half();
            }

            // evaluate each split
            for i in 0..BVH_BINS - 1 {
                // the surface area heuristic cost, lower is better
                // see https://jacco.ompf2.com/2022/04/18/how-to-build-a-bvh-part-2-faster-rays/
                let cost = (left_count[i] as f32)
                    .mul_add(left_area[i], right_count[i] as f32 * right_area[i]); // traversal cost is not considered
                if cost < best_cost {
                    best_cost = cost;
                    best_axis = axis;
                    best_split = i;
                }
            }
        }

        // if not splitting at all is better than the best SAH split, terminate here
        let leaf_cost = tri_count as f32 * {
            let e = nodes[node_index].aabb_max - nodes[node_index].aabb_min;
            e.z.mul_add(e.x, e.x.mul_add(e.y, e.y * e.z)) // omitting 2.0 coefficient
        };
        if best_cost >= leaf_cost {
            nodes[node_index].left_first = start as u32;
            nodes[node_index].tri_count = tri_count as u32;
            return;
        }

        // now that a split is chosen, reorder bvh_triangles so triangles on each side of the split are contiguous

        let aabb_min = centroids_aabb.min[best_axis];
        let aabb_max = centroids_aabb.max[best_axis];

        let scale = BVH_BINS as f32 / (aabb_max - aabb_min);

        let get_bin = |centroid: f32| -> usize {
            (((centroid - aabb_min) * scale) as usize).min(BVH_BINS - 1)
        };

        // modified Hoare's partition algorithm
        // move left and right inward until both find an out-of-place triangle; swap them; repeat
        let mut left = start;
        let mut right = end - 1;
        loop {
            while left <= right && get_bin(bvh_triangles[left].centroid[best_axis]) <= best_split {
                left += 1;
            }

            while right > left && get_bin(bvh_triangles[right].centroid[best_axis]) > best_split {
                right -= 1;
            }

            if left >= right {
                break;
            }

            bvh_triangles.swap(left, right);
            left += 1;
            right -= 1;
        }

        let split_index = left;

        // create child BVH nodes
        let left_child_index = nodes.len();
        let right_child_index = left_child_index + 1;
        nodes.push(GpuBvhNode::new());
        nodes.push(GpuBvhNode::new());
        nodes[left_child_index].update_aabb(&bvh_triangles[start..split_index]);
        nodes[right_child_index].update_aabb(&bvh_triangles[split_index..end]);

        // convert the current node from a leaf node to an internal node
        nodes[node_index].left_first = left_child_index as u32;
        nodes[node_index].tri_count = 0;

        Self::subdivide(left_child_index, nodes, bvh_triangles, start, split_index);
        Self::subdivide(right_child_index, nodes, bvh_triangles, split_index, end);
    }

    /// Find the material hit by a ray.
    // pub for criterion
    pub fn pick_material(&self, position: glam::Vec3, yaw: f32, pitch: f32) -> Option<usize> {
        let rotation = glam::Mat3::from_rotation_y(yaw) * glam::Mat3::from_rotation_x(pitch);
        let dir = rotation * glam::Vec3::NEG_Z;
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

        let mut stack = [0u32; 64];
        let mut stack_ptr = 0;

        stack[stack_ptr] = 0;
        stack_ptr += 1;

        let mut closest_t = f32::MAX;
        let mut hit_index: Option<usize> = None;

        while stack_ptr > 0 {
            stack_ptr -= 1;
            let node = self.bvh_nodes[stack[stack_ptr] as usize];

            let t0 = (node.aabb_min - position) * dir_recip;
            let t1 = (node.aabb_max - position) * dir_recip;

            let t_near = t0.min(t1).max_element();
            let t_far = t0.max(t1).min_element();

            if !(t_near <= t_far && t_far > 0.0 && t_near < closest_t) {
                continue;
            }

            if node.tri_count > 0 {
                for i in 0..node.tri_count {
                    let tri_index = (node.left_first + i) as usize;
                    let tri = self.geometries[tri_index];

                    let a = (tri.v0 - position).to_array();
                    let b = (tri.v1 - position).to_array();
                    let c = (tri.v2 - position).to_array();

                    let ax = sx.mul_add(a[kz], a[kx]);
                    let ay = sy.mul_add(a[kz], a[ky]);
                    let bx = sx.mul_add(b[kz], b[kx]);
                    let by = sy.mul_add(b[kz], b[ky]);
                    let cx = sx.mul_add(c[kz], c[kx]);
                    let cy = sy.mul_add(c[kz], c[ky]);

                    let u = cy.mul_add(-bx, cx * by);
                    let v = ay.mul_add(-cx, ax * cy);
                    let w = by.mul_add(-ax, bx * ay);

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
                // not testing children's aabb here for simplicity
                assert!(stack_ptr + 2 <= stack.len(), "BVH traversal stack overflow");

                stack[stack_ptr] = node.left_first + 1;
                stack_ptr += 1;
                stack[stack_ptr] = node.left_first;
                stack_ptr += 1;
            }
        }

        hit_index.map(|index| self.attributes[index].index as usize)
    }

    /// Get the scene uniforms and alias table.
    fn uniforms_and_alias_table(&self) -> (GpuSceneUniforms, Vec<GpuSceneAliasEntry>) {
        let mut indices = Vec::new();
        let mut weights = Vec::new();

        for (index, (geometry, attribute)) in
            self.geometries.iter().zip(&self.attributes).enumerate()
        {
            let material = self.materials[attribute.index as usize];
            let emission = luminance(material.emission_color_and_strength.truncate())
                * f64::from(material.emission_color_and_strength.w.max(0.0))
                * f64::from(material.OpenPBR_emission_weight.max(0.0));
            if emission == 0.0 {
                continue;
            }
            let area = (geometry.v1.as_dvec3() - geometry.v0.as_dvec3())
                .cross(geometry.v2.as_dvec3() - geometry.v0.as_dvec3())
                .length()
                * 0.5;
            // trace.wgsl currently adds emission from both sides of every triangle.
            let flux = std::f64::consts::TAU * area * emission;
            if flux > 0.0 && flux.is_finite() {
                indices.push(index as u32);
                weights.push(flux);
            }
        }

        let uniforms = GpuSceneUniforms {
            total_luminous_flux: (weights.iter().sum::<f64>()).min(f64::from(f32::MAX)) as f32,
            emissive_triangles_count: indices.len() as u32,
            light_probabilities: [0.0; 2], // updated after both categories' fluxes are ready
        };

        let entries = alias_table(&weights)
            .into_iter()
            .zip(indices)
            .map(|(entry, triangle_index)| GpuSceneAliasEntry {
                threshold: entry.threshold,
                alias_index: entry.alias_index,
                triangle_index,
                selection_probability: entry.selection_probability,
            })
            .collect();

        (uniforms, entries)
    }

    /// The radius of the entire scene, squared.
    fn radius_squared(&self) -> f64 {
        let root = self.bvh_nodes[0];
        (root.aabb_max.as_dvec3() - root.aabb_min.as_dvec3()).length_squared() * 0.25
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
                    log::info!("Saved scene data YAML to {}", output_path.display());
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

impl Skybox {
    /// Create a new [`Skybox`] using [`image::load_from_memory()`] with the given `bytes`.
    fn new(bytes: &[u8]) -> anyhow::Result<Self> {
        let img = image::load_from_memory(bytes)?.into_rgba32f();
        // alpha isn't used, but still stored and passed to the compute shader since there's no Rgb32Float wgpu::TextureFormat

        let (width, height) = img.dimensions();
        if width / 2 != height {
            log::warn!(
                "Skybox image should usually have 2:1 aspect ratio, but is {width}x{height}"
            );
        }

        let mut pixels = img.into_raw();
        // pixels[0, 1, 2, 3] are the RGBA values of pixel at the top-left corner
        // then, increasing indices go row-major (rightward first, then downward)

        let mut max_luma = 0.0_f64;
        let mut total_luma = 0.0_f64;
        let total_weight = 2.0 * f64::from(width);

        // Exact solid-angle row weights; the common longitude factor cancels.
        for (y, row) in pixels.chunks_exact(width as usize * 4).enumerate() {
            let weight = row_weight(y as u32, height);
            for pixel in row.as_chunks::<4>().0 {
                let luma = luminance(glam::Vec3::from_slice(pixel));
                max_luma = max_luma.max(luma);
                total_luma = luma.mul_add(weight, total_luma);
            }
        }

        // total_weight is zero only when height or width is zero, which is impossible

        // total_luma is zero only when height or width is zero or all pixels are black
        // then, just return a black texture
        if total_luma == 0.0 {
            log::info!("Loaded completely black skybox");

            return Ok(Self {
                rgba: vec![0.0; 4],
                uniforms: GpuSkyboxUniforms {
                    width: 1,
                    height: 1,
                    ..Default::default()
                },
                alias_table: Vec::new(),
            });
        }

        let avg_luma = total_luma / total_weight;
        let scale_factor = avg_luma.recip();

        // used to calculate standard deviation
        let mut variance_sum = 0.0;

        for (y, row) in pixels.chunks_exact_mut(width as usize * 4).enumerate() {
            let weight = row_weight(y as u32, height);
            for pixel in row.as_chunks_mut::<4>().0 {
                for channel in &mut pixel[..3] {
                    *channel = (f64::from(*channel) * scale_factor) as f32;
                }
                let scaled_luma = luminance(glam::Vec3::from_slice(pixel));

                // because pixels are scaled, the mean is exactly 1.0
                // variance is the squared difference from the mean
                variance_sum = (scaled_luma - 1.0).powi(2).mul_add(weight, variance_sum);
            }
        }

        // completely uniform skybox: max_to_avg_luma = 1.0, standard_deviation = 0.0
        let standard_deviation = (variance_sum / total_weight).sqrt();
        let max_to_avg_luma = max_luma * scale_factor; // equivalent to max_luma / avg_luma

        log::info!(
            "Loaded skybox {width}x{height} with originally {avg_luma} nits on average, {max_to_avg_luma} max/avg nits, standard deviation of {standard_deviation}"
        );

        Ok(Self {
            rgba: pixels,
            uniforms: GpuSkyboxUniforms {
                average_luminance: avg_luma as f32,
                width,
                height,
                ..Default::default()
            },
            alias_table: Vec::new(),
        })
    }

    fn update_alias_table(&mut self) {
        let mut weights = Vec::with_capacity(self.rgba.len() / 4);
        for (y, row) in self
            .rgba
            .chunks_exact(self.uniforms.width as usize * 4)
            .enumerate()
        {
            let weight = row_weight(y as u32, self.uniforms.height);
            weights.extend(
                row.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|pixel| luminance(glam::Vec3::from_slice(pixel)) * weight),
            );
        }
        self.alias_table = alias_table(&weights);
    }

    fn update_total_luminous_flux(&mut self, radius_squared: f64) {
        self.uniforms.total_luminous_flux = if self.alias_table.is_empty() {
            0.0
        } else {
            // see https://www.pbr-book.org/4ed/Light_Sources/Infinite_Area_Lights
            (4.0 * std::f64::consts::PI.powi(2)
                * radius_squared
                * f64::from(self.uniforms.average_luminance))
            .min(f64::from(f32::MAX)) as f32
        };
    }
}

impl GpuMaterial {
    /// Convert a glTF 2.0 material into a [`GpuMaterial`] for passing to the GPU, using the provided mapping of image indices to atlas layers and UV offset/scale.
    fn new(
        mat: &gltf::Material,
        srgb_image_uvs: &std::collections::HashMap<usize, (i32, glam::Vec4)>,
        linear_image_uvs: &std::collections::HashMap<usize, (i32, glam::Vec4)>,
    ) -> Self {
        let get_srgb_layer_and_uv = |img_index_opt: Option<usize>| -> (i32, glam::Vec4) {
            img_index_opt
                .and_then(|index| srgb_image_uvs.get(&index).copied())
                .unwrap_or((-1, glam::Vec4::ZERO)) // return -1 layer and zero UV if no texture attached
        };
        let get_linear_layer_and_uv = |img_index_opt: Option<usize>| -> (i32, glam::Vec4) {
            img_index_opt
                .and_then(|index| linear_image_uvs.get(&index).copied())
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

        let (emission_tex_layer, emission_uv) =
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

        Self {
            base_color_factor: glam::Vec4::from(pbr.base_color_factor()),
            OpenPBR_base_weight,
            metallic_factor: pbr.metallic_factor(),
            roughness_factor: pbr.roughness_factor(),
            OpenPBR_emission_weight,
            emission_color_and_strength: glam::Vec4::from((
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

            alpha_mode: match mat.alpha_mode() {
                gltf::material::AlphaMode::Opaque => ALPHA_MODE_OPAQUE,
                gltf::material::AlphaMode::Mask => ALPHA_MODE_MASK,
                gltf::material::AlphaMode::Blend => ALPHA_MODE_BLEND,
            },
            alpha_cutoff: mat.alpha_cutoff().unwrap_or(0.5),

            pad: 0.0,

            base_color_tex_layer,
            normal_tex_layer,
            metallic_roughness_tex_layer,
            emission_tex_layer,
            specular_factor_tex_layer,
            specular_color_tex_layer,

            base_color_uv,
            normal_uv,
            metallic_roughness_uv,
            emission_uv,
            specular_factor_uv,
            specular_color_uv,
        }
    }
}

impl Camera {
    fn gpu_camera(&self) -> GpuCamera {
        let rotation =
            glam::Mat3::from_rotation_y(self.yaw) * glam::Mat3::from_rotation_x(self.pitch);
        let height = 2.0 * (self.fov * 0.5).tan();
        let horizontal = rotation * glam::Vec3::X * self.aspect_ratio * height;
        let vertical = rotation * glam::Vec3::NEG_Y * height;
        GpuCamera {
            position: self.position.extend(0.0),
            lower_left_corner_minus_position: (rotation * glam::Vec3::NEG_Z
                - (horizontal + vertical) * 0.5)
                .extend(0.0),
            horizontal: horizontal.extend(0.0),
            vertical: vertical.extend(0.0),
        }
    }

    /// Calculate decent starting camera values from scene bounds.
    ///
    /// `min` and `max` should come from the root BVH node's axis-aligned bounding box.
    fn new(min: glam::Vec3, max: glam::Vec3) -> Self {
        let scene_center = (min + max) * 0.5;
        Self {
            position: glam::Vec3::new(
                scene_center.x,
                scene_center.y,
                (max.x - min.x).max(max.y - min.y).mul_add(0.6, max.z), // TODO: when more camera options are implemented, this offset value can be calculated so that it's guaranteed that the whole scene is visible in the window
            ),
            yaw: 0.0,
            pitch: 0.0,
            fov: std::f32::consts::FRAC_PI_2,
            aspect_ratio: 1.0,
            movement_speeds: [(max.x - min.x).max(max.z - min.z) * 0.4; 2],
            rotation_speeds: [0.003; 2],
        }
    }

    /// Move the camera based on currently pressed keys and distance multipliers (the amount of time elapsed since the last frame).
    pub(super) fn translate(
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

        self.translate_and_rotate_to(
            Some(self.position + move_xz + glam::Vec3::new(0.0, vertical_movement, 0.0)),
            None,
            None,
        );
    }

    /// Rotate the camera based on mouse movement deltas. `dx` and `dy` are the changes in mouse position.
    pub(super) const fn rotate(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * self.rotation_speeds[0];
        self.pitch -= dy * self.rotation_speeds[1];

        self.pitch = self
            .pitch
            .clamp(-std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2);
    }

    /// Resize the camera's aspect ratio.
    pub(super) fn resize(&mut self, aspect_ratio: f32) {
        if aspect_ratio.is_finite() && aspect_ratio > 0.0 {
            self.aspect_ratio = aspect_ratio;
        } else {
            log::error!("Aspect ratio must be finite and positive.");
        }
    }

    /// Change the camera's vertical field of view in radians. Clamps to [0, π).
    pub(super) const fn set_fov(&mut self, fov: f32) {
        self.fov = fov.clamp(0.0, std::f32::consts::PI - f32::EPSILON);
    }

    /// Get the vertical field of view.
    pub(super) const fn fov(&self) -> f32 {
        self.fov
    }

    /// Move the camera to a specific world-space position and rotation; yaw and pitch in radians.
    ///
    /// The coordinate system is: +X to the right of the screen, +Y up the screen, and -Z into the screen.
    pub(super) fn translate_and_rotate_to(
        &mut self,
        position: Option<glam::Vec3>,
        yaw: Option<f32>,
        pitch: Option<f32>,
    ) {
        self.position = position.unwrap_or(self.position);
        self.yaw = yaw.unwrap_or(self.yaw);
        self.pitch = pitch
            .unwrap_or(self.pitch)
            .clamp(-std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2);
    }

    /// Get the position in world space.
    pub(super) const fn position(&self) -> glam::Vec3 {
        self.position
    }

    /// Get the yaw in radians.
    pub(super) const fn yaw(&self) -> f32 {
        self.yaw
    }

    /// Get the pitch in radians.
    pub(super) const fn pitch(&self) -> f32 {
        self.pitch
    }
}
