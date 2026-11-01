use auto_palette::{ImageData, Palette};
use image::DynamicImage;

pub struct TrackColors {
    pub dominant: [u8; 3],
    pub text_color: [u8; 3],
}

pub fn extract_colors(img: &DynamicImage) -> TrackColors {
    let rgb = img.to_rgba8(); // Use RGBA8 as ImageData expects 4 bytes per pixel
    let (w, h) = (rgb.width(), rgb.height());
    let raw_pixels = rgb.into_raw(); // Vec<u8> with R,G,B,A order

    // 1. Create ImageData from the raw pixel buffer
    let image_data = ImageData::new(w, h, &raw_pixels)
        .expect("Failed to create ImageData from pixels");

    // 2. Extract the palette
    let palette: Palette<f64> = Palette::extract(&image_data)
        .expect("Failed to extract color palette");

    // 3. Get the dominant color
    // Inside extract_colors, replace the swatch color extraction

let dominant = palette
    .find_swatches(1)
    .first()
    .map(|swatch| {
        let rgb = swatch.color().to_rgb();
        // Access fields directly
        [rgb.r, rgb.g, rgb.b]
    })
    .unwrap_or([255, 255, 255]);
    // 4. Calculate contrasting text color based on luminance
    let luminance = 0.299 * dominant[0] as f32
        + 0.587 * dominant[1] as f32
        + 0.114 * dominant[2] as f32;
    let text_color = if luminance > 128.0 { [0, 0, 0] } else { [255, 255, 255] };

    TrackColors {
        dominant,
        text_color,
    }
}
