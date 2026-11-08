// Draws a magnified image with square pixels at any zoom. Inside a source pixel
// the colour stays exact; only the screen pixel a seam falls on gets blended.
// At whole-number zooms no fragment lands on a seam, so nothing is blended.

struct Locals {
    // In framebuffer pixels.
    rect_min: vec2<f32>,
    rect_size: vec2<f32>,
    alpha: f32,
    srgb_target: u32,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> locals: Locals;
@group(0) @binding(1) var image: texture_2d<f32>;
@group(0) @binding(2) var image_sampler: sampler;

// One triangle over the whole viewport. Position comes from the fragment, so
// the image can be far larger than any viewport the GPU allows.
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

fn linear_from_gamma(srgb: vec3<f32>) -> vec3<f32> {
    let cutoff = srgb < vec3<f32>(10.31475);
    let lower = srgb / vec3<f32>(3294.6);
    let higher = pow((srgb + vec3<f32>(14.025)) / vec3<f32>(269.025), vec3<f32>(2.4));
    return select(higher, lower, cutoff);
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = (pos.xy - locals.rect_min) / locals.rect_size;
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) {
        discard;
    }
    let size = vec2<f32>(textureDimensions(image));
    let texel = uv * size;
    let texels_per_pixel = size / locals.rect_size;
    let seam = floor(texel + vec2<f32>(0.5));
    let snapped = seam + clamp((texel - seam) / texels_per_pixel, vec2<f32>(-0.5), vec2<f32>(0.5));
    // egui's textures hold gamma values and are blended as such.
    let colour = textureSampleLevel(image, image_sampler, snapped / size, 0.0) * locals.alpha;
    if locals.srgb_target == 1u {
        return vec4<f32>(linear_from_gamma(colour.rgb * 255.0), colour.a);
    }
    return colour;
}
