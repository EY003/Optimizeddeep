use eframe::egui;
use std::sync::{Arc, Mutex};
use visualizer_core::{audio::AudioEngine, config::AppConfig};

struct VisualizerApp {
    config: AppConfig,
    show_settings: bool,
    // Audio & MBRC channels would be integrated here
}

impl eframe::App for VisualizerApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // 1. Render Overlay (Click-through)
        if !self.show_settings {
            // Setup transparent, click-through overlay
            let rect = ctx.screen_rect();
            egui::Window::new("Overlay")
                .title_bar(false)
                .resizable(false)
                .fixed_pos(rect.min)
                .fixed_size(rect.size())
                .frame(egui::Frame::none())
                .show(ctx, |ui| {
                    // Draw Bars here using ui.painter()
                    self.draw_bars(ui);
                    
                    // Draw Cover Art and Text (Top-Right corner)
                    self.draw_song_info(ui, rect);
                });
        }

        // 2. Render Settings Window
        if self.show_settings {
            egui::Window::new("Settings")
                .open(&mut self.show_settings)
                .show(ctx, |ui| {
                    self.draw_settings_ui(ui);
                });
        }

        // Keep the app running and requesting repaints (e.g., 60 FPS for visualizer)
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

impl VisualizerApp {
    fn draw_bars(&self, ui: &mut egui::Ui) {
        let painter = ui.painter();
        let rect = ui.max_rect();
        let bar_width = self.config.bar_width;
        let gap = self.config.bar_gap;
        // Mock bars for now - these would come from visualizer-core FFT
        for i in 0..86 {
            let height = (i as f32 * 5.0) % 200.0; // Mock data
            let x = rect.left() + (i as f32 * (bar_width + gap));
            let bar_rect = egui::Rect::from_min_max(
                egui::pos2(x, rect.bottom() - height),
                egui::pos2(x + bar_width, rect.bottom())
            );
            painter.rect_filled(bar_rect, self.config.bar_radius, egui::Color32::WHITE);
        }
    }

    fn draw_song_info(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        // Draw cover art and text in the top right corner
        // Match layout from your screenshots
    }

    fn draw_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("General");
        ui.add(egui::Slider::new(&mut self.config.bar_width, 1.0..=50.0).text("Bar width"));
        ui.add(egui::Slider::new(&mut self.config.bar_gap, 0.0..=20.0).text("Bar gap"));
        ui.add(egui::Slider::new(&mut self.config.sensitivity, 1.0..=100.0).text("Sensitivity"));
        ui.add(egui::Slider::new(&mut self.config.fft_size, 256..=4096).text("FFT Size").logarithmic(true));
        
        ui.separator();
        ui.heading("Style");
        ui.checkbox(&mut self.config.dynamic_colors, "Dynamic colors (Cover)");
        ui.checkbox(&mut self.config.text_border, "Text border");
        
        // Font path selection
        if ui.button("Select Font").clicked() {
            if let Some(path) = rfd::FileDialog::new().add_filter("Font", &["ttf", "otf"]).pick_file() {
                self.config.font_path = Some(path.to_string_lossy().to_string());
            }
        }
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_transparent(true)
            .with_decorations(false)
            .with_always_on_top()
            .with_mouse_passthrough(true) // Click-through
            .with_maximized(true),        // Cover primary monitor
        ..Default::default()
    };

    let app = VisualizerApp {
        config: AppConfig::default(),
        show_settings: false,
    };

    eframe::run_native("Rust Visualizer", options, Box::new(|_cc| Box::new(app)))
}
