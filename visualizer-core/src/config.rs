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
    Dynamic,
    StaticSingle,
    StaticMultiple,
    Gradient,
    ColorChanging,
}

impl Default for ColorMode {
    fn default() -> Self {
        ColorMode::Dynamic
    }
}

// Renamed and expanded distribution modes
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum StaticDistributionMode {
    Blocks,     // Evenly divided blocks across the bars
    Sequential, // Cycles through colors bar by bar (1,2,3,1,2,3...)
    Random,     // Randomly assigned to each bar
    Vertical,   // Stacked inside each bar
}

impl Default for StaticDistributionMode {
    fn default() -> Self {
        StaticDistributionMode::Blocks
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GradientStop {
    pub color: [u8; 3],
    pub position: f32, // 0.0 to 1.0
}

#[derive(Serialize, Deserialize, Clone)]
pub struct StaticMultiPreset {
    pub name: String,
    pub colors: Vec<[u8; 3]>,
    pub mode: StaticDistributionMode,
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

    pub bar_color: [u8; 3],
    pub bg_opacity: u8,
    pub color_presets: Vec<[u8; 3]>,
    pub x_offset: f32,
    pub y_offset: f32,

    pub max_bar_height: f32,
    pub target_fps: f32,
    pub show_fps: bool,
    pub audio_update_ms: u64,
    pub use_vsync: bool,

    #[serde(default)]
    pub visualizer_direction: VisualizerDirection,
    #[serde(default)]
    pub frequency_direction: FrequencyDirection,

    // ── NEW FIELDS ──
    #[serde(default)]
    pub color_mode: ColorMode,

    #[serde(default = "default_static_colors")]
    pub static_colors: Vec<[u8; 3]>,
    #[serde(default)]
    pub static_mode: StaticDistributionMode,
    #[serde(default)]
    pub static_multi_presets: Vec<StaticMultiPreset>,

    #[serde(default = "default_gradient_stops")]
    pub gradient_stops: Vec<GradientStop>,
    #[serde(default)]
    pub gradient_angle: f32,
    #[serde(default)]
    pub gradient_presets: Vec<GradientPreset>,

    #[serde(default = "default_static_colors")]
    pub color_change_colors: Vec<[u8; 3]>,
    #[serde(default = "default_speed")]
    pub color_change_speed: f32,
    #[serde(default)]
    pub color_change_presets: Vec<ColorChangePreset>,
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

            bar_color: [0, 255, 255],
            bg_opacity: 0,
            color_presets: vec![
                [255, 255, 255],
                [0, 255, 255],
                [255, 0, 255],
                [255, 255, 0],
                [255, 0, 0],
                [0, 255, 0],
                [0, 0, 255],
            ],
            x_offset: 0.0,
            y_offset: 0.0,

            max_bar_height: 0.0,
            target_fps: 60.0,
            show_fps: false,
            audio_update_ms: 33,
            use_vsync: false,

            visualizer_direction: VisualizerDirection::default(),
            frequency_direction: FrequencyDirection::default(),

            color_mode: ColorMode::Dynamic,
            static_colors: default_static_colors(),
            static_mode: StaticDistributionMode::Blocks,
            static_multi_presets: vec![],

            gradient_stops: default_gradient_stops(),
            gradient_angle: 0.0,
            gradient_presets: vec![],

            color_change_colors: default_static_colors(),
            color_change_speed: default_speed(),
            color_change_presets: vec![],
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
            if let Ok(config) = toml::from_str(&content) {
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
