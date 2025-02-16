use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Host, Sample, SampleFormat, SizedSample};
use std::sync::Arc;
use tokio::sync::mpsc;

#[cfg(target_os = "macos")]
use {
    core_media_rs::cm_sample_buffer::CMSampleBuffer,
    screencapturekit::{
        shareable_content::SCShareableContent,
        stream::{
            configuration::SCStreamConfiguration, content_filter::SCContentFilter,
            output_trait::SCStreamOutputTrait, output_type::SCStreamOutputType, SCStream,
        },
    },
    std::sync::mpsc as std_mpsc,
};

use crate::Result;

#[derive(Debug)]
pub enum DeviceType {
    Physical,
    Virtual,
    SystemAudio,
}

#[derive(Debug)]
pub struct DeviceInfo {
    pub name: String,
    pub is_default: bool,
    pub index: usize,
    pub device_type: DeviceType,
}

pub struct AudioCapture {
    host: Host,
    config: CaptureConfig,
    #[cfg(target_os = "macos")]
    screen_capture: Option<SCStream>,
}

#[derive(Clone, Debug)]
pub struct CaptureConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub buffer_size: u32,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            channels: 2,
            buffer_size: 480, // 10ms buffer at 48kHz (reduced from 4096)
        }
    }
}

#[cfg(target_os = "macos")]
struct AudioStreamOutput {
    sender: std_mpsc::Sender<CMSampleBuffer>,
}

#[cfg(target_os = "macos")]
impl SCStreamOutputTrait for AudioStreamOutput {
    fn did_output_sample_buffer(
        &self,
        sample_buffer: CMSampleBuffer,
        _of_type: SCStreamOutputType,
    ) {
        self.sender
            .send(sample_buffer)
            .expect("could not send to output_buffer");
    }
}

impl AudioCapture {
    pub fn new() -> Result<Self> {
        let host = cpal::default_host();
        Ok(Self {
            host,
            config: CaptureConfig::default(),
            #[cfg(target_os = "macos")]
            screen_capture: None,
        })
    }

    pub fn with_config(config: CaptureConfig) -> Result<Self> {
        let host = cpal::default_host();
        Ok(Self {
            host,
            config,
            #[cfg(target_os = "macos")]
            screen_capture: None,
        })
    }

    fn is_virtual_device(name: &str) -> bool {
        let virtual_device_keywords = [
            "BlackHole",
            "Soundflower",
            "VB-CABLE",
            "CABLE Output",
            "Virtual Audio Cable",
        ];
        virtual_device_keywords
            .iter()
            .any(|keyword| name.contains(keyword))
    }

    pub fn list_input_devices(&self) -> Result<Vec<DeviceInfo>> {
        let mut devices = Vec::new();
        let default_device = self.host.default_input_device();

        // Add system audio capture option first on supported platforms
        #[cfg(any(windows, target_os = "macos"))]
        {
            devices.push(DeviceInfo {
                #[cfg(windows)]
                name: "System Audio (Windows)".to_string(),
                #[cfg(target_os = "macos")]
                name: if self.screen_capture.is_some() {
                    "System Audio (macOS)".to_string()
                } else {
                    "System Audio (requires Screen Recording permission)".to_string()
                },
                is_default: false,
                index: 0,
                device_type: DeviceType::SystemAudio,
            });
        }

        // Then add all physical and virtual devices
        for (index, device) in self.host.input_devices()?.enumerate() {
            let name = device
                .name()
                .unwrap_or_else(|_| "Unknown Device".to_string());

            let device_type = if Self::is_virtual_device(&name) {
                DeviceType::Virtual
            } else {
                DeviceType::Physical
            };

            let is_default = default_device
                .as_ref()
                .map(|d| d.name().map(|n| n == name).unwrap_or(false))
                .unwrap_or(false);

            devices.push(DeviceInfo {
                name,
                is_default,
                index: index
                    + if cfg!(any(windows, target_os = "macos")) {
                        1
                    } else {
                        0
                    },
                device_type,
            });
        }

        // Add virtual device hint if none found and not on Windows/macOS
        #[cfg(not(any(windows, target_os = "macos")))]
        if !devices
            .iter()
            .any(|d| matches!(d.device_type, DeviceType::Virtual))
        {
            devices.push(DeviceInfo {
                name: "System Audio (requires BlackHole/Soundflower installation)".to_string(),
                is_default: false,
                index: devices.len(),
                device_type: DeviceType::Virtual,
            });
        }

        Ok(devices)
    }

    pub fn start_capture_with_device(
        &self,
        device_index: usize,
    ) -> Result<(
        mpsc::Sender<Vec<f32>>,
        mpsc::Receiver<Vec<f32>>,
        cpal::Stream,
    )> {
        #[cfg(windows)]
        if device_index == 0 {
            return self.start_wasapi_loopback();
        }

        #[cfg(target_os = "macos")]
        if device_index == 0 {
            return self.start_screen_capture();
        }

        let mut devices = self.host.input_devices()?;
        let adjusted_index = if cfg!(any(windows, target_os = "macos")) {
            device_index - 1
        } else {
            device_index
        };

        let device = devices.nth(adjusted_index).ok_or_else(|| {
            crate::AudioStreamerError::DeviceError("Selected device not found".into())
        })?;

        let config = device.default_input_config()?;
        let (tx, rx) = mpsc::channel(32);
        let tx = Arc::new(tx);

        let err_fn = |err| eprintln!("An error occurred on the audio stream: {}", err);

        let stream = match config.sample_format() {
            SampleFormat::F32 => {
                self.build_stream::<f32>(&device, &config.into(), tx.clone(), err_fn)?
            }
            SampleFormat::I16 => {
                self.build_stream::<i16>(&device, &config.into(), tx.clone(), err_fn)?
            }
            SampleFormat::U16 => {
                self.build_stream::<u16>(&device, &config.into(), tx.clone(), err_fn)?
            }
            _ => {
                return Err(crate::AudioStreamerError::DeviceError(
                    "Unsupported sample format".into(),
                ))
            }
        };

        stream.play()?;
        Ok((tx.as_ref().clone(), rx, stream))
    }

    #[cfg(target_os = "macos")]
    fn start_screen_capture(
        &self,
    ) -> Result<(
        mpsc::Sender<Vec<f32>>,
        mpsc::Receiver<Vec<f32>>,
        cpal::Stream,
    )> {
        let (tx, rx) = mpsc::channel(32);
        let tx = Arc::new(tx);
        let tx_clone = tx.clone();

        // Set up the screen capture
        let (std_tx, std_rx) = std_mpsc::channel();
        let stream = unsafe {
            self.get_screen_capture_stream(std_tx)
                .map_err(|e| crate::AudioStreamerError::DeviceError(e.to_string()))?
        };

        // Start a thread to process audio samples
        std::thread::spawn(move || {
            while let Ok(sample) = std_rx.recv() {
                let buffer_list = match sample.get_audio_buffer_list() {
                    Ok(list) => list,
                    Err(_) => continue,
                };

                for buffer_index in 0..buffer_list.num_buffers() {
                    let buffer = match buffer_list.get(buffer_index) {
                        Some(buf) => buf,
                        None => continue,
                    };

                    // Convert raw audio data to f32 samples
                    let samples: Vec<f32> = buffer
                        .data()
                        .chunks_exact(4)
                        .map(|chunk| {
                            let mut bytes = [0u8; 4];
                            bytes.copy_from_slice(chunk);
                            f32::from_le_bytes(bytes)
                        })
                        .collect();

                    let _ = tx_clone.blocking_send(samples);
                }
            }
        });

        // Start the capture
        stream
            .start_capture()
            .map_err(|e| crate::AudioStreamerError::DeviceError(e.to_string()))?;

        // Create a dummy CPAL stream to match the API
        let device = self.host.default_output_device().ok_or_else(|| {
            crate::AudioStreamerError::DeviceError("No output device found".into())
        })?;
        let config = device.default_output_config()?;
        let dummy_stream = device.build_output_stream(
            &config.into(),
            move |_data: &mut [f32], _: &cpal::OutputCallbackInfo| {},
            |err| eprintln!("Stream error: {}", err),
            None,
        )?;

        Ok((tx.as_ref().clone(), rx, dummy_stream))
    }

    #[cfg(target_os = "macos")]
    unsafe fn get_screen_capture_stream(
        &self,
        tx: std_mpsc::Sender<CMSampleBuffer>,
    ) -> Result<SCStream> {
        let config = SCStreamConfiguration::new()
            .set_captures_audio(true)
            .map_err(|e| crate::AudioStreamerError::DeviceError(e.to_string()))?;

        let content = SCShareableContent::get()
            .map_err(|e| crate::AudioStreamerError::DeviceError(e.to_string()))?;

        let displays = content.displays();
        let display = displays
            .first()
            .ok_or_else(|| crate::AudioStreamerError::DeviceError("No display found".into()))?;

        let filter = SCContentFilter::new().with_display_excluding_windows(display, &[]);
        let mut stream = SCStream::new(&filter, &config);
        stream.add_output_handler(AudioStreamOutput { sender: tx }, SCStreamOutputType::Audio);
        Ok(stream)
    }

    #[cfg(windows)]
    fn build_loopback_stream<T>(
        &self,
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        tx: Arc<mpsc::Sender<Vec<f32>>>,
        error_fn: impl FnMut(cpal::StreamError) + Send + 'static,
    ) -> Result<cpal::Stream>
    where
        T: Sample + SizedSample + Send + Sync + 'static,
        f32: cpal::FromSample<T>,
    {
        let mut samples_buffer = Vec::with_capacity(self.config.buffer_size as usize);
        let buffer_size = self.config.buffer_size;

        log::info!(
            "Starting Windows loopback capture with config: {:?}",
            config
        );

        // Use WASAPI loopback mode for system audio capture
        let stream = device.build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                let mut new_samples = Vec::with_capacity(data.len());
                for &sample in data.iter() {
                    new_samples.push(f32::from_sample(sample));
                }

                samples_buffer.extend(new_samples.drain(..));

                if samples_buffer.len() >= buffer_size as usize {
                    let buffer_to_send = samples_buffer
                        .drain(..buffer_size as usize)
                        .collect::<Vec<f32>>();

                    // Enhanced logging for audio data
                    let max_amplitude = buffer_to_send
                        .iter()
                        .fold(0.0f32, |max, &x| max.max(x.abs()));
                    
                    let rms = (buffer_to_send.iter()
                        .map(|&x| x * x)
                        .sum::<f32>() / buffer_to_send.len() as f32)
                        .sqrt();
                        
                    if max_amplitude > 0.01 {
                        log::debug!(
                            "Captured audio data - Max amplitude: {:.3}, RMS: {:.3}, Buffer size: {}",
                            max_amplitude,
                            rms,
                            buffer_to_send.len()
                        );
                    } else {
                        log::trace!(
                            "Low/no audio signal - Max amplitude: {:.3}, RMS: {:.3}",
                            max_amplitude,
                            rms
                        );
                    }

                    if let Err(e) = tx.blocking_send(buffer_to_send) {
                        log::error!("Failed to send captured audio data: {}", e);
                    }
                }
            },
            error_fn,
            None,
        )?;

        Ok(stream)
    }

    #[cfg(windows)]
    fn start_wasapi_loopback(
        &self,
    ) -> Result<(
        mpsc::Sender<Vec<f32>>,
        mpsc::Receiver<Vec<f32>>,
        cpal::Stream,
    )> {
        use cpal::traits::HostTrait;

        let device = self.host.default_output_device().ok_or_else(|| {
            crate::AudioStreamerError::DeviceError("No output device found".into())
        })?;

        log::info!("Starting WASAPI loopback capture on device: {}", device.name()?);
        
        let config = device.default_output_config()?;
        log::info!("Using WASAPI config: {:?}", config);
        
        let (tx, rx) = mpsc::channel(32);
        let tx: Arc<mpsc::Sender<Vec<f32>>> = Arc::new(tx);

        let err_fn = |err| log::error!("WASAPI stream error: {}", err);

        let stream = match config.sample_format() {
            SampleFormat::F32 => {
                self.build_loopback_stream::<f32>(&device, &config.into(), tx.clone(), err_fn)?
            }
            SampleFormat::I16 => {
                self.build_loopback_stream::<i16>(&device, &config.into(), tx.clone(), err_fn)?
            }
            SampleFormat::U16 => {
                self.build_loopback_stream::<u16>(&device, &config.into(), tx.clone(), err_fn)?
            }
            _ => {
                return Err(crate::AudioStreamerError::DeviceError(
                    "Unsupported sample format".into(),
                ))
            }
        };

        stream.play()?;
        Ok((tx.as_ref().clone(), rx, stream))
    }

    fn build_stream<T>(
        &self,
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        tx: Arc<mpsc::Sender<Vec<f32>>>,
        error_fn: impl FnMut(cpal::StreamError) + Send + 'static,
    ) -> Result<cpal::Stream>
    where
        T: Sample + SizedSample + Send + Sync + 'static,
        f32: cpal::FromSample<T>,
    {
        let mut samples_buffer = Vec::with_capacity(self.config.buffer_size as usize);
        let buffer_size = self.config.buffer_size;

        let stream = device.build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                let mut new_samples = Vec::with_capacity(data.len());
                for &sample in data.iter() {
                    new_samples.push(f32::from_sample(sample));
                }

                samples_buffer.extend(new_samples.drain(..));

                if samples_buffer.len() >= buffer_size as usize {
                    let buffer_to_send = samples_buffer
                        .drain(..buffer_size as usize)
                        .collect::<Vec<f32>>();
                    let _ = tx.blocking_send(buffer_to_send);
                }
            },
            error_fn,
            None,
        )?;

        Ok(stream)
    }

    // Keep the old method for backward compatibility, using default device
    pub fn start_capture(
        &self,
    ) -> Result<(
        mpsc::Sender<Vec<f32>>,
        mpsc::Receiver<Vec<f32>>,
        cpal::Stream,
    )> {
        let devices = self.list_input_devices()?;
        let default_index = devices.iter().position(|d| d.is_default).unwrap_or(0);
        self.start_capture_with_device(default_index)
    }
}
