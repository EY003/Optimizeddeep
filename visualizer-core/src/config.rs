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
    /// Low frequencies on the left, high on the right (default).
    LeftToRight,
    /// High frequencies on the left, low on the right.
    RightToLeft,
    /// Low -> high on the first half, high -> low on the second half.
    Mirrored,
}

impl Default for FrequencyDirection {
    fn default() -> Self {
        FrequencyDirection::LeftToRight
    }
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

    // ✅ NEW: VSync Option
    pub use_vsync: bool,

    // ✅ NEW: Bar orientation (Up/Down/Left/Right)
    // `#[serde(default)]` keeps older config files loadable.
    #[serde(default)]
    pub visualizer_direction: VisualizerDirection,

    // ✅ NEW: Frequency ordering (LeftToRight / RightToLeft / Mirrored)
    #[serde(default)]
    pub frequency_direction: FrequencyDirection,
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
            use_vsync: false, // Default to false so manual FPS limiter works

            visualizer_direction: VisualizerDirection::default(),
            frequency_direction: FrequencyDirection::default(),
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
