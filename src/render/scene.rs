//! Load and format a 3D scene stored in a .glb glTF 2.0 file for passing to the GPU.

// this module is only used by state.rs
// on both native and wasm, loading is at runtime
// this allows the glTF file to change without having to rebuild the WASM

/// The size of each texture atlas in pixels.
///
/// 8192 is the default value of [`wgpu::Limits::max_texture_dimension_2d`].
/// Since passing an array of images with different sizes isn't supported universally,
/// images are packed into atlases which are larger square images that store multiple smaller images.
/// Then, an array of atlases are passed to the GPU without problems.
pub const ATLAS_SIZE: i32 = 8192;

// TODO: set atlas size dynamically; if one atlas is enough, use a smaller atlas size; if multiple are needed, use 8192

// TODO: fix transparency in textures; parsing is done correctly, but shader is not correct. also, in gltf::material, parse alpha_mode and alpha_cutoff to do this properly
// TODO: textures are messed up with sponza https://github.com/ludicon/sponza-gltf
// TODO: test against the sample glTFs

/// Contains all the data loaded from a glTF file and formatted for the GPU.
/// The coordiante system is: +X to the right of the screen, +Y up the screen, and -Z into the screen.
#[derive(Debug)]
pub struct Scene {
    // geometries[i] and attributes[i] together represent a triangle at index i
    // hence, only GpuTriangleAttribute contains material index (geometries doesn't contain any index)
    pub geometries: Vec<GpuTriangleGeometry>,
    pub attributes: Vec<GpuTriangleAttribute>,
    pub materials: Vec<GpuMaterial>,
    pub bvh_nodes: Vec<GpuBvhNode>,
    /// Each texture atlas is a RGBA8 image (stored as a `Vec<u8>`) that contains multiple smaller textures packed together;
    /// the shader uses an index into the array of texture atlases, the UV offset, and scale to access the correct atlas, then the correct portion of that atlas for each material.
    pub texture_atlases: Vec<Vec<u8>>,
    camera: Camera,
}

/// Used for keeping track of camera data internally; not passed to the GPU.
/// See [`GpuCamera`] for the struct that is passed to the GPU, which is converted from [`Camera`] with [`Scene::prepare_gpu_camera()`].
#[derive(Debug)]
struct Camera {
    /// Camera position x, y, z in world space.
    position: glam::Vec3,
    // focus distance is not needed since this is a pinhole camera; focus distance is implicitly 1
    /// Vertical FOV in degrees.
    fov_y: f32,
    /// Width divided by height of the image plane.
    aspect_ratio: f32,
    yaw: f32,
    pitch: f32,
    /// Movement speeds for horizontal and vertical movement, respectively, in units per second.
    movement_speeds: [f32; 2],
}

/// Contains the positions of the three vertices of a triangle.
// GpuTriangleGeometry and GpuTriangleAttribute are separate structs because the shader fetches vertices much more frequently than normals
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)] // Clone and Copy are required by Pod
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
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
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
    /// Normal vector at vertex 0.
    n0: glam::Vec3,
    /// Normal vector at vertex 1.
    n1: glam::Vec3,
    /// Normal vector at vertex 2.
    n2: glam::Vec3,
    /// UV vector at vertex 0.
    uv0: glam::Vec2,
    /// UV vector at vertex 1.
    uv1: glam::Vec2,
    /// UV vector at vertex 2.
    uv2: glam::Vec2,
    /// Tangent vector at vertex 0.
    t0: glam::Vec4,
    /// Tangent vector at vertex 1.
    t1: glam::Vec4,
    /// Tangent vector at vertex 2.
    t2: glam::Vec4,
}

/// Contains properties for one complete material.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[cfg_attr(
    all(feature = "testing", not(target_arch = "wasm32")),
    derive(serde::Serialize)
)]
pub struct GpuMaterial {
    /// Base color factor in RGBA; applied as a linear multiplier to the base color texture if it exists. If no texture, the base color is just this factor.
    base_color_factor: glam::Vec4,
    /// Offset (x, y) and scale (x, y) for the base color texture in an atlas; if no texture, this is zero and should be ignored in the shader.
    base_color_uv: glam::Vec4,
    /// Offset (x, y) and scale (x, y) for the normal texture in an atlas; if no texture, this is zero and should be ignored in the shader.
    normal_uv: glam::Vec4,
    /// Offset (x, y) and scale (x, y) for the metallic-roughness texture in an atlas; if no texture, this is zero and should be ignored in the shader.
    metallic_roughness_uv: glam::Vec4,
    /// Offset (x, y) and scale (x, y) for the emissive texture in an atlas; if no texture, this is zero and should be ignored in the shader.
    emissive_uv: glam::Vec4,
    /// Emissive color in RGB, and emissive strength in the 4th value; applied as a linear multiplier to the emissive texture if it exists. If no texture, the emissive color is just this factor.
    emissive: glam::Vec4,
    /// Index into the array of texture atlases for the base color texture; -1 if no texture.
    base_color_tex_layer: i32,
    /// Linear scale factor for the normal texture; if no normal texture, this should be ignored.
    normal_scale: f32,
    /// Index into the array of texture atlases for the normal texture; -1 if no texture.
    normal_tex_layer: i32,
    /// Index into the array of texture atlases for the metallic-roughness texture; -1 if no texture.
    metallic_roughness_tex_layer: i32,
    /// Index into the array of texture atlases for the emissive texture; -1 if no texture.
    emissive_tex_layer: i32,
    /// Metallic factor; a linear multiplier for the metallic value in the metallic-roughness texture if it exists. If no texture, this is the metallic value.
    metallic_factor: f32,
    /// Roughness factor; a linear multiplier for the roughness value in the metallic-roughness texture if it exists. If no texture, this is the roughness value.
    roughness_factor: f32,
    /// Flag indicating if the material is double-sided; 0 for false, 1 for true.
    double_sided: i32,
}

/// A node in the BVH tree.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuBvhNode {
    /// Axis-aligned bounding box's minimum of this node.
    aabb_min: glam::Vec3,
    /// If this node is an internal node, this value is the index of the left child node in [`Scene::bvh_nodes`].
    /// If this node is a leaf ([`Self::tri_count`] is not 0), this value is the starting offset into [`Scene::geometries`] for the triangles contained in this leaf.
    left_first: u32,
    /// Axis-aligned bounding box's maximum of this node.
    aabb_max: glam::Vec3,
    /// If this node is a leaf, this value is the number of triangles contained in this leaf.
    /// If this node is an internal node, this value is 0 and [`Self::left_first`] is the index in [`Scene::bvh_nodes`] of this node's left child.
    tri_count: u32,
}

/// Contains camera data formatted for the GPU; obtained with [`Scene::prepare_gpu_camera()`].
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
/* although the 4th value of each Vec4 is unused except for position_and_frame_count, if having Vec3 with matching WGSL:
struct GpuCamera {
    data: array<f32, 12>,
};
does not work for UNIFORM buffers
Error: Global variable [5] 'camera' is invalid: Alignment requirements for address space Uniform are not met by [10]The array stride 4 is not a multiple of the required alignment 16naga(15)
Even putting #[repr(C, align(16))] on this struct does not work
*/
pub struct GpuCamera {
    /// Camera position and frame count in the 4th value.
    /// When the camera moves, frame count is reset to 0, which is used for accumulation.
    position_and_frame_count: glam::Vec4,
    /// Lower-left pixel coordinate of image plane in world space.
    lower_left_corner: glam::Vec4,
    /// Vector that spans the full x of image plane in world space.
    horizontal: glam::Vec4,
    /// Vector that spans the full y of image plane in world space.
    vertical: glam::Vec4,
}

/// Internal BVH helper struct.
#[derive(Clone, Copy, Debug)]
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
    #[allow(clippy::use_self)]
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
#[derive(Clone, Copy)]
struct TriangleInfo {
    aabb: Aabb,
    centroid: glam::Vec3,
}

/// Adapter to use [`mikktspace::Geometry`] for tangent generation if needed.
struct MikkTSpaceGeometry<'a> {
    indices: &'a [u32],
    positions: &'a [glam::Vec3],
    normals: &'a [glam::Vec3],
    tex_coords: &'a [glam::Vec2],
    tangents: &'a mut [glam::Vec4],
}
impl mikktspace::Geometry for MikkTSpaceGeometry<'_> {
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
        if self.normals.is_empty() {
            let i0 = self.indices[face * 3] as usize;
            let i1 = self.indices[face * 3 + 1] as usize;
            let i2 = self.indices[face * 3 + 2] as usize;
            let p0 = self.positions[i0];
            let p1 = self.positions[i1];
            let p2 = self.positions[i2];
            (p1 - p0).cross(p2 - p0).normalize().into() // calculate flat normals
        } else {
            self.normals[self.indices[face * 3 + vert] as usize].into()
        }
    }
    fn tex_coord(&self, face: usize, vert: usize) -> [f32; 2] {
        self.tex_coords[self.indices[face * 3 + vert] as usize].into()
    }
    fn set_tangent_encoded(&mut self, tangent: [f32; 4], face: usize, vert: usize) {
        self.tangents[face * 3 + vert] = glam::Vec4::from(tangent);
    }
}

/// Load a glTF file as bytes, either from the filesystem on native or over HTTP on web.
// not included directly inside new because conditional compilation with variable scopes would get messy
#[allow(clippy::unused_async)]
async fn load_gltf_bytes(path: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Ok(std::fs::read(path)?)
    }

    // on web, the glTF file is fetched over HTTP
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast; // trait for dyn_into()

        let response: web_sys::Response = wasm_bindgen_futures::JsFuture::from(
            web_sys::window()
                .ok_or("no window object")? // ok_or() converts Option<> to Result<> with error message
                .fetch_with_str(path),
        )
        .await
        .map_err(|e| format!("{:?}", e))?
        .dyn_into()
        .map_err(|e| format!("{:?}", e))?;

        if !response.ok() {
            return Err(format!("network error: status {}", response.status()).into());
        }

        let u8_array = js_sys::Uint8Array::new(
            &wasm_bindgen_futures::JsFuture::from(
                response.array_buffer().map_err(|e| format!("{:?}", e))?,
            )
            .await
            .map_err(|e| format!("{:?}", e))?,
        );
        let mut bytes = vec![0u8; u8_array.length() as usize];
        u8_array.copy_to(&mut bytes[..]);
        Ok(bytes)
    }
}

/// Convert a glTF material into a [`GpuMaterial`] for passing to the GPU, using the provided mapping of image indices to atlas layers and UV offset/scale.
// not a closure inside new() because using a function seems to be slightly faster
fn parse_gpu_material(
    mat: &gltf::Material,
    image_uvs: &std::collections::HashMap<usize, (i32, glam::Vec4)>,
) -> GpuMaterial {
    let get_layer_and_uv = |img_idx_opt: Option<usize>| -> (i32, glam::Vec4) {
        img_idx_opt
            .and_then(|idx| image_uvs.get(&idx).copied())
            .unwrap_or((-1, glam::Vec4::ZERO)) // return -1 layer and zero UV if no texture attached
    };

    let pbr = mat.pbr_metallic_roughness();

    let (base_color_tex_layer, base_color_uv) = get_layer_and_uv(
        pbr.base_color_texture()
            .map(|t| t.texture().source().index()),
    );
    let (metallic_roughness_tex_layer, metallic_roughness_uv) = get_layer_and_uv(
        pbr.metallic_roughness_texture()
            .map(|t| t.texture().source().index()),
    );
    let norm_tex = mat.normal_texture();
    let (normal_tex_layer, normal_uv) =
        get_layer_and_uv(norm_tex.as_ref().map(|t| t.texture().source().index()));
    let (emissive_tex_layer, emissive_uv) =
        get_layer_and_uv(mat.emissive_texture().map(|t| t.texture().source().index()));

    GpuMaterial {
        base_color_factor: glam::Vec4::from(pbr.base_color_factor()),
        base_color_uv,
        normal_uv,
        metallic_roughness_uv,
        emissive_uv,
        emissive: glam::Vec4::new(
            mat.emissive_factor()[0],
            mat.emissive_factor()[1],
            mat.emissive_factor()[2],
            mat.emissive_strength().unwrap_or(1.0),
        ),
        base_color_tex_layer,
        normal_scale: norm_tex.map_or(1.0, |t| t.scale()),
        normal_tex_layer,
        metallic_roughness_tex_layer,
        emissive_tex_layer,
        metallic_factor: pbr.metallic_factor(),
        roughness_factor: pbr.roughness_factor(),
        double_sided: i32::from(mat.double_sided()),
    }
}

impl Scene {
    /// Load a glTF file from the given path and format it for the GPU.
    pub async fn new(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let (document, buffers, images) = gltf::import_slice(load_gltf_bytes(path).await?)?;

        let mut geometries = Vec::new();
        let mut attributes = Vec::new();
        let mut materials = Vec::new();
        let mut tri_infos = Vec::new();

        // the 2 extensions enabled are KHR_materials_emissive_strength and KHR_materials_specular
        // in Blender, with principled BSDF, if emission strength is larger than 1.0, KHR_materials_emissive_strength will automatically be used in the exported glTF file
        document.extensions_used().for_each(|s| {
            log::info!("glTF includes extension: {s}");
        });

        // precompute an 8-bit sRGB to linear lookup table to avoid expensive math per-pixel
        // TODO: Just hardcode the LUT
        // TODO: 8 bits isn't enough when converting from sRGB to linear, so consider 12 bits; see https://youtu.be/eWUDoms2iJo?t=240&si=sjInLYsrJn2H6EbX
        let mut srgb_to_linear_lut = [0u8; 256];
        for (i, c) in srgb_to_linear_lut.iter_mut().enumerate() {
            let f = i as f32 / 255.0;
            let linear = if f <= 0.04045 {
                f / 12.92
            } else {
                ((f + 0.055) / 1.055).powf(2.4)
            };
            *c = (linear * 255.0).round() as u8;
        }

        // collect indices of images designated as sRGB per glTF spec
        // only base color and emissive textures are sRGB in glTF
        let mut srgb_images = std::collections::HashSet::new();
        for material in document.materials() {
            if let Some(tex) = material.pbr_metallic_roughness().base_color_texture() {
                srgb_images.insert(tex.texture().source().index());
            }
            if let Some(tex) = material.emissive_texture() {
                srgb_images.insert(tex.texture().source().index());
            }
        }

        struct AtlasLayer {
            allocator: guillotiere::AtlasAllocator,
            pixels: Vec<u8>,
        }
        let mut atlases = vec![AtlasLayer {
            allocator: guillotiere::AtlasAllocator::new(guillotiere::size2(ATLAS_SIZE, ATLAS_SIZE)),
            pixels: vec![0; (ATLAS_SIZE * ATLAS_SIZE * 4) as usize], // * 4 for RGBA
        }];

        let mut image_uvs = std::collections::HashMap::new();

        let mut rgba = Vec::new(); // declare once to save performance

        // loop through each image
        for (img_idx, image) in images.iter().enumerate() {
            rgba.clear();

            use gltf::image::Format;

            #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
            log::info!("Converting image with format {:?} to RGBA8", image.format);

            // convert image into RGBA8
            // most textures from most models are 8 bit; higher bit depth textures are uncommon
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

            // convert from sRGB to linear
            if srgb_images.contains(&img_idx) {
                for pixel in rgba.chunks_exact_mut(4) {
                    pixel[0] = srgb_to_linear_lut[pixel[0] as usize];
                    pixel[1] = srgb_to_linear_lut[pixel[1] as usize];
                    pixel[2] = srgb_to_linear_lut[pixel[2] as usize];
                    // the alpha channel is linear per glTF spec, so don't modify pixel[3]
                }
            }

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
                let mut new_layer = AtlasLayer {
                    allocator: guillotiere::AtlasAllocator::new(guillotiere::size2(
                        ATLAS_SIZE, ATLAS_SIZE,
                    )),
                    pixels: vec![0; (ATLAS_SIZE * ATLAS_SIZE * 4) as usize],
                };
                allocation = new_layer.allocator.allocate(size);
                layer_idx = atlases.len();
                atlases.push(new_layer);
            }

            let alloc = allocation.expect("Single image is larger than ATLAS_SIZE atlas.");
            let rect = alloc.rectangle;

            // blit the image into the atlas buffer
            let layer = &mut atlases[layer_idx];
            for y in 0..image.height.cast_signed() {
                let src_start = (y * image.width.cast_signed() * 4) as usize;
                let src_end = src_start + (image.width.cast_signed() * 4) as usize;

                let dst_start = ((rect.min.y + y) * ATLAS_SIZE * 4 + rect.min.x * 4) as usize;
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

            #[allow(clippy::cast_possible_wrap)]
            image_uvs.insert(img_idx, (layer_idx as i32, uv_offset_scale));
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
                            temp_tex_coords.resize(temp_positions.len(), glam::Vec2::ZERO); // if there are no tex coords, use (0, 0) for all vertices
                        }

                        temp_normals.clear();
                        if let Some(it) = reader.read_normals() {
                            temp_normals.extend(it.map(|n| {
                                normal_mat
                                    .transform_vector3(glam::Vec3::from_array(n))
                                    .normalize()
                            }));
                        }

                        temp_tangents.clear();
                        if !temp_normals.is_empty() // per glTF spec, if tangents are provided but normals aren't, the provided tangents should be ignored
                            && let Some(it) = reader.read_tangents()
                        {
                            temp_tangents.extend(it.map(|t| {
                                let t_vec = glam::Vec4::from_array(t);
                                model_mat
                                    .transform_vector3(t_vec.truncate())
                                    .normalize()
                                    .extend(t_vec.w)
                            }));
                        }

                        computed_tangents.clear();
                        let use_computed_tangents = temp_tangents.is_empty(); // temp_tangents is only populated if both normals and tangents are available in the glb

                        if use_computed_tangents {
                            // compute tangents with mikktspace
                            computed_tangents.resize(temp_indices.len(), glam::Vec4::ZERO);

                            mikktspace::generate_tangents(&mut MikkTSpaceGeometry {
                                indices: &temp_indices,
                                positions: &temp_positions,
                                normals: &temp_normals,
                                tex_coords: &temp_tex_coords,
                                tangents: &mut computed_tangents,
                            });
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
                                materials.push(parse_gpu_material(&mat, &image_uvs));
                                new_idx
                            })
                        } else {
                            // if there's no index, caching is impossible, so just parse the material and push it to the materials vec without caching
                            let new_idx = materials.len() as u32;
                            materials.push(parse_gpu_material(&mat, &image_uvs));
                            new_idx
                        };

                        for (tri, chunk) in temp_indices.chunks_exact(3).enumerate() {
                            let [i0, i1, i2] =
                                [chunk[0] as usize, chunk[1] as usize, chunk[2] as usize];

                            let p0 = temp_positions[i0];
                            let p1 = temp_positions[i1];
                            let p2 = temp_positions[i2];

                            let (n0, n1, n2) = if temp_normals.is_empty() {
                                let n = (p1 - p0).cross(p2 - p0).normalize(); // calculate flat normal
                                (n, n, n)
                            } else {
                                (temp_normals[i0], temp_normals[i1], temp_normals[i2])
                            };

                            let (mut t0, mut t1, mut t2) = if use_computed_tangents {
                                let base = tri * 3;
                                (
                                    computed_tangents[base],
                                    computed_tangents[base + 1],
                                    computed_tangents[base + 2],
                                )
                            } else {
                                (temp_tangents[i0], temp_tangents[i1], temp_tangents[i2])
                            };

                            // per glTF spec, if a triangle's tangents' w values are not the same, the triangle's tangent space is considered undefined
                            #[allow(clippy::float_cmp)]
                            if t0.w != t1.w || t0.w != t2.w {
                                t0 = glam::Vec4::ZERO;
                                t1 = glam::Vec4::ZERO;
                                t2 = glam::Vec4::ZERO;
                            }

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
                                n0,
                                n1,
                                n2,
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
            "Loaded scene with {} triangles, {} materials, {} texture atlases; starting BVH construction",
            geometries.len(),
            materials.len(),
            atlases.len(),
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
            "Finished BVH construction with {} nodes, took {:?}",
            bvh_nodes.len(),
            bvh_construction_start.elapsed()
        );

        // calculate a decent starting camera position from scene bounds
        // to disable this automatic camera positioning, just use move_camera_to() immediately after scene creation
        let root_aabb = bvh_nodes[0];
        let scene_center = (root_aabb.aabb_min + root_aabb.aabb_max) * 0.5;
        let scene_height = root_aabb.aabb_max.y - root_aabb.aabb_min.y;

        let scene = Self {
            geometries,
            attributes,
            materials,
            bvh_nodes,
            texture_atlases: atlases.into_iter().map(|layer| layer.pixels).collect(), // map out just the pixels from the AtlasLayer structs
            camera: Camera {
                position: glam::Vec3::new(
                    scene_center.x,
                    scene_height.mul_add(0.01, root_aabb.aabb_max.y), // automatically position the camera above the top of the scene bounds
                    scene_center.z,
                ),
                fov_y: 90.0,
                aspect_ratio: 1.0,
                yaw: 0.0,
                pitch: 0.0,
                movement_speeds: [(root_aabb.aabb_max.x - root_aabb.aabb_min.x)
                    .max(root_aabb.aabb_max.z - root_aabb.aabb_min.z)
                    * 0.25; 2],
            },
        };

        #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
        {
            for (idx, pixels) in scene.texture_atlases.iter().enumerate() {
                Self::save_rgba_image(
                    &format!("atlas_{idx}.png"),
                    pixels,
                    ATLAS_SIZE as u32,
                    ATLAS_SIZE as u32,
                );
            }
            scene.save_scene_data_json();
        }

        Ok(scene)
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
            #[allow(clippy::float_cmp)] // there should be no precision drift here
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

    /// Resize the camera's aspect ratio, typically called when the viewport is resized.
    pub fn resize_camera_aspect_ratio(&mut self, width: f32, height: f32) {
        self.camera.aspect_ratio = width / height;
    }

    /// Move the camera based on currently pressed keys. Returns `true` if the camera was moved, `false` if no relevant keys were pressed or if opposite keys cancel out.
    pub fn move_camera(
        &mut self,
        pressed_keys: &std::collections::HashSet<winit::keyboard::KeyCode>,
        horizontal_speed: f32,
        vertical_speed: f32,
    ) -> bool {
        let w = pressed_keys.contains(&winit::keyboard::KeyCode::KeyW);
        let s = pressed_keys.contains(&winit::keyboard::KeyCode::KeyS);
        let a = pressed_keys.contains(&winit::keyboard::KeyCode::KeyA);
        let d = pressed_keys.contains(&winit::keyboard::KeyCode::KeyD);
        let space = pressed_keys.contains(&winit::keyboard::KeyCode::Space);
        let shift = pressed_keys.contains(&winit::keyboard::KeyCode::ShiftLeft);
        let q = pressed_keys.contains(&winit::keyboard::KeyCode::KeyQ);

        if !(w || s || a || d || space || shift) {
            return false;
        }

        let speed_multiplier = if q { 4.0 } else { 1.0 };

        let (sin_yaw, cos_yaw) = self.camera.yaw.sin_cos();

        let forward_xz = glam::Vec3::new(-sin_yaw, 0.0, -cos_yaw);
        let right_xz = glam::Vec3::new(cos_yaw, 0.0, -sin_yaw);

        let forward_coeff = (if w { 1.0 } else { 0.0 }) - (if s { 1.0 } else { 0.0 });
        let right_coeff = (if d { 1.0 } else { 0.0 }) - (if a { 1.0 } else { 0.0 });

        let intent = forward_xz * forward_coeff + right_xz * right_coeff;

        let move_xz = if intent.length_squared() > f32::EPSILON {
            intent.normalize()
                * horizontal_speed
                * speed_multiplier
                * self.camera.movement_speeds[0]
        } else {
            glam::Vec3::ZERO
        };

        let vert_dir = (if space { 1.0 } else { 0.0 }) - (if shift { 1.0 } else { 0.0 });
        let vertical_movement =
            vert_dir * vertical_speed * speed_multiplier * self.camera.movement_speeds[1];

        // check if there's any actual movement
        // keys might be pressed but cancel each other out (e.g. W and S)
        // == 0.0 instead of <= f32::EPSILON because with tiny scenes, allow very slow movement speeds without it being considered as no movement
        if move_xz.length_squared() == 0.0 && vertical_movement.abs() == 0.0 {
            return false;
        }

        self.camera.position += move_xz + glam::Vec3::new(0.0, vertical_movement, 0.0);

        true
    }

    /// Instantly move the camera to a specific position, ignoring any currently pressed keys. Useful for teleporting or resetting camera position.
    pub const fn move_camera_to(&mut self, position: glam::Vec3) {
        self.camera.position = position;
    }

    /// Rotate the camera based on mouse movement deltas. `dx` and `dy` are the changes in mouse position since the last frame, and the sensitivities control how much the camera rotates in response to mouse movement.
    pub fn rotate_camera(
        &mut self,
        dx: f32,
        dy: f32,
        horizontal_sensitivity: f32,
        vertical_sensitivity: f32,
    ) {
        self.camera.yaw -= dx * horizontal_sensitivity;
        self.camera.pitch -= dy * vertical_sensitivity;

        self.camera.pitch = self
            .camera
            .pitch
            .clamp(-std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2);
    }

    /// Instantly rotate the camera to specific yaw and pitch angles (in radians), ignoring mouse movement. Useful for teleporting or resetting the camera orientation.
    pub fn rotate_camera_to(&mut self, yaw: f32, pitch: f32) {
        self.camera.yaw = yaw;
        self.camera.pitch = pitch;

        self.camera.pitch = self
            .camera
            .pitch
            .clamp(-std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2);
    }

    /// Prepare a struct for the GPU with the current camera parameters. The `frame_count` parameter is included in the `position_and_frame_count` field, which can be used in shaders for effects that depend on the number of frames rendered (e.g., accumulation).
    pub fn prepare_gpu_camera(&self, frame_count: u32) -> GpuCamera {
        let cam = &self.camera;

        let rotation_mat =
            glam::Mat3::from_rotation_y(cam.yaw) * glam::Mat3::from_rotation_x(cam.pitch);

        let image_plane_height = 2.0 * (cam.fov_y.to_radians() / 2.0).tan();

        let horizontal3 = rotation_mat.mul_vec3(glam::Vec3::X).normalize()
            * cam.aspect_ratio
            * image_plane_height;

        let vertical3 = rotation_mat.mul_vec3(glam::Vec3::NEG_Y).normalize() * image_plane_height;

        let forward3 = rotation_mat.mul_vec3(glam::Vec3::NEG_Z).normalize();

        let lower_left_corner3 = cam.position + forward3 - horizontal3 / 2.0 - vertical3 / 2.0;

        GpuCamera {
            position_and_frame_count: glam::Vec4::from((cam.position, frame_count as f32)),
            lower_left_corner: glam::Vec4::from((lower_left_corner3, 0.0)),
            horizontal: glam::Vec4::from((horizontal3, 0.0)),
            vertical: glam::Vec4::from((vertical3, 0.0)),
        }
    }

    /// Save the scene data (geometries, attributes, materials) to a YAML file in the `debug_output/` directory at the crate root. This is only compiled when the `testing` feature is enabled and the target architecture is not wasm32, as file system access is not available in WebAssembly.
    #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
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

    /// Save a texture atlas as a PNG image in the `debug_output/` directory at the crate root. This is only compiled when the `testing` feature is enabled and the target architecture is not wasm32, as file system access is not available in WebAssembly.
    #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
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
