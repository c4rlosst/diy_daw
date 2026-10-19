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
    pub looping: ClipLoop,
    pub content: ClipContent,
}

/// Loop region and start marker of a clip.
///
/// Units follow the clip's content space: beats for MIDI clips and warped audio clips
/// (the clip's "sample beats", which warp markers map to source seconds), and seconds of
/// source audio for unwarped audio clips. Positions may be negative, meaning silence
/// before the content starts.
///
/// The clip begins playing at `start + start_relative`. With `enabled`, it plays on to
/// `end` and then repeats `start..end`; otherwise it plays on for the clip's length.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ClipLoop {
    pub start: f64,
    pub end: f64,
    pub start_relative: f64,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClipContent {
    Midi {
        notes: Vec<Note>,
    },
    Audio {
        sample: SampleRef,
        /// Sample rate and length of the source file, as recorded in the set.
        sample_rate: u32,
        sample_frames: u64,
        warped: bool,
        warp_markers: Vec<WarpMarker>,
        warp_mode: WarpMode,
        /// Linear clip gain, 1.0 = unity.
        gain: f32,
        /// Transposition in semitones (coarse + fine).
        pitch_semitones: f64,
    },
}

/// Where a clip's audio file lives, as stored in the source project.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SampleRef {
    /// Path relative to the library or project folder.
    pub path: String,
    /// Library the path is relative to (e.g. "Core Library"); empty for the project itself.
    pub pack: String,
}

/// Time-stretching algorithm chosen for a clip. Only `Complex` was observed in the factory
/// sets; the other ids follow common documentation of the format and are unverified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarpMode {
    Beats,
    Tones,
    Texture,
    /// Playback speed changes pitch (no time-stretching).
    RePitch,
    Complex,
    ComplexPro,
    Other(i32),
}

impl WarpMode {
    pub fn from_id(id: i32) -> WarpMode {
        match id {
            0 => WarpMode::Beats,
            1 => WarpMode::Tones,
            2 => WarpMode::Texture,
            3 => WarpMode::RePitch,
            4 => WarpMode::Complex,
            6 => WarpMode::ComplexPro,
            other => WarpMode::Other(other),
        }
    }
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
