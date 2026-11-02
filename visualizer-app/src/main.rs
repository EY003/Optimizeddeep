#![windows_subsystem = "windows"]
use eframe::egui;
use visualizer_core::{
    audio::AudioEngine, 
    config::AppConfig
};

struct VisualizerApp {
    config: AppConfig,
    show_settings: bool,
    current_bars: Vec<f32>,
    audio_rx: crossbeam_channel::Receiver<Vec<f32>>,
}

impl VisualizerApp {
    fn new() -> Self {
        let config = AppConfig::default();
        
        // Start the WASAPI loopback audio engine
        let engine = AudioEngine::new(config.num_bars);
        
        Self {
            current_bars: vec![0.0; config.num_bars],
            audio_rx: engine.data_rx,
            config,
            show_settings: false,
        }
    }
}

impl eframe::App for VisualizerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 1. Fetch new audio data from the background thread
        while let Ok(bars) = self.audio_rx.try_recv() {
            for (i, &val) in bars.iter().enumerate() {
                if i < self.current_bars.len() {
                    // Smoothing: Interpolate between old height and new height
                    // This prevents the bars from flickering too fast
                    self.current_bars[i] = self.current_bars[i] * 0.6 + val * 0.4;
                }
            }
        }

        // 2. Render the Click-Through Overlay
        if !self.show_settings {
            let rect = ctx.screen_rect();
            egui::Area::new(egui::Id::new("overlay"))
                .fixed_pos(rect.min)
                .show(ctx, |ui| {
                    ui.set_min_size(rect.size());
                    // Clear background to be fully transparent
                    ui.painter().rect_filled(rect, 0.0, egui::Color32::TRANSPARENT);
                    
                    self.draw_bars(ui, rect);
                    self.draw_song_info(ui, rect);
                });
        }

        // 3. Render the Settings Window
if self.show_settings {
    let mut is_open = self.show_settings;
    egui::Window::new("Settings")
        .open(&mut is_open)
        .show(ctx, |ui| {
            self.draw_settings_ui(ui);
        });
    self.show_settings = is_open;
}

        // Request continuous repaint for the 60 FPS animation
        ctx.request_repaint();
    }
}

impl VisualizerApp {
    fn draw_bars(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let painter = ui.painter();
        let bar_width = self.config.bar_width;
        let gap = self.config.bar_gap;
        
        let total_width = (self.config.num_bars as f32) * (bar_width + gap) - gap;
        // Center the bars horizontally
        let start_x = rect.center().x - (total_width / 2.0);
        
        for (i, &height_normalized) in self.current_bars.iter().enumerate() {
            // Apply sensitivity (your screenshot shows 33db, so we scale it)
            let sensitivity_multiplier = self.config.sensitivity / 10.0;
            let final_height = (height_normalized * sensitivity_multiplier * rect.height())
                .min(rect.height())
                .max(0.0);
            
            let x = start_x + (i as f32 * (bar_width + gap));
            let bar_rect = egui::Rect::from_min_max(
                egui::pos2(x, rect.bottom() - final_height),
                egui::pos2(x + bar_width, rect.bottom())
            );
            
            // Temporary static color (we will replace this with dynamic cover colors later)
            let color = if self.config.dynamic_colors {
                egui::Color32::from_rgb(180, 200, 180) 
            } else {
                egui::Color32::WHITE
            };

            painter.rect_filled(bar_rect, self.config.bar_radius, color);
        }
    }

    fn draw_song_info(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        // Placeholder for cover art and text (Top Right corner)
        let pos = rect.right_top() + egui::vec2(-30.0, 50.0);
        ui.painter().text(
            pos,
            egui::Align2::RIGHT_TOP,
            "Waiting for MusicBee...",
            egui::FontId::proportional(32.0),
            egui::Color32::WHITE,
        );
    }

    fn draw_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("General");
        ui.add(egui::Slider::new(&mut self.config.bar_width, 1.0..=50.0).text("Bar width"));
        ui.add(egui::Slider::new(&mut self.config.bar_gap, 0.0..=20.0).text("Bar gap"));
        ui.add(egui::Slider::new(&mut self.config.sensitivity, 1.0..=100.0).text("Sensitivity"));
        ui.add(egui::Slider::new(&mut self.config.num_bars, 16..=256).text("Number of Bars"));
        
        ui.separator();
        ui.heading("Style");
        ui.checkbox(&mut self.config.dynamic_colors, "Dynamic colors (Cover)");
        ui.checkbox(&mut self.config.text_border, "Text border");
        
        if ui.button("Select Custom Font").clicked() {
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
            .with_mouse_passthrough(true) // Makes it click-through!
            .with_maximized(true),        // Covers the screen
        ..Default::default()
    };

    eframe::run_native(
        "Rust Visualizer",
        options,
        Box::new(|_cc| Box::new(VisualizerApp::new())),
    )
}
