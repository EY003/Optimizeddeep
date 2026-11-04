use auto_palette::{ImageData, Palette};
use image::DynamicImage;

pub struct TrackColors {
    pub dominant: [u8; 3],
    pub text_color: [u8; 3],
}

/// Extract colors from a decoded image. Returns `None` if the palette
/// extraction fails (e.g. on a 0×0 image) instead of panicking.
pub fn extract_colors(img: &DynamicImage) -> Option<TrackColors> {
    // Downscale for speed — we only need enough pixels to sense the palette.
    let small = img.thumbnail(128, 128);
    let rgb = small.to_rgba8();
    let (w, h) = (rgb.width(), rgb.height());
    let raw_pixels = rgb.into_raw();

    let image_data = ImageData::new(w, h, &raw_pixels).ok()?;
    let palette: Palette<f64> = Palette::extract(&image_data).ok()?;

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

    Some(TrackColors { dominant, text_color })
}

/// Convenience helper: decode raw bytes and extract colors in one call.
pub fn extract_colors_from_bytes(bytes: &[u8]) -> Option<(TrackColors, DynamicImage)> {
    let img = image::load_from_memory(bytes).ok()?;
    let colors = extract_colors(&img)?;
    Some((colors, img))
}

// ── Color helpers (unchanged) ────────────────────────────────

pub fn lerp_rgb(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        (a[0] as f32 + (b[0] as f32 - a[0] as f32) * t) as u8,
        (a[1] as f32 + (b[1] as f32 - a[1] as f32) * t) as u8,
        (a[2] as f32 + (b[2] as f32 - a[2] as f32) * t) as u8,
    ]
}

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
