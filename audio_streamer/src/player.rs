use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat, SizedSample};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use crate::Result;

pub struct AudioPlayer {
    host: cpal::Host,
}

impl AudioPlayer {
    pub fn new() -> Result<Self> {
        let host = cpal::default_host();
        Ok(Self { host })
    }

    pub fn start_playback(&self) -> Result<(mpsc::Sender<Vec<f32>>, cpal::Stream)> {
        let device = self.host.default_output_device().ok_or_else(|| {
            crate::AudioStreamerError::DeviceError("No output device found".into())
        })?;

        log::info!("Starting audio playback on device: {}", device.name()?);

        // Use the lowest possible buffer size for minimum latency
        let config = cpal::StreamConfig {
            channels: 2,
            sample_rate: cpal::SampleRate(48000),
            buffer_size: cpal::BufferSize::Default, // Let the system choose the lowest safe value
        };

        log::info!("Using output config: {:?}", config);

        let (tx, rx) = mpsc::channel(32);
        let rx = Arc::new(Mutex::new(Some(rx)));

        let err_fn = |err| log::error!("Playback error: {}", err);

        let stream = match device.default_output_config()?.sample_format() {
            SampleFormat::F32 => {
                self.build_output_stream::<f32>(&device, &config, rx.clone(), err_fn)?
            }
            SampleFormat::I16 => {
                self.build_output_stream::<i16>(&device, &config, rx.clone(), err_fn)?
            }
            SampleFormat::U16 => {
                self.build_output_stream::<u16>(&device, &config, rx.clone(), err_fn)?
            }
            _ => {
                return Err(crate::AudioStreamerError::DeviceError(
                    "Unsupported sample format".into(),
                ))
            }
        };

        stream.play()?;
        Ok((tx, stream))
    }

    fn build_output_stream<T>(
        &self,
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        rx: Arc<Mutex<Option<mpsc::Receiver<Vec<f32>>>>>,
        error_fn: impl FnMut(cpal::StreamError) + Send + 'static + 'static,
    ) -> Result<cpal::Stream>
    where
        T: Sample + SizedSample + cpal::FromSample<f32>,
    {
        let stream = device.build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                // Try to get new samples without blocking
                let mut rx_lock = rx.lock().unwrap();
                if let Some(rx) = rx_lock.as_mut() {
                    if let Ok(samples) = rx.try_recv() {
                        // We have new samples, play them
                        for (i, &sample) in samples.iter().take(data.len()).enumerate() {
                            data[i] = T::from_sample(sample);
                        }
                        // Fill any remaining space with silence
                        for sample in data.iter_mut().skip(samples.len()) {
                            *sample = T::from_sample(0.0f32);
                        }
                        return;
                    }
                }

                // If we couldn't get new samples, output silence
                for sample in data.iter_mut() {
                    *sample = T::from_sample(0.0f32);
                }
            },
            error_fn,
            None,
        )?;

        Ok(stream)
    }
}
