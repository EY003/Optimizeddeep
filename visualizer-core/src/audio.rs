use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use crossbeam_channel::Receiver;
use wasapi::*;
use realfft::RealFftPlanner;

pub struct AudioEngine {
    pub device_name: String,
    pub data_rx: Receiver<Vec<f32>>,
    pub target_bars: std::sync::Arc<AtomicUsize>,
}

impl AudioEngine {
    pub fn new(initial_bars: usize) -> Self {
        let (tx, rx) = crossbeam_channel::bounded(10);
        let (name_tx, name_rx) = crossbeam_channel::bounded(1);
        let target_bars = std::sync::Arc::new(AtomicUsize::new(initial_bars));
        let target_bars_clone = target_bars.clone();

        thread::spawn(move || {
            initialize_mta().ok().unwrap();
            // ... (keep existing device initialization) ...

            // Pass the atomic to the FFT Processor
            let mut fft_processor = FFTProcessor::new(1024, target_bars_clone);

            loop {
                // ... (keep existing packet reading logic) ...
                
                if frames_read > 0 {
                    let samples: &[f32] = bytemuck_cast_slice(&buffer);
                    let mut mono_samples = Vec::with_capacity(samples.len() / channels);
                    for chunk in samples.chunks_exact(channels) {
                        let sum: f32 = chunk.iter().sum();
                        mono_samples.push(sum / channels as f32);
                    }

                    if let Some(bars) = fft_processor.process(&mono_samples) {
                        let _ = tx.try_send(bars);
                    }
                }
            }
        });

        let device_name = name_rx.recv().unwrap_or_else(|_| "Unknown".to_string());

        Self { device_name, data_rx: rx, target_bars }
    }
}
}

/// Helper to safely cast &[u8] to &[f32]
fn bytemuck_cast_slice(bytes: &[u8]) -> &[f32] {
    // SAFETY: WASAPI provides aligned 32-bit float data. 
    // We check length to prevent out-of-bounds.
    if bytes.len() % 4 != 0 { return &[]; }
    unsafe {
        std::slice::from_raw_parts(bytes.as_ptr() as *const f32, bytes.len() / 4)
    }
}

/// Handles the Math: FFT -> Frequency Bins -> Visualizer Bars
pub struct FFTProcessor {
    fft: std::sync::Arc<dyn realfft::RealToComplex<f32>>,
    input_buffer: Vec<f32>,
    output_buffer: Vec<realfft::num_complex::Complex32>,
    scratch_buffer: Vec<realfft::num_complex::Complex32>,
    target_bars: std::sync::Arc<AtomicUsize>, // <-- Dynamic
}

impl FFTProcessor {
    pub fn new(fft_size: usize, target_bars: std::sync::Arc<AtomicUsize>) -> Self {
        let mut planner = RealFftPlanner::new();
        let fft = planner.plan_fft_forward(fft_size);
        let input_buffer = fft.make_input_vec();
        let output_buffer = fft.make_output_vec();
        let scratch_buffer = fft.make_scratch_vec();
        Self { fft, input_buffer, output_buffer, scratch_buffer, target_bars }
    }

    pub fn process(&mut self, samples: &[f32]) -> Option<Vec<f32>> {
        // ... (keep the zero-padding and windowing code) ...
        
        if self.fft.process_with_scratch(&mut self.input_buffer, &mut self.output_buffer, &mut self.scratch_buffer).is_err() {
            return None;
        }

        // ✅ Read the live target from the slider
        let num_bars = self.target_bars.load(Ordering::Relaxed).max(1);
        let mut bars = vec![0.0; num_bars];
        let nyquist = self.input_buffer.len() / 2;
        
        let min_bin = 1;
        let max_bin = (self.input_buffer.len() / 3).max(min_bin + 1);

        for (i, bar) in bars.iter_mut().enumerate() {
            let low = min_bin as f32 * ((max_bin as f32 / min_bin as f32).powf(i as f32 / num_bars as f32));
            let high = min_bin as f32 * ((max_bin as f32 / min_bin as f32).powf((i + 1) as f32 / num_bars as f32));
            
            let mut magnitude = 0.0;
            let mut count = 0;

            for bin in (low as usize)..(high as usize).min(nyquist) {
                magnitude += self.output_buffer[bin].norm();
                count += 1;
            }

            if count > 0 { magnitude /= count as f32; }

            let normalized_magnitude = magnitude / (self.input_buffer.len() as f32 / 2.0);
            let db = 20.0 * (normalized_magnitude + 1e-9).log10();
            let normalized = ((db + 80.0) / 80.0).clamp(0.0, 1.0);
            *bar = normalized.powf(1.5); 
        }

        Some(bars)
    }
}
