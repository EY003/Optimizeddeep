use std::collections::VecDeque;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use crossbeam_channel::{Receiver, Sender};
use realfft::num_complex::Complex32;
use realfft::{RealFftPlanner, RealToComplex};
use wasapi::*;

use crate::config::{
    AudioSettings, DecayMode, FrequencyScaling, WindowFunction,
};

// ─────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────

/// One analysis output frame. `values[i]` is the bar height in [0, 1];
/// `peaks[i]` is the peak-hold marker for the same bar. `beat`/`bpm`/`phase`
/// are exposed so future effects can react to rhythm without re-analysing.
#[derive(Clone, Debug)]
pub struct BarFrame {
    pub values: Vec<f32>,
    pub peaks: Vec<f32>,
    pub beat: bool,
    pub bpm: f32,
    pub phase: f32,
}

pub struct AudioEngine {
    pub device_name: String,
    pub frame_rx: Receiver<BarFrame>,
    pub settings: Arc<ArcSwap<AudioSettings>>,
}

impl AudioEngine {
    pub fn new(initial: AudioSettings) -> Self {
        let settings = Arc::new(ArcSwap::from_pointee(initial));

        let (frame_tx, frame_rx) = crossbeam_channel::bounded(2);
        // Bounded sample channel — if the analysis thread ever falls
        // behind, WASAPI continues to mirror into its own buffer and we
        // simply drop the oldest pending packet. 32 packets at ~10 ms is
        // ~320 ms of slack.
        let (sample_tx, sample_rx) = crossbeam_channel::bounded::<Vec<f32>>(32);
        let (info_tx, info_rx) = crossbeam_channel::bounded(1);

        // ── Capture thread ────────────────────────────────────────
        let capture_handle = thread::spawn(move || {
            capture_loop(sample_tx, info_tx);
        });

        // Wait for sample rate + device name before spawning analysis.
        let (device_name, sample_rate) = info_rx
            .recv()
            .unwrap_or_else(|_| ("Unknown".to_string(), 48000.0));

        // ── Analysis thread ──────────────────────────────────────
        let settings_for_analysis = settings.clone();
        thread::spawn(move || {
            analysis_loop(
                settings_for_analysis,
                sample_rx,
                frame_tx,
                sample_rate,
            );
        });

        // Detach the capture thread handle. It exits when `sample_tx` is
        // dropped, i.e. never during normal operation.
        drop(capture_handle);

        Self { device_name, frame_rx, settings }
    }
}

// ─────────────────────────────────────────────────────────────
// Capture thread — WASAPI loopback → mono f32 chunks
// ─────────────────────────────────────────────────────────────

fn capture_loop(sample_tx: Sender<Vec<f32>>, info_tx: Sender<(String, f32)>) {
    let _ = initialize_mta();

    let device = match get_default_device(&Direction::Render) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("WASAPI: failed to get default render device: {e:?}");
            let _ = info_tx.send(("Unknown".to_string(), 48000.0));
            return;
        }
    };
    let mut audio_client = match device.get_iaudioclient() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("WASAPI: failed to get IAudioClient: {e:?}");
            let _ = info_tx.send(("Unknown".to_string(), 48000.0));
            return;
        }
    };

    let device_name = device
        .get_friendlyname()
        .unwrap_or_else(|_| "Default Device".to_string());

    let format = match audio_client.get_mixformat() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("WASAPI: failed to get mix format: {e:?}");
            let _ = info_tx.send((device_name, 48000.0));
            return;
        }
    };
    let sample_rate = format.get_samplespersec() as f32;
    let channels = format.get_nchannels() as usize;
    let blockalign = format.get_blockalign() as usize;
    let samples_per_frame = (blockalign / 4).max(1);

    // `get_periods()` returns `(i64, i64)` — both values are REFERENCE_TIME
    // in 100 ns units (Windows convention), *not* `Duration`. Fall back to
    // 100_000 units = 10 ms if the query fails.
    let (_, min_time) = match audio_client.get_periods() {
        Ok(p) => p,
        Err(_) => (100_000i64, 100_000i64),
    };

    if audio_client
        .initialize_client(&format, min_time, &Direction::Capture, &ShareMode::Shared, true)
        .is_err()
    {
        let _ = info_tx.send((device_name, sample_rate));
        return;
    }

    let _h_event = audio_client.set_get_eventhandle().ok();
    let capture_client = match audio_client.get_audiocaptureclient() {
        Ok(c) => c,
        Err(_) => {
            let _ = info_tx.send((device_name, sample_rate));
            return;
        }
    };

    if audio_client.start_stream().is_err() {
        let _ = info_tx.send((device_name, sample_rate));
        return;
    }

    let _ = info_tx.send((device_name, sample_rate));

    let mut sample_buffer: Vec<f32> = Vec::new();
    let mut mono: Vec<f32> = Vec::with_capacity(4096);

    loop {
        let mut packet_size = 0u32;
        // Spin-wait on the WASAPI capture client with a small backoff.
        for _ in 0..2000 {
            packet_size = capture_client
                .get_next_nbr_frames()
                .ok()
                .flatten()
                .unwrap_or(0);
            if packet_size > 0 {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        if packet_size == 0 {
            continue;
        }

        let needed = packet_size as usize * samples_per_frame;
        sample_buffer.resize(needed, 0.0);

        let byte_slice: &mut [u8] = unsafe {
            std::slice::from_raw_parts_mut(
                sample_buffer.as_mut_ptr() as *mut u8,
                needed * std::mem::size_of::<f32>(),
            )
        };

        let (frames_read, _) = capture_client
            .read_from_device(byte_slice)
            .unwrap_or((0, BufferFlags::none()));
        if frames_read == 0 {
            continue;
        }

        let valid_samples =
            (frames_read as usize * samples_per_frame).min(sample_buffer.len());
        let samples = &sample_buffer[..valid_samples];

        // ── Mono mixdown: RMS per-channel then sum ─────────────
        //
        // For each frame, compute `sqrt( sum(ch^2) / n )`. This
        // preserves energy even when L and R cancel (phase-inverted
        // bass), unlike a simple (L+R)/2 average which zeroes out.
        // Because every frame value is non-negative, the DC offset
        // shows up in FFT bin 0, which we skip.
        mono.clear();
        mono.reserve(valid_samples / channels.max(1));
        for chunk in samples.chunks_exact(channels.max(1)) {
            let sum_sq: f32 = chunk.iter().map(|s| s * s).sum();
            mono.push((sum_sq / chunk.len() as f32).sqrt());
        }

        if mono.is_empty() {
            continue;
        }

        // Hand the analysis thread a fresh Vec. If it can't keep up,
        // the send will return Err and we drop this chunk — the visual
        // will skip one poll interval, which is far preferable to
        // blocking the WASAPI thread.
        if sample_tx.send(mono.clone()).is_err() {
            return;
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Analysis thread — the full DSP pipeline
// ─────────────────────────────────────────────────────────────

fn analysis_loop(
    settings: Arc<ArcSwap<AudioSettings>>,
    sample_rx: Receiver<Vec<f32>>,
    frame_tx: Sender<BarFrame>,
    sample_rate: f32,
) {
    let mut processor = AudioProcessor::new(settings.clone(), sample_rate);
    let mut history: Vec<f32> = Vec::with_capacity(16_384);
    let mut last_tick = Instant::now();

    loop {
        // ── 1. Read current settings ──
        let current = settings.load_full();
        let poll_period = Duration::from_secs_f32(1.0 / current.poll_rate_hz.max(1.0));

        // ── 2. Drain whatever samples have arrived ──
        while let Ok(chunk) = sample_rx.try_recv() {
            history.extend_from_slice(&chunk);
        }

        // Cap history so it can't grow unbounded if the poll rate is
        // very low. 4× fft_size is enough for any window we might
        // need, plus smooth overlap.
        let cap = (current.fft_size * 4).max(8192);
        if history.len() > cap {
            let drop_n = history.len() - cap;
            history.drain(0..drop_n);
        }

        // ── 3. Process ──
        let dt = last_tick.elapsed().as_secs_f32().clamp(0.001, 0.2);
        last_tick = Instant::now();

        if let Some(frame) = processor.process(&history, &current, dt) {
            let _ = frame_tx.try_send(frame);
        }

        // ── 4. Sleep until the next tick ──
        let elapsed = last_tick.elapsed();
        if elapsed < poll_period {
            spin_sleep::sleep(poll_period - elapsed);
        }
    }
}

// ─────────────────────────────────────────────────────────────
// The processor — all DSP state lives here
// ─────────────────────────────────────────────────────────────

struct AudioProcessor {
    settings: Arc<ArcSwap<AudioSettings>>,
    sample_rate: f32,

    // FFT state
    cached_fft_size: usize,
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    cached_window_fn: WindowFunction,
    input_buffer: Vec<f32>,
    output_buffer: Vec<Complex32>,
    scratch_buffer: Vec<Complex32>,

    // Bin mapper (bin ranges per bar)
    cached_num_bars: usize,
    cached_scaling: FrequencyScaling,
    bin_ranges: Vec<(usize, usize)>,

    // Per-bar output state
    bar_values: Vec<f32>,
    bar_peaks: Vec<f32>,
    bar_velocities: Vec<f32>,
    peak_hold_remaining: Vec<f32>,

    // Temporal smoothing (moving average across frames)
    frame_history: VecDeque<Vec<f32>>,

    // Amplitude
    agc_envelope: f32,

    // Beat detection
    lp_state: f32,
    lp_alpha: f32,
    low_energy_history: VecDeque<f32>,

    // BPM lock
    beat_times: VecDeque<f64>,
    bpm: f32,
    phase: f32,
    time_elapsed: f64,
}

impl AudioProcessor {
    fn new(settings: Arc<ArcSwap<AudioSettings>>, sample_rate: f32) -> Self {
        let current = settings.load();
        let fft_size = current.fft_size;
        let window_fn = current.window_function;
        let num_bars = current.num_bars;
        let scaling = current.frequency_scaling;

        let mut planner = RealFftPlanner::new();
        let fft = planner.plan_fft_forward(fft_size);
        let input_buffer = fft.make_input_vec();
        let output_buffer = fft.make_output_vec();
        let scratch_buffer = fft.make_scratch_vec();

        let window = make_window(window_fn, fft_size);
        let bin_ranges = make_bin_ranges(fft_size, num_bars, scaling, sample_rate);

        // One-pole low-pass at 200 Hz for beat detection.
        let rc = 1.0 / (2.0 * std::f32::consts::PI * 200.0);
        let dt_s = 1.0 / sample_rate.max(1.0);
        let lp_alpha = dt_s / (rc + dt_s);

        let mut lp = Self {
            settings,
            sample_rate,
            cached_fft_size: fft_size,
            fft,
            window,
            cached_window_fn: window_fn,
            input_buffer,
            output_buffer,
            scratch_buffer,
            cached_num_bars: num_bars,
            cached_scaling: scaling,
            bin_ranges,
            bar_values: vec![0.0; num_bars],
            bar_peaks: vec![0.0; num_bars],
            bar_velocities: vec![0.0; num_bars],
            peak_hold_remaining: vec![0.0; num_bars],
            frame_history: VecDeque::with_capacity(16),
            agc_envelope: 0.1,
            lp_state: 0.0,
            lp_alpha,
            low_energy_history: VecDeque::with_capacity(240),
            beat_times: VecDeque::with_capacity(8),
            bpm: 0.0,
            phase: 0.0,
            time_elapsed: 0.0,
        };
        lp.update_initial_state(&current);
        lp
    }

    /// Resize / re-tune per-bar state if the current settings changed
    /// since last call. Called at the top of `process`.
    fn update_initial_state(&mut self, s: &AudioSettings) {
        // FFT size changed → re-plan FFT + window + bin ranges.
        if s.fft_size != self.cached_fft_size {
            let mut planner = RealFftPlanner::new();
            self.fft = planner.plan_fft_forward(s.fft_size);
            self.input_buffer = self.fft.make_input_vec();
            self.output_buffer = self.fft.make_output_vec();
            self.scratch_buffer = self.fft.make_scratch_vec();
            self.cached_fft_size = s.fft_size;
            self.cached_window_fn = s.window_function;
            self.window = make_window(s.window_function, s.fft_size);
            self.bin_ranges = make_bin_ranges(
                s.fft_size, s.num_bars, s.frequency_scaling, self.sample_rate,
            );
        }

        // Window function changed without FFT size change.
        if s.window_function != self.cached_window_fn {
            self.cached_window_fn = s.window_function;
            self.window = make_window(s.window_function, s.fft_size);
        }

        // Bin mapping inputs changed.
        if s.num_bars != self.cached_num_bars || s.frequency_scaling != self.cached_scaling {
            self.cached_num_bars = s.num_bars;
            self.cached_scaling = s.frequency_scaling;
            self.bin_ranges = make_bin_ranges(
                s.fft_size, s.num_bars, s.frequency_scaling, self.sample_rate,
            );
            self.resize_bar_state(s.num_bars);
        }
    }

    /// Resize per-bar state vectors so their length equals `n`.
    /// `bin_ranges` is always regenerated by `update_initial_state`
    /// before this is called, so we don't touch it here.
    fn resize_bar_state(&mut self, n: usize) {
        self.bar_values.resize(n, 0.0);
        self.bar_peaks.resize(n, 0.0);
        self.bar_velocities.resize(n, 0.0);
        self.peak_hold_remaining.resize(n, 0.0);
        for f in self.frame_history.iter_mut() {
            f.resize(n, 0.0);
        }
    }

    fn process(
        &mut self,
        history: &[f32],
        s: &AudioSettings,
        dt: f32,
    ) -> Option<BarFrame> {
        self.update_initial_state(s);
        self.time_elapsed += dt as f64;

        let n_fft = self.cached_fft_size;
        if history.len() < n_fft {
            // Not enough audio yet — emit a silent frame instead of
            // dropping it, so the UI doesn't freeze the last value.
            return Some(BarFrame {
                values: self.bar_values.clone(),
                peaks: self.bar_peaks.clone(),
                beat: false,
                bpm: self.bpm,
                phase: self.phase,
            });
        }
        let window_slice = &history[history.len() - n_fft..];

        // ── AGC on the raw input ──────────────────────────────
        // Compute RMS of the current window, then update a slow
        // envelope. AGC gain is applied *after* FFT so that the
        // values inside one frame are consistently scaled.
        let raw_rms = {
            let sum_sq: f32 = window_slice.iter().map(|x| x * x).sum();
            (sum_sq / n_fft as f32).sqrt()
        };
        if s.agc_enabled {
            let desired = 0.1 / raw_rms.max(1e-6);
            let coeff = if desired < self.agc_envelope {
                s.agc_attack.clamp(0.0001, 1.0)
            } else {
                s.agc_release.clamp(0.0001, 1.0)
            };
            self.agc_envelope += (desired - self.agc_envelope) * coeff;
        } else {
            self.agc_envelope = 1.0;
        }
        let agc_gain = self.agc_envelope.clamp(0.01, 100.0);

        // ── Pre-FFT gain ──────────────────────────────────────
        let user_gain = s.raw_gain.max(0.01) * agc_gain;

        // ── FFT ───────────────────────────────────────────────
        self.input_buffer.copy_from_slice(window_slice);
        for i in 0..n_fft {
            self.input_buffer[i] *= self.window[i];
        }
        if self.fft
            .process_with_scratch(
                &mut self.input_buffer,
                &mut self.output_buffer,
                &mut self.scratch_buffer,
            )
            .is_err()
        {
            return None;
        }

        // ── Bin → bar mapping (with intra-bar averaging) ──────
        let num_bars = self.cached_num_bars;
        let nyquist = n_fft / 2;
        let norm = 1.0 / (n_fft as f32 / 2.0) * user_gain;

        let mut raw_bars = vec![0.0f32; num_bars];
        for (i, range) in self.bin_ranges.iter().enumerate().take(num_bars) {
            let (start, end) = *range;
            let end = end.min(nyquist).max(start + 1);
            let mut mag = 0.0f32;
            for bin in start..end {
                mag += self.output_buffer[bin].norm();
            }
            mag /= (end - start) as f32;
            raw_bars[i] = (mag * norm + 1e-9).max(1e-9);
        }

        // ── dB mapping with sensitivity shift ─────────────────
        let sens_norm = (s.sensitivity / 100.0).clamp(0.0, 1.0);
        let min_db_effective = s.min_decibels - (sens_norm - 0.5) * 20.0;
        let db_span = (s.max_decibels - min_db_effective).max(1.0);
        let target_ratio = db_span / s.target_dynamic_range_db.max(1.0);
        let gamma = s.response_gamma.max(0.1) * target_ratio;

        let mut mapped = vec![0.0f32; num_bars];
        for i in 0..num_bars {
            let db = 20.0 * raw_bars[i].log10();
            let n01 = ((db - min_db_effective) / db_span).clamp(0.0, 1.0);
            mapped[i] = n01.powf(gamma);
        }

        // ── Temporal smoothing (average_size) ─────────────────
        let avg = s.average_size.max(1);
        if self.frame_history.len() >= avg {
            self.frame_history.pop_front();
        }
        self.frame_history.push_back(mapped);
        let smoothed: Vec<f32> = {
            let n = num_bars;
            let count = self.frame_history.len() as f32;
            let mut out = vec![0.0f32; n];
            for frame in &self.frame_history {
                for i in 0..n.min(frame.len()) {
                    out[i] += frame[i];
                }
            }
            for x in &mut out {
                *x /= count.max(1.0);
            }
            out
        };

        // ── Ballistics ────────────────────────────────────────
        self.update_ballistics(&smoothed, s, dt);

        // ── Beat detection on low-band energy ─────────────────
        let (beat, phase) = self.detect_rhythm(window_slice, s, dt);

        Some(BarFrame {
            values: self.bar_values.clone(),
            peaks: self.bar_peaks.clone(),
            beat,
            bpm: self.bpm,
            phase,
        })
    }

    fn update_ballistics(&mut self, inputs: &[f32], s: &AudioSettings, dt: f32) {
        let n = self.bar_values.len().min(inputs.len());
        let attack_alpha = if s.attack_time_ms <= 0.0 {
            1.0
        } else {
            1.0 - (-dt * 1000.0 / s.attack_time_ms).exp()
        };
        let decay_s = (s.decay_time_ms / 1000.0).max(0.001);
        let peak_hold_s = s.peak_hold_ms / 1000.0;

        for i in 0..n {
            let input = inputs[i].clamp(0.0, 1.0);
            let prev = self.bar_values[i];

            if input > prev {
                // Attack — reset gravity velocity.
                self.bar_velocities[i] = 0.0;
                self.bar_values[i] = prev + (input - prev) * attack_alpha;
            } else {
                // Decay.
                self.bar_values[i] = match s.decay_mode {
                    DecayMode::Linear => (prev - dt / decay_s).max(input),
                    DecayMode::Exponential => {
                        let tau = decay_s / 3.0;
                        (input + (prev - input) * (-dt / tau).exp()).max(input)
                    }
                    DecayMode::Gravity => {
                        let g = 2.0 / (decay_s * decay_s)
                            * s.gravity_coefficient.max(0.01);
                        self.bar_velocities[i] += g * dt;
                        (prev - self.bar_velocities[i] * dt).max(input)
                    }
                };
            }
            self.bar_values[i] = self.bar_values[i].clamp(0.0, 1.0);

            // ── Peak-hold marker ──────────────────────────────
            if self.bar_values[i] >= self.bar_peaks[i] {
                self.bar_peaks[i] = self.bar_values[i];
                self.peak_hold_remaining[i] = peak_hold_s;
            } else if self.peak_hold_remaining[i] > 0.0 {
                self.peak_hold_remaining[i] -= dt;
            } else {
                let fall_rate = 1.0 / decay_s;
                self.bar_peaks[i] =
                    (self.bar_peaks[i] - dt * fall_rate).max(self.bar_values[i]);
            }
        }
    }

    fn detect_rhythm(&mut self, window: &[f32], s: &AudioSettings, dt: f32) -> (bool, f32) {
        // One-pole low-pass the current window to isolate kick-band energy.
        let mut lp = self.lp_state;
        let mut sum_sq = 0.0f32;
        for &x in window {
            lp += self.lp_alpha * (x - lp);
            sum_sq += lp * lp;
        }
        self.lp_state = lp;
        let energy = (sum_sq / window.len() as f32).sqrt();

        // Maintain ~1.5 s of energy history.
        let hist_len = (s.poll_rate_hz * 1.5).max(30.0) as usize;
        self.low_energy_history.push_back(energy);
        while self.low_energy_history.len() > hist_len {
            self.low_energy_history.pop_front();
        }
        let local_avg = if self.low_energy_history.is_empty() {
            energy
        } else {
            self.low_energy_history.iter().sum::<f32>()
                / self.low_energy_history.len() as f32
        };

        let sensitivity = s.beat_detection_sensitivity.clamp(0.0, 1.0);
        let threshold = local_avg
            * s.energy_threshold_multiplier.max(1.0)
            * (1.0 - sensitivity * 0.5);
        let is_beat = energy > threshold
            && energy > local_avg * 1.05
            && self.time_elapsed - self.beat_times.back().copied().unwrap_or(0.0) > 0.15;

        if is_beat {
            self.beat_times.push_back(self.time_elapsed);
            while self.beat_times.len() > 8 {
                self.beat_times.pop_front();
            }
            self.recompute_bpm();
        }

        // Phase advance — always runs so future effects have a
        // continuous phase even during silence.
        let period_s = if self.bpm > 20.0 { 60.0 / self.bpm } else { 0.5 };
        self.phase += dt / period_s;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }

        // Phase-lock nudge.
        if is_beat && s.bpm_lock_enabled {
            let gain = s.bpm_phase_smoothing.clamp(0.0, 1.0);
            let error = if self.phase < 0.5 {
                -self.phase
            } else {
                1.0 - self.phase
            };
            self.phase = (self.phase + error * gain).rem_euclid(1.0);
        }

        (is_beat, self.phase)
    }

    fn recompute_bpm(&mut self) {
        if self.beat_times.len() < 2 {
            self.bpm = 0.0;
            return;
        }
        let mut intervals: Vec<f64> = self
            .beat_times
            .iter()
            .zip(self.beat_times.iter().skip(1))
            .map(|(a, b)| b - a)
            .collect();
        intervals.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = intervals[intervals.len() / 2];
        if median > 0.001 {
            self.bpm = (60.0 / median) as f32;
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────

fn make_window(kind: WindowFunction, n: usize) -> Vec<f32> {
    let mut w = vec![0.0f32; n];
    let denom = n.saturating_sub(1).max(1) as f32;
    let two_pi = 2.0 * std::f32::consts::PI;
    for (i, x) in w.iter_mut().enumerate() {
        let t = i as f32 / denom;
        *x = match kind {
            WindowFunction::Hann => 0.5 * (1.0 - (two_pi * t).cos()),
            WindowFunction::Hamming => 0.54 - 0.46 * (two_pi * t).cos(),
            WindowFunction::BlackmanHarris => {
                0.35875 - 0.48829 * (two_pi * t).cos()
                    + 0.14128 * (4.0 * std::f32::consts::PI * t).cos()
                    - 0.01168 * (6.0 * std::f32::consts::PI * t).cos()
            }
            WindowFunction::FlatTop => {
                0.21557895 - 0.41663158 * (two_pi * t).cos()
                    + 0.277263158 * (4.0 * std::f32::consts::PI * t).cos()
                    - 0.083578947 * (6.0 * std::f32::consts::PI * t).cos()
                    + 0.006947368 * (8.0 * std::f32::consts::PI * t).cos()
            }
            WindowFunction::Rectangular => 1.0,
        };
    }
    w
}

fn make_bin_ranges(
    fft_size: usize,
    num_bars: usize,
    scaling: FrequencyScaling,
    _sample_rate: f32,
) -> Vec<(usize, usize)> {
    let nyquist = fft_size / 2;
    let mut ranges = Vec::with_capacity(num_bars);
    match scaling {
        FrequencyScaling::Linear => {
            for i in 0..num_bars {
                let s = i * nyquist / num_bars;
                let e = ((i + 1) * nyquist / num_bars).max(s + 1).min(nyquist);
                ranges.push((s.max(1), e.max(s.max(1) + 1).min(nyquist)));
            }
        }
        FrequencyScaling::Logarithmic => {
            let min_bin = 1.0f32;
            let max_bin = (nyquist as f32).max(2.0);
            for i in 0..num_bars {
                let lo = (min_bin
                    * (max_bin / min_bin).powf(i as f32 / num_bars as f32))
                    .floor() as usize;
                let hi = (min_bin
                    * (max_bin / min_bin).powf((i + 1) as f32 / num_bars as f32))
                    .ceil() as usize;
                let s = lo.max(1);
                let e = hi.max(s + 1).min(nyquist);
                ranges.push((s, e));
            }
        }
    }
    ranges
}
