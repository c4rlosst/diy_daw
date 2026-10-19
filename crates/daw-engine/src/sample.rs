//! Decoded audio samples, file decoding, and resolving sample paths stored in projects.

use std::fs::File;
use std::path::{Path, PathBuf};

use daw_model::SampleRef;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

#[derive(Debug, thiserror::Error)]
pub enum SampleError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("decode error: {0}")]
    Decode(#[from] SymphoniaError),
    #[error("no audio track")]
    NoAudioTrack,
    #[error("file contains no audio")]
    Empty,
}

/// Fully decoded audio, kept as interleaved `f32` at the file's own sample rate.
#[derive(Clone, PartialEq)]
pub struct Sample {
    sample_rate: u32,
    channels: usize,
    data: Vec<f32>,
}

impl std::fmt::Debug for Sample {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Sample {{ {} Hz, {} ch, {} frames }}",
            self.sample_rate,
            self.channels,
            self.frames()
        )
    }
}

impl Sample {
    pub fn from_interleaved(sample_rate: u32, channels: usize, data: Vec<f32>) -> Sample {
        assert!(channels > 0, "a sample needs at least one channel");
        let whole = data.len() - data.len() % channels;
        let mut data = data;
        data.truncate(whole);
        Sample {
            sample_rate,
            channels,
            data,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    pub fn channels(&self) -> usize {
        self.channels
    }
    pub fn frames(&self) -> usize {
        self.data.len() / self.channels
    }
    pub fn duration_seconds(&self) -> f64 {
        self.frames() as f64 / f64::from(self.sample_rate)
    }

    /// Stereo frame at a fractional frame position, using 4-point Catmull-Rom interpolation.
    /// Positions outside the sample read as silence. Mono is duplicated to both sides and
    /// channels beyond the second are ignored.
    pub fn frame(&self, position: f64) -> [f32; 2] {
        if !(position > -1.0 && position < self.frames() as f64) {
            return [0.0, 0.0];
        }
        let base = position.floor();
        let t = (position - base) as f32;
        let i = base as isize;
        let right = usize::from(self.channels > 1);
        let mut out = [0.0; 2];
        for (slot, ch) in out.iter_mut().zip([0, right]) {
            let p0 = self.at(i - 1, ch);
            let p1 = self.at(i, ch);
            let p2 = self.at(i + 1, ch);
            let p3 = self.at(i + 2, ch);
            *slot = p1
                + 0.5
                    * t
                    * (p2 - p0
                        + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3
                            + t * (3.0 * (p1 - p2) + p3 - p0)));
        }
        out
    }

    fn at(&self, frame: isize, channel: usize) -> f32 {
        if frame < 0 || frame as usize >= self.frames() {
            0.0
        } else {
            self.data[frame as usize * self.channels + channel]
        }
    }

    /// Decodes an audio file (WAV, AIFF, Ogg Vorbis, FLAC, MP3).
    pub fn decode_file(path: &Path) -> Result<Sample, SampleError> {
        let mss = MediaSourceStream::new(Box::new(File::open(path)?), Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
        let mut format = symphonia::default::get_probe().probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )?;
        let (track_id, mut decoder) = {
            let track = format
                .default_track(TrackType::Audio)
                .ok_or(SampleError::NoAudioTrack)?;
            let params = track
                .codec_params
                .as_ref()
                .and_then(|p| p.audio())
                .ok_or(SampleError::NoAudioTrack)?;
            let decoder = symphonia::default::get_codecs()
                .make_audio_decoder(params, &AudioDecoderOptions::default())?;
            (track.id, decoder)
        };

        let mut data: Vec<f32> = Vec::new();
        let mut scratch: Vec<f32> = Vec::new();
        let mut layout: Option<(u32, usize)> = None;
        loop {
            let packet = match format.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => break,
                Err(SymphoniaError::IoError(e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    break
                }
                Err(e) => return Err(e.into()),
            };
            if packet.track_id != track_id {
                continue;
            }
            match decoder.decode(&packet) {
                Ok(buf) => {
                    let spec = buf.spec();
                    layout.get_or_insert((spec.rate(), spec.channels().count()));
                    scratch.resize(buf.samples_interleaved(), 0.0);
                    buf.copy_to_slice_interleaved(&mut scratch);
                    data.extend_from_slice(&scratch);
                }
                Err(SymphoniaError::DecodeError(_)) => {} // skip a corrupt packet
                Err(e) => return Err(e.into()),
            }
        }
        match layout {
            Some((rate, channels)) if channels > 0 && !data.is_empty() => {
                Ok(Sample::from_interleaved(rate, channels, data))
            }
            _ => Err(SampleError::Empty),
        }
    }
}

/// Finds the audio file a project refers to by trying a list of root folders in order.
#[derive(Debug, Clone, Default)]
pub struct SampleResolver {
    roots: Vec<PathBuf>,
}

impl SampleResolver {
    pub fn new() -> SampleResolver {
        SampleResolver::default()
    }

    /// Adds a folder that sample paths may be relative to (a library, or the project folder).
    pub fn with_root(mut self, root: impl Into<PathBuf>) -> SampleResolver {
        self.roots.push(root.into());
        self
    }

    pub fn resolve(&self, sample: &SampleRef) -> Option<PathBuf> {
        if sample.path.is_empty() {
            return None;
        }
        let relative = Path::new(&sample.path);
        if relative.is_absolute() && relative.is_file() {
            return Some(relative.to_path_buf());
        }
        self.roots
            .iter()
            .map(|root| root.join(relative))
            .find(|p| p.is_file())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp() -> Sample {
        Sample::from_interleaved(48_000, 1, (0..100).map(|i| i as f32 / 100.0).collect())
    }

    #[test]
    fn integer_positions_return_exact_samples_and_outside_is_silent() {
        let s = ramp();
        assert_eq!(s.frame(10.0), [0.10, 0.10]);
        assert_eq!(s.frame(-3.0), [0.0, 0.0]);
        assert_eq!(s.frame(150.0), [0.0, 0.0]);
    }

    #[test]
    fn interpolation_is_linear_on_a_ramp() {
        let s = ramp();
        let [l, _] = s.frame(40.25);
        assert!((l - 0.4025).abs() < 1e-5, "{l}");
    }

    #[test]
    fn stereo_channels_are_kept_apart() {
        let s = Sample::from_interleaved(44_100, 2, vec![0.1, -0.1, 0.2, -0.2, 0.3, -0.3]);
        assert_eq!(s.frames(), 3);
        assert_eq!(s.frame(1.0), [0.2, -0.2]);
    }

    #[test]
    fn resolver_tries_roots_in_order() {
        let dir = std::env::temp_dir().join(format!("daw-resolver-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Samples")).unwrap();
        std::fs::write(dir.join("Samples/a.wav"), b"x").unwrap();
        let resolver = SampleResolver::new()
            .with_root(dir.join("nowhere"))
            .with_root(&dir);
        let found = resolver.resolve(&SampleRef {
            path: "Samples/a.wav".into(),
            pack: String::new(),
        });
        assert_eq!(found, Some(dir.join("Samples/a.wav")));
        assert_eq!(
            resolver.resolve(&SampleRef {
                path: "Samples/missing.wav".into(),
                pack: String::new()
            }),
            None
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wav_round_trip_through_the_decoder() {
        let samples: Vec<f32> = (0..2_000)
            .flat_map(|i| {
                let x = (i as f32 * 0.05).sin() * 0.5;
                [x, -x]
            })
            .collect();
        let path = std::env::temp_dir().join(format!("daw-roundtrip-{}.wav", std::process::id()));
        crate::wav::write_wav_i16(&path, 44_100, 2, &samples).unwrap();
        let decoded = Sample::decode_file(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            (decoded.sample_rate(), decoded.channels(), decoded.frames()),
            (44_100, 2, 2_000)
        );
        let [l, r] = decoded.frame(1_000.0);
        assert!((l - samples[2_000]).abs() < 1e-3 && (r + l).abs() < 1e-3);
    }
}
