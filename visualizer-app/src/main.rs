#![windows_subsystem = "windows"] // Hides the console window

use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use visualizer_core::{
    audio::AudioEngine,
    config::AppConfig,
};

// Global Hotkeys & Tray imports
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, hotkey::{Code, HotKey, Modifiers}};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    TrayIconBuilder, TrayIcon,
};

struct VisualizerApp {
    config: AppConfig,
    current_bars: Vec<f32>,
    audio_rx: crossbeam_channel::Receiver<Vec<f32>>,
    should_quit: Arc<AtomicBool>,
    
    show_settings: bool,
    last_show_settings: bool,

    // We must keep these alive so they aren't dropped and destroyed
    _tray_icon: TrayIcon,
    _hotkey_manager: GlobalHotKeyManager,
    settings_menu_id: tray_icon::menu::MenuId,
    quit_menu_id: tray_icon::menu::MenuId,
}

impl VisualizerApp {
    fn new() -> Self {
        let config = AppConfig::load(); // Load from disk
        let engine = AudioEngine::new(config.num_bars);
        let should_quit = Arc::new(AtomicBool::new(false));

        // 1. Setup System Tray
        let tray_menu = Menu::new();
        let settings_item = MenuItem::new("Open Settings", true, None);
        let quit_item = MenuItem::new("Quit", true, None);
        tray_menu.append_items(&[&settings_item, &quit_item]).unwrap();

        // Create a simple 16x16 green square icon programmatically
        let mut icon_rgba = vec![0; 16 * 16 * 4];
        for i in 0..(16 * 16) {
            icon_rgba[i * 4 + 1] = 255; // Green
            icon_rgba[i * 4 + 3] = 255; // Alpha
        }
        let icon = tray_icon::Icon::from_rgba(icon_rgba, 16, 16).unwrap();

        let tray_icon = TrayIconBuilder::new()
            .with_menu(Box::new(tray_menu))
            .with_tooltip("Rust Music Visualizer")
            .with_icon(icon)
            .build()
            .unwrap();

        // 2. Setup Global Hotkey (Ctrl+Shift+V)
        let hotkey_manager = GlobalHotKeyManager::new().unwrap();
        let hotkey = HotKey::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyV);
        hotkey_manager.register(hotkey).unwrap();

        Self {
            current_bars: vec![0.0; config.num_bars],
            audio_rx: engine.data_rx,
            config,
            should_quit,
            show_settings: true, // Start with settings open on first launch
            last_show_settings: false, 
            _tray_icon: tray_icon,
            _hotkey_manager: hotkey_manager,
            settings_menu_id: settings_item.id().clone(),
            quit_menu_id: quit_item.id().clone(),
        }
    }
}

impl eframe::App for VisualizerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // --- Handle Events (Tray & Hotkeys) ---
        if let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == self.settings_menu_id {
                self.show_settings = true;
            } else if event.id == self.quit_menu_id {
                self.should_quit.store(true, Ordering::Relaxed);
            }
        }

        if let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            if event.state == global_hotkey::HotKeyState::Pressed {
                self.show_settings = !self.show_settings;
            }
        }

        if self.should_quit.load(Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // --- Handle Window State Changes (Overlay vs Settings) ---
        // We only send these commands if the state has changed, to prevent flickering
        if self.show_settings != self.last_show_settings {
            if self.show_settings {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(500.0, 600.0)));
                ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            }
            self.last_show_settings = self.show_settings;
        }

        // --- Process Audio ---
        while let Ok(bars) = self.audio_rx.try_recv() {
            for (i, &val) in bars.iter().enumerate() {
                if i < self.current_bars.len() {
                    self.current_bars[i] = self.current_bars[i] * 0.6 + val * 0.4;
                }
            }
        }

        // --- Render UI ---
        if self.show_settings {
            // Draw Settings Window (Opaque background)
            egui::CentralPanel::default()
                .frame(egui::Frame::window(&ctx.style()).fill(egui::Color32::from_rgb(25, 25, 25)))
                .show(ctx, |ui| {
                    self.draw_settings_ui(ui);
                    if ui.button("Close Settings (Show Overlay)").clicked() {
                        self.show_settings = false;
                    }
                });
        } else {
            // Draw Transparent Overlay
            let rect = ctx.screen_rect();
            egui::Area::new(egui::Id::new("overlay"))
                .fixed_pos(rect.min)
                .show(ctx, |ui| {
                    ui.set_min_size(rect.size());
                    // Clear background
                    ui.painter().rect_filled(rect, 0.0, egui::Color32::TRANSPARENT);
                    
                    self.draw_bars(ui, rect);
                    self.draw_song_info(ui, rect);
                });
        }

        // Limit to ~60 FPS
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
    
    // Save config automatically when app closes
    fn on_exit(&mut self, _gl: Option<&glow::Context>) {
        self.config.save();
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
        ui.heading("Visualizer Settings");
        ui.separator();
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
        
        ui.separator();
        ui.label("Press Ctrl+Shift+V to toggle this menu.");
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_transparent(true) // Required for the overlay to work
            .with_title("Rust Visualizer"),
        ..Default::default()
    };

    eframe::run_native(
        "Rust Visualizer",
        options,
        Box::new(|_cc| Box::new(VisualizerApp::new())),
    )
}
