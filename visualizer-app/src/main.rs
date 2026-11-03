#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use visualizer_core::{
    audio::AudioEngine,
    color,
    config::{
        AppConfig, ColorChangePreset, ColorMode, DistributionDirection, FrequencyDirection,
        GradientPreset, GradientStop, StaticMultiPreset, VisualizerDirection,
    },
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
        // FPS counter
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame_time).as_secs_f32();
        self.last_frame_time = now;
        if dt > 0.0 {
            let instant_fps = 1.0 / dt;
            self.current_fps = self.current_fps * 0.9 + instant_fps * 0.1;
        }

        if ctx.pixels_per_point() != 1.0 {
            ctx.set_pixels_per_point(1.0);
        }

        // Tray & hotkey events
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

        // Audio thread sync
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

        // Drain audio channel
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

        if !self.is_passthrough_set {
            ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(true));
            self.is_passthrough_set = true;
        }

        // Overlay
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

        // Settings viewport
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

        // Frame pacing
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

fn frequency_index(i: usize, base_num_bars: usize, dir: FrequencyDirection) -> usize {
    match dir {
        FrequencyDirection::LeftToRight => i,
        FrequencyDirection::RightToLeft => base_num_bars - 1 - i,
        FrequencyDirection::Mirrored => {
            if i < base_num_bars {
                i
            } else {
                2 * base_num_bars - 1 - i
            }
        }
    }
}

impl VisualizerApp {
    /// Helper to get the color for a single bar (or the start of a stacked bar)
    fn get_bar_color(
        &self,
        i: usize,
        total_bars: usize,
        bar_rect: egui::Rect,
        screen_rect: egui::Rect,
        time: f32,
    ) -> egui::Color32 {
        match self.config.color_mode {
            ColorMode::Dynamic => egui::Color32::from_rgb(180, 200, 180), // Placeholder for dynamic mode
            ColorMode::StaticSingle => egui::Color32::from_rgb(
                self.config.bar_color[0],
                self.config.bar_color[1],
                self.config.bar_color[2],
            ),
            ColorMode::StaticMultiple => {
                // Horizontal distribution (across bars)
                let t = i as f32 / (total_bars.max(1) - 1) as f32;
                let idx = (t * (self.config.static_colors.len().max(1) - 1) as f32).round() as usize;
                let c = self.config.static_colors.get(idx).copied().unwrap_or([255, 255, 255]);
                egui::Color32::from_rgb(c[0], c[1], c[2])
            }
            ColorMode::Gradient => {
                let angle_rad = self.config.gradient_angle.to_radians();
                let dir = egui::vec2(angle_rad.cos(), angle_rad.sin());

                // Project the bar's center onto the gradient direction
                let norm_x = (bar_rect.center().x - screen_rect.left()) / screen_rect.width();
                let norm_y = (bar_rect.center().y - screen_rect.top()) / screen_rect.height();
                
                // Center the coordinates (range -0.5 to 0.5) so the gradient rotates from the center
                let centered_x = norm_x - 0.5;
                let centered_y = norm_y - 0.5;

                let projection = centered_x * dir.x + centered_y * dir.y;
                let t = (projection + 0.5).clamp(0.0, 1.0);

                let c = color::sample_gradient(&self.config.gradient_stops, t);
                egui::Color32::from_rgb(c[0], c[1], c[2])
            }
            ColorMode::ColorChanging => {
                // Map 0.0-1.0 slider to a time multiplier
                let speed_hz = 0.05 + (self.config.color_change_speed * 1.95);
                let phase = time * speed_hz;
                let c = color::cyclic_color(&self.config.color_change_colors, phase);
                egui::Color32::from_rgb(c[0], c[1], c[2])
            }
        }
    }

    fn draw_bars(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let base_num_bars = self.current_bars.len();
        if base_num_bars == 0 {
            return;
        }

        let visual_num_bars = if self.config.frequency_direction == FrequencyDirection::Mirrored {
            base_num_bars * 2
        } else {
            base_num_bars
        };

        let bar_width = self.config.bar_width;
        let gap = self.config.bar_gap;
        let stride = bar_width + gap;
        let total_size = visual_num_bars as f32 * stride - gap;

        let sensitivity_multiplier = self.config.sensitivity / 30.0;
        let max_h = self.config.max_bar_height;
        let radius = self.config.bar_radius;
        let freq_dir = self.config.frequency_direction;
        let time = ui.input(|i| i.time) as f32;
        let painter = ui.painter();

        // Determine if we should draw vertical stacked slices
        let is_vertical_stack = self.config.color_mode == ColorMode::StaticMultiple 
            && self.config.static_direction == DistributionDirection::Vertical;

        match self.config.visualizer_direction {
            VisualizerDirection::Up => {
                let start_x = rect.center().x - (total_size / 2.0) + self.config.x_offset;
                let bottom_y = rect.bottom() + self.config.y_offset;
                let rect_h = rect.height();

                for i in 0..visual_num_bars {
                    let idx = frequency_index(i, base_num_bars, freq_dir);
                    let mut h = self.current_bars[idx] * sensitivity_multiplier * rect_h;
                    if max_h > 0.0 { h = h.min(max_h); }
                    if h <= 0.0 { continue; }

                    let x = start_x + i as f32 * stride;
                    let bar_rect = egui::Rect::from_min_max(
                        egui::pos2(x, bottom_y - h),
                        egui::pos2(x + bar_width, bottom_y),
                    );

                    if is_vertical_stack {
                        let num_colors = self.config.static_colors.len().max(1);
                        let slice_h = h / num_colors as f32;
                        for c_idx in 0..num_colors {
                            let c = self.config.static_colors.get(c_idx).copied().unwrap_or([255, 255, 255]);
                            let y = bottom_y - h + (c_idx as f32 * slice_h);
                            let slice_rect = egui::Rect::from_min_max(
                                egui::pos2(x, y),
                                egui::pos2(x + bar_width, y + slice_h),
                            );
                            let mut r = egui::Rounding::ZERO;
                            if c_idx == num_colors - 1 { r.nw = radius; r.ne = radius; }
                            else if c_idx == 0 { r.sw = radius; r.se = radius; }
                            painter.rect_filled(slice_rect, r, egui::Color32::from_rgb(c[0], c[1], c[2]));
                        }
                    } else {
                        let color = self.get_bar_color(i, visual_num_bars, bar_rect, rect, time);
                        painter.rect_filled(bar_rect, radius, color);
                    }
                }
            }

            VisualizerDirection::Down => {
                let start_x = rect.center().x - (total_size / 2.0) + self.config.x_offset;
                let top_y = rect.top() + self.config.y_offset;
                let rect_h = rect.height();

                for i in 0..visual_num_bars {
                    let idx = frequency_index(i, base_num_bars, freq_dir);
                    let mut h = self.current_bars[idx] * sensitivity_multiplier * rect_h;
                    if max_h > 0.0 { h = h.min(max_h); }
                    if h <= 0.0 { continue; }

                    let x = start_x + i as f32 * stride;
                    let bar_rect = egui::Rect::from_min_max(
                        egui::pos2(x, top_y),
                        egui::pos2(x + bar_width, top_y + h),
                    );

                    if is_vertical_stack {
                        let num_colors = self.config.static_colors.len().max(1);
                        let slice_h = h / num_colors as f32;
                        for c_idx in 0..num_colors {
                            let c = self.config.static_colors.get(c_idx).copied().unwrap_or([255, 255, 255]);
                            let y = top_y + (c_idx as f32 * slice_h);
                            let slice_rect = egui::Rect::from_min_max(
                                egui::pos2(x, y),
                                egui::pos2(x + bar_width, y + slice_h),
                            );
                            let mut r = egui::Rounding::ZERO;
                            if c_idx == 0 { r.nw = radius; r.ne = radius; }
                            else if c_idx == num_colors - 1 { r.sw = radius; r.se = radius; }
                            painter.rect_filled(slice_rect, r, egui::Color32::from_rgb(c[0], c[1], c[2]));
                        }
                    } else {
                        let color = self.get_bar_color(i, visual_num_bars, bar_rect, rect, time);
                        painter.rect_filled(bar_rect, radius, color);
                    }
                }
            }

            VisualizerDirection::Right => {
                let left_x = rect.left() + self.config.x_offset;
                let start_y = rect.center().y - (total_size / 2.0) + self.config.y_offset;
                let rect_w = rect.width();

                for i in 0..visual_num_bars {
                    let idx = frequency_index(i, base_num_bars, freq_dir);
                    let mut w = self.current_bars[idx] * sensitivity_multiplier * rect_w;
                    if max_h > 0.0 { w = w.min(max_h); }
                    if w <= 0.0 { continue; }

                    let y = start_y + i as f32 * stride;
                    let bar_rect = egui::Rect::from_min_max(
                        egui::pos2(left_x, y),
                        egui::pos2(left_x + w, y + bar_width),
                    );

                    if is_vertical_stack {
                        let num_colors = self.config.static_colors.len().max(1);
                        let slice_w = w / num_colors as f32;
                        for c_idx in 0..num_colors {
                            let c = self.config.static_colors.get(c_idx).copied().unwrap_or([255, 255, 255]);
                            let x = left_x + (c_idx as f32 * slice_w);
                            let slice_rect = egui::Rect::from_min_max(
                                egui::pos2(x, y),
                                egui::pos2(x + slice_w, y + bar_width),
                            );
                            let mut r = egui::Rounding::ZERO;
                            if c_idx == num_colors - 1 { r.ne = radius; r.se = radius; }
                            else if c_idx == 0 { r.nw = radius; r.sw = radius; }
                            painter.rect_filled(slice_rect, r, egui::Color32::from_rgb(c[0], c[1], c[2]));
                        }
                    } else {
                        let color = self.get_bar_color(i, visual_num_bars, bar_rect, rect, time);
                        painter.rect_filled(bar_rect, radius, color);
                    }
                }
            }

            VisualizerDirection::Left => {
                let right_x = rect.right() + self.config.x_offset;
                let start_y = rect.center().y - (total_size / 2.0) + self.config.y_offset;
                let rect_w = rect.width();

                for i in 0..visual_num_bars {
                    let idx = frequency_index(i, base_num_bars, freq_dir);
                    let mut w = self.current_bars[idx] * sensitivity_multiplier * rect_w;
                    if max_h > 0.0 { w = w.min(max_h); }
                    if w <= 0.0 { continue; }

                    let y = start_y + i as f32 * stride;
                    let bar_rect = egui::Rect::from_min_max(
                        egui::pos2(right_x - w, y),
                        egui::pos2(right_x, y + bar_width),
                    );

                    if is_vertical_stack {
                        let num_colors = self.config.static_colors.len().max(1);
                        let slice_w = w / num_colors as f32;
                        for c_idx in 0..num_colors {
                            let c = self.config.static_colors.get(c_idx).copied().unwrap_or([255, 255, 255]);
                            let x = right_x - w + (c_idx as f32 * slice_w);
                            let slice_rect = egui::Rect::from_min_max(
                                egui::pos2(x, y),
                                egui::pos2(x + slice_w, y + bar_width),
                            );
                            let mut r = egui::Rounding::ZERO;
                            if c_idx == 0 { r.ne = radius; r.se = radius; }
                            else if c_idx == num_colors - 1 { r.nw = radius; r.sw = radius; }
                            painter.rect_filled(slice_rect, r, egui::Color32::from_rgb(c[0], c[1], c[2]));
                        }
                    } else {
                        let color = self.get_bar_color(i, visual_num_bars, bar_rect, rect, time);
                        painter.rect_filled(bar_rect, radius, color);
                    }
                }
            }
        }
    }

    fn draw_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Performance & Optimization");
        ui.checkbox(&mut self.config.use_vsync, "Enable VSync (Requires Restart)");
        ui.add(egui::Slider::new(&mut self.config.target_fps, 15.0..=144.0).text("Max Target FPS"));
        ui.add(egui::Slider::new(&mut self.config.audio_update_ms, 10..=100).text("Audio Poll Rate (ms)"));
        ui.label("Higher ms = lower CPU usage but more input latency.");
        ui.checkbox(&mut self.config.show_fps, "Show FPS Counter");
        ui.label(format!("Current FPS: {:.1}", self.current_fps));

        ui.separator();
        ui.heading("Visualizer Direction");
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.config.visualizer_direction, VisualizerDirection::Up, "Up");
            ui.radio_value(&mut self.config.visualizer_direction, VisualizerDirection::Down, "Down");
            ui.radio_value(&mut self.config.visualizer_direction, VisualizerDirection::Left, "Left");
            ui.radio_value(&mut self.config.visualizer_direction, VisualizerDirection::Right, "Right");
        });
        ui.label("Direction the bars grow from their anchor edge.");

        ui.separator();
        ui.heading("Frequency Direction");
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.config.frequency_direction, FrequencyDirection::LeftToRight, "Left To Right");
            ui.radio_value(&mut self.config.frequency_direction, FrequencyDirection::RightToLeft, "Right To Left");
            ui.radio_value(&mut self.config.frequency_direction, FrequencyDirection::Mirrored, "Mirrored (Symmetrical)");
        });

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
        ui.horizontal_wrapped(|ui| {
            ui.radio_value(&mut self.config.color_mode, ColorMode::Dynamic, "Dynamic");
            ui.radio_value(&mut self.config.color_mode, ColorMode::StaticSingle, "Static Single");
            ui.radio_value(&mut self.config.color_mode, ColorMode::StaticMultiple, "Static Multi");
            ui.radio_value(&mut self.config.color_mode, ColorMode::Gradient, "Gradient");
            ui.radio_value(&mut self.config.color_mode, ColorMode::ColorChanging, "Color Changing");
        });

        ui.separator();

        match self.config.color_mode {
            ColorMode::StaticSingle => {
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

            ColorMode::StaticMultiple => {
                ui.heading("Multiple Static Colors");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.config.static_direction, DistributionDirection::Horizontal, "Horizontal (Across Bars)");
                    ui.radio_value(&mut self.config.static_direction, DistributionDirection::Vertical, "Vertical (Inside Bars)");
                });

                let mut to_remove = None;
                // Fix: cache length to avoid immutable borrow while iterating mutably
                let current_len = self.config.static_colors.len();
                for (i, color) in self.config.static_colors.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.color_edit_button_srgb(color);
                        if ui.button("❌").clicked() { to_remove = Some(i); }
                    });
                }
                if let Some(i) = to_remove { self.config.static_colors.remove(i); }
                if ui.button("➕ Add Color").clicked() {
                    self.config.static_colors.push([255, 255, 255]);
                }

                ui.label("Presets:");
                ui.horizontal_wrapped(|ui| {
                    let mut presets_to_apply = None;
                    for (i, preset) in self.config.static_multi_presets.iter().enumerate() {
                        if ui.button(format!("Load {}", i + 1)).clicked() {
                            presets_to_apply = Some(preset.clone());
                        }
                    }
                    if let Some(p) = presets_to_apply {
                        self.config.static_colors = p.colors;
                        self.config.static_direction = p.direction;
                    }
                    if ui.button("💾 Save Preset").clicked() {
                        self.config.static_multi_presets.push(StaticMultiPreset {
                            colors: self.config.static_colors.clone(),
                            direction: self.config.static_direction,
                        });
                    }
                });
            }

            ColorMode::Gradient => {
                ui.heading("Gradient Colors");
                ui.add(egui::Slider::new(&mut self.config.gradient_angle, 0.0..=360.0).text("Angle (°)"));

                // Gradient Bar Preview & Interaction
                let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::click_and_drag());
                let painter = ui.painter();

                // Draw the gradient background
                let steps = 100;
                for i in 0..steps {
                    let t1 = i as f32 / steps as f32;
                    let t2 = (i + 1) as f32 / steps as f32;
                    let c1 = color::sample_gradient(&self.config.gradient_stops, t1);
                    let x1 = rect.left() + rect.width() * t1;
                    let x2 = rect.left() + rect.width() * t2;
                    let r = egui::Rect::from_min_max(egui::pos2(x1, rect.top()), egui::pos2(x2, rect.bottom()));
                    painter.rect_filled(r, 0.0, egui::Color32::from_rgb(c1[0], c1[1], c1[2]));
                }

                // Draw the handles for stops
                for (i, stop) in self.config.gradient_stops.iter_mut().enumerate() {
                    let x = rect.left() + rect.width() * stop.position;
                    let stop_rect = egui::Rect::from_center_size(egui::pos2(x, rect.center().y), egui::vec2(12.0, 30.0));
                    let stop_response = ui.interact(stop_rect, ui.id().with("gradient_stop").with(i), egui::Sense::click_and_drag());
                    
                    if stop_response.dragged() {
                        stop.position = (stop.position + stop_response.drag_delta().x / rect.width()).clamp(0.0, 1.0);
                    }
                    
                    let handle_color = egui::Color32::from_rgb(stop.color[0], stop.color[1], stop.color[2]);
                    painter.rect_filled(stop_rect, 2.0, handle_color);
                    painter.rect_stroke(stop_rect, 2.0_f32, egui::Stroke::new(2.0_f32, egui::Color32::WHITE)); // Fix: explicitly typed as f32
                }

                // Double-click to add a stop
                if response.double_clicked() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let t = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                        let c = color::sample_gradient(&self.config.gradient_stops, t);
                        self.config.gradient_stops.push(GradientStop { color: c, position: t });
                        self.config.gradient_stops.sort_by(|a, b| a.position.partial_cmp(&b.position).unwrap());
                    }
                }
                ui.label("Double-click the bar to add a color stop.");

                // List of stops
                let mut to_remove = None;
                // Fix: cache length to avoid immutable borrow while iterating mutably
                let current_len = self.config.gradient_stops.len();
                for (i, stop) in self.config.gradient_stops.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.color_edit_button_srgb(&mut stop.color);
                        ui.label(format!("Pos: {:.2}", stop.position));
                        if current_len > 2 && ui.button("❌").clicked() {
                            to_remove = Some(i);
                        }
                    });
                }
                if let Some(i) = to_remove { self.config.gradient_stops.remove(i); }

                // Presets
                ui.label("Presets:");
                ui.horizontal_wrapped(|ui| {
                    let mut presets_to_apply = None;
                    for (i, preset) in self.config.gradient_presets.iter().enumerate() {
                        if ui.button(format!("Load {}", i + 1)).clicked() {
                            presets_to_apply = Some(preset.clone());
                        }
                    }
                    if let Some(p) = presets_to_apply {
                        self.config.gradient_stops = p.stops;
                        self.config.gradient_angle = p.angle;
                    }
                    if ui.button("💾 Save Preset").clicked() {
                        self.config.gradient_presets.push(GradientPreset {
                            stops: self.config.gradient_stops.clone(),
                            angle: self.config.gradient_angle,
                        });
                    }
                });
            }

            ColorMode::ColorChanging => {
                ui.heading("Color Changing");
                let mut to_remove = None;
                // Fix: cache length to avoid immutable borrow while iterating mutably
                let current_len = self.config.color_change_colors.len();
                for (i, color) in self.config.color_change_colors.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.color_edit_button_srgb(color);
                        if current_len > 2 && ui.button("❌").clicked() {
                            to_remove = Some(i);
                        }
                    });
                }
                if let Some(i) = to_remove { self.config.color_change_colors.remove(i); }
                if ui.button("➕ Add Color").clicked() {
                    self.config.color_change_colors.push([255, 255, 255]);
                }

                ui.add(egui::Slider::new(&mut self.config.color_change_speed, 0.0..=1.0)
                    .text("Speed")
                    .custom_formatter(|n, _| format!("{:.0}%", n * 100.0)));

                ui.label("Presets:");
                ui.horizontal_wrapped(|ui| {
                    let mut presets_to_apply = None;
                    for (i, preset) in self.config.color_change_presets.iter().enumerate() {
                        if ui.button(format!("Load {}", i + 1)).clicked() {
                            presets_to_apply = Some(preset.clone());
                        }
                    }
                    if let Some(p) = presets_to_apply {
                        self.config.color_change_colors = p.colors;
                        self.config.color_change_speed = p.speed;
                    }
                    if ui.button("💾 Save Preset").clicked() {
                        self.config.color_change_presets.push(ColorChangePreset {
                            colors: self.config.color_change_colors.clone(),
                            speed: self.config.color_change_speed,
                        });
                    }
                });
            }

            _ => {}
        }

        ui.separator();
        ui.heading("Appearance");
        ui.add(egui::Slider::new(&mut self.config.bg_opacity, 0..=255).text("Background Opacity"));

        ui.separator();
        if ui.button("Reset to Defaults").clicked() {
            self.config = AppConfig::default();
            self.audio_target_bars.store(self.config.num_bars, Ordering::Relaxed);
            self.audio_update_interval.store(self.config.audio_update_ms, Ordering::Relaxed);
            self.current_bars = vec![0.0; self.config.num_bars];
        }
    }
}

fn main() -> eframe::Result<()> {
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
