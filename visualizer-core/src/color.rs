use auto_palette::{ImageData, Palette};
use image::DynamicImage;

pub struct TrackColors {
    pub dominant: [u8; 3],
    pub text_color: [u8; 3],
}

pub fn extract_colors(img: &DynamicImage) -> TrackColors {
    let rgb = img.to_rgba8();
    let (w, h) = (rgb.width(), rgb.height());
    let raw_pixels = rgb.into_raw();

    let image_data = ImageData::new(w, h, &raw_pixels)
        .expect("Failed to create ImageData from pixels");

    let palette: Palette<f64> = Palette::extract(&image_data)
        .expect("Failed to extract color palette");

    let dominant = palette
        .find_swatches(1)
        .first()
        .map(|swatch| {
            let rgb = swatch.color().to_rgb();
            [rgb.r, rgb.g, rgb.b]
        })
        .unwrap_or([255, 255, 255]);
        
    let luminance = 0.299 * dominant[0] as f32
        + 0.587 * dominant[1] as f32
        + 0.114 * dominant[2] as f32;
    let text_color = if luminance > 128.0 { [0, 0, 0] } else { [255, 255, 255] };

    TrackColors {
        dominant,
        text_color,
    }
}

// ── NEW COLOR HELPERS ──────────────────────────────────────────

/// Linear interpolation between two RGB colors.
pub fn lerp_rgb(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        (a[0] as f32 + (b[0] as f32 - a[0] as f32) * t) as u8,
        (a[1] as f32 + (b[1] as f32 - a[1] as f32) * t) as u8,
        (a[2] as f32 + (b[2] as f32 - a[2] as f32) * t) as u8,
    ]
}

/// Sample a multi-stop gradient at a specific position (0.0 to 1.0).
pub fn sample_gradient(stops: &[crate::config::GradientStop], t: f32) -> [u8; 3] {
    if stops.is_empty() { return [255, 255, 255]; }
    if stops.len() == 1 { return stops[0].color; }
    
    let t = t.clamp(0.0, 1.0);
    for i in 0..stops.len() - 1 {
        let s1 = &stops[i];
        let s2 = &stops[i + 1];
        if t >= s1.position && t <= s2.position {
            let range = s2.position - s1.position;
            let local_t = if range > 0.0 { (t - s1.position) / range } else { 0.0 };
            return lerp_rgb(s1.color, s2.color, local_t);
        }
    }
    stops.last().unwrap().color
}

/// Cyclically interpolate through a list of colors based on a phase (0.0 to 1.0).
pub fn cyclic_color(colors: &[[u8; 3]], phase: f32) -> [u8; 3] {
    if colors.is_empty() { return [255, 255, 255]; }
    if colors.len() == 1 { return colors[0]; }
    
    let n = colors.len() as f32;
    let scaled = phase.fract() * n;
    let idx = scaled.floor() as usize;
    let t = scaled - idx as f32;
    
    let a = colors[idx % colors.len()];
    let b = colors[(idx + 1) % colors.len()];
    lerp_rgb(a, b, t)
}
