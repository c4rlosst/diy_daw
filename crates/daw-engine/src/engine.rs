//! The realtime engine: transport, sequencer and one synth per MIDI track.
//!
//! [`Engine::render`] is called from the audio callback and never blocks or locks.
//! The rest of the program talks to it through an [`EngineHandle`], which sends
//! [`Command`]s over a channel that the engine drains at the start of each block.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use crate::song::{Event, EventKind, Song};
use crate::synth::{Synth, Waveform};

pub const MIN_TEMPO: f64 = 20.0;
pub const MAX_TEMPO: f64 = 999.0;

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
    LoadSong(Box<Song>),
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
    pub fn load_song(&self, song: Song) {
        self.send(Command::LoadSong(Box::new(song)));
    }

    /// Playhead position in beats, as of the last audio block.
    pub fn position_beats(&self) -> f64 {
        f64::from_bits(self.shared.position_bits.load(Ordering::Relaxed))
    }
    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Relaxed)
    }
}

pub struct Engine {
    sample_rate: f32,
    rx: Receiver<Command>,
    shared: Arc<Shared>,
    song: Song,
    synths: Vec<Synth>,
    waveform: Waveform,
    playing: bool,
    looping: bool,
    tempo: f64,
    position: f64,
    next_event: usize,
}

impl Engine {
    pub fn new(sample_rate: u32) -> (Engine, EngineHandle) {
        let (tx, rx) = channel();
        let shared = Arc::new(Shared::default());
        let engine = Engine {
            sample_rate: sample_rate as f32,
            rx,
            shared: shared.clone(),
            song: Song::default(),
            synths: Vec::new(),
            waveform: Waveform::default(),
            playing: false,
            looping: true,
            tempo: 120.0,
            position: 0.0,
            next_event: 0,
        };
        (engine, EngineHandle { tx, shared })
    }

    /// Fills `out` (interleaved, `channels` per frame) with the next block of audio.
    pub fn render(&mut self, out: &mut [f32], channels: usize) {
        self.process_commands();
        let channels = channels.max(1);
        let beats_per_frame = self.tempo / 60.0 / f64::from(self.sample_rate);

        for frame in out.chunks_exact_mut(channels) {
            if self.playing {
                self.dispatch_due_events();
            }
            let mut mix = 0.0;
            for (synth, gain) in self.synths.iter_mut().zip(&self.song.track_gains) {
                mix += synth.next_sample() * gain;
            }
            let sample = mix.tanh(); // soft clip so dense passages never exceed full scale
            frame.fill(sample);

            if self.playing {
                self.position += beats_per_frame;
                if self.looping
                    && self.song.length_beats > 0.0
                    && self.position >= self.song.length_beats
                {
                    self.position %= self.song.length_beats;
                    self.release_all();
                    // The playhead overshoots the loop start by a fraction of a frame; restart at
                    // event 0 so events at the very start still fire on the next frame.
                    self.next_event = 0;
                }
            }
        }

        self.shared
            .position_bits
            .store(self.position.to_bits(), Ordering::Relaxed);
        self.shared.playing.store(self.playing, Ordering::Relaxed);
    }

    fn process_commands(&mut self) {
        while let Ok(command) = self.rx.try_recv() {
            match command {
                Command::Play => self.playing = true,
                Command::Stop => {
                    if self.playing {
                        self.playing = false;
                        self.release_all();
                    } else {
                        self.seek(0.0);
                    }
                }
                Command::SetTempo(bpm) => self.tempo = bpm.clamp(MIN_TEMPO, MAX_TEMPO),
                Command::Seek(beats) => self.seek(beats),
                Command::SetLooping(looping) => self.looping = looping,
                Command::SetWaveform(waveform) => {
                    self.waveform = waveform;
                    for synth in &mut self.synths {
                        synth.waveform = waveform;
                    }
                }
                Command::LoadSong(song) => self.load_song(*song),
            }
        }
    }

    fn load_song(&mut self, song: Song) {
        self.synths = song
            .track_names
            .iter()
            .map(|_| {
                let mut synth = Synth::new(self.sample_rate);
                synth.waveform = self.waveform;
                synth
            })
            .collect();
        if song.tempo > 0.0 {
            self.tempo = song.tempo.clamp(MIN_TEMPO, MAX_TEMPO);
        }
        self.song = song;
        self.seek(self.position);
    }

    fn seek(&mut self, beats: f64) {
        self.release_all();
        let mut position = beats.max(0.0);
        if self.looping && self.song.length_beats > 0.0 {
            position %= self.song.length_beats;
        }
        self.position = position;
        self.next_event = self.first_event_at(position);
    }

    fn first_event_at(&self, beat: f64) -> usize {
        self.song.events.partition_point(|e| e.beat < beat)
    }

    fn release_all(&mut self) {
        for synth in &mut self.synths {
            synth.release_all();
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
        if let Some(synth) = self.synths.get_mut(event.track) {
            match event.kind {
                EventKind::NoteOn { key, velocity } => synth.note_on(key, velocity),
                EventKind::NoteOff { key } => synth.note_off(key),
            }
        }
    }
}
