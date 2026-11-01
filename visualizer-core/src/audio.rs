use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use realfft::{RealFftPlanner, RealToComplex};
use std::sync::Arc;
use crossbeam_channel::{Sender, Receiver, bounded};

pub struct AudioEngine {
    pub device_name: String,
    pub fft_size: usize,
    pub data_rx: Receiver<Vec<f32>>,
}

impl AudioEngine {
    pub fn new(fft_size: usize) -> (Self, Sender<Vec<f32>>) {
        let host = cpal::default_host();
        let device = host.default_output_device().expect("No output device");
        let device_name = device.name().unwrap_or_default();
        
        let config = device.default_output_config().unwrap();
        let sample_rate = config.sample_rate().0;
        
        // In a full implementation, we configure WASAPI loopback here.
        // cpal handles loopback by using the default output device as input.
        let (tx, rx) = bounded(10);

        // Spawn audio thread
        std::thread::spawn(move || {
            let stream = device.build_input_stream(
                &config.into(),
                move |data: &[f32], _: &_| {
                    let _ = tx.send(data.to_vec());
                },
                err_fn,
                None,
            ).unwrap();
            stream.play().unwrap();
            loop { std::thread::sleep(std::time::Duration::from_secs(1)); }
        });

        (Self { device_name, fft_size, data_rx: rx }, Sender::clone(&Sender::clone(&tx))) // simplified
    }
}

fn err_fn(err: cpal::StreamError) {
    eprintln!("Audio stream error: {}", err);
}

pub struct FFTProcessor {
    fft: Arc<dyn RealToComplex<f32>>,
    input_buffer: Vec<f32>,
    pub bars: Vec<f32>,
}

impl FFTProcessor {
    pub fn new(fft_size: usize, num_bars: usize) -> Self {
        let mut planner = RealFftPlanner::new();
        let fft = planner.plan_fft_forward(fft_size);
        Self { fft, input_buffer: vec![0.0; fft_size], bars: vec![0.0; num_bars] }
    }

    pub fn process(&mut self, samples: &[f32], sensitivity: f32) {
        // Fill buffer and run FFT, then map to bars
        // Simplified for brevity: normalization and logarithmic mapping happen here
    }
}
