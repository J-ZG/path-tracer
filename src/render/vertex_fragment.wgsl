// use a full-screen triangle to display the output texture, and sample from the texture in the fragment shader to apply tone mapping and sRGB gamma correction
// full-screen triangle is more efficient than rendering a quad with two triangles, and avoids issues with interpolation at the edges of the quad.

@vertex
fn vs_main(@builtin(vertex_index) vert_index: u32) -> @builtin(position) vec4f {
    let pos = array(
        vec2f(-1.0, -1.0), // clip space range [-1, 1] so extending to 3 stretches the triangle to cover the clip space
        vec2f(3.0, -1.0), // https://webgpufundamentals.org/webgpu/lessons/webgpu-large-triangle-to-cover-clip-space.html
        vec2f(-1.0, 3.0),
    );
    return vec4f(pos[vert_index], 0.0, 1.0);
}

@group(0) @binding(0) var output_texture: texture_2d<f32>;

// ACES filmic tone mapping approximation
// see https://knarkowicz.wordpress.com/2016/01/06/aces-filmic-tone-mapping-curve/
fn aces_film(color: vec3f) -> vec3f { // TODO: AgX may be better?
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;

    return clamp((color * (a * color + b)) / (color * (c * color + d) + e),
        vec3f(0.0),
        vec3f(1.0));
}

@fragment
fn fs_main(@builtin(position) frag_position: vec4f) -> @location(0) vec4f {
    let color = textureLoad(output_texture, vec2i(frag_position.xy), 0); // not using textureSample since "Non-filterable float textures can't be sampled with a filtering sampler"

    // tone mapping
    let mapped = aces_film(color.rgb);

    // linear -> sRGB gamma conversion
    let srgb_color = select(
        mapped * 12.92,
        pow(mapped, vec3f(1.0 / 2.4)) * 1.055 - 0.055,
        mapped > vec3f(0.0031308)
    );

    return vec4f(srgb_color, color.a);
}