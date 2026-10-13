// The ink pipeline's shaders (§3.1). One draw call for a whole page's worth of strokes: the colour
// rides on the vertex, so there is no per-stroke bind group to switch.
//
// The vertex stage converts page pixels to clip space with y pointing down, matching the coordinate
// system the stroke model uses. The fragment stage is a flat fill for now; the pressure-driven edge
// fade, the pencil texture along `u` and the highlighter's multiply blend are the next pass, and they
// are why the mesh already carries `u` and `half_width`.

struct Viewport {
    // Target size in pixels.
    size: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> viewport: Viewport;

struct VertexIn {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    // Page pixels (origin top-left, y down) to clip space (origin centre, y up). Clip space is what
    // wgpu wants for both a window surface and an offscreen texture, so this holds for readback tests
    // and for the real canvas alike.
    let ndc = vec2<f32>(
        in.position.x / viewport.size.x * 2.0 - 1.0,
        1.0 - in.position.y / viewport.size.y * 2.0,
    );
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return in.color;
}