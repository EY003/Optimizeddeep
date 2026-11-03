#![windows_subsystem = "windows"]

use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use visualizer_core::{
    audio::AudioEngine,
    config::AppConfig,
};

use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, hotkey::{Code, HotKey, Modifiers}};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    TrayIconBuilder, TrayIcon,
};

struct VisualizerApp {
    config: AppConfig,
    current_bars: Vec<f32>,
    audio_rx: crossbeam_channel::Receiver<Vec<f32>>,
    audio_target_bars: Arc<std::sync::atomic::AtomicUsize>,
    audio_update_interval: Arc<std::sync::atomic::AtomicU64>, // ✅ NEW
    should_quit: Arc<AtomicBool>,
    
    show_settings: bool,
    current_fps: f32,
    
    _tray_icon: TrayIcon,
    _hotkey_manager: GlobalHotKeyManager,
    settings_menu_id: tray_icon::menu::MenuId,
    quit_menu_id: tray_icon::menu::MenuId,
}

impl VisualizerApp {
    fn new() -> Self {
        let config = AppConfig::load();
        let engine = AudioEngine::new(config.num_bars, config.audio_update_ms);
        let should_quit = Arc::new(AtomicBool::new(false));
        let audio_target_bars = engine.target_bars.clone();
        let audio_update_interval = engine.update_interval_ms.clone();

        let tray_menu = Menu::new();
        let settings_item = MenuItem::new("Open Settings", true, None);
        let quit_item = MenuItem::new("Quit", true, None);
        tray_menu.append_items(&[&settings_item, &quit_item]).unwrap();

        let mut icon_rgba = vec![0; 16 * 16 * 4];
        for i in 0..(16 * 16) {
            icon_rgba[i * 4 + 1] = 255;
            icon_rgba[i * 4 + 3] = 255;
        }
        let icon = tray_icon::Icon::from_rgba(icon_rgba, 16, 16).unwrap();

        let tray_icon = TrayIconBuilder::new()
            .with_menu(Box::new(tray_menu))
            .with_tooltip("Rust Music Visualizer")
            .with_icon(icon)
            .build()
            .unwrap();

        let hotkey_manager = GlobalHotKeyManager::new().unwrap();
        let hotkey = HotKey::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyV);
        hotkey_manager.register(hotkey).unwrap();

        Self {
            current_bars: vec![0.0; config.num_bars],
            audio_rx: engine.data_rx,
            audio_target_bars,
            audio_update_interval,
            config,
            should_quit,
            show_settings: true, 
            current_fps: 0.0,
            _tray_icon: tray_icon,
            _hotkey_manager: hotkey_manager,
            settings_menu_id: settings_item.id().clone(),
            quit_menu_id: quit_item.id().clone(),
        }
    }
}

impl eframe::App for VisualizerApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::TRANSPARENT.to_array()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dt = ctx.input(|i| i.unstable_dt);
        if dt > 0.0 {
            let instant_fps = 1.0 / dt;
            self.current_fps = self.current_fps * 0.9 + instant_fps * 0.1;
        }

        if let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == self.settings_menu_id { self.show_settings = true; } 
            else if event.id == self.quit_menu_id { self.should_quit.store(true, Ordering::Relaxed); }
        }

        if let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            if event.state == global_hotkey::HotKeyState::Pressed { self.show_settings = !self.show_settings; }
        }

        if self.should_quit.load(Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // Sync Audio Bars
        let current_target = self.audio_target_bars.load(Ordering::Relaxed);
        if current_target != self.config.num_bars {
            self.audio_target_bars.store(self.config.num_bars, Ordering::Relaxed);
            self.current_bars.resize(self.config.num_bars, 0.0);
        }

        // ✅ Sync Audio Throttle
        let current_interval = self.audio_update_interval.load(Ordering::Relaxed);
        if current_interval != self.config.audio_update_ms {
            self.audio_update_interval.store(self.config.audio_update_ms, Ordering::Relaxed);
        }

        while let Ok(bars) = self.audio_rx.try_recv() {
            for (i, &val) in bars.iter().enumerate() {
                if i < self.current_bars.len() {
                    self.current_bars[i] = self.current_bars[i] * 0.6 + val * 0.4;
                }
            }
        }

        // ==========================================
        // 1. OVERLAY
        // ==========================================
        ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(true));

        let rect = ctx.screen_rect();
        egui::Area::new(egui::Id::new("overlay"))
            .fixed_pos(rect.min)
            .show(ctx, |ui| {
                ui.set_min_size(rect.size());
                
                if self.config.bg_opacity > 0 {
                    let bg_color = egui::Color32::from_rgba_unmultiplied(0, 0, 0, self.config.bg_opacity);
                    ui.painter().rect_filled(rect, 0.0, bg_color);
                }
                
                self.draw_bars(ui, rect);
                
                if self.config.show_fps {
                    ui.painter().text(
                        rect.left_top() + egui::vec2(20.0, 20.0),
                        egui::Align2::LEFT_TOP,
                        format!("FPS: {:.1}", self.current_fps),
                        egui::FontId::monospace(16.0),
                        egui::Color32::GREEN,
                    );
                }
            });

        // ==========================================
        // 2. SETTINGS
        // ==========================================
        if self.show_settings {
            let settings_builder = egui::ViewportBuilder::default()
                .with_title("Visualizer Settings")
                .with_inner_size([400.0, 800.0])
                .with_decorations(true)
                .with_resizable(true)
                .with_always_on_top();

            let mut close_window = false;

            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("settings"),
                settings_builder,
                |ctx, _class| {
                    if ctx.input(|i| i.viewport().close_requested()) {
                        close_window = true;
                    }

                    egui::CentralPanel::default()
                        .frame(egui::Frame::window(&ctx.style()).fill(egui::Color32::from_rgb(25, 25, 25)))
                        .show(ctx, |ui| {
                            self.draw_settings_ui(ui);
                            
                            ui.separator();
                            if ui.button("❌ Close Settings").clicked() {
                                close_window = true;
                            }
                        });
                },
            );

            if close_window {
                self.show_settings = false;
                ctx.send_viewport_cmd_to(egui::ViewportId::from_hash_of("settings"), egui::ViewportCommand::Close);
            }
        }

        // Enforce custom FPS limit
        ctx.request_repaint_after(std::time::Duration::from_secs_f32(1.0 / self.config.target_fps.max(1.0)));
    }
    
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
        
        let start_x = rect.center().x - (total_width / 2.0) + self.config.x_offset;
        let bottom_y = rect.bottom() + self.config.y_offset;

        for (i, &height_normalized) in self.current_bars.iter().enumerate() {
            let sensitivity_multiplier = self.config.sensitivity / 30.0; 
            let mut final_height = height_normalized * sensitivity_multiplier * rect.height();

            if self.config.max_bar_height > 0.0 {
                final_height = final_height.min(self.config.max_bar_height);
            }

            let x = start_x + (i as f32 * (bar_width + gap));
            let bar_rect = egui::Rect::from_min_max(
                egui::pos2(x, bottom_y - final_height),
                egui::pos2(x + bar_width, bottom_y),
            );

            let color = if self.config.dynamic_colors {
                egui::Color32::from_rgb(180, 200, 180)
            } else {
                egui::Color32::from_rgb(self.config.bar_color[0], self.config.bar_color[1], self.config.bar_color[2])
            };

            painter.rect_filled(bar_rect, self.config.bar_radius, color);
        }
    }

    fn draw_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Performance & Optimization");
        ui.add(egui::Slider::new(&mut self.config.target_fps, 15.0..=144.0).text("Max Target FPS"));
        ui.add(egui::Slider::new(&mut self.config.audio_update_ms, 10..=100).text("Audio Poll Rate (ms)"));
        ui.label("Higher ms = lower CPU usage but more input latency.");
        ui.checkbox(&mut self.config.show_fps, "Show FPS Counter");
        ui.label(format!("Current FPS: {:.1}", self.current_fps));
        
        ui.separator();
        ui.heading("Layout");
        ui.add(egui::Slider::new(&mut self.config.bar_width, 1.0..=50.0).text("Bar width"));
        ui.add(egui::Slider::new(&mut self.config.bar_gap, 0.0..=20.0).text("Bar gap"));
        ui.add(egui::Slider::new(&mut self.config.sensitivity, 1.0..=100.0).text("Sensitivity"));
        ui.add(egui::Slider::new(&mut self.config.max_bar_height, 0.0..=1000.0).text("Max Bar Height (0 = Full)"));
        ui.add(egui::Slider::new(&mut self.config.num_bars, 16..=256).text("Number of Bars"));
        
        ui.separator();
        ui.heading("Positioning");
        ui.add(egui::Slider::new(&mut self.config.x_offset, -1000.0..=1000.0).text("X Offset"));
        ui.add(egui::Slider::new(&mut self.config.y_offset, -1000.0..=1000.0).text("Y Offset"));

        ui.separator();
        ui.heading("Color Mode");
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.config.dynamic_colors, true, "Dynamic (Cover Art)");
            ui.radio_value(&mut self.config.dynamic_colors, false, "Static Color");
        });

        if !self.config.dynamic_colors {
            ui.horizontal(|ui| {
                ui.label("Bar Color:");
                ui.color_edit_button_srgb(&mut self.config.bar_color);
                if ui.button("Save Preset").clicked() {
                    if !self.config.color_presets.contains(&self.config.bar_color) {
                        self.config.color_presets.push(self.config.bar_color);
                    }
                }
            });

            ui.label("Presets:");
            ui.horizontal_wrapped(|ui| {
                for preset in &self.config.color_presets {
                    let (rect, response) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
                    if ui.is_rect_visible(rect) {
                        let color = egui::Color32::from_rgb(preset[0], preset[1], preset[2]);
                        ui.painter().rect_filled(rect, 4.0, color);
                        if response.clicked() {
                            self.config.bar_color = *preset;
                        }
                    }
                }
            });
        }

        ui.separator();
        ui.heading("Appearance");
        ui.add(egui::Slider::new(&mut self.config.bg_opacity, 0..=255).text("Background Opacity"));
        
        ui.separator();
        if ui.button("⚠️ Reset to Defaults").clicked() {
            self.config = AppConfig::default();
            self.audio_target_bars.store(self.config.num_bars, Ordering::Relaxed);
            self.audio_update_interval.store(self.config.audio_update_ms, Ordering::Relaxed);
            self.current_bars.resize(self.config.num_bars, 0.0);
        }
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_transparent(true)
            .with_decorations(false)
            .with_always_on_top()
            .with_maximized(true),
        vsync: false, // ✅ Turn off VSync so manual FPS slider works
        ..Default::default()
    };

    eframe::run_native(
        "Rust Visualizer",
        options,
        Box::new(|_cc| Box::new(VisualizerApp::new())),
    )
}
