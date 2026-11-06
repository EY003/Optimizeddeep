use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum VisualizerDirection {
    Up,
    Down,
    Left,
    Right,
}

impl Default for VisualizerDirection {
    fn default() -> Self {
        VisualizerDirection::Up
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum FrequencyDirection {
    LeftToRight,
    RightToLeft,
    Mirrored,
}

impl Default for FrequencyDirection {
    fn default() -> Self {
        FrequencyDirection::LeftToRight
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorMode {
    /// Legacy variant — always migrated to `StaticSingle` + `Dynamic`
    /// source during `AppConfig::load`. Never shown in the UI.
    Dynamic,
    StaticSingle,
    StaticMultiple,
    Gradient,
    #[serde(alias = "ColorChanging")]
    Animated,
}

impl Default for ColorMode {
    fn default() -> Self {
        ColorMode::StaticSingle
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum StaticDistributionMode {
    Blocks,
    Sequential,
    Random,
    Vertical,
}

impl Default for StaticDistributionMode {
    fn default() -> Self {
        StaticDistributionMode::Blocks
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct GradientStop {
    pub color: [u8; 3],
    pub position: f32,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct StaticMultiPreset {
    pub name: String,
    pub colors: Vec<[u8; 3]>,
    pub mode: StaticDistributionMode,
    #[serde(default = "default_sequential_bars_per_color")]
    pub bars_per_color: usize,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct GradientPreset {
    pub name: String,
    pub stops: Vec<GradientStop>,
    pub angle: f32,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ColorChangePreset {
    pub name: String,
    pub colors: Vec<[u8; 3]>,
    pub speed: f32,
}

// ─────────────────────────────────────────────────────────────
// Palette selection — used by every dynamic colour source
// ─────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum PalettePriority {
    /// Raw most-frequent colours (current behaviour).
    MostCommon,
    /// Prefer colours with high luminance — useful for dark covers.
    MostCommonLight,
    /// Prefer saturated / bright colours that "pop" against the cover.
    Vivid,
}

impl Default for PalettePriority {
    fn default() -> Self {
        PalettePriority::MostCommon
    }
}

impl PalettePriority {
    pub fn display_name(self) -> &'static str {
        match self {
            PalettePriority::MostCommon => "Most Common",
            PalettePriority::MostCommonLight => "Most Common Light",
            PalettePriority::Vivid => "Vivid / High Contrast",
        }
    }
    pub fn all() -> [PalettePriority; 3] {
        [
            PalettePriority::MostCommon,
            PalettePriority::MostCommonLight,
            PalettePriority::Vivid,
        ]
    }
}

// ─────────────────────────────────────────────────────────────
// Colour sources — static or dynamic, per mode
// ─────────────────────────────────────────────────────────────

/// Colour source for modes whose colours are a plain list (StaticSingle,
/// StaticMultiple, Animated).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub enum ColorSource {
    Static(Vec<[u8; 3]>),
    Dynamic {
        #[serde(default = "default_palette_size")]
        palette_size: u8,
        #[serde(default)]
        variants_per_color: u8,
        #[serde(default)]
        priority: PalettePriority,
    },
}

impl Default for ColorSource {
    fn default() -> Self {
        ColorSource::Static(default_static_colors())
    }
}

impl ColorSource {
    pub fn is_dynamic(&self) -> bool {
        matches!(self, ColorSource::Dynamic { .. })
    }
    pub fn static_colors(&self) -> Option<&[[u8; 3]]> {
        match self {
            ColorSource::Static(v) => Some(v.as_slice()),
            _ => None,
        }
    }
    pub fn static_colors_mut(&mut self) -> Option<&mut Vec<[u8; 3]>> {
        match self {
            ColorSource::Static(v) => Some(v),
            _ => None,
        }
    }
    pub fn dynamic_settings(&self) -> Option<(u8, u8, PalettePriority)> {
        match self {
            ColorSource::Dynamic {
                palette_size,
                variants_per_color,
                priority,
            } => Some((*palette_size, *variants_per_color, *priority)),
            _ => None,
        }
    }
}

/// Colour source for Gradient mode — static carries stops (colour + position),
/// dynamic carries palette settings (positions are auto-assigned).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub enum GradientSource {
    Static(Vec<GradientStop>),
    Dynamic {
        #[serde(default = "default_palette_size")]
        palette_size: u8,
        #[serde(default)]
        variants_per_color: u8,
        #[serde(default)]
        priority: PalettePriority,
    },
}

impl Default for GradientSource {
    fn default() -> Self {
        GradientSource::Static(default_gradient_stops())
    }
}

impl GradientSource {
    pub fn is_dynamic(&self) -> bool {
        matches!(self, GradientSource::Dynamic { .. })
    }
    pub fn static_stops(&self) -> Option<&[GradientStop]> {
        match self {
            GradientSource::Static(s) => Some(s.as_slice()),
            _ => None,
        }
    }
    pub fn static_stops_mut(&mut self) -> Option<&mut Vec<GradientStop>> {
        match self {
            GradientSource::Static(s) => Some(s),
            _ => None,
        }
    }
    pub fn dynamic_settings(&self) -> Option<(u8, u8, PalettePriority)> {
        match self {
            GradientSource::Dynamic {
                palette_size,
                variants_per_color,
                priority,
            } => Some((*palette_size, *variants_per_color, *priority)),
            _ => None,
        }
    }
}

fn default_palette_size() -> u8 {
    1
}

// ─────────────────────────────────────────────────────────────
// Color Changing: Motion Shapes  (unchanged)
// ─────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum MotionShape {
    Uniform,
    HorizontalWipe,
    VerticalWipe,
    DiagonalWipe,
    SineWave,
    TriangleWave,
    SquareWave,
    SawtoothWave,
    Strobe,
    Pulse,
    Heartbeat,
    Stutter,
    Comet,
    Chase,
    BouncingBall,
    Sparkle,
    FireSimulation,
    Plasma,
    RainbowCycle,
    Confetti,
    Sinelon,
}

impl Default for MotionShape {
    fn default() -> Self {
        MotionShape::Uniform
    }
}

impl MotionShape {
    pub fn display_name(self) -> &'static str {
        match self {
            MotionShape::Uniform => "Uniform",
            MotionShape::HorizontalWipe => "Horizontal Wipe",
            MotionShape::VerticalWipe => "Vertical Wipe",
            MotionShape::DiagonalWipe => "Diagonal Wipe",
            MotionShape::SineWave => "Sine Wave",
            MotionShape::TriangleWave => "Triangle Wave",
            MotionShape::SquareWave => "Square Wave",
            MotionShape::SawtoothWave => "Sawtooth Wave",
            MotionShape::Strobe => "Strobe",
            MotionShape::Pulse => "Pulse",
            MotionShape::Heartbeat => "Heartbeat",
            MotionShape::Stutter => "Stutter",
            MotionShape::Comet => "Comet",
            MotionShape::Chase => "Chase",
            MotionShape::BouncingBall => "Bouncing Ball",
            MotionShape::Sparkle => "Sparkle",
            MotionShape::FireSimulation => "Fire Simulation",
            MotionShape::Plasma => "Plasma",
            MotionShape::RainbowCycle => "Rainbow Cycle",
            MotionShape::Confetti => "Confetti",
            MotionShape::Sinelon => "Sinelon",
        }
    }

    pub fn uses_vertical_axis(self) -> bool {
        matches!(
            self,
            MotionShape::VerticalWipe | MotionShape::DiagonalWipe
        )
    }
    pub fn uses_intensity(self) -> bool {
        matches!(
            self,
            MotionShape::Strobe
                | MotionShape::Pulse
                | MotionShape::Heartbeat
                | MotionShape::Comet
                | MotionShape::Chase
                | MotionShape::BouncingBall
                | MotionShape::Sparkle
                | MotionShape::Sinelon
                | MotionShape::FireSimulation
                | MotionShape::Confetti
        )
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlendMode {
    Smooth,
    Hard,
}

impl Default for BlendMode {
    fn default() -> Self {
        BlendMode::Smooth
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MotionSettings {
    pub shape: MotionShape,
    pub blend: BlendMode,
    pub speed: f32,
    pub trail_length: f32,
    pub stutter_intensity: f32,
    pub wave_frequency: f32,
    pub wipe_softness: f32,
    #[serde(default)]
    pub off_color: [u8; 3],
}

impl Default for MotionSettings {
    fn default() -> Self {
        Self {
            shape: MotionShape::Uniform,
            blend: BlendMode::Smooth,
            speed: 1.0,
            trail_length: 0.25,
            stutter_intensity: 0.3,
            wave_frequency: 2.0,
            wipe_softness: 0.1,
            off_color: [0, 0, 0],
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Track Info (unchanged)
// ─────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrackTextColorMode {
    Static,
    Visualizer,
}

impl Default for TrackTextColorMode {
    fn default() -> Self {
        TrackTextColorMode::Static
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrackTextAlign {
    Left,
    Center,
    Right,
}

impl Default for TrackTextAlign {
    fn default() -> Self {
        TrackTextAlign::Right
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TrackInfoConfig {
    pub show_cover_art: bool,
    pub cover_size: f32,
    pub cover_x_offset: f32,
    pub cover_y_offset: f32,

    pub show_title: bool,
    pub show_artist: bool,

    pub title_font_size: f32,
    pub artist_font_size: f32,

    #[serde(default, alias = "custom_font_path")]
    pub font_path: Option<String>,

    pub color_mode: TrackTextColorMode,
    pub static_color: [u8; 3],
    pub opacity: u8,

    pub letter_spacing: f32,
    pub line_height: f32,
    pub alignment: TrackTextAlign,
    pub text_x_offset: f32,
    pub text_y_offset: f32,

    pub shadow_enabled: bool,
    pub shadow_offset_x: f32,
    pub shadow_offset_y: f32,
    pub shadow_blur: f32,
    pub shadow_color: [u8; 3],

    pub outline_enabled: bool,
    pub outline_thickness: f32,
    pub outline_color: [u8; 3],
}

impl Default for TrackInfoConfig {
    fn default() -> Self {
        Self {
            show_cover_art: true,
            cover_size: 190.0,
            cover_x_offset: 0.0,
            cover_y_offset: 0.0,

            show_title: true,
            show_artist: true,

            title_font_size: 21.0,
            artist_font_size: 30.0,

            font_path: None,

            color_mode: TrackTextColorMode::Static,
            static_color: [220, 220, 220],
            opacity: 255,

            letter_spacing: 0.0,
            line_height: 1.3,
            alignment: TrackTextAlign::Right,
            text_x_offset: 0.0,
            text_y_offset: 0.0,

            shadow_enabled: true,
            shadow_offset_x: 2.0,
            shadow_offset_y: 2.0,
            shadow_blur: 0.0,
            shadow_color: [0, 0, 0],

            outline_enabled: false,
            outline_thickness: 1.0,
            outline_color: [0, 0, 0],
        }
    }
}

// ─────────────────────────────────────────────────────────────
// AppConfig
// ─────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
pub struct AppConfig {
    pub bar_width: f32,
    pub bar_gap: f32,
    pub bar_radius: f32,
    pub sensitivity: f32,
    pub fft_size: usize,
    pub num_bars: usize,
    pub dynamic_colors: bool,
    pub text_border: bool,
    pub font_path: Option<String>,

    pub bg_opacity: u8,
    /// RGB colour drawn behind the bars when `bg_opacity > 0`.
    #[serde(default)]
    pub bg_color: [u8; 3],
    pub color_presets: Vec<[u8; 3]>,
    pub x_offset: f32,
    pub y_offset: f32,

    // ── Bar outlines ──
    #[serde(default)]
    pub bar_outline_enabled: bool,
    #[serde(default = "default_bar_outline_thickness")]
    pub bar_outline_thickness: f32,
    #[serde(default = "default_bar_outline_color")]
    pub bar_outline_color: [u8; 3],

    pub max_bar_height: f32,
    pub target_fps: f32,
    pub show_fps: bool,
    pub audio_update_ms: u64,
    pub use_vsync: bool,

    #[serde(default)]
    pub visualizer_direction: VisualizerDirection,
    #[serde(default)]
    pub frequency_direction: FrequencyDirection,

    // ── Color Mode ──
    #[serde(default)]
    pub color_mode: ColorMode,

    // ── Static Single ──
    #[serde(default = "default_static_single_source")]
    pub static_single_source: ColorSource,

    // ── Static Multiple ──
    #[serde(default = "default_static_multi_source")]
    pub static_multi_source: ColorSource,
    #[serde(default)]
    pub static_mode: StaticDistributionMode,
    #[serde(default = "default_sequential_bars_per_color")]
    pub sequential_bars_per_color: usize,
    #[serde(default = "default_static_multi_presets")]
    pub static_multi_presets: Vec<StaticMultiPreset>,

    // ── Gradient ──
    #[serde(default = "default_gradient_source")]
    pub gradient_source: GradientSource,
    #[serde(default)]
    pub gradient_angle: f32,
    #[serde(default = "default_gradient_presets")]
    pub gradient_presets: Vec<GradientPreset>,

    // ── Animated ──
    #[serde(default = "default_animated_source")]
    pub animated_source: ColorSource,
    #[serde(default = "default_speed")]
    pub color_change_speed: f32,
    #[serde(default = "default_color_change_presets")]
    pub color_change_presets: Vec<ColorChangePreset>,
    #[serde(default)]
    pub color_change_motion: MotionSettings,

    // ── Cover Art / Song Info Text ──
    #[serde(default)]
    pub track_info: TrackInfoConfig,

    // ── Legacy fields (migrated on load; never re-serialised) ──
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bar_color: Option<[u8; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub static_colors: Option<Vec<[u8; 3]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient_stops: Option<Vec<GradientStop>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_change_colors: Option<Vec<[u8; 3]>>,
}

fn default_static_colors() -> Vec<[u8; 3]> {
    vec![[255, 0, 0], [0, 255, 0], [0, 0, 255]]
}
fn default_gradient_stops() -> Vec<GradientStop> {
    vec![
        GradientStop { color: [255, 0, 0], position: 0.0 },
        GradientStop { color: [0, 0, 255], position: 1.0 },
    ]
}
fn default_speed() -> f32 {
    0.5
}
fn default_sequential_bars_per_color() -> usize {
    1
}

fn default_bar_outline_thickness() -> f32 {
    1.0
}
fn default_bar_outline_color() -> [u8; 3] {
    [0, 0, 0]
}

fn default_static_single_source() -> ColorSource {
    // Matches the old "Dynamic" out-of-the-box behaviour.
    ColorSource::Dynamic {
        palette_size: 1,
        variants_per_color: 0,
        priority: PalettePriority::MostCommon,
    }
}
fn default_static_multi_source() -> ColorSource {
    ColorSource::Static(default_static_colors())
}
fn default_animated_source() -> ColorSource {
    ColorSource::Static(default_static_colors())
}
fn default_gradient_source() -> GradientSource {
    GradientSource::Static(default_gradient_stops())
}

// ─────────────────────────────────────────────────────────────
// Default presets (unchanged)
// ─────────────────────────────────────────────────────────────

fn default_static_multi_presets() -> Vec<StaticMultiPreset> {
    vec![
        StaticMultiPreset {
            name: "Sunset Boulevard".into(),
            colors: vec![[255, 94, 0], [255, 0, 128], [128, 0, 255]],
            mode: StaticDistributionMode::Blocks,
            bars_per_color: 1,
        },
        StaticMultiPreset {
            name: "Deep Ocean".into(),
            colors: vec![[0, 30, 60], [0, 100, 160], [0, 180, 216], [144, 224, 239]],
            mode: StaticDistributionMode::Sequential,
            bars_per_color: 2,
        },
        StaticMultiPreset {
            name: "Enchanted Forest".into(),
            colors: vec![[20, 80, 20], [34, 139, 34], [50, 205, 50], [144, 238, 144]],
            mode: StaticDistributionMode::Random,
            bars_per_color: 1,
        },
        StaticMultiPreset {
            name: "Cyberpunk Neon".into(),
            colors: vec![[255, 0, 255], [0, 255, 255], [255, 255, 0]],
            mode: StaticDistributionMode::Vertical,
            bars_per_color: 1,
        },
        StaticMultiPreset {
            name: "Pastel Dream".into(),
            colors: vec![[255, 182, 193], [173, 216, 230], [144, 238, 144], [255, 255, 224]],
            mode: StaticDistributionMode::Blocks,
            bars_per_color: 1,
        },
        StaticMultiPreset {
            name: "Fire & Ice".into(),
            colors: vec![[255, 50, 0], [255, 150, 0], [100, 200, 255], [0, 150, 255]],
            mode: StaticDistributionMode::Vertical,
            bars_per_color: 1,
        },
        StaticMultiPreset {
            name: "Vaporwave".into(),
            colors: vec![[255, 113, 206], [1, 205, 254], [5, 255, 161], [185, 103, 255]],
            mode: StaticDistributionMode::Sequential,
            bars_per_color: 1,
        },
        StaticMultiPreset {
            name: "Candy Shop".into(),
            colors: vec![[255, 105, 180], [255, 215, 0], [0, 255, 255]],
            mode: StaticDistributionMode::Sequential,
            bars_per_color: 3,
        },
        StaticMultiPreset {
            name: "Lava Lamp".into(),
            colors: vec![[255, 0, 0], [255, 100, 0], [150, 0, 200], [80, 0, 150]],
            mode: StaticDistributionMode::Random,
            bars_per_color: 1,
        },
        StaticMultiPreset {
            name: "Monochrome".into(),
            colors: vec![[255, 255, 255], [150, 150, 150], [80, 80, 80]],
            mode: StaticDistributionMode::Sequential,
            bars_per_color: 2,
        },
        StaticMultiPreset {
            name: "Rainbow Blocks".into(),
            colors: vec![
                [255, 0, 0], [255, 127, 0], [255, 255, 0],
                [0, 255, 0], [0, 0, 255], [75, 0, 130], [148, 0, 211],
            ],
            mode: StaticDistributionMode::Blocks,
            bars_per_color: 1,
        },
        StaticMultiPreset {
            name: "Tropical".into(),
            colors: vec![[0, 200, 150], [255, 200, 0], [255, 80, 80]],
            mode: StaticDistributionMode::Random,
            bars_per_color: 1,
        },
    ]
}

fn default_gradient_presets() -> Vec<GradientPreset> {
    vec![
        GradientPreset {
            name: "Sunset".into(),
            stops: vec![
                GradientStop { color: [255, 94, 0], position: 0.0 },
                GradientStop { color: [255, 0, 128], position: 0.5 },
                GradientStop { color: [128, 0, 255], position: 1.0 },
            ],
            angle: 0.0,
        },
        GradientPreset {
            name: "Ocean Deep".into(),
            stops: vec![
                GradientStop { color: [0, 30, 60], position: 0.0 },
                GradientStop { color: [0, 180, 216], position: 0.5 },
                GradientStop { color: [144, 224, 239], position: 1.0 },
            ],
            angle: 90.0,
        },
        GradientPreset {
            name: "Rainbow".into(),
            stops: vec![
                GradientStop { color: [255, 0, 0], position: 0.0 },
                GradientStop { color: [255, 127, 0], position: 0.17 },
                GradientStop { color: [255, 255, 0], position: 0.33 },
                GradientStop { color: [0, 255, 0], position: 0.5 },
                GradientStop { color: [0, 0, 255], position: 0.67 },
                GradientStop { color: [75, 0, 130], position: 0.83 },
                GradientStop { color: [148, 0, 211], position: 1.0 },
            ],
            angle: 45.0,
        },
        GradientPreset {
            name: "Fire".into(),
            stops: vec![
                GradientStop { color: [255, 0, 0], position: 0.0 },
                GradientStop { color: [255, 165, 0], position: 0.5 },
                GradientStop { color: [255, 255, 0], position: 1.0 },
            ],
            angle: 90.0,
        },
        GradientPreset {
            name: "Aurora".into(),
            stops: vec![
                GradientStop { color: [0, 255, 150], position: 0.0 },
                GradientStop { color: [0, 150, 255], position: 0.5 },
                GradientStop { color: [150, 0, 255], position: 1.0 },
            ],
            angle: 90.0,
        },
        GradientPreset {
            name: "Cotton Candy".into(),
            stops: vec![
                GradientStop { color: [255, 182, 193], position: 0.0 },
                GradientStop { color: [173, 216, 230], position: 0.5 },
                GradientStop { color: [255, 105, 180], position: 1.0 },
            ],
            angle: 0.0,
        },
        GradientPreset {
            name: "Gold Rush".into(),
            stops: vec![
                GradientStop { color: [80, 40, 0], position: 0.0 },
                GradientStop { color: [255, 180, 0], position: 0.5 },
                GradientStop { color: [255, 255, 150], position: 1.0 },
            ],
            angle: 90.0,
        },
        GradientPreset {
            name: "Neon Nights".into(),
            stops: vec![
                GradientStop { color: [255, 0, 150], position: 0.0 },
                GradientStop { color: [100, 0, 255], position: 0.5 },
                GradientStop { color: [0, 255, 255], position: 1.0 },
            ],
            angle: 45.0,
        },
        GradientPreset {
            name: "Enchanted Forest".into(),
            stops: vec![
                GradientStop { color: [0, 50, 0], position: 0.0 },
                GradientStop { color: [50, 150, 50], position: 0.5 },
                GradientStop { color: [200, 255, 150], position: 1.0 },
            ],
            angle: 90.0,
        },
        GradientPreset {
            name: "Grayscale".into(),
            stops: vec![
                GradientStop { color: [0, 0, 0], position: 0.0 },
                GradientStop { color: [255, 255, 255], position: 1.0 },
            ],
            angle: 90.0,
        },
        GradientPreset {
            name: "Twilight".into(),
            stops: vec![
                GradientStop { color: [25, 25, 80], position: 0.0 },
                GradientStop { color: [150, 50, 150], position: 0.5 },
                GradientStop { color: [255, 150, 100], position: 1.0 },
            ],
            angle: 90.0,
        },
        GradientPreset {
            name: "Ice".into(),
            stops: vec![
                GradientStop { color: [200, 240, 255], position: 0.0 },
                GradientStop { color: [100, 180, 255], position: 0.5 },
                GradientStop { color: [30, 100, 200], position: 1.0 },
            ],
            angle: 90.0,
        },
    ]
}

fn default_color_change_presets() -> Vec<ColorChangePreset> {
    vec![
        ColorChangePreset {
            name: "Rainbow".into(),
            colors: vec![
                [255, 0, 0], [255, 127, 0], [255, 255, 0],
                [0, 255, 0], [0, 0, 255], [75, 0, 130], [148, 0, 211],
            ],
            speed: 0.5,
        },
        ColorChangePreset {
            name: "Pastel Cycle".into(),
            colors: vec![[255, 182, 193], [173, 216, 230], [144, 238, 144], [255, 255, 224]],
            speed: 0.2,
        },
        ColorChangePreset {
            name: "Neon Pulse".into(),
            colors: vec![[255, 0, 255], [0, 255, 255], [255, 255, 0]],
            speed: 0.7,
        },
        ColorChangePreset {
            name: "Warm Fire".into(),
            colors: vec![[255, 50, 0], [255, 150, 0], [255, 220, 0]],
            speed: 0.4,
        },
        ColorChangePreset {
            name: "Ocean Waves".into(),
            colors: vec![[0, 50, 150], [0, 150, 220], [100, 220, 255]],
            speed: 0.3,
        },
        ColorChangePreset {
            name: "Traffic".into(),
            colors: vec![[255, 0, 0], [255, 220, 0], [0, 200, 0]],
            speed: 0.5,
        },
        ColorChangePreset {
            name: "Cotton Candy".into(),
            colors: vec![[255, 105, 180], [255, 182, 193], [200, 150, 255]],
            speed: 0.2,
        },
        ColorChangePreset {
            name: "Cyberpunk".into(),
            colors: vec![[255, 0, 255], [0, 255, 255], [255, 255, 0]],
            speed: 0.6,
        },
    ]
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            bar_width: 9.0,
            bar_gap: 1.0,
            bar_radius: 2.0,
            sensitivity: 10.0,
            fft_size: 1024,
            num_bars: 86,
            dynamic_colors: true,
            text_border: false,
            font_path: None,

            bg_opacity: 0,
            bg_color: [0, 0, 0],
            color_presets: vec![
                [255, 255, 255],
                [0, 255, 255],
                [255, 0, 255],
                [255, 255, 0],
                [255, 0, 0],
                [0, 255, 0],
                [0, 0, 255],
                [255, 165, 0],
                [128, 0, 128],
                [0, 128, 128],
            ],
            x_offset: 0.0,
            y_offset: 0.0,

            bar_outline_enabled: false,
            bar_outline_thickness: 1.0,
            bar_outline_color: [0, 0, 0],

            max_bar_height: 0.0,
            target_fps: 60.0,
            show_fps: false,
            audio_update_ms: 33,
            use_vsync: false,

            visualizer_direction: VisualizerDirection::default(),
            frequency_direction: FrequencyDirection::default(),

            color_mode: ColorMode::StaticSingle,

            static_single_source: default_static_single_source(),
            static_multi_source: default_static_multi_source(),
            static_mode: StaticDistributionMode::Blocks,
            sequential_bars_per_color: default_sequential_bars_per_color(),
            static_multi_presets: default_static_multi_presets(),

            gradient_source: default_gradient_source(),
            gradient_angle: 0.0,
            gradient_presets: default_gradient_presets(),

            animated_source: default_animated_source(),
            color_change_speed: default_speed(),
            color_change_presets: default_color_change_presets(),
            color_change_motion: MotionSettings::default(),

            track_info: TrackInfoConfig::default(),

            bar_color: None,
            static_colors: None,
            gradient_stops: None,
            color_change_colors: None,
        }
    }
}

impl AppConfig {
    pub fn get_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("RustVisualizer")
            .join("config.toml")
    }

    pub fn load() -> Self {
        let path = Self::get_path();
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(mut config) = toml::from_str::<AppConfig>(&content) {
                // ── Legacy field migration ──
                if let Some(c) = config.bar_color.take() {
                    config.static_single_source = ColorSource::Static(vec![c]);
                }
                if let Some(c) = config.static_colors.take() {
                    config.static_multi_source = ColorSource::Static(c);
                }
                if let Some(s) = config.gradient_stops.take() {
                    config.gradient_source = GradientSource::Static(s);
                }
                if let Some(c) = config.color_change_colors.take() {
                    config.animated_source = ColorSource::Static(c);
                }
                // ── Legacy Dynamic mode → StaticSingle + Dynamic source ──
                if config.color_mode == ColorMode::Dynamic {
                    config.color_mode = ColorMode::StaticSingle;
                    config.static_single_source = ColorSource::Dynamic {
                        palette_size: 1,
                        variants_per_color: 0,
                        priority: PalettePriority::MostCommon,
                    };
                }

                if config.static_multi_presets.is_empty() {
                    config.static_multi_presets = default_static_multi_presets();
                }
                if config.gradient_presets.is_empty() {
                    config.gradient_presets = default_gradient_presets();
                }
                if config.color_change_presets.is_empty() {
                    config.color_change_presets = default_color_change_presets();
                }
                return config;
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        let path = Self::get_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(toml) = toml::to_string(self) {
            let _ = std::fs::write(path, toml);
        }
    }
}
