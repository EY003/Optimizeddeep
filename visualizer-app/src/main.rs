#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use visualizer_core::{
    audio::AudioEngine,
    config::{AppConfig, FrequencyDirection, VisualizerDirection},
};

use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager,
};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    TrayIcon, TrayIconBuilder,
};

struct VisualizerApp {
    config: AppConfig,
    current_bars: Vec<f32>,
    audio_rx: crossbeam_channel::Receiver<Vec<f32>>,
    audio_target_bars: Arc<std::sync::atomic::AtomicUsize>,
    audio_update_interval: Arc<std::sync::atomic::AtomicU64>,
    should_quit: Arc<AtomicBool>,

    show_settings: bool,
    current_fps: f32,
    is_passthrough_set: bool,

    /// Timestamp at the *start* of the previous `update()` call.
    /// Used to measure real inter-frame time for the FPS counter.
    last_frame_time: Instant,

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
        tray_menu
            .append_items(&[&settings_item, &quit_item])
            .unwrap();

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
            is_passthrough_set: false,
            last_frame_time: Instant::now(),
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
        // ── 0. FPS counter ─────────────────────────────────────────────
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame_time).as_secs_f32();
        self.last_frame_time = now;
        if dt > 0.0 {
            let instant_fps = 1.0 / dt;
            self.current_fps = self.current_fps * 0.9 + instant_fps * 0.1;
        }

        // ── 1. Force 1.0 pixels-per-point (only when needed) ──────────
        if ctx.pixels_per_point() != 1.0 {
            ctx.set_pixels_per_point(1.0);
        }

        // ── 2. Tray & hotkey events ───────────────────────────────────
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == self.settings_menu_id {
                self.show_settings = true;
            } else if event.id == self.quit_menu_id {
                self.should_quit.store(true, Ordering::Relaxed);
            }
        }

        while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            if event.state == global_hotkey::HotKeyState::Pressed {
                if self.show_settings {
                    self.show_settings = false;
                    ctx.send_viewport_cmd_to(
                        egui::ViewportId::from_hash_of("settings"),
                        egui::ViewportCommand::Close,
                    );
                } else {
                    self.show_settings = true;
                }
            }
        }

        if self.should_quit.load(Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // ── 3. Push settings into the audio thread ────────────────────
        let current_target = self.audio_target_bars.load(Ordering::Relaxed);
        if current_target != self.config.num_bars {
            self.audio_target_bars
                .store(self.config.num_bars, Ordering::Relaxed);
            self.current_bars.resize(self.config.num_bars, 0.0);
        }

        let current_interval = self.audio_update_interval.load(Ordering::Relaxed);
        if current_interval != self.config.audio_update_ms {
            self.audio_update_interval
                .store(self.config.audio_update_ms, Ordering::Relaxed);
        }

        // ── 4. Drain audio channel — keep only the newest frame ───────
        let mut latest_bars: Option<Vec<f32>> = None;
        while let Ok(bars) = self.audio_rx.try_recv() {
            latest_bars = Some(bars);
        }
        if let Some(bars) = latest_bars {
            let limit = self.current_bars.len().min(bars.len());
            for i in 0..limit {
                let val = bars[i];
                if val > self.current_bars[i] {
                    self.current_bars[i] = val;
                } else {
                    self.current_bars[i] = self.current_bars[i] * 0.8 + val * 0.2;
                }
            }
        }

        // ── 5. One-shot mouse passthrough ─────────────────────────────
        if !self.is_passthrough_set {
            ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(true));
            self.is_passthrough_set = true;
        }

        // ── 6. Overlay ────────────────────────────────────────────────
        let rect = ctx.screen_rect();
        egui::Area::new(egui::Id::new("overlay"))
            .fixed_pos(rect.min)
            .show(ctx, |ui| {
                ui.set_min_size(rect.size());

                if self.config.bg_opacity > 0 {
                    let bg_color =
                        egui::Color32::from_rgba_unmultiplied(0, 0, 0, self.config.bg_opacity);
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

        // ── 7. Settings viewport ──────────────────────────────────────
        if self.show_settings {
            let settings_builder = egui::ViewportBuilder::default()
                .with_title("Visualizer Settings")
                .with_inner_size([400.0, 860.0])
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
                        .frame(
                            egui::Frame::window(&ctx.style())
                                .fill(egui::Color32::from_rgb(25, 25, 25)),
                        )
                        .show(ctx, |ui| {
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                self.draw_settings_ui(ui);

                                ui.separator();
                                if ui.button("❌ Close Settings").clicked() {
                                    close_window = true;
                                }
                            });
                        });
                },
            );

            if close_window {
                self.show_settings = false;
                self.config.save();
                ctx.send_viewport_cmd_to(
                    egui::ViewportId::from_hash_of("settings"),
                    egui::ViewportCommand::Close,
                );
            }
        }

        // ── 8. Frame pacing ──────────────────────────────────────────
        let target_duration = Duration::from_secs_f32(1.0 / self.config.target_fps.max(1.0));

        let repaint_after = if self.show_settings {
            target_duration.min(Duration::from_millis(16))
        } else {
            target_duration
        };

        ctx.request_repaint_after(repaint_after);
    }

    fn on_exit(&mut self, _gl: Option<&glow::Context>) {
        self.config.save();
    }
}

/// Map a visual bar position to the FFT bin it should read from.
///
/// - `LeftToRight`: bin order is 0, 1, 2, ...
/// - `RightToLeft`: bin order is N-1, N-2, ...
/// - `Mirrored`:    bin order is 0, 1, ..., N/2-1, N/2-1, ..., 1, 0
///                  (low -> high -> high -> low, symmetric around the centre).
fn frequency_index(i: usize, num_bars: usize, dir: FrequencyDirection) -> usize {
    match dir {
        FrequencyDirection::LeftToRight => i,
        FrequencyDirection::RightToLeft => num_bars - 1 - i,
        FrequencyDirection::Mirrored => {
            let half = num_bars / 2;
            if i < half {
                i
            } else {
                num_bars - 1 - i
            }
        }
    }
}

impl VisualizerApp {
    fn draw_bars(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let num_bars = self.current_bars.len();
        if num_bars == 0 {
            return;
        }

        let bar_width = self.config.bar_width;
        let gap = self.config.bar_gap;
        let stride = bar_width + gap;
        let total_size = num_bars as f32 * stride - gap;

        let sensitivity_multiplier = self.config.sensitivity / 30.0;
        let max_h = self.config.max_bar_height;
        let radius = self.config.bar_radius;
        let freq_dir = self.config.frequency_direction;

        // NOTE: "Dynamic (cover art)" is currently a fixed tint — this is
        // where a cover-art sampler would inject its dominant colour.
        let color = if self.config.dynamic_colors {
            egui::Color32::from_rgb(180, 200, 180)
        } else {
            egui::Color32::from_rgb(
                self.config.bar_color[0],
                self.config.bar_color[1],
                self.config.bar_color[2],
            )
        };

        let painter = ui.painter();

        match self.config.visualizer_direction {
            VisualizerDirection::Up => {
                // Bars grow upward from the bottom edge.
                let start_x = rect.center().x - (total_size / 2.0) + self.config.x_offset;
                let bottom_y = rect.bottom() + self.config.y_offset;
                let rect_h = rect.height();

                for i in 0..num_bars {
                    let idx = frequency_index(i, num_bars, freq_dir);
                    let mut h = self.current_bars[idx] * sensitivity_multiplier * rect_h;
                    if max_h > 0.0 {
                        h = h.min(max_h);
                    }
                    if h <= 0.0 {
                        continue;
                    }

                    let x = start_x + i as f32 * stride;
                    let bar_rect = egui::Rect::from_min_max(
                        egui::pos2(x, bottom_y - h),
                        egui::pos2(x + bar_width, bottom_y),
                    );
                    painter.rect_filled(bar_rect, radius, color);
                }
            }

            VisualizerDirection::Down => {
                // Bars grow downward from the top edge.
                let start_x = rect.center().x - (total_size / 2.0) + self.config.x_offset;
                let top_y = rect.top() + self.config.y_offset;
                let rect_h = rect.height();

                for i in 0..num_bars {
                    let idx = frequency_index(i, num_bars, freq_dir);
                    let mut h = self.current_bars[idx] * sensitivity_multiplier * rect_h;
                    if max_h > 0.0 {
                        h = h.min(max_h);
                    }
                    if h <= 0.0 {
                        continue;
                    }

                    let x = start_x + i as f32 * stride;
                    let bar_rect = egui::Rect::from_min_max(
                        egui::pos2(x, top_y),
                        egui::pos2(x + bar_width, top_y + h),
                    );
                    painter.rect_filled(bar_rect, radius, color);
                }
            }

            VisualizerDirection::Right => {
                // Horizontal bars extending rightward from the left edge.
                let left_x = rect.left() + self.config.x_offset;
                let start_y = rect.center().y - (total_size / 2.0) + self.config.y_offset;
                let rect_w = rect.width();

                for i in 0..num_bars {
                    let idx = frequency_index(i, num_bars, freq_dir);
                    let mut w = self.current_bars[idx] * sensitivity_multiplier * rect_w;
                    if max_h > 0.0 {
                        w = w.min(max_h);
                    }
                    if w <= 0.0 {
                        continue;
                    }

                    let y = start_y + i as f32 * stride;
                    let bar_rect = egui::Rect::from_min_max(
                        egui::pos2(left_x, y),
                        egui::pos2(left_x + w, y + bar_width),
                    );
                    painter.rect_filled(bar_rect, radius, color);
                }
            }

            VisualizerDirection::Left => {
                // Horizontal bars extending leftward from the right edge.
                let right_x = rect.right() + self.config.x_offset;
                let start_y = rect.center().y - (total_size / 2.0) + self.config.y_offset;
                let rect_w = rect.width();

                for i in 0..num_bars {
                    let idx = frequency_index(i, num_bars, freq_dir);
                    let mut w = self.current_bars[idx] * sensitivity_multiplier * rect_w;
                    if max_h > 0.0 {
                        w = w.min(max_h);
                    }
                    if w <= 0.0 {
                        continue;
                    }

                    let y = start_y + i as f32 * stride;
                    let bar_rect = egui::Rect::from_min_max(
                        egui::pos2(right_x - w, y),
                        egui::pos2(right_x, y + bar_width),
                    );
                    painter.rect_filled(bar_rect, radius, color);
                }
            }
        }
    }

    fn draw_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Performance & Optimization");
        ui.checkbox(&mut self.config.use_vsync, "Enable VSync (Requires Restart)");
        ui.add(
            egui::Slider::new(&mut self.config.target_fps, 15.0..=144.0).text("Max Target FPS"),
        );
        ui.add(
            egui::Slider::new(&mut self.config.audio_update_ms, 10..=100)
                .text("Audio Poll Rate (ms)"),
        );
        ui.label("Higher ms = lower CPU usage but more input latency.");
        ui.checkbox(&mut self.config.show_fps, "Show FPS Counter");
        ui.label(format!("Current FPS: {:.1}", self.current_fps));

        ui.separator();
        ui.heading("Visualizer Direction");
        ui.horizontal(|ui| {
            ui.radio_value(
                &mut self.config.visualizer_direction,
                VisualizerDirection::Up,
                "Up",
            );
            ui.radio_value(
                &mut self.config.visualizer_direction,
                VisualizerDirection::Down,
                "Down",
            );
            ui.radio_value(
                &mut self.config.visualizer_direction,
                VisualizerDirection::Left,
                "Left",
            );
            ui.radio_value(
                &mut self.config.visualizer_direction,
                VisualizerDirection::Right,
                "Right",
            );
        });
        ui.label("Direction the bars grow from their anchor edge.");

        ui.separator();
        ui.heading("Frequency Direction");
        ui.horizontal(|ui| {
            ui.radio_value(
                &mut self.config.frequency_direction,
                FrequencyDirection::LeftToRight,
                "Low → High",
            );
            ui.radio_value(
                &mut self.config.frequency_direction,
                FrequencyDirection::RightToLeft,
                "High → Low",
            );
            ui.radio_value(
                &mut self.config.frequency_direction,
                FrequencyDirection::Mirrored,
                "Mirrored",
            );
        });
        ui.label("Mirrored: low → high → high → low (symmetric).");

        ui.separator();
        ui.heading("Layout");
        ui.add(egui::Slider::new(&mut self.config.bar_width, 1.0..=50.0).text("Bar width"));
        ui.add(egui::Slider::new(&mut self.config.bar_gap, 0.0..=20.0).text("Bar gap"));
        ui.add(egui::Slider::new(&mut self.config.sensitivity, 1.0..=100.0).text("Sensitivity"));
        ui.add(
            egui::Slider::new(&mut self.config.max_bar_height, 0.0..=1000.0)
                .text("Max Bar Height (0 = Full)"),
        );
        ui.add(
            egui::Slider::new(&mut self.config.num_bars, 16..=256).text("Number of Bars"),
        );

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
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
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
            self.audio_target_bars
                .store(self.config.num_bars, Ordering::Relaxed);
            self.audio_update_interval
                .store(self.config.audio_update_ms, Ordering::Relaxed);
            self.current_bars = vec![0.0; self.config.num_bars];
        }
    }
}

fn main() -> eframe::Result<()> {
    // Load config before window creation so VSync can be applied.
    let config = AppConfig::load();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_transparent(true)
            .with_decorations(false)
            .with_always_on_top()
            .with_maximized(true),
        vsync: config.use_vsync,
        ..Default::default()
    };

    eframe::run_native(
        "Rust Visualizer",
        options,
        Box::new(|_cc| Box::new(VisualizerApp::new())),
    )
}
