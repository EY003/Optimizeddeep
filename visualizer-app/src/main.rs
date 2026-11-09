#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use image::DynamicImage;
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use arc_swap::ArcSwap;
use visualizer_core::{
    audio::{AudioEngine, BarFrame},
    color,
    config::{
        self as cfg,
        AppConfig, AudioPreset, BarInterpolation, BlendMode,
        ColorChangePreset, ColorMode, ColorPreset, ColorSource, CustomAudioPreset, DecayMode,
        FrequencyDirection, GradientPreset, GradientSource, GradientStop,
        MotionShape, PalettePriority, StaticDistributionMode, StaticMultiPreset,
        TrackInfoConfig, TrackTextAlign, TrackTextColorMode, VerticalColorStyle,
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

use bytemuck::{Pod, Zeroable};
use egui_wgpu::{CallbackTrait, wgpu};

#[cfg(windows)]
#[link(name = "winmm")]
extern "system" {
    fn timeBeginPeriod(uperiod: u32) -> u32;
    fn timeEndPeriod(uperiod: u32) -> u32;
}

// ═══════════════════════════════════════════════════════════════
// MotionSample (was originally a local struct in main.rs)
// ═══════════════════════════════════════════════════════════════

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

// ═══════════════════════════════════════════════════════════════
// GPU structs (must match visualizer.wgsl layout exactly)
// ═══════════════════════════════════════════════════════════════

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct GpuBar {
    pub height: f32,            // offset 0
    pub peak_height: f32,       // offset 4
    pub _pad0: [f32; 2],        // offset 8..16 (aligns color_top to 16)
    pub color_top: [f32; 4],    // offset 16
    pub color_bottom: [f32; 4], // offset 32
    pub show_peak: f32,         // offset 48
    pub _pad1: [f32; 3],        // offset 52..64 (struct size = 64)
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct ShaderUniforms {
    pub dims: [f32; 4],          // num_bars, is_vertical, is_smooth, is_mirrored
    pub layout: [f32; 4],        // bar_width, bar_gap, screen_width, screen_height
    pub outline_color: [f32; 4], // r, g, b, a
    pub extras: [f32; 4],        // outline_thickness, bar_radius, opacity, direction
}

pub struct VisualizerResources {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group: wgpu::BindGroup,
    pub bar_buffer: wgpu::Buffer,
    pub uniform_buffer: wgpu::Buffer,
}

pub struct VisualizerCallback {
    pub bars: Vec<GpuBar>,
    pub uniforms: ShaderUniforms,
}

impl CallbackTrait for VisualizerCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let resources: &mut VisualizerResources = callback_resources.get_mut().unwrap();
        queue.write_buffer(&resources.bar_buffer, 0, bytemuck::cast_slice(&self.bars));
        queue.write_buffer(&resources.uniform_buffer, 0, bytemuck::bytes_of(&self.uniforms));
        Vec::new()
    }

    fn paint<'a>(
        &'a self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'a>,
        callback_resources: &'a egui_wgpu::CallbackResources,
    ) {
        let resources: &VisualizerResources = callback_resources.get().unwrap();
        render_pass.set_pipeline(&resources.pipeline);
        render_pass.set_bind_group(0, &resources.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

fn setup_wgpu(render_state: &egui_wgpu::RenderState) -> VisualizerResources {
    let device = &render_state.device;

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Visualizer Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("visualizer.wgsl").into()),
    });

    let bar_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Bar Buffer"),
        size: (std::mem::size_of::<GpuBar>() * 256) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Uniform Buffer"),
        size: std::mem::size_of::<ShaderUniforms>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
        label: Some("Visualizer Bind Group Layout"),
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: bar_buffer.as_entire_binding(),
            },
        ],
        label: Some("Visualizer Bind Group"),
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Visualizer Pipeline Layout"),
        bind_group_layouts: &[&bind_group_layout],
        push_constant_ranges: &[],
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Visualizer Pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: "vs_main",
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: "fs_main",
            targets: &[Some(wgpu::ColorTargetState {
                format: render_state.target_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
    });

    VisualizerResources { pipeline, bind_group, bar_buffer, uniform_buffer }
}

// ═══════════════════════════════════════════════════════════════
// Icon loading
// ═══════════════════════════════════════════════════════════════

fn load_icon() -> (egui::IconData, tray_icon::Icon) {
    let icon_bytes = include_bytes!("icon.png");
    let image = image::load_from_memory(icon_bytes)
        .unwrap_or_else(|_| image::DynamicImage::new_rgba8(1, 1));
    let rgba = image.to_rgba8();
    let (w, h) = rgba.dimensions();
    let raw = rgba.into_raw();

    let egui_icon = egui::IconData { rgba: raw.clone(), width: w, height: h };
    let tray = tray_icon::Icon::from_rgba(raw, w, h).unwrap();
    (egui_icon, tray)
}

// ═══════════════════════════════════════════════════════════════
// Utilities
// ═══════════════════════════════════════════════════════════════

fn hash_u32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    x = x.wrapping_mul(0xC2B2_AE35);
    x ^= x >> 16;
    x
}

fn random_color_index(i: usize, num_colors: usize) -> usize {
    (hash_u32(i as u32) as usize) % num_colors
}

fn tri01(t: f32) -> f32 { 1.0 - (2.0 * t.fract() - 1.0).abs() }

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

fn static_multi_vertical_color(colors: &[[u8; 3]], t: f32, blocks: bool) -> egui::Color32 {
    if colors.is_empty() { return egui::Color32::WHITE; }
    let n = colors.len();
    if n == 1 {
        let c = colors[0];
        return egui::Color32::from_rgb(c[0], c[1], c[2]);
    }
    if blocks {
        let idx = ((t * n as f32) as usize).min(n - 1);
        let c = colors[idx];
        egui::Color32::from_rgb(c[0], c[1], c[2])
    } else {
        let scaled = t.clamp(0.0, 1.0) * (n - 1) as f32;
        let i0 = scaled.floor() as usize;
        let i1 = (i0 + 1).min(n - 1);
        let f = scaled - i0 as f32;
        let a = colors[i0];
        let b = colors[i1];
        egui::Color32::from_rgb(
            (a[0] as f32 * (1.0 - f) + b[0] as f32 * f) as u8,
            (a[1] as f32 * (1.0 - f) + b[1] as f32 * f) as u8,
            (a[2] as f32 * (1.0 - f) + b[2] as f32 * f) as u8,
        )
    }
}

fn stops_from_colors(colors: &[[u8; 3]]) -> Vec<GradientStop> {
    match colors.len() {
        0 => vec![GradientStop { color: [255, 255, 255], position: 0.0 }],
        1 => vec![GradientStop { color: colors[0], position: 0.0 }],
        n => colors.iter().enumerate()
            .map(|(i, c)| GradientStop { color: *c, position: i as f32 / (n - 1) as f32 })
            .collect(),
    }
}

fn compute_track_id(bytes: Option<&[u8]>, artist: &str, title: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    if let Some(b) = bytes { b.hash(&mut h); }
    artist.hash(&mut h);
    title.hash(&mut h);
    h.finish()
}

fn frequency_index(i: usize, base_num_bars: usize, dir: FrequencyDirection) -> usize {
    match dir {
        FrequencyDirection::LeftToRight => i,
        FrequencyDirection::RightToLeft => base_num_bars - 1 - i,
        FrequencyDirection::Mirrored => if i < base_num_bars { i } else { 2 * base_num_bars - 1 - i },
    }
}

// ═══════════════════════════════════════════════════════════════
// Font & Palette caches
// ═══════════════════════════════════════════════════════════════

const CUSTOM_FONT_FAMILY: &str = "custom_track";
const CUSTOM_FONT_KEY: &str = "track_custom";

fn custom_font_family() -> egui::FontFamily {
    egui::FontFamily::Name(CUSTOM_FONT_FAMILY.into())
}

fn list_system_fonts() -> Vec<(String, String)> {
    let fonts_dir = std::path::Path::new("C:\\Windows\\Fonts");
    let mut result = Vec::new();
    if let Ok(entries) = std::fs::read_dir(fonts_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(ext) = path.extension() {
                let ext = ext.to_string_lossy().to_lowercase();
                if ext == "ttf" || ext == "otf" {
                    if let Some(name) = path.file_name() {
                        result.push((
                            name.to_string_lossy().to_string(),
                            path.to_string_lossy().to_string(),
                        ));
                    }
                }
            }
        }
    }
    result.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    result
}

struct PaletteCacheEntry {
    track_id: u64, palette_size: u8, variants: u8, priority: PalettePriority,
    colors: Vec<[u8; 3]>,
}
#[derive(Default)]
struct PaletteCache { entry: Option<PaletteCacheEntry> }
impl PaletteCache {
    fn get(&self, track_id: u64, size: u8, variants: u8, priority: PalettePriority) -> Option<&[[u8; 3]]> {
        self.entry.as_ref()
            .filter(|e| e.track_id == track_id && e.palette_size == size
                    && e.variants == variants && e.priority == priority)
            .map(|e| e.colors.as_slice())
    }
    fn store(&mut self, track_id: u64, size: u8, variants: u8, priority: PalettePriority, colors: Vec<[u8; 3]>) {
        self.entry = Some(PaletteCacheEntry { track_id, palette_size: size, variants, priority, colors });
    }
}

#[derive(Default, Clone)]
struct ActivePalettes {
    static_single_colors: Vec<[u8; 3]>,
    static_multi_colors: Vec<[u8; 3]>,
    gradient_colors: Vec<[u8; 3]>,
    gradient_stops: Vec<GradientStop>,
    animated_colors: Vec<[u8; 3]>,
}

fn hash_color_source<H: std::hash::Hasher>(s: &ColorSource, h: &mut H) {
    use std::hash::Hash;
    match s {
        ColorSource::Static(v) => { 0u8.hash(h); v.hash(h); }
        ColorSource::Dynamic { palette_size, variants_per_color, priority } => {
            1u8.hash(h);
            palette_size.hash(h);
            variants_per_color.hash(h);
            priority.hash(h);
        }
    }
}
fn hash_gradient_source<H: std::hash::Hasher>(s: &GradientSource, h: &mut H) {
    use std::hash::Hash;
    match s {
        GradientSource::Static(stops) => {
            0u8.hash(h);
            for stop in stops {
                stop.color.hash(h);
                stop.position.to_bits().hash(h);
            }
        }
        GradientSource::Dynamic { palette_size, variants_per_color, priority } => {
            1u8.hash(h);
            palette_size.hash(h);
            variants_per_color.hash(h);
            priority.hash(h);
        }
    }
}

fn accent() -> egui::Color32 { egui::Color32::from_rgb(0xB7, 0x41, 0x0E) }

fn section_header(ui: &mut egui::Ui, title: &str) {
    ui.add_space(8.0);
    ui.label(egui::RichText::new(title).size(16.0).strong().color(accent()));
    ui.add_space(2.0);
    ui.separator();
    ui.add_space(4.0);
}

fn reset_btn<F: FnOnce()>(ui: &mut egui::Ui, changed: bool, on_reset: F) {
    let text = egui::RichText::new("Reset").size(12.0);
    if ui.add_enabled(changed, egui::Button::new(text).small()).clicked() {
        on_reset();
    }
}

fn help_text(ui: &mut egui::Ui, show: bool, text: &str) {
    if show {
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new(format!("ℹ {}", text))
                .size(12.0)
                .color(egui::Color32::from_gray(150))
                .italics()
        );
        ui.add_space(6.0);
    }
}

fn slider_f32(ui: &mut egui::Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, label: &str, default: f32) {
    ui.horizontal(|ui| {
        let r = ui.add(egui::Slider::new(value, range).text(label));
        if r.secondary_clicked() { *value = default; }
        reset_btn(ui, (*value - default).abs() > 1e-6, || *value = default);
    });
}

fn slider_f32_scale(ui: &mut egui::Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, label: &str, default: f32) {
    ui.horizontal(|ui| {
        let r = ui.add(egui::Slider::new(value, range).text(label)
            .custom_formatter(|n, _| format!("{:.0}%", n * 100.0)));
        if r.secondary_clicked() { *value = default; }
        reset_btn(ui, (*value - default).abs() > 1e-6, || *value = default);
    });
}

fn slider_u8(ui: &mut egui::Ui, value: &mut u8, label: &str, default: u8) {
    ui.horizontal(|ui| {
        let mut v = *value as i32;
        let r = ui.add(egui::Slider::new(&mut v, 0..=255).text(label));
        if v != *value as i32 { *value = v.clamp(0, 255) as u8; }
        if r.secondary_clicked() { *value = default; }
        reset_btn(ui, *value != default, || *value = default);
    });
}

fn checkbox_reset(ui: &mut egui::Ui, value: &mut bool, label: &str, default: bool) {
    ui.horizontal(|ui| {
        let r = ui.checkbox(value, label);
        if r.secondary_clicked() { *value = default; }
        reset_btn(ui, *value != default, || *value = default);
    });
}

fn color_edit_reset(ui: &mut egui::Ui, value: &mut [u8; 3], label: &str, default: [u8; 3]) {
    ui.horizontal(|ui| {
        ui.label(label);
        let r = ui.color_edit_button_srgb(value);
        if r.secondary_clicked() { *value = default; }
        reset_btn(ui, *value != default, || *value = default);
    });
}

fn combo_reset<T: PartialEq + Copy + 'static>(
    ui: &mut egui::Ui,
    value: &mut T,
    default: T,
    label: &str,
    id: &str,
    width: f32,
    options: &[T],
    name: fn(T) -> &'static str,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        egui::ComboBox::from_id_source(id)
            .selected_text(name(*value))
            .width(width)
            .show_ui(ui, |ui| {
                for &opt in options {
                    ui.selectable_value(value, opt, name(opt));
                }
            });
        reset_btn(ui, *value != default, || *value = default);
    });
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    Visualizer, Colors, Audio, TrackInfo, Appearance, Performance,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ColorSubTab {
    Single, Multi, Gradient, Animated,
}

const SEARCH_INDEX: &[(SettingsTab, &str, &[&str])] = &[
    (SettingsTab::Visualizer,  "Direction",          &["direction", "up", "down", "left", "right"]),
    (SettingsTab::Visualizer,  "Frequency Layout",   &["frequency", "mirrored", "layout", "scale"]),
    (SettingsTab::Visualizer,  "Bar Shape",          &["bar", "shape", "step", "smooth", "interpolation"]),
    (SettingsTab::Visualizer,  "Bars & Position",    &["bar", "width", "gap", "height", "offset", "position"]),
    (SettingsTab::Visualizer,  "Outlines",           &["outline", "border", "stroke"]),
    (SettingsTab::Colors,      "Color Source",       &["color", "palette", "album", "dynamic"]),
    (SettingsTab::Colors,      "Distribution",       &["blocks", "sequential", "random", "vertical"]),
    (SettingsTab::Colors,      "Gradient",           &["gradient", "stops", "angle"]),
    (SettingsTab::Colors,      "Motion",             &["motion", "animated", "shape"]),
    (SettingsTab::Audio,       "Presets",            &["preset", "audio", "custom"]),
    (SettingsTab::Audio,       "Signal Processing",  &["fft", "window", "frequency", "channel", "weighting", "smoothing", "dc", "harmonic"]),
    (SettingsTab::Audio,       "Amplitude",          &["sensitivity", "gain", "gamma", "noise", "gate", "db", "dynamic", "agc"]),
    (SettingsTab::Audio,       "Equalizer",          &["eq", "bass", "mid", "treble", "tilt", "spectrum"]),
    (SettingsTab::Audio,       "Ballistics",         &["attack", "decay", "ballistic", "gravity", "peak", "average"]),
    (SettingsTab::Audio,       "Rhythm",             &["beat", "bpm", "rhythm", "tempo", "phase"]),
    (SettingsTab::TrackInfo,   "Cover Art",          &["cover", "art", "album", "opacity"]),
    (SettingsTab::TrackInfo,   "Text",               &["title", "artist", "font", "text", "spacing"]),
    (SettingsTab::TrackInfo,   "Shadow",             &["shadow", "glow", "blur"]),
    (SettingsTab::TrackInfo,   "Outline",            &["outline", "border"]),
    (SettingsTab::Appearance,  "Background",         &["background", "bg", "color", "opacity"]),
    (SettingsTab::Appearance,  "UI",                 &["scale", "size", "help", "launch"]),
    (SettingsTab::Performance, "Performance",        &["fps", "vsync", "performance", "frame"]),
];

struct TrackLoadResult {
    artist: String,
    title: String,
    cover_art: Option<Vec<u8>>,
    extracted: Option<(color::TrackColors, DynamicImage)>,
}

// ═══════════════════════════════════════════════════════════════
// App struct
// ═══════════════════════════════════════════════════════════════

struct VisualizerApp {
    config: AppConfig,
    current_bars: Vec<f32>,
    current_peaks: Vec<f32>,
    last_bpm: f32,
    shared_frame: Arc<ArcSwap<BarFrame>>,
    audio_settings: Arc<ArcSwap<visualizer_core::config::AudioSettings>>,
    should_quit: Arc<AtomicBool>,
    show_settings: bool,
    current_fps: f32,
    is_passthrough_set: bool,
    last_frame_time: Instant,
    new_preset_name: String,
    new_audio_preset_name: String,
    last_decay_mode: DecayMode,
    track_rx: crossbeam_channel::Receiver<TrackInfo>,
    track_info: Option<TrackInfo>,
    cover_texture: Option<egui::TextureHandle>,
    cover_image: Option<DynamicImage>,
    track_id: u64,
    active_palettes: ActivePalettes,
    static_single_cache: PaletteCache,
    static_multi_cache: PaletteCache,
    gradient_cache: PaletteCache,
    animated_cache: PaletteCache,
    last_palette_hash: u64,
    loaded_font_path: Option<String>,
    system_fonts: Vec<(String, String)>,
    smtc_session_count: Arc<AtomicUsize>,

    settings_tab: SettingsTab,
    color_subtab: ColorSubTab,
    settings_search: String,
    pending_audio_preset_delete: Option<usize>,
    settings_icon: egui::IconData,
    old_settings_scale: f32,

    last_audio_frame_time: Instant,

    track_load_tx: crossbeam_channel::Sender<TrackLoadResult>,
    track_load_rx: crossbeam_channel::Receiver<TrackLoadResult>,

    _tray_icon: TrayIcon,
    _hotkey_manager: GlobalHotKeyManager,
    settings_menu_id: tray_icon::menu::MenuId,
    quit_menu_id: tray_icon::menu::MenuId,
}

impl VisualizerApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let config = AppConfig::load();
        let engine = AudioEngine::new(config.audio.clone());
        let should_quit = Arc::new(AtomicBool::new(false));
        musicbee::log_smtc("APP: Initializing VisualizerApp.");
        let audio_settings = engine.settings.clone();
        let shared_frame = engine.shared_frame.clone();
        let (track_tx, track_rx) = crossbeam_channel::unbounded();
        let smtc_session_count = Arc::new(AtomicUsize::new(0));
        musicbee::spawn_smtc_listener(track_tx, should_quit.clone(), smtc_session_count.clone());

        let (egui_icon, tray_icon_img) = load_icon();

        let tray_menu = Menu::new();
        let settings_item = MenuItem::new("Open Settings", true, None);
        let quit_item = MenuItem::new("Quit", true, None);
        tray_menu.append_items(&[&settings_item, &quit_item]).unwrap();
        let tray_icon = TrayIconBuilder::new()
            .with_menu(Box::new(tray_menu))
            .with_tooltip("Rust Music Visualizer")
            .with_icon(tray_icon_img)
            .build()
            .unwrap();

        let hotkey_manager = GlobalHotKeyManager::new().unwrap();
        let hotkey = HotKey::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyV);
        hotkey_manager.register(hotkey).unwrap();

        let system_fonts = list_system_fonts();
        let num_bars = config.audio.num_bars;
        let last_decay_mode = config.audio.decay_mode;
        let show_settings = config.open_settings_on_launch;
        let old_settings_scale = config.settings_scale;

        let (track_load_tx, track_load_rx) = crossbeam_channel::unbounded();

        // WGPU setup
        let wgpu_render_state = cc.wgpu_render_state.as_ref().expect("WGPU render state required");
        let resources = setup_wgpu(wgpu_render_state);
        wgpu_render_state.renderer.write().callback_resources.insert(resources);

        Self {
            current_bars: vec![0.0; num_bars],
            current_peaks: vec![0.0; num_bars],
            last_bpm: 0.0,
            shared_frame,
            audio_settings,
            show_settings,
            config,
            should_quit,
            current_fps: 0.0,
            is_passthrough_set: false,
            last_frame_time: Instant::now(),
            new_preset_name: String::new(),
            new_audio_preset_name: String::new(),
            last_decay_mode,
            track_rx,
            track_info: None,
            cover_texture: None,
            cover_image: None,
            track_id: 0,
            active_palettes: ActivePalettes::default(),
            static_single_cache: PaletteCache::default(),
            static_multi_cache: PaletteCache::default(),
            gradient_cache: PaletteCache::default(),
            animated_cache: PaletteCache::default(),
            last_palette_hash: 0,
            loaded_font_path: Some("__init__".to_string()),
            system_fonts,
            smtc_session_count,
            settings_tab: SettingsTab::Visualizer,
            color_subtab: ColorSubTab::Single,
            settings_search: String::new(),
            pending_audio_preset_delete: None,
            settings_icon: egui_icon,
            old_settings_scale,
            last_audio_frame_time: Instant::now() - Duration::from_secs(5),
            track_load_tx,
            track_load_rx,
            _tray_icon: tray_icon,
            _hotkey_manager: hotkey_manager,
            settings_menu_id: settings_item.id().clone(),
            quit_menu_id: quit_item.id().clone(),
        }
    }

    fn poll_track_updates(&mut self, ctx: &egui::Context) {
        while let Ok(result) = self.track_load_rx.try_recv() {
            self.track_id = compute_track_id(
                result.cover_art.as_deref(),
                &result.artist,
                &result.title,
            );
            if let Some((_colors, img)) = result.extracted {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let color_image = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                self.cover_texture = Some(ctx.load_texture("cover_art", color_image, egui::TextureOptions::LINEAR));
                self.cover_image = Some(img);
            } else {
                self.cover_texture = None;
                self.cover_image = None;
            }
        }

        while let Ok(info) = self.track_rx.try_recv() {
            if info.title.is_empty() && info.artist.is_empty() {
                self.track_info = None;
                self.cover_texture = None;
                self.cover_image = None;
                self.track_id = 0;
                continue;
            }

            let artist = info.artist.clone();
            let title = info.title.clone();
            let cover = info.cover_art.clone();
            self.track_info = Some(info);

            let tx = self.track_load_tx.clone();
            std::thread::spawn(move || {
                let extracted = cover.as_deref().and_then(|bytes| color::extract_colors_from_bytes(bytes));
                let _ = tx.send(TrackLoadResult { artist, title, cover_art: cover, extracted });
            });
        }
    }

    fn reload_custom_font(&mut self, ctx: &egui::Context) {
        let target = self.config.track_info.font_path.clone();
        let mut fonts = egui::FontDefinitions::default();
        let mut custom_family_names: Vec<String> = Vec::new();
        if let Some(ref path) = target {
            if let Ok(data) = std::fs::read(path) {
                fonts.font_data.insert(CUSTOM_FONT_KEY.to_owned(), egui::FontData::from_owned(data));
                custom_family_names.push(CUSTOM_FONT_KEY.to_owned());
            }
        }
        if custom_family_names.is_empty() {
            if let Some(defaults) = fonts.families.get(&egui::FontFamily::Proportional).cloned() {
                custom_family_names = defaults;
            }
        }
        fonts.families.insert(custom_font_family(), custom_family_names);
        ctx.set_fonts(fonts);
        self.loaded_font_path = target;
    }

    fn update_palettes(&mut self) {
        match self.config.color_mode {
            ColorMode::StaticSingle => self.update_static_single_palette(),
            ColorMode::StaticMultiple => self.update_static_multi_palette(),
            ColorMode::Gradient => self.update_gradient_palette(),
            ColorMode::Animated => self.update_animated_palette(),
        }
    }

    fn update_static_single_palette(&mut self) {
        let colors = match self.config.static_single_source.clone() {
            ColorSource::Static(colors) => vec![colors.first().copied().unwrap_or([255, 255, 255])],
            ColorSource::Dynamic { priority, .. } => {
                let tid = self.track_id;
                self.static_single_cache.get(tid, 1, 0, priority).map(|s| s.to_vec()).unwrap_or_else(|| {
                    let extracted = self.cover_image.as_ref()
                        .map(|img| color::extract_palette(img, 1, 0, priority))
                        .unwrap_or_else(|| vec![[255, 255, 255]]);
                    self.static_single_cache.store(tid, 1, 0, priority, extracted.clone());
                    extracted
                })
            }
        };
        self.active_palettes.static_single_colors = colors;
    }

    fn update_static_multi_palette(&mut self) {
        let colors = match self.config.static_multi_source.clone() {
            ColorSource::Static(colors) => if colors.is_empty() { vec![[255, 255, 255]] } else { colors },
            ColorSource::Dynamic { palette_size, variants_per_color, priority } => {
                let tid = self.track_id;
                self.static_multi_cache.get(tid, palette_size, variants_per_color, priority)
                    .map(|s| s.to_vec())
                    .unwrap_or_else(|| {
                        let extracted = self.cover_image.as_ref()
                            .map(|img| color::extract_palette(img, palette_size, variants_per_color, priority))
                            .unwrap_or_else(|| vec![[255, 255, 255]]);
                        self.static_multi_cache.store(tid, palette_size, variants_per_color, priority, extracted.clone());
                        extracted
                    })
            }
        };
        self.active_palettes.static_multi_colors = colors;
    }

    fn update_gradient_palette(&mut self) {
        let colors = match self.config.gradient_source.clone() {
            GradientSource::Static(stops) => stops.iter().map(|s| s.color).collect::<Vec<_>>(),
            GradientSource::Dynamic { palette_size, variants_per_color, priority } => {
                let tid = self.track_id;
                self.gradient_cache.get(tid, palette_size, variants_per_color, priority)
                    .map(|s| s.to_vec())
                    .unwrap_or_else(|| {
                        let extracted = self.cover_image.as_ref()
                            .map(|img| color::extract_palette(img, palette_size, variants_per_color, priority))
                            .unwrap_or_else(|| vec![[255, 255, 255]]);
                        self.gradient_cache.store(tid, palette_size, variants_per_color, priority, extracted.clone());
                        extracted
                    })
            }
        };
        self.active_palettes.gradient_stops = match &self.config.gradient_source {
            GradientSource::Static(stops) => stops.clone(),
            GradientSource::Dynamic { .. } => stops_from_colors(&colors),
        };
        self.active_palettes.gradient_colors = colors;
    }

    fn update_animated_palette(&mut self) {
        let colors = match self.config.animated_source.clone() {
            ColorSource::Static(colors) => if colors.is_empty() { vec![[255, 255, 255]] } else { colors },
            ColorSource::Dynamic { palette_size, variants_per_color, priority } => {
                let tid = self.track_id;
                self.animated_cache.get(tid, palette_size, variants_per_color, priority)
                    .map(|s| s.to_vec())
                    .unwrap_or_else(|| {
                        let extracted = self.cover_image.as_ref()
                            .map(|img| color::extract_palette(img, palette_size, variants_per_color, priority))
                            .unwrap_or_else(|| vec![[255, 255, 255]]);
                        self.animated_cache.store(tid, palette_size, variants_per_color, priority, extracted.clone());
                        extracted
                    })
            }
        };
        self.active_palettes.animated_colors = colors;
    }

    fn palette_fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.track_id.hash(&mut h);
        std::mem::discriminant(&self.config.color_mode).hash(&mut h);
        hash_color_source(&self.config.static_single_source, &mut h);
        hash_color_source(&self.config.static_multi_source, &mut h);
        hash_gradient_source(&self.config.gradient_source, &mut h);
        hash_color_source(&self.config.animated_source, &mut h);
        h.finish()
    }

    fn get_visualizer_text_color(&self, time: f32) -> [u8; 3] {
        match self.config.color_mode {
            ColorMode::StaticSingle => self.active_palettes.static_single_colors.first().copied().unwrap_or([255, 255, 255]),
            ColorMode::StaticMultiple => self.active_palettes.static_multi_colors.first().copied().unwrap_or([255, 255, 255]),
            ColorMode::Gradient => color::sample_gradient(&self.active_palettes.gradient_stops, 0.5),
            ColorMode::Animated => { let c = self.sample_color_changing(0.5, 0.5, time); [c.r(), c.g(), c.b()] }
        }
    }

    fn apply_vis(&self, c: egui::Color32) -> egui::Color32 {
        let a = (c.a() as u32 * self.config.visualizer_opacity as u32 / 255) as u8;
        egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
    }

    fn get_color_at(&self, i: usize, total_bars: usize, x: f32, y: f32, t: f32, rect: egui::Rect, time: f32) -> egui::Color32 {
        let c = match self.config.color_mode {
            ColorMode::StaticSingle => {
                let c = self.active_palettes.static_single_colors.first().copied().unwrap_or([255, 255, 255]);
                egui::Color32::from_rgb(c[0], c[1], c[2])
            }
            ColorMode::StaticMultiple => {
                let colors = &self.active_palettes.static_multi_colors;
                if colors.is_empty() { return egui::Color32::WHITE; }
                let num_colors = colors.len();
                if self.config.static_mode == StaticDistributionMode::Vertical {
                    let blocks = self.config.vertical_color_style == VerticalColorStyle::Blocks;
                    return static_multi_vertical_color(colors, t, blocks);
                }
                let idx = match self.config.static_mode {
                    StaticDistributionMode::Blocks => {
                        let mut chunk = (i as f32 / total_bars as f32 * num_colors as f32) as usize;
                        if chunk >= num_colors { chunk = num_colors - 1; }
                        chunk
                    }
                    StaticDistributionMode::Sequential => {
                        let bars_per_color = self.config.sequential_bars_per_color.max(1);
                        (i / bars_per_color) % num_colors
                    }
                    StaticDistributionMode::Random => random_color_index(i, num_colors),
                    StaticDistributionMode::Vertical => 0,
                };
                let c = colors.get(idx).copied().unwrap_or([255, 255, 255]);
                egui::Color32::from_rgb(c[0], c[1], c[2])
            }
            ColorMode::Gradient => {
                let angle_rad = self.config.gradient_angle.to_radians();
                let dir = egui::vec2(angle_rad.cos(), angle_rad.sin());
                let corners = [rect.left_top(), rect.right_top(), rect.left_bottom(), rect.right_bottom()];
                let mut min_proj = f32::MAX;
                let mut max_proj = f32::MIN;
                for c in corners {
                    let p = c.x * dir.x + c.y * dir.y;
                    min_proj = min_proj.min(p);
                    max_proj = max_proj.max(p);
                }
                let bar_proj = x * dir.x + y * dir.y;
                let t_grad = if (max_proj - min_proj).abs() > 0.0001 {
                    ((bar_proj - min_proj) / (max_proj - min_proj)).clamp(0.0, 1.0)
                } else { 0.0 };
                let c = color::sample_gradient(&self.active_palettes.gradient_stops, t_grad);
                egui::Color32::from_rgb(c[0], c[1], c[2])
            }
            ColorMode::Animated => {
                let norm_x = ((x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
                let norm_y = ((y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
                self.sample_color_changing(norm_x, norm_y, time)
            }
        };
        self.apply_vis(c)
    }

    fn sample_color_changing(&self, x: f32, y: f32, time: f32) -> egui::Color32 {
        let colors = &self.active_palettes.animated_colors;
        if colors.is_empty() { return egui::Color32::WHITE; }
        let m = &self.config.color_change_motion;
        let speed_hz = 0.05 + (self.config.color_change_speed * 1.95);
        let effective_speed = speed_hz * m.speed;
        let base_t = time * effective_speed;
        let s = self.evaluate_motion(m.shape, x, y, time, base_t);
        let ca = color::cyclic_color(colors, s.phase_a);
        let cb = color::cyclic_color(colors, s.phase_b);
        let t = s.blend_t.clamp(0.0, 1.0);
        let k = s.intensity.clamp(0.0, 1.0);
        let pr = ca[0] as f32 * (1.0 - t) + cb[0] as f32 * t;
        let pg = ca[1] as f32 * (1.0 - t) + cb[1] as f32 * t;
        let pb = ca[2] as f32 * (1.0 - t) + cb[2] as f32 * t;

        let off = m.off_color;
        let off_a = m.off_color_opacity as f32 / 255.0;
        let r = off[0] as f32 * (1.0 - k) + pr * k;
        let g = off[1] as f32 * (1.0 - k) + pg * k;
        let b = off[2] as f32 * (1.0 - k) + pb * k;
        let a = off_a + (1.0 - off_a) * k;

        egui::Color32::from_rgba_unmultiplied(r as u8, g as u8, b as u8, (a * 255.0) as u8)
    }

    fn evaluate_motion(&self, shape: MotionShape, x: f32, y: f32, time: f32, base_t: f32) -> MotionSample {
        let m = &self.config.color_change_motion;
        let hard = m.blend == BlendMode::Hard;
        let freq = m.wave_frequency;
        match shape {
            MotionShape::Uniform => MotionSample { phase_a: base_t, intensity: 1.0, ..Default::default() },
            MotionShape::HorizontalWipe => {
                let boundary = tri01(base_t * 0.5);
                let t = wipe_blend(x, boundary, m.wipe_softness, hard);
                MotionSample { phase_a: base_t, phase_b: base_t + 0.5, blend_t: t, intensity: 1.0 }
            }
            MotionShape::VerticalWipe => {
                let boundary = tri01(base_t * 0.5);
                let t = wipe_blend(y, boundary, m.wipe_softness, hard);
                MotionSample { phase_a: base_t, phase_b: base_t + 0.5, blend_t: t, intensity: 1.0 }
            }
            MotionShape::DiagonalWipe => {
                let boundary = tri01(base_t * 0.5);
                let diag = (x + y) * 0.5;
                let t = wipe_blend(diag, boundary, m.wipe_softness, hard);
                MotionSample { phase_a: base_t, phase_b: base_t + 0.5, blend_t: t, intensity: 1.0 }
            }
            MotionShape::RadialWipe => {
                let boundary = tri01(base_t * 0.5);
                let dx = x - 0.5;
                let dy = y - 0.5;
                let r = (dx * dx + dy * dy).sqrt() * 2.0;
                let t = wipe_blend(r, boundary, m.wipe_softness, hard);
                MotionSample { phase_a: base_t, phase_b: base_t + 0.5, blend_t: t, intensity: 1.0 }
            }
            MotionShape::Spiral => {
                let dx = x - 0.5;
                let dy = y - 0.5;
                let r = (dx * dx + dy * dy).sqrt() * 2.0;
                let angle = dy.atan2(dx) / TAU + 0.5;
                let t = (angle + r * 2.0 + base_t).fract();
                MotionSample { phase_a: base_t, phase_b: base_t + 0.5, blend_t: t, intensity: 1.0 }
            }
            MotionShape::WaveInterference => {
                let wave1 = (x * freq * TAU + base_t * TAU).sin();
                let wave2 = (y * freq * TAU + base_t * TAU * 0.7).sin();
                let wave = (wave1 + wave2) * 0.25 + 0.5;
                MotionSample { phase_a: base_t + wave * 0.5, intensity: 1.0, ..Default::default() }
            }
            MotionShape::Ripple => {
                let dx = x - 0.5;
                let dy = y - 0.5;
                let r = (dx * dx + dy * dy).sqrt() * 2.0;
                let wave = (r * 8.0 - base_t * TAU).sin() * 0.5 + 0.5;
                MotionSample { phase_a: base_t + wave * 0.5, intensity: 1.0, ..Default::default() }
            }
            MotionShape::SineWave => { let w = (x * freq * TAU + base_t * TAU).sin() * 0.5 + 0.5; MotionSample { phase_a: base_t + w * 0.5, intensity: 1.0, ..Default::default() } }
            MotionShape::TriangleWave => { let w = tri01(x * freq + base_t); MotionSample { phase_a: base_t + w * 0.5, intensity: 1.0, ..Default::default() } }
            MotionShape::SquareWave => { let w = ((x * freq + base_t).fract() < 0.5) as u8 as f32; MotionSample { phase_a: base_t + w * 0.5, intensity: 1.0, ..Default::default() } }
            MotionShape::SawtoothWave => { let w = (x * freq + base_t).fract(); MotionSample { phase_a: base_t + w * 0.5, intensity: 1.0, ..Default::default() } }
            MotionShape::Strobe => {
                let on = ((base_t * 4.0).fract() < 0.5) as u8 as f32;
                let k = if hard { on } else { smoothstep(0.35, 0.65, on + base_t.fract() * 0.3) };
                MotionSample { phase_a: base_t, intensity: k, ..Default::default() }
            }
            MotionShape::Pulse => { let k = (base_t * TAU).sin() * 0.5 + 0.5; MotionSample { phase_a: base_t, intensity: k, ..Default::default() } }
            MotionShape::Heartbeat => {
                let beat = base_t.fract();
                let k = if hard {
                    if beat < 0.10 { 1.0 } else if beat < 0.18 { 0.0 } else if beat < 0.26 { 0.7 } else { 0.0 }
                } else {
                    let a = (-((beat - 0.08) / 0.04).powi(2)).exp();
                    let b = (-((beat - 0.22) / 0.04).powi(2)).exp() * 0.7;
                    (a + b).clamp(0.0, 1.0)
                };
                MotionSample { phase_a: base_t, phase_b: base_t + 0.5, blend_t: 0.0, intensity: k }
            }
            MotionShape::Stutter => {
                let step = (time * 10.0) as u32;
                let pos = (x * 200.0) as u32;
                let hash = hash_u32(step.wrapping_mul(37).wrapping_add(pos));
                let event = (hash % 100) as f32 / 100.0 < m.stutter_intensity;
                let offset = if event { 0.5 } else { 0.0 };
                MotionSample { phase_a: base_t + offset, intensity: 1.0, ..Default::default() }
            }
            MotionShape::Comet => {
                let head = base_t.fract();
                let raw = (x - head).abs();
                let dist = raw.min(1.0 - raw);
                let k = (1.0 - dist / m.trail_length.max(0.01)).clamp(0.0, 1.0);
                let k = if hard { (k > 0.5) as u8 as f32 } else { k };
                MotionSample { phase_a: base_t, intensity: k, ..Default::default() }
            }
            MotionShape::Chase => {
                let pos = (x - base_t * 0.5).rem_euclid(1.0);
                let k = if hard { (pos < 0.15) as u8 as f32 } else { smoothstep(0.05, 0.15, 0.15 - (pos - 0.075).abs()) };
                MotionSample { phase_a: base_t, intensity: k, ..Default::default() }
            }
            MotionShape::BouncingBall => {
                let head = tri01(base_t * 0.5);
                let dist = (x - head).abs();
                let k = (1.0 - dist / m.trail_length.max(0.01)).clamp(0.0, 1.0);
                let k = if hard { (k > 0.4) as u8 as f32 } else { k };
                MotionSample { phase_a: base_t, intensity: k, ..Default::default() }
            }
            MotionShape::Sparkle => {
                let step = (time * 12.0) as u32;
                let pos = (x * 300.0) as u32;
                let hash = hash_u32(step.wrapping_mul(13).wrapping_add(pos));
                let p = (hash % 100) as f32 / 100.0;
                let on = p < 0.04 + m.stutter_intensity * 0.1;
                MotionSample { phase_a: base_t + p, intensity: if on { 1.0 } else { 0.0 }, ..Default::default() }
            }
            MotionShape::Sinelon => {
                let head = tri01(base_t * 0.5);
                let dist = (x - head).abs();
                let k = (1.0 - dist / m.trail_length.max(0.01)).clamp(0.0, 1.0);
                MotionSample { phase_a: base_t, intensity: k.powf(2.0), ..Default::default() }
            }
            MotionShape::FireSimulation => {
                let n = (base_t * 1.0 + x * 5.0).sin() + (base_t * 1.3 + x * 11.0).sin() * 0.5 + (base_t * 2.1 + x * 23.0).sin() * 0.25;
                MotionSample { phase_a: base_t + n * 0.05, intensity: (n * 0.25 + 0.75).clamp(0.0, 1.0), ..Default::default() }
            }
            MotionShape::Plasma => {
                let p = (x * 6.0 + base_t * 2.0).sin() + (y * 6.0 - base_t * 1.5).sin() + ((x + y) * 4.0 + base_t).sin();
                MotionSample { phase_a: base_t + p * 0.08, intensity: 1.0, ..Default::default() }
            }
            MotionShape::RainbowCycle => MotionSample { phase_a: base_t + x, intensity: 1.0, ..Default::default() },
            MotionShape::Confetti => {
                let step = (time * 10.0) as u32;
                let pos = (x * 400.0) as u32;
                let hash = hash_u32(step.wrapping_mul(97).wrapping_add(pos));
                let p = (hash % 100) as f32 / 100.0;
                let on = p < m.stutter_intensity * 0.15;
                MotionSample { phase_a: (hash % 1000) as f32 / 1000.0 + base_t * 0.1, intensity: if on { 1.0 } else { 0.0 }, ..Default::default() }
            }
            MotionShape::Breathing => {
                let k = (base_t * TAU * 0.5).sin() * 0.5 + 0.5;
                MotionSample { phase_a: base_t, intensity: k, ..Default::default() }
            }
            MotionShape::FlipFlop => {
                let k = if (base_t * 2.0).fract() < 0.5 { 1.0 } else { 0.0 };
                MotionSample { phase_a: base_t, intensity: k, ..Default::default() }
            }
            MotionShape::Kaleidoscope => {
                let wave = ((x * freq * TAU + base_t * TAU).sin() * (y * freq * TAU + base_t * TAU).cos()).abs();
                MotionSample { phase_a: base_t + wave * 0.5, intensity: 1.0, ..Default::default() }
            }
            MotionShape::DigitalRain => {
                let col = (x * 50.0) as u32;
                let hash = hash_u32(col);
                let speed = 1.0 + (hash % 100) as f32 / 50.0;
                let y_offset = (hash % 1000) as f32 / 1000.0;
                let pos = (y + base_t * speed + y_offset).fract();
                let k = if pos < 0.2 { 1.0 - pos / 0.2 } else { 0.0 };
                MotionSample { phase_a: base_t + col as f32 * 0.1, intensity: k, ..Default::default() }
            }
            MotionShape::MeteorShower => {
                let head = base_t.fract() * 2.0;
                let dist = ((x * 2.0 - head) % 1.0).abs();
                let k = (1.0 - dist / 0.15).clamp(0.0, 1.0);
                MotionSample { phase_a: base_t, intensity: k, ..Default::default() }
            }
        }
    }

    fn draw_styled_text(&self, ctx: &egui::Context, painter: &egui::Painter, text: &str, pos: egui::Pos2, font_id: &egui::FontId, align: egui::Align2, color: egui::Color32, cfg: &TrackInfoConfig) {
        if text.is_empty() { return; }
        let ls = cfg.letter_spacing;
        if cfg.shadow_enabled {
            let base_alpha = (cfg.opacity as f32 * 0.6).max(1.0);
            let offsets: Vec<(f32, f32)> = if cfg.shadow_blur < 1.0 { vec![(0.0, 0.0)] } else {
                let mut v = vec![(0.0, 0.0)];
                let n = 12;
                for i in 0..n { let a = i as f32 / n as f32 * TAU; v.push((a.cos() * cfg.shadow_blur, a.sin() * cfg.shadow_blur)); }
                v
            };
            let per_alpha = (base_alpha / (offsets.len() as f32).sqrt()).clamp(1.0, 255.0) as u8;
            let s_color = egui::Color32::from_rgba_unmultiplied(cfg.shadow_color[0], cfg.shadow_color[1], cfg.shadow_color[2], per_alpha);
            for (dx, dy) in offsets {
                let sp = pos + egui::vec2(cfg.shadow_offset_x + dx, cfg.shadow_offset_y + dy);
                draw_spaced(ctx, painter, text, sp, font_id, align, s_color, ls);
            }
        }
        if cfg.outline_enabled && cfg.outline_thickness > 0.1 {
            let o_color = egui::Color32::from_rgba_unmultiplied(cfg.outline_color[0], cfg.outline_color[1], cfg.outline_color[2], cfg.opacity);
            let t = cfg.outline_thickness;
            let n = 12;
            for i in 0..n {
                let a = i as f32 / n as f32 * TAU;
                let op = pos + egui::vec2(a.cos() * t, a.sin() * t);
                draw_spaced(ctx, painter, text, op, font_id, align, o_color, ls);
            }
        }
        draw_spaced(ctx, painter, text, pos, font_id, align, color, ls);
    }

    fn draw_track_info(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let Some(info) = &self.track_info else { return };
        let cfg = &self.config.track_info;
        let ctx = ui.ctx();
        let painter = ui.painter();
        let time = ui.input(|i| i.time) as f32;
        let margin = 40.0;
        let art_size = cfg.cover_size;
        let text_gap = 14.0;
        let base_rgb = match cfg.color_mode {
            TrackTextColorMode::Static => cfg.static_color,
            TrackTextColorMode::Visualizer => self.get_visualizer_text_color(time),
        };
        let text_color = egui::Color32::from_rgba_unmultiplied(base_rgb[0], base_rgb[1], base_rgb[2], cfg.opacity);
        let family = if cfg.font_path.is_some() { custom_font_family() } else { egui::FontFamily::Proportional };
        let artist_font = egui::FontId::new(cfg.artist_font_size, family.clone());
        let title_font = egui::FontId::new(cfg.title_font_size, family);
        let align = match cfg.alignment {
            TrackTextAlign::Left => egui::Align2::LEFT_TOP,
            TrackTextAlign::Center => egui::Align2::CENTER_TOP,
            TrackTextAlign::Right => egui::Align2::RIGHT_TOP,
        };
        let cover_visible = cfg.show_cover_art && self.cover_texture.is_some();
        if cover_visible {
            if let Some(tex) = &self.cover_texture {
                let art_x = rect.right() - margin - art_size + cfg.cover_x_offset;
                let art_y = rect.top() + margin + cfg.cover_y_offset;
                let art_rect = egui::Rect::from_min_size(egui::pos2(art_x, art_y), egui::vec2(art_size, art_size));
                let tint = egui::Color32::from_white_alpha(cfg.cover_opacity);
                painter.image(tex.id(), art_rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), tint);
            }
        }
        let mut text_y = rect.top() + margin + cfg.text_y_offset;
        if cover_visible { text_y += art_size + text_gap; }
        let text_x = rect.right() - margin + cfg.text_x_offset;
        let mut current_y = text_y;
        if cfg.show_artist && !info.artist.is_empty() {
            self.draw_styled_text(ctx, painter, &info.artist, egui::pos2(text_x, current_y), &artist_font, align, text_color, cfg);
            current_y += cfg.artist_font_size * cfg.line_height;
        }
        if cfg.show_title && !info.title.is_empty() {
            self.draw_styled_text(ctx, painter, &info.title, egui::pos2(text_x, current_y), &title_font, align, text_color, cfg);
        }
    }
}

fn draw_spaced(ctx: &egui::Context, painter: &egui::Painter, text: &str, pos: egui::Pos2, font_id: &egui::FontId, align: egui::Align2, color: egui::Color32, letter_spacing: f32) {
    if text.is_empty() { return; }
    if letter_spacing.abs() < 0.01 { painter.text(pos, align, text, font_id.clone(), color); return; }
    let chars: Vec<char> = text.chars().collect();
    let widths: Vec<f32> = chars.iter().map(|&ch| ctx.fonts(|f| f.glyph_width(font_id, ch))).collect();
    let total_width: f32 = widths.iter().sum::<f32>() + letter_spacing * (chars.len().saturating_sub(1)) as f32;
    let start_x = match align {
        egui::Align2::LEFT_TOP | egui::Align2::LEFT_CENTER | egui::Align2::LEFT_BOTTOM => pos.x,
        egui::Align2::RIGHT_TOP | egui::Align2::RIGHT_CENTER | egui::Align2::RIGHT_BOTTOM => pos.x - total_width,
        egui::Align2::CENTER_TOP | egui::Align2::CENTER_CENTER | egui::Align2::CENTER_BOTTOM => pos.x - total_width / 2.0,
    };
    let mut x = start_x;
    for (ch, w) in chars.iter().zip(widths.iter()) {
        painter.text(egui::pos2(x, pos.y), egui::Align2::LEFT_TOP, ch.to_string(), font_id.clone(), color);
        x += w + letter_spacing;
    }
}

// ═══════════════════════════════════════════════════════════════
// GPU bar drawing
// ═══════════════════════════════════════════════════════════════

impl VisualizerApp {
    fn draw_bars_gpu(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let base_num_bars = self.current_bars.len();
        if base_num_bars == 0 { return; }

        if self.current_bars.iter().all(|&v| v < 0.001) && self.current_peaks.iter().all(|&p| p < 0.001) {
            return;
        }

        let visual_num_bars = if self.config.frequency_direction == FrequencyDirection::Mirrored {
            base_num_bars * 2
        } else {
            base_num_bars
        };

        let time = ui.input(|i| i.time) as f32;
        let dir = self.config.visualizer_direction;

        let mut gpu_bars = Vec::with_capacity(visual_num_bars);
        for i in 0..visual_num_bars {
            let idx = frequency_index(i, base_num_bars, self.config.frequency_direction);
            let height = self.current_bars[idx].min(1.0);
            let peak = self.current_peaks[idx].min(1.0);

            let (color_top, color_bottom) = match self.config.color_mode {
                ColorMode::Gradient => {
                    let rect_w = rect.width();
                    let rect_h = rect.height();
                    let x_center = rect.left() + (i as f32 / visual_num_bars as f32) * rect_w;
                    let cb = self.get_color_at(i, visual_num_bars, x_center, rect.bottom(), 0.0, rect, time);
                    let ct = self.get_color_at(i, visual_num_bars, x_center, rect.top() + (1.0 - height) * rect_h, 1.0, rect, time);
                    (ct, cb)
                }
                ColorMode::Animated => {
                    let norm_x = i as f32 / visual_num_bars as f32;
                    let cb = self.sample_color_changing(norm_x, 0.0, time);
                    let ct = self.sample_color_changing(norm_x, height, time);
                    (ct, cb)
                }
                _ => {
                    let c = self.get_color_at(i, visual_num_bars, 0.0, 0.0, 0.0, rect, time);
                    (c, c)
                }
            };

            gpu_bars.push(GpuBar {
                height,
                peak_height: peak,
                _pad0: [0.0; 2],
                color_top: [
                    color_top.r() as f32 / 255.0,
                    color_top.g() as f32 / 255.0,
                    color_top.b() as f32 / 255.0,
                    color_top.a() as f32 / 255.0,
                ],
                color_bottom: [
                    color_bottom.r() as f32 / 255.0,
                    color_bottom.g() as f32 / 255.0,
                    color_bottom.b() as f32 / 255.0,
                    color_bottom.a() as f32 / 255.0,
                ],
                show_peak: if self.config.audio.peak_hold_ms > 0.0 && peak > 0.001 { 1.0 } else { 0.0 },
                _pad1: [0.0; 3],
            });
        }

        let uniforms = ShaderUniforms {
            dims: [
                visual_num_bars as f32,
                (dir == VisualizerDirection::Up || dir == VisualizerDirection::Down) as u32 as f32,
                (self.config.bar_interpolation == BarInterpolation::Smooth) as u32 as f32,
                (self.config.frequency_direction == FrequencyDirection::Mirrored) as u32 as f32,
            ],
            layout: [
                self.config.bar_width,
                self.config.bar_gap,
                rect.width(),
                rect.height(),
            ],
            outline_color: [
                self.config.visualizer_outline_color[0] as f32 / 255.0,
                self.config.visualizer_outline_color[1] as f32 / 255.0,
                self.config.visualizer_outline_color[2] as f32 / 255.0,
                1.0,
            ],
            extras: [
                if self.config.visualizer_outline_enabled { self.config.visualizer_outline_thickness } else { 0.0 },
                self.config.bar_radius,
                self.config.visualizer_opacity as f32 / 255.0,
                match dir {
                    VisualizerDirection::Up => 0.0,
                    VisualizerDirection::Down => 1.0,
                    VisualizerDirection::Right => 2.0,
                    VisualizerDirection::Left => 3.0,
                },
            ],
        };

        let callback = egui_wgpu::Callback::new_paint_callback(
            rect,
            VisualizerCallback { bars: gpu_bars, uniforms },
        );
        ui.painter().add(callback);
    }
}

// ═══════════════════════════════════════════════════════════════
// Settings UI (unchanged - identical to previous version)
// ═══════════════════════════════════════════════════════════════

impl VisualizerApp {
    fn draw_settings_ui(&mut self, ui: &mut egui::Ui) {
        let scale = self.config.settings_scale.clamp(0.75, 1.5);

        egui::SidePanel::left("settings_nav")
            .resizable(false)
            .exact_width(140.0)
            .frame(egui::Frame::none().fill(egui::Color32::BLACK).inner_margin(egui::Margin::same(8.0)))
            .show_inside(ui, |ui| {
                self.draw_sidebar(ui);
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(egui::Color32::BLACK).inner_margin(egui::Margin::same(12.0)))
            .show_inside(ui, |ui| {
                ui.style_mut().text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
                ui.style_mut().text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
                ui.style_mut().text_styles.insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
                ui.style_mut().text_styles.insert(egui::TextStyle::Heading, egui::FontId::proportional(18.0));

                ui.style_mut().spacing.item_spacing *= scale;
                ui.style_mut().spacing.button_padding *= scale;
                ui.style_mut().spacing.slider_width *= scale;
                ui.style_mut().spacing.combo_width *= scale;

                for (_, font) in ui.style_mut().text_styles.iter_mut() {
                    font.size *= scale;
                }

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        match self.settings_tab {
                            SettingsTab::Visualizer  => self.tab_visualizer(ui),
                            SettingsTab::Colors      => self.tab_colors(ui),
                            SettingsTab::Audio       => self.tab_audio(ui),
                            SettingsTab::TrackInfo   => self.tab_track_info(ui),
                            SettingsTab::Appearance  => self.tab_appearance(ui),
                            SettingsTab::Performance => self.tab_performance(ui),
                        }
                    });
            });
    }

    fn draw_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Settings").size(16.0).strong().color(accent()));
        ui.add_space(8.0);

        ui.add(egui::TextEdit::singleline(&mut self.settings_search)
            .hint_text("Search…")
            .desired_width(f32::INFINITY));
        ui.add_space(6.0);

        let query = self.settings_search.trim().to_lowercase();
        if !query.is_empty() {
            ui.label(egui::RichText::new("Results").small().weak());
            let mut any = false;
            for (tab, name, keywords) in SEARCH_INDEX {
                let hit = name.to_lowercase().contains(&query)
                    || keywords.iter().any(|k| k.contains(&query));
                if hit {
                    any = true;
                    if ui.small_button(*name).clicked() {
                        self.settings_tab = *tab;
                    }
                }
            }
            if !any {
                ui.label(egui::RichText::new("No matches").small().weak());
            }
            ui.add_space(6.0);
            ui.separator();
        }

        ui.add_space(4.0);
        let tabs = [
            (SettingsTab::Visualizer,  "Visualizer"),
            (SettingsTab::Colors,      "Colors"),
            (SettingsTab::Audio,       "Audio"),
            (SettingsTab::TrackInfo,   "Track Info"),
            (SettingsTab::Appearance,  "Appearance"),
            (SettingsTab::Performance, "Performance"),
        ];
        for (tab, label) in tabs {
            let selected = self.settings_tab == tab;
            let mut text = egui::RichText::new(label).size(13.0);
            if selected { text = text.strong().color(accent()); }
            if ui.add_sized([ui.available_width(), 26.0],
                            egui::SelectableLabel::new(selected, text)).clicked() {
                self.settings_tab = tab;
            }
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.add_space(8.0);
            if ui.add_sized([ui.available_width(), 24.0],
                egui::Button::new(egui::RichText::new("Close").size(13.0)).frame(false)).clicked()
            {
                self.show_settings = false;
                self.config.save();
            }
            if ui.add_sized([ui.available_width(), 24.0],
                egui::Button::new(egui::RichText::new("Reset All").size(13.0).color(egui::Color32::from_rgb(255, 100, 100))).frame(false)).clicked()
            {
                self.config = AppConfig::default();
                self.last_palette_hash = 0;
                self.loaded_font_path = Some("__reset__".to_string());
            }
        });
    }

    fn tab_visualizer(&mut self, ui: &mut egui::Ui) {
        use cfg::*;

        section_header(ui, "Direction");
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.config.visualizer_direction, VisualizerDirection::Up, "Up");
            ui.radio_value(&mut self.config.visualizer_direction, VisualizerDirection::Down, "Down");
            ui.radio_value(&mut self.config.visualizer_direction, VisualizerDirection::Left, "Left");
            ui.radio_value(&mut self.config.visualizer_direction, VisualizerDirection::Right, "Right");
            reset_btn(ui, self.config.visualizer_direction != VisualizerDirection::Up,
                      || self.config.visualizer_direction = VisualizerDirection::Up);
        });
        help_text(ui, self.config.show_help_text, "Direction bars grow from.");

        section_header(ui, "Frequency Layout");
        ui.horizontal_wrapped(|ui| {
            ui.radio_value(&mut self.config.frequency_direction, FrequencyDirection::LeftToRight, "Left to Right");
            ui.radio_value(&mut self.config.frequency_direction, FrequencyDirection::RightToLeft, "Right to Left");
            ui.radio_value(&mut self.config.frequency_direction, FrequencyDirection::Mirrored, "Mirrored (Symmetrical)");
            reset_btn(ui, self.config.frequency_direction != FrequencyDirection::LeftToRight,
                      || self.config.frequency_direction = FrequencyDirection::LeftToRight);
        });
        help_text(ui, self.config.show_help_text, "How the frequency spectrum is laid out across the screen.");

        section_header(ui, "Bar Shape");
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.config.bar_interpolation, BarInterpolation::Step, "Step");
            ui.radio_value(&mut self.config.bar_interpolation, BarInterpolation::Smooth, "Smooth");
            reset_btn(ui, self.config.bar_interpolation != BarInterpolation::Step,
                      || self.config.bar_interpolation = BarInterpolation::Step);
        });
        help_text(ui, self.config.show_help_text, self.config.bar_interpolation.description());

        section_header(ui, "Bars");
        slider_f32(ui, &mut self.config.bar_width, 1.0..=50.0, "Bar Width", DEFAULT_BAR_WIDTH);
        slider_f32(ui, &mut self.config.bar_gap, 0.0..=20.0, "Bar Gap", DEFAULT_BAR_GAP);
        ui.horizontal(|ui| {
            let mut n = self.config.audio.num_bars as i32;
            let r = ui.add(egui::Slider::new(&mut n, 16..=256).text("Bar Count"));
            if r.secondary_clicked() { n = DEFAULT_NUM_BARS as i32; }
            if n as usize != self.config.audio.num_bars { self.config.audio.num_bars = n as usize; }
            reset_btn(ui, n as usize != DEFAULT_NUM_BARS, || self.config.audio.num_bars = DEFAULT_NUM_BARS);
        });
        slider_f32(ui, &mut self.config.max_bar_height, 0.0..=1000.0, "Max Bar Height (0 = full)", DEFAULT_MAX_BAR_HEIGHT);
        slider_f32(ui, &mut self.config.height_scale, 0.1..=1.0, "Height Scale", DEFAULT_HEIGHT_SCALE);

        section_header(ui, "Position");
        slider_f32(ui, &mut self.config.x_offset, -1000.0..=1000.0, "Horizontal Offset", DEFAULT_X_OFFSET);
        slider_f32(ui, &mut self.config.y_offset, -1000.0..=1000.0, "Vertical Offset", DEFAULT_Y_OFFSET);

        section_header(ui, "Appearance");
        slider_u8(ui, &mut self.config.visualizer_opacity, "Visualizer Opacity", DEFAULT_VISUALIZER_OPACITY);
        help_text(ui, self.config.show_help_text, "0 = invisible, 255 = fully opaque. Applies to bars, peaks, and outlines.");

        checkbox_reset(ui, &mut self.config.visualizer_outline_enabled, "Enable Outlines", false);
        if self.config.visualizer_outline_enabled {
            slider_f32(ui, &mut self.config.visualizer_outline_thickness, 0.5..=10.0, "Outline Thickness", default_visualizer_outline_thickness());
            color_edit_reset(ui, &mut self.config.visualizer_outline_color, "Outline Color", default_visualizer_outline_color());
            help_text(ui, self.config.show_help_text, "Outlines follow the visualizer's shape. In Smooth mode, a single perimeter is drawn.");
        }
    }

    fn tab_colors(&mut self, ui: &mut egui::Ui) {
        use cfg::*;

        ui.horizontal_wrapped(|ui| {
            let subtabs = [
                (ColorSubTab::Single,   "Single",   ColorMode::StaticSingle),
                (ColorSubTab::Multi,    "Multi",    ColorMode::StaticMultiple),
                (ColorSubTab::Gradient, "Gradient", ColorMode::Gradient),
                (ColorSubTab::Animated, "Animated", ColorMode::Animated),
            ];
            for (sub, label, mode) in subtabs {
                let selected = self.color_subtab == sub;
                let mut text = egui::RichText::new(label).size(13.0);
                if selected { text = text.strong().color(accent()); }
                if ui.add(egui::SelectableLabel::new(selected, text)).clicked() {
                    self.color_subtab = sub;
                    self.config.color_mode = mode;
                }
                if selected { self.config.color_mode = mode; }
            }
        });
        ui.add_space(4.0);
        ui.separator();

        match self.color_subtab {
            ColorSubTab::Single   => self.color_sub_single(ui),
            ColorSubTab::Multi    => self.color_sub_multi(ui),
            ColorSubTab::Gradient => self.color_sub_gradient(ui),
            ColorSubTab::Animated => self.color_sub_animated(ui),
        }
    }

    fn color_sub_single(&mut self, ui: &mut egui::Ui) {
        let is_dynamic = self.config.static_single_source.is_dynamic();
        ui.horizontal(|ui| {
            if ui.selectable_label(!is_dynamic, "Static").clicked() && is_dynamic {
                let colors = self.active_palettes.static_single_colors.clone();
                self.config.static_single_source = ColorSource::Static(if colors.is_empty() { vec![[255, 255, 255]] } else { colors });
            }
            if ui.selectable_label(is_dynamic, "Dynamic (Album Art)").clicked() && !is_dynamic {
                self.config.static_single_source = ColorSource::Dynamic { palette_size: 1, variants_per_color: 0, priority: PalettePriority::MostCommon };
            }
        });
        if is_dynamic {
            if let ColorSource::Dynamic { priority, .. } = &mut self.config.static_single_source {
                Self::draw_dynamic_palette_settings(ui, "ss", &mut 1u8, &mut 0u8, priority, false, false);
            }
            Self::draw_palette_preview(ui, &self.active_palettes.static_single_colors);
            help_text(ui, self.config.show_help_text, "Single color picked from the album art using the selected priority.");
        } else {
            if let Some(colors) = self.config.static_single_source.static_colors_mut() {
                if colors.is_empty() { colors.push([255, 255, 255]); }
                color_edit_reset(ui, &mut colors[0], "Bar Color", [255, 255, 255]);
            }
            ui.label("Presets:");
            let mut chosen: Option<[u8; 3]> = None;
            let mut to_remove: Option<usize> = None;
            for (chunk_idx, chunk) in self.config.color_presets.chunks(12).enumerate() {
                ui.horizontal(|ui| {
                    for (i, preset) in chunk.iter().enumerate() {
                        let global_i = chunk_idx * 12 + i;
                        let (rect, response) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
                        if ui.is_rect_visible(rect) {
                            let color = egui::Color32::from_rgb(preset.color[0], preset.color[1], preset.color[2]);
                            ui.painter().rect_filled(rect, 4.0, color);
                            if response.clicked() { chosen = Some(preset.color); }
                        }
                        if !preset.is_premade {
                            let btn_rect = egui::Rect::from_center_size(rect.right_top() + egui::vec2(-2.0, 2.0), egui::vec2(16.0, 16.0));
                            let btn = ui.put(btn_rect, egui::Button::new("✖").frame(false).small());
                            if btn.clicked() { to_remove = Some(global_i); }
                        }
                    }
                });
            }
            if let Some(i) = to_remove { self.config.color_presets.remove(i); }
            if let Some(c) = chosen { self.config.static_single_source = ColorSource::Static(vec![c]); }

            if ui.button("💾 Save Current Color as Preset").clicked() {
                if let Some(colors) = self.config.static_single_source.static_colors() {
                    if let Some(c) = colors.first().copied() {
                        if !self.config.color_presets.iter().any(|p| p.color == c) {
                            self.config.color_presets.push(ColorPreset { color: c, is_premade: false });
                        }
                    }
                }
            }
        }
    }

    fn color_sub_multi(&mut self, ui: &mut egui::Ui) {
        let is_dynamic = self.config.static_multi_source.is_dynamic();
        ui.horizontal(|ui| {
            if ui.selectable_label(!is_dynamic, "Static").clicked() && is_dynamic {
                let colors = self.active_palettes.static_multi_colors.clone();
                self.config.static_multi_source = ColorSource::Static(if colors.is_empty() { vec![[255, 255, 255]] } else { colors });
            }
            if ui.selectable_label(is_dynamic, "Dynamic (Album Art)").clicked() && !is_dynamic {
                self.config.static_multi_source = ColorSource::Dynamic { palette_size: 3, variants_per_color: 0, priority: PalettePriority::MostCommon };
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.radio_value(&mut self.config.static_mode, StaticDistributionMode::Blocks, "Blocks");
            ui.radio_value(&mut self.config.static_mode, StaticDistributionMode::Sequential, "Sequential");
            ui.radio_value(&mut self.config.static_mode, StaticDistributionMode::Random, "Random");
            ui.radio_value(&mut self.config.static_mode, StaticDistributionMode::Vertical, "Vertical");
        });
        if self.config.static_mode == StaticDistributionMode::Sequential {
            ui.horizontal(|ui| {
                let mut n = self.config.sequential_bars_per_color as i32;
                let r = ui.add(egui::Slider::new(&mut n, 1..=20).text("Bars per Color"));
                if r.secondary_clicked() { n = 1; }
                self.config.sequential_bars_per_color = n.max(1) as usize;
                reset_btn(ui, self.config.sequential_bars_per_color != 1,
                          || self.config.sequential_bars_per_color = 1);
            });
        }
        if self.config.static_mode == StaticDistributionMode::Vertical {
            ui.horizontal(|ui| {
                ui.label("Vertical Color Style:");
                ui.radio_value(&mut self.config.vertical_color_style, VerticalColorStyle::Gradient, "Gradient");
                ui.radio_value(&mut self.config.vertical_color_style, VerticalColorStyle::Blocks, "Blocks");
            });
            help_text(ui, self.config.show_help_text, self.config.vertical_color_style.description());
        }
        ui.add_space(4.0);
        if is_dynamic {
            if let ColorSource::Dynamic { palette_size, variants_per_color, priority } = &mut self.config.static_multi_source {
                Self::draw_dynamic_palette_settings(ui, "sm", palette_size, variants_per_color, priority, true, true);
            }
            Self::draw_palette_preview(ui, &self.active_palettes.static_multi_colors);
        } else {
            let mut to_remove = None;
            let num_colors = self.config.static_multi_source.static_colors().map(|c| c.len()).unwrap_or(0);
            if let Some(colors) = self.config.static_multi_source.static_colors_mut() {
                for (i, color) in colors.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.color_edit_button_srgb(color);
                        if num_colors > 1 && ui.button("❌").clicked() { to_remove = Some(i); }
                    });
                }
            }
            if let Some(i) = to_remove { if let Some(colors) = self.config.static_multi_source.static_colors_mut() { colors.remove(i); } }
            if ui.button("➕ Add Color").clicked() { if let Some(colors) = self.config.static_multi_source.static_colors_mut() { colors.push([255, 255, 255]); } }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.new_preset_name);
                if ui.button("💾 Save Preset").clicked() {
                    let name = if self.new_preset_name.trim().is_empty() { format!("Preset {}", self.config.static_multi_presets.len() + 1) } else { self.new_preset_name.clone() };
                    let colors = self.config.static_multi_source.static_colors().map(|c| c.to_vec()).unwrap_or_default();
                    self.config.static_multi_presets.push(StaticMultiPreset {
                        name, colors, mode: self.config.static_mode,
                        bars_per_color: self.config.sequential_bars_per_color, is_premade: false,
                    });
                    self.new_preset_name.clear();
                }
            });
            egui::Frame::none()
                .fill(egui::Color32::from_rgb(30, 30, 30))
                .inner_margin(egui::Margin::symmetric(8.0, 4.0))
                .rounding(4.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Left-Click").strong().color(accent()));
                        ui.label("= Colors only  |  ");
                        ui.label(egui::RichText::new("Right-Click").strong().color(accent()));
                        ui.label("= All settings");
                    });
                });
            ui.add_space(4.0);

            let mut apply_colors: Option<usize> = None;
            let mut apply_full: Option<usize> = None;
            let mut remove_idx: Option<usize> = None;
            ui.horizontal_wrapped(|ui| {
                for (i, preset) in self.config.static_multi_presets.iter().enumerate() {
                    let mut btn = ui.button(&preset.name);
                    if btn.clicked() { apply_colors = Some(i); }
                    if btn.secondary_clicked() { apply_full = Some(i); }
                    btn = btn.on_hover_text("Left: colors only · Right: colors + mode");
                    let _ = btn;
                    if !preset.is_premade && ui.small_button("✖").clicked() { remove_idx = Some(i); }
                }
            });
            if let Some(i) = apply_colors {
                let p = &self.config.static_multi_presets[i];
                self.config.static_multi_source = ColorSource::Static(p.colors.clone());
            }
            if let Some(i) = apply_full {
                let p = &self.config.static_multi_presets[i];
                self.config.static_multi_source = ColorSource::Static(p.colors.clone());
                self.config.static_mode = p.mode;
                self.config.sequential_bars_per_color = p.bars_per_color;
            }
            if let Some(i) = remove_idx { if !self.config.static_multi_presets[i].is_premade { self.config.static_multi_presets.remove(i); } }
        }
    }

    fn color_sub_gradient(&mut self, ui: &mut egui::Ui) {
        let is_dynamic = self.config.gradient_source.is_dynamic();
        ui.horizontal(|ui| {
            if ui.selectable_label(!is_dynamic, "Static").clicked() && is_dynamic {
                let stops = self.active_palettes.gradient_stops.clone();
                self.config.gradient_source = GradientSource::Static(
                    if stops.len() >= 2 { stops }
                    else if stops.len() == 1 { vec![stops[0].clone(), GradientStop { color: stops[0].color, position: 1.0 }] }
                    else { vec![GradientStop { color: [255, 0, 0], position: 0.0 }, GradientStop { color: [0, 0, 255], position: 1.0 }] }
                );
            }
            if ui.selectable_label(is_dynamic, "Dynamic (Album Art)").clicked() && !is_dynamic {
                self.config.gradient_source = GradientSource::Dynamic { palette_size: 2, variants_per_color: 0, priority: PalettePriority::MostCommon };
            }
        });
        slider_f32(ui, &mut self.config.gradient_angle, 0.0..=360.0, "Angle (°)", 0.0);
        help_text(ui, self.config.show_help_text, "0° = horizontal, 90° = vertical, 45° = diagonal.");
        if is_dynamic {
            if let GradientSource::Dynamic { palette_size, variants_per_color, priority } = &mut self.config.gradient_source {
                Self::draw_dynamic_palette_settings(ui, "g", palette_size, variants_per_color, priority, true, true);
            }
            let preview: Vec<[u8; 3]> = self.active_palettes.gradient_stops.iter().map(|s| s.color).collect();
            Self::draw_palette_preview(ui, &preview);
        } else {
            let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width() - 16.0, 30.0), egui::Sense::click_and_drag());
            let painter = ui.painter();
            let preview_stops = self.config.gradient_source.static_stops().map(|s| s.to_vec()).unwrap_or_default();
            let steps = 100;
            for i in 0..steps {
                let t1 = i as f32 / steps as f32;
                let t2 = (i + 1) as f32 / steps as f32;
                let c1 = color::sample_gradient(&preview_stops, t1);
                let x1 = rect.left() + rect.width() * t1;
                let x2 = rect.left() + rect.width() * t2;
                let r = egui::Rect::from_min_max(egui::pos2(x1, rect.top()), egui::pos2(x2, rect.bottom()));
                painter.rect_filled(r, 0.0, egui::Color32::from_rgb(c1[0], c1[1], c1[2]));
            }
            let mut sort_stops = false;
            if let Some(stops) = self.config.gradient_source.static_stops_mut() {
                for (i, stop) in stops.iter_mut().enumerate() {
                    let x = rect.left() + rect.width() * stop.position;
                    let stop_rect = egui::Rect::from_center_size(egui::pos2(x, rect.center().y), egui::vec2(12.0, 30.0));
                    let stop_response = ui.interact(stop_rect, ui.id().with("gradient_stop").with(i), egui::Sense::click_and_drag());
                    if stop_response.dragged() {
                        stop.position = (stop.position + stop_response.drag_delta().x / rect.width()).clamp(0.0, 1.0);
                        sort_stops = true;
                    }
                    let handle_color = egui::Color32::from_rgb(stop.color[0], stop.color[1], stop.color[2]);
                    painter.rect_filled(stop_rect, 2.0, handle_color);
                    painter.rect_stroke(stop_rect, 2.0_f32, egui::Stroke::new(2.0_f32, egui::Color32::WHITE));
                }
            }
            if sort_stops { if let Some(stops) = self.config.gradient_source.static_stops_mut() { stops.sort_by(|a, b| a.position.partial_cmp(&b.position).unwrap()); } }
            if response.double_clicked() {
                if let Some(pos) = response.interact_pointer_pos() {
                    let t = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                    let c = color::sample_gradient(&preview_stops, t);
                    if let Some(stops) = self.config.gradient_source.static_stops_mut() {
                        stops.push(GradientStop { color: c, position: t });
                        stops.sort_by(|a, b| a.position.partial_cmp(&b.position).unwrap());
                    }
                }
            }
            ui.label("Double-click the bar to add a color stop.");
            let mut to_remove = None;
            let current_len = self.config.gradient_source.static_stops().map(|s| s.len()).unwrap_or(0);
            if let Some(stops) = self.config.gradient_source.static_stops_mut() {
                for (i, stop) in stops.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.color_edit_button_srgb(&mut stop.color);
                        ui.label(format!("Pos: {:.2}", stop.position));
                        if current_len > 2 && ui.button("❌").clicked() { to_remove = Some(i); }
                    });
                }
            }
            if let Some(i) = to_remove { if let Some(stops) = self.config.gradient_source.static_stops_mut() { stops.remove(i); } }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.new_preset_name);
                if ui.button("💾 Save Preset").clicked() {
                    let name = if self.new_preset_name.trim().is_empty() { format!("Preset {}", self.config.gradient_presets.len() + 1) } else { self.new_preset_name.clone() };
                    let stops = self.config.gradient_source.static_stops().map(|s| s.to_vec()).unwrap_or_default();
                    self.config.gradient_presets.push(GradientPreset {
                        name, stops, angle: self.config.gradient_angle, is_premade: false,
                    });
                    self.new_preset_name.clear();
                }
            });
            egui::Frame::none()
                .fill(egui::Color32::from_rgb(30, 30, 30))
                .inner_margin(egui::Margin::symmetric(8.0, 4.0))
                .rounding(4.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Left-Click").strong().color(accent()));
                        ui.label("= Colors only  |  ");
                        ui.label(egui::RichText::new("Right-Click").strong().color(accent()));
                        ui.label("= All settings");
                    });
                });
            ui.add_space(4.0);

            let mut apply_colors: Option<usize> = None;
            let mut apply_full: Option<usize> = None;
            let mut remove_idx: Option<usize> = None;
            ui.horizontal_wrapped(|ui| {
                for (i, preset) in self.config.gradient_presets.iter().enumerate() {
                    let btn = ui.button(&preset.name);
                    if btn.clicked() { apply_colors = Some(i); }
                    if btn.secondary_clicked() { apply_full = Some(i); }
                    btn.on_hover_text("Left: colors only · Right: colors + angle");
                    if !preset.is_premade && ui.small_button("✖").clicked() { remove_idx = Some(i); }
                }
            });
            if let Some(i) = apply_colors {
                let p = &self.config.gradient_presets[i];
                self.config.gradient_source = GradientSource::Static(p.stops.clone());
            }
            if let Some(i) = apply_full {
                let p = &self.config.gradient_presets[i];
                self.config.gradient_source = GradientSource::Static(p.stops.clone());
                self.config.gradient_angle = p.angle;
            }
            if let Some(i) = remove_idx { if !self.config.gradient_presets[i].is_premade { self.config.gradient_presets.remove(i); } }
        }
    }

    fn color_sub_animated(&mut self, ui: &mut egui::Ui) {
        let is_dynamic = self.config.animated_source.is_dynamic();
        ui.horizontal(|ui| {
            if ui.selectable_label(!is_dynamic, "Static").clicked() && is_dynamic {
                let colors = self.active_palettes.animated_colors.clone();
                self.config.animated_source = ColorSource::Static(if colors.is_empty() { vec![[255, 255, 255]] } else { colors });
            }
            if ui.selectable_label(is_dynamic, "Dynamic (Album Art)").clicked() && !is_dynamic {
                self.config.animated_source = ColorSource::Dynamic { palette_size: 3, variants_per_color: 0, priority: PalettePriority::MostCommon };
            }
        });
        if is_dynamic {
            if let ColorSource::Dynamic { palette_size, variants_per_color, priority } = &mut self.config.animated_source {
                Self::draw_dynamic_palette_settings(ui, "anim", palette_size, variants_per_color, priority, true, true);
            }
            Self::draw_palette_preview(ui, &self.active_palettes.animated_colors);
        } else {
            let mut to_remove = None;
            let current_len = self.config.animated_source.static_colors().map(|c| c.len()).unwrap_or(0);
            if let Some(colors) = self.config.animated_source.static_colors_mut() {
                for (i, color) in colors.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.color_edit_button_srgb(color);
                        if current_len > 1 && ui.button("❌").clicked() { to_remove = Some(i); }
                    });
                }
            }
            if let Some(i) = to_remove { if let Some(colors) = self.config.animated_source.static_colors_mut() { colors.remove(i); } }
            if ui.button("➕ Add Color").clicked() { if let Some(colors) = self.config.animated_source.static_colors_mut() { colors.push([255, 255, 255]); } }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.new_preset_name);
                if ui.button("💾 Save Preset").clicked() {
                    let name = if self.new_preset_name.trim().is_empty() { format!("Preset {}", self.config.color_change_presets.len() + 1) } else { self.new_preset_name.clone() };
                    let colors = self.config.animated_source.static_colors().map(|c| c.to_vec()).unwrap_or_default();
                    self.config.color_change_presets.push(ColorChangePreset {
                        name, colors, speed: self.config.color_change_speed,
                        is_premade: false, motion: self.config.color_change_motion.clone(),
                    });
                    self.new_preset_name.clear();
                }
            });
            egui::Frame::none()
                .fill(egui::Color32::from_rgb(30, 30, 30))
                .inner_margin(egui::Margin::symmetric(8.0, 4.0))
                .rounding(4.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Left-Click").strong().color(accent()));
                        ui.label("= Colors only  |  ");
                        ui.label(egui::RichText::new("Right-Click").strong().color(accent()));
                        ui.label("= All settings");
                    });
                });
            ui.add_space(4.0);

            let mut apply_colors: Option<usize> = None;
            let mut apply_full: Option<usize> = None;
            let mut remove_idx: Option<usize> = None;
            ui.horizontal_wrapped(|ui| {
                for (i, preset) in self.config.color_change_presets.iter().enumerate() {
                    let btn = ui.button(&preset.name);
                    if btn.clicked() { apply_colors = Some(i); }
                    if btn.secondary_clicked() { apply_full = Some(i); }
                    btn.on_hover_text("Left: colors only · Right: colors + speed + motion");
                    if !preset.is_premade && ui.small_button("✖").clicked() { remove_idx = Some(i); }
                }
            });
            if let Some(i) = apply_colors {
                self.config.animated_source = ColorSource::Static(self.config.color_change_presets[i].colors.clone());
            }
            if let Some(i) = apply_full {
                let p = self.config.color_change_presets[i].clone();
                self.config.animated_source = ColorSource::Static(p.colors);
                self.config.color_change_speed = p.speed;
                self.config.color_change_motion = p.motion;
            }
            if let Some(i) = remove_idx { if !self.config.color_change_presets[i].is_premade { self.config.color_change_presets.remove(i); } }
        }
        slider_f32_scale(ui, &mut self.config.color_change_speed, 0.0..=1.0, "Speed", 0.5);

        section_header(ui, "Motion Shape");
        combo_reset(ui, &mut self.config.color_change_motion.shape, MotionShape::default(),
                    "Shape", "motion_shape", 180.0,
                    &MotionShape::all(), MotionShape::display_name);
        if self.config.color_change_motion.shape.uses_vertical_axis() {
            ui.horizontal(|ui| {
                ui.label("Color Style:");
                ui.radio_value(&mut self.config.vertical_color_style, VerticalColorStyle::Gradient, "Gradient");
                ui.radio_value(&mut self.config.vertical_color_style, VerticalColorStyle::Blocks, "Blocks");
            });
            help_text(ui, self.config.show_help_text, self.config.vertical_color_style.description());
        }

        section_header(ui, "Blend");
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.config.color_change_motion.blend, BlendMode::Smooth, "Smooth");
            ui.radio_value(&mut self.config.color_change_motion.blend, BlendMode::Hard, "Hard");
        });
        help_text(ui, self.config.show_help_text, "Smooth = gradients / soft fades. Hard = sharp on/off edges.");

        section_header(ui, "Motion Speed");
        slider_f32(ui, &mut self.config.color_change_motion.speed, 0.0..=2.0, "Motion Speed", 1.0);

        let shape = self.config.color_change_motion.shape;
        let hard = self.config.color_change_motion.blend == BlendMode::Hard;
        if matches!(shape, MotionShape::SineWave | MotionShape::TriangleWave | MotionShape::SquareWave | MotionShape::SawtoothWave | MotionShape::WaveInterference) {
            slider_f32(ui, &mut self.config.color_change_motion.wave_frequency, 0.5..=8.0, "Wave Frequency", 2.0);
        }
        if matches!(shape, MotionShape::Comet | MotionShape::BouncingBall | MotionShape::Sinelon) {
            slider_f32(ui, &mut self.config.color_change_motion.trail_length, 0.05..=0.9, "Trail Length", 0.25);
        }
        if matches!(shape, MotionShape::Stutter | MotionShape::Sparkle | MotionShape::Confetti) {
            slider_f32(ui, &mut self.config.color_change_motion.stutter_intensity, 0.0..=1.0, "Event Intensity", 0.3);
        }
        if matches!(shape, MotionShape::HorizontalWipe | MotionShape::VerticalWipe | MotionShape::DiagonalWipe | MotionShape::RadialWipe) && !hard {
            slider_f32(ui, &mut self.config.color_change_motion.wipe_softness, 0.01..=0.5, "Wipe Softness", 0.1);
        }
        if shape.uses_intensity() {
            color_edit_reset(ui, &mut self.config.color_change_motion.off_color, "Off Color", [0, 0, 0]);
            slider_u8(ui, &mut self.config.color_change_motion.off_color_opacity, "Off-Color Opacity", 255);
            help_text(ui, self.config.show_help_text, "Colour that 'off' pixels fade toward.");
        }
    }

    fn tab_audio(&mut self, ui: &mut egui::Ui) {
        use cfg::*;
        let show_help = self.config.show_help_text;

        section_header(ui, "Presets");
        self.draw_audio_preset_selector(ui);

        let a = &mut self.config.audio;

        section_header(ui, "Signal Processing");
        fft_size_combo(ui, a);

        combo_reset(ui, &mut a.window_function, WindowFunction::default(), "Window Function", "window_fn", 160.0, &WindowFunction::all(), WindowFunction::display_name);
        help_text(ui, show_help, a.window_function.description());

        combo_reset(ui, &mut a.frequency_scaling, FrequencyScaling::default(), "Frequency Scaling", "freq_scaling", 160.0, &[FrequencyScaling::Linear, FrequencyScaling::Logarithmic], FrequencyScaling::display_name);
        help_text(ui, show_help, a.frequency_scaling.description());

        combo_reset(ui, &mut a.channel_mix, ChannelMixMode::default(), "Channel Mix", "channel_mix", 160.0, &ChannelMixMode::all(), ChannelMixMode::display_name);
        help_text(ui, show_help, a.channel_mix.description());

        combo_reset(ui, &mut a.weighting, WeightingCurve::default(), "Weighting Curve", "weighting", 180.0, &WeightingCurve::all(), WeightingCurve::display_name);
        help_text(ui, show_help, a.weighting.description());

        checkbox_reset(ui, &mut a.dc_block, "DC Block / High-Pass Filter", DEFAULT_DC_BLOCK);
        help_text(ui, show_help, "Filters out DC offset and sub-10 Hz rumble before analysis.");

        slider_f32(ui, &mut a.frequency_smoothing, 0.0..=1.0, "Frequency Smoothing", DEFAULT_FREQ_SMOOTHING);
        help_text(ui, show_help, "Averages adjacent bars. Higher = fewer spiky, jittery bars.");

        checkbox_reset(ui, &mut a.harmonic_emphasis, "Harmonic Emphasis", DEFAULT_HARMONIC_EMPHASIS);
        help_text(ui, show_help, "Blooms local spectral peaks into wider blobs. Helps with solo piano and sparse material.");

        ui.horizontal(|ui| {
            ui.label("Min Frequency:");
            let r = ui.add(egui::DragValue::new(&mut a.min_frequency_hz).speed(10.0).clamp_range(10.0..=1000.0).suffix(" Hz"));
            if r.secondary_clicked() { a.min_frequency_hz = DEFAULT_MIN_FREQ; }
            reset_btn(ui, (a.min_frequency_hz - DEFAULT_MIN_FREQ).abs() > 1e-3, || a.min_frequency_hz = DEFAULT_MIN_FREQ);
        });
        ui.horizontal(|ui| {
            ui.label("Max Frequency:");
            let r = ui.add(egui::DragValue::new(&mut a.max_frequency_hz).speed(100.0).clamp_range(2000.0..=24000.0).suffix(" Hz"));
            if r.secondary_clicked() { a.max_frequency_hz = DEFAULT_MAX_FREQ; }
            reset_btn(ui, (a.max_frequency_hz - DEFAULT_MAX_FREQ).abs() > 1e-3, || a.max_frequency_hz = DEFAULT_MAX_FREQ);
        });

        section_header(ui, "Amplitude");
        slider_f32(ui, &mut a.sensitivity, 0.0..=100.0, "Sensitivity", DEFAULT_SENSITIVITY);
        help_text(ui, show_help, "Primary reactivity slider. Drives raw input gain and lowers the noise floor.");

        slider_f32(ui, &mut a.raw_gain, 0.25..=4.0, "Raw Gain", DEFAULT_RAW_GAIN);
        help_text(ui, show_help, "Direct pre-analysis multiplier. Normally driven by Sensitivity.");

        slider_f32(ui, &mut a.response_gamma, 0.5..=2.5, "Response Curve (γ)", DEFAULT_RESPONSE_GAMMA);
        help_text(ui, show_help, "<1 lifts quiet passages; >1 emphasises peaks. Default 1.5 = balanced.");

        slider_f32(ui, &mut a.noise_gate, 0.0..=0.5, "Noise Gate", DEFAULT_NOISE_GATE);
        help_text(ui, show_help, "Bars below this threshold are snapped to 0. Removes hovering bars during silence.");

        ui.horizontal(|ui| {
            ui.label("Min dB:");
            let r = ui.add(egui::Slider::new(&mut a.min_decibels, -120.0..=-30.0).show_value(true));
            if r.secondary_clicked() { a.min_decibels = DEFAULT_MIN_DB; }
            reset_btn(ui, (a.min_decibels - DEFAULT_MIN_DB).abs() > 1e-3, || a.min_decibels = DEFAULT_MIN_DB);
        });
        ui.horizontal(|ui| {
            ui.label("Max dB:");
            let r = ui.add(egui::Slider::new(&mut a.max_decibels, -60.0..=0.0).show_value(true));
            if r.secondary_clicked() { a.max_decibels = DEFAULT_MAX_DB; }
            reset_btn(ui, (a.max_decibels - DEFAULT_MAX_DB).abs() > 1e-3, || a.max_decibels = DEFAULT_MAX_DB);
        });
        if a.min_decibels >= a.max_decibels {
            ui.colored_label(egui::Color32::RED, "⚠ Max dB must be greater than Min dB.");
        }
        slider_f32(ui, &mut a.target_dynamic_range_db, 20.0..=90.0, "Target Dynamic Range (dB)", DEFAULT_TARGET_RANGE);
        help_text(ui, show_help, "Contrast between quiet and loud passages. Smaller = more compression/punch.");

        checkbox_reset(ui, &mut a.per_frame_normalization, "Per-Frame Peak Normalization", DEFAULT_PER_FRAME_NORMALIZATION);
        help_text(ui, show_help, "Scales each frame so the loudest bar hits the top. Makes quiet tracks visible.");

        checkbox_reset(ui, &mut a.agc_enabled, "AGC / Peak Normalisation", DEFAULT_AGC_ENABLED);
        help_text(ui, show_help, "Slowly normalises track loudness so quiet songs stay lively.");
        if a.agc_enabled {
            slider_f32(ui, &mut a.agc_attack, 0.01..=0.9, "AGC Attack", DEFAULT_AGC_ATTACK);
            slider_f32(ui, &mut a.agc_release, 0.0001..=0.05, "AGC Release", DEFAULT_AGC_RELEASE);
        }

        section_header(ui, "Frequency Shaping");
        slider_f32(ui, &mut a.eq_bass_db, -10.0..=10.0, "Bass EQ (< 250 Hz)", DEFAULT_EQ_BASS);
        slider_f32(ui, &mut a.eq_mid_db, -10.0..=10.0, "Mid EQ (250–4k Hz)", DEFAULT_EQ_MID);
        slider_f32(ui, &mut a.eq_treble_db, -10.0..=10.0, "Treble EQ (> 4k Hz)", DEFAULT_EQ_TREBLE);
        slider_f32(ui, &mut a.spectrum_tilt, -6.0..=6.0, "Spectrum Tilt (dB/oct)", DEFAULT_SPECTRUM_TILT);
        help_text(ui, show_help, "Balances bass and treble. Positive values boost highs and tame bass.");

        section_header(ui, "Timing");
        slider_f32(ui, &mut a.poll_rate_hz, 10.0..=120.0, "Poll Rate (Hz)", DEFAULT_POLL_RATE);

        ui.horizontal(|ui| {
            let mut n = a.average_size as i32;
            let r = ui.add(egui::Slider::new(&mut n, 1..=16).text("Average Size (frames)"));
            if r.secondary_clicked() { n = DEFAULT_AVERAGE_SIZE as i32; }
            a.average_size = n.max(1) as usize;
            reset_btn(ui, a.average_size != DEFAULT_AVERAGE_SIZE, || a.average_size = DEFAULT_AVERAGE_SIZE);
        });

        section_header(ui, "Ballistics");
        if a.decay_mode != self.last_decay_mode {
            let (atk, dec) = a.decay_mode.default_ballistics();
            a.attack_time_ms = atk;
            a.decay_time_ms = dec;
            self.last_decay_mode = a.decay_mode;
        }
        combo_reset(ui, &mut a.decay_mode, DecayMode::default(), "Decay Mode", "decay_mode", 160.0, &[DecayMode::Linear, DecayMode::Exponential, DecayMode::Gravity], DecayMode::display_name);
        help_text(ui, show_help, a.decay_mode.description());

        slider_f32(ui, &mut a.attack_time_ms, 0.0..=100.0, "Attack Time (ms)", DEFAULT_ATTACK_MS);
        slider_f32(ui, &mut a.decay_time_ms, 50.0..=2000.0, "Decay Time (ms)", DEFAULT_DECAY_MS);
        ui.horizontal(|ui| {
            ui.label("Peak Hold (ms):");
            let r = ui.add(egui::Slider::new(&mut a.peak_hold_ms, 0.0..=2000.0));
            if r.secondary_clicked() { a.peak_hold_ms = DEFAULT_PEAK_HOLD_MS; }
            reset_btn(ui, (a.peak_hold_ms - DEFAULT_PEAK_HOLD_MS).abs() > 1e-3, || a.peak_hold_ms = DEFAULT_PEAK_HOLD_MS);
        });
        if a.decay_mode == DecayMode::Gravity || a.peak_hold_ms > 0.0 {
            slider_f32(ui, &mut a.gravity_coefficient, 0.1..=5.0, "Gravity Coefficient", DEFAULT_GRAVITY_COEFF);
        }
        checkbox_reset(ui, &mut a.per_band_ballistics, "Per-Band Ballistics", DEFAULT_PER_BAND_BALLISTICS);
        help_text(ui, show_help, "Lets bass / mid / treble use different attack and decay times.");
        if a.per_band_ballistics {
            ui.label("Bass:");
            slider_f32(ui, &mut a.bass_attack_ms, 0.0..=100.0, "  Attack (ms)", DEFAULT_ATTACK_MS);
            slider_f32(ui, &mut a.bass_decay_ms, 50.0..=2000.0, "  Decay (ms)", DEFAULT_DECAY_MS);
            ui.label("Mid:");
            slider_f32(ui, &mut a.mid_attack_ms, 0.0..=100.0, "  Attack (ms)", DEFAULT_ATTACK_MS);
            slider_f32(ui, &mut a.mid_decay_ms, 50.0..=2000.0, "  Decay (ms)", DEFAULT_DECAY_MS);
            ui.label("Treble:");
            slider_f32(ui, &mut a.treble_attack_ms, 0.0..=100.0, "  Attack (ms)", DEFAULT_ATTACK_MS);
            slider_f32(ui, &mut a.treble_decay_ms, 50.0..=2000.0, "  Decay (ms)", DEFAULT_DECAY_MS);
        }

        section_header(ui, "Rhythm");
        combo_reset(ui, &mut a.beat_focus, BeatFocusMode::default(), "Beat Focus", "beat_focus", 160.0, &BeatFocusMode::all(), BeatFocusMode::display_name);
        help_text(ui, show_help, a.beat_focus.description());

        slider_f32(ui, &mut a.beat_detection_sensitivity, 0.0..=1.0, "Beat Detection Sensitivity", DEFAULT_BEAT_SENS);
        slider_f32(ui, &mut a.energy_threshold_multiplier, 1.0..=3.0, "Energy Threshold Multiplier", DEFAULT_ENERGY_THRESHOLD);
        slider_f32(ui, &mut a.beat_envelope_decay_ms, 20.0..=500.0, "Beat Envelope Decay (ms)", DEFAULT_BEAT_ENVELOPE_DECAY_MS);
        help_text(ui, show_help, "How quickly the beat flash fades out after a detection.");

        checkbox_reset(ui, &mut a.bpm_lock_enabled, "BPM Lock / Phase Smoothing", false);
        if a.bpm_lock_enabled {
            slider_f32(ui, &mut a.bpm_phase_smoothing, 0.0..=1.0, "Phase Smoothing", DEFAULT_BPM_PHASE_SMOOTHING);
        }

        let _ = &a;
        if self.last_bpm > 0.0 {
            ui.label(format!("Detected BPM: {:.1}", self.last_bpm));
        }
    }

    fn draw_audio_preset_selector(&mut self, ui: &mut egui::Ui) {
        let detected = AudioPreset::detect(&self.config.audio);
        let custom_match = self.config.custom_audio_presets.iter()
            .find(|p| p.settings == self.config.audio);

        let current_label = if let Some(p) = detected {
            p.display_name().to_string()
        } else if let Some(p) = custom_match {
            p.name.clone()
        } else {
            "Custom".to_string()
        };

        let mut chosen_builtin: Option<AudioPreset> = None;
        let mut chosen_custom: Option<usize> = None;

        ui.horizontal(|ui| {
            ui.label("Preset:");
            egui::ComboBox::from_id_source("audio_preset")
                .selected_text(current_label)
                .width(200.0)
                .show_ui(ui, |ui| {
                    for p in AudioPreset::all() {
                        let active = detected == Some(p);
                        if ui.selectable_label(active, p.display_name())
                            .on_hover_text(p.description())
                            .clicked()
                        {
                            chosen_builtin = Some(p);
                        }
                    }
                    if !self.config.custom_audio_presets.is_empty() {
                        ui.separator();
                        for (i, p) in self.config.custom_audio_presets.iter().enumerate() {
                            let active = custom_match.map(|c| c.name == p.name).unwrap_or(false);
                            if ui.selectable_label(active, &p.name).clicked() {
                                chosen_custom = Some(i);
                            }
                        }
                    }
                });
        });
        if let Some(p) = chosen_builtin {
            p.apply(&mut self.config.audio);
            self.last_decay_mode = self.config.audio.decay_mode;
        }
        if let Some(i) = chosen_custom {
            self.config.audio = self.config.custom_audio_presets[i].settings.clone();
            self.last_decay_mode = self.config.audio.decay_mode;
        }
        if let Some(p) = detected {
            help_text(ui, self.config.show_help_text, p.description());
        }

        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.new_audio_preset_name);
            if ui.button("💾 Save as Preset").clicked() {
                let name = if self.new_audio_preset_name.trim().is_empty() {
                    format!("My Preset {}", self.config.custom_audio_presets.len() + 1)
                } else {
                    self.new_audio_preset_name.trim().to_string()
                };
                self.config.custom_audio_presets.push(CustomAudioPreset {
                    name,
                    settings: self.config.audio.clone(),
                });
                self.new_audio_preset_name.clear();
            }
        });

        let mut to_remove = None;
        let custom_list: Vec<(usize, String)> = self.config.custom_audio_presets.iter()
            .enumerate().map(|(i, p)| (i, p.name.clone())).collect();
        ui.horizontal_wrapped(|ui| {
            for (i, name) in &custom_list {
                if ui.button(name).clicked() {
                    self.config.audio = self.config.custom_audio_presets[*i].settings.clone();
                    self.last_decay_mode = self.config.audio.decay_mode;
                }
                if ui.small_button("✖").clicked() { to_remove = Some(*i); }
            }
        });
        if let Some(i) = to_remove { self.pending_audio_preset_delete = Some(i); }
    }

    fn tab_track_info(&mut self, ui: &mut egui::Ui) {
        use cfg::*;
        let system_fonts = self.system_fonts.clone();
        let cfg = &mut self.config.track_info;

        section_header(ui, "Cover Art");
        checkbox_reset(ui, &mut cfg.show_cover_art, "Show Cover Art", true);
        if cfg.show_cover_art {
            ui.horizontal(|ui| {
                let r = ui.add(egui::Slider::new(&mut cfg.cover_size, 40.0..=600.0).text("Size (px)"));
                if r.secondary_clicked() { cfg.cover_size = DEFAULT_COVER_SIZE; }
                reset_btn(ui, (cfg.cover_size - DEFAULT_COVER_SIZE).abs() > 1e-3, || cfg.cover_size = DEFAULT_COVER_SIZE);
            });
            ui.horizontal(|ui| {
                ui.label("X Offset:");
                let r = ui.add(egui::DragValue::new(&mut cfg.cover_x_offset).speed(1.0).max_decimals(0).clamp_range(-5000.0..=200.0).suffix(" px"));
                if r.secondary_clicked() { cfg.cover_x_offset = 0.0; }
                reset_btn(ui, (cfg.cover_x_offset).abs() > 1e-3, || cfg.cover_x_offset = 0.0);
            });
            ui.horizontal(|ui| {
                ui.label("Y Offset:");
                let r = ui.add(egui::DragValue::new(&mut cfg.cover_y_offset).speed(1.0).max_decimals(0).clamp_range(-200.0..=5000.0).suffix(" px"));
                if r.secondary_clicked() { cfg.cover_y_offset = 0.0; }
                reset_btn(ui, (cfg.cover_y_offset).abs() > 1e-3, || cfg.cover_y_offset = 0.0);
            });
            slider_u8(ui, &mut cfg.cover_opacity, "Cover Art Opacity", DEFAULT_COVER_OPACITY);
        }

        section_header(ui, "Song Info Text");
        ui.horizontal(|ui| {
            checkbox_reset(ui, &mut cfg.show_title, "Show Title", true);
            checkbox_reset(ui, &mut cfg.show_artist, "Show Artist", true);
        });
        ui.horizontal(|ui| {
            let r = ui.add(egui::Slider::new(&mut cfg.title_font_size, 6.0..=120.0).text("Title Font Size"));
            if r.secondary_clicked() { cfg.title_font_size = DEFAULT_TITLE_FONT_SIZE; }
            reset_btn(ui, (cfg.title_font_size - DEFAULT_TITLE_FONT_SIZE).abs() > 1e-3, || cfg.title_font_size = DEFAULT_TITLE_FONT_SIZE);
        });
        ui.horizontal(|ui| {
            let r = ui.add(egui::Slider::new(&mut cfg.artist_font_size, 6.0..=120.0).text("Artist Font Size"));
            if r.secondary_clicked() { cfg.artist_font_size = DEFAULT_ARTIST_FONT_SIZE; }
            reset_btn(ui, (cfg.artist_font_size - DEFAULT_ARTIST_FONT_SIZE).abs() > 1e-3, || cfg.artist_font_size = DEFAULT_ARTIST_FONT_SIZE);
        });
        ui.horizontal(|ui| {
            ui.label("Font:");
            let selected_label = match &cfg.font_path {
                None => "Default (proportional)".to_string(),
                Some(p) => std::path::Path::new(p).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| p.clone()),
            };
            egui::ComboBox::from_id_source("track_font_select")
                .selected_text(selected_label)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(cfg.font_path.is_none(), "Default (proportional)").clicked() {
                        cfg.font_path = None;
                    }
                    for (name, path) in &system_fonts {
                        let selected = cfg.font_path.as_deref() == Some(path.as_str());
                        if ui.selectable_label(selected, name).clicked() { cfg.font_path = Some(path.clone()); }
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("Custom path:");
            let mut path_str = cfg.font_path.clone().unwrap_or_default();
            if ui.text_edit_singleline(&mut path_str).changed() {
                cfg.font_path = if path_str.trim().is_empty() { None } else { Some(path_str) };
            }
        });

        section_header(ui, "Text Color");
        ui.horizontal(|ui| {
            ui.radio_value(&mut cfg.color_mode, TrackTextColorMode::Static, "Static");
            ui.radio_value(&mut cfg.color_mode, TrackTextColorMode::Visualizer, "Visualizer Color");
        });
        if cfg.color_mode == TrackTextColorMode::Static {
            color_edit_reset(ui, &mut cfg.static_color, "Color", [220, 220, 220]);
        }
        slider_u8(ui, &mut cfg.opacity, "Text Opacity", 255);

        section_header(ui, "Text Layout");
        ui.horizontal(|ui| {
            ui.label("Alignment:");
            ui.radio_value(&mut cfg.alignment, TrackTextAlign::Left, "Left");
            ui.radio_value(&mut cfg.alignment, TrackTextAlign::Center, "Center");
            ui.radio_value(&mut cfg.alignment, TrackTextAlign::Right, "Right");
        });
        ui.horizontal(|ui| {
            let r = ui.add(egui::Slider::new(&mut cfg.letter_spacing, -5.0..=20.0).text("Letter Spacing (px)"));
            if r.secondary_clicked() { cfg.letter_spacing = 0.0; }
            reset_btn(ui, cfg.letter_spacing.abs() > 1e-3, || cfg.letter_spacing = 0.0);
        });
        ui.horizontal(|ui| {
            let r = ui.add(egui::Slider::new(&mut cfg.line_height, 0.5..=3.0).text("Line Height"));
            if r.secondary_clicked() { cfg.line_height = 1.3; }
            reset_btn(ui, (cfg.line_height - 1.3).abs() > 1e-3, || cfg.line_height = 1.3);
        });
        ui.horizontal(|ui| {
            ui.label("Text X Offset:");
            let r = ui.add(egui::DragValue::new(&mut cfg.text_x_offset).speed(1.0).max_decimals(0).clamp_range(-5000.0..=200.0).suffix(" px"));
            if r.secondary_clicked() { cfg.text_x_offset = 0.0; }
            reset_btn(ui, cfg.text_x_offset.abs() > 1e-3, || cfg.text_x_offset = 0.0);
        });
        ui.horizontal(|ui| {
            ui.label("Text Y Offset:");
            let r = ui.add(egui::DragValue::new(&mut cfg.text_y_offset).speed(1.0).max_decimals(0).clamp_range(-200.0..=5000.0).suffix(" px"));
            if r.secondary_clicked() { cfg.text_y_offset = 0.0; }
            reset_btn(ui, cfg.text_y_offset.abs() > 1e-3, || cfg.text_y_offset = 0.0);
        });

        section_header(ui, "Shadow");
        checkbox_reset(ui, &mut cfg.shadow_enabled, "Enable Shadow", true);
        if cfg.shadow_enabled {
            slider_f32(ui, &mut cfg.shadow_offset_x, -30.0..=30.0, "Shadow X Offset", 2.0);
            slider_f32(ui, &mut cfg.shadow_offset_y, -30.0..=30.0, "Shadow Y Offset", 2.0);
            slider_f32(ui, &mut cfg.shadow_blur, 0.0..=10.0, "Shadow Blur", 0.0);
            color_edit_reset(ui, &mut cfg.shadow_color, "Shadow Color", [0, 0, 0]);
        }

        section_header(ui, "Outline");
        checkbox_reset(ui, &mut cfg.outline_enabled, "Enable Outline", false);
        if cfg.outline_enabled {
            slider_f32(ui, &mut cfg.outline_thickness, 0.1..=6.0, "Outline Thickness", 1.0);
            color_edit_reset(ui, &mut cfg.outline_color, "Outline Color", [0, 0, 0]);
        }
    }

    fn tab_appearance(&mut self, ui: &mut egui::Ui) {
        use cfg::*;
        let show_help = self.config.show_help_text;

        section_header(ui, "Background");
        color_edit_reset(ui, &mut self.config.bg_color, "Background Color", [0, 0, 0]);
        slider_u8(ui, &mut self.config.bg_opacity, "Background Opacity", DEFAULT_BG_OPACITY);
        help_text(ui, show_help, "0 = fully transparent overlay.");

        section_header(ui, "Menu");
        ui.horizontal(|ui| {
            ui.label("Menu Scale:");
            let mut current_scale = self.config.settings_scale;
            egui::ComboBox::from_id_source("menu_scale")
                .selected_text(format!("{:.0}%", current_scale * 100.0))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut current_scale, 0.75, "75%");
                    ui.selectable_value(&mut current_scale, 1.00, "100%");
                    ui.selectable_value(&mut current_scale, 1.25, "125%");
                    ui.selectable_value(&mut current_scale, 1.50, "150%");
                });
            if current_scale != self.config.settings_scale {
                self.config.settings_scale = current_scale;
            }
            reset_btn(ui, (self.config.settings_scale - 1.0).abs() > 1e-3, || self.config.settings_scale = 1.0);
        });
        checkbox_reset(ui, &mut self.config.open_settings_on_launch, "Open Settings on Launch", DEFAULT_OPEN_SETTINGS_ON_LAUNCH);
        checkbox_reset(ui, &mut self.config.show_help_text, "Show Inline Help Text", DEFAULT_SHOW_HELP_TEXT);
    }

    fn tab_performance(&mut self, ui: &mut egui::Ui) {
        use cfg::*;
        section_header(ui, "Frame Rate");

        ui.label(
            egui::RichText::new(format!(
                "Active mode: {}",
                if self.config.use_vsync { "VSync ON (fixed 60 Hz cap)" } else { "VSync OFF (timer cap)" }
            ))
            .size(12.0)
            .color(egui::Color32::from_gray(150)),
        );
        ui.add_space(4.0);

        let mut new_vsync = self.config.use_vsync;
        checkbox_reset(ui, &mut new_vsync, "Enable VSync (Requires Restart)", true);
        if new_vsync != self.config.use_vsync {
            self.config.use_vsync = new_vsync;
        }
        if new_vsync != self.config_initial_vsync() {
            ui.colored_label(
                egui::Color32::YELLOW,
                "↺ Restart the app for VSync to apply.",
            );
        }
        help_text(
            ui,
            self.config.show_help_text,
            "VSync ON is strongly recommended: it fixes the FPS spikes you get when the mouse \
             is moving and removes tearing. Timer caps cannot fully prevent those spikes.",
        );

        slider_f32(ui, &mut self.config.target_fps, 15.0..=144.0, "Target FPS", DEFAULT_TARGET_FPS);
        checkbox_reset(ui, &mut self.config.show_fps, "Show FPS Counter", false);
        ui.label(format!("Current FPS: {:.1}", self.current_fps));

        ui.add_space(8.0);
        section_header(ui, "Power");
        ui.label(
            egui::RichText::new(
                "When no audio is playing the visualizer automatically drops to 4 Hz \
                 to keep idle GPU/CPU usage near zero. Playback resumes at full speed \
                 within 250 ms of the first audio frame."
            )
            .size(12.0)
            .color(egui::Color32::from_gray(150)),
        );
    }

    fn config_initial_vsync(&self) -> bool {
        self.config.use_vsync
    }

    fn draw_dynamic_palette_settings(ui: &mut egui::Ui, id_suffix: &str, palette_size: &mut u8, variants: &mut u8, priority: &mut PalettePriority, show_size: bool, show_variants: bool) {
        ui.push_id(id_suffix, |ui| {
            ui.horizontal(|ui| {
                ui.label("Priority:");
                egui::ComboBox::from_id_source("priority")
                    .selected_text(priority.display_name())
                    .show_ui(ui, |ui| {
                        for p in PalettePriority::all() {
                            ui.selectable_value(priority, p, p.display_name());
                        }
                    });
            });
            if show_size {
                let mut v = *palette_size as i32;
                let r = ui.add(egui::Slider::new(&mut v, 1..=3).text("Colors from Cover"));
                if r.secondary_clicked() { v = 1; }
                *palette_size = v.clamp(1, 3) as u8;
            }
            if show_variants {
                let mut v = *variants as i32;
                let r = ui.add(egui::Slider::new(&mut v, 0..=5).text("Shade Variants per Color"));
                if r.secondary_clicked() { v = 0; }
                *variants = v.clamp(0, 5) as u8;
            }
        });
    }

    fn draw_palette_preview(ui: &mut egui::Ui, colors: &[[u8; 3]]) {
        ui.label("Current palette:");
        ui.horizontal_wrapped(|ui| {
            for c in colors {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 4.0, egui::Color32::from_rgb(c[0], c[1], c[2]));
            }
        });
    }
}

fn fft_size_combo(ui: &mut egui::Ui, a: &mut visualizer_core::config::AudioSettings) {
    ui.horizontal(|ui| {
        ui.label("FFT Size:");
        egui::ComboBox::from_id_source("fft_size")
            .selected_text(format!("{}", a.fft_size))
            .show_ui(ui, |ui| {
                for size in [512usize, 1024, 2048, 4096, 8192] {
                    ui.selectable_value(&mut a.fft_size, size, format!("{size}"));
                }
            });
        reset_btn(ui, a.fft_size != 2048, || a.fft_size = 2048);
    });
}

impl eframe::App for VisualizerApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] { egui::Rgba::TRANSPARENT.to_array() }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame_time).as_secs_f32();
        self.last_frame_time = now;
        if dt > 0.0 {
            let instant_fps = 1.0 / dt;
            self.current_fps = self.current_fps * 0.9 + instant_fps * 0.1;
        }
        if ctx.pixels_per_point() != 1.0 { ctx.set_pixels_per_point(1.0); }

        self.poll_track_updates(ctx);

        let new_hash = self.palette_fingerprint();
        if new_hash != self.last_palette_hash {
            self.update_palettes();
            self.last_palette_hash = new_hash;
        }

        if self.config.track_info.font_path != self.loaded_font_path {
            self.reload_custom_font(ctx);
        }

        if self.config.audio != *self.audio_settings.load_full() {
            self.audio_settings.store(Arc::new(self.config.audio.clone()));
        }

        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == self.settings_menu_id { self.show_settings = true; }
            else if event.id == self.quit_menu_id { self.should_quit.store(true, Ordering::Relaxed); }
        }
        while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            if event.state == global_hotkey::HotKeyState::Pressed {
                self.show_settings = !self.show_settings;
                if !self.show_settings {
                    ctx.send_viewport_cmd_to(egui::ViewportId::from_hash_of("settings"), egui::ViewportCommand::Close);
                }
            }
        }
        if self.should_quit.load(Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        let n_bars = self.config.audio.num_bars;
        if self.current_bars.len() != n_bars {
            self.current_bars.resize(n_bars, 0.0);
            self.current_peaks.resize(n_bars, 0.0);
        }

        let frame = self.shared_frame.load();
        let limit = self.current_bars.len().min(frame.values.len());
        for i in 0..limit {
            self.current_bars[i] = frame.values[i];
            self.current_peaks[i] = frame.peaks[i];
        }
        self.last_bpm = frame.bpm;
        if frame.values.iter().cloned().fold(0.0f32, f32::max) > 0.001 {
            self.last_audio_frame_time = now;
        }
        drop(frame);

        if !self.is_passthrough_set {
            ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(true));
            self.is_passthrough_set = true;
        }

        let rect = ctx.screen_rect();
        egui::Area::new(egui::Id::new("overlay")).fixed_pos(rect.min).show(ctx, |ui| {
            ui.set_min_size(rect.size());
            if self.config.bg_opacity > 0 {
                let c = self.config.bg_color;
                let bg_color = egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], self.config.bg_opacity);
                ui.painter().rect_filled(rect, 0.0, bg_color);
            }
            self.draw_bars_gpu(ui, rect);
            self.draw_track_info(ui, rect);
            if self.config.show_fps {
                ui.painter().text(rect.left_top() + egui::vec2(20.0, 20.0),
                                  egui::Align2::LEFT_TOP,
                                  format!("FPS: {:.1}", self.current_fps),
                                  egui::FontId::monospace(16.0),
                                  egui::Color32::GREEN);
            }
        });

        if self.show_settings {
            let scale = self.config.settings_scale.clamp(0.75, 1.5);

            if (self.config.settings_scale - self.old_settings_scale).abs() > 0.01 {
                self.old_settings_scale = self.config.settings_scale;
                ctx.send_viewport_cmd_to(
                    egui::ViewportId::from_hash_of("settings"),
                    egui::ViewportCommand::InnerSize(egui::vec2(560.0 * scale, 720.0 * scale))
                );
            }

            let mut settings_builder = egui::ViewportBuilder::default()
                .with_title("Rust Visualizer — Settings")
                .with_inner_size([560.0 * scale, 720.0 * scale])
                .with_decorations(true)
                .with_resizable(true)
                .with_always_on_top();

            if self.settings_icon.width > 0 {
                settings_builder = settings_builder.with_icon(self.settings_icon.clone());
            }

            let mut close_window = false;
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("settings"),
                settings_builder,
                |ctx, _class| {
                    if ctx.input(|i| i.viewport().close_requested()) { close_window = true; }
                    egui::CentralPanel::default()
                        .frame(egui::Frame::none().fill(egui::Color32::BLACK))
                        .show(ctx, |ui| {
                            self.draw_settings_ui(ui);
                        });

                    if let Some(idx) = self.pending_audio_preset_delete {
                        egui::Window::new("Confirm Delete")
                            .collapsible(false)
                            .resizable(false)
                            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                            .show(ctx, |ui| {
                                ui.label(format!("Delete preset '{}'?", self.config.custom_audio_presets[idx].name));
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    if ui.button("Yes").clicked() {
                                        self.config.custom_audio_presets.remove(idx);
                                        self.pending_audio_preset_delete = None;
                                    }
                                    if ui.button("No").clicked() {
                                        self.pending_audio_preset_delete = None;
                                    }
                                });
                            });
                    }
                });
            if close_window {
                self.show_settings = false;
                self.config.save();
                ctx.send_viewport_cmd_to(egui::ViewportId::from_hash_of("settings"), egui::ViewportCommand::Close);
            }
        }

        let audio_active = now.duration_since(self.last_audio_frame_time) < Duration::from_millis(500);
        let is_animated  = self.config.color_mode == ColorMode::Animated;

        if audio_active || is_animated {
            if self.config.use_vsync {
                ctx.request_repaint();
            } else {
                let target_dt = Duration::from_secs_f32(1.0 / self.config.target_fps.max(1.0));
                ctx.request_repaint_after(target_dt);
            }
        } else {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    fn on_exit(&mut self, _gl: Option<&glow::Context>) {
        self.config.save();
        #[cfg(windows)]
        unsafe { timeEndPeriod(1); }
    }
}

fn main() -> eframe::Result<()> {
    #[cfg(windows)]
    unsafe { timeBeginPeriod(1); }

    let config = AppConfig::load();
    let (egui_icon, _) = load_icon();

    let mut wgpu_options = eframe::egui_wgpu::WgpuConfiguration::default();
    wgpu_options.supported_backends = wgpu::Backends::DX12;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_transparent(true)
            .with_decorations(false)
            .with_always_on_top()
            .with_maximized(true)
            .with_taskbar(false)
            .with_icon(egui_icon),
        wgpu_options,
        vsync: config.use_vsync,
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "Rust Visualizer",
        options,
        Box::new(|cc| Box::new(VisualizerApp::new(cc))),
    )
}
