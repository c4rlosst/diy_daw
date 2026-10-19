//! Format-independent project model.
//!
//! Time is measured in beats (quarter notes) unless a field says otherwise.

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Project {
    /// Tempo in beats per minute.
    pub tempo: f64,
    pub tracks: Vec<Track>,
    pub return_tracks: Vec<Track>,
    pub scene_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Midi,
    Audio,
    Group,
    Return,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub id: i64,
    pub name: String,
    pub kind: TrackKind,
    /// Linear gain, 1.0 = unity.
    pub volume: f64,
    /// Device class names in signal-chain order.
    pub devices: Vec<String>,
    /// One entry per scene; `None` is an empty slot.
    pub session_clips: Vec<Option<Clip>>,
    pub arrangement_clips: Vec<Clip>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub name: String,
    /// Position on the timeline (arrangement) or 0 for session clips.
    pub start: f64,
    pub end: f64,
    pub content: ClipContent,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClipContent {
    Midi {
        notes: Vec<Note>,
    },
    Audio {
        /// Sample path relative to its library, as stored in the source file.
        sample_path: String,
        warped: bool,
        warp_markers: Vec<WarpMarker>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Note {
    /// MIDI note number.
    pub key: u8,
    /// Beats from the start of the clip.
    pub time: f64,
    pub duration: f64,
    pub velocity: f32,
}

/// Maps a position in the source audio (seconds) to a position on the timeline (beats).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WarpMarker {
    pub sec_time: f64,
    pub beat_time: f64,
}

impl Clip {
    pub fn note_count(&self) -> usize {
        match &self.content {
            ClipContent::Midi { notes } => notes.len(),
            ClipContent::Audio { .. } => 0,
        }
    }
}

impl Track {
    pub fn session_clip_count(&self) -> usize {
        self.session_clips.iter().flatten().count()
    }
}
