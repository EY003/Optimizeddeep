use serde::{Deserialize, Serialize};

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
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            bar_width: 9.0,
            bar_gap: 1.0,
            bar_radius: 2.0,
            sensitivity: 33.0, // Matches your screenshot
            fft_size: 1024,    // Matches your screenshot
            num_bars: 86,      // Matches the Rainmeter theme
            dynamic_colors: true,
            text_border: false,
            font_path: None,
        }
    }
}
