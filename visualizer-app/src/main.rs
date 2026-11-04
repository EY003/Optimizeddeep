#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use visualizer_core::{
    audio::AudioEngine,
    color,
    config::{
        AppConfig, BlendMode, ColorChangePreset, ColorMode, FrequencyDirection, GradientPreset,
        GradientStop, MotionShape, StaticDistributionMode, StaticMultiPreset, VisualizerDirection,
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

/// MurmurHash3 32-bit finalizer — excellent avalanche & uniformity.
fn hash_u32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    x = x.wrapping_mul(0xC2B2_AE35);
    x ^= x >> 16;
    x
}

/// Triangle wave 0 → 1 → 0 over t in [0, 1).
fn tri01(t: f32) -> f32 {
    1.0 - (2.0 * t.fract() - 1.0).abs()
}

/// Smoothstep with graceful handling of reversed/zero-width edges.
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Interpolate a wipe boundary into a 0..=1 blend factor.
fn wipe_blend(pos: f32, boundary: f32, softness: f32, hard: bool) -> f32 {
    if hard {
        if pos < boundary { 0.0 } else { 1.0 }
    } else {
        smoothstep(boundary - softness, boundary + softness, pos)
    }
}

/// Everything the color-sampling layer needs to turn one (x, y, t) point
/// into an egui colour. Two palette phases plus a blend factor and an
/// intensity (brightness) multiplier.
#[derive(Clone, Copy)]
struct MotionSample {
    phase_a: f32,
    phase_b: f32,
    blend_t: f32,
    intensity: f32,
}

impl Default for MotionSample {
    fn default() -> Self {
        Self { phase_a: 0.0, phase_b: 0.0, blend_t: 0.0, intensity: 1.0 }
    }
}

/// MurmurHash-based random index (used by the `Random` static distribution mode).
fn random_color_index(i: usize, num_colors: usize) -> usize {
    let mut x = i as u32;
    x ^= x >> 16;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    x = x.wrapping_mul(0xC2B2_AE35);
    x ^= x >> 16;
    (x as usize) % num_colors
}

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

    new_preset_name: String,

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
            new_preset_name: String::new(),
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

        if self.show_settings {
            let settings_builder = egui::ViewportBuilder::default()
                .with_title("Visualizer Settings")
                .with_inner_size([400.0, 900.0])
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
    // ─────────────────────────────────────────────────────────
    // Color Changing Motion Engine
    // ─────────────────────────────────────────────────────────

    /// Evaluates a motion shape at a given point `(x, y)` in normalized
    /// coordinates (both in [0, 1]) and a "base time" `base_t` that already
    /// has the speed factor folded in.
    fn evaluate_motion(
        &self,
        shape: MotionShape,
        x: f32,
        y: f32,
        time: f32,
        base_t: f32,
    ) -> MotionSample {
        let m = &self.config.color_change_motion;
        let hard = m.blend == BlendMode::Hard;
        let freq = m.wave_frequency;

        match shape {
            // ── Uniform ─────────────────────────────────────
            MotionShape::Uniform => MotionSample {
                phase_a: base_t,
                intensity: 1.0,
                ..Default::default()
            },

            // ── Directional Wipes ───────────────────────────
            MotionShape::HorizontalWipe => {
                let boundary = tri01(base_t * 0.5);
                let t = wipe_blend(x, boundary, m.wipe_softness, hard);
                MotionSample {
                    phase_a: base_t,
                    phase_b: base_t + 0.5,
                    blend_t: t,
                    intensity: 1.0,
                }
            }
            MotionShape::VerticalWipe => {
                let boundary = tri01(base_t * 0.5);
                let t = wipe_blend(y, boundary, m.wipe_softness, hard);
                MotionSample {
                    phase_a: base_t,
                    phase_b: base_t + 0.5,
                    blend_t: t,
                    intensity: 1.0,
                }
            }
            MotionShape::DiagonalWipe => {
                let boundary = tri01(base_t * 0.5);
                let diag = (x + y) * 0.5;
                let t = wipe_blend(diag, boundary, m.wipe_softness, hard);
                MotionSample {
                    phase_a: base_t,
                    phase_b: base_t + 0.5,
                    blend_t: t,
                    intensity: 1.0,
                }
            }

            // ── Wave-based ──────────────────────────────────
            MotionShape::SineWave => {
                let w = (x * freq * TAU + base_t * TAU).sin() * 0.5 + 0.5;
                MotionSample {
                    phase_a: base_t + w * 0.5,
                    intensity: 1.0,
                    ..Default::default()
                }
            }
            MotionShape::TriangleWave => {
                let w = tri01(x * freq + base_t);
                MotionSample {
                    phase_a: base_t + w * 0.5,
                    intensity: 1.0,
                    ..Default::default()
                }
            }
            MotionShape::SquareWave => {
                let w = ((x * freq + base_t).fract() < 0.5) as u8 as f32;
                MotionSample {
                    phase_a: base_t + w * 0.5,
                    intensity: 1.0,
                    ..Default::default()
                }
            }
            MotionShape::SawtoothWave => {
                let w = (x * freq + base_t).fract();
                MotionSample {
                    phase_a: base_t + w * 0.5,
                    intensity: 1.0,
                    ..Default::default()
                }
            }

            // ── Stutter & Pulse ─────────────────────────────
            MotionShape::Strobe => {
                let on = ((base_t * 4.0).fract() < 0.5) as u8 as f32;
                let k = if hard {
                    on
                } else {
                    smoothstep(0.35, 0.65, on + base_t.fract() * 0.3)
                };
                MotionSample {
                    phase_a: base_t,
                    intensity: k,
                    ..Default::default()
                }
            }
            MotionShape::Pulse => {
                let k = (base_t * TAU).sin() * 0.5 + 0.5;
                MotionSample {
                    phase_a: base_t,
                    intensity: k,
                    ..Default::default()
                }
            }
            MotionShape::Heartbeat => {
                let beat = base_t.fract();
                let k = if hard {
                    if beat < 0.10 {
                        1.0
                    } else if beat < 0.18 {
                        0.0
                    } else if beat < 0.26 {
                        0.7
                    } else {
                        0.0
                    }
                } else {
                    let a = (-((beat - 0.08) / 0.04).powi(2)).exp();
                    let b = (-((beat - 0.22) / 0.04).powi(2)).exp() * 0.7;
                    (a + b).clamp(0.0, 1.0)
                };
                MotionSample {
                    phase_a: base_t,
                    intensity: k,
                    ..Default::default()
                }
            }
            MotionShape::Stutter => {
                let step = (time * 10.0) as u32;
                let pos = (x * 200.0) as u32;
                let hash = hash_u32(step.wrapping_mul(37).wrapping_add(pos));
                let event = (hash % 100) as f32 / 100.0 < m.stutter_intensity;
                let offset = if event { 0.5 } else { 0.0 };
                MotionSample {
                    phase_a: base_t + offset,
                    intensity: 1.0,
                    ..Default::default()
                }
            }

            // ── Particle & Trail ────────────────────────────
            MotionShape::Comet => {
                let head = base_t.fract();
                let raw = (x - head).abs();
                let dist = raw.min(1.0 - raw);
                let k = (1.0 - dist / m.trail_length.max(0.01)).clamp(0.0, 1.0);
                let k = if hard { (k > 0.5) as u8 as f32 } else { k };
                MotionSample {
                    phase_a: base_t,
                    intensity: k,
                    ..Default::default()
                }
            }
            MotionShape::Chase => {
                let pos = (x - base_t * 0.5).rem_euclid(1.0);
                let k = if hard {
                    (pos < 0.15) as u8 as f32
                } else {
                    smoothstep(0.05, 0.15, 0.15 - (pos - 0.075).abs())
                };
                MotionSample {
                    phase_a: base_t,
                    intensity: k,
                    ..Default::default()
                }
            }
            MotionShape::BouncingBall => {
                let head = tri01(base_t * 0.5);
                let dist = (x - head).abs();
                let k = (1.0 - dist / m.trail_length.max(0.01)).clamp(0.0, 1.0);
                let k = if hard { (k > 0.4) as u8 as f32 } else { k };
                MotionSample {
                    phase_a: base_t,
                    intensity: k,
                    ..Default::default()
                }
            }
            MotionShape::Sparkle => {
                let step = (time * 12.0) as u32;
                let pos = (x * 300.0) as u32;
                let hash = hash_u32(step.wrapping_mul(13).wrapping_add(pos));
                let p = (hash % 100) as f32 / 100.0;
                let on = p < 0.04 + m.stutter_intensity * 0.1;
                MotionSample {
                    phase_a: base_t + p,
                    intensity: if on { 1.0 } else { 0.0 },
                    ..Default::default()
                }
            }
            MotionShape::Sinelon => {
                let head = tri01(base_t * 0.5);
                let dist = (x - head).abs();
                let k = (1.0 - dist / m.trail_length.max(0.01)).clamp(0.0, 1.0);
                MotionSample {
                    phase_a: base_t,
                    intensity: k.powf(2.0),
                    ..Default::default()
                }
            }

            // ── Complex & Organic ───────────────────────────
            MotionShape::FireSimulation => {
                let n = (base_t * 1.0 + x * 5.0).sin()
                    + (base_t * 1.3 + x * 11.0).sin() * 0.5
                    + (base_t * 2.1 + x * 23.0).sin() * 0.25;
                MotionSample {
                    phase_a: base_t + n * 0.05,
                    intensity: (n * 0.25 + 0.75).clamp(0.0, 1.0),
                    ..Default::default()
                }
            }
            MotionShape::Plasma => {
                let p = (x * 6.0 + base_t * 2.0).sin()
                    + (y * 6.0 - base_t * 1.5).sin()
                    + ((x + y) * 4.0 + base_t).sin();
                MotionSample {
                    phase_a: base_t + p * 0.08,
                    intensity: 1.0,
                    ..Default::default()
                }
            }
            MotionShape::RainbowCycle => MotionSample {
                phase_a: base_t + x,
                intensity: 1.0,
                ..Default::default()
            },
            MotionShape::Confetti => {
                let step = (time * 10.0) as u32;
                let pos = (x * 400.0) as u32;
                let hash = hash_u32(step.wrapping_mul(97).wrapping_add(pos));
                let p = (hash % 100) as f32 / 100.0;
                let on = p < m.stutter_intensity * 0.15;
                MotionSample {
                    phase_a: (hash % 1000) as f32 / 1000.0 + base_t * 0.1,
                    intensity: if on { 1.0 } else { 0.0 },
                    ..Default::default()
                }
            }
        }
    }

    /// Turn a motion sample into an `egui::Color32` by looking up the palette.
    fn sample_color_changing(&self, x: f32, y: f32, time: f32) -> egui::Color32 {
        if self.config.color_change_colors.is_empty() {
            return egui::Color32::WHITE;
        }

        let speed_hz = 0.05 + (self.config.color_change_speed * 1.95);
        let effective_speed = speed_hz * self.config.color_change_motion.speed;
        let base_t = time * effective_speed;

        let s = self.evaluate_motion(
            self.config.color_change_motion.shape,
            x,
            y,
            time,
            base_t,
        );

        let ca = color::cyclic_color(&self.config.color_change_colors, s.phase_a);
        let cb = color::cyclic_color(&self.config.color_change_colors, s.phase_b);
        let t = s.blend_t.clamp(0.0, 1.0);
        let k = s.intensity.clamp(0.0, 1.0);

        let r = (ca[0] as f32 * (1.0 - t) + cb[0] as f32 * t) * k;
        let g = (ca[1] as f32 * (1.0 - t) + cb[1] as f32 * t) * k;
        let b = (ca[2] as f32 * (1.0 - t) + cb[2] as f32 * t) * k;

        egui::Color32::from_rgb(r as u8, g as u8, b as u8)
    }

    // ─────────────────────────────────────────────────────────
    // Legacy per-bar color resolution (unchanged except ColorChanging)
    // ─────────────────────────────────────────────────────────

    fn get_bar_color(
        &self,
        i: usize,
        total_bars: usize,
        bar_rect: egui::Rect,
        screen_rect: egui::Rect,
        time: f32,
    ) -> egui::Color32 {
        match self.config.color_mode {
            ColorMode::Dynamic => egui::Color32::from_rgb(180, 200, 180),
            ColorMode::StaticSingle => egui::Color32::from_rgb(
                self.config.bar_color[0],
                self.config.bar_color[1],
                self.config.bar_color[2],
            ),
            ColorMode::StaticMultiple => {
                let num_colors = self.config.static_colors.len().max(1);
                if self.config.static_colors.is_empty() {
                    return egui::Color32::WHITE;
                }

                let idx = match self.config.static_mode {
                    StaticDistributionMode::Blocks => {
                        let mut chunk =
                            (i as f32 / total_bars as f32 * num_colors as f32) as usize;
                        if chunk >= num_colors {
                            chunk = num_colors - 1;
                        }
                        chunk
                    }
                    StaticDistributionMode::Sequential => {
                        let bars_per_color = self.config.sequential_bars_per_color.max(1);
                        (i / bars_per_color) % num_colors
                    }
                    StaticDistributionMode::Random => random_color_index(i, num_colors),
                    StaticDistributionMode::Vertical => 0,
                };

                let c = self
                    .config
                    .static_colors
                    .get(idx)
                    .copied()
                    .unwrap_or([255, 255, 255]);
                egui::Color32::from_rgb(c[0], c[1], c[2])
            }
            ColorMode::Gradient => {
                let angle_rad = self.config.gradient_angle.to_radians();
                let dir = egui::vec2(angle_rad.cos(), angle_rad.sin());

                let corners = [
                    screen_rect.left_top(),
                    screen_rect.right_top(),
                    screen_rect.left_bottom(),
                    screen_rect.right_bottom(),
                ];
                let mut min_proj = f32::MAX;
                let mut max_proj = f32::MIN;
                for c in corners {
                    let p = c.x * dir.x + c.y * dir.y;
                    min_proj = min_proj.min(p);
                    max_proj = max_proj.max(p);
                }

                let center = bar_rect.center();
                let bar_proj = center.x * dir.x + center.y * dir.y;

                let t = if (max_proj - min_proj).abs() > 0.0001 {
                    ((bar_proj - min_proj) / (max_proj - min_proj)).clamp(0.0, 1.0)
                } else {
                    0.0
                };

                let c = color::sample_gradient(&self.config.gradient_stops, t);
                egui::Color32::from_rgb(c[0], c[1], c[2])
            }
            ColorMode::ColorChanging => {
                let x = if total_bars > 1 {
                    i as f32 / (total_bars - 1) as f32
                } else {
                    0.5
                };
                self.sample_color_changing(x, 0.5, time)
            }
        }
    }

    // ─────────────────────────────────────────────────────────
    // Renderers
    // ─────────────────────────────────────────────────────────

    /// Renders a single bar as a gradient mesh (Gradient mode).
    fn draw_gradient_bar(
        &self,
        painter: &egui::Painter,
        bar_rect: egui::Rect,
        screen_rect: egui::Rect,
    ) {
        let angle_rad = self.config.gradient_angle.to_radians();
        let dir = egui::vec2(angle_rad.cos(), angle_rad.sin());

        let screen_corners = [
            screen_rect.left_top(),
            screen_rect.right_top(),
            screen_rect.left_bottom(),
            screen_rect.right_bottom(),
        ];
        let mut min_proj = f32::MAX;
        let mut max_proj = f32::MIN;
        for c in screen_corners {
            let p = c.x * dir.x + c.y * dir.y;
            min_proj = min_proj.min(p);
            max_proj = max_proj.max(p);
        }
        let range = (max_proj - min_proj).max(0.0001);

        let corners = [
            bar_rect.left_top(),
            bar_rect.right_top(),
            bar_rect.right_bottom(),
            bar_rect.left_bottom(),
        ];

        let mut mesh = egui::Mesh::default();
        for corner in corners {
            let t =
                ((corner.x * dir.x + corner.y * dir.y - min_proj) / range).clamp(0.0, 1.0);
            let c = color::sample_gradient(&self.config.gradient_stops, t);
            mesh.vertices.push(egui::epaint::Vertex {
                pos: corner,
                uv: egui::Pos2::ZERO,
                color: egui::Color32::from_rgb(c[0], c[1], c[2]),
            });
        }
        mesh.indices.extend_from_slice(&[0u32, 1, 2, 0, 2, 3]);

        painter.add(egui::Shape::mesh(mesh));
    }

    /// Renders a ColorChanging bar. If the active motion shape reads the
    /// vertical position (VerticalWipe / DiagonalWipe), the bar is subdivided
    /// vertically so motion becomes visible *inside* the bar. Otherwise it's
    /// a plain single-coloured rect.
    fn draw_color_changing_bar(
        &self,
        painter: &egui::Painter,
        bar_rect: egui::Rect,
        x: f32,
        time: f32,
        radius: f32,
        round_top_bottom: bool,
    ) {
        if !self.config.color_change_motion.shape.uses_vertical_axis() {
            let c = self.sample_color_changing(x, 0.5, time);
            painter.rect_filled(bar_rect, radius, c);
            return;
        }

        let h = bar_rect.height();
        if h <= 0.0 {
            return;
        }

        // Adaptive subdivision: ~6px per segment, between 8 and 64 segments.
        let segments = ((h / 6.0) as usize).clamp(8, 64);

        for k in 0..segments {
            let v0 = k as f32 / segments as f32;
            let v1 = (k + 1) as f32 / segments as f32;
            let v_mid = (v0 + v1) * 0.5;

            // v = 0 is the bottom of the bar rect, v = 1 is the top.
            let y_top = bar_rect.bottom() - h * v1;
            let y_bot = bar_rect.bottom() - h * v0;

            let seg_rect = egui::Rect::from_min_max(
                egui::pos2(bar_rect.left(), y_top),
                egui::pos2(bar_rect.right(), y_bot),
            );

            let rounding = if round_top_bottom {
                if k == segments - 1 {
                    egui::Rounding { nw: radius, ne: radius, sw: 0.0, se: 0.0 }
                } else if k == 0 {
                    egui::Rounding { nw: 0.0, ne: 0.0, sw: radius, se: radius }
                } else {
                    egui::Rounding::ZERO
                }
            } else {
                egui::Rounding::ZERO
            };

            let c = self.sample_color_changing(x, v_mid, time);
            painter.rect_filled(seg_rect, rounding, c);
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

        let is_vertical_stack = self.config.color_mode == ColorMode::StaticMultiple
            && self.config.static_mode == StaticDistributionMode::Vertical;
        let is_gradient = self.config.color_mode == ColorMode::Gradient;
        let is_color_changing_vertical = self.config.color_mode == ColorMode::ColorChanging
            && self.config.color_change_motion.shape.uses_vertical_axis();

        // Normalized x for a bar index (shared across directions).
        let x_norm = |i: usize| -> f32 {
            if visual_num_bars > 1 {
                i as f32 / (visual_num_bars - 1) as f32
            } else {
                0.5
            }
        };

        match self.config.visualizer_direction {
            VisualizerDirection::Up => {
                let start_x = rect.center().x - (total_size / 2.0) + self.config.x_offset;
                let bottom_y = rect.bottom() + self.config.y_offset;
                let rect_h = rect.height();

                for i in 0..visual_num_bars {
                    let idx = frequency_index(i, base_num_bars, freq_dir);
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

                    if is_vertical_stack {
                        let num_colors = self.config.static_colors.len().max(1);
                        let slice_h = h / num_colors as f32;
                        for c_idx in 0..num_colors {
                            let c = self
                                .config
                                .static_colors
                                .get(c_idx)
                                .copied()
                                .unwrap_or([255, 255, 255]);
                            let y = bottom_y - h + (c_idx as f32 * slice_h);
                            let slice_rect = egui::Rect::from_min_max(
                                egui::pos2(x, y),
                                egui::pos2(x + bar_width, y + slice_h),
                            );
                            let mut r = egui::Rounding::ZERO;
                            if c_idx == num_colors - 1 {
                                r.nw = radius;
                                r.ne = radius;
                            } else if c_idx == 0 {
                                r.sw = radius;
                                r.se = radius;
                            }
                            painter.rect_filled(
                                slice_rect,
                                r,
                                egui::Color32::from_rgb(c[0], c[1], c[2]),
                            );
                        }
                    } else if is_color_changing_vertical {
                        self.draw_color_changing_bar(
                            painter, bar_rect, x_norm(i), time, radius, true,
                        );
                    } else if is_gradient {
                        self.draw_gradient_bar(painter, bar_rect, rect);
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

                    if is_vertical_stack {
                        let num_colors = self.config.static_colors.len().max(1);
                        let slice_h = h / num_colors as f32;
                        for c_idx in 0..num_colors {
                            let c = self
                                .config
                                .static_colors
                                .get(c_idx)
                                .copied()
                                .unwrap_or([255, 255, 255]);
                            let y = top_y + (c_idx as f32 * slice_h);
                            let slice_rect = egui::Rect::from_min_max(
                                egui::pos2(x, y),
                                egui::pos2(x + bar_width, y + slice_h),
                            );
                            let mut r = egui::Rounding::ZERO;
                            if c_idx == 0 {
                                r.nw = radius;
                                r.ne = radius;
                            } else if c_idx == num_colors - 1 {
                                r.sw = radius;
                                r.se = radius;
                            }
                            painter.rect_filled(
                                slice_rect,
                                r,
                                egui::Color32::from_rgb(c[0], c[1], c[2]),
                            );
                        }
                    } else if is_color_changing_vertical {
                        self.draw_color_changing_bar(
                            painter, bar_rect, x_norm(i), time, radius, true,
                        );
                    } else if is_gradient {
                        self.draw_gradient_bar(painter, bar_rect, rect);
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

                    if is_vertical_stack {
                        let num_colors = self.config.static_colors.len().max(1);
                        let slice_w = w / num_colors as f32;
                        for c_idx in 0..num_colors {
                            let c = self
                                .config
                                .static_colors
                                .get(c_idx)
                                .copied()
                                .unwrap_or([255, 255, 255]);
                            let x = left_x + (c_idx as f32 * slice_w);
                            let slice_rect = egui::Rect::from_min_max(
                                egui::pos2(x, y),
                                egui::pos2(x + slice_w, y + bar_width),
                            );
                            let mut r = egui::Rounding::ZERO;
                            if c_idx == num_colors - 1 {
                                r.ne = radius;
                                r.se = radius;
                            } else if c_idx == 0 {
                                r.nw = radius;
                                r.sw = radius;
                            }
                            painter.rect_filled(
                                slice_rect,
                                r,
                                egui::Color32::from_rgb(c[0], c[1], c[2]),
                            );
                        }
                    } else if is_color_changing_vertical {
                        self.draw_color_changing_bar(
                            painter, bar_rect, x_norm(i), time, radius, false,
                        );
                    } else if is_gradient {
                        self.draw_gradient_bar(painter, bar_rect, rect);
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

                    if is_vertical_stack {
                        let num_colors = self.config.static_colors.len().max(1);
                        let slice_w = w / num_colors as f32;
                        for c_idx in 0..num_colors {
                            let c = self
                                .config
                                .static_colors
                                .get(c_idx)
                                .copied()
                                .unwrap_or([255, 255, 255]);
                            let x = right_x - w + (c_idx as f32 * slice_w);
                            let slice_rect = egui::Rect::from_min_max(
                                egui::pos2(x, y),
                                egui::pos2(x + slice_w, y + bar_width),
                            );
                            let mut r = egui::Rounding::ZERO;
                            if c_idx == 0 {
                                r.ne = radius;
                                r.se = radius;
                            } else if c_idx == num_colors - 1 {
                                r.nw = radius;
                                r.sw = radius;
                            }
                            painter.rect_filled(
                                slice_rect,
                                r,
                                egui::Color32::from_rgb(c[0], c[1], c[2]),
                            );
                        }
                    } else if is_color_changing_vertical {
                        self.draw_color_changing_bar(
                            painter, bar_rect, x_norm(i), time, radius, false,
                        );
                    } else if is_gradient {
                        self.draw_gradient_bar(painter, bar_rect, rect);
                    } else {
                        let color = self.get_bar_color(i, visual_num_bars, bar_rect, rect, time);
                        painter.rect_filled(bar_rect, radius, color);
                    }
                }
            }
        }
    }

    // ─────────────────────────────────────────────────────────
    // Settings UI
    // ─────────────────────────────────────────────────────────

    fn draw_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Performance & Optimization");
        ui.checkbox(&mut self.config.use_vsync, "Enable VSync (Requires Restart)");
        ui.add(egui::Slider::new(&mut self.config.target_fps, 15.0..=144.0).text("Max Target FPS"));
        ui.add(egui::Slider::new(&mut self.config.audio_update_ms, 10..=100).text("Audio Poll Rate (ms)"));
        ui.label("Higher ms = lower CPU usage but more input latency.");
        ui.checkbox(&mut self.config.show_fps, "Show FPS Counter");
        ui.label(format!("Current FPS: {:.1}", self.current_fps));

        ui.separator();
       
