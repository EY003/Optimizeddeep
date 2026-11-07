use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ─────────────────────────────────────────────────────────────
// Existing enums (unchanged)
// ─────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum VisualizerDirection { Up, Down, Left, Right }
impl Default for VisualizerDirection { fn default() -> Self { VisualizerDirection::Up } }

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum FrequencyDirection { LeftToRight, RightToLeft, Mirrored }
impl Default for FrequencyDirection { fn default() -> Self { FrequencyDirection::LeftToRight } }

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorMode {
    Dynamic, StaticSingle, StaticMultiple, Gradient,
    #[serde(alias = "ColorChanging")] Animated,
}
impl Default for ColorMode { fn default() -> Self { ColorMode::StaticSingle } }

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum StaticDistributionMode { Blocks, Sequential, Random, Vertical }
impl Default for StaticDistributionMode { fn default() -> Self { StaticDistributionMode::Blocks } }

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum VerticalColorStyle { Gradient, Blocks }
impl Default for VerticalColorStyle { fn default() -> Self { VerticalColorStyle::Gradient } }
impl VerticalColorStyle {
    pub fn display_name(self) -> &'static str {
        match self {
            VerticalColorStyle::Gradient => "Gradient (Smooth Blend)",
            VerticalColorStyle::Blocks => "Blocks (Distinct Sections)",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            VerticalColorStyle::Gradient => "Smoothly blends all colors vertically across the bar.",
            VerticalColorStyle::Blocks => "Divides the bar into solid, distinct color sections (classic style).",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct GradientStop { pub color: [u8; 3], pub position: f32 }

#[derive(Serialize, Deserialize, Clone)]
pub struct StaticMultiPreset {
    pub name: String, pub colors: Vec<[u8; 3]>,
    pub mode: StaticDistributionMode,
    #[serde(default = "default_sequential_bars_per_color")] pub bars_per_color: usize,
    #[serde(default)] pub is_premade: bool,
}
#[derive(Serialize, Deserialize, Clone)]
pub struct GradientPreset {
    pub name: String, pub stops: Vec<GradientStop>, pub angle: f32,
    #[serde(default)] pub is_premade: bool,
}
#[derive(Serialize, Deserialize, Clone)]
pub struct ColorChangePreset {
    pub name: String, pub colors: Vec<[u8; 3]>, pub speed: f32,
    #[serde(default)] pub is_premade: bool,
    #[serde(default)] pub motion: MotionSettings,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum PalettePriority { MostCommon, MostCommonLight, Vivid }
impl Default for PalettePriority { fn default() -> Self { PalettePriority::MostCommon } }
impl PalettePriority {
    pub fn display_name(self) -> &'static str {
        match self {
            PalettePriority::MostCommon => "Most Common",
            PalettePriority::MostCommonLight => "Most Common Light",
            PalettePriority::Vivid => "Vivid / High Contrast",
        }
    }
    pub fn all() -> [PalettePriority; 3] {
        [PalettePriority::MostCommon, PalettePriority::MostCommonLight, PalettePriority::Vivid]
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub enum ColorSource {
    Static(Vec<[u8; 3]>),
    Dynamic {
        #[serde(default = "default_palette_size")] palette_size: u8,
        #[serde(default)] variants_per_color: u8,
        #[serde(default)] priority: PalettePriority,
    },
}
impl Default for ColorSource { fn default() -> Self { ColorSource::Static(default_static_colors()) } }
impl ColorSource {
    pub fn is_dynamic(&self) -> bool { matches!(self, ColorSource::Dynamic { .. }) }
    pub fn static_colors(&self) -> Option<&[[u8; 3]]> {
        match self { ColorSource::Static(v) => Some(v.as_slice()), _ => None }
    }
    pub fn static_colors_mut(&mut self) -> Option<&mut Vec<[u8; 3]>> {
        match self { ColorSource::Static(v) => Some(v), _ => None }
    }
    pub fn dynamic_settings(&self) -> Option<(u8, u8, PalettePriority)> {
        match self {
            ColorSource::Dynamic { palette_size, variants_per_color, priority } =>
                Some((*palette_size, *variants_per_color, *priority)),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub enum GradientSource {
    Static(Vec<GradientStop>),
    Dynamic {
        #[serde(default = "default_palette_size")] palette_size: u8,
        #[serde(default)] variants_per_color: u8,
        #[serde(default)] priority: PalettePriority,
    },
}
impl Default for GradientSource { fn default() -> Self { GradientSource::Static(default_gradient_stops()) } }
impl GradientSource {
    pub fn is_dynamic(&self) -> bool { matches!(self, GradientSource::Dynamic { .. }) }
    pub fn static_stops(&self) -> Option<&[GradientStop]> {
        match self { GradientSource::Static(s) => Some(s.as_slice()), _ => None }
    }
    pub fn static_stops_mut(&mut self) -> Option<&mut Vec<GradientStop>> {
        match self { GradientSource::Static(s) => Some(s), _ => None }
    }
    pub fn dynamic_settings(&self) -> Option<(u8, u8, PalettePriority)> {
        match self {
            GradientSource::Dynamic { palette_size, variants_per_color, priority } =>
                Some((*palette_size, *variants_per_color, *priority)),
            _ => None,
        }
    }
}
fn default_palette_size() -> u8 { 1 }

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum MotionShape {
    Uniform, HorizontalWipe, VerticalWipe, DiagonalWipe,
    RadialWipe, Spiral, WaveInterference, Ripple,
    SineWave, TriangleWave, SquareWave, SawtoothWave,
    Strobe, Pulse, Heartbeat, Stutter, Comet, Chase, BouncingBall, Sparkle,
    FireSimulation, Plasma, RainbowCycle, Confetti, Sinelon,
}
impl Default for MotionShape { fn default() -> Self { MotionShape::Uniform } }
impl MotionShape {
    pub fn display_name(self) -> &'static str {
        match self {
            MotionShape::Uniform => "Uniform",
            MotionShape::HorizontalWipe => "Horizontal Wipe",
            MotionShape::VerticalWipe => "Vertical Wipe",
            MotionShape::DiagonalWipe => "Diagonal Wipe",
            MotionShape::RadialWipe => "Radial Wipe",
            MotionShape::Spiral => "Spiral",
            MotionShape::WaveInterference => "Wave Interference",
            MotionShape::Ripple => "Ripple",
            MotionShape::SineWave => "Sine Wave",
            MotionShape::TriangleWave => "Triangle Wave",
            MotionShape::SquareWave => "Square Wave",
            MotionShape::SawtoothWave => "Sawtooth Wave",
            MotionShape::Strobe => "Strobe",
            MotionShape::Pulse => "Pulse",
            MotionShape::Heartbeat => "Heartbeat",
            MotionShape::Stutter => "Stutter",
            MotionShape::Comet => "Comet",
            MotionShape::Chase => "Chase",
            MotionShape::BouncingBall => "Bouncing Ball",
            MotionShape::Sparkle => "Sparkle",
            MotionShape::FireSimulation => "Fire Simulation",
            MotionShape::Plasma => "Plasma",
            MotionShape::RainbowCycle => "Rainbow Cycle",
            MotionShape::Confetti => "Confetti",
            MotionShape::Sinelon => "Sinelon",
        }
    }
    pub fn uses_vertical_axis(self) -> bool {
        matches!(self,
            MotionShape::VerticalWipe | MotionShape::DiagonalWipe
            | MotionShape::RadialWipe | MotionShape::Spiral
            | MotionShape::WaveInterference | MotionShape::Ripple
            | MotionShape::Plasma
        )
    }
    pub fn uses_intensity(self) -> bool {
        matches!(
            self,
            MotionShape::Strobe | MotionShape::Pulse | MotionShape::Heartbeat
            | MotionShape::Comet | MotionShape::Chase | MotionShape::BouncingBall
            | MotionShape::Sparkle | MotionShape::Sinelon
            | MotionShape::FireSimulation | MotionShape::Confetti
        )
    }
    pub fn all() -> [MotionShape; 25] {
        [
            MotionShape::Uniform, MotionShape::HorizontalWipe, MotionShape::VerticalWipe,
            MotionShape::DiagonalWipe, MotionShape::RadialWipe, MotionShape::Spiral,
            MotionShape::WaveInterference, MotionShape::Ripple,
            MotionShape::SineWave, MotionShape::TriangleWave, MotionShape::SquareWave,
            MotionShape::SawtoothWave, MotionShape::Strobe, MotionShape::Pulse,
            MotionShape::Heartbeat, MotionShape::Stutter, MotionShape::Comet,
            MotionShape::Chase, MotionShape::BouncingBall, MotionShape::Sparkle,
            MotionShape::FireSimulation, MotionShape::Plasma, MotionShape::RainbowCycle,
            MotionShape::Confetti, MotionShape::Sinelon,
        ]
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlendMode { Smooth, Hard }
impl Default for BlendMode { fn default() -> Self { BlendMode::Smooth } }

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MotionSettings {
    pub shape: MotionShape, pub blend: BlendMode, pub speed: f32,
    pub trail_length: f32, pub stutter_intensity: f32,
    pub wave_frequency: f32, pub wipe_softness: f32,
    #[serde(default)] pub off_color: [u8; 3],
    #[serde(default = "default_off_color_opacity")] pub off_color_opacity: u8,
}
fn default_off_color_opacity() -> u8 { 255 }
impl Default for MotionSettings {
    fn default() -> Self {
        Self {
            shape: MotionShape::Uniform, blend: BlendMode::Smooth, speed: 1.0,
            trail_length: 0.25, stutter_intensity: 0.3,
            wave_frequency: 2.0, wipe_softness: 0.1,
            off_color: [0, 0, 0], off_color_opacity: 255,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrackTextColorMode { Static, Visualizer }
impl Default for TrackTextColorMode { fn default() -> Self { TrackTextColorMode::Static } }

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrackTextAlign { Left, Center, Right }
impl Default for TrackTextAlign { fn default() -> Self { TrackTextAlign::Right } }

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TrackInfoConfig {
    pub show_cover_art: bool, pub cover_size: f32,
    pub cover_x_offset: f32, pub cover_y_offset: f32,
    pub show_title: bool, pub show_artist: bool,
    pub title_font_size: f32, pub artist_font_size: f32,
    #[serde(default, alias = "custom_font_path")] pub font_path: Option<String>,
    pub color_mode: TrackTextColorMode, pub static_color: [u8; 3], pub opacity: u8,
    pub letter_spacing: f32, pub line_height: f32, pub alignment: TrackTextAlign,
    pub text_x_offset: f32, pub text_y_offset: f32,
    pub shadow_enabled: bool, pub shadow_offset_x: f32, pub shadow_offset_y: f32,
    pub shadow_blur: f32, pub shadow_color: [u8; 3],
    pub outline_enabled: bool, pub outline_thickness: f32, pub outline_color: [u8; 3],
}
impl Default for TrackInfoConfig {
    fn default() -> Self {
        Self {
            show_cover_art: true, cover_size: 190.0, cover_x_offset: 0.0, cover_y_offset: 0.0,
            show_title: true, show_artist: true, title_font_size: 21.0, artist_font_size: 30.0,
            font_path: None, color_mode: TrackTextColorMode::Static,
            static_color: [220, 220, 220], opacity: 255,
            letter_spacing: 0.0, line_height: 1.3, alignment: TrackTextAlign::Right,
            text_x_offset: 0.0, text_y_offset: 0.0,
            shadow_enabled: true, shadow_offset_x: 2.0, shadow_offset_y: 2.0,
            shadow_blur: 0.0, shadow_color: [0, 0, 0],
            outline_enabled: false, outline_thickness: 1.0, outline_color: [0, 0, 0],
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Audio signal-processing enums
// ─────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum WindowFunction { Hann, Hamming, BlackmanHarris, FlatTop, Rectangular }
impl Default for WindowFunction { fn default() -> Self { WindowFunction::Hann } }
impl WindowFunction {
    pub fn display_name(self) -> &'static str {
        match self {
            WindowFunction::Hann => "Hann",
            WindowFunction::Hamming => "Hamming",
            WindowFunction::BlackmanHarris => "Blackman-Harris",
            WindowFunction::FlatTop => "Flat Top",
            WindowFunction::Rectangular => "Rectangular",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            WindowFunction::Hann => "Balanced default. Good amplitude accuracy with low spectral leakage.",
            WindowFunction::Hamming => "Slightly narrower main lobe than Hann; less leakage suppression.",
            WindowFunction::BlackmanHarris => "Strongest side-lobe suppression. Tones look cleanest, at the cost of a wider main lobe.",
            WindowFunction::FlatTop => "Best amplitude accuracy. Peaks read true, but neighbouring bins blur more.",
            WindowFunction::Rectangular => "No tapering. Sharpest time resolution, but leaks badly on non-integer-period tones.",
        }
    }
    pub fn all() -> [WindowFunction; 5] {
        [WindowFunction::Hann, WindowFunction::Hamming, WindowFunction::BlackmanHarris, WindowFunction::FlatTop, WindowFunction::Rectangular]
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum FrequencyScaling { Linear, Logarithmic }
impl Default for FrequencyScaling { fn default() -> Self { FrequencyScaling::Logarithmic } }
impl FrequencyScaling {
    pub fn display_name(self) -> &'static str {
        match self {
            FrequencyScaling::Linear => "Linear",
            FrequencyScaling::Logarithmic => "Logarithmic",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            FrequencyScaling::Linear => "Even Hz spacing. Favours treble; bass compresses into a few bars.",
            FrequencyScaling::Logarithmic => "Matches human hearing. Gives bass and mids more visual space.",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum DecayMode { Linear, Exponential, Gravity }
impl Default for DecayMode { fn default() -> Self { DecayMode::Exponential } }
impl DecayMode {
    pub fn display_name(self) -> &'static str {
        match self {
            DecayMode::Linear => "Linear",
            DecayMode::Exponential => "Exponential",
            DecayMode::Gravity => "Gravity",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            DecayMode::Linear => "Steady fall at a constant rate. Predictable, slightly mechanical.",
            DecayMode::Exponential => "Fast initial drop that slows near the bottom. Natural-looking.",
            DecayMode::Gravity => "Accelerates downward like a falling object. Punchy, physical.",
        }
    }
    pub fn default_ballistics(self) -> (f32, f32) {
        match self {
            DecayMode::Linear => (20.0, 400.0),
            DecayMode::Exponential => (5.0, 300.0),
            DecayMode::Gravity => (0.0, 500.0),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChannelMixMode { Mono, Left, Right, Mid, Side }
impl Default for ChannelMixMode { fn default() -> Self { ChannelMixMode::Mono } }
impl ChannelMixMode {
    pub fn display_name(self) -> &'static str {
        match self {
            ChannelMixMode::Mono => "Mono (L+R)",
            ChannelMixMode::Left => "Left Channel",
            ChannelMixMode::Right => "Right Channel",
            ChannelMixMode::Mid => "Mid (Centered)",
            ChannelMixMode::Side => "Side (Stereo Width)",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            ChannelMixMode::Mono => "Standard L+R mix. Best for balanced visualizers.",
            ChannelMixMode::Left => "Only uses the left channel. Useful for checking stereo panning.",
            ChannelMixMode::Right => "Only uses the right channel.",
            ChannelMixMode::Mid => "Extracts the center (L+R). Highlights vocals and kick drums.",
            ChannelMixMode::Side => "Extracts the stereo width (L-R). Highlights reverbs and wide synths.",
        }
    }
    pub fn all() -> [ChannelMixMode; 5] {
        [ChannelMixMode::Mono, ChannelMixMode::Left, ChannelMixMode::Right, ChannelMixMode::Mid, ChannelMixMode::Side]
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum BeatFocusMode { Bass, Mid, Treble }
impl Default for BeatFocusMode { fn default() -> Self { BeatFocusMode::Bass } }
impl BeatFocusMode {
    pub fn display_name(self) -> &'static str {
        match self {
            BeatFocusMode::Bass => "Bass (Kick)",
            BeatFocusMode::Mid => "Mid (Snare)",
            BeatFocusMode::Treble => "Treble (Hi-hat)",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            BeatFocusMode::Bass => "Detects kicks and sub-bass (20-250 Hz). Best for EDM, Hip-hop, Pop.",
            BeatFocusMode::Mid => "Detects snares and vocals (250-4000 Hz). Best for Rock, Jazz.",
            BeatFocusMode::Treble => "Detects hi-hats and cymbals (4000+ Hz). Best for fast, complex genres.",
        }
    }
    pub fn all() -> [BeatFocusMode; 3] {
        [BeatFocusMode::Bass, BeatFocusMode::Mid, BeatFocusMode::Treble]
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum BarInterpolation { Step, Smooth }
impl Default for BarInterpolation { fn default() -> Self { BarInterpolation::Step } }
impl BarInterpolation {
    pub fn display_name(self) -> &'static str {
        match self {
            BarInterpolation::Step => "Step (Blocks)",
            BarInterpolation::Smooth => "Smooth (Fluid Wave)",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            BarInterpolation::Step => "Classic rigid rectangles. Precise frequency reading.",
            BarInterpolation::Smooth => "Draws a filled curve between the bars. Looks like a fluid wave.",
        }
    }
    pub fn all() -> [BarInterpolation; 2] {
        [BarInterpolation::Step, BarInterpolation::Smooth]
    }
}

// ─────────────────────────────────────────────────────────────
// AudioSettings (unchanged)
// ─────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AudioSettings {
    #[serde(default = "default_fft_size")]             pub fft_size: usize,
    #[serde(default)]                                  pub window_function: WindowFunction,
    #[serde(default = "default_num_bars")]             pub num_bars: usize,
    #[serde(default)]                                  pub frequency_scaling: FrequencyScaling,
    #[serde(default = "default_min_freq")]             pub min_frequency_hz: f32,
    #[serde(default = "default_max_freq")]             pub max_frequency_hz: f32,
    #[serde(default = "default_min_db")]               pub min_decibels: f32,
    #[serde(default = "default_max_db")]               pub max_decibels: f32,
    #[serde(default = "default_target_range")]         pub target_dynamic_range_db: f32,
    #[serde(default = "default_sensitivity")]          pub sensitivity: f32,
    #[serde(default = "default_raw_gain")]             pub raw_gain: f32,
    #[serde(default = "default_response_gamma")]       pub response_gamma: f32,
    #[serde(default = "default_spectrum_tilt")]        pub spectrum_tilt: f32,
    #[serde(default = "default_eq_bass")]              pub eq_bass_db: f32,
    #[serde(default = "default_eq_mid")]               pub eq_mid_db: f32,
    #[serde(default = "default_eq_treble")]            pub eq_treble_db: f32,
    #[serde(default = "default_noise_gate")]           pub noise_gate: f32,
    #[serde(default = "default_agc_enabled")]          pub agc_enabled: bool,
    #[serde(default = "default_agc_attack")]           pub agc_attack: f32,
    #[serde(default = "default_agc_release")]          pub agc_release: f32,
    #[serde(default = "default_poll_rate_hz")]         pub poll_rate_hz: f32,
    #[serde(default = "default_attack_ms")]            pub attack_time_ms: f32,
    #[serde(default = "default_decay_ms")]             pub decay_time_ms: f32,
    #[serde(default)]                                  pub decay_mode: DecayMode,
    #[serde(default = "default_average_size")]         pub average_size: usize,
    #[serde(default = "default_peak_hold_ms")]         pub peak_hold_ms: f32,
    #[serde(default = "default_gravity_coeff")]        pub gravity_coefficient: f32,
    #[serde(default)]                                  pub channel_mix: ChannelMixMode,
    #[serde(default)]                                  pub beat_focus: BeatFocusMode,
    #[serde(default)]                                  pub bar_interpolation: BarInterpolation,
    #[serde(default = "default_beat_sensitivity")]     pub beat_detection_sensitivity: f32,
    #[serde(default = "default_energy_threshold")]     pub energy_threshold_multiplier: f32,
    #[serde(default)]                                  pub bpm_lock_enabled: bool,
    #[serde(default = "default_bpm_phase_smoothing")]  pub bpm_phase_smoothing: f32,
}

fn default_fft_size() -> usize { 2048 }
fn default_num_bars() -> usize { 86 }
fn default_min_freq() -> f32 { 20.0 }
fn default_max_freq() -> f32 { 20000.0 }
fn default_min_db() -> f32 { -85.0 }
fn default_max_db() -> f32 { -20.0 }
fn default_target_range() -> f32 { 60.0 }
fn default_sensitivity() -> f32 { 50.0 }
fn default_raw_gain() -> f32 { 1.0 }
fn default_response_gamma() -> f32 { 1.5 }
fn default_spectrum_tilt() -> f32 { 3.0 }
fn default_eq_bass() -> f32 { 0.0 }
fn default_eq_mid() -> f32 { 0.0 }
fn default_eq_treble() -> f32 { 0.0 }
fn default_noise_gate() -> f32 { 0.0 }
fn default_agc_enabled() -> bool { true }
fn default_agc_attack() -> f32 { 0.15 }
fn default_agc_release() -> f32 { 0.005 }
fn default_poll_rate_hz() -> f32 { 60.0 }
fn default_attack_ms() -> f32 { 5.0 }
fn default_decay_ms() -> f32 { 300.0 }
fn default_average_size() -> usize { 1 }
fn default_peak_hold_ms() -> f32 { 0.0 }
fn default_gravity_coeff() -> f32 { 1.0 }
fn default_beat_sensitivity() -> f32 { 0.5 }
fn default_energy_threshold() -> f32 { 1.5 }
fn default_bpm_phase_smoothing() -> f32 { 0.3 }

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            fft_size: default_fft_size(), window_function: WindowFunction::default(),
            num_bars: default_num_bars(), frequency_scaling: FrequencyScaling::default(),
            min_frequency_hz: default_min_freq(), max_frequency_hz: default_max_freq(),
            min_decibels: default_min_db(), max_decibels: default_max_db(),
            target_dynamic_range_db: default_target_range(), sensitivity: default_sensitivity(),
            raw_gain: default_raw_gain(), response_gamma: default_response_gamma(),
            spectrum_tilt: default_spectrum_tilt(), eq_bass_db: default_eq_bass(),
            eq_mid_db: default_eq_mid(), eq_treble_db: default_eq_treble(),
            noise_gate: default_noise_gate(), agc_enabled: default_agc_enabled(),
            agc_attack: default_agc_attack(), agc_release: default_agc_release(),
            poll_rate_hz: default_poll_rate_hz(), attack_time_ms: default_attack_ms(),
            decay_time_ms: default_decay_ms(), decay_mode: DecayMode::default(),
            average_size: default_average_size(), peak_hold_ms: default_peak_hold_ms(),
            gravity_coefficient: default_gravity_coeff(), channel_mix: ChannelMixMode::default(),
            beat_focus: BeatFocusMode::default(), bar_interpolation: BarInterpolation::default(),
            beat_detection_sensitivity: default_beat_sensitivity(),
            energy_threshold_multiplier: default_energy_threshold(),
            bpm_lock_enabled: false, bpm_phase_smoothing: default_bpm_phase_smoothing(),
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Audio presets (unchanged)
// ─────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum AudioPreset {
    Universal, Pop, Rock, Electronic, HipHop, Classical, Piano, Jazz, Ambient,
}
impl AudioPreset {
    pub fn all() -> [AudioPreset; 9] {
        [
            AudioPreset::Universal, AudioPreset::Pop, AudioPreset::Rock,
            AudioPreset::Electronic, AudioPreset::HipHop, AudioPreset::Classical,
            AudioPreset::Piano, AudioPreset::Jazz, AudioPreset::Ambient,
        ]
    }
    pub fn display_name(self) -> &'static str {
        match self {
            AudioPreset::Universal => "Universal",
            AudioPreset::Pop => "Pop",
            AudioPreset::Rock => "Rock",
            AudioPreset::Electronic => "Electronic / EDM",
            AudioPreset::HipHop => "Hip-hop",
            AudioPreset::Classical => "Classical",
            AudioPreset::Piano => "Piano",
            AudioPreset::Jazz => "Jazz",
            AudioPreset::Ambient => "Ambient",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            AudioPreset::Universal => "Balanced defaults that work well across most music.",
            AudioPreset::Pop => "Punchy and bright — emphasises vocals and kicks for modern radio pop.",
            AudioPreset::Rock => "Mid-forward with dynamic transients — good for guitars and drums.",
            AudioPreset::Electronic => "Kick-focused with tight decay and BPM lock — tuned for EDM and house.",
            AudioPreset::HipHop => "Bass-heavy and punchy — emphasises 808s and kick drums.",
            AudioPreset::Classical => "Preserves dynamic range (AGC off) — softer attack, long decay, clean tones.",
            AudioPreset::Piano => "Clean tone with sustain — good for solo piano and acoustic material.",
            AudioPreset::Jazz => "Warm and smooth — gentler response curve and longer decay.",
            AudioPreset::Ambient => "Slow and flowing — heavy smoothing, no peak-hold, soft attack.",
        }
    }
    pub fn settings(self) -> AudioSettings {
        let d = AudioSettings::default();
        match self {
            AudioPreset::Universal => d,
            AudioPreset::Pop => AudioSettings {
                fft_size: 2048, window_function: WindowFunction::Hann,
                sensitivity: 55.0, response_gamma: 1.8, spectrum_tilt: 1.5,
                target_dynamic_range_db: 55.0, decay_time_ms: 200.0,
                decay_mode: DecayMode::Exponential, average_size: 2,
                beat_detection_sensitivity: 0.6, energy_threshold_multiplier: 1.4,
                ..d
            },
            AudioPreset::Rock => AudioSettings {
                fft_size: 2048, window_function: WindowFunction::Hamming,
                sensitivity: 55.0, response_gamma: 1.6, spectrum_tilt: 2.0,
                target_dynamic_range_db: 60.0, attack_time_ms: 3.0,
                decay_time_ms: 250.0, decay_mode: DecayMode::Exponential,
                average_size: 2, beat_detection_sensitivity: 0.55,
                eq_mid_db: 1.5, beat_focus: BeatFocusMode::Mid, ..d
            },
            AudioPreset::Electronic => AudioSettings {
                fft_size: 1024, window_function: WindowFunction::Hann,
                sensitivity: 60.0, response_gamma: 2.0, spectrum_tilt: 1.5,
                target_dynamic_range_db: 50.0, attack_time_ms: 0.0,
                decay_time_ms: 150.0, decay_mode: DecayMode::Gravity,
                average_size: 1, beat_detection_sensitivity: 0.7,
                energy_threshold_multiplier: 1.3, bpm_lock_enabled: true,
                eq_bass_db: 1.5, eq_treble_db: 1.0, beat_focus: BeatFocusMode::Bass, ..d
            },
            AudioPreset::HipHop => AudioSettings {
                fft_size: 1024, window_function: WindowFunction::Hann,
                sensitivity: 65.0, response_gamma: 1.8, spectrum_tilt: 1.0,
                target_dynamic_range_db: 55.0, attack_time_ms: 0.0,
                decay_time_ms: 250.0, decay_mode: DecayMode::Gravity,
                average_size: 1, beat_detection_sensitivity: 0.7,
                energy_threshold_multiplier: 1.3, eq_bass_db: 2.5,
                beat_focus: BeatFocusMode::Bass, ..d
            },
            AudioPreset::Classical => AudioSettings {
                fft_size: 4096, window_function: WindowFunction::BlackmanHarris,
                min_decibels: -95.0, max_decibels: -15.0, target_dynamic_range_db: 75.0,
                sensitivity: 65.0, response_gamma: 1.2, spectrum_tilt: 4.5,
                agc_enabled: false, attack_time_ms: 15.0, decay_time_ms: 500.0,
                decay_mode: DecayMode::Linear, average_size: 3,
                beat_detection_sensitivity: 0.4, max_frequency_hz: 18000.0,
                beat_focus: BeatFocusMode::Mid, ..d
            },
            AudioPreset::Piano => AudioSettings {
                fft_size: 4096, window_function: WindowFunction::BlackmanHarris,
                min_decibels: -90.0, max_decibels: -20.0, target_dynamic_range_db: 65.0,
                sensitivity: 60.0, response_gamma: 1.3, spectrum_tilt: 4.5,
                attack_time_ms: 5.0, decay_time_ms: 600.0, decay_mode: DecayMode::Exponential,
                average_size: 2, beat_detection_sensitivity: 0.35,
                max_frequency_hz: 5000.0, eq_mid_db: 1.0,
                beat_focus: BeatFocusMode::Mid, ..d
            },
            AudioPreset::Jazz => AudioSettings {
                fft_size: 2048, window_function: WindowFunction::Hamming,
                min_decibels: -85.0, max_decibels: -22.0, target_dynamic_range_db: 60.0,
                sensitivity: 55.0, response_gamma: 1.4, spectrum_tilt: 3.5,
                attack_time_ms: 10.0, decay_time_ms: 400.0, decay_mode: DecayMode::Exponential,
                average_size: 3, beat_detection_sensitivity: 0.45, ..d
            },
            AudioPreset::Ambient => AudioSettings {
                fft_size: 4096, window_function: WindowFunction::BlackmanHarris,
                min_decibels: -95.0, max_decibels: -25.0, target_dynamic_range_db: 70.0,
                sensitivity: 70.0, response_gamma: 1.2, spectrum_tilt: 4.5,
                attack_time_ms: 40.0, decay_time_ms: 800.0, decay_mode: DecayMode::Exponential,
                average_size: 8, beat_detection_sensitivity: 0.3, ..d
            },
        }
    }
    pub fn apply(self, s: &mut AudioSettings) { *s = self.settings(); }
    pub fn detect(s: &AudioSettings) -> Option<AudioPreset> {
        for p in Self::all() {
            if p.settings() == *s { return Some(p); }
        }
        None
    }
}

// ─────────────────────────────────────────────────────────────
// Preset builders
// ─────────────────────────────────────────────────────────────

fn smp(name: &str, colors: Vec<[u8; 3]>, mode: StaticDistributionMode, bars: usize) -> StaticMultiPreset {
    StaticMultiPreset { name: name.into(), colors, mode, bars_per_color: bars, is_premade: true }
}

fn gp(name: &str, stops: Vec<GradientStop>, angle: f32) -> GradientPreset {
    GradientPreset { name: name.into(), stops, angle, is_premade: true }
}

fn gs(color: [u8; 3], position: f32) -> GradientStop { GradientStop { color, position } }

fn ccms(shape: MotionShape, blend: BlendMode, speed: f32, trail: f32, stutter: f32, wave: f32, wipe: f32, off: [u8; 3], off_a: u8) -> MotionSettings {
    MotionSettings {
        shape, blend, speed, trail_length: trail, stutter_intensity: stutter,
        wave_frequency: wave, wipe_softness: wipe,
        off_color: off, off_color_opacity: off_a,
    }
}

fn ccp(name: &str, colors: Vec<[u8; 3]>, speed: f32, motion: MotionSettings) -> ColorChangePreset {
    ColorChangePreset { name: name.into(), colors, speed, is_premade: true, motion }
}

// ─────────────────────────────────────────────────────────────
// Default presets
// ─────────────────────────────────────────────────────────────

fn default_static_colors() -> Vec<[u8; 3]> { vec![[255, 0, 0], [0, 255, 0], [0, 0, 255]] }
fn default_gradient_stops() -> Vec<GradientStop> {
    vec![GradientStop { color: [255, 0, 0], position: 0.0 }, GradientStop { color: [0, 0, 255], position: 1.0 }]
}
fn default_speed() -> f32 { 0.5 }
fn default_sequential_bars_per_color() -> usize { 1 }
fn default_visualizer_outline_thickness() -> f32 { 1.0 }
fn default_visualizer_outline_color() -> [u8; 3] { [0, 0, 0] }
fn default_height_scale() -> f32 { 0.5 }

fn default_static_single_source() -> ColorSource {
    ColorSource::Dynamic { palette_size: 1, variants_per_color: 0, priority: PalettePriority::MostCommon }
}
fn default_static_multi_source() -> ColorSource { ColorSource::Static(default_static_colors()) }
fn default_animated_source() -> ColorSource { ColorSource::Static(default_static_colors()) }
fn default_gradient_source() -> GradientSource { GradientSource::Static(default_gradient_stops()) }

fn default_static_multi_presets() -> Vec<StaticMultiPreset> {
    use StaticDistributionMode::*;
    vec![
        smp("Sunset Boulevard",  vec![[255, 94, 0], [255, 0, 128], [128, 0, 255]],                              Blocks,     1),
        smp("Deep Ocean",        vec![[0, 30, 60], [0, 100, 160], [0, 180, 216], [144, 224, 239]],             Sequential, 2),
        smp("Enchanted Forest",  vec![[20, 80, 20], [34, 139, 34], [50, 205, 50], [144, 238, 144]],            Random,     1),
        smp("Cyberpunk Neon",    vec![[255, 0, 255], [0, 255, 255], [255, 255, 0]],                           Vertical,   1),
        smp("Traffic Light",     vec![[255, 60, 60], [255, 200, 60], [60, 220, 60]],                          Vertical,   1),
        smp("Thermal Camera",    vec![[40, 20, 80], [140, 20, 140], [240, 80, 40], [255, 220, 60]],            Vertical,   1),
        smp("Fire & Ice",        vec![[255, 50, 0], [255, 150, 0], [100, 200, 255], [0, 150, 255]],            Vertical,   1),
        smp("Vaporwave",         vec![[255, 113, 206], [1, 205, 254], [5, 255, 161], [185, 103, 255]],          Vertical,   1),
        smp("Rainbow Blocks",    vec![[255, 0, 0], [255, 127, 0], [255, 255, 0], [0, 255, 0], [0, 0, 255], [75, 0, 130], [148, 0, 211]], Blocks, 1),
        smp("Candy Shop",        vec![[255, 105, 180], [255, 215, 0], [0, 255, 255]],                          Sequential, 3),
        smp("Lava Lamp",         vec![[255, 0, 0], [255, 100, 0], [150, 0, 200], [80, 0, 150]],                Random,     1),
        smp("Monochrome",        vec![[255, 255, 255], [150, 150, 150], [80, 80, 80]],                        Sequential, 2),
        smp("Iris Stack",        vec![[80, 40, 20], [180, 80, 60], [240, 180, 100], [220, 240, 180], [100, 180, 120]], Vertical, 1),
        smp("Pastel Dream",      vec![[255, 182, 193], [173, 216, 230], [144, 238, 144], [255, 255, 224]],     Blocks,     1),
        smp("Tropical",          vec![[0, 200, 150], [255, 200, 0], [255, 80, 80]],                           Random,     1),
        smp("Northern Sky",      vec![[10, 10, 60], [40, 40, 140], [80, 120, 220], [160, 200, 255]],           Vertical,   1),
    ]
}

fn default_gradient_presets() -> Vec<GradientPreset> {
    vec![
        gp("Sunset",           vec![gs([255, 94, 0], 0.0), gs([255, 0, 128], 0.5), gs([128, 0, 255], 1.0)], 0.0),
        gp("Ocean Deep",       vec![gs([0, 30, 60], 0.0), gs([0, 180, 216], 0.5), gs([144, 224, 239], 1.0)], 90.0),
        gp("Rainbow",          vec![gs([255, 0, 0], 0.0), gs([255, 127, 0], 0.17), gs([255, 255, 0], 0.33), gs([0, 255, 0], 0.5), gs([0, 0, 255], 0.67), gs([75, 0, 130], 0.83), gs([148, 0, 211], 1.0)], 45.0),
        gp("Fire",             vec![gs([255, 0, 0], 0.0), gs([255, 165, 0], 0.5), gs([255, 255, 0], 1.0)], 90.0),
        gp("Aurora",           vec![gs([0, 255, 150], 0.0), gs([0, 150, 255], 0.5), gs([150, 0, 255], 1.0)], 90.0),
        gp("Cotton Candy",     vec![gs([255, 182, 193], 0.0), gs([173, 216, 230], 0.5), gs([255, 105, 180], 1.0)], 0.0),
        gp("Gold Rush",        vec![gs([80, 40, 0], 0.0), gs([255, 180, 0], 0.5), gs([255, 255, 150], 1.0)], 90.0),
        gp("Neon Nights",      vec![gs([255, 0, 150], 0.0), gs([100, 0, 255], 0.5), gs([0, 255, 255], 1.0)], 45.0),
        gp("Enchanted Forest", vec![gs([0, 50, 0], 0.0), gs([50, 150, 50], 0.5), gs([200, 255, 150], 1.0)], 90.0),
        gp("Grayscale",        vec![gs([0, 0, 0], 0.0), gs([255, 255, 255], 1.0)], 90.0),
        gp("Twilight",         vec![gs([25, 25, 80], 0.0), gs([150, 50, 150], 0.5), gs([255, 150, 100], 1.0)], 90.0),
        gp("Ice",              vec![gs([200, 240, 255], 0.0), gs([100, 180, 255], 0.5), gs([30, 100, 200], 1.0)], 90.0),
        gp("Lava Flow",        vec![gs([10, 0, 0], 0.0), gs([180, 30, 0], 0.3), gs([255, 180, 0], 0.7), gs([255, 255, 200], 1.0)], 90.0),
        gp("Cyber Grid",       vec![gs([255, 0, 150], 0.0), gs([100, 0, 255], 0.5), gs([0, 255, 255], 1.0)], 45.0),
        gp("Cherry Blossom",   vec![gs([255, 200, 220], 0.0), gs([255, 105, 180], 0.5), gs([180, 50, 120], 1.0)], 0.0),
        gp("Northern Lights",  vec![gs([0, 20, 20], 0.0), gs([0, 180, 150], 0.4), gs([100, 255, 180], 0.7), gs([200, 240, 255], 1.0)], 90.0),
    ]
}

fn default_color_change_presets() -> Vec<ColorChangePreset> {
    use MotionShape::*; use BlendMode::*;
    vec![
        ccp("Rainbow Cycle",     vec![[255, 0, 0], [255, 127, 0], [255, 255, 0], [0, 255, 0], [0, 0, 255], [75, 0, 130], [148, 0, 211]],
            0.5, ccms(Uniform, Smooth, 1.0, 0.25, 0.3, 2.0, 0.1, [0, 0, 0], 255)),
        ccp("Aurora Veil",       vec![[0, 255, 180], [60, 140, 255], [180, 60, 255]],
            0.35, ccms(VerticalWipe, Smooth, 1.0, 0.25, 0.3, 2.0, 0.35, [4, 8, 20], 220)),
        ccp("Comet Tail",        vec![[255, 200, 80], [255, 90, 0], [120, 20, 0]],
            0.55, ccms(Comet, Smooth, 1.0, 0.55, 0.3, 2.0, 0.1, [0, 0, 0], 255)),
        ccp("Neon Rain",         vec![[255, 0, 255], [0, 255, 255], [0, 255, 120]],
            0.75, ccms(Chase, Hard, 1.0, 0.25, 0.3, 2.0, 0.1, [0, 0, 0], 255)),
        ccp("Heartbeat Pop",     vec![[255, 60, 90], [255, 180, 180]],
            0.45, ccms(Heartbeat, Smooth, 1.0, 0.25, 0.3, 2.0, 0.1, [30, 0, 10], 180)),
        ccp("Candy Sparkle",     vec![[255, 105, 180], [255, 215, 0], [0, 255, 255]],
            0.4, ccms(Sparkle, Hard, 1.0, 0.25, 0.5, 2.0, 0.1, [20, 0, 20], 200)),
        ccp("Fireflies",         vec![[255, 220, 0], [255, 120, 0]],
            0.5, ccms(Confetti, Smooth, 1.0, 0.25, 0.35, 2.0, 0.1, [10, 5, 0], 150)),
        ccp("Diagonal Storm",    vec![[40, 40, 255], [200, 0, 255], [255, 0, 80]],
            0.6, ccms(DiagonalWipe, Smooth, 1.0, 0.25, 0.3, 2.0, 0.2, [0, 0, 10], 230)),
        ccp("Plasma Drift",      vec![[255, 50, 200], [0, 200, 255], [180, 255, 0]],
            0.3, ccms(Plasma, Smooth, 1.0, 0.25, 0.3, 2.0, 0.1, [0, 0, 0], 255)),
        ccp("Electric Pulse",    vec![[0, 255, 255], [255, 0, 255]],
            0.65, ccms(Pulse, Smooth, 1.0, 0.25, 0.3, 2.0, 0.1, [0, 0, 0], 255)),
        ccp("Bouncing Comet",    vec![[255, 255, 0], [255, 0, 0]],
            0.5, ccms(BouncingBall, Smooth, 1.0, 0.4, 0.3, 2.0, 0.1, [10, 10, 0], 220)),
        ccp("Retro Strobe",      vec![[255, 80, 80], [80, 80, 255]],
            0.85, ccms(Strobe, Hard, 1.0, 0.25, 0.3, 2.0, 0.1, [0, 0, 0], 255)),
        ccp("Smooth Sinelon",    vec![[120, 255, 120], [0, 180, 255], [255, 120, 255]],
            0.4, ccms(Sinelon, Smooth, 1.0, 0.6, 0.3, 2.0, 0.1, [0, 0, 0], 255)),
        ccp("Radial Burst",      vec![[255, 200, 0], [255, 50, 100], [100, 50, 255]],
            0.4, ccms(RadialWipe, Smooth, 1.0, 0.25, 0.3, 2.0, 0.15, [0, 0, 0], 255)),
        ccp("Spiral Galaxy",     vec![[255, 255, 255], [150, 100, 255], [50, 20, 150]],
            0.3, ccms(Spiral, Smooth, 1.0, 0.25, 0.3, 2.0, 0.1, [0, 0, 0], 255)),
        ccp("Ripple Pond",       vec![[0, 150, 200], [100, 220, 255], [255, 255, 255]],
            0.35, ccms(Ripple, Smooth, 1.0, 0.25, 0.3, 2.0, 0.1, [0, 0, 0], 255)),
        ccp("Wave Interference", vec![[255, 100, 100], [100, 255, 100], [100, 100, 255]],
            0.5, ccms(WaveInterference, Smooth, 1.0, 0.25, 0.3, 2.0, 0.1, [0, 0, 0], 255)),
    ]
}

// ─────────────────────────────────────────────────────────────
// AppConfig
// ─────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
pub struct AppConfig {
    pub bar_width: f32, pub bar_gap: f32, pub bar_radius: f32,
    pub dynamic_colors: bool, pub text_border: bool, pub font_path: Option<String>,
    pub bg_opacity: u8,
    #[serde(default)] pub bg_color: [u8; 3],
    pub color_presets: Vec<[u8; 3]>,
    pub x_offset: f32, pub y_offset: f32,

    #[serde(default, alias = "bar_outline_enabled")]
    pub visualizer_outline_enabled: bool,
    #[serde(default = "default_visualizer_outline_thickness", alias = "bar_outline_thickness")]
    pub visualizer_outline_thickness: f32,
    #[serde(default = "default_visualizer_outline_color", alias = "bar_outline_color")]
    pub visualizer_outline_color: [u8; 3],

    pub max_bar_height: f32, pub target_fps: f32, pub show_fps: bool, pub use_vsync: bool,
    #[serde(default = "default_height_scale")] pub height_scale: f32,

    #[serde(default)] pub visualizer_direction: VisualizerDirection,
    #[serde(default)] pub frequency_direction: FrequencyDirection,
    #[serde(default)] pub color_mode: ColorMode,
    #[serde(default = "default_static_single_source")] pub static_single_source: ColorSource,
    #[serde(default = "default_static_multi_source")]  pub static_multi_source: ColorSource,
    #[serde(default)] pub static_mode: StaticDistributionMode,
    #[serde(default = "default_sequential_bars_per_color")] pub sequential_bars_per_color: usize,
    #[serde(default = "default_static_multi_presets")] pub static_multi_presets: Vec<StaticMultiPreset>,

    #[serde(default)] pub vertical_color_style: VerticalColorStyle,

    #[serde(default = "default_gradient_source")] pub gradient_source: GradientSource,
    #[serde(default)] pub gradient_angle: f32,
    #[serde(default = "default_gradient_presets")] pub gradient_presets: Vec<GradientPreset>,

    #[serde(default = "default_animated_source")] pub animated_source: ColorSource,
    #[serde(default = "default_speed")] pub color_change_speed: f32,
    #[serde(default = "default_color_change_presets")] pub color_change_presets: Vec<ColorChangePreset>,
    #[serde(default)] pub color_change_motion: MotionSettings,
    #[serde(default)] pub track_info: TrackInfoConfig,
    #[serde(default)] pub audio: AudioSettings,

    #[serde(default, skip_serializing_if = "Option::is_none")] pub num_bars: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub fft_size: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub audio_update_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub sensitivity: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub bar_color: Option<[u8; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub static_colors: Option<Vec<[u8; 3]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub gradient_stops: Option<Vec<GradientStop>>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub color_change_colors: Option<Vec<[u8; 3]>>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            bar_width: 9.0, bar_gap: 1.0, bar_radius: 2.0,
            dynamic_colors: true, text_border: false, font_path: None,
            bg_opacity: 0, bg_color: [0, 0, 0],
            color_presets: vec![
                [255, 255, 255], [0, 255, 255], [255, 0, 255], [255, 255, 0], [255, 0, 0],
                [0, 255, 0], [0, 0, 255], [255, 165, 0], [128, 0, 128], [0, 128, 128],
            ],
            x_offset: 0.0, y_offset: 0.0,
            visualizer_outline_enabled: false,
            visualizer_outline_thickness: 1.0,
            visualizer_outline_color: [0, 0, 0],
            max_bar_height: 0.0, target_fps: 60.0, show_fps: false, use_vsync: false,
            height_scale: default_height_scale(),
            visualizer_direction: VisualizerDirection::default(),
            frequency_direction: FrequencyDirection::default(),
            color_mode: ColorMode::StaticSingle,
            static_single_source: default_static_single_source(),
            static_multi_source: default_static_multi_source(),
            static_mode: StaticDistributionMode::Blocks,
            sequential_bars_per_color: default_sequential_bars_per_color(),
            static_multi_presets: default_static_multi_presets(),
            vertical_color_style: VerticalColorStyle::default(),
            gradient_source: default_gradient_source(),
            gradient_angle: 0.0,
            gradient_presets: default_gradient_presets(),
            animated_source: default_animated_source(),
            color_change_speed: default_speed(),
            color_change_presets: default_color_change_presets(),
            color_change_motion: MotionSettings::default(),
            track_info: TrackInfoConfig::default(),
            audio: AudioSettings::default(),
            num_bars: None, fft_size: None, audio_update_ms: None, sensitivity: None,
            bar_color: None, static_colors: None, gradient_stops: None, color_change_colors: None,
        }
    }
}

impl AppConfig {
    pub fn get_path() -> PathBuf {
        dirs::config_dir().unwrap_or_else(|| PathBuf::from("."))
            .join("RustVisualizer").join("config.toml")
    }

    /// Overwrites all premade presets with the shipped defaults, preserving any user presets.
    fn refresh_premade_presets(&mut self) {
    // If no preset anywhere is marked premade, this config predates the field.
    // Wipe and replace everything (user said they haven't released yet).
    let has_any_premade = self.static_multi_presets.iter().any(|p| p.is_premade)
        || self.gradient_presets.iter().any(|p| p.is_premade)
        || self.color_change_presets.iter().any(|p| p.is_premade);
    if !has_any_premade {
        self.static_multi_presets = default_static_multi_presets();
        self.gradient_presets = default_gradient_presets();
        self.color_change_presets = default_color_change_presets();
        return;
    }
    // Otherwise: replace premades, keep user presets.
    self.static_multi_presets.retain(|p| !p.is_premade);
    self.gradient_presets.retain(|p| !p.is_premade);
    self.color_change_presets.retain(|p| !p.is_premade);

    let mut new_smp = default_static_multi_presets();
    new_smp.append(&mut self.static_multi_presets);
    self.static_multi_presets = new_smp;

    let mut new_gp = default_gradient_presets();
    new_gp.append(&mut self.gradient_presets);
    self.gradient_presets = new_gp;

    let mut new_ccp = default_color_change_presets();
    new_ccp.append(&mut self.color_change_presets);
    self.color_change_presets = new_ccp;
}

    pub fn load() -> Self {
        let path = Self::get_path();
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(mut config) = toml::from_str::<AppConfig>(&content) {
                if let Some(c) = config.bar_color.take() { config.static_single_source = ColorSource::Static(vec![c]); }
                if let Some(c) = config.static_colors.take() { config.static_multi_source = ColorSource::Static(c); }
                if let Some(s) = config.gradient_stops.take() { config.gradient_source = GradientSource::Static(s); }
                if let Some(c) = config.color_change_colors.take() { config.animated_source = ColorSource::Static(c); }
                if config.color_mode == ColorMode::Dynamic {
                    config.color_mode = ColorMode::StaticSingle;
                    config.static_single_source = ColorSource::Dynamic { palette_size: 1, variants_per_color: 0, priority: PalettePriority::MostCommon };
                }
                if let Some(n) = config.num_bars.take() { config.audio.num_bars = n; }
                if let Some(f) = config.fft_size.take() { config.audio.fft_size = f; }
                if let Some(ms) = config.audio_update_ms.take() {
                    if ms > 0 { config.audio.poll_rate_hz = 1000.0 / ms as f32; }
                }
                if let Some(s) = config.sensitivity.take() { config.audio.sensitivity = s.clamp(0.0, 100.0); }

                config.refresh_premade_presets();
                return config;
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        let path = Self::get_path();
        if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
        if let Ok(s) = toml::to_string(self) { let _ = std::fs::write(path, s); }
    }
}
