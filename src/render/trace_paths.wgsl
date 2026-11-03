// the main path tracing shader

const PI = 3.14159265359;
const INV_PI = 0.31830988618;
const INV_2PI = 0.15915494309189535;

const LUMA_COEFFICIENTS = vec3f(0.2126, 0.7152, 0.0722); // standard Rec. 709 coefficients to convert RGB -> luminance value

// this path tracer works in photometric units
// note that radiometric and photometric units follow the same mathematical laws, meaning this path tracer can operate directly in photometric units while using the radiometric lighting equations
// many RGB values like final_color, sky_color, and values in accumulation_buffer are luminance values, each channel measured in nits,
// which is the unit OpenPBR specifies for emission_luminance
// since nits = cd/m^2, this also implies that world position coordinate unit in this path tracer is actually meters

// change EXPOSURE below to make worlds look properly exposed
// don't change, for example, sky_color (it's physically accurate for a clear sky to be 8000 nits)
// in other words: don't cheat the lighting; only change the camera

// Sunny 16: aperture 16, shutter_speed 0.008 (1/125th of a second), iso 100
// then, eq.67 from https://seblagarde.wordpress.com/wp-content/uploads/2015/07/course_notes_moving_frostbite_to_pbr_v32.pdf:
// EV100 = log2((aperture^2 * 100) / (shutter_speed * iso)) ≈ 14.9657842847
// max_luminance = 1.2 * 2^(EV100) = 38400
// EXPOSURE = 1 / max_luminance ≈ 0.00002604166
// also, EXPOSURE = 0.001 can come from approximately aperture 4, shutter speed 1/50, iso 100; typical for indoor worlds
const EXPOSURE = 0.00002604166;
// TODO: implement and allow the user to adjust (can be stored in Camera struct):
// - aperture: enables depth of field
// - shutter speed: needed for calculating EXPOSURE assuming ISO is 100, also implement auto exposure
// - focus distance: also implement auto focus
// - sensor width: must be different from window size
// - focal length: together with sensor width determines FOV (get rid of just the FOV slider)
// - optionally aperture blades: defines the look of bokeh

// TODO: implement tracing shadow rays for better denoising (NEE), then combine everything with MIS; eventually, look into ReSTIR or something
// TODO: look into possibly making a branched path tracer
// TODO: look into spectral rendering

// TODO: mirror materials with normals have mirror reflections that aren't smooth; can see triangle edges

// to go from the debug YAML to Blender: negate Z, then swap Z and Y

// these structs match with Rust side definitions (struct names have Gpu- prefix removed)
// see Rust code for more detailed documentation

// TriangleGeometry and TriangleAttribute are separate structs just because the shader fetches vertices much more frequently than normals
// although, actual benchmarks show that the performance difference is negligible, so they could be combined into one struct, but would require more padding, so they're kept separate for now
struct TriangleGeometry {
    vertices: array<f32, 9>, // 3 vertices with x, y, z for each vertex
    // stored as a flat array since vec3<T> has alignment of 16 bytes, which misaligns with Rust's glam::Vec3 which has alignment of 12 bytes
    // using a flat array allows packing the vertex data without padding
    // the extra code like
    // v0 = vec3f(tri.vertices[0], tri.vertices[1], tri.vertices[2])
    // should be optimized by the compiler and not cause a performance difference
    // benchmarks show that using a flat array vs vec3 doesn't cause a performance difference
}
struct TriangleAttribute {
    index: u32, // index into materials for the Material this triangle uses
    normals: array<f32, 9>, // same packing logic as TriangleGeometry for the normals of the 3 vertices
    uv0: vec2f,
    uv1: vec2f,
    uv2: vec2f,
    t0: vec4f,
    t1: vec4f,
    t2: vec4f,
}

struct BvhNode {
    aabb_min: vec3f,
    left_first: u32,
    aabb_max: vec3f,
    tri_count: u32,
}

// field names here follow OpenPBR
struct Material {
    base_color: vec4f, // OpenPBR uses only RGB, not A
    base_weight: f32,
    base_metalness: f32,
    base_diffuse_roughness: f32,
    emission_weight: f32,
    emission_color_and_luminance: vec4f, // glTF restricts RGB to [0, 1] while OpenPBR allows [0, ∞)
    specular_color_and_weight: vec4f, // OpenPBR specifies specular color in [0, 1], but the RGB values stored here may exceed 1
    specular_roughness: f32,
    specular_roughness_anisotropy: f32,
    specular_haze: f32,
    specular_haze_spread: f32,
    specular_ior: f32, // glTF restricts to [1, ∞) while OpenPBR allows (0, ∞); the shading code supports this value being < 1
    specular_retroreflectivity: f32,
    normal_scale: f32,
    pad: array<f32, 3>,
    base_color_tex_layer: i32,
    normal_tex_layer: i32,
    metallic_roughness_tex_layer: i32,
    emission_tex_layer: i32,
    specular_color_tex_layer: i32,
    specular_weight_tex_layer: i32,
    base_color_uv: vec4f,
    normal_uv: vec4f,
    metallic_roughness_uv: vec4f,
    emission_uv: vec4f,
    specular_color_uv: vec4f,
    specular_weight_uv: vec4f,
}

struct Camera {
    position: vec4f,
    lower_left_corner_minus_position: vec4f,
    horizontal: vec4f,
    vertical: vec4f,
}

struct SceneUniforms {
    total_luminous_flux: f32,
    emissive_triangles_count: u32,
}

struct SceneAliasEntry {
    threshold: f32,
    alias_index: u32,
    triangle_index: u32,
    selection_probability: f32,
}

struct SkyboxUniforms {
    total_luminous_flux: f32,
    average_luminance: f32,
    width: u32,
    height: u32,
}

struct SkyboxAliasEntry {
    threshold: f32,
    alias_index: u32,
    selection_probability: f32,
}

@group(0) @binding(0) var screen: texture_storage_2d<rgba32float, write>;
@group(0) @binding(1) var<storage, read_write> accumulation_buffer: array<vec4f>; // this is actually used as a texture, but since textures cannot be both read and write, a storage buffer is used instead

@group(1) @binding(0) var<storage, read> geometries: array<TriangleGeometry>;
@group(1) @binding(1) var<storage, read> bvh_nodes: array<BvhNode>;
@group(1) @binding(2) var<storage, read> attributes: array<TriangleAttribute>;
@group(1) @binding(3) var<storage, read> materials: array<Material>;
@group(1) @binding(4) var srgb_texture_atlas: texture_2d_array<f32>;
@group(1) @binding(5) var linear_texture_atlas: texture_2d_array<f32>;
@group(1) @binding(6) var atlas_sampler: sampler;
@group(1) @binding(7) var<uniform> camera: Camera;
@group(1) @binding(8) var<uniform> frame_count: u32;
@group(1) @binding(9) var skybox_texture: texture_2d<f32>;
@group(1) @binding(10) var<uniform> skybox: SkyboxUniforms;
@group(1) @binding(11) var skybox_sampler: sampler;
@group(1) @binding(12) var<uniform> scene: SceneUniforms;
@group(1) @binding(13) var<storage, read> scene_aliases: array<SceneAliasEntry>;
@group(1) @binding(14) var<storage, read> skybox_aliases: array<SkyboxAliasEntry>;

// PCG random number generator
// https://www.pcg-random.org/index.html
// TODO: low-discrepancy sequences and blue noise are better
var<private> rng_state = 0u;
// returns random f32 ∈ [0, 1)
// even with multiple rand() calls after the same seed_rand, each call still produces a different random value
fn rand() -> f32 {
    rng_state = rng_state * 747796405u + 2891336453u;
    let word = ((rng_state >> ((rng_state >> 28u) + 4u)) ^ rng_state) * 277803737u;
    return bitcast<f32>(0x3f800000u | (((word >> 22u) ^ word) >> 9u)) - 1.0;
}

fn fifth_power(x: f32) -> f32 {
    return x * x * x * x * x;
}

// sample all lobes
struct SampleLobesResult {
    f_total: vec3f,
    pdf_total: f32,
}
fn sample_lobes(
    wo: vec3f, wi: vec3f,
    p_ggx: f32, p_diffuse: f32,
    base_color: vec3f, base_weight: f32, base_metalness: f32, base_diffuse_roughness: f32,
    specular_color: vec3f, specular_weight: f32,
    specular_ior_modulated: f32,
    alpha: vec2f,
    one_plus_lambda_wo: f32,
    F0_conductor: vec3f,
    E_dielectric: vec3f,
    ltc_coefficients: vec4f, orthonormal_basis_ltc_wo: mat3x3f, P_u: f32, P_c: f32,
) -> SampleLobesResult {
    // half vector
    let H = normalize(wo + wi);
    let HdotW = dot(H, wo); // doesn't matter if it's dot(H, wi) or dot(H, wo) since they're equivalent

    // https://www.pbr-book.org/4ed/Reflection_Models/Roughness_Using_Microfacet_Theory#eq:microfacet-lambda
    let G1 = 1.0 / one_plus_lambda_wo;

    // https://www.pbr-book.org/4ed/Reflection_Models/Roughness_Using_Microfacet_Theory#eq:microfacet-masking-shadowing-ross
    // height-corrected
    let G2 = 1.0 / (one_plus_lambda_wo + ggx_lambda(wi, alpha));

    // GGX (Trowbridge-Reitz) normal distribution function, generalized to anisotropic
    // https://www.pbr-book.org/4ed/Reflection_Models/Roughness_Using_Microfacet_Theory#eq:tr-d-function
    // equivalent to OpenPBR eq.20 and Dupuy eq.12
    // this form can be derived from these relationships:
    //     H.x = sin(theta_m)cos(phi_m)
    //     H.y = sin(theta_m)sin(phi_m)
    //     H.z = cos(theta_m)
    let D_helper = (H.x * H.x) / (alpha.x * alpha.x) + (H.y * H.y) / (alpha.y * alpha.y) + H.z * H.z;
    let D = INV_PI / (alpha.x * alpha.y * D_helper * D_helper);

    // https://pbr-book.org/4ed/Reflection_Models/Roughness_Using_Microfacet_Theory#eq:torrance-sparrow
    let torrance_sparrow_without_fresnel = D * G2 / (4.0 * wi.z * wo.z);

    // metal: Torrance-Sparrow, and Fresnel factor is F82-tint per OpenPBR Metallic Base section
    // originally described in "Novel aspects of the Adobe Standard Material" by Peter Kutz, Milos Hasan, and Paul Edmondson (2021)
    // https://helpx.adobe.com/content/dam/substance-3d-ecosystem-and-plugins/Adobe%20Standard%20Material%20-%20Technical%20Documentation.pdf
    // mathematically equivalent to OpenPBR spec
    let one_minus_HdotW = 1.0 - HdotW;
    let F_82 = F0_conductor - fifth_power(one_minus_HdotW) * fma(
        0.00105024005 * HdotW * one_minus_HdotW * (1.0 - specular_color), // 0.00105024005 ≈ 7^2 / 6^6
        9031.0 * F0_conductor + 7776.0,
        F0_conductor - 1.0
    );
    let f_conductor = saturate(specular_weight * F_82) * torrance_sparrow_without_fresnel;

    // gloss: Torrance-Sparrow, and Fresnel factor with modulated IOR
    let f_dielectric = specular_color * fresnel_exact(specular_ior_modulated, HdotW) * torrance_sparrow_without_fresnel;

    // diffuse: EON
    // Portsmouth Listing 1
    let rho = base_color;
    let s_f = dot(wi.xy, wo.xy);
    let sovertF = select(s_f, s_f / max(wo.z, wi.z), s_f > 0.0);
    let AF = 1.0 / fma(0.28779340921, base_diffuse_roughness, 1.0); // 0.28779340921 ≈ 0.5 - 2 / (3 * PI)
    let f_ss = rho * INV_PI * AF * fma(base_diffuse_roughness, sovertF, 1.0);
    let EFo = E_FON_exact(wo.z, base_diffuse_roughness);
    let EFi = E_FON_exact(wi.z, base_diffuse_roughness);
    let avgEF = AF * fma(0.07248821245, base_diffuse_roughness, 1.0); // 0.07248821245 ≈ 2 / 3 - 28 / (15 * PI)
    let rho_ms = rho * rho * avgEF / (1.0 - rho * (1.0 - avgEF));
    let f_ms = (rho_ms * INV_PI) * max(1.0e-7f, 1.0 - EFo)
                                            * max(1.0e-7f, 1.0 - EFi)
                                            / max(1.0e-7f, 1.0 - avgEF);
    let f_diffuse = base_weight * (f_ss + f_ms);

    // OpenPBR eq.47; non-reciprocal formulation
    let f_glossy_diffuse = f_dielectric + (1.0 - E_dielectric) * f_diffuse; // TODO: could E_dielectric exceed 1?

    let f_total = mix(f_glossy_diffuse, f_conductor, base_metalness);

    let pdf_ggx = D * G1 / (4.0 * wo.z); // Dupuy eq.20

    // Portsmouth Listing 4 pdf_EON
    let wi_LTC = transpose(orthonormal_basis_ltc_wo) * wi;
    let a = ltc_coefficients.x;
    let b = ltc_coefficients.y;
    let c = ltc_coefficients.z;
    let d = ltc_coefficients.w;
    let detM = c * fma(-b, d, a);
    let wh = vec3f(c * (wi_LTC.x - b * wi_LTC.z), (a - b * d) * wi_LTC.y, -c * (d * wi_LTC.x - a * wi_LTC.z));
    let lenSqr = dot(wh, wh);
    let s_pdf = fma(0.5, inverseSqrt(fma(d, d, 1.0)), 0.5);
    let pdf_c = (detM * detM) / (lenSqr * lenSqr) * max(wh.z, 0.0) / (PI * s_pdf);
    let pdf_eon = P_u * 0.15915494309 + P_c * pdf_c; // 0.15915494309 ≈ 1 / (2 * PI), is pdf_u

    let pdf_total = p_ggx * pdf_ggx + p_diffuse * pdf_eon;

    return SampleLobesResult(f_total, pdf_total);
}

// helper function for EON
fn E_FON_exact(mu: f32, r: f32) -> f32 {
    let AF = 1.0 / fma(0.28779340921, r, 1.0); // 0.28779340921 ≈ 0.5 - 2 / (3 * PI)
    let Si = sqrt(fma(-mu, mu, 1.0));
    let G = Si * (acos(mu) - Si * mu) + 0x1.555556p-1 * ((Si / mu) * (1.0 - (Si * Si * Si)) - Si); // the hexadecimal ≈ 2/3
    return AF + r * AF * INV_PI * G;
}

// exact Fresnel
// https://en.wikipedia.org/wiki/Fresnel_equations#Power_(intensity)_reflection_and_transmission_coefficients
// to approximate exact Fresnel to improve speed, use improved Schlick's approximation: https://www.photometric.io/blog/improving-schlicks-approximation/
// requires that ior ∈ (0, ∞) and cos_i ∈ [0, 1]
// returns value ∈ (0, 1] where return value of exactly 1.0 happens only when ior < sin_i (TIR is occurring)
fn fresnel_exact(ior: f32, cos_i: f32) -> f32 { // ior is n_2/n_1, where n_1 is often assumed to be 1.0
    let sin_t_div_ior_squared = fma(-cos_i, cos_i, 1.0) / (ior * ior); // cannot be division by zero since ior > 0
    if sin_t_div_ior_squared >= 1.0 { // checked here since sqrt for cos_t would be negative if this wasn't checked
        // physically, this means TIR is occurring
        return 1.0;
    } else {
        let cos_t = sqrt(1.0 - sin_t_div_ior_squared);
        let Rs = fma(-ior, cos_t, cos_i) / fma(ior, cos_t, cos_i);
        let Rp = fma(-ior, cos_i, cos_t) / fma(ior, cos_i, cos_t);
        return 0.5 * (Rs * Rs + Rp * Rp);
    }
}

// https://www.pbr-book.org/4ed/Reflection_Models/Roughness_Using_Microfacet_Theory#eq:microfacet-tr-lambda
// generalized to anisotropic
// requires only that v.z != 0
// return value ∈ [0, ∞)
fn ggx_lambda(v: vec3f, a: vec2f) -> f32 {
    return fma(
        0.5,
        sqrt(1.0 + (a.x * a.x * v.x * v.x + a.y * a.y * v.y * v.y) / (v.z * v.z)),
        -0.5
    );
}

// VNDF sampling
// a function here because this is used both when VNDF is chosen for calculating ray "incoming" direction and when calculating energy for layering gloss and diffuse
fn sample_VNDF(wo: vec3f, a: vec2f, rand: f32, two_pi_rand: f32) -> vec3f {
    let wo_warped = normalize(vec3f(wo.xy * a, wo.z));
    let z = fma(1.0 - rand, 1.0 + wo_warped.z, -wo_warped.z); // 1.0 - rand flips the bounds to (0, 1]
    let sin_theta = sqrt(saturate(fma(-z, z, 1.0)));
    let x = sin_theta * cos(two_pi_rand);
    let y = sin_theta * sin(two_pi_rand);
    let h = vec3f(x, y, z) + wo_warped;
    return normalize(vec3f(h.xy * a, h.z));
}

// OpenPBR eq.21
// convert user-given parameters to alpha values (anisotropic)
// returns alpha tangent and alpha bitangent, both > 0
fn alpha_anisotropic(specular_roughness: f32, specular_roughness_anisotropy: f32) -> vec2f {
    let isotropy = 1.0 - specular_roughness_anisotropy;
    let alpha_t = specular_roughness * specular_roughness * 1.41421356237 * inverseSqrt(fma(isotropy, isotropy, 1.0)); // 1.41421356237 ≈ sqrt(2)
    // isotropy and alpha_t can both be zero since specular_roughness can be 0.0 and specular_roughness_anisotropy can be 1.0;
    // the D term in Torrance-Sparrow has division by (stuff * alpha), so alpha cannot be zero 
    return max(vec2f(alpha_t, alpha_t * isotropy), vec2f(1e-4)); // 1e-4 is so close to 0.0, visually it's hard to tell a difference
}

// OpenPBR eq.34 and eq.36
// calculate modulated specular IOR ∈ (0, ∞), used for Fresnel calculations for dielectric base (all of base except for metal, which uses F82-tint model instead)
fn specular_ior_modulated(specular_ior: f32, specular_weight: f32) -> f32 {
    // we assume that ambient_ior = 1.0

    let num = specular_ior - 1.0;
    let den = specular_ior + 1.0;
    let F0_dielectric = num * num / (den * den); // this must be positive mathematically
    let F0_dielectric_sqrt = sign(specular_ior - 1.0) * sqrt(min(specular_weight * F0_dielectric, bitcast<f32>(0x3f800000u - 1u)));
    // F0_dielectric_sqrt is in (-1, 1)
    // bitcast<f32>(0x3f800000u - 1u) is the largest float smaller than 1.0, which seems to work here
    // used here because there's division by (1.0 - F0_dielectric_sqrt) in the next line, so F0_dielectric_sqrt can't be 1.0 exactly
    // 0x3F7FFFFF ≠ bitcast<f32>(0x3F7FFFFFu) = bitcast<f32>(0x3f800000u - 1u), by the way
    return (1.0 + F0_dielectric_sqrt) / (1.0 - F0_dielectric_sqrt);
}

// returns, for gloss, with a given light direction (wo), the percentage of light that reflects in all directions
// 0% means the gloss surface absorbs all light, and 100% means it reflects all light (in whatever directions)
// estimated on the fly (since we don't have precomputed LUTs)
fn estimate_e_dielectric(wo: vec3f, one_plus_lambda_wo: f32, alpha: vec2f, specular_ior_modulated: f32, specular_color: vec3f) -> vec3f {
    var sum_W = 0.0;
    let num_samples = 16u;
    for (var i = 0u; i < num_samples; i += 1u) {
        let rand1 = rand();
        let two_pi_rand = 2.0 * PI * rand();
        let H = sample_VNDF(wo, alpha, rand1, two_pi_rand);
        let wi = reflect(-wo, H);
        if wi.z <= 0.0 { continue; } // TODO: bright fireflies when specular weight is 0.0, specular roughness is close to zero, and there are normals that aren't flat. diffuse roughness, base weight, base metalness, specular IOR, and specular anisotropy don't seem to matter very much
        let F = fresnel_exact(specular_ior_modulated, dot(H, wo));
        sum_W += F * one_plus_lambda_wo / (one_plus_lambda_wo + ggx_lambda(wi, alpha)); // F * G2 / G1
    }
    return specular_color * sum_W / f32(num_samples);
}

// ray-aabb intersection during BVH traversal
// returns the distance from ray_origin to the intersection (0.0 if already inside aabb), or -1.0 if missed
// slab method
fn intersect_aabb(node: BvhNode, neg_ray_origin_div_direction: vec3<f32>, ray_direction_reciprocal: vec3<f32>, hit_t: f32) -> f32 {
    let t0 = fma(node.aabb_min, ray_direction_reciprocal, neg_ray_origin_div_direction);
    let t1 = fma(node.aabb_max, ray_direction_reciprocal, neg_ray_origin_div_direction);

    let tmin = min(t0, t1);
    let tmax = max(t0, t1);

    let t_near = max(max(tmin.x, tmin.y), tmin.z);
    let t_far = min(min(tmax.x, tmax.y), tmax.z);

    // if the ray hits the aabb and it's closer than hit_t, return max(0.0, t_near), which ensures that if the camera is inside the AABB, returns distance 0 instead of a negative value
    return select(-1.0, max(0.0, t_near), t_near <= t_far && t_far > 0.0 && t_near < hit_t);
}

@compute @workgroup_size(8, 8, 1)
fn compute_main(@builtin(global_invocation_id) global_id: vec3u) {
    // each global_id comes from compute_pass.dispatch_workgroups() in Rust
    // each global_id.x and global_id.y should yield a pixel on the texture/surface
    // hence this compute_main is ran once for each pixel each frame

    let screen_dims = vec2f(textureDimensions(screen));

    if global_id.x >= u32(screen_dims.x) || global_id.y >= u32(screen_dims.y) {
        return;
    }

    // seed the random number generator
    rng_state = (global_id.x * 0x9E3779B9u)
      ^ (global_id.y * 0x85EBCA6Bu)
      ^ (global_id.z * 0xC2B2AE35u)
      ^ (frame_count * 0x27D4EB2Du)
      ^ 0xA511E9B3u;
    rng_state |= 1u;

    // convert the x and y on the texture/surface to normalized values between 0 and 1
    // + vec2f with rand() enables subpixel jitter for anti-aliasing (TAA)
    let pixel_uv = (vec2f(global_id.xy) + vec2f(rand(), rand())) / screen_dims;

    // create a ray with origin at camera and direction to the given image plane point
    var ray_origin = camera.position.xyz;
    // normalized vector going from camera origin to the image plane coordinate in world space
    var ray_direction = normalize(
        camera.lower_left_corner_minus_position.xyz + pixel_uv.x * camera.horizontal.xyz + pixel_uv.y * camera.vertical.xyz
    );
    var ray_direction_reciprocal = 1.0 / ray_direction; // division by zero can happen, but is rare

    var throughput = vec3f(1.0, 1.0, 1.0);
    var final_color = vec3f(0.0, 0.0, 0.0);

    // path tracing loop
    for (var bounce = 0u; bounce < 10u; bounce += 1u) { // russian roulette is implemented, so this can be large
        var hit_t = 0x1.fffffep+127f; // max float value IEEE-754 binary32; how far along the ray the intersection with a triangle is
        // specifically, we use f(x) = m * x + b to find the exact coordinates of the intersection, where m is the ray direction, x is this hit_t, and b is the ray origin
        var hit_index = -1; // if the ray intersects with a triangle, this is that triangle's index in geometries
        var hit_v = 0.0; // barycentric coordinates of the ray-triangle intersection
        var hit_w = 0.0;

        // watertight ray-triangle intersection from "Watertight Ray/Triangle Intersection" by Woop, Benthin, and Wald (2013)
        // more robust than Möller-Trumbore and also slightly faster
        // https://jcgt.org/published/0002/01/05/paper.pdf
        // intersection test setup (hoisted out of BVH loop)

        let abs_d = abs(ray_direction);
        let kxy = select(1, 0, abs_d.x > abs_d.y);
        let kz = select(2, kxy, abs_d[kxy] > abs_d.z);

        let kx_val = select(kz + 1, 0, kz == 2);
        let ky_val = select(kx_val + 1, 0, kx_val == 2);

        let is_neg = ray_direction[kz] < 0.0;
        let kx = select(kx_val, ky_val, is_neg);
        let ky = select(ky_val, kx_val, is_neg);

        let Sz = ray_direction_reciprocal[kz];
        // compared to the paper, extra negation here makes math later on simpler
        let Sx = -ray_direction[kx] * Sz;
        let Sy = -ray_direction[ky] * Sz;

        // note that refactoring the following traversal out into a separate function can drastically change performance
        // performance seems to be worsened no matter if the function returns a struct or it takes ptr<function, f32 or i32> for hit_t hit_index hit_v hit_w

        // also, changing stack size in a refactored function vs. inline produces different results
        // for example, in function, size of 64 produces 17 FPS while size of 128 surprisingly produces 46 FPS; inlined, 64 vs 128 produce same 53 FPS

        // setup fixed-size stack for BVH traversal (depth 64 is sufficient for millions of triangles)
        var stack: array<u32, 64>; // TODO: try stackless

        // push root node
        stack[0] = 0u;
        var stack_ptr = 1u;

        // BVH traversal
        while stack_ptr > 0u {
            // pop node
            stack_ptr -= 1u;
            let node = bvh_nodes[stack[stack_ptr]];

            if node.tri_count > 0u {
                // leaf node: intersect with triangles
                for (var i = 0u; i < node.tri_count; i += 1u) {
                    let tri_index = node.left_first + i;
                    let tri = geometries[tri_index];

                    let v0 = vec3f(tri.vertices[0], tri.vertices[1], tri.vertices[2]);
                    let v1 = vec3f(tri.vertices[3], tri.vertices[4], tri.vertices[5]);
                    let v2 = vec3f(tri.vertices[6], tri.vertices[7], tri.vertices[8]);

                    // watertight ray-triangle intersection

                    let A = v0 - ray_origin;
                    let B = v1 - ray_origin;
                    let C = v2 - ray_origin;

                    let Ax = fma(Sx, A[kz], A[kx]);
                    let Ay = fma(Sy, A[kz], A[ky]);
                    let Bx = fma(Sx, B[kz], B[kx]);
                    let By = fma(Sy, B[kz], B[ky]);
                    let Cx = fma(Sx, C[kz], C[kx]);
                    let Cy = fma(Sy, C[kz], C[ky]);

                    let U = fma(Cx, By, -Cy * Bx);
                    let V = fma(Ax, Cy, -Ay * Cx);
                    let W = fma(Bx, Ay, -By * Ax);

                    if min(U, min(V, W)) < 0.0 && max(U, max(V, W)) > 0.0 {
                        continue;
                    }

                    let det = U + V + W;

                    if det == 0.0 {
                        continue;
                    }

                    let inv_det = 1.0 / det;

                    let T = Sz * fma(U, A[kz], fma(V, B[kz], W * C[kz])) * inv_det;

                    if T > 0.0 && T < hit_t {
                        hit_t = T;
                        hit_index = i32(tri_index);

                        hit_v = V * inv_det;
                        hit_w = W * inv_det;
                    }
                }
            } else {
                // internal node: test both children's aabb and push children accordingly

                let left_index = node.left_first;
                let right_index = node.left_first + 1u;

                let neg_ray_origin_div_direction = -ray_origin * ray_direction_reciprocal;

                let dist_left = intersect_aabb(bvh_nodes[left_index], neg_ray_origin_div_direction, ray_direction_reciprocal, hit_t);
                let dist_right = intersect_aabb(bvh_nodes[right_index], neg_ray_origin_div_direction, ray_direction_reciprocal, hit_t);

                let hit_left = dist_left >= 0.0;
                let hit_right = dist_right >= 0.0;

                if hit_left && hit_right {
                    // both children nodes are hit
                    // push the further node first, so the closer node is popped first
                    let left_closer = dist_left < dist_right;
                    stack[stack_ptr] = select(left_index, right_index, left_closer);
                    stack[stack_ptr + 1u] = select(right_index, left_index, left_closer);
                    stack_ptr += 2u;
                } else if hit_left {
                    stack[stack_ptr] = left_index;
                    stack_ptr += 1u;
                } else if hit_right {
                    stack[stack_ptr] = right_index;
                    stack_ptr += 1u;
                }
            }
        }

        // ray escaped the scene, render sky color and terminate
        if hit_index == -1 {
            // black sky
            // let sky_color = vec3f(0.0);

            // hard-edged color bands for easy reflection debugging
            // let dir_y = ray_direction.y;
            // var raw_sky_color = vec3f(0.0);
            // if dir_y > 0.6 {
            //     raw_sky_color = vec3f(1.0, 0.0, 0.0); // red (top)
            // } else if dir_y > 0.3 {
            //     raw_sky_color = vec3f(0.0, 1.0, 0.0); // green
            // } else if dir_y > 0.0 {
            //     raw_sky_color = vec3f(0.0, 0.0, 1.0); // blue
            // } else if dir_y > -0.3 {
            //     raw_sky_color = vec3f(1.0, 1.0, 0.0); // yellow
            // } else if dir_y > -0.6 {
            //     raw_sky_color = vec3f(1.0, 0.0, 1.0); // magenta
            // } else {
            //     raw_sky_color = vec3f(0.0, 1.0, 1.0); // cyan (bottom)
            // }
            // var sky_color = vec3f(0.0);
            // let luma = dot(raw_sky_color, LUMA_COEFFICIENTS);
            // if luma > 0.0 {
            //     sky_color = (raw_sky_color / luma) * 8000.0;
            // }

            // normal clear sky at 8000 nits (should be paired with EXPOSURE from Sunny 16), calculated like so:
            // raw_sky_color = vec3f(0.4, 0.6, 1.0), and target_nits = 8000
            // sky_color = (raw_sky_color / dot(raw_sky_color, LUMA_COEFFICIENTS)) * target_nits
            // let sky_color = vec3f(5457.4, 8186.1, 13643.5);

            // use skybox texture

            let u = fma(atan2(ray_direction.z, ray_direction.x), INV_2PI, 0.5);
            let v = fma(-asin(ray_direction.y), INV_PI, 0.5);

            let sky_color = textureSampleLevel(skybox_texture, skybox_sampler, vec2f(u, v), 0.0).rgb;

            final_color += throughput * sky_color * skybox.average_luminance;
            break; // terminate this ray; this pixel's final color is now determined
        }

        let attr = attributes[u32(hit_index)];

        // only 2 barycentric coordinates are given, but we know that the point is valid inside the triangle so all coordinates must sum to 1
        // so, the third barycentric coordinate can just be computed as 1 - (v + w)
        let hit_u = 1.0 - hit_v - hit_w;
        let interpolated_uv = fract(hit_u * attr.uv0 + hit_v * attr.uv1 + hit_w * attr.uv2); // fract is used to allow UVs outside of [0, 1] for tiling/repeating textures

        let mat = materials[attr.index];

        var base_color = mat.base_color.rgb; // OpenPBR does not allow alpha in base_color, so alpha value is not used at all even though it is included in glTF
        let base_weight = mat.base_weight;
        var base_metalness = mat.base_metalness;
        var base_diffuse_roughness = mat.base_diffuse_roughness;
        let emission_weight = mat.emission_weight;
        var emission_color = mat.emission_color_and_luminance.rgb;
        let emission_luminance = mat.emission_color_and_luminance.a;
        var specular_color = mat.specular_color_and_weight.rgb;
        var specular_weight = mat.specular_color_and_weight.a;
        let specular_roughness = mat.specular_roughness;
        let specular_roughness_anisotropy = mat.specular_roughness_anisotropy; // TODO: investigate how anisotropy actually works; should there be a parameter for the direction of anisotropy? OpenPBR doesn't specify it
        // let specular_haze = mat.specular_haze;
        // let specular_haze_spread = mat.specular_haze_spread;
        let specular_ior = mat.specular_ior;
        // let specular_retroreflectivity = mat.specular_retroreflectivity;

        // TODO: specular haze and retroreflectivity are not implemented yet

        if mat.base_color_tex_layer >= 0 {
            let base_color_tex = textureSampleLevel(
                srgb_texture_atlas,
                atlas_sampler,
                fma(interpolated_uv, mat.base_color_uv.zw, mat.base_color_uv.xy),
                mat.base_color_tex_layer,
                0.0
            );
            base_color *= base_color_tex.rgb; // for base_color texture, the alpha channel is linear per glTF spec, but OpenPBR does not specify alpha channel for base_color, so it is ignored
            // not sure if textureSampleLevel() on a wgpu::TextureFormat::Rgba8UnormSrgb would convert the alpha channel to linear, though
        }
        if mat.metallic_roughness_tex_layer >= 0 {
            let mr_tex = textureSampleLevel(
                linear_texture_atlas,
                atlas_sampler,
                fma(interpolated_uv, mat.metallic_roughness_uv.zw, mat.metallic_roughness_uv.xy),
                mat.metallic_roughness_tex_layer,
                0.0
            );
            base_metalness *= mr_tex.b;
            base_diffuse_roughness *= mr_tex.g;
        }
        if mat.specular_color_tex_layer >= 0 {
            let specular_color_tex = textureSampleLevel(
                srgb_texture_atlas,
                atlas_sampler,
                fma(interpolated_uv, mat.specular_color_uv.zw, mat.specular_color_uv.xy),
                mat.specular_color_tex_layer,
                0.0
            );
            specular_color *= specular_color_tex.rgb; // alpha channel ignored per glTF spec; OpenPBR also doesn't specify alpha channel for specular color
        }
        if mat.specular_weight_tex_layer >= 0 {
            let specular_factor_tex = textureSampleLevel(
                linear_texture_atlas,
                atlas_sampler,
                fma(interpolated_uv, mat.specular_weight_uv.zw, mat.specular_weight_uv.xy),
                mat.specular_weight_tex_layer,
                0.0
            );
            specular_weight *= specular_factor_tex.a;
        }
        if mat.emission_tex_layer >= 0 {
            let emission_tex = textureSampleLevel(
                srgb_texture_atlas,
                atlas_sampler,
                fma(interpolated_uv, mat.emission_uv.zw, mat.emission_uv.xy),
                mat.emission_tex_layer,
                0.0
            );
            emission_color *= emission_tex.rgb; // alpha channel ignored per glTF spec; OpenPBR also doesn't specify alpha channel for emission color
        }

        // not dividing by dot(emission_color, LUMA_COEFFICIENTS) because OpenPBR did not specify that
        final_color += throughput * emission_weight * emission_luminance * emission_color;

        let V = -ray_direction; // view vector, going from the surface to the camera expressed in world space

        // flat normals can be precomputed and passed to the shader in TriangleAttribute, but this calculation seems simple, so precomputation probably isn't better
        let tri = geometries[u32(hit_index)];
        let v0 = vec3f(tri.vertices[0], tri.vertices[1], tri.vertices[2]);
        let v1 = vec3f(tri.vertices[3], tri.vertices[4], tri.vertices[5]);
        let v2 = vec3f(tri.vertices[6], tri.vertices[7], tri.vertices[8]);
        let flat_N_raw = normalize(cross(v1 - v0, v2 - v0));
        // normalize is safe since the triangle must be degenerate for the cross product to have length of zero

        // flat normal is used for this check, not normal interpolated with vertex normal
        let back_face_multiplier = select(-1.0, 1.0, dot(V, flat_N_raw) > 0.0);

        let flat_N = flat_N_raw * back_face_multiplier;

        // get T tangent (x), B bitangent (y), and N normal (z), all orthogonal to each other, 
        // which form a coordinate basis used to reorient vectors from local normal space -> world space or vice versa

        let n0 = vec3f(attr.normals[0], attr.normals[1], attr.normals[2]);
        let n1 = vec3f(attr.normals[3], attr.normals[4], attr.normals[5]);
        let n2 = vec3f(attr.normals[6], attr.normals[7], attr.normals[8]);
        var N = normalize(hit_u * n0 + hit_v * n1 + hit_w * n2) * back_face_multiplier; // should be safe

        // T should also be multiplied by back_face_multiplier
        var T = normalize(hit_u * attr.t0.xyz + hit_v * attr.t1.xyz + hit_w * attr.t2.xyz) * back_face_multiplier; // should be safe

        // Gram-Schmidt orthogonalization
        // length(T - dot(N, T) * N) = 0 happens when T = 0 or when T and N are parallel; neither should ever be the case
        // so, no need to check for division by zero for this normalization
        T = normalize(T - dot(N, T) * N);

        var B = cross(N, T) * attr.t0.w * back_face_multiplier;
        // no normalization needed since the cross product of two unit, orthogonal vectors always produces a unit vector

        var N_changed = false;

        if mat.normal_tex_layer >= 0 {
            let normal_tex = textureSampleLevel(
                linear_texture_atlas,
                atlas_sampler,
                fma(interpolated_uv, mat.normal_uv.zw, mat.normal_uv.xy),
                mat.normal_tex_layer,
                0.0
            );

            // re-range from [0, 1] RGB to [-1, 1] vector space
            // https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#_material_normaltextureinfo_scale
            var local_n = normal_tex.rgb * 2.0 - 1.0; // in WGSL, vec minus scalar works component-wise as intended; see https://www.w3.org/TR/WGSL/#arithmetic-expr

            // per glTF spec, normal textures should have blue values in (0.5, 1.0]
            // hence, local_n.z should be >0, so the normalize below is safe
            local_n = normalize(vec3f(local_n.xy * mat.normal_scale, local_n.z));

            N = normalize(mat3x3f(T, B, N) * local_n); // normalize is also safe
            N_changed = true;
        }

        // dot(N, V) being < 0 means that the light ray (wo) actually came from below the surface, which is impossible, since the ray should have been occluded by other geometry earlier on
        // the physically accurate solution would be to just break (terminate the ray) here, but that would create ugly black
        // after all, normals are inherently an unphysical illusion; so instead, reorient N so that shading makes sense, at the cost of disobeying the artist a little bit
        // also note that just handling dot(N, V) < 0 by reorienting N just enough so that dot(N, V) >= 0 is not a robust solution because: 
        // even when dot(N, V) >= 0, the new light ray (wi) can still have a high chance of going under the surface (particularly probable when the material is a perfect mirror)
        // when the new light ray goes under the surface, the light ray is terminated, which, among other problems, causes ugly black
        // so here, reorient N by more than enough to make not only dot(N, V) >= 0, but also to make potentially less new light rays go under the surface
        // specifically, reorient N so that dot(N, V) = 0.5 * dot(flat_N, V)
        // 0.5 * dot(flat_N, V) comes from this derivation:
        // suppose the material is a perfect mirror; then, the new light ray is R = reflect(-V, N) = 2 * dot(N, V) * N - V
        // the goal is to reorient N just enough so that dot(R, flat_N) >= 0; this also guarantees that dot(N, V) >= 0
        // then, dot(2 * dot(N, V) * N - V, flat_N) >= 0
        // distribute to get 2 * dot(N, V) * dot(N, flat_N) - dot(V, flat_N) >= 0
        // here, an assumption is made, dot(N, flat_N) ≈ 1, which underestimates the amount N should be reoriented
        // then, 2 * dot(N, V) - dot(flat_N, V) >= 0
        // so dot(N, V) >= 0.5 * dot(flat_N, V)

        // note that N, V, and flat_N should all be unit vectors
        let flat_NdotV = dot(flat_N, V); // this is definitely a positive value 
        let threshold = 0.5 * flat_NdotV;
        var NdotV = dot(N, V);
        if NdotV < threshold {
            // reorient N just enough so that dot(N, V) = threshold:
            // dot(mix(N, flat_N, t), V) = threshold, and we solve for t
            // note that mix(N, flat_N, t) is equivalent to N + t * (flat_N - N)
            // hence t = (threshold - NdotV) / (flat_NdotV - NdotV)
            // note that (flat_NdotV > NdotV) should be true, since (0.5 * flat_NdotV > NdotV) was the condition; division by zero is impossible here (and no saturate() is needed either)
            N = normalize(mix(N, flat_N, (threshold - NdotV) / (flat_NdotV - NdotV))); // note that this doesn't actually guarantee that dot(N, V) = threshold, since mix() does not produce a unit vector, and the normalize() changes things
            NdotV = dot(N, V); // note that technically this could be 0.0, which would result in division by zero later on, but I don't observe any problems during runtime as it is now
            N_changed = true;
        }

        // if we don't assume dot(N, flat_N) ≈ 1:
        // let R = reflect(-V, N);
        // let flat_NdotR = dot(R, flat_N);
        // if flat_NdotR < 0.0 { // dot(N, V) < 0 is implicitly handled too, since dot(N, V) < 0 when flat_NdotR < 0 thanks to how the math in reflect() works
        //     N = normalize(R - flat_NdotR * flat_N + V); // R - flat_NdotR * flat_N is the new R
        //     N_changed = true;
        // }
        // then just let NdotV = dot(N, V);
        // this approach is not adopted because somehow it produces less visually intuitive results compared to when dot(N, flat_N) ≈ 1 is assumed
        // better approaches might be brute-force geometry or "Microfacet-based Normal Mapping for Robust Monte Carlo Path Tracing" https://jo.dreggn.org/home/2017_normalmap.pdf

        if N_changed {
            // re-orthogonalize since N changed
            // division by zero happens if the new N is parallel to T
            T = normalize(T - dot(N, T) * N);
            B = cross(N, T) * attr.t0.w * back_face_multiplier;
        }

        // basic implementation of OpenPBR dev_1.2 spec, assuming:
        // M_PBR = mix(layer(S_diffuse, S_gloss), S_metal, base_metalness)
        // see https://github.com/AcademySoftwareFoundation/OpenPBR/tree/dev_1.2

        let alpha = alpha_anisotropic(specular_roughness, specular_roughness_anisotropy);

        let specular_ior_modulated = specular_ior_modulated(specular_ior, specular_weight);

        // there should be a check for TIR here: if specular_ior >= 1.0, just use Fresnel, else, TIR is possible
        // no need to also check if specular_ior_modulated >= 1.0 since math guarantees that if specular_ior >= 1.0, specular_ior_modulated >= 1.0 as well
        // when specular_ior < 1, specular_ior_modulate < 1 as well, except for when specular_weight = 0, for which specular_ior_modulated = 1 for all specular_ior

        let F0_conductor = base_weight * base_color; // OpenPBR right above eq.29; RGB reflectivity at normal incidence

        // ray, in local tangent space, going from the surface to the camera (or to the previous hit)
        // this ray is "outgoing," but note that since we're using reverse ray tracing, this ray is actually our input, the old ray direction
        let wo = vec3f(dot(T, V), dot(B, V), NdotV);
        // ray "incoming" direction in local tangent space
        // the new ray direction
        var wi: vec3f;

        // precompute some values
        let one_plus_lambda_wo = 1.0 + ggx_lambda(wo, alpha);
        let ltc_coefficients = vec4f(
            1.0 + base_diffuse_roughness * (0.303392 + (-0.518982 + 0.111709 * wo.z) * wo.z + (-0.276266 + 0.335918 * wo.z) * base_diffuse_roughness),
            base_diffuse_roughness * (-1.16407 + 1.15859 * wo.z + (0.150815 - 0.150105 * wo.z) * base_diffuse_roughness) / (wo.z * wo.z * wo.z - 1.43545),
            1.0 + base_diffuse_roughness * (0.20013 + (-0.506373 + 0.261777 * wo.z) * wo.z),
            base_diffuse_roughness * (0.540852 + (-1.01625 + 0.475392 * wo.z) * wo.z) / (-1.0743 + (0.0725628 + wo.z) * wo.z),
        );
        let orthonormal_basis_ltc_lenSqr = dot(wo.xy, wo.xy);
        var orthonormal_basis_ltc_X = vec3f(1.0, 0.0, 0.0);
        if orthonormal_basis_ltc_lenSqr > 0.0 {
            orthonormal_basis_ltc_X = vec3f(wo.x, wo.y, 0.0) * inverseSqrt(orthonormal_basis_ltc_lenSqr);
        }
        let orthonormal_basis_ltc_wo = mat3x3f(
            orthonormal_basis_ltc_X,
            vec3f(-orthonormal_basis_ltc_X.y, orthonormal_basis_ltc_X.x, 0.0),
            vec3f(0.0, 0.0, 1.0)
        );

        // TODO: OpenPBR 3.6.3 Multiple Scattering: Torrance-Sparrow as it is used below (single-scattering) doesn't conserve energy, as it neglects to account for multiple scattering between the microfacets
        // TODO: white furnace test fails slightly, unrelated to multiple scattering, related to EON

        // metal (f_conductor), gloss (f_dielectric), and diffuse (f_diffuse), referred to as "lobes," all are described by microfacet model
        // we assume that specifically, metal and gloss have microfacets of perfect mirror-like surface
        // thus, both metal and gloss can use the Torrance-Sparrow (or Cook-Torrance) equation as their BRDFs
        // the only thing that differs between metal and gloss is their Fresnel factor in their Torrance-Sparrow BRDFs
        // diffuse, though, has microfacets of Lambertian surface, not mirror-like
        // so, diffuse doesn't use Torrance-Sparrow, but instead uses energy-preserving Oren-Nayar (EON) BRDF

        // note that OpenPBR states, "For dielectrics there is also a BTDF..."
        // but that is not implemented here since layering is done with albedo-scaling approximation (with E_dielectric below)
        // it's also not implemented because the spec states that gloss has "infinitesimal layer thickness," 
        // and that diffuse is "an (index-matched) opaque slab with a diffuse BRDF" (not a BSDF)

        // Monte Carlo is basically: we can estimate the area of a shape by encompassing it in another shape (where we know this outer shape's area already),
        // and we throw darts randomly and count what percentage of all darts landed in the inner area; 
        // if we throw an infinite amount of darts, then this percentage, multiplied with the outer shape's area, should yield the inner shape's area

        // importance sampling with Monte Carlo would be to just throw darts more frequently in some direction we choose rather than completely randomly,
        // and then compensating by weighing each dart (their contribution to the percentage) according to how often darts were thrown in that direction, 
        // so that with infinite amount of darts, the estimated value should still equal the exact value

        // below, we do importance sampling for determining a singular wi
        // we choose only a singular wi because otherwise, the amount of rays to evaluate would explode exponentially
        // we use importance sampling because there are multiple sampling strategies for determining a wi, and we reduce variance by assigning each strategy a different probability of being chosen:
        // for metal and gloss, for the normal distribution function (D term) in Torrance-Sparrow BRDF, we use GGX (Trowbridge-Reitz); to sample this GGX normal distribution function, we use the VNDF (visible normal distribution function) sampling strategy
        // for diffuse, EON BRDF has its own sampling strategy (a mix between uniform and CLTC sampling)
        // hence, we calculate probabilities for selecting either VNDF or EON sampling

        // E_dielectric is used to calculate better probabilities below and also to calculate f_glossy_diffuse with albedo-scaling approximation later
        let E_dielectric = estimate_e_dielectric(wo, one_plus_lambda_wo, alpha, specular_ior_modulated, specular_color);

        // calculate probabilities for selecting either VNDF or EON sampling

        let albedo_gloss = dot(E_dielectric, LUMA_COEFFICIENTS);
        let albedo_diffuse = (1.0 - albedo_gloss) * base_weight * dot(base_color, LUMA_COEFFICIENTS);
        let fresnel_schlick_metal = F0_conductor + (1.0 - F0_conductor) * fifth_power(1.0 - wo.z);
        let albedo_metal = dot(fresnel_schlick_metal, LUMA_COEFFICIENTS);

        let w_ggx = mix(albedo_gloss, albedo_metal, base_metalness);
        let w_diffuse = (1.0 - base_metalness) * albedo_diffuse;

        // probabilities
        let p_ggx = clamp(w_ggx / max(w_ggx + w_diffuse, 1e-7), 0.1, 0.9); // TODO: this 1e-7 small number hasn't been tested
        let p_diffuse = 1.0 - p_ggx;

        // compute two values for EON
        // will be needed no matter if EON sampling is chosen or not, since in the end, need to calculate total PDF which includes calculating EON PDF, which needs these two values
        let P_u = pow(base_diffuse_roughness, 0.1) * (0.162925 + (-0.372058 + (0.538233 - 0.290822 * wo.z) * wo.z) * wo.z); // base_diffuse_roughness of 0.0 causes P_u to be 0.0
        let P_c = 1.0 - P_u;

        let two_pi_rand = 2.0 * PI * rand();
        let rand1 = rand();

        // determine a wi by choosing a sampling strategy according to the calculated probabilities
        if rand() < p_ggx {
            // VNDF sampling
            // "Sampling Visible GGX Normals with Spherical Caps" by Dupuy and Benyoub (2023)
            // https://arxiv.org/abs/2306.05044
            // note that the paper uses reversed convention for ray naming compared to this path tracer: swap wo and wi from the paper while implementing its code
            wi = reflect(-wo, sample_VNDF(wo, alpha, rand1, two_pi_rand));
        } else {
            // EON sampling
            // "EON: A practical energy-preserving rough diffuse BRDF" by Portsmouth, Kutz, and Hill (2025)
            // https://arxiv.org/abs/2410.18026

            // Listing 4 sample_EON

            // note that the paper specifies random numbers in [0, 1]; using [0, 1) instead shouldn't matter
            if rand1 < P_u { // rand1 <= P_u is problematic in the paper; P_u and rand1 being both 0.0 would cause division by zero
                // sample uniformly

                let u1 = rand1 / P_u; // cannot be division by zero since rand1 < P_u is the condition
                let sinTheta = sqrt(1.0 - u1 * u1);
                wi = vec3f(sinTheta * cos(two_pi_rand), sinTheta * sin(two_pi_rand), u1);
            } else {
                // sample with CLTC

                let u1 = (rand1 - P_u) / P_c;

                let a = ltc_coefficients.x;
                let b = ltc_coefficients.y;
                let c = ltc_coefficients.z;
                let d = ltc_coefficients.w;
                let R = sqrt(u1);
                let y = R * sin(two_pi_rand);
                let s = fma(0.5, inverseSqrt(fma(d, d, 1.0)), 0.5);
                let x = -mix(sqrt(fma(-y, y, 1.0)), R * cos(two_pi_rand), s);
                let wh = vec3f(x, y, sqrt(max(1.0 - (x * x + y * y), 0.0)));
                let wi_unnormalized = vec3f(a * wh.x + b * wh.z, c * wh.y, d * wh.x + wh.z);

                wi = normalize(orthonormal_basis_ltc_wo * wi_unnormalized);
            }
        }

        // if "incoming" ray goes beneath the surface, this ray ends
        // TODO: bright fireflies when specular weight is 0.0, specular roughness is close to zero, and there are normals that aren't flat. diffuse roughness, base weight, base metalness, specular IOR, and specular anisotropy don't seem to matter very much
        if wi.z <= 0.0 { break; }

        let sample_lobes_result = sample_lobes(
            wo, wi,
            p_ggx, p_diffuse,
            base_color, base_weight, base_metalness, base_diffuse_roughness, specular_color, specular_weight,
            specular_ior_modulated,
            alpha,
            one_plus_lambda_wo,
            F0_conductor,
            E_dielectric,
            ltc_coefficients, orthonormal_basis_ltc_wo, P_u, P_c
        );

        throughput *= sample_lobes_result.f_total * wi.z / sample_lobes_result.pdf_total;

        // russian roulette
        // terminate paths early if they contribute very little light
        if bounce > 3u { // don't start terminating until after a few bounces to allow light to bounce around and contribute meaningfully
            let p_survive = clamp(max(throughput.x, max(throughput.y, throughput.z)), 0.05, 0.95);
            if rand() > p_survive {
                break;
            }
            // originally, the expected contribution is X
            // say p_survive = 30%; then, the contribution is 30% * X + 70% * 0 = 30% * X
            // thus, to keep the sampling unbiased, divide both sides by 30%: (30% * X + 70% * 0) / 30% = X
            // hence, divide throughput by p_survive
            throughput /= p_survive;
        }

        let hit_point = ray_direction * hit_t + ray_origin; // this matches f(x) = m * x + b

        // offset ray origin slightly to prevent self-intersection shadowing (shadow acne)
        ray_origin = hit_point + flat_N * max(1e-4, 1e-4 * length(hit_point));
        ray_direction = wi.x * T + wi.y * B + wi.z * N;
        ray_direction_reciprocal = 1.0 / ray_direction;
    }

    // blend current frame into accumulation buffer
    let pixel_index = global_id.y * u32(screen_dims.x) + global_id.x;

    // frame_count resets to 0 when the camera moves, but the increment happens after the reset, so frame count can never be zero in the shader
    // only from the second frame onward can we be sure that the accumulation buffer has valid data to blend with
    let accumulated = select(vec3f(0.0), accumulation_buffer[pixel_index].rgb, frame_count > 1u) + final_color;

    accumulation_buffer[pixel_index] = vec4f(accumulated, 1.0);

    // average out result
    textureStore(screen, global_id.xy, vec4f(EXPOSURE * accumulated / f32(frame_count), 1.0)); // frame count is always at least 1
}