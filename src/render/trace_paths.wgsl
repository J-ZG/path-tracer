// the main path tracing shader
// implements a basic path tracer with support for diffuse and metallic/specular materials, normal interpolation, emissive materials, etc.
// supports texture maps for base color, metallic/roughness, normal, emissive

// TODO: implement tracing shadow rays for better denoising, avoid fireflies, also multiple importance sampling? 
// TODO: investigate why web performance is much worse than on native
// TODO: investigate black edges on round models

// 20ms 20ms 15μs at 800x600 dorm_light for compute and render scope, compute pass scoped, and render pass scoped

const PI = 3.14159265359f;
const MAX_BOUNCES = 20u;
const ZERO_THRESHOLD = 1e-5f;
const MAX_F32 = 0x1.fffffep+127f; // max float value IEEE-754 binary32

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
    // TODO: double_sided is never actually used; currently, every material is treated as double-sided
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
    direction_reciprocal: vec3<f32>, // precomputed reciprocal of direction for faster ray-box intersection; this optimization seems to barely improves performance, but it's common
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
    var word = ((seed >> ((seed >> 28u) + 4u)) ^ seed) * 277803737u;
    return (word >> 22u) ^ word;
}
// get random f32 in [0, 1)
// even with multiple rand() calls with the same invocation_id and frame_count, each call still produces a different random value
fn rand() -> f32 {
    return f32(pcg_hash()) * bitcast<f32>(0x2f800004u); // slightly faster than dividing by 4294967296.0
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

    // convert the x and y on the texture/surface to normalized values between 0 and 1
    // + vec2<f32> with rand() enables subpixel jitter for anti-aliasing (TAA)
    let pixel_uv = (vec2<f32>(global_id.xy) + vec2<f32>(rand(), rand())) / screen_dims;

    // create a Ray with origin at camera and direction to the given image plane point
    var ray: Ray;
    ray.origin = camera.position_and_frame_count.xyz;
    // normalized vector going from camera origin to the image plane coordinate in world space
    ray.direction = normalize(
        camera.lower_left_corner.xyz + pixel_uv.x * camera.horizontal.xyz + pixel_uv.y * camera.vertical.xyz - ray.origin
    );
    ray.direction_reciprocal = 1.0 / ray.direction;

    var throughput = vec3<f32>(1.0, 1.0, 1.0);
    var final_color = vec3<f32>(0.0, 0.0, 0.0);

    // path tracing loop
    for (var bounce = 0u; bounce < MAX_BOUNCES; bounce += 1u) {
        var closest_t = MAX_F32;
        var hit_index = -1;
        var hit_v = 0.0;
        var hit_w = 0.0;

        // watertight ray setup (hoisted out of BVH loop)

        // find the dominant axis of the ray direction
        let abs_d = abs(ray.direction);
        let max_xy = max(abs_d.x, abs_d.y);
        let kz = select(
            select(0u, 1u, abs_d.y > abs_d.x),
            2u,
            abs_d.z > max_xy
        );

        // compute permutation indices to preserve winding order
        let next_dim = array<u32, 3>(1u, 2u, 0u);
        let prev_dim = array<u32, 3>(2u, 0u, 1u);
        let kx_val = next_dim[kz];
        let ky_val = prev_dim[kz];
        let is_neg = ray.direction[kz] < 0.0;

        let kx = select(kx_val, ky_val, is_neg);
        let ky = select(ky_val, kx_val, is_neg);

        // calculate shear constants
        let Sz = 1.0 / ray.direction[kz];
        let Sx = -ray.direction[kx] * Sz;
        let Sy = -ray.direction[ky] * Sz;

        // fixed-size stack for BVH traversal (depth 64 is sufficient for millions of primitives)
        var stack: array<u32, 64>;
        var stack_ptr = 0u;

        // push root node
        stack[stack_ptr] = 0u;
        stack_ptr += 1u;

        // BVH traversal
        while stack_ptr > 0u {
            // pop node
            stack_ptr -= 1u;
            let node = bvh_nodes[stack[stack_ptr]];

            // ray-AABB intersection (slab method)

            let t0 = (node.aabb_min - ray.origin) * ray.direction_reciprocal;
            let t1 = (node.aabb_max - ray.origin) * ray.direction_reciprocal;

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
                    let tri_index = node.left_first + i;
                    let tri = triangles_geo[tri_index];

                    let p0 = vec3<f32>(tri.coords[0], tri.coords[1], tri.coords[2]);
                    let p1 = vec3<f32>(tri.coords[3], tri.coords[4], tri.coords[5]);
                    let p2 = vec3<f32>(tri.coords[6], tri.coords[7], tri.coords[8]);

                    // watertight ray-triangle intersection using the method from "Watertight Ray/Triangle Intersection" by Woop, Benthin, and Wald (2013)
                    // more robust than Möller–Trumbore, and also slightly faster
                    // see https://jcgt.org/published/0002/01/05/paper.pdf

                    // calculate vertices relative to the ray origin
                    let A = p0 - ray.origin;
                    let B = p1 - ray.origin;
                    let C = p2 - ray.origin;

                    // perform shear and scale of vertices
                    let Ax = A[kx] + Sx * A[kz]; let Ay = A[ky] + Sy * A[kz];
                    let Bx = B[kx] + Sx * B[kz]; let By = B[ky] + Sy * B[kz];
                    let Cx = C[kx] + Sx * C[kz]; let Cy = C[ky] + Sy * C[kz];

                    // calculate scaled barycentric coordinates
                    let U = Cx * By - Cy * Bx;
                    let V = Ax * Cy - Ay * Cx;
                    let W = Bx * Ay - By * Ax;

                    // if the signs of U, V, and W are mixed, the ray misses the triangle
                    let min_UVW = min(U, min(V, W));
                    let max_UVW = max(U, max(V, W));
                    if min_UVW < 0.0 && max_UVW > 0.0 {
                        continue;
                    }

                    // calculate determinant
                    let det = U + V + W;
                    if det == 0.0 {
                        continue;
                    }

                    // calculate scaled Z coordinates and the hit distance t
                    let Az = Sz * A[kz];
                    let Bz = Sz * B[kz];
                    let Cz = Sz * C[kz];
                    let inv_det = 1.0 / det;
                    let t = (U * Az + V * Bz + W * Cz) * inv_det;

                    if t > 0.0 && t < closest_t {
                        closest_t = t;
                        hit_index = i32(tri_index);

                        hit_v = V * inv_det;
                        hit_w = W * inv_det;
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

        // ray escaped the scene, render sky color and terminate
        if hit_index == -1 {
            // normal sky gradient
            let sky_color = mix(vec3<f32>(0.05, 0.05, 0.05), vec3<f32>(0.2, 0.3, 0.4), 0.5 * (ray.direction.y + 1.0));

            // hard-edged color bands for easy reflection debugging
            // let dir_y = ray.direction.y;
            // var sky_color = vec3<f32>(0.0);
            // if dir_y > 0.6 {
            //     sky_color = vec3<f32>(1.0, 0.0, 0.0); // red (top)
            // } else if dir_y > 0.3 {
            //     sky_color = vec3<f32>(0.0, 1.0, 0.0); // green
            // } else if dir_y > 0.0 {
            //     sky_color = vec3<f32>(0.0, 0.0, 1.0); // blue
            // } else if dir_y > -0.3 {
            //     sky_color = vec3<f32>(1.0, 1.0, 0.0); // yellow
            // } else if dir_y > -0.6 {
            //     sky_color = vec3<f32>(1.0, 0.0, 1.0); // magenta
            // } else {
            //     sky_color = vec3<f32>(0.0, 1.0, 1.0); // cyan (bottom)
            // }

            final_color += throughput * sky_color;
            break; // terminate this ray; this pixel's final color is now determined
        }

        let attr = triangles_attr[u32(hit_index)];
        let mat = materials[attr.index];

        // only 2 barycentric coordinates are given, but we know that the point is valid inside the triangle so all coordinates must sum to 1
        // so, the third barycentric coordinate can just be computed as 1 - (v + w)
        let hit_u = 1.0 - hit_v - hit_w;
        let interpolated_uv = hit_u * attr.uv0 + hit_v * attr.uv1 + hit_w * attr.uv2;

        var base_color = mat.base_color_factor.rgb; // TODO: rgba textures are not supported yet
        if mat.base_color_tex_layer >= 0 {
            let base_color_tex = textureSampleLevel(
                texture_atlas,
                atlas_sampler,
                mat.base_color_uv.xy + interpolated_uv * mat.base_color_uv.zw,
                mat.base_color_tex_layer,
                0.0
            );
            base_color = base_color_tex.rgb * mat.base_color_factor.rgb;
        }

        var metallic = mat.metallic_factor;
        var roughness = mat.roughness_factor;
        if mat.metallic_roughness_tex_layer >= 0 {
            let mr_tex = textureSampleLevel(
                texture_atlas,
                atlas_sampler,
                mat.metallic_roughness_uv.xy + interpolated_uv * mat.metallic_roughness_uv.zw,
                mat.metallic_roughness_tex_layer,
                0.0
            );
            roughness *= mr_tex.g;
            metallic *= mr_tex.b;
        }

        var emissive_color = mat.emissive.rgb;
        if mat.emissive_tex_layer >= 0 {
            let emissive_tex = textureSampleLevel(
                texture_atlas,
                atlas_sampler,
                mat.emissive_uv.xy + interpolated_uv * mat.emissive_uv.zw,
                mat.emissive_tex_layer,
                0.0
            );
            emissive_color *= emissive_tex.rgb;
        }
        final_color += throughput * (emissive_color * mat.emissive.w);

        let n0 = vec3<f32>(attr.normals[0], attr.normals[1], attr.normals[2]);
        let n1 = vec3<f32>(attr.normals[3], attr.normals[4], attr.normals[5]);
        let n2 = vec3<f32>(attr.normals[6], attr.normals[7], attr.normals[8]);
        let interpolated_n = normalize(hit_u * n0 + hit_v * n1 + hit_w * n2);

        let is_back_face = dot(ray.direction, interpolated_n) > 0.0; // false if the ray hits the front face
        let hit_n = select(interpolated_n, -interpolated_n, is_back_face);
        var N = hit_n;

        // TODO: tangents can be zero if given tangents' w don't match for the same triangle; not sure what that does to the shader code

        let interpolated_tangent = vec4<f32>(normalize(hit_u * attr.t0.xyz + hit_v * attr.t1.xyz + hit_w * attr.t2.xyz), attr.t0.w);
        let hit_tangent = select(interpolated_tangent, -interpolated_tangent, is_back_face);

        // Gram-Schmidt orthogonalization
        let T = normalize(hit_tangent.xyz - N * dot(N, hit_tangent.xyz));

        let B = normalize(cross(N, T) * hit_tangent.w); // bitangent according to glTF

        if mat.normal_tex_layer >= 0 {
            let normal_tex = textureSampleLevel(
                texture_atlas,
                atlas_sampler,
                mat.normal_uv.xy + interpolated_uv * mat.normal_uv.zw,
                mat.normal_tex_layer,
                0.0
            );

            // re-range from [0, 1] RGB to [-1, 1] vector space
            // https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#_material_normaltextureinfo_scale
            var local_n = normal_tex.rgb * 2.0 - 1.0; // in WGSL, vec - scalar works component-wise as intended; see https://www.w3.org/TR/WGSL/#arithmetic-expr

            local_n = normalize(vec3<f32>(local_n.xy * mat.normal_scale, local_n.z));

            let TBN = mat3x3<f32>(T, B, N);
            N = normalize(TBN * local_n);
        }

        let V = -ray.direction; // view vector

        let NdotV = max(dot(N, V), ZERO_THRESHOLD);

        // reflection color at normal incidence; for dielectrics, this is usually around 4%, while for metals, this is the base color
        let F0 = mix(vec3<f32>(0.04), base_color, metallic);
        // estimate Fresnel at view angle to probabilistically decide diffuse or specular bounce
        let F_view = F0 + (1.0 - F0) * pow(1.0 - NdotV, 5.0);
        // probability clamped between 10% and 90%
        let p_specular = clamp(max(F_view.x, max(F_view.y, F_view.z)), 0.1, 0.9);

        let rand1 = rand();
        let rand2 = rand();

        var next_ray_dir: vec3<f32>;

        if rand() < p_specular {
            // metallic/specular bounce

            let alpha = roughness * roughness; // glTF expects alpha = roughness^2
            let theta = atan(alpha * sqrt(rand1) / sqrt(1.0 - rand1));
            let phi = 2.0 * PI * rand2;
            let H_ts = vec3<f32>(sin(theta) * cos(phi), sin(theta) * sin(phi), cos(theta));
            let H = normalize(T * H_ts.x + B * H_ts.y + N * H_ts.z);

            let L = reflect(-V, H);
            next_ray_dir = L;

            let NdotL = max(dot(N, L), 0.0);

            if NdotL > 0.0 && dot(hit_n, L) > 0.0 {
                let VdotH = max(dot(V, H), 0.0);

                // how much light is reflected at this specific micro-angle
                let F = F0 + (1.0 - F0) * pow(1.0 - VdotH, 5.0);

                // Schlick-GGX model with Smith approximation
                let k = alpha / 2.0;
                let G1_V = NdotV / (NdotV * (1.0 - k) + k);
                let G1_L = NdotL / (NdotL * (1.0 - k) + k);
                let G = G1_V * G1_L;

                // Cook-Torrance specular and PDF
                let spec_weight = (F * G * VdotH) / max(NdotV * max(dot(N, H), 0.0), ZERO_THRESHOLD);

                // divide by the probability to conserve energy
                throughput *= spec_weight / p_specular;
            } else {
                throughput = vec3<f32>(0.0);
            }
        } else {
            // diffuse bounce

            let r = sqrt(rand1);
            let phi = 2.0 * PI * rand2;
            let L_ts = vec3<f32>(r * cos(phi), r * sin(phi), sqrt(1.0 - rand1));

            // transform light direction into world space
            let L = normalize(T * L_ts.x + B * L_ts.y + N * L_ts.z);
            next_ray_dir = L;

            if dot(hit_n, L) > 0.0 {
                // half vector
                let H = normalize(V + L);

                let F = F0 + (1.0 - F0) * pow(1.0 - max(dot(V, H), 0.0), 5.0);

                let diff_weight = base_color * (1.0 - metallic) * (1.0 - F);

                throughput *= diff_weight / (1.0 - p_specular);
            } else {
                throughput = vec3<f32>(0.0);
            }
        }

        // russian roulette: terminate paths early if they contribute very little light
        if bounce > 2u { // don't start terminating until after a few bounces to allow light to bounce around and contribute meaningfully
            let p_survive = clamp(max(throughput.x, max(throughput.y, throughput.z)), 0.05, 0.95);
            if rand() > p_survive {
                break;
            }
            throughput /= p_survive;
        }

        // this matches f(x) = m * x + b, but in this case in 3D, f(x) outputs a vec3 point, which is the intersection point
        let hit_p = ray.direction * closest_t + ray.origin;

        // offset ray origin slightly to prevent self-intersection shadowing (shadow acne)
        ray.origin = hit_p + N * max(1e-4, 1e-4 * length(hit_p));
        ray.direction = next_ray_dir;
        ray.direction_reciprocal = 1.0 / ray.direction;
    }

    // blend current frame into accumulation buffer
    let pixel_idx = global_id.y * u32(screen_dims.x) + global_id.x;
    var accumulated = vec3<f32>(0.0);

    // frame_count resets to 0 when the camera moves, but the increment happens after the reset, so frame count can never be zero in the shader
    // only from the second frame onward can we be sure that the accumulation buffer has valid data to blend with
    if frame_count > 1u {
        accumulated = accumulation_buffer[pixel_idx].rgb;
    }

    accumulated += final_color;
    accumulation_buffer[pixel_idx] = vec4<f32>(accumulated, 1.0);

    // average out result
    textureStore(screen, global_id.xy, vec4<f32>(accumulated / f32(frame_count), 1.0)); // frame count is always at least 1
}