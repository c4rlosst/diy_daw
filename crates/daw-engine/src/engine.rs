//! The realtime engine: transport, sequencer, a synth per track, and audio clip playback.
//!
//! [`Engine::render`] is called from the audio callback and never blocks or locks.
//! The rest of the program talks to it through an [`EngineHandle`], which sends
//! [`Command`]s over a channel that the engine drains at the start of each block.
//!
//! Audio clips are played by position: for every output frame the engine maps the
//! timeline position to a position in the source file (through the clip's loop settings and
//! warp markers). Warped clips are time-stretched so pitch is preserved
//! ([`crate::stretch`]); clips in Re-Pitch mode, unwarped clips, and builds without the
//! `stretch` feature read the file with interpolation, so pitch follows speed.
//! Seeks, loops and tempo changes stay in sync either way.

use std::any::Any;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use crate::region::AudioRegion;
use crate::song::{Event, EventKind, Song};
use crate::stretch::{Feed, StretchVoice};
use crate::synth::{Synth, Waveform};

pub const MIN_TEMPO: f64 = 20.0;
pub const MAX_TEMPO: f64 = 999.0;

/// Fade applied when an audio region starts or stops sounding, to avoid clicks.
const AUDIO_FADE_SECONDS: f32 = 0.002;
/// After Stop, audio keeps running this long so the fade-out has something to fade.
const STOP_TAIL_SECONDS: f32 = 0.01;

#[derive(Debug)]
pub enum Command {
    Play,
    /// Halts playback; if already stopped, rewinds to the start (like Live's Stop button).
    Stop,
    SetTempo(f64),
    Seek(f64),
    SetLooping(bool),
    SetWaveform(Waveform),
    /// Replaces the song and adopts its tempo. Playback state is kept.
    LoadSong(Box<LoadedSong>),
}

/// A song together with the per-track playback state built for it. Created by
/// [`EngineHandle::load_song`] on the caller's thread, so the audio thread only swaps it in.
pub struct LoadedSong {
    song: Song,
    tracks: Vec<TrackState>,
}

impl std::fmt::Debug for LoadedSong {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LoadedSong({} tracks)", self.tracks.len())
    }
}

#[derive(Default)]
struct Shared {
    position_bits: AtomicU64,
    playing: AtomicBool,
}

/// Cheap, cloneable control surface for a running [`Engine`].
#[derive(Clone)]
pub struct EngineHandle {
    tx: Sender<Command>,
    shared: Arc<Shared>,
    sample_rate: u32,
}

impl EngineHandle {
    fn send(&self, command: Command) {
        // The engine only goes away when the stream is dropped; nothing useful to do then.
        let _ = self.tx.send(command);
    }

    pub fn play(&self) {
        self.send(Command::Play);
    }
    pub fn stop(&self) {
        self.send(Command::Stop);
    }
    pub fn set_tempo(&self, bpm: f64) {
        self.send(Command::SetTempo(bpm));
    }
    pub fn seek(&self, beats: f64) {
        self.send(Command::Seek(beats));
    }
    pub fn set_looping(&self, looping: bool) {
        self.send(Command::SetLooping(looping));
    }
    pub fn set_waveform(&self, waveform: Waveform) {
        self.send(Command::SetWaveform(waveform));
    }

    /// Loads a song. The per-track state (including time-stretchers) is built here, not on
    /// the audio thread.
    pub fn load_song(&self, mut song: Song) {
        let tracks = song.track_names.len();
        song.regions.resize_with(tracks, Vec::new);
        song.track_gains.resize(tracks, 1.0);
        let tracks = song
            .regions
            .iter()
            .map(|regions| TrackState::new(self.sample_rate, regions))
            .collect();
        self.send(Command::LoadSong(Box::new(LoadedSong { song, tracks })));
    }

    /// Playhead position in beats, as of the last audio block.
    pub fn position_beats(&self) -> f64 {
        f64::from_bits(self.shared.position_bits.load(Ordering::Relaxed))
    }
    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Transport {
    Stopped,
    Playing,
    /// Stop was pressed; audio fades out over the remaining frames.
    Stopping {
        frames_left: u32,
    },
}

struct TrackState {
    synth: Synth,
    /// Present only for tracks with regions that are time-stretched.
    stretch: Option<StretchVoice>,
    /// Index of the first region that has not ended yet (regions are sorted by start).
    cursor: usize,
    /// Region currently (or most recently) sounding, kept while its fade-out runs.
    region: Option<usize>,
    envelope: f32,
}

impl TrackState {
    fn new(sample_rate: u32, regions: &[AudioRegion]) -> TrackState {
        TrackState {
            synth: Synth::new(sample_rate as f32),
            stretch: regions
                .iter()
                .any(AudioRegion::uses_stretch)
                .then(|| StretchVoice::new(sample_rate)),
            cursor: 0,
            region: None,
            envelope: 0.0,
        }
    }

    fn reset_audio(&mut self) {
        self.cursor = 0;
        self.region = None;
        self.envelope = 0.0;
    }

    /// Next stereo frame of this track's audio at timeline position `t`.
    fn audio_frame(&mut self, feed: &Feed, t: f64, playing: bool, fade_step: f32) -> [f32; 2] {
        let regions = feed.regions;
        if regions.is_empty() {
            return [0.0, 0.0];
        }
        while self.cursor < regions.len() && regions[self.cursor].end <= t {
            self.cursor += 1;
        }
        let active =
            (self.cursor < regions.len() && regions[self.cursor].start <= t).then_some(self.cursor);
        if active.is_some() {
            self.region = active;
        }
        let target = if playing && active.is_some() {
            1.0
        } else {
            0.0
        };
        if self.envelope < target {
            self.envelope = (self.envelope + fade_step).min(target);
        } else {
            self.envelope = (self.envelope - fade_step).max(target);
        }
        if self.envelope <= 0.0 {
            return [0.0, 0.0];
        }
        let Some(region) = self.region.and_then(|i| regions.get(i)) else {
            return [0.0, 0.0];
        };
        let [l, r] = match &mut self.stretch {
            Some(voice) if region.uses_stretch() => voice.next_frame(feed, t),
            _ => region.frame_at(t, feed.tempo),
        };
        let gain = self.envelope * region.gain;
        [l * gain, r * gain]
    }
}

pub struct Engine {
    sample_rate: f32,
    rx: Receiver<Command>,
    shared: Arc<Shared>,
    /// Replaced songs are sent here to be freed off the audio thread.
    retire: Sender<Box<dyn Any + Send>>,
    song: Song,
    tracks: Vec<TrackState>,
    waveform: Waveform,
    transport: Transport,
    looping: bool,
    tempo: f64,
    position: f64,
    /// Beats advanced since Stop was pressed; only audio uses it, `position` stays put.
    tail_beats: f64,
    next_event: usize,
}

impl Engine {
    pub fn new(sample_rate: u32) -> (Engine, EngineHandle) {
        let (tx, rx) = channel();
        let shared = Arc::new(Shared::default());

        let (retire, retired) = channel::<Box<dyn Any + Send>>();
        // If the thread cannot start, replaced songs are simply freed where they are dropped.
        let _ = std::thread::Builder::new()
            .name("daw-engine-gc".into())
            .spawn(move || while retired.recv().is_ok() {});

        let engine = Engine {
            sample_rate: sample_rate as f32,
            rx,
            shared: shared.clone(),
            retire,
            song: Song::default(),
            tracks: Vec::new(),
            waveform: Waveform::default(),
            transport: Transport::Stopped,
            looping: true,
            tempo: 120.0,
            position: 0.0,
            tail_beats: 0.0,
            next_event: 0,
        };
        (
            engine,
            EngineHandle {
                tx,
                shared,
                sample_rate,
            },
        )
    }

    /// Fills `out` (interleaved, `channels` per frame) with the next block of audio.
    /// Mono output gets the mid mix; with more than two channels only the first two are used.
    pub fn render(&mut self, out: &mut [f32], channels: usize) {
        self.process_commands();
        let channels = channels.max(1);
        let beats_per_frame = self.tempo / 60.0 / f64::from(self.sample_rate);
        let fade_step = 1.0 / (AUDIO_FADE_SECONDS * self.sample_rate);
        let wrap = (self.looping && self.song.length_beats > 0.0).then_some(self.song.length_beats);

        for frame in out.chunks_exact_mut(channels) {
            let playing = self.transport == Transport::Playing;
            if playing {
                self.dispatch_due_events();
            }

            let t = self.position + self.tail_beats;
            let (mut left, mut right) = (0.0f32, 0.0f32);
            for ((track, gain), regions) in self
                .tracks
                .iter_mut()
                .zip(&self.song.track_gains)
                .zip(&self.song.regions)
            {
                let synth = track.synth.next_sample();
                let feed = Feed {
                    regions,
                    tempo: self.tempo,
                    beats_per_frame,
                    wrap,
                };
                let [al, ar] = track.audio_frame(&feed, t, playing, fade_step);
                left += (synth + al) * gain;
                right += (synth + ar) * gain;
            }
            // Soft clip so dense passages never exceed full scale.
            let (left, right) = (left.tanh(), right.tanh());
            match channels {
                1 => frame[0] = 0.5 * (left + right),
                _ => {
                    frame.fill(0.0);
                    frame[0] = left;
                    frame[1] = right;
                }
            }

            match self.transport {
                Transport::Playing => {
                    self.position += beats_per_frame;
                    if self.looping
                        && self.song.length_beats > 0.0
                        && self.position >= self.song.length_beats
                    {
                        self.position %= self.song.length_beats;
                        self.release_all();
                        for track in &mut self.tracks {
                            track.cursor = 0;
                        }
                        // The playhead overshoots the loop start by a fraction of a frame;
                        // restart at event 0 so events at the very start still fire.
                        self.next_event = 0;
                    }
                }
                Transport::Stopping { frames_left } => {
                    self.tail_beats += beats_per_frame;
                    if frames_left <= 1 {
                        self.transport = Transport::Stopped;
                        self.tail_beats = 0.0;
                        for track in &mut self.tracks {
                            // `position` did not move, so the cursor is already right.
                            track.envelope = 0.0;
                        }
                    } else {
                        self.transport = Transport::Stopping {
                            frames_left: frames_left - 1,
                        };
                    }
                }
                Transport::Stopped => {}
            }
        }

        self.shared
            .position_bits
            .store(self.position.to_bits(), Ordering::Relaxed);
        self.shared
            .playing
            .store(self.transport == Transport::Playing, Ordering::Relaxed);
    }

    fn process_commands(&mut self) {
        while let Ok(command) = self.rx.try_recv() {
            match command {
                Command::Play => {
                    self.tail_beats = 0.0;
                    self.transport = Transport::Playing;
                }
                Command::Stop => match self.transport {
                    Transport::Playing => {
                        self.release_all();
                        self.transport = Transport::Stopping {
                            frames_left: (STOP_TAIL_SECONDS * self.sample_rate) as u32,
                        };
                    }
                    Transport::Stopping { .. } => {}
                    Transport::Stopped => self.seek(0.0),
                },
                Command::SetTempo(bpm) => self.tempo = bpm.clamp(MIN_TEMPO, MAX_TEMPO),
                Command::Seek(beats) => self.seek(beats),
                Command::SetLooping(looping) => self.looping = looping,
                Command::SetWaveform(waveform) => {
                    self.waveform = waveform;
                    for track in &mut self.tracks {
                        track.synth.waveform = waveform;
                    }
                }
                Command::LoadSong(loaded) => self.load_song(*loaded),
            }
        }
    }

    fn load_song(&mut self, loaded: LoadedSong) {
        let LoadedSong { song, mut tracks } = loaded;
        for track in &mut tracks {
            track.synth.waveform = self.waveform;
        }
        if song.tempo > 0.0 {
            self.tempo = song.tempo.clamp(MIN_TEMPO, MAX_TEMPO);
        }
        let old_song = std::mem::replace(&mut self.song, song);
        let old_tracks = std::mem::replace(&mut self.tracks, tracks);
        let _ = self.retire.send(Box::new((old_song, old_tracks)));
        self.seek(self.position);
    }

    fn seek(&mut self, beats: f64) {
        self.release_all();
        if matches!(self.transport, Transport::Stopping { .. }) {
            self.transport = Transport::Stopped;
        }
        self.tail_beats = 0.0;
        let mut position = beats.max(0.0);
        if self.looping && self.song.length_beats > 0.0 {
            position %= self.song.length_beats;
        }
        self.position = position;
        self.next_event = self.first_event_at(position);
        for track in &mut self.tracks {
            track.reset_audio();
        }
    }

    fn first_event_at(&self, beat: f64) -> usize {
        self.song.events.partition_point(|e| e.beat < beat)
    }

    fn release_all(&mut self) {
        for track in &mut self.tracks {
            track.synth.release_all();
        }
    }

    fn dispatch_due_events(&mut self) {
        while let Some(&event) = self.song.events.get(self.next_event) {
            if event.beat > self.position {
                break;
            }
            self.apply(event);
            self.next_event += 1;
        }
    }

    fn apply(&mut self, event: Event) {
        if let Some(track) = self.tracks.get_mut(event.track) {
            match event.kind {
                EventKind::NoteOn { key, velocity } => track.synth.note_on(key, velocity),
                EventKind::NoteOff { key } => track.synth.note_off(key),
            }
        }
    }
}
