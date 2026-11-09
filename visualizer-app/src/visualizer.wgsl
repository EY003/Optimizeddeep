struct Uniforms {
    dims: vec4<f32>,          // num_bars, is_vertical, is_smooth, is_mirrored
    layout: vec4<f32>,        // bar_width, bar_gap, screen_width, screen_height
    transform: vec4<f32>,     // x_offset, y_offset, height_scale, max_bar_height
    outline_color: vec4<f32>, // r, g, b, a
    extras: vec4<f32>,        // outline_thickness, bar_radius, opacity, direction
};

struct Bar {
    height: f32,
    peak_height: f32,
    show_peak: f32,
    _pad0: f32,
};

const NUM_SEGMENTS: u32 = 32u;

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var<storage, read> bars: array<Bar>;
@group(0) @binding(2) var<storage, read> colors: array<vec4<f32>>;

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
    
    let x_offset = u.transform.x;
    let y_offset = u.transform.y;
    let height_scale = u.transform.z;
    let max_h = u.transform.w;

    let outline_thickness = u.extras.x;
    let bar_radius = u.extras.y;
    let opacity = u.extras.z;
    let direction = u.extras.w;

    let stride = bar_width + bar_gap;
    let total_size = (num_bars * stride) - bar_gap;
    let screen_center_x = screen_w * 0.5;
    let screen_center_y = screen_h * 0.5;

    var p = in.uv * vec2<f32>(screen_w, screen_h);

    var local_x = 0.0;
    var local_y = 0.0;

    // Apply direction and offsets
    if (direction < 0.5) {           // Up
        local_x = p.x - screen_center_x + (total_size * 0.5) - x_offset;
        local_y = (screen_h - y_offset) - p.y; 
    } else if (direction < 1.5) {    // Down
        local_x = p.x - screen_center_x + (total_size * 0.5) - x_offset;
        local_y = p.y - y_offset;
    } else if (direction < 2.5) {    // Right
        local_x = p.y - screen_center_y + (total_size * 0.5) - y_offset;
        local_y = p.x - x_offset;
    } else {                         // Left
        local_x = p.y - screen_center_y + (total_size * 0.5) - y_offset;
        local_y = (screen_w - x_offset) - p.x;
    }

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
    if (bar_local_x > bar_width) { return vec4<f32>(0.0); }

    let center_x = bar_width * 0.5;

    var current_height = bar.height;
    if (is_smooth) {
        var prev_h = bar.height;
        var next_h = bar.height;
        if (bar_idx > 0u) { prev_h = bars[bar_idx - 1u].height; }
        if (bar_idx < u32(num_bars) - 1u) { next_h = bars[bar_idx + 1u].height; }
        
        let t = bar_local_x / bar_width;
        let s = t * t * (3.0 - 2.0 * t);
        current_height = prev_h * (1.0 - s) * 0.5 + bar.height * 1.0 + next_h * s * 0.5;
        current_height = clamp(current_height, 0.0, 1.0);
    }

    var draw_extent: f32;
    if (direction < 1.5) { draw_extent = screen_h; } else { draw_extent = screen_w; }
    
    var draw_height = current_height * draw_extent * height_scale;
    if (max_h > 0.0) { draw_height = min(draw_height, max_h); }

    // Peak hold
    if (local_y > draw_height) {
        if (bar.show_peak > 0.5) {
            var peak_y = bar.peak_height * draw_extent * height_scale;
            if (max_h > 0.0) { peak_y = min(peak_y, max_h); }
            if (abs(local_y - peak_y) < 2.0) {
                let seg_idx = min(u32(peak_y / max(draw_height, 1.0) * f32(NUM_SEGMENTS)), NUM_SEGMENTS - 1u);
                return colors[bar_idx * NUM_SEGMENTS + seg_idx];
            }
        }
        return vec4<f32>(0.0);
    }

    // Lookup color based on vertical segment
    let seg_idx = min(u32(local_y / max(draw_height, 1.0) * f32(NUM_SEGMENTS)), NUM_SEGMENTS - 1u);
    var final_color = colors[bar_idx * NUM_SEGMENTS + seg_idx];
    final_color.a *= opacity;

    // Outline
    if (outline_thickness > 0.0) {
        let box_size = vec2<f32>(center_x, draw_height * 0.5);
        let box_center = vec2<f32>(center_x, draw_height * 0.5);
        let p_box = vec2<f32>(bar_local_x, local_y) - box_center;
        let dist = sd_rounded_box(p_box, box_size, bar_radius);
        if (dist > -outline_thickness) {
            var o = u.outline_color;
            o.a *= opacity;
            return o;
        }
    }

    return final_color;
}
