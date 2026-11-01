use auto_palette::{Palette, Swatch};
use image::DynamicImage;

pub struct TrackColors {
    pub dominant: [u8; 3],
    pub text_color: [u8; 3],
}

pub fn extract_colors(img: &DynamicImage) -> TrackColors {
    let (w, h) = (img.width(), img.height());
    let mut pixels = Vec::with_capacity((w * h) as usize);
    let rgb = img.to_rgb8();
    for pixel in rgb.pixels() {
        pixels.push([pixel[0], pixel[1], pixel[2]]);
    }

    let palette = Palette::from_pixels(&pixels).unwrap_or_default();
    let dominant = palette.swatches().first().map(|s| *s.rgb()).unwrap_or([255, 255, 255]);
    
    // Calculate contrasting text color based on luminance
    let luminance = 0.299 * dominant[0] as f32 + 0.587 * dominant[1] as f32 + 0.114 * dominant[2] as f32;
    let text_color = if luminance > 128.0 { [0, 0, 0] } else { [255, 255, 255] };

    TrackColors { dominant, text_color }
}
