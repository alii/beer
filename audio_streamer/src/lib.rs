pub mod capture;
pub mod network;
pub mod player;

use cpal::StreamError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AudioStreamerError {
    #[error("Audio device error: {0}")]
    DeviceError(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Encoding error: {0}")]
    EncodingError(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Stream error: {0}")]
    StreamError(String),

    #[error("Stream config error: {0}")]
    StreamConfigError(String),

    #[error("Stream build error: {0}")]
    StreamBuildError(String),

    #[error("Address parse error: {0}")]
    AddressError(#[from] std::net::AddrParseError),
}

pub type Result<T> = std::result::Result<T, AudioStreamerError>;

// Convert CPAL errors to our error type
impl From<cpal::BuildStreamError> for AudioStreamerError {
    fn from(err: cpal::BuildStreamError) -> Self {
        AudioStreamerError::StreamBuildError(err.to_string())
    }
}

impl From<cpal::PlayStreamError> for AudioStreamerError {
    fn from(err: cpal::PlayStreamError) -> Self {
        AudioStreamerError::StreamError(err.to_string())
    }
}

impl From<cpal::DefaultStreamConfigError> for AudioStreamerError {
    fn from(err: cpal::DefaultStreamConfigError) -> Self {
        AudioStreamerError::StreamConfigError(err.to_string())
    }
}

impl From<cpal::SupportedStreamConfigsError> for AudioStreamerError {
    fn from(err: cpal::SupportedStreamConfigsError) -> Self {
        AudioStreamerError::StreamConfigError(err.to_string())
    }
}

impl From<StreamError> for AudioStreamerError {
    fn from(err: StreamError) -> Self {
        AudioStreamerError::StreamError(err.to_string())
    }
}

impl From<cpal::DevicesError> for AudioStreamerError {
    fn from(err: cpal::DevicesError) -> Self {
        AudioStreamerError::DeviceError(err.to_string())
    }
}

impl From<cpal::DeviceNameError> for AudioStreamerError {
    fn from(err: cpal::DeviceNameError) -> Self {
        AudioStreamerError::DeviceError(err.to_string())
    }
}

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
