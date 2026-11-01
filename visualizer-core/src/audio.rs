use std::sync::Arc;
use std::thread;
use crossbeam_channel::Receiver;
use wasapi::*;
use realfft::RealFftPlanner;

pub struct AudioEngine {
    pub device_name: String,
    pub data_rx: Receiver<Vec<f32>>,
}

impl AudioEngine {
    /// Starts a background thread that captures system audio via WASAPI loopback.
    /// Returns the engine and a receiver channel for the FFT data.
    pub fn new(num_bars: usize) -> Self {
        let (tx, rx) = crossbeam_channel::bounded(10);
        let (name_tx, name_rx) = crossbeam_channel::bounded(1);

        thread::spawn(move || {
            // 1. Initialize COM for this thread (Required for WASAPI)
            initialize_mta().ok().unwrap();

            // 2. Get the default audio render device
            let device = get_default_device(&Direction::Render).expect("Failed to get default audio device");
            let mut audio_client = device.get_iaudioclient().expect("Failed to get audio client");
            
            let device_name = device.get_friendlyname().unwrap_or_else(|_| "Default Device".to_string());
            let _ = name_tx.send(device_name);

            // 3. Configure Loopback Capture
            // We use the mix format of the device to ensure compatibility
            let format = audio_client.get_mixformat().expect("Failed to get mix format");
            let (_, min_time) = audio_client.get_periods().expect("Failed to get periods");

            audio_client.initialize_client(
                &format,
                min_time,
                &Direction::Capture,
                &ShareMode::Shared,
                true, // TRUE = Enable Loopback (System Audio)
            ).expect("Failed to initialize loopback client");

            let capture_client = audio_client.get_audiocaptureclient().expect("Failed to get capture client");
            audio_client.start_stream().expect("Failed to start audio stream");

            // 4. Setup FFT Processor
            let channels = format.get_nchannels() as usize;
            let mut fft_processor = FFTProcessor::new(1024, num_bars);

            // 5. Read Data Loop
            loop {
                // Wait for a packet
                let mut packet_size = 0;
                while packet_size == 0 {
                    packet_size = capture_client.get_next_packet_size().unwrap_or(0);
                    if packet_size == 0 {
                        thread::sleep(std::time::Duration::from_millis(5));
                    }
                }

                // Read the raw bytes
                let mut buffer = vec![0u8; packet_size as usize * (format.get_blockalign() as usize)];
                let (frames_read, _) = capture_client.read_from_device(&mut buffer).unwrap_or((0, 0));

                if frames_read > 0 {
                    // Convert bytes to f32 (WASAPI loopback usually provides 32-bit float)
                    let samples: &[f32] = bytemuck_cast_slice(&buffer);
                    
                    // Downmix to Mono (average all channels) for the visualizer
                    let mut mono_samples = Vec::with_capacity(samples.len() / channels);
                    for chunk in samples.chunks_exact(channels) {
                        let sum: f32 = chunk.iter().sum();
                        mono_samples.push(sum / channels as f32);
                    }

                    // Process FFT and send to UI
                    if let Some(bars) = fft_processor.process(&mono_samples) {
                        let _ = tx.try_send(bars);
                    }
                }
            }
        });

        let device_name = name_rx.recv().unwrap_or_else(|_| "Unknown".to_string());

        Self {
            device_name,
            data_rx: rx,
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
    fft: Arc<dyn realfft::RealToComplex<f32>>,
    input_buffer: Vec<f32>,
    output_buffer: Vec<realfft::num_complex::Complex32>,
    scratch_buffer: Vec<realfft::num_complex::Complex32>,
    num_bars: usize,
}

impl FFTProcessor {
    pub fn new(fft_size: usize, num_bars: usize) -> Self {
        let mut planner = RealFftPlanner::new();
        let fft = planner.plan_fft_forward(fft_size);
        
        let input_buffer = fft.make_input_vec();
        let output_buffer = fft.make_output_vec();
        let scratch_buffer = fft.make_scratch_vec();

        Self { fft, input_buffer, output_buffer, scratch_buffer, num_bars }
    }

    /// Takes mono audio samples, runs FFT, and maps to logarithmic visualizer bars.
    pub fn process(&mut self, samples: &[f32]) -> Option<Vec<f32>> {
        // We only process a chunk of size `fft_size`. If less, we zero-pad.
        let fft_size = self.input_buffer.len();
        let len = samples.len().min(fft_size);
        
        // Apply Hann Window to reduce spectral leakage
        for i in 0..len {
            let multiplier = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / fft_size as f32).cos());
            self.input_buffer[i] = samples[i] * multiplier;
        }
        for i in len..fft_size {
            self.input_buffer[i] = 0.0;
        }

        // Execute FFT
        if self.fft.process_with_scratch(&mut self.input_buffer, &mut self.output_buffer, &mut self.scratch_buffer).is_err() {
            return None;
        }

        // Map FFT bins to visualizer bars (Logarithmic scale)
        let mut bars = vec![0.0; self.num_bars];
        let nyquist = fft_size / 2;
        
        // We ignore the very low frequencies (bass rumble) and very high
        let min_bin = 1;
        let max_bin = nyquist;

        for (i, bar) in bars.iter_mut().enumerate() {
            // Logarithmic mapping
            let low = min_bin as f32 * ((max_bin as f32 / min_bin as f32).powf(i as f32 / self.num_bars as f32));
            let high = min_bin as f32 * ((max_bin as f32 / min_bin as f32).powf((i + 1) as f32 / self.num_bars as f32));
            
            let mut magnitude = 0.0;
            let mut count = 0;

            for bin in (low as usize)..(high as usize).min(nyquist) {
                magnitude += self.output_buffer[bin].norm();
                count += 1;
            }

            if count > 0 {
                magnitude /= count as f32;
            }

            // Convert to decibels and normalize
            let db = 20.0 * (magnitude + 1e-6).log10();
            // Map -80dB to 0dB into 0.0 -> 1.0 range
            let normalized = ((db + 80.0) / 80.0).clamp(0.0, 1.0);
            
            // Apply a slight curve to make the bars look more dynamic
            *bar = normalized.powf(1.5); 
        }

        Some(bars)
    }
}
