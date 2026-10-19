//! Turns the MIDI clips of a [`Project`] into a flat, time-sorted event list.
//!
//! Simplifications (the project model does not carry clip loop settings yet):
//! * Session clips repeat over the song length, each at its own `end - start` length.
//! * Arrangement clips play once, starting at their position on the timeline.

use std::str::FromStr;

use daw_model::{ClipContent, Project, TrackKind};

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
    pub track_names: Vec<String>,
    /// Linear gain per track, taken from the project's track volume.
    pub track_gains: Vec<f32>,
    /// Sorted by beat, note-offs first within a beat.
    pub events: Vec<Event>,
}

impl Song {
    pub fn from_project(project: &Project, source: Source) -> Song {
        let mut song = Song {
            tempo: project.tempo,
            ..Song::default()
        };
        let mut clips: Vec<(usize, f64, f64, &[daw_model::Note])> = Vec::new(); // track, start, len, notes

        let midi_tracks = project.tracks.iter().filter(|t| t.kind == TrackKind::Midi);
        for (index, track) in midi_tracks.enumerate() {
            song.track_names.push(track.name.clone());
            song.track_gains.push(track.volume.clamp(0.0, 2.0) as f32);
            match source {
                Source::Scene(n) => {
                    if let Some(Some(clip)) = track.session_clips.get(n) {
                        push_clip(&mut clips, index, clip, 0.0);
                    }
                }
                Source::Arrangement => {
                    for clip in &track.arrangement_clips {
                        push_clip(&mut clips, index, clip, clip.start);
                    }
                }
            }
        }

        song.length_beats = match source {
            Source::Scene(_) => clips.iter().map(|c| c.2).fold(0.0, f64::max),
            Source::Arrangement => clips.iter().map(|c| c.1 + c.2).fold(0.0, f64::max),
        };

        for &(track, start, len, notes) in &clips {
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
}

fn push_clip<'a>(
    clips: &mut Vec<(usize, f64, f64, &'a [daw_model::Note])>,
    track: usize,
    clip: &'a daw_model::Clip,
    start: f64,
) {
    if let ClipContent::Midi { notes } = &clip.content {
        let len = clip.end - clip.start;
        if len > 0.0 {
            clips.push((track, start, len, notes));
        }
    }
}
