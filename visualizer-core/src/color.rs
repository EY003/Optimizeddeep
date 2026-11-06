use auto_palette::{ImageData, Palette};
use image::DynamicImage;

use crate::config::PalettePriority;

pub struct TrackColors {
    pub dominant: [u8; 3],
    pub text_color: [u8; 3],
}

/// Extract dominant + text colour from a decoded image.
pub fn extract_colors(img: &DynamicImage) -> Option<TrackColors> {
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

pub fn extract_colors_from_bytes(bytes: &[u8]) -> Option<(TrackColors, DynamicImage)> {
    let img = image::load_from_memory(bytes).ok()?;
    let colors = extract_colors(&img)?;
    Some((colors, img))
}

// ─────────────────────────────────────────────────────────────
// Dynamic palette extraction
// ─────────────────────────────────────────────────────────────

/// Build a palette of `palette_size * (1 + variants_per_color)` colours from
/// the cover art, using the given priority filter.
///
/// If the cover yields fewer distinct colours than requested, shade variants
/// of the colours we *did* find are appended until the requested count is met.
pub fn extract_palette(
    img: &DynamicImage,
    palette_size: u8,
    variants_per_color: u8,
    priority: PalettePriority,
) -> Vec<[u8; 3]> {
    let size = palette_size.max(1) as usize;
    let variants = variants_per_color as usize;

    let mut bases = extract_base_colors(img, size, priority)
        .unwrap_or_else(|| vec![[255, 255, 255]]);
    if bases.is_empty() {
        bases.push([255, 255, 255]);
    }

    // Pad to `size` with shade variants of existing base colours if needed.
    let mut pad_idx = 0u8;
    while bases.len() < size {
        let src = bases[(pad_idx as usize) % bases.len()];
        pad_idx += 1;
        bases.push(variant_of(src, pad_idx, (size as u8) + 2));
    }
    bases.truncate(size);

    // Expand each base colour into base + N variants.
    let mut out = Vec::with_capacity(size * (1 + variants));
    for base in bases {
        out.push(base);
        for v in 1..=variants {
            out.push(variant_of(base, v as u8, variants as u8 + 1));
        }
    }
    out
}

fn extract_base_colors(
    img: &DynamicImage,
    count: usize,
    priority: PalettePriority,
) -> Option<Vec<[u8; 3]>> {
    // Smaller thumbnail than dominant extraction — palette only needs
    // approximate clusters and this is called on every config change.
    let small = img.thumbnail(64, 64);
    let rgb = small.to_rgba8();
    let (w, h) = (rgb.width(), rgb.height());
    let raw_pixels = rgb.into_raw();

    let image_data = ImageData::new(w, h, &raw_pixels).ok()?;
    let palette: Palette<f64> = Palette::extract(&image_data).ok()?;

    let all: Vec<[u8; 3]> = palette
        .find_swatches((count * 4).max(count))
        .iter()
        .map(|swatch| {
            let rgb = swatch.color().to_rgb();
            [rgb.r, rgb.g, rgb.b]
        })
        .collect();

    if all.is_empty() {
        return None;
    }

    let filtered: Vec<[u8; 3]> = match priority {
        PalettePriority::MostCommon => all.clone(),
        PalettePriority::MostCommonLight => {
            let f: Vec<_> = all.iter().copied().filter(|c| is_light(*c)).collect();
            if f.is_empty() { vec![brightest(&all)] } else { f }
        }
        PalettePriority::Vivid => {
            let f: Vec<_> = all.iter().copied().filter(|c| is_vivid(*c)).collect();
            if f.is_empty() { vec![most_vivid(&all)] } else { f }
        }
    };

    // Deduplicate visually-similar colours.
    let mut deduped: Vec<[u8; 3]> = Vec::new();
    for c in filtered {
        if !deduped.iter().any(|e| color_distance(*e, c) < 30) {
            deduped.push(c);
        }
        if deduped.len() >= count {
            break;
        }
    }

    Some(deduped)
}

fn color_distance(a: [u8; 3], b: [u8; 3]) -> u32 {
    let dr = a[0] as i32 - b[0] as i32;
    let dg = a[1] as i32 - b[1] as i32;
    let db = a[2] as i32 - b[2] as i32;
    ((dr * dr + dg * dg + db * db) as f32).sqrt() as u32
}

fn is_light(c: [u8; 3]) -> bool {
    let lum = 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32;
    lum > 100.0
}

fn is_vivid(c: [u8; 3]) -> bool {
    let max = c[0].max(c[1]).max(c[2]) as f32;
    let min = c[0].min(c[1]).min(c[2]) as f32;
    let sat = if max > 0.0 { (max - min) / max } else { 0.0 };
    let lum = (0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32) / 255.0;
    sat > 0.5 || (lum > 0.6 && sat > 0.25)
}

fn brightest(colors: &[[u8; 3]]) -> [u8; 3] {
    colors
        .iter()
        .copied()
        .max_by(|a, b| {
            let la = 0.299 * a[0] as f32 + 0.587 * a[1] as f32 + 0.114 * a[2] as f32;
            let lb = 0.299 * b[0] as f32 + 0.587 * b[1] as f32 + 0.114 * b[2] as f32;
            la.partial_cmp(&lb).unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or([255, 255, 255])
}

fn most_vivid(colors: &[[u8; 3]]) -> [u8; 3] {
    colors
        .iter()
        .copied()
        .max_by(|a, b| {
            vivid_score(*a)
                .partial_cmp(&vivid_score(*b))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or([255, 255, 255])
}
fn vivid_score(c: [u8; 3]) -> f32 {
    let max = c[0].max(c[1]).max(c[2]) as f32;
    let min = c[0].min(c[1]).min(c[2]) as f32;
    let sat = if max > 0.0 { (max - min) / max } else { 0.0 };
    sat * (max / 255.0)
}

// ─────────────────────────────────────────────────────────────
// Variant generation — shifts HSL lightness in alternating directions,
// with a small hue shift so even pure black/white produce distinct shades.
// ─────────────────────────────────────────────────────────────

fn variant_of(base: [u8; 3], index: u8, total: u8) -> [u8; 3] {
    if index == 0 || total <= 1 {
        return base;
    }
    let (h, s, l) = rgb_to_hsl(base);
    let step = index as f32 / (total as f32 + 1.0);
    let direction = if index % 2 == 1 { -1.0 } else { 1.0 };
    let magnitude = 0.15 + step * 0.35;

    let mut delta_l = if direction < 0.0 {
        -magnitude * l
    } else {
        magnitude * (1.0 - l)
    };
    let mut new_l = l + delta_l;

    // If the requested direction has no room (e.g. lighter on pure white,
    // darker on pure black), flip direction instead of returning a copy.
    if delta_l.abs() < 0.05 {
        let alt = -direction;
        delta_l = if alt < 0.0 {
            -magnitude * l
        } else {
            magnitude * (1.0 - l)
        };
        new_l = l + delta_l;
    }
    new_l = new_l.clamp(0.02, 0.98);

    let hue_shift = direction * step * 20.0;
    let new_h = (h + hue_shift).rem_euclid(360.0);
    let new_s = (s + 0.1).min(1.0);

    hsl_to_rgb(new_h, new_s, new_l)
}

fn rgb_to_hsl(rgb: [u8; 3]) -> (f32, f32, f32) {
    let r = rgb[0] as f32 / 255.0;
    let g = rgb[1] as f32 / 255.0;
    let b = rgb[2] as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    let s = if d.abs() < 1e-9 {
        0.0
    } else {
        d / (1.0 - (2.0 * l - 1.0).abs()).max(1e-9)
    };
    let h = if d.abs() < 1e-9 {
        0.0
    } else if (max - r).abs() < 1e-9 {
        60.0 * (((g - b) / d) % 6.0)
    } else if (max - g).abs() < 1e-9 {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h.rem_euclid(360.0), s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [u8; 3] {
    let h = h.rem_euclid(360.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h2 = h / 60.0;
    let x = c * (1.0 - (h2 % 2.0 - 1.0).abs());
    let (r1, g1, b1) = if h2 < 1.0 {
        (c, x, 0.0)
    } else if h2 < 2.0 {
        (x, c, 0.0)
    } else if h2 < 3.0 {
        (0.0, c, x)
    } else if h2 < 4.0 {
        (0.0, x, c)
    } else if h2 < 5.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    let m = l - c / 2.0;
    [
        ((r1 + m) * 255.0).clamp(0.0, 255.0) as u8,
        ((g1 + m) * 255.0).clamp(0.0, 255.0) as u8,
        ((b1 + m) * 255.0).clamp(0.0, 255.0) as u8,
    ]
}

// ── Shared helpers (unchanged) ───────────────────────────────

pub fn lerp_rgb(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        (a[0] as f32 + (b[0] as f32 - a[0] as f32) * t) as u8,
        (a[1] as f32 + (b[1] as f32 - a[1] as f32) * t) as u8,
        (a[2] as f32 + (b[2] as f32 - a[2] as f32) * t) as u8,
    ]
}

pub fn sample_gradient(stops: &[crate::config::GradientStop], t: f32) -> [u8; 3] {
    if stops.is_empty() {
        return [255, 255, 255];
    }
    if stops.len() == 1 {
        return stops[0].color;
    }

    let t = t.clamp(0.0, 1.0);
    for i in 0..stops.len() - 1 {
        let s1 = &stops[i];
        let s2 = &stops[i + 1];
        if t >= s1.position && t <= s2.position {
            let range = s2.position - s1.position;
            let local_t = if range > 0.0 {
                (t - s1.position) / range
            } else {
                0.0
            };
            return lerp_rgb(s1.color, s2.color, local_t);
        }
    }
    stops.last().unwrap().color
}

pub fn cyclic_color(colors: &[[u8; 3]], phase: f32) -> [u8; 3] {
    if colors.is_empty() {
        return [255, 255, 255];
    }
    if colors.len() == 1 {
        return colors[0];
    }

    let n = colors.len() as f32;
    let scaled = phase.fract() * n;
    let idx = scaled.floor() as usize;
    let t = scaled - idx as f32;

    let a = colors[idx % colors.len()];
    let b = colors[(idx + 1) % colors.len()];
    lerp_rgb(a, b, t)
}
