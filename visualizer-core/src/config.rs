use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    
    // New settings
    pub bar_color: [u8; 3],
    pub bg_opacity: u8,
    pub color_presets: Vec<[u8; 3]>,
    pub x_offset: f32,
    pub y_offset: f32,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            bar_width: 9.0,
            bar_gap: 1.0,
            bar_radius: 2.0,
            sensitivity: 33.0,
            fft_size: 1024,
            num_bars: 86,
            dynamic_colors: true,
            text_border: false,
            font_path: None,
            
            bar_color: [180, 200, 180],
            bg_opacity: 0, // 0 = fully transparent
            color_presets: vec![],
            x_offset: 0.0,
            y_offset: 0.0,
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
