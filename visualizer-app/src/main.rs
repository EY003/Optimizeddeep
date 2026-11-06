#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use visualizer_core::{
    audio::AudioEngine,
    color,
    config::{
        AppConfig, BlendMode, ColorChangePreset, ColorMode, FontStyle, FrequencyDirection,
        GradientPreset, GradientStop, MotionShape, StaticDistributionMode, StaticMultiPreset,
        TrackFontFamily, TrackInfoConfig, TrackTextAlign, TrackTextColorMode,
        VisualizerDirection,
    },
    musicbee::{self, TrackInfo},
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

fn tri01(t: f32) -> f32 {
    1.0 - (2.0 * t.fract() - 1.0).abs()
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn wipe_blend(pos: f32, boundary: f32, softness: f32, hard: bool) -> f32 {
    if hard {
        if pos < boundary { 0.0 } else { 1.0 }
    } else {
        smoothstep(boundary - softness, boundary + softness, pos)
    }
}

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

fn random_color_index(i: usize, num_colors: usize) -> usize {
    let mut x = i as u32;
    x ^= x >> 16;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    x = x.wrapping_mul(0xC2B2_AE35);
    x ^= x >> 16;
    (x as usize) % num_colors
}

// ─────────────────────────────────────────────────────────────
// Custom track-text font plumbing
// ─────────────────────────────────────────────────────────────

/// Family name used for the user-supplied custom font. Kept distinct from
/// egui's built-in `Proportional` so selecting "Default" never accidentally
/// uses the custom font.
const CUSTOM_FONT_FAMILY: &str = "custom_track";
const CUSTOM_FONT_KEY: &str = "track_custom";

fn custom_font_family() -> egui::FontFamily {
    egui::FontFamily::Name(CUSTOM_FONT_FAMILY.into())
}

// ─────────────────────────────────────────────────────────────
// Main app struct
// ─────────────────────────────────────────────────────────────

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

    // ── Track / cover art state ──────────────────────────────
    track_rx: crossbeam_channel::Receiver<TrackInfo>,
    track_info: Option<TrackInfo>,
    cover_texture: Option<egui::TextureHandle>,
    dynamic_color: [u8; 3],

    /// Last `custom_font_path` we successfully (or unsuccessfully) tried to
    /// load. Sentinel `Some("__init__")` forces one reload on the first frame.
    loaded_font_path: Option<String>,

    /// Live count of SMTC sessions visible to Windows (for diagnostics).
    smtc_session_count: Arc<AtomicUsize>,

    _tray_icon: TrayIcon,
    _hotkey_manager: GlobalHotKeyManager,
    settings_menu_id: tray_icon::menu::MenuId,
    quit_menu_id: tray_icon::menu::MenuId,
}

impl VisualizerApp {
    fn new() -> Self {
        let config = AppConfig::load();
        let engine = AudioEngine::new(config.num_bars, config.audio_update_ms);

        // Explicitly set to false before spawning the thread.
        let should_quit = Arc::new(AtomicBool::new(false));

        // Log to confirm the app starts and the flag is false.
        musicbee::log_smtc("APP: Initializing VisualizerApp. should_quit = false.");

        let audio_target_bars = engine.target_bars.clone();
        let audio_update_interval = engine.update_interval_ms.clone();

        // Spawn the SMTC listener
        let (track_tx, track_rx) = crossbeam_channel::unbounded();
        let smtc_session_count = Arc::new(AtomicUsize::new(0));
        musicbee::spawn_smtc_listener(
            track_tx,
            should_quit.clone(),
            smtc_session_count.clone(),
        );

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

            track_rx,
            track_info: None,
            cover_texture: None,
            dynamic_color: [180, 200, 180],
            loaded_font_path: Some("__init__".to_string()),
            smtc_session_count,

            _tray_icon: tray_icon,
            _hotkey_manager: hotkey_manager,
            settings_menu_id: settings_item.id().clone(),
            quit_menu_id: quit_item.id().clone(),
        }
    }

    /// Drain the SMTC channel, decode artwork, extract colors, and rebuild
    /// the texture.
    fn poll_track_updates(&mut self, ctx: &egui::Context) {
        while let Ok(info) = self.track_rx.try_recv() {
            // Decode artwork + extract dominant color
            if let Some(bytes) = &info.cover_art {
                if let Some((colors, img)) = color::extract_colors_from_bytes(bytes) {
                    self.dynamic_color = colors.dominant;

                    let rgba = img.to_rgba8();
                    let size = [rgba.width() as usize, rgba.height() as usize];
                    let color_image =
                        egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                    self.cover_texture = Some(ctx.load_texture(
                        "cover_art",
                        color_image,
                        egui::TextureOptions::LINEAR,
                    ));
                }
            } else {
                self.cover_texture = None;
            }

            if info.title.is_empty() && info.artist.is_empty() {
                self.track_info = None;
            } else {
                self.track_info = Some(info);
            }
        }
    }

    /// Rebuild egui's font atlas. Registers the custom font (if any) under
    /// `CUSTOM_FONT_FAMILY`, with egui's default Proportional as fallback so
    /// the `Custom` family never ends up empty.
    fn reload_custom_font(&mut self, ctx: &egui::Context) {
        let target = self.config.track_info.custom_font_path.clone();
        let mut fonts = egui::FontDefinitions::default();

        let mut custom_family_names: Vec<String> = Vec::new();
        if let Some(ref path) = target {
            match std::fs::read(path) {
                Ok(data) => {
                    fonts
                        .font_data
                        .insert(CUSTOM_FONT_KEY.to_owned(), egui::FontData::from_owned(data));
                    custom_family_names.push(CUSTOM_FONT_KEY.to_owned());
                }
                Err(e) => {
                    eprintln!("Failed to load custom font '{}': {}", path, e);
                }
            }
        }

        if custom_family_names.is_empty() {
            // Fall back to egui's default proportional list so the family
            // always resolves to *something*.
            if let Some(defaults) =
                fonts.families.get(&egui::FontFamily::Proportional).cloned()
            {
                custom_family_names = defaults;
            }
        }

        fonts
            .families
            .insert(custom_font_family(), custom_family_names);

        ctx.set_fonts(fonts);
        self.loaded_font_path = target;
    }

    /// Resolve the representative "visualizer colour" for the current mode,
    /// so the track text can track it in real time.
    fn get_visualizer_text_color(&self, time: f32) -> [u8; 3] {
        match self.config.color_mode {
            ColorMode::Dynamic => self.dynamic_color,
            ColorMode::StaticSingle => self.config.bar_color,
            ColorMode::StaticMultiple => self
                .config
                .static_colors
                .first()
                .copied()
                .unwrap_or([255, 255, 255]),
            ColorMode::Gradient => color::sample_gradient(&self.config.gradient_stops, 0.5),
            ColorMode::ColorChanging => {
                let c = self.sample_color_changing(0.5, 0.5, time);
                [c.r(), c.g(), c.b()]
            }
        }
    }

    // ── Track-info rendering helpers ────────────────────────

    /// Draws a single piece of text with all shared styling: shadow, outline,
    /// bold simulation, letter spacing, colour, opacity.
    fn draw_styled_text(
        &self,
        ctx: &egui::Context,
        painter: &egui::Painter,
        text: &str,
        pos: egui::Pos2,
        font_id: &egui::FontId,
        align: egui::Align2,
        color: egui::Color32,
        cfg: &TrackInfoConfig,
    ) {
        if text.is_empty() {
            return;
        }

        let bold = matches!(cfg.font_style, FontStyle::Bold | FontStyle::BoldItalic);
        let ls = cfg.letter_spacing;

        // ── Shadow ──
        if cfg.shadow_enabled {
            let base_alpha = (cfg.opacity as f32 * 0.6).max(1.0);
            let offsets: Vec<(f32, f32)> = if cfg.shadow_blur < 1.0 {
                vec![(0.0, 0.0)]
            } else {
                let mut v = vec![(0.0, 0.0)];
                let n = 12;
                for i in 0..n {
                    let a = i as f32 / n as f32 * TAU;
                    v.push((a.cos() * cfg.shadow_blur, a.sin() * cfg.shadow_blur));
                }
                v
            };
            let per_alpha =
                (base_alpha / (offsets.len() as f32).sqrt()).clamp(1.0, 255.0) as u8;
            let s_color = egui::Color32::from_rgba_unmultiplied(
                cfg.shadow_color[0],
                cfg.shadow_color[1],
                cfg.shadow_color[2],
                per_alpha,
            );
            for (dx, dy) in offsets {
                let sp = pos + egui::vec2(cfg.shadow_offset_x + dx, cfg.shadow_offset_y + dy);
                draw_text_layered(ctx, painter, text, sp, font_id, align, s_color, ls, bold);
            }
        }

        // ── Outline (8-direction offset technique) ──
        if cfg.outline_enabled && cfg.outline_thickness > 0.1 {
            let o_color = egui::Color32::from_rgba_unmultiplied(
                cfg.outline_color[0],
                cfg.outline_color[1],
                cfg.outline_color[2],
                cfg.opacity,
            );
            let t = cfg.outline_thickness;
            let n = 12;
            for i in 0..n {
                let a = i as f32 / n as f32 * TAU;
                let op = pos + egui::vec2(a.cos() * t, a.sin() * t);
                draw_text_layered(ctx, painter, text, op, font_id, align, o_color, ls, bold);
            }
        }

        // ── Main text ──
        draw_text_layered(ctx, painter, text, pos, font_id, align, color, ls, bold);
    }

    /// Draws album art + artist/title text using the user's `TrackInfoConfig`.
    fn draw_track_info(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let Some(info) = &self.track_info else { return };

        let cfg = &self.config.track_info;
        let ctx = ui.ctx();
        let painter = ui.painter();
        let time = ui.input(|i| i.time) as f32;

        let margin = 40.0;
        let art_size = cfg.cover_size;
        let text_gap = 14.0;

        // ── Resolve colour ──
        let base_rgb = match cfg.color_mode {
            TrackTextColorMode::Static => cfg.static_color,
            TrackTextColorMode::Visualizer => self.get_visualizer_text_color(time),
        };
        let text_color = egui::Color32::from_rgba_unmultiplied(
            base_rgb[0],
            base_rgb[1],
            base_rgb[2],
            cfg.opacity,
        );

        // ── Font family ──
        let family = match cfg.font_family {
            TrackFontFamily::Proportional => egui::FontFamily::Proportional,
            TrackFontFamily::Monospace => egui::FontFamily::Monospace,
            TrackFontFamily::Custom => custom_font_family(),
        };

        let artist_font = egui::FontId::new(cfg.artist_font_size, family.clone());
        let title_font = egui::FontId::new(cfg.title_font_size, family);

        let align = match cfg.alignment {
            TrackTextAlign::Left => egui::Align2::LEFT_TOP,
            TrackTextAlign::Center => egui::Align2::CENTER_TOP,
            TrackTextAlign::Right => egui::Align2::RIGHT_TOP,
        };

        // ── Cover art ──
        let mut text_anchor_y = rect.top() + margin + cfg.text_y_offset;

        if cfg.show_cover_art {
            if let Some(tex) = &self.cover_texture {
                let art_x = rect.right() - margin - art_size + cfg.cover_x_offset;
                let art_y = rect.top() + margin + cfg.cover_y_offset;
                let art_rect = egui::Rect::from_min_size(
                    egui::pos2(art_x, art_y),
                    egui::vec2(art_size, art_size),
                );
                painter.image(
                    tex.id(),
                    art_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
                text_anchor_y = art_rect.bottom() + text_gap + cfg.text_y_offset;
            }
        }

        let text_x = rect.right() - margin + cfg.text_x_offset;

        // ── Artist (top line) ──
        let artist_pos = egui::pos2(text_x, text_anchor_y);
        self.draw_styled_text(
            ctx,
            painter,
            &info.artist,
            artist_pos,
            &artist_font,
            align,
            text_color,
            cfg,
        );

        // ── Title (second line, spaced by line_height × artist font) ──
        let title_y = text_anchor_y + cfg.artist_font_size * cfg.line_height;
        let title_pos = egui::pos2(text_x, title_y);
        self.draw_styled_text(
            ctx,
            painter,
            &info.title,
            title_pos,
            &title_font,
            align,
            text_color,
            cfg,
        );
    }
}

// ─────────────────────────────────────────────────────────────
// Free helpers for drawing text with letter spacing + bold
// ─────────────────────────────────────────────────────────────

fn draw_text_layered(
    ctx: &egui::Context,
    painter: &egui::Painter,
    text: &str,
    pos: egui::Pos2,
    font_id: &egui::FontId,
    align: egui::Align2,
    color: egui::Color32,
    letter_spacing: f32,
    bold: bool,
) {
    draw_spaced(ctx, painter, text, pos, font_id, align, color, letter_spacing);
    if bold {
        // Simulated bold: nudge right by ~1px and redraw. Works on any font.
        draw_spaced(
            ctx,
            painter,
            text,
            pos + egui::vec2(0.8, 0.0),
            font_id,
            align,
            color,
            letter_spacing,
        );
    }
}

fn draw_spaced(
    ctx: &egui::Context,
    painter: &egui::Painter,
    text: &str,
    pos: egui::Pos2,
    font_id: &egui::FontId,
    align: egui::Align2,
    color: egui::Color32,
    letter_spacing: f32,
) {
    if text.is_empty() {
        return;
    }

    // Fast path: no letter spacing — one call per glyph run.
    if letter_spacing.abs() < 0.01 {
        painter.text(pos, align, text, font_id.clone(), color);
        return;
    }

    let chars: Vec<char> = text.chars().collect();
    let widths: Vec<f32> = chars
        .iter()
        .map(|&ch| ctx.fonts(|f| f.glyph_width(font_id, ch)))
        .collect();

    let total_width: f32 =
        widths.iter().sum::<f32>() + letter_spacing * (chars.len().saturating_sub(1)) as f32;

    let start_x = match align {
        egui::Align2::LEFT_TOP
        | egui::Align2::LEFT_CENTER
        | egui::Align2::LEFT_BOTTOM => pos.x,
        egui::Align2::RIGHT_TOP
        | egui::Align2::RIGHT_CENTER
        | egui::Align2::RIGHT_BOTTOM => pos.x - total_width,
        egui::Align2::CENTER_TOP
        | egui::Align2::CENTER_CENTER
        | egui::Align2::CENTER_BOTTOM => pos.x - total_width / 2.0,
    };

    let mut x = start_x;
    for (ch, w) in chars.iter().zip(widths.iter()) {
        painter.text(
            egui::pos2(x, pos.y),
            egui::Align2::LEFT_TOP,
            ch.to_string(),
            font_id.clone(),
            color,
        );
        x += w + letter_spacing;
    }
}

// ─────────────────────────────────────────────────────────────
// eframe::App
// ─────────────────────────────────────────────────────────────

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

        self.poll_track_updates(ctx);

        // Reload custom font whenever the configured path changes.
        if self.config.track_info.custom_font_path != self.loaded_font_path {
            self.reload_custom_font(ctx);
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
                self.draw_track_info(ui, rect);

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
                .with_inner_size([430.0, 950.0])
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

// ─────────────────────────────────────────────────────────────
// Frequency / motion helpers
// ─────────────────────────────────────────────────────────────

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
            MotionShape::Uniform => MotionSample {
                phase_a: base_t,
                intensity: 1.0,
                ..Default::default()
            },
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
                    if beat < 0.10 { 1.0 }
                    else if beat < 0.18 { 0.0 }
                    else if beat < 0.26 { 0.7 }
                    else { 0.0 }
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

        let pr = ca[0] as f32 * (1.0 - t) + cb[0] as f32 * t;
        let pg = ca[1] as f32 * (1.0 - t) + cb[1] as f32 * t;
        let pb = ca[2] as f32 * (1.0 - t) + cb[2] as f32 * t;

        let off = self.config.color_change_motion.off_color;
        let r = off[0] as f32 + (pr - off[0] as f32) * k;
        let g = off[1] as f32 + (pg - off[1] as f32) * k;
        let b = off[2] as f32 + (pb - off[2] as f32) * k;

        egui::Color32::from_rgb(r as u8, g as u8, b as u8)
    }

    fn get_bar_color(
        &self,
        i: usize,
        total_bars: usize,
        bar_rect: egui::Rect,
        screen_rect: egui::Rect,
        time: f32,
    ) -> egui::Color32 {
        match self.config.color_mode {
            ColorMode::Dynamic => egui::Color32::from_rgb(
                self.dynamic_color[0],
                self.dynamic_color[1],
                self.dynamic_color[2],
            ),
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

        let segments = ((h / 6.0) as usize).clamp(8, 64);

        for k in 0..segments {
            let v0 = k as f32 / segments as f32;
            let v1 = (k + 1) as f32 / segments as f32;
            let v_mid = (v0 + v1) * 0.5;

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

    fn draw_track_info_settings(&mut self, ui: &mut egui::Ui) {
        let cfg = &mut self.config.track_info;

        // ── Cover Art ──
        ui.heading("🖼️ Cover Art");
        ui.checkbox(&mut cfg.show_cover_art, "Show Cover Art");
        if cfg.show_cover_art {
            ui.add(egui::Slider::new(&mut cfg.cover_size, 40.0..=600.0).text("Size (px)"));
            ui.add(egui::Slider::new(&mut cfg.cover_x_offset, -1500.0..=1500.0).text("X Offset"));
            ui.add(egui::Slider::new(&mut cfg.cover_y_offset, -1500.0..=1500.0).text("Y Offset"));
            ui.label("Positive offsets move the cover right / down (screen coordinates).");
        }

        // ── Song Info Text ──
        ui.separator();
        ui.heading("📝 Song Info Text");

        ui.label("Font sizes (per-line, independent):");
        ui.add(egui::Slider::new(&mut cfg.title_font_size, 6.0..=120.0).text("Title Font Size"));
        ui.add(egui::Slider::new(&mut cfg.artist_font_size, 6.0..=120.0).text("Artist Font Size"));

        ui.add_space(6.0);
        ui.label("Font family (shared):");
        ui.horizontal(|ui| {
            ui.radio_value(&mut cfg.font_family, TrackFontFamily::Proportional, "Default");
            ui.radio_value(&mut cfg.font_family, TrackFontFamily::Monospace, "Monospace");
            ui.radio_value(&mut cfg.font_family, TrackFontFamily::Custom, "Custom…");
        });

        if cfg.font_family == TrackFontFamily::Custom {
            ui.label("Custom font file (.ttf / .otf):");
            let mut path_str = cfg.custom_font_path.clone().unwrap_or_default();
            if ui.text_edit_singleline(&mut path_str).changed() {
                cfg.custom_font_path = if path_str.trim().is_empty() {
                    None
                } else {
                    Some(path_str)
                };
            }
            ui.label("Tip: system fonts live in C:\\Windows\\Fonts\\");
            ui.label("(Changes take effect on the next frame.)");
        }

        ui.horizontal_wrapped(|ui| {
            ui.label("Style:");
            ui.radio_value(&mut cfg.font_style, FontStyle::Regular, "Regular");
            ui.radio_value(&mut cfg.font_style, FontStyle::Bold, "Bold");
            ui.radio_value(&mut cfg.font_style, FontStyle::Italic, "Italic");
            ui.radio_value(&mut cfg.font_style, FontStyle::BoldItalic, "Bold+Italic");
        });
        ui.label("Bold is simulated by double-drawing. Italic currently has no visual effect \
                  (it requires loading an italic variant file as a Custom font).");

        // Text colour
        ui.add_space(6.0);
        ui.label("Text color (shared):");
        ui.horizontal(|ui| {
            ui.radio_value(&mut cfg.color_mode, TrackTextColorMode::Static, "Static");
            ui.radio_value(
                &mut cfg.color_mode,
                TrackTextColorMode::Visualizer,
                "Visualizer Color (dynamic)",
            );
        });
        if cfg.color_mode == TrackTextColorMode::Static {
            ui.horizontal(|ui| {
                ui.label("Color:");
                ui.color_edit_button_srgb(&mut cfg.static_color);
            });
        } else {
            ui.label("Text tracks the current visualizer color in real time.");
        }

        ui.add(egui::Slider::new(&mut cfg.opacity, 0..=255).text("Text Opacity"));

        // Alignment / spacing
        ui.horizontal(|ui| {
            ui.label("Alignment:");
            ui.radio_value(&mut cfg.alignment, TrackTextAlign::Left, "Left");
            ui.radio_value(&mut cfg.alignment, TrackTextAlign::Center, "Center");
            ui.radio_value(&mut cfg.alignment, TrackTextAlign::Right, "Right");
        });

        ui.add(egui::Slider::new(&mut cfg.letter_spacing, -5.0..=20.0).text("Letter Spacing (px)"));
        ui.add(egui::Slider::new(&mut cfg.line_height, 0.5..=3.0).text("Line Height (× artist font)"));

        ui.add_space(6.0);
        ui.label("Text position offsets:");
        ui.add(egui::Slider::new(&mut cfg.text_x_offset, -1500.0..=1500.0).text("Text X Offset"));
        ui.add(egui::Slider::new(&mut cfg.text_y_offset, -1500.0..=1500.0).text("Text Y Offset"));

        // Shadow
        ui.separator();
        ui.heading("Text Shadow");
        ui.checkbox(&mut cfg.shadow_enabled, "Enable Shadow");
        if cfg.shadow_enabled {
            ui.add(egui::Slider::new(&mut cfg.shadow_offset_x, -30.0..=30.0).text("Shadow X Offset"));
            ui.add(egui::Slider::new(&mut cfg.shadow_offset_y, -30.0..=30.0).text("Shadow Y Offset"));
            ui.add(egui::Slider::new(&mut cfg.shadow_blur, 0.0..=20.0).text("Shadow Blur"));
            ui.horizontal(|ui| {
                ui.label("Shadow Color:");
                ui.color_edit_button_srgb(&mut cfg.shadow_color);
            });
        }

        // Outline
        ui.separator();
        ui.heading("Text Outline");
        ui.checkbox(&mut cfg.outline_enabled, "Enable Outline");
        if cfg.outline_enabled {
            ui.add(egui::Slider::new(&mut cfg.outline_thickness, 0.1..=10.0).text("Outline Thickness"));
            ui.horizontal(|ui| {
                ui.label("Outline Color:");
                ui.color_edit_button_srgb(&mut cfg.outline_color);
            });
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
            ui.radio_value(&mut self.config.color_mode, ColorMode::Dynamic, "Dynamic (Album Art)");
            ui.radio_value(&mut self.config.color_mode, ColorMode::StaticSingle, "Static Single");
            ui.radio_value(&mut self.config.color_mode, ColorMode::StaticMultiple, "Static Multi");
            ui.radio_value(&mut self.config.color_mode, ColorMode::Gradient, "Gradient");
            ui.radio_value(&mut self.config.color_mode, ColorMode::ColorChanging, "Color Changing");
        });

        if self.config.color_mode == ColorMode::Dynamic {
            ui.label("Bars tint automatically to the dominant color of the current album art.");

            let count = self.smtc_session_count.load(Ordering::Relaxed);
            ui.label(format!("SMTC sessions active: {}", count));
            if count == 0 {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 180, 80),
                    "⚠ No SMTC sessions found. Is a supported player running?",
                );
                ui.label(
                    "MusicBee needs the 'mb_MediaControl' plugin installed for SMTC support.",
                );
                ui.colored_label(
                    egui::Color32::from_rgb(255, 180, 80),
                    "Note: If this visualizer is running as Administrator, it cannot see media players running as normal users.",
                );
                ui.label("Check 'smtc_debug.log' in the app folder for diagnostic details.");
            }

            if let Some(info) = &self.track_info {
                ui.label(format!("Now Playing: {} — {}", info.artist, info.title));
            } else {
                ui.label("No track detected via SMTC.");
            }
            ui.horizontal(|ui| {
                ui.label("Current color:");
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(24.0, 24.0),
                    egui::Sense::hover(),
                );
                ui.painter().rect_filled(
                    rect,
                    4.0,
                    egui::Color32::from_rgb(
                        self.dynamic_color[0],
                        self.dynamic_color[1],
                        self.dynamic_color[2],
                    ),
                );
            });
        }

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
                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(24.0, 24.0),
                            egui::Sense::click(),
                        );
                        if ui.is_rect_visible(rect) {
                            let color =
                                egui::Color32::from_rgb(preset[0], preset[1], preset[2]);
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
                ui.horizontal_wrapped(|ui| {
                    ui.radio_value(&mut self.config.static_mode, StaticDistributionMode::Blocks, "Blocks");
                    ui.radio_value(&mut self.config.static_mode, StaticDistributionMode::Sequential, "Sequential");
                    ui.radio_value(&mut self.config.static_mode, StaticDistributionMode::Random, "Random");
                    ui.radio_value(&mut self.config.static_mode, StaticDistributionMode::Vertical, "Vertical");
                });

                if self.config.static_mode == StaticDistributionMode::Sequential {
                    ui.add(
                        egui::Slider::new(&mut self.config.sequential_bars_per_color, 1..=20)
                            .text("Bars per Color"),
                    );
                    ui.label("1 = R, W, B, R…   2 = R, R, W, W, B, B…");
                }

                ui.add_space(5.0);

                let mut to_remove = None;
                for (i, color) in self.config.static_colors.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.color_edit_button_srgb(color);
                        if ui.button("❌").clicked() {
                            to_remove = Some(i);
                        }
                    });
                }
                if let Some(i) = to_remove {
                    self.config.static_colors.remove(i);
                }
                if ui.button("➕ Add Color").clicked() {
                    self.config.static_colors.push([255, 255, 255]);
                }

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.new_preset_name);
                    if ui.button("💾 Save Preset").clicked() {
                        let name = if self.new_preset_name.trim().is_empty() {
                            format!("Preset {}", self.config.static_multi_presets.len() + 1)
                        } else {
                            self.new_preset_name.clone()
                        };
                        self.config.static_multi_presets.push(StaticMultiPreset {
                            name,
                            colors: self.config.static_colors.clone(),
                            mode: self.config.static_mode,
                            bars_per_color: self.config.sequential_bars_per_color,
                        });
                        self.new_preset_name.clear();
                    }
                });

                ui.label("Presets:");
                let mut preset_to_remove = None;
                for (i, preset) in self.config.static_multi_presets.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.button(&preset.name).clicked() {
                            self.config.static_colors = preset.colors.clone();
                            self.config.static_mode = preset.mode;
                            self.config.sequential_bars_per_color = preset.bars_per_color;
                        }
                        if ui.button("❌").clicked() {
                            preset_to_remove = Some(i);
                        }
                    });
                }
                if let Some(i) = preset_to_remove {
                    self.config.static_multi_presets.remove(i);
                }
            }

            ColorMode::Gradient => {
                ui.heading("Gradient Colors");
                ui.add(egui::Slider::new(&mut self.config.gradient_angle, 0.0..=360.0).text("Angle (°)"));
                ui.label("0° = horizontal, 90° = vertical, 45° = diagonal.");

                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 30.0),
                    egui::Sense::click_and_drag(),
                );
                let painter = ui.painter();

                let steps = 100;
                for i in 0..steps {
                    let t1 = i as f32 / steps as f32;
                    let t2 = (i + 1) as f32 / steps as f32;
                    let c1 = color::sample_gradient(&self.config.gradient_stops, t1);
                    let x1 = rect.left() + rect.width() * t1;
                    let x2 = rect.left() + rect.width() * t2;
                    let r = egui::Rect::from_min_max(
                        egui::pos2(x1, rect.top()),
                        egui::pos2(x2, rect.bottom()),
                    );
                    painter.rect_filled(r, 0.0, egui::Color32::from_rgb(c1[0], c1[1], c1[2]));
                }

                let mut sort_stops = false;
                for (i, stop) in self.config.gradient_stops.iter_mut().enumerate() {
                    let x = rect.left() + rect.width() * stop.position;
                    let stop_rect = egui::Rect::from_center_size(
                        egui::pos2(x, rect.center().y),
                        egui::vec2(12.0, 30.0),
                    );
                    let stop_response = ui.interact(
                        stop_rect,
                        ui.id().with("gradient_stop").with(i),
                        egui::Sense::click_and_drag(),
                    );

                    if stop_response.dragged() {
                        stop.position = (stop.position
                            + stop_response.drag_delta().x / rect.width())
                        .clamp(0.0, 1.0);
                        sort_stops = true;
                    }

                    let handle_color =
                        egui::Color32::from_rgb(stop.color[0], stop.color[1], stop.color[2]);
                    painter.rect_filled(stop_rect, 2.0, handle_color);
                    painter.rect_stroke(
                        stop_rect,
                        2.0_f32,
                        egui::Stroke::new(2.0_f32, egui::Color32::WHITE),
                    );
                }

                if sort_stops {
                    self.config
                        .gradient_stops
                        .sort_by(|a, b| a.position.partial_cmp(&b.position).unwrap());
                }

                if response.double_clicked() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let t = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                        let c = color::sample_gradient(&self.config.gradient_stops, t);
                        self.config
                            .gradient_stops
                            .push(GradientStop { color: c, position: t });
                        self.config
                            .gradient_stops
                            .sort_by(|a, b| a.position.partial_cmp(&b.position).unwrap());
                    }
                }
                ui.label("Double-click the bar to add a color stop.");

                let mut to_remove = None;
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
                if let Some(i) = to_remove {
                    self.config.gradient_stops.remove(i);
                }

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.new_preset_name);
                    if ui.button("💾 Save Preset").clicked() {
                        let name = if self.new_preset_name.trim().is_empty() {
                            format!("Preset {}", self.config.gradient_presets.len() + 1)
                        } else {
                            self.new_preset_name.clone()
                        };
                        self.config.gradient_presets.push(GradientPreset {
                            name,
                            stops: self.config.gradient_stops.clone(),
                            angle: self.config.gradient_angle,
                        });
                        self.new_preset_name.clear();
                    }
                });

                ui.label("Presets:");
                let mut preset_to_remove = None;
                for (i, preset) in self.config.gradient_presets.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.button(&preset.name).clicked() {
                            self.config.gradient_stops = preset.stops.clone();
                            self.config.gradient_angle = preset.angle;
                        }
                        if ui.button("❌").clicked() {
                            preset_to_remove = Some(i);
                        }
                    });
                }
                if let Some(i) = preset_to_remove {
                    self.config.gradient_presets.remove(i);
                }
            }

            ColorMode::ColorChanging => {
                ui.heading("Color Changing");
                let mut to_remove = None;
                let current_len = self.config.color_change_colors.len();
                for (i, color) in self.config.color_change_colors.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.color_edit_button_srgb(color);
                        if current_len > 2 && ui.button("❌").clicked() {
                            to_remove = Some(i);
                        }
                    });
                }
                if let Some(i) = to_remove {
                    self.config.color_change_colors.remove(i);
                }
                if ui.button("➕ Add Color").clicked() {
                    self.config.color_change_colors.push([255, 255, 255]);
                }

                ui.add(egui::Slider::new(&mut self.config.color_change_speed, 0.0..=1.0)
                    .text("Speed")
                    .custom_formatter(|n, _| format!("{:.0}%", n * 100.0)));

                ui.add_space(10.0);
                ui.separator();
                ui.heading("Motion Shape");

                egui::ComboBox::from_label("Shape")
                    .selected_text(self.config.color_change_motion.shape.display_name())
                    .show_ui(ui, |ui| {
                        let all_shapes = [
                            MotionShape::Uniform,
                            MotionShape::HorizontalWipe,
                            MotionShape::VerticalWipe,
                            MotionShape::DiagonalWipe,
                            MotionShape::SineWave,
                            MotionShape::TriangleWave,
                            MotionShape::SquareWave,
                            MotionShape::SawtoothWave,
                            MotionShape::Strobe,
                            MotionShape::Pulse,
                            MotionShape::Heartbeat,
                            MotionShape::Stutter,
                            MotionShape::Comet,
                            MotionShape::Chase,
                            MotionShape::BouncingBall,
                            MotionShape::Sparkle,
                            MotionShape::FireSimulation,
                            MotionShape::Plasma,
                            MotionShape::RainbowCycle,
                            MotionShape::Confetti,
                            MotionShape::Sinelon,
                        ];
                        for shape in all_shapes {
                            ui.selectable_value(
                                &mut self.config.color_change_motion.shape,
                                shape,
                                shape.display_name(),
                            );
                        }
                    });

                ui.horizontal(|ui| {
                    ui.label("Blend:");
                    ui.radio_value(
                        &mut self.config.color_change_motion.blend,
                        BlendMode::Smooth,
                        "Smooth",
                    );
                    ui.radio_value(
                        &mut self.config.color_change_motion.blend,
                        BlendMode::Hard,
                        "Hard",
                    );
                });
                ui.label("Smooth = gradients / soft fades. Hard = sharp on/off edges.");

                ui.add(egui::Slider::new(
                    &mut self.config.color_change_motion.speed,
                    0.0..=2.0,
                )
                .text("Motion Speed"));

                let shape = self.config.color_change_motion.shape;
                let hard = self.config.color_change_motion.blend == BlendMode::Hard;

                if matches!(
                    shape,
                    MotionShape::SineWave
                        | MotionShape::TriangleWave
                        | MotionShape::SquareWave
                        | MotionShape::SawtoothWave
                ) {
                    ui.add(egui::Slider::new(
                        &mut self.config.color_change_motion.wave_frequency,
                        0.5..=8.0,
                    )
                    .text("Wave Frequency"));
                }

                if matches!(
                    shape,
                    MotionShape::Comet
                        | MotionShape::BouncingBall
                        | MotionShape::Sinelon
                ) {
                    ui.add(egui::Slider::new(
                        &mut self.config.color_change_motion.trail_length,
                        0.05..=0.9,
                    )
                    .text("Trail Length"));
                }

                if matches!(
                    shape,
                    MotionShape::Stutter | MotionShape::Sparkle | MotionShape::Confetti
                ) {
                    ui.add(egui::Slider::new(
                        &mut self.config.color_change_motion.stutter_intensity,
                        0.0..=1.0,
                    )
                    .text("Stutter / Event Intensity"));
                }

                if matches!(
                    shape,
                    MotionShape::HorizontalWipe
                        | MotionShape::VerticalWipe
                        | MotionShape::DiagonalWipe
                ) && !hard
                {
                    ui.add(egui::Slider::new(
                        &mut self.config.color_change_motion.wipe_softness,
                        0.01..=0.5,
                    )
                    .text("Wipe Softness"));
                }

                if shape.uses_intensity() {
                    ui.horizontal(|ui| {
                        ui.label("Off color:");
                        ui.color_edit_button_srgb(
                            &mut self.config.color_change_motion.off_color,
                        );
                        if ui
                            .add_enabled(
                                self.config.color_change_motion.off_color != [0, 0, 0],
                                egui::Button::new("Reset to black"),
                            )
                            .clicked()
                        {
                            self.config.color_change_motion.off_color = [0, 0, 0];
                        }
                    });
                    ui.label(
                        "Colour that 'off' pixels fade toward \
                         (defaults to black — the original behaviour).",
                    );
                }

                ui.add_space(10.0);
                ui.separator();
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.new_preset_name);
                    if ui.button("💾 Save Preset").clicked() {
                        let name = if self.new_preset_name.trim().is_empty() {
                            format!("Preset {}", self.config.color_change_presets.len() + 1)
                        } else {
                            self.new_preset_name.clone()
                        };
                        self.config.color_change_presets.push(ColorChangePreset {
                            name,
                            colors: self.config.color_change_colors.clone(),
                            speed: self.config.color_change_speed,
                        });
                        self.new_preset_name.clear();
                    }
                });

                ui.label("Presets:");
                let mut preset_to_remove = None;
                for (i, preset) in self.config.color_change_presets.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.button(&preset.name).clicked() {
                            self.config.color_change_colors = preset.colors.clone();
                            self.config.color_change_speed = preset.speed;
                        }
                        if ui.button("❌").clicked() {
                            preset_to_remove = Some(i);
                        }
                    });
                }
                if let Some(i) = preset_to_remove {
                    self.config.color_change_presets.remove(i);
                }
            }

            _ => {}
        }

        ui.separator();
        ui.heading("Appearance");
        ui.add(egui::Slider::new(&mut self.config.bg_opacity, 0..=255).text("Background Opacity"));

        ui.separator();
        self.draw_track_info_settings(ui);

        ui.separator();
        if ui.button("⚠️ Reset to Defaults").clicked() {
            self.config = AppConfig::default();
            self.audio_target_bars
                .store(self.config.num_bars, Ordering::Relaxed);
            self.audio_update_interval
                .store(self.config.audio_update_ms, Ordering::Relaxed);
            self.current_bars = vec![0.0; self.config.num_bars];
            // Force the font cache to drop any stale custom font.
            self.loaded_font_path = Some("__reset__".to_string());
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
