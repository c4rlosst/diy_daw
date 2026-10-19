//! Audio device output via cpal.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    Device, FromSample, OutputCallbackInfo, SampleFormat, SizedSample, Stream, StreamConfig,
};

use crate::engine::{Engine, EngineHandle};

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("no default audio output device")]
    NoDevice,
    #[error("unsupported sample format: {0}")]
    UnsupportedFormat(String),
    #[error("audio error: {0}")]
    Cpal(#[from] cpal::Error),
}

/// A running output stream. Audio stops when this is dropped.
pub struct AudioOutput {
    _stream: Stream,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
}

impl AudioOutput {
    /// Opens the default output device and starts an engine on it.
    /// The engine is silent and stopped until [`EngineHandle::play`] is called.
    pub fn start() -> Result<(AudioOutput, EngineHandle), AudioError> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or(AudioError::NoDevice)?;
        let supported = device.default_output_config()?;
        let format = supported.sample_format();
        let config: StreamConfig = supported.into();

        let (engine, handle) = Engine::new(config.sample_rate);
        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, &config, engine),
            SampleFormat::F64 => build::<f64>(&device, &config, engine),
            SampleFormat::I16 => build::<i16>(&device, &config, engine),
            SampleFormat::U16 => build::<u16>(&device, &config, engine),
            SampleFormat::I32 => build::<i32>(&device, &config, engine),
            other => return Err(AudioError::UnsupportedFormat(other.to_string())),
        }?;
        stream.play()?;

        let output = AudioOutput {
            _stream: stream,
            device_name: device
                .id()
                .map(|id| id.to_string())
                .unwrap_or_else(|_| "unknown".into()),
            sample_rate: config.sample_rate,
            channels: config.channels,
        };
        Ok((output, handle))
    }
}

fn build<T>(
    device: &Device,
    config: &StreamConfig,
    mut engine: Engine,
) -> Result<Stream, cpal::Error>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels);
    // Large enough that resizing for a typical callback never reallocates on the audio thread.
    let mut scratch: Vec<f32> = Vec::with_capacity(1 << 16);
    device.build_output_stream(
        *config,
        move |data: &mut [T], _: &OutputCallbackInfo| {
            scratch.resize(data.len(), 0.0);
            engine.render(&mut scratch, channels);
            for (out, sample) in data.iter_mut().zip(&scratch) {
                *out = T::from_sample(*sample);
            }
        },
        |err| eprintln!("audio stream error: {err}"),
        None,
    )
}
