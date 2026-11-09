struct Uniforms {
    num_bars: u32,
    is_vertical: u32,
    is_smooth: u32,
    is_mirrored: u32,
    bar_width: f32,
    bar_gap: f32,
    screen_width: f32,
    screen_height: f32,
    outline_thickness: f32,
    outline_color: vec4<f32>,
    bar_radius: f32,
    opacity: f32,
    direction: u32, // 0=Up, 1=Down, 2=Left, 3=Right
    _padding: vec3<f32>,
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

// Signed Distance Function for a rounded rectangle
fn sd_rounded_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0))) - r;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let stride = u.bar_width + u.bar_gap;
    let total_size = (f32(u.num_bars) * stride) - u.bar_gap;

    // Determine pixel position relative to the visualizer's bounding box
    var p = in.uv * vec2<f32>(u.screen_width, u.screen_height);
    
    // Map to local 0..total_size space
    var local_x = p.x - (u.screen_width * 0.5) + (total_size * 0.5);
    var local_y = p.y;

    if (u.direction == 1u) { // Down
        local_y = u.screen_height - p.y;
    } else if (u.direction == 2u) { // Right
        local_x = p.y - (u.screen_height * 0.5) + (total_size * 0.5);
        local_y = p.x;
    } else if (u.direction == 3u) { // Left
        local_x = u.screen_height - p.y - (u.screen_height * 0.5) + (total_size * 0.5);
        local_y = u.screen_width - p.x;
    }

    // Figure out which bar this pixel is in
    var bar_idx = u32(floor(local_x / stride));
    
    // Handle mirroring (center axis mirror)
    if (u.is_mirrored == 1u) {
        let half = u.num_bars / 2u;
        if (bar_idx >= half) {
            bar_idx = u.num_bars - 1u - bar_idx;
        }
    }
    
    if (bar_idx >= u.num_bars) { return vec4<f32>(0.0); }
    
    let bar = bars[bar_idx];
    
    // Local x within the bar (used for rounding and smoothing)
    let bar_local_x = local_x - (f32(bar_idx) * stride);
    if (bar_local_x > u.bar_width) { return vec4<f32>(0.0); } // In the gap
    
    // Center X for distance calculations
    let center_x = u.bar_width * 0.5;
    let dist_x = abs(bar_local_x - center_x);

    // Interpolate height if smooth mode is enabled
    var current_height = bar.height;
    if (u.is_smooth == 1u) {
        var prev_h = bar.height;
        var next_h = bar.height;
        
        if (bar_idx > 0u) { prev_h = bars[bar_idx - 1u].height; }
        if (bar_idx < u.num_bars - 1u) { next_h = bars[bar_idx + 1u].height; }
        
        // Simple smoothstep between adjacent bars
        let t = bar_local_x / u.bar_width;
        let left_blend = mix(prev_h, bar.height, t + 0.5);
        let right_blend = mix(bar.height, next_h, t - 0.5);
        current_height = mix(left_blend, right_blend, t);
    }

    let max_height = u.screen_width.max(u.screen_height);
    let draw_height = current_height * max_height;

    // Check if pixel is outside the bar's height
    if (local_y > draw_height) {
        // Peak hold logic
        if (bar.show_peak > 0.5) {
            let peak_y = bar.peak_height * max_height;
            let dist_to_peak = abs(local_y - peak_y);
            if (dist_to_peak < 2.0) { // 4px thick line
                return bar.color_top;
            }
        }
        return vec4<f32>(0.0);
    }

    // Outline math (SDF)
    var final_color = mix(bar.color_bottom, bar.color_top, local_y / draw_height.max(1.0));
    
    if (u.outline_thickness > 0.0) {
        let box_size = vec2<f32>(center_x, draw_height * 0.5);
        let box_center = vec2<f32>(center_x, draw_height * 0.5);
        let p_box = vec2<f32>(bar_local_x, local_y) - box_center;
        
        let dist = sd_rounded_box(p_box, box_size, u.bar_radius);
        
        // If we are within the outline thickness of the edge
        if (dist > -u.outline_thickness) {
            final_color = u.outline_color;
        }
    }

    final_color.a *= u.opacity;
    return final_color;
}
