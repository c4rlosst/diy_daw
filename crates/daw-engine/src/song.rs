//! Turns the clips of a [`Project`] into what the engine plays: a flat, time-sorted list of
//! MIDI events, plus audio regions with their warp maps.
//!
//! Simplifications:
//! * MIDI clip loop settings are not used yet: session clips repeat at their own
//!   `end - start` length and arrangement clips play once.
//! * Audio clips follow their loop settings and warp markers (see [`AudioRegion`]).

use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use daw_model::{ClipContent, Project, SampleRef, TrackKind};

use crate::region::AudioRegion;
use crate::sample::{Sample, SampleResolver};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EventKind {
    NoteOff { key: u8 },
    NoteOn { key: u8, velocity: f32 },
}

impl EventKind {
    /// Note-offs sort before note-ons at the same beat so repeated notes retrigger.
    fn order(self) -> u8 {
        match self {
            EventKind::NoteOff { .. } => 0,
            EventKind::NoteOn { .. } => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Event {
    pub beat: f64,
    /// Index into [`Song::track_names`] / [`Song::track_gains`].
    pub track: usize,
    pub kind: EventKind,
}

/// Which part of the project to play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// One row of session clips, looped.
    Scene(usize),
    /// The arrangement timeline, played once.
    Arrangement,
}

impl FromStr for Source {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        if s.eq_ignore_ascii_case("arrangement") || s.eq_ignore_ascii_case("arr") {
            Ok(Source::Arrangement)
        } else {
            s.parse()
                .map(Source::Scene)
                .map_err(|_| format!("expected a scene number or 'arrangement', got {s:?}"))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Song {
    pub tempo: f64,
    /// Loop length in beats; 0 means the song is empty and never wraps.
    pub length_beats: f64,
    /// One entry per MIDI and audio track, in project order.
    pub track_names: Vec<String>,
    /// Linear gain per track, taken from the project's track volume.
    pub track_gains: Vec<f32>,
    /// Sorted by beat, note-offs first within a beat.
    pub events: Vec<Event>,
    /// Audio regions per track (empty for MIDI tracks), each sorted by start.
    pub regions: Vec<Vec<AudioRegion>>,
}

/// What [`Song::load_samples`] could not load.
#[derive(Debug, Default)]
pub struct LoadReport {
    pub loaded: usize,
    pub missing: Vec<SampleRef>,
    pub failed: Vec<(PathBuf, String)>,
}

impl Song {
    pub fn from_project(project: &Project, source: Source) -> Song {
        let mut song = Song {
            tempo: project.tempo,
            ..Song::default()
        };
        let mut midi: Vec<(usize, f64, f64, &[daw_model::Note])> = Vec::new(); // track, start, len, notes
        let mut audio_length: f64 = 0.0;

        let playable = project
            .tracks
            .iter()
            .filter(|t| matches!(t.kind, TrackKind::Midi | TrackKind::Audio));
        for (index, track) in playable.enumerate() {
            song.track_names.push(track.name.clone());
            song.track_gains.push(track.volume.clamp(0.0, 2.0) as f32);
            song.regions.push(Vec::new());
            let clips: Vec<(&daw_model::Clip, f64)> = match source {
                Source::Scene(n) => track
                    .session_clips
                    .get(n)
                    .into_iter()
                    .flatten()
                    .map(|c| (c, 0.0))
                    .collect(),
                Source::Arrangement => track
                    .arrangement_clips
                    .iter()
                    .map(|c| (c, c.start))
                    .collect(),
            };
            for (clip, start) in clips {
                let len = clip.end - clip.start;
                if len <= 0.0 {
                    continue;
                }
                match &clip.content {
                    ClipContent::Midi { notes } => midi.push((index, start, len, notes)),
                    ClipContent::Audio {
                        sample,
                        warped,
                        warp_markers,
                        warp_mode,
                        gain,
                        pitch_semitones,
                        ..
                    } => {
                        let session = matches!(source, Source::Scene(_));
                        audio_length = audio_length.max(if session { len } else { start + len });
                        song.regions[index].push(AudioRegion {
                            start,
                            // A looping session clip keeps playing for as long as the scene does.
                            end: if session && clip.looping.enabled {
                                f64::INFINITY
                            } else {
                                start + len
                            },
                            sample_ref: sample.clone(),
                            sample: None,
                            warped: *warped,
                            markers: AudioRegion::normalize_markers(warp_markers),
                            looping: clip.looping,
                            warp_mode: *warp_mode,
                            gain: *gain,
                            pitch_semitones: *pitch_semitones,
                        });
                    }
                }
            }
            song.regions[index].sort_by(|a, b| a.start.total_cmp(&b.start));
        }

        let midi_length = match source {
            Source::Scene(_) => midi.iter().map(|c| c.2).fold(0.0, f64::max),
            Source::Arrangement => midi.iter().map(|c| c.1 + c.2).fold(0.0, f64::max),
        };
        song.length_beats = midi_length.max(audio_length);

        for &(track, start, len, notes) in &midi {
            let repeat = matches!(source, Source::Scene(_));
            let mut offset = start;
            loop {
                for n in notes
                    .iter()
                    .filter(|n| n.time >= 0.0 && n.time < len && n.duration > 0.0)
                {
                    let on = offset + n.time;
                    let off = offset + (n.time + n.duration).min(len);
                    if on >= song.length_beats {
                        continue;
                    }
                    song.events.push(Event {
                        beat: on,
                        track,
                        kind: EventKind::NoteOn {
                            key: n.key,
                            velocity: n.velocity / 127.0,
                        },
                    });
                    // A note-off at or past the loop end is handled by the wrap itself.
                    if off < song.length_beats {
                        song.events.push(Event {
                            beat: off,
                            track,
                            kind: EventKind::NoteOff { key: n.key },
                        });
                    }
                }
                offset += len;
                if !repeat || offset >= song.length_beats - 1e-9 {
                    break;
                }
            }
        }

        song.events.sort_by(|a, b| {
            a.beat
                .total_cmp(&b.beat)
                .then(a.kind.order().cmp(&b.kind.order()))
        });
        song
    }

    /// Finds and decodes the audio files of all regions. Each file is decoded once and
    /// shared between the regions that use it. Regions whose file cannot be found or
    /// decoded stay silent and are listed in the report.
    pub fn load_samples(&mut self, resolver: &SampleResolver) -> LoadReport {
        let mut report = LoadReport::default();
        let mut cache: HashMap<PathBuf, Option<Arc<Sample>>> = HashMap::new();
        for region in self.regions.iter_mut().flatten() {
            let Some(path) = resolver.resolve(&region.sample_ref) else {
                if !report.missing.contains(&region.sample_ref) {
                    report.missing.push(region.sample_ref.clone());
                }
                continue;
            };
            let sample =
                cache
                    .entry(path.clone())
                    .or_insert_with(|| match Sample::decode_file(&path) {
                        Ok(sample) => {
                            report.loaded += 1;
                            Some(Arc::new(sample))
                        }
                        Err(e) => {
                            report.failed.push((path.clone(), e.to_string()));
                            None
                        }
                    });
            region.sample = sample.clone();
        }
        report
    }
}
