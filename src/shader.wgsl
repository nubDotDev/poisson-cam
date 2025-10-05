@group(0) @binding(0)
var<uniform> dims: vec2<u32>;

@group(0) @binding(1)
var<uniform> radius: f32;

@group(0) @binding(2)
var<storage> points: array<u32>;

const SQRT3 = sqrt(3.0);

struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) center: vec2<f32>,
};

@vertex
fn emit_triangle(@builtin(vertex_index) idx: u32) -> VertexOutput {
    // Draw three vertices per dart.
    let dart_idx = idx / 3;
    let vert_idx = i32(idx % 3);

    let dart = points[dart_idx];

    // Unpack pixel coordinates.
    let x = dart >> 18;
    let y = (dart & 65535) >> 2;

    let x_sub = f32(x) + 0.5;
    let y_sub = f32(y) + 0.5;

    // The triangle's incircle's radius is the desired radius of the final circle. 
    var out: VertexOutput;
    out.pos = vec4<f32>(
        (x_sub + f32(vert_idx - 1) * SQRT3 * radius) / f32(dims.x) * 2.0 - 1.0,
        (y_sub + f32((vert_idx & 1) * 3 - 1) * radius) / f32(dims.y) * -2.0 + 1.0,
        1.0,
        1.0,
    );
    out.center = vec2<f32>(x_sub, y_sub);
    return out;
}

@fragment
fn carve_circle(
    @builtin(position) pos: vec4<f32>,
    @location(0) center: vec2<f32>,
) -> @location(0) vec4<f32> {
    let d = pos.xy - center.xy;
    if dot(d, d) > radius * radius {
        discard;
    }
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
}

@group(0) @binding(3)
var texture: texture_2d<f32>;

@group(0) @binding(4)
var<storage, read_write> radii: array<f32>;

@group(0) @binding(5)
var<uniform> r_bounds: vec2<f32>;

@group(0) @binding(6)
var<uniform> mode: u32;

@compute @workgroup_size(64)
fn calc_radii(
    @builtin(global_invocation_id) global_id: vec3<u32>,
    @builtin(num_workgroups) num_workgroups: vec3<u32>
) {
    let idx = global_id.x + global_id.y * num_workgroups.x * 64;
    if idx < dims.x * dims.y {
        let coord = vec2<u32>(idx % dims.x, idx / dims.x);
        let color = textureLoad(texture, coord, 0);
        var t: f32;
        switch mode {
            case 0: { t = bgra_to_luma(color); }
            case 1: { t = 1.0 - bgra_to_luma(color); }
            case 2: { t = color.z; }
            case 3: { t = color.y; }
            case 4: { t = color.x; }
            default: { return; }
        }
        radii[idx] = inverseSqrt(
            t / (r_bounds.y * r_bounds.y) + (1.0 - t) / (r_bounds.x * r_bounds.x)
        );
    }
}

fn bgra_to_luma(color: vec4<f32>) -> f32 {
    return 0.298936021293775 * color.z +
        0.587043074451121 * color.y +
        0.114020904255103 * color.x;
}
