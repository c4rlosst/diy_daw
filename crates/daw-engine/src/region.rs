//! An audio clip placed on the timeline, and the mapping from timeline position to
//! position in the source audio.

use std::sync::Arc;

use daw_model::{ClipLoop, SampleRef, WarpMarker, WarpMode};

use crate::sample::Sample;

#[derive(Debug, Clone, PartialEq)]
pub struct AudioRegion {
    /// Timeline extent in beats. `end` may be infinite for looping session clips.
    pub start: f64,
    pub end: f64,
    pub sample_ref: SampleRef,
    /// Filled in by [`crate::Song::load_samples`]; regions without a sample are silent.
    pub sample: Option<Arc<Sample>>,
    pub warped: bool,
    /// Sorted by `beat_time`, with distinct beat times.
    pub markers: Vec<WarpMarker>,
    pub looping: ClipLoop,
    pub warp_mode: WarpMode,
    pub gain: f32,
    pub pitch_semitones: f64,
}

impl AudioRegion {
    /// Sorts warp markers and drops ones that would make the mapping degenerate.
    pub fn normalize_markers(markers: &[WarpMarker]) -> Vec<WarpMarker> {
        let mut sorted = markers.to_vec();
        sorted.sort_by(|a, b| a.beat_time.total_cmp(&b.beat_time));
        sorted.dedup_by(|b, a| (b.beat_time - a.beat_time).abs() < 1e-12);
        sorted
    }

    /// Position inside the clip's content at timeline position `t` (beats): sample beats for
    /// warped clips, seconds of source audio for unwarped ones. `tempo` is only used to turn
    /// elapsed beats into seconds for unwarped clips.
    pub fn content_position(&self, t: f64, tempo: f64) -> f64 {
        let elapsed = t - self.start;
        let x = if self.warped {
            elapsed
        } else {
            elapsed * 60.0 / tempo
        };
        let l = &self.looping;
        let start = l.start + l.start_relative;
        let len = l.end - l.start;
        if l.enabled && len > 0.0 {
            let first_pass = l.end - start;
            if x < first_pass {
                start + x
            } else {
                l.start + (x - first_pass) % len
            }
        } else {
            start + x
        }
    }

    /// Position in the source file, in seconds, at timeline position `t`. Negative values
    /// (and values past the end of the file) mean silence.
    pub fn source_seconds(&self, t: f64, tempo: f64) -> f64 {
        let p = self.content_position(t, tempo);
        if self.warped {
            self.beats_to_seconds(p, tempo)
        } else {
            p
        }
    }

    /// Maps a position in sample beats to source seconds through the warp markers,
    /// extrapolating with the slope of the first or last segment.
    fn beats_to_seconds(&self, p: f64, tempo: f64) -> f64 {
        let m = &self.markers;
        match m.len() {
            0 => p * 60.0 / tempo,
            1 => m[0].sec_time + (p - m[0].beat_time) * 60.0 / tempo,
            n => {
                let j = m
                    .partition_point(|x| x.beat_time <= p)
                    .saturating_sub(1)
                    .min(n - 2);
                let (a, b) = (m[j], m[j + 1]);
                let seconds_per_beat = (b.sec_time - a.sec_time) / (b.beat_time - a.beat_time);
                a.sec_time + (p - a.beat_time) * seconds_per_beat
            }
        }
    }

    /// Stereo audio at timeline position `t`, before gain.
    pub fn frame_at(&self, t: f64, tempo: f64) -> [f32; 2] {
        match &self.sample {
            Some(s) => s.frame(self.source_seconds(t, tempo) * f64::from(s.sample_rate())),
            None => [0.0, 0.0],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(warped: bool, looping: ClipLoop, markers: &[(f64, f64)]) -> AudioRegion {
        AudioRegion {
            start: 100.0,
            end: 200.0,
            sample_ref: SampleRef::default(),
            sample: None,
            warped,
            markers: AudioRegion::normalize_markers(
                &markers
                    .iter()
                    .map(|&(sec_time, beat_time)| WarpMarker {
                        sec_time,
                        beat_time,
                    })
                    .collect::<Vec<_>>(),
            ),
            looping,
            warp_mode: WarpMode::Complex,
            gain: 1.0,
            pitch_semitones: 0.0,
        }
    }

    const NO_LOOP: ClipLoop = ClipLoop {
        start: 0.0,
        end: 64.0,
        start_relative: 0.0,
        enabled: false,
    };

    #[test]
    fn warp_map_interpolates_and_extrapolates_with_the_end_slopes() {
        // 0.5 s per beat up to beat 4, then 1 s per beat.
        let r = region(true, NO_LOOP, &[(0.0, 0.0), (2.0, 4.0), (6.0, 8.0)]);
        assert_eq!(r.source_seconds(100.0, 120.0), 0.0);
        assert_eq!(r.source_seconds(102.0, 120.0), 1.0);
        assert_eq!(r.source_seconds(106.0, 120.0), 4.0);
        assert_eq!(r.source_seconds(110.0, 120.0), 8.0); // beyond the last marker: slope 1
        assert_eq!(r.source_seconds(99.0, 120.0), -0.5); // before the first: slope 0.5
    }

    #[test]
    fn marker_not_at_zero_anchors_the_map() {
        // Beat 0 sits 31.5 s into the file, and the clip starts at beat 3.
        let looping = ClipLoop {
            start: 3.0,
            end: 36.0,
            ..NO_LOOP
        };
        let r = region(true, looping, &[(31.5, 0.0), (32.0, 1.0)]);
        assert!((r.source_seconds(100.0, 120.0) - 33.0).abs() < 1e-9);
    }

    #[test]
    fn looping_clip_starts_at_the_start_marker_then_repeats_the_loop() {
        // Loop -4..28 with the start marker 4 beats before the loop start.
        let looping = ClipLoop {
            start: -4.0,
            end: 28.0,
            start_relative: -4.0,
            enabled: true,
        };
        let r = region(true, looping, &[(0.0, 0.0), (1.0, 1.0)]);
        let at = |t: f64| r.content_position(t, 120.0);
        let near = |a: f64, b: f64| assert!((a - b).abs() < 1e-9, "{a} != {b}");
        near(at(100.0), -8.0);
        near(at(110.0), 2.0);
        near(at(135.9), 27.9);
        near(at(136.0), -4.0); // first pass is 36 beats
        near(at(136.0 + 32.0), -4.0); // then 32-beat loops
        near(at(136.0 + 40.0), 4.0);
    }

    #[test]
    fn unwarped_clip_plays_at_native_speed_whatever_the_tempo() {
        let looping = ClipLoop {
            start: 1.0,
            end: 9.0,
            ..NO_LOOP
        };
        let r = region(false, looping, &[]);
        assert_eq!(r.source_seconds(102.0, 120.0), 2.0); // 2 beats = 1 s at 120 bpm
        assert_eq!(r.source_seconds(102.0, 60.0), 3.0); // 2 beats = 2 s at 60 bpm
    }

    #[test]
    fn missing_markers_fall_back_to_the_project_tempo() {
        let r = region(true, NO_LOOP, &[]);
        assert_eq!(r.source_seconds(104.0, 120.0), 2.0);
        let r = region(true, NO_LOOP, &[(10.0, 0.0)]);
        assert_eq!(r.source_seconds(104.0, 120.0), 12.0);
    }

    #[test]
    fn duplicate_beat_times_are_removed() {
        let markers = AudioRegion::normalize_markers(&[
            WarpMarker {
                sec_time: 1.0,
                beat_time: 1.0,
            },
            WarpMarker {
                sec_time: 0.0,
                beat_time: 0.0,
            },
            WarpMarker {
                sec_time: 1.0,
                beat_time: 1.0,
            },
        ]);
        assert_eq!(markers.len(), 2);
        assert_eq!(markers[0].beat_time, 0.0);
    }
}
