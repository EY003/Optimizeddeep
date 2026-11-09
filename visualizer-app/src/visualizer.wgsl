struct Uniforms {
    dims: vec4<f32>,          // num_bars, is_vertical, is_smooth, is_mirrored
    layout: vec4<f32>,        // bar_width, bar_gap, screen_width, screen_height
    outline_color: vec4<f32>, // r, g, b, a
    extras: vec4<f32>,        // outline_thickness, bar_radius, opacity, direction (0=Up,1=Down,2=Right,3=Left)
};

struct Bar {
    height: f32,
    peak_height: f32,
    color_top: vec4<f32>,
    color_bottom: vec4<f32>,
    show_peak: f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var<storage, read> bars: array<Bar>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) in_vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32((in_vertex_index << 1u) & 2u);
    let y = f32(in_vertex_index & 2u);
    out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

// Signed distance function for a rounded rectangle
fn sd_rounded_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0))) - r;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let num_bars = u.dims.x;
    let is_smooth = u.dims.z > 0.5;
    let is_mirrored = u.dims.w > 0.5;
    let bar_width = u.layout.x;
    let bar_gap = u.layout.y;
    let screen_w = u.layout.z;
    let screen_h = u.layout.w;
    let outline_thickness = u.extras.x;
    let bar_radius = u.extras.y;
    let direction = u.extras.w;

    let stride = bar_width + bar_gap;
    let total_size = (num_bars * stride) - bar_gap;

    var p = in.uv * vec2<f32>(screen_w, screen_h);

    var local_x = 0.0;
    var local_y = 0.0;

    if (direction < 0.5) {           // Up
        local_x = p.x - screen_w * 0.5 + total_size * 0.5;
        local_y = screen_h - p.y;    // measure from bottom
    } else if (direction < 1.5) {    // Down
        local_x = p.x - screen_w * 0.5 + total_size * 0.5;
        local_y = p.y;                // measure from top
    } else if (direction < 2.5) {    // Right
        local_x = p.y - screen_h * 0.5 + total_size * 0.5;
        local_y = p.x;                // measure from left
    } else {                         // Left
        local_x = p.y - screen_h * 0.5 + total_size * 0.5;
        local_y = screen_w - p.x;     // measure from right
    }

    // Identify which bar the pixel belongs to
    var bar_idx_f = floor(local_x / stride);
    if (bar_idx_f < 0.0) { return vec4<f32>(0.0); }

    var bar_idx = u32(bar_idx_f);

    if (is_mirrored) {
        let half = u32(num_bars) / 2u;
        if (bar_idx >= half) {
            bar_idx = u32(num_bars) - 1u - bar_idx;
        }
    }

    if (bar_idx >= u32(num_bars)) { return vec4<f32>(0.0); }

    let bar = bars[bar_idx];

    let bar_local_x = local_x - (bar_idx_f * stride);
    if (bar_local_x > bar_width) { return vec4<f32>(0.0); }  // in gap

    let center_x = bar_width * 0.5;

    // Smooth mode: interpolate height between adjacent bars
    var current_height = bar.height;
    if (is_smooth) {
        var prev_h = bar.height;
        var next_h = bar.height;

        if (bar_idx > 0u) { prev_h = bars[bar_idx - 1u].height; }
        if (bar_idx < u32(num_bars) - 1u) { next_h = bars[bar_idx + 1u].height; }

        let t = bar_local_x / bar_width;
        // Smoothstep-style blend to neighbours
        let s = t * t * (3.0 - 2.0 * t);
        current_height = mix(prev_h, bar.height, s + 0.5) * (1.0 - s) 
                       + mix(bar.height, next_h, s) * s;
        // Simpler approximation that looks good:
        current_height = prev_h * (1.0 - s) * 0.5 
                       + bar.height * 1.0
                       + next_h * s * 0.5;
        current_height = clamp(current_height, 0.0, 1.0);
    }

    let max_dim = max(screen_w, screen_h);
    let draw_height = current_height * max_dim;

    // Outside the bar (above the top)
    if (local_y > draw_height) {
        // Peak hold line
        if (bar.show_peak > 0.5) {
            let peak_y = bar.peak_height * max_dim;
            let dist_to_peak = abs(local_y - peak_y);
            if (dist_to_peak < 2.0) {
                return vec4<f32>(bar.color_top.rgb, bar.color_top.a);
            }
        }
        return vec4<f32>(0.0);
    }

    // Base color: gradient from bottom to top
    let gradient_t = clamp(local_y / max(draw_height, 1.0), 0.0, 1.0);
    var final_color = mix(bar.color_bottom, bar.color_top, gradient_t);

    // Outline
    if (outline_thickness > 0.0) {
        let box_size = vec2<f32>(center_x, draw_height * 0.5);
        let box_center = vec2<f32>(center_x, draw_height * 0.5);
        let p_box = vec2<f32>(bar_local_x, local_y) - box_center;
        let dist = sd_rounded_box(p_box, box_size, bar_radius);
        if (dist > -outline_thickness) {
            final_color = u.outline_color;
        }
    }

    return final_color;
}
