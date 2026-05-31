// the main path tracing shader
// implements a basic path tracer with support for diffuse and metallic/specular materials, normal interpolation, emissive materials, etc.
// supports texture maps for base color, metallic/roughness, normal, emissive

// TODO: double_sided is never actually used
// TODO: implement tracing shadow rays for better denoising
// TODO: dorm scene has weird vertical shading problem
// TODO: not sure about the math; recheck them
// TODO: investigate why web performance is much worse than on native
// TODO: investigate black edges on round models

// 21.23ms 21.2ms 15.36μs at 800x600 dorm_light for compute and render scope, compute pass scoped, and render pass scoped

// these structs match with Rust side definitions

// GpuTriangleGeometry and GpuTriangleAttribute are separate structs just because the shader fetches vertices much more frequently than normals
// although, actual benchmarks show that the performance difference is negligible, so they could be combined into one struct, but would require more padding, so they're kept separate for now
struct GpuTriangleGeometry {
    coords: array<f32, 9>, // 3 vertices with x, y, z for each vertex
    // stored as a flat array since vec3<T> has alignment of 16 bytes, which misaligns with Rust's glam::Vec3 which has alignment of 12 bytes
    // using a flat array allows packing the vertex data without padding
    // the extra code like
    // p0 = vec3<f32>(tri.coords[0], tri.coords[1], tri.coords[2])
    // should be optimized by the compiler and not cause a performance difference
    // benchmarks show that using a flat array vs vec3 doesn't cause a performance difference
};
struct GpuTriangleAttribute {
    index: u32, // index into the GpuMaterial
    normals: array<f32, 9>, // same packing logic as GpuTriangleGeometry for the normals of the 3 vertices
    uv0: vec2<f32>,
    uv1: vec2<f32>,
    uv2: vec2<f32>,
    t0: vec4<f32>,
    t1: vec4<f32>,
    t2: vec4<f32>,
};

struct BvhNode {
    aabb_min: vec3<f32>,
    left_first: u32,
    aabb_max: vec3<f32>,
    prim_count: u32,
}

// see Rust code for more detailed comments
struct GpuMaterial {
    base_color_factor: vec4<f32>,
    base_color_uv: vec4<f32>, // offset_x, offset_y, scale_x, scale_y
    normal_uv: vec4<f32>,
    metallic_roughness_uv: vec4<f32>,
    emissive_uv: vec4<f32>,
    emissive: vec4<f32>, // r, g, b, strength
    base_color_tex_layer: i32, // index the array of texture atlases for this texture; -1 if no texture; same goes for the other texture layers
    normal_scale: f32,
    normal_tex_layer: i32,
    metallic_roughness_tex_layer: i32,
    emissive_tex_layer: i32,
    metallic_factor: f32,
    roughness_factor: f32,
    double_sided: i32, // 0 for false, 1 for true; used to conditionally enable backface culling
    // when true, the back-face must have its normals reversed before the lighting is evaluated
};

struct GpuCamera {
    position_and_frame_count: vec4<f32>, // camera position, 4th value is the frame_count for accumulation
    // in Rust, frame_count resets to zero when the camera pans or moves
    lower_left_corner: vec4<f32>,
    horizontal: vec4<f32>,
    vertical: vec4<f32>,
};

@group(0) @binding(0) var screen: texture_storage_2d<rgba16float, write>;
@group(0) @binding(1) var<storage, read_write> accumulation_buffer: array<vec4<f32>>; // this is actually used as a texture, but since textures cannot be both read and write, a storage buffer is used instead
@group(1) @binding(0) var<storage, read> triangles_geo: array<GpuTriangleGeometry>;
@group(1) @binding(1) var<storage, read> bvh_nodes: array<BvhNode>;
@group(1) @binding(2) var<storage, read> triangles_attr: array<GpuTriangleAttribute>;
@group(1) @binding(3) var<storage, read> materials: array<GpuMaterial>;
@group(1) @binding(4) var texture_atlas: texture_2d_array<f32>;
@group(1) @binding(5) var atlas_sampler: sampler;
@group(1) @binding(6) var<uniform> camera: GpuCamera;

struct Ray {
    origin: vec3<f32>, // camera origin position in world space
    direction: vec3<f32>, // normalized, direction going from camera origin to a point on the image plane
};

struct HitRecord {
    t: f32, // distance along the ray where a triangle is hit
    p: vec3<f32>, // world space point where the ray intersects a triangle
    material_index: u32,
    uv: vec2<f32>,   // interpolated triangle UV
    normal: vec3<f32>,
    tangent: vec4<f32>, // xyz and sign
};

// PCG random number generator
// https://www.pcg-random.org/index.html
// based on https://github.com/bevyengine/bevy/pull/11956/changes
var<private> seed: u32;
fn init_rand(invocation_id: vec3<u32>, frame_count: u32) {
    seed = ((invocation_id.x * 1973u) + (invocation_id.y * 9277u) + (frame_count * 26699u)) | 1u;
    pcg_hash();
    pcg_hash();
}
fn pcg_hash() -> u32 {
    seed = seed * 747796405u + 2891336453u;
    var word: u32 = ((seed >> ((seed >> 28u) + 4u)) ^ seed) * 277803737u;
    return (word >> 22u) ^ word;
}
fn rand() -> f32 {
    return f32(pcg_hash()) / 4294967296.0;
}

// importance sampling functions

fn get_tangent_space(n: vec3<f32>) -> mat3x3<f32> {
    let up = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(n.z) < 0.999);
    let t = normalize(cross(up, n));
    let b = cross(n, t);
    return mat3x3<f32>(t, b, n);
}

// diffuse cosine-weighted hemisphere sampling
fn cosine_sample_hemisphere(n: vec3<f32>) -> vec3<f32> {
    let r1 = rand();
    let r2 = rand();
    let phi = 2.0 * 3.14159265 * r1;
    let r = sqrt(r2);
    let x = cos(phi) * r;
    let y = sin(phi) * r;
    let z = sqrt(max(0.0, 1.0 - r2));
    return get_tangent_space(n) * vec3<f32>(x, y, z);
}

// specular GGX hemisphere sampling
fn ggx_sample_hemisphere(n: vec3<f32>, alpha: f32) -> vec3<f32> {
    let r1 = rand();
    let r2 = rand();
    let phi = 2.0 * 3.14159265 * r1;
    let cosTheta = sqrt((1.0 - r2) / (1.0 + (alpha * alpha - 1.0) * r2));
    let sinTheta = sqrt(max(0.0, 1.0 - cosTheta * cosTheta));
    let x = cos(phi) * sinTheta;
    let y = sin(phi) * sinTheta;
    let z = cosTheta;
    return get_tangent_space(n) * vec3<f32>(x, y, z);
}

// BVH traversal to find closest ray-triangle intersection; returns a HitRecord with t < 0 if no hit
fn trace(ray: Ray) -> HitRecord {
    var closest_t = 1.0e+20;
    var hit_idx: i32 = -1;
    var hit_u: f32 = 0.0;
    var hit_v: f32 = 0.0;

    // fixed-size stack for BVH traversal (depth 64 is sufficient for millions of primitives)
    var stack: array<u32, 64>;
    var stack_ptr: u32 = 0u;

    // push root node
    stack[stack_ptr] = 0u;
    stack_ptr += 1u;

    while stack_ptr > 0u {
        // pop node
        stack_ptr -= 1u;
        let node_idx = stack[stack_ptr];
        let node = bvh_nodes[node_idx];

        // ray-AABB intersection (slab method)
        let inv_dir = 1.0 / ray.direction; // https://www.w3.org/TR/WGSL/#differences-from-ieee754 division by zero naturally results in infinity, which works with AABB math

        let t0 = (node.aabb_min - ray.origin) * inv_dir;
        let t1 = (node.aabb_max - ray.origin) * inv_dir;

        let tmin = min(t0, t1);
        let tmax = max(t0, t1);

        let t_near = max(max(tmin.x, tmin.y), tmin.z);
        let t_far = min(min(tmax.x, tmax.y), tmax.z);

        // if ray misses the bounding box or is further than the closest hit, skip it
        if !(t_near <= t_far && t_far > 0.0 && t_near < closest_t) {
            continue;
        }

        if node.prim_count > 0u { // leaf node; intersect with primitives
            for (var i = 0u; i < node.prim_count; i += 1u) {
                let tri_idx = node.left_first + i;

                // Möller–Trumbore algorithm
                // implementation based on https://w.wiki/y6d
                let tri = triangles_geo[tri_idx];
                let p0 = vec3<f32>(tri.coords[0], tri.coords[1], tri.coords[2]);
                let p1 = vec3<f32>(tri.coords[3], tri.coords[4], tri.coords[5]);
                let p2 = vec3<f32>(tri.coords[6], tri.coords[7], tri.coords[8]);
                let edge1 = p1 - p0; // two edges spanning the triangle
                let edge2 = p2 - p0;

                let ray_cross_edge2 = cross(ray.direction, edge2); // ray_cross_edge2 is perpendicular to ray.direction and edge2
                let det = dot(edge1, ray_cross_edge2); // det measures how non-parallel the ray is to the plane the triangle is on 

                // testing if abs(det) < 0.000001 here is common to determine if ray is parallel to the triangle plane, but can cause issues with very small triangles

                let inv_det = 1.0 / det;
                let s = ray.origin - p0;
                let u = dot(s, ray_cross_edge2) * inv_det;
                if u < 0.0 || u > 1.0 { continue; }

                let s_cross_edge1 = cross(s, edge1);
                let v = dot(ray.direction, s_cross_edge1) * inv_det;
                if v < 0.0 || u + v > 1.0 { continue; }

                let t = dot(edge2, s_cross_edge1) * inv_det;
                if t > 0.0 && t < closest_t {
                    closest_t = t;
                    hit_idx = i32(tri_idx);
                    hit_u = u;
                    hit_v = v;
                }
            }
        } else { // internal node; push children onto stack
            // left_first contains the index of the left child; right child is contiguous at left_first + 1
            stack[stack_ptr] = node.left_first + 1u; // push right
            stack_ptr += 1u;
            stack[stack_ptr] = node.left_first;      // push left
            stack_ptr += 1u;
        }
    }

    var hit_rec: HitRecord;
    hit_rec.t = -1.0;

    if hit_idx != -1 {
        let attr = triangles_attr[u32(hit_idx)];

        hit_rec.t = closest_t;
        hit_rec.p = ray.direction * closest_t + ray.origin; // this matches f(x) = m * x + b, but in this case in 3D, f(x) outputs a vec3 point, which is the intersection point
        hit_rec.material_index = attr.index;

        // only 2 barycentric coordinates are given, but we know that the point is valid inside the triangle so all coordinates must sum to 1
        // so, the third barycentric coordinate can just be computed
        let w = 1.0 - hit_u - hit_v;

        hit_rec.uv = w * attr.uv0 + hit_u * attr.uv1 + hit_v * attr.uv2;

        let n0 = vec3<f32>(attr.normals[0], attr.normals[1], attr.normals[2]);
        let n1 = vec3<f32>(attr.normals[3], attr.normals[4], attr.normals[5]);
        let n2 = vec3<f32>(attr.normals[6], attr.normals[7], attr.normals[8]);
        let interp_normal = normalize(w * n0 + hit_u * n1 + hit_v * n2);

        let is_back_face = dot(ray.direction, interp_normal) > 0.0;
        hit_rec.normal = select(interp_normal, -interp_normal, is_back_face);

        let interp_tangent = w * attr.t0 + hit_u * attr.t1 + hit_v * attr.t2;
        let t_xyz = select(interp_tangent.xyz, -interp_tangent.xyz, is_back_face);
        let t_len_sq = dot(t_xyz, t_xyz);
        let safe_t_len_sq = max(t_len_sq, 0.0);

        let valid_t = select(vec3<f32>(1.0, 0.0, 0.0), t_xyz * inverseSqrt(safe_t_len_sq), t_len_sq > 0.0);
        hit_rec.tangent = vec4<f32>(valid_t, select(-1.0, 1.0, interp_tangent.w >= 0.0));
    }

    return hit_rec;
}

// Smith G1 function
fn smith_g1(NdotV: f32, alpha: f32) -> f32 {
    let alpha_sq = alpha * alpha;
    let NdotV_sq = NdotV * NdotV;
    return 2.0 * NdotV / (NdotV + sqrt(alpha_sq + (1.0 - alpha_sq) * NdotV_sq));
}

@compute @workgroup_size(8, 8, 1)
fn compute_main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    // each global_id comes from compute_pass.dispatch_workgroups() in Rust
    // each global_id.x and global_id.y should yield a pixel on the texture/surface

    let screen_dims = vec2<f32>(textureDimensions(screen));

    if global_id.x >= u32(screen_dims.x) || global_id.y >= u32(screen_dims.y) {
        return;
    }

    let frame_count = u32(camera.position_and_frame_count.w);
    init_rand(global_id, frame_count);

    // subpixel jitter for anti-aliasing (TAA)
    let jitter = vec2<f32>(rand() - 0.5, rand() - 0.5);
    // convert the x and y on the texture/surface to normalized values between 0 and 1
    // + vec2<f32>(0.5) to realign each texel to the center of the texel 
    let uv = (vec2<f32>(global_id.xy) + vec2<f32>(0.5) + jitter) / screen_dims;

    // create a Ray with origin at camera and direction to a given image plane point
    // the image plane point is defined by uv, which has x and y normalized to 0 to 1
    var ray: Ray;
    ray.origin = camera.position_and_frame_count.xyz;
    // normalized vector going from camera origin to the image plane coordinate in world space
    ray.direction = normalize(
        camera.lower_left_corner.xyz + uv.x * camera.horizontal.xyz + uv.y * camera.vertical.xyz - ray.origin
    );

    var throughput = vec3<f32>(1.0, 1.0, 1.0);
    var final_color = vec3<f32>(0.0, 0.0, 0.0);

    // path tracing loop
    for (var bounce = 0; bounce < 20; bounce += 1) {
        let hit = trace(ray);

        // ray escaped the scene, render sky/environment with colorful debug bands
        if hit.t < 0.0 {
            // normal sky gradient
            // let t_bg = 0.5 * (ray.direction.y + 1.0);
            // let sky_color = mix(vec3<f32>(0.05, 0.05, 0.05), vec3<f32>(0.2, 0.3, 0.4), t_bg);

            let dir_y = ray.direction.y;
            var sky_color = vec3<f32>(0.0);

            // hard-edged color bands for easy reflection debugging
            if dir_y > 0.6 {
                sky_color = vec3<f32>(1.0, 0.0, 0.0); // red (top)
            } else if dir_y > 0.3 {
                sky_color = vec3<f32>(0.0, 1.0, 0.0); // green
            } else if dir_y > 0.0 {
                sky_color = vec3<f32>(0.0, 0.0, 1.0); // blue
            } else if dir_y > -0.3 {
                sky_color = vec3<f32>(1.0, 1.0, 0.0); // yellow
            } else if dir_y > -0.6 {
                sky_color = vec3<f32>(1.0, 0.0, 1.0); // magenta
            } else {
                sky_color = vec3<f32>(0.0, 1.0, 1.0); // cyan (bottom)
            }

            final_color += throughput * sky_color;
            break;
        }

        let mat = materials[hit.material_index];

        var base_color = mat.base_color_factor.rgb;
        if mat.base_color_tex_layer >= 0 {
            let bc_uv = mat.base_color_uv.xy + hit.uv * mat.base_color_uv.zw;
            let base_color_tex = textureSampleLevel(texture_atlas, atlas_sampler, bc_uv, i32(mat.base_color_tex_layer), 0.0);
            base_color = base_color_tex.rgb * mat.base_color_factor.rgb;
        }

        var metallic = mat.metallic_factor;
        var roughness = mat.roughness_factor;
        if mat.metallic_roughness_tex_layer >= 0 {
            let mr_uv = mat.metallic_roughness_uv.xy + hit.uv * mat.metallic_roughness_uv.zw;
            let mr_tex = textureSampleLevel(texture_atlas, atlas_sampler, mr_uv, i32(mat.metallic_roughness_tex_layer), 0.0);
            roughness *= mr_tex.g;
            metallic *= mr_tex.b;
        }

        var emissive_color = mat.emissive.rgb;
        if mat.emissive_tex_layer >= 0 {
            let e_uv = mat.emissive_uv.xy + hit.uv * mat.emissive_uv.zw;
            let emissive_tex = textureSampleLevel(texture_atlas, atlas_sampler, e_uv, i32(mat.emissive_tex_layer), 0.0);
            emissive_color *= emissive_tex.rgb;
        }
        final_color += throughput * (emissive_color * mat.emissive.w);

        let N_geom = hit.normal;
        var N = hit.normal;

        if mat.normal_tex_layer >= 0 {
            let n_uv = mat.normal_uv.xy + hit.uv * mat.normal_uv.zw;
            let normal_tex = textureSampleLevel(texture_atlas, atlas_sampler, n_uv, i32(mat.normal_tex_layer), 0.0);

            // re-range [0, 1] mapped texture back to [-1, 1] vector
            var local_n = normal_tex.xyz * 2.0 - 1.0;

            local_n = vec3<f32>(local_n.xy * mat.normal_scale, local_n.z);
            local_n = normalize(local_n);

            let T = hit.tangent.xyz;
            let t_ortho_unnorm = T - N * dot(N, T);
            let t_ortho_len_sq = dot(t_ortho_unnorm, t_ortho_unnorm);
            let safe_t_ortho_len_sq = max(t_ortho_len_sq, 0.0);

            let fallback_up = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(N.z) < 1.0);
            let fallback_t = normalize(cross(fallback_up, N));
            let T_ortho = select(fallback_t, t_ortho_unnorm * inverseSqrt(safe_t_ortho_len_sq), t_ortho_len_sq > 0.0);

            // bitangent
            let B = cross(N, T_ortho) * hit.tangent.w;

            let tbn = mat3x3<f32>(T_ortho, B, N);
            N = normalize(tbn * local_n);
        }

        let V = -ray.direction;
        let NdV = max(dot(N, V), 0.000000001); // TODO: a tiny value here is needed to prevent black pixels, I don't know why

        // Schlick Fresnel approximation
        let F0 = mix(vec3<f32>(0.04), base_color, metallic);
        let F = F0 + (1.0 - F0) * pow(1.0 - NdV, 5.0);

        // probability of choosing a metallic/specular bounce vs a diffuse bounce
        let p_specular = clamp((F.x + F.y + F.z) / 3.0, 0.1, 0.9);

        var next_dir: vec3<f32>;
        if rand() < p_specular {
            // metallic/specular bounce
            let alpha = max(0.0, roughness * roughness); // glTF expects alpha = roughness^2
            let H = ggx_sample_hemisphere(N, alpha);
            next_dir = reflect(-V, H);

            // invalidate if bounce heads under the surface
            if dot(next_dir, N_geom) <= 0.0 || dot(next_dir, N) <= 0.0 {
                break;
            }

            let L = next_dir;
            let NdotL = max(dot(N, L), 0.0);
            let NdotH = max(dot(N, H), 0.0);
            let VdotH = max(dot(V, H), 0.0);

            let G = smith_g1(NdV, alpha) * smith_g1(NdotL, alpha);

            let spec_weight = F * (G * VdotH) / (NdV * NdotH);

            throughput *= spec_weight / p_specular;
        } else {
            // diffuse bounce
            next_dir = cosine_sample_hemisphere(N);

            // invalidate if bounce heads under the surface
            if dot(next_dir, N_geom) <= 0.0 {
                break;
            }

            let diffuse_albedo = base_color * (1.0 - metallic);
            throughput *= diffuse_albedo / (1.0 - p_specular);
        }

        // offset ray slightly to prevent self-intersection shadowing (shadow acne)
        ray.origin = hit.p + N_geom * max(1e-4, 1e-4 * length(hit.p));
        ray.direction = next_dir;

        // russian roulette: terminate paths early if they contribute very little light
        if bounce > 2 {
            let p_survive = clamp(max(throughput.x, max(throughput.y, throughput.z)), 0.1, 0.95);
            if rand() > p_survive {
                break;
            }
            throughput /= p_survive;
        }
    }

    // blend current frame into accumulation buffer
    let pixel_idx = global_id.y * u32(screen_dims.x) + global_id.x;
    var accumulated = vec3<f32>(0.0);

    // frame_count resets to 0 when the camera moves, but is then incremented every frame no matter if the camera moves
    // so the first frame after a camera move will have frame_count of 1, and only from the second frame onwards can we be sure that the accumulation buffer has valid data to blend with
    if frame_count > 1u {
        accumulated = accumulation_buffer[pixel_idx].rgb;
    }

    accumulated += final_color;
    accumulation_buffer[pixel_idx] = vec4<f32>(accumulated, 1.0);

    // average out result
    let display_color = accumulated / f32(frame_count + 1u);
    textureStore(screen, global_id.xy, vec4<f32>(display_color, 1.0));
}