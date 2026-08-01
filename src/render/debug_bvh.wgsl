// this shader renders the BVH structure as a wireframe, which is very useful for debugging the BVH
// leaf nodes are colored red, and internal nodes are colored blue
// see the main path tracing compute shader trace_paths.wgsl for explanation on the bindings, how the BVH is traversed, how ray-AABB intersection is performed, etc.

// TODO: update this, or just put it in trace_paths, also allowing user to switch between path tracing and BVH rendering during runtime

struct GpuTriangleGeometry {
    coords: array<f32, 9>,
};
struct GpuTriangleAttribute {
    index: u32,
    normals: array<f32, 9>,
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

struct GpuMaterial {
    base_color_factor: vec4<f32>,
    base_color_uv: vec4<f32>,
    normal_uv: vec4<f32>,
    metallic_roughness_uv: vec4<f32>,
    emissive_uv: vec4<f32>,
    emissive: vec4<f32>,
    base_color_tex_layer: i32,
    normal_scale: f32,
    normal_tex_layer: i32,
    metallic_roughness_tex_layer: i32,
    emissive_tex_layer: i32,
    metallic_factor: f32,
    roughness_factor: f32,
    double_sided: i32,
};

struct GpuCamera {
    position_and_frame_count: vec4<f32>,
    lower_left_corner: vec4<f32>,
    horizontal: vec4<f32>,
    vertical: vec4<f32>,
};

@group(0) @binding(0) var screen: texture_storage_2d<rgba16float, write>;
@group(0) @binding(1) var<storage, read_write> accumulation_buffer: array<vec4<f32>>;
@group(1) @binding(0) var<storage, read> triangles_geo: array<GpuTriangleGeometry>;
@group(1) @binding(1) var<storage, read> bvh_nodes: array<BvhNode>;
@group(1) @binding(2) var<storage, read> triangles_attr: array<GpuTriangleAttribute>;
@group(1) @binding(3) var<storage, read> materials: array<GpuMaterial>;
@group(1) @binding(4) var texture_atlas: texture_2d_array<f32>;
@group(1) @binding(5) var atlas_sampler: sampler;
@group(1) @binding(6) var<uniform> camera: GpuCamera;

struct Ray {
    origin: vec3<f32>,
    direction: vec3<f32>,
};

// PCG random number generator for TAA jittering
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

@compute @workgroup_size(8, 8, 1)
fn compute_main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let screen_dims = vec2<f32>(textureDimensions(screen));

    if global_id.x >= u32(screen_dims.x) || global_id.y >= u32(screen_dims.y) {
        return;
    }

    let frame_count = u32(camera.position_and_frame_count.w);
    init_rand(global_id, frame_count);

    // subpixel jitter for anti-aliasing (TAA) continues to work perfectly!
    let jitter = vec2<f32>(rand() - 0.5, rand() - 0.5);
    let uv = (vec2<f32>(global_id.xy) + vec2<f32>(0.5) + jitter) / screen_dims;

    var ray: Ray;
    ray.origin = camera.position_and_frame_count.xyz;
    ray.direction = normalize(
        camera.lower_left_corner.xyz + uv.x * camera.horizontal.xyz + uv.y * camera.vertical.xyz - ray.origin
    );

    // Keep the background completely black
    var final_color = vec3<f32>(0.0, 0.0, 0.0);

    // Prevent possible NaN or division by zero in AABB logic
    var safe_dir = ray.direction;
    if abs(safe_dir.x) < 1e-7 { safe_dir.x = 1e-7; }
    if abs(safe_dir.y) < 1e-7 { safe_dir.y = 1e-7; }
    if abs(safe_dir.z) < 1e-7 { safe_dir.z = 1e-7; }
    let inv_dir = 1.0 / safe_dir;

    var stack: array<u32, 64>;
    var stack_ptr: u32 = 0u;

    stack[stack_ptr] = 0u;
    stack_ptr += 1u;

    // evaluate intersections with all BVH nodes
    while stack_ptr > 0u {
        stack_ptr -= 1u;
        let node_idx = stack[stack_ptr];
        let node = bvh_nodes[node_idx];

        let t0 = (node.aabb_min - ray.origin) * inv_dir;
        let t1 = (node.aabb_max - ray.origin) * inv_dir;

        let tmin = min(t0, t1);
        let tmax = max(t0, t1);

        let t_near = max(max(tmin.x, tmin.y), tmin.z);
        let t_far = min(min(tmax.x, tmax.y), tmax.z);

        if t_near <= t_far && t_far > 0.0 {
            var hit_outline = false;
            let t_arr = array<f32, 2>(t_near, t_far);
            
            for (var i = 0; i < 2; i += 1) {
                let t = t_arr[i];
                if t > 0.0 {
                    let P = ray.origin + t * ray.direction;
                    
                    let d_min = abs(P - node.aabb_min);
                    let d_max = abs(node.aabb_max - P);
                    let d = min(d_min, d_max);
                    
                    let thickness = max(t * 0.002, 0.0001);
                    
                    let cx = select(0, 1, d.x < thickness);
                    let cy = select(0, 1, d.y < thickness);
                    let cz = select(0, 1, d.z < thickness);
                    
                    if cx + cy + cz >= 2 { hit_outline = true; }
                }
            }

            if hit_outline {

                if node.prim_count > 0u {
                    final_color += vec3<f32>(0.06, 0.015, 0.015);
                } else {
                    final_color += vec3<f32>(0.01, 0.02, 0.04);
                }
            }

            if node.prim_count == 0u {
                if stack_ptr + 2u <= 64u {
                    stack[stack_ptr] = node.left_first + 1u;
                    stack_ptr += 1u;
                    stack[stack_ptr] = node.left_first;
                    stack_ptr += 1u;
                }
            }
        }
    }

    let pixel_idx = global_id.y * u32(screen_dims.x) + global_id.x;
    var accumulated = vec3<f32>(0.0);

    if frame_count > 1u {
        accumulated = accumulation_buffer[pixel_idx].rgb;
    }

    accumulated += final_color;
    accumulation_buffer[pixel_idx] = vec4<f32>(accumulated, 1.0);

    let display_color = accumulated / f32(frame_count + 1u);
    textureStore(screen, global_id.xy, vec4<f32>(display_color, 1.0));
}