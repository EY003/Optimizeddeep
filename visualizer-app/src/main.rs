#![windows_subsystem = "windows"]

use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use visualizer_core::{
    audio::AudioEngine,
    config::AppConfig,
};

struct VisualizerApp {
    config: AppConfig,
    show_settings: bool,
    current_bars: Vec<f32>,
    audio_rx: crossbeam_channel::Receiver<Vec<f32>>,
    should_quit: Arc<AtomicBool>,
}

impl VisualizerApp {
    fn new() -> Self {
        let config = AppConfig::default();
        let engine = AudioEngine::new(config.num_bars);
        let should_quit = Arc::new(AtomicBool::new(false));

        // Debug helper: pressing Enter in the terminal closes the app
        let quit_flag = should_quit.clone();
        std::thread::spawn(move || {
            use std::io::BufRead;
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                if line.is_err() {
                    break;
                }
                quit_flag.store(true, Ordering::Relaxed);
                break;
            }
        });

        Self {
            current_bars: vec![0.0; config.num_bars],
            audio_rx: engine.data_rx,
            config,
            show_settings: true, // Start with settings open so you can see it
            should_quit,
        }
    }
}

impl eframe::App for VisualizerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Allow exiting via Enter key
        if self.should_quit.load(Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // Fetch audio data and apply smoothing
        while let Ok(bars) = self.audio_rx.try_recv() {
            let max_val = bars.iter().cloned().fold(0.0_f32, f32::max);
            eprintln!("Audio packet received. Max bar: {:.3}", max_val);
            for (i, &val) in bars.iter().enumerate() {
                if i < self.current_bars.len() {
                    self.current_bars[i] = self.current_bars[i] * 0.6 + val * 0.4;
                }
            }
        }

        // Overlay rendering
        if !self.show_settings {
            let rect = ctx.screen_rect();
            egui::Area::new(egui::Id::new("overlay"))
                .fixed_pos(rect.min)
                .show(ctx, |ui| {
                    ui.set_min_size(rect.size());
                    ui.painter()
                        .rect_filled(rect, 0.0, egui::Color32::TRANSPARENT);

                    // TEMPORARY: red reference line to confirm rendering
                    ui.painter().line_segment(
                        [rect.left_bottom(), rect.right_bottom()],
                        egui::Stroke::new(3.0, egui::Color32::RED),
                    );

                    self.draw_bars(ui, rect);
                    self.draw_song_info(ui, rect);
                });
        }

        // Settings window
        if self.show_settings {
            let mut is_open = self.show_settings;
            egui::Window::new("Settings")
                .open(&mut is_open)
                .resizable(true)
                .default_size([400.0, 600.0])
                .show(ctx, |ui| {
                    self.draw_settings_ui(ui);
                    ui.separator();
                    ui.label(format!(
                        "Active bars: {}",
                        self.current_bars.iter().filter(|v| **v > 0.01).count()
                    ));
                    if ui.button("Close Settings (Start Overlay)").clicked() {
                        self.show_settings = false;
                    }
                });
            self.show_settings = is_open;
        }

        ctx.request_repaint();
    }
}

impl VisualizerApp {
    fn draw_bars(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let painter = ui.painter();
        let bar_width = self.config.bar_width;
        let gap = self.config.bar_gap;

        let total_width = (self.config.num_bars as f32) * (bar_width + gap) - gap;
        let start_x = rect.center().x - (total_width / 2.0);

        for (i, &height_normalized) in self.current_bars.iter().enumerate() {
            let sensitivity_multiplier = self.config.sensitivity / 10.0;
            let final_height = (height_normalized * sensitivity_multiplier * rect.height())
                .min(rect.height())
                .max(0.0);

            let x = start_x + (i as f32 * (bar_width + gap));
            let bar_rect = egui::Rect::from_min_max(
                egui::pos2(x, rect.bottom() - final_height),
                egui::pos2(x + bar_width, rect.bottom()),
            );

            let color = if self.config.dynamic_colors {
                egui::Color32::from_rgb(180, 200, 180)
            } else {
                egui::Color32::WHITE
            };

            painter.rect_filled(bar_rect, self.config.bar_radius, color);
        }
    }

    fn draw_song_info(&self, ui: &mut egui::Ui, rect: egui::Rect) {
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
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Font", &["ttf", "otf"])
                .pick_file()
            {
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
            .with_mouse_passthrough(true)
            .with_maximized(true),
        ..Default::default()
    };

    eframe::run_native(
        "Rust Visualizer",
        options,
        Box::new(|_cc| Box::new(VisualizerApp::new())),
    )
}
