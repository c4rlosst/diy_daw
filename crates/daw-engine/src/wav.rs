//! Minimal 16-bit PCM WAV writer, for offline rendering and tests.

use std::io::{self, Write};
use std::path::Path;

/// Writes interleaved `samples` (nominally -1.0..=1.0) as a 16-bit PCM WAV file.
pub fn write_wav_i16(
    path: impl AsRef<Path>,
    sample_rate: u32,
    channels: u16,
    samples: &[f32],
) -> io::Result<()> {
    let data_len = (samples.len() * 2) as u32;
    let block_align = channels * 2;
    let mut w = io::BufWriter::new(std::fs::File::create(path)?);
    w.write_all(b"RIFF")?;
    w.write_all(&(36 + data_len).to_le_bytes())?;
    w.write_all(b"WAVEfmt ")?;
    w.write_all(&16u32.to_le_bytes())?;
    w.write_all(&1u16.to_le_bytes())?; // PCM
    w.write_all(&channels.to_le_bytes())?;
    w.write_all(&sample_rate.to_le_bytes())?;
    w.write_all(&(sample_rate * u32::from(block_align)).to_le_bytes())?;
    w.write_all(&block_align.to_le_bytes())?;
    w.write_all(&16u16.to_le_bytes())?;
    w.write_all(b"data")?;
    w.write_all(&data_len.to_le_bytes())?;
    for s in samples {
        w.write_all(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())?;
    }
    w.flush()
}
