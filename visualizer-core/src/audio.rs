use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use realfft::RealFftPlanner;
use wasapi::*;

pub struct AudioEngine {
    pub device_name: String,
    pub data_rx: Receiver<Vec<f32>>,
    pub target_bars: Arc<AtomicUsize>,
    pub update_interval_ms: Arc<AtomicU64>,
}

impl AudioEngine {
    pub fn new(initial_bars: usize, initial_interval_ms: u64) -> Self {
        // Small buffer: UI keeps only the newest frame, so we *want* to
        // drop stale data if the UI thread falls behind.
        let (tx, rx) = crossbeam_channel::bounded(2);
        let (name_tx, name_rx) = crossbeam_channel::bounded(1);
        let target_bars = Arc::new(AtomicUsize::new(initial_bars));
        let update_interval_ms = Arc::new(AtomicU64::new(initial_interval_ms));

        let target_bars_clone = target_bars.clone();
        let interval_clone = update_interval_ms.clone();

        thread::spawn(move || {
            // Non-panicking: harmless if COM was already initialized.
            let _ = initialize_mta();

            let device = get_default_device(&Direction::Render)
                .expect("Failed to get default audio device");
            let mut audio_client = device
                .get_iaudioclient()
                .expect("Failed to get audio client");

            let device_name = device
                .get_friendlyname()
                .unwrap_or_else(|_| "Default Device".to_string());
            let _ = name_tx.send(device_name);

            let format = audio_client
                .get_mixformat()
                .expect("Failed to get mix format");
            let (_, min_time) = audio_client.get_periods().expect("Failed to get periods");

            audio_client
                .initialize_client(
                    &format,
                    min_time,
                    &Direction::Capture,
                    &ShareMode::Shared,
                    true,
                )
                .expect("Failed to initialize loopback client");

            // Keep the event handle alive; it must outlive the loop.
            let _h_event = audio_client
                .set_get_eventhandle()
                .expect("Failed to set event handle");

            let capture_client = audio_client
                .get_audiocaptureclient()
                .expect("Failed to get capture client");

            audio_client
                .start_stream()
                .expect("Failed to start audio stream");

            let channels = format.get_nchannels() as usize;
            let blockalign = format.get_blockalign() as usize;

            // The visualizer assumes 32-bit float mix format (the same
            // assumption the original code implicitly made). Assert it
            // for clarity.
            debug_assert_eq!(
                blockalign % 4,
                0,
                "Expected 32-bit float samples; blockalign={}",
                blockalign
            );
            let samples_per_frame = (blockalign / 4).max(1); // = channels for f32

            let mut fft_processor = FFTProcessor::new(1024, target_bars_clone);

            // Reused across iterations; both are 4-byte aligned because
            // the element type is `f32`.
            let mut sample_buffer: Vec<f32> = Vec::new();
            let mut mono_samples: Vec<f32> = Vec::new();

            loop {
                let start_time = Instant::now();

                let mut packet_size = 0u32;
                while packet_size == 0 {
                    packet_size = capture_client
                        .get_next_nbr_frames()
                        .unwrap_or(None)
                        .unwrap_or(0);
                    if packet_size == 0 {
                        thread::sleep(Duration::from_millis(2));
                    }
                }

                // Reserve room for the whole packet as f32 samples.
                // `resize` preserves capacity, so this reallocates at most
                // once per distinct packet size.
                let needed_samples = packet_size as usize * samples_per_frame;
                sample_buffer.resize(needed_samples, 0.0);

                // Safe cast: `sample_buffer` is backed by `Vec<f32>`, so
                // its pointer is guaranteed to be 4-byte aligned. We expose
                // it as a byte slice purely because `read_from_device`
                // demands `&mut [u8]`.
                let byte_slice: &mut [u8] = unsafe {
                    std::slice::from_raw_parts_mut(
                        sample_buffer.as_mut_ptr() as *mut u8,
                        needed_samples * std::mem::size_of::<f32>(),
                    )
                };

                let (frames_read, _) = capture_client
                    .read_from_device(byte_slice)
                    .unwrap_or((0, BufferFlags::none()));

                if frames_read > 0 {
                    // Only consume *actually-read* frames — partial reads
                    // would otherwise include stale bytes from the
                    // previous, larger packet.
                    let valid_samples =
                        (frames_read as usize * samples_per_frame).min(sample_buffer.len());
                    let samples = &sample_buffer[..valid_samples];

                    mono_samples.clear();
                    mono_samples.reserve(valid_samples / channels);
                    for chunk in samples.chunks_exact(channels) {
                        let sum: f32 = chunk.iter().sum();
                        mono_samples.push(sum / channels as f32);
                    }

                    if !mono_samples.is_empty() {
                        if let Some(bars) = fft_processor.process(&mono_samples) {
                            // If the UI can't keep up, drop the oldest
                            // queued frame rather than buffering latency.
                            let _ = tx.try_send(bars);
                        }
                    }
                }

                // Throttle: sleep for the remainder of the interval.
                let elapsed = start_time.elapsed();
                let target_duration =
                    Duration::from_millis(interval_clone.load(Ordering::Relaxed));
                if elapsed < target_duration {
                    thread::sleep(target_duration - elapsed);
                }
            }
        });

        let device_name = name_rx.recv().unwrap_or_else(|_| "Unknown".to_string());

        Self {
            device_name,
            data_rx: rx,
            target_bars,
            update_interval_ms,
        }
    }
}

pub struct FFTProcessor {
    fft: Arc<dyn realfft::RealToComplex<f32>>,
    input_buffer: Vec<f32>,
    output_buffer: Vec<realfft::num_complex::Complex32>,
    scratch_buffer: Vec<realfft::num_complex::Complex32>,
    target_bars: Arc<AtomicUsize>,
}

impl FFTProcessor {
    pub fn new(fft_size: usize, target_bars: Arc<AtomicUsize>) -> Self {
        let mut planner = RealFftPlanner::new();
        let fft = planner.plan_fft_forward(fft_size);
        let input_buffer = fft.make_input_vec();
        let output_buffer = fft.make_output_vec();
        let scratch_buffer = fft.make_scratch_vec();

        Self {
            fft,
            input_buffer,
            output_buffer,
            scratch_buffer,
            target_bars,
        }
    }

    pub fn process(&mut self, samples: &[f32]) -> Option<Vec<f32>> {
        if samples.is_empty() {
            return None;
        }

        let fft_size = self.input_buffer.len();

        // ── Sliding window ────────────────────────────────────────────
        // Keep the last `fft_size` samples. Zero-padding (the previous
        // behaviour) leaves half the FFT buffer empty for typical
        // packet sizes, which broadens every spectral peak. Sliding
        // always hands the FFT a full, continuously-windowed block.
        if samples.len() >= fft_size {
            self.input_buffer
                .copy_from_slice(&samples[samples.len() - fft_size..]);
        } else {
            let keep = fft_size - samples.len();
            self.input_buffer.copy_within(samples.len().., 0);
            self.input_buffer[keep..].copy_from_slice(samples);
        }

        // Hann window — reduces edge discontinuities (which otherwise
        // leak energy into neighbouring bins).
        let two_pi = 2.0 * std::f32::consts::PI;
        for i in 0..fft_size {
            let w = 0.5 * (1.0 - (two_pi * i as f32 / fft_size as f32).cos());
            self.input_buffer[i] *= w;
        }

        if self
            .fft
            .process_with_scratch(
                &mut self.input_buffer,
                &mut self.output_buffer,
                &mut self.scratch_buffer,
            )
            .is_err()
        {
            return None;
        }

        let num_bars = self.target_bars.load(Ordering::Relaxed).max(1);
        let mut bars = vec![0.0; num_bars];
        let nyquist = fft_size / 2;

        let min_bin = 1;
        let max_bin = (fft_size / 3).max(min_bin + 1);

        // Precompute the log-frequency ratio once instead of per bar.
        let ratio = max_bin as f32 / min_bin as f32;
        let norm = 1.0 / (fft_size as f32 / 2.0);

        for (i, bar) in bars.iter_mut().enumerate() {
            let low =
                min_bin as f32 * ratio.powf(i as f32 / num_bars as f32).floor();
            let high =
                min_bin as f32 * ratio.powf((i + 1) as f32 / num_bars as f32).ceil();

            let start = low as usize;
            let end = (high as usize).max(start + 1);

            let mut magnitude = 0.0;
            let mut count = 0;

            for bin in start..end.min(nyquist) {
                magnitude += self.output_buffer[bin].norm();
                count += 1;
            }

            if count > 0 {
                magnitude /= count as f32;
            }

            let normalized_magnitude = magnitude * norm;
            let db = 20.0 * (normalized_magnitude + 1e-9).log10();

            // Map [-90 dB, -20 dB] to [0, 1], then square for a
            // perceptual (log-ish) response.
            let normalized = ((db + 90.0) / 70.0).clamp(0.0, 1.0);
            *bar = normalized * normalized; // cheaper than powf(2.0)
        }

        Some(bars)
    }
}
