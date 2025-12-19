struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(
    @location(0) in_pos: vec2<f32>,
    @location(1) in_uv: vec2<f32>,
) -> VsOut {
    var out: VsOut;
    out.pos = vec4<f32>(in_pos, 0.0, 1.0);
    out.uv = in_uv;
    return out;
}

@group(0) @binding(0)
var tex: texture_2d<f32>;

@group(0) @binding(1)
var samp: sampler;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, in.uv);
}
