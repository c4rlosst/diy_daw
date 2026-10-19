//! Pitch-preserving time-stretching of warped audio regions, using Signalsmith Stretch (MIT).
//!
//! The engine plays audio by position: for every output frame it knows where in the source
//! file the timeline position maps to. Plain resampling reads the file at that position, so
//! pitch follows speed. A [`StretchVoice`] instead feeds the source audio through a streaming
//! time-stretcher in small chunks, so the speed follows the tempo but the pitch does not.
//!
//! The stretcher delays its output by about 120 ms. To keep the audio on the timeline, the
//! voice feeds it source audio from *ahead* of the playhead by exactly that latency (see
//! `resync`), starting each stream with a pre-roll of the audio that precedes the start
//! position. Loop wraps and clip boundaries inside the feed are simply spliced into the input,
//! so they land on the right beat. The voice re-syncs (a brief cut) only when the playhead
//! jumps (seek, region change) or the playback rate changes by more than 15% (the latency in
//! beats changes with the rate).

use crate::region::AudioRegion;

/// Whether the stretcher was compiled in (the `stretch` cargo feature).
pub const AVAILABLE: bool = cfg!(feature = "stretch");

/// Output frames produced per call into the stretcher.
pub const CHUNK: usize = 128;

/// What a voice needs to know about the engine for each frame.
pub struct Feed<'a> {
    /// The track's regions, sorted by start.
    pub regions: &'a [AudioRegion],
    pub tempo: f64,
    pub beats_per_frame: f64,
    /// Song length, if the transport loops over it.
    pub wrap: Option<f64>,
}

#[cfg(feature = "stretch")]
pub use real::StretchVoice;
#[cfg(not(feature = "stretch"))]
pub use stub::StretchVoice;

#[cfg(not(feature = "stretch"))]
mod stub {
    use super::Feed;

    pub struct StretchVoice;

    impl StretchVoice {
        pub fn new(_sample_rate: u32) -> StretchVoice {
            StretchVoice
        }

        pub fn next_frame(&mut self, _feed: &Feed, _t: f64) -> [f32; 2] {
            [0.0, 0.0]
        }
    }
}

#[cfg(feature = "stretch")]
mod real {
    use ssstretch::Stretch;

    use super::{Feed, CHUNK};
    use crate::region::{region_at, AudioRegion};

    /// Largest number of input frames per chunk (16x speed).
    const MAX_INPUT: usize = 16 * CHUNK;
    /// Re-sync when the playback rate moves this far (relative) from the rate at the last sync.
    const DRIFT_RESYNC: f64 = 0.15;
    /// Output frames over which the playback rate is measured.
    const PROBE_FRAMES: f64 = 8.0;

    struct SendStretch(Stretch);

    // SAFETY: the C++ stretcher has no thread affinity and no shared state; a voice owns its
    // instance exclusively and is only ever used from one thread at a time.
    unsafe impl Send for SendStretch {}

    pub struct StretchVoice {
        stretch: SendStretch,
        sample_rate: f64,
        input: [Vec<f32>; 2],
        output: [[f32; CHUNK]; 2],
        /// Next unread frame of `output`; `CHUNK` means a new chunk is needed.
        cursor: usize,
        synced: bool,
        /// Timeline position of the next frame to hand out.
        expected_t: f64,
        /// Timeline position of the next input frame to feed (ahead of `expected_t`).
        feed_t: f64,
        /// Source frame index of the next input frame to feed.
        fed_pos: i64,
        /// Whether `fed_pos` follows the current region's mapping.
        anchored: bool,
        /// Source frames per output frame when the lead was last computed.
        sync_rate: f64,
        transpose: f32,
        input_latency: f64,
        output_latency: f64,
        preroll: usize,
        /// Scratch for the discarded output of the warm-up run.
        warmup: [Vec<f32>; 2],
    }

    fn wrap(t: f64, length: Option<f64>) -> f64 {
        match length {
            Some(l) if l > 0.0 && t >= l => t % l,
            _ => t,
        }
    }

    /// Source frames per output frame at timeline position `tf`. A probe that straddles a loop
    /// jump gives nonsense, so the probe before `tf` is tried too and the one closer to
    /// `reference` wins.
    fn rate_of(region: &AudioRegion, feed: &Feed, tf: f64, reference: f64) -> f64 {
        let step = PROBE_FRAMES * feed.beats_per_frame;
        let at = |t: f64| region.source_frames(t, feed.tempo);
        let ahead = ((at(tf + step) - at(tf)) / PROBE_FRAMES).abs();
        let behind = ((at(tf) - at(tf - step)) / PROBE_FRAMES).abs();
        let r = if (ahead / reference - 1.0).abs() <= (behind / reference - 1.0).abs() {
            ahead
        } else {
            behind
        };
        r.max(1e-3)
    }

    impl StretchVoice {
        pub fn new(sample_rate: u32) -> StretchVoice {
            let mut stretch = Stretch::with_seed(1);
            stretch.preset_default(2, sample_rate as f32);
            let preroll = (stretch.block_samples() + stretch.interval_samples()) as usize;
            let capacity = MAX_INPUT.max(preroll);
            StretchVoice {
                input_latency: f64::from(stretch.input_latency()),
                output_latency: f64::from(stretch.output_latency()),
                stretch: SendStretch(stretch),
                sample_rate: f64::from(sample_rate),
                input: [vec![0.0; capacity], vec![0.0; capacity]],
                warmup: [vec![0.0; preroll], vec![0.0; preroll]],
                output: [[0.0; CHUNK]; 2],
                cursor: CHUNK,
                synced: false,
                expected_t: 0.0,
                feed_t: 0.0,
                fed_pos: 0,
                anchored: false,
                sync_rate: 1.0,
                transpose: 1.0,
                preroll,
            }
        }

        /// The next stereo frame for timeline position `t`. Frames must be requested in order
        /// (one per output frame); a gap or jump in `t` restarts the stream.
        pub fn next_frame(&mut self, feed: &Feed, t: f64) -> [f32; 2] {
            if !self.synced || (t - self.expected_t).abs() > 2.0 * feed.beats_per_frame {
                self.resync(feed, t);
            }
            if self.cursor >= CHUNK {
                self.refill(feed, true);
            }
            let frame = [self.output[0][self.cursor], self.output[1][self.cursor]];
            self.cursor += 1;
            self.expected_t = wrap(self.expected_t + feed.beats_per_frame, feed.wrap);
            frame
        }

        /// Restarts the stream so that its next output frame is the audio at `t_out`.
        fn resync(&mut self, feed: &Feed, t_out: f64) {
            let mut rate = match region_at(feed.regions, t_out) {
                Some(i) => rate_of(&feed.regions[i], feed, t_out, self.sync_rate),
                None => self.sync_rate,
            };
            // Feed source audio from ahead of the playhead by the stretcher's total latency:
            // `input_latency` source frames, plus `output_latency` output frames' worth.
            let mut tf = t_out;
            for _ in 0..2 {
                let lead_frames = self.output_latency + self.input_latency / rate;
                tf = wrap(t_out + lead_frames * feed.beats_per_frame, feed.wrap);
                if let Some(i) = region_at(feed.regions, tf) {
                    rate = rate_of(&feed.regions[i], feed, tf, rate);
                }
            }

            self.stretch.0.reset();
            self.cursor = CHUNK;
            self.synced = true;
            self.expected_t = t_out;
            self.feed_t = tf;
            self.sync_rate = rate;
            self.anchored = false;

            let region = region_at(feed.regions, tf).map(|i| &feed.regions[i]);
            let Some((region, sample)) = region.and_then(|r| Some((r, r.sample.as_deref()?)))
            else {
                return;
            };
            self.set_transpose(region, sample.sample_rate());
            // Pre-roll: run the source audio just before the first frame to be fed through the
            // stretcher and throw the output away. (`seek` would only hand it the input; its
            // overlap-add would still start empty, so the first ~60 ms of real output would
            // ramp up from partial overlap and a clip starting here would sound late and weak.)
            // Processing as many output frames as input frames keeps the latency arithmetic of
            // `lead_frames` intact: the next output is the audio one window behind the anchor.
            let anchor = region.source_frames(tf, feed.tempo).round() as i64;
            let p = self.preroll;
            for j in 0..p {
                let f = sample.stereo_at(anchor - p as i64 + j as i64);
                self.input[0][j] = f[0];
                self.input[1][j] = f[1];
            }
            let inputs = [self.input[0].as_ptr(), self.input[1].as_ptr()];
            let mut outputs = [self.warmup[0].as_mut_ptr(), self.warmup[1].as_mut_ptr()];
            // SAFETY: the input buffers hold at least `p` initialised frames and each warm-up
            // buffer holds exactly `p` frames; the stretcher is configured for two channels.
            unsafe {
                self.stretch
                    .0
                    .process(&inputs, p as i32, &mut outputs, p as i32)
            };
            self.fed_pos = anchor;
            self.anchored = true;
        }

        /// Produces the next `CHUNK` output frames.
        fn refill(&mut self, feed: &Feed, allow_resync: bool) {
            let n = CHUNK as f64;
            let tf = self.feed_t;
            let region = region_at(feed.regions, tf).map(|i| &feed.regions[i]);
            let loaded = region.and_then(|r| Some((r, r.sample.as_deref()?)));

            let m = if let Some((region, sample)) = loaded {
                let rate = rate_of(region, feed, tf, self.sync_rate);
                if allow_resync && (rate / self.sync_rate - 1.0).abs() > DRIFT_RESYNC {
                    let t_out = self.expected_t;
                    self.resync(feed, t_out);
                    return self.refill(feed, false);
                }
                self.set_transpose(region, sample.sample_rate());

                let start = region.source_frames(tf, feed.tempo);
                let end = region
                    .source_frames(wrap(tf + n * feed.beats_per_frame, feed.wrap), feed.tempo);
                let nominal = rate * n;
                let ideal = end - self.fed_pos as f64;
                // A big mismatch means the source position jumped (loop wrap, new clip):
                // splice by continuing from the new position.
                let continuous =
                    self.anchored && (ideal - nominal).abs() <= (0.25 * nominal).max(16.0);
                let m = if continuous {
                    ideal.round()
                } else {
                    self.fed_pos = start.round() as i64;
                    self.anchored = true;
                    nominal.round()
                };
                let m = m.clamp(0.0, MAX_INPUT as f64) as usize;
                for j in 0..m {
                    let f = sample.stereo_at(self.fed_pos + j as i64);
                    self.input[0][j] = f[0];
                    self.input[1][j] = f[1];
                }
                m
            } else {
                // Nothing to play at this point of the feed: silence at the last known speed.
                self.anchored = false;
                let m = ((self.sync_rate * n).round().clamp(0.0, MAX_INPUT as f64)) as usize;
                self.input[0][..m].fill(0.0);
                self.input[1][..m].fill(0.0);
                m
            };

            let inputs = [self.input[0].as_ptr(), self.input[1].as_ptr()];
            let mut outputs = [self.output[0].as_mut_ptr(), self.output[1].as_mut_ptr()];
            // SAFETY: `input` buffers hold at least `m` frames and each output buffer holds
            // exactly `CHUNK` frames; the stretcher is configured for two channels.
            unsafe {
                self.stretch
                    .0
                    .process(&inputs, m as i32, &mut outputs, CHUNK as i32)
            };
            self.fed_pos += m as i64;
            self.feed_t = wrap(tf + n * feed.beats_per_frame, feed.wrap);
            self.cursor = 0;
        }

        /// Sets the stretcher's pitch factor: it undoes the sample-rate difference between the
        /// file and the output, and applies the clip's transposition.
        fn set_transpose(&mut self, region: &AudioRegion, source_rate: u32) {
            let factor = (f64::from(source_rate) / self.sample_rate
                * 2f64.powf(region.pitch_semitones / 12.0)) as f32;
            if (factor - self.transpose).abs() > 1e-6 {
                self.stretch.0.set_transpose_factor(factor, None);
                self.transpose = factor;
            }
        }
    }
}
