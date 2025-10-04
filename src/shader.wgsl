// @group(0) @binding(0)
// var<uniform> dims: vec2<u32>;

// @group(0) @binding(1)
// var<storage> points: array<u32>;

// @group(0) @binding(2)
// var<storage> radius: f32;

// const SQRT3 = sqrt(3.0);

// struct VertexOutput {
//     @builtin(position) pos: vec4<f32>,
//     @location(0) center: vec2<f32>,
// };

// @vertex
// fn emit_triangle(@builtin(vertex_index) idx: u32) -> VertexOutput {
//     // Draw three vertices per dart.
//     let dart_idx = idx / 3;
//     let vert_idx = i32(idx % 3);

//     let dart = points[dart_idx];

//     // Unpack pixel coordinates.
//     let x = dart >> 18;
//     let y = (dart & 65535) >> 2;

//     // Unpack fixed point coordinates.
//     let x_sub = f32(dart >> 16) / 4.0;
//     let y_sub = f32(dart & (65535)) / 4.0;

//     // The triangle's incircle's radius is the desired radius of the final circle. 
//     var out: VertexOutput;
//     out.pos = vec4<f32>(
//         (x_sub + f32(vert_idx - 1) * SQRT3 * radius) / f32(dims.x) * 2.0 - 1.0,
//         (y_sub + f32((vert_idx & 1) * 3 - 1) * radius) / f32(dims.y) * -2.0 + 1.0,
//         1.0,
//         1.0,
//     );
//     out.center = vec2<f32>(x_sub, y_sub);
//     return out;
// }

// @fragment
// fn carve_circle(
//     @builtin(position) pos: vec4<f32>,
//     @location(0) center: vec2<f32>,
// ) {
//     let d = pos.xy - center.xy;
//     if dot(d, d) > radius * radius {
//         discard;
//     }
// }



@group(0) @binding(0) var mySampler: sampler;
@group(0) @binding(1) var myTex: texture_depth_2d;

struct VSOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VSOut {
    // Fullscreen triangle, no vertex buffer
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>( 3.0,  1.0),
        vec2<f32>(-1.0,  1.0),
    );

    var vs_out: VSOut;
    vs_out.pos = vec4<f32>(positions[idx], 0.0, 1.0);
    vs_out.uv = (vs_out.pos.xy * 0.5) + vec2<f32>(0.5, 0.5);
    return vs_out;
}

@fragment
fn fs_main(in: VSOut) -> @location(0) vec4<f32> {
    let depth = textureSample(myTex, mySampler, in.uv);
    return vec4<f32>(depth, depth, depth, 1.0);
}
