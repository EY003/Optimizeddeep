use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use visualizer_core::{
    audio::AudioEngine,
    config::AppConfig,
};

struct VisualizerApp {
    config: AppConfig,
    current_bars: Vec<f32>,
    audio_rx: crossbeam_channel::Receiver<Vec<f32>>,
    should_quit: Arc<AtomicBool>,
    packets_received: u64,
}

impl VisualizerApp {
    fn new() -> Self {
        let config = AppConfig::default();
        let engine = AudioEngine::new(config.num_bars);
        let should_quit = Arc::new(AtomicBool::new(false));

        Self {
            current_bars: vec![0.0; config.num_bars],
            audio_rx: engine.data_rx,
            config,
            should_quit,
            packets_received: 0,
        }
    }
}

impl eframe::App for VisualizerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.should_quit.load(Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // Drain audio channel and smooth bars
        while let Ok(bars) = self.audio_rx.try_recv() {
            self.packets_received += 1;
            let max_val = bars.iter().cloned().fold(0.0_f32, f32::max);
            if self.packets_received % 30 == 0 {
                // Print every 30 packets to reduce spam
                eprintln!(
                    "[packet {}] max: {:.4}",
                    self.packets_received, max_val
                );
            }
            for (i, &val) in bars.iter().enumerate() {
                if i < self.current_bars.len() {
                    self.current_bars[i] = self.current_bars[i] * 0.6 + val * 0.4;
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Visualizer Debug");
            ui.label(format!("Audio packets received: {}", self.packets_received));
            let active = self.current_bars.iter().filter(|v| **v > 0.01).count();
            ui.label(format!("Active bars: {} / {}", active, self.current_bars.len()));
            ui.separator();

            // Reserve the rest of the window for bars
            let rect = ui.available_rect_before_wrap();
            self.draw_bars(ui, rect);
            ui.allocate_space(rect.size());

            ui.separator();
            self.draw_settings_ui(ui);
        });

                // ✅ FIX: Limit to ~60 FPS to drastically reduce CPU/GPU usage
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
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
            // ✅ FIX: Use a gentler scaling factor for the UI
            let sensitivity_multiplier = self.config.sensitivity / 15.0; 
            
            let final_height_normalized = (height_normalized * sensitivity_multiplier).clamp(0.0, 1.0);
            let final_height = final_height_normalized * rect.height();
            
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

    fn draw_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("General");
        ui.add(egui::Slider::new(&mut self.config.bar_width, 1.0..=50.0).text("Bar width"));
        ui.add(egui::Slider::new(&mut self.config.bar_gap, 0.0..=20.0).text("Bar gap"));
        ui.add(egui::Slider::new(&mut self.config.sensitivity, 1.0..=100.0).text("Sensitivity"));
        ui.add(egui::Slider::new(&mut self.config.num_bars, 16..=256).text("Number of Bars"));
        ui.checkbox(&mut self.config.dynamic_colors, "Dynamic colors (Cover)");
        ui.checkbox(&mut self.config.text_border, "Text border");
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1024.0, 600.0])
            .with_title("Visualizer Debug"),
        ..Default::default()
    };

    eframe::run_native(
        "Rust Visualizer",
        options,
        Box::new(|_cc| Box::new(VisualizerApp::new())),
    )
}
