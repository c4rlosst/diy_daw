//! diy_daw audio engine: transport, a sequencer for MIDI clips, a simple synth,
//! and cpal output. The engine itself ([`Engine::render`]) is independent of any
//! audio device, so it can also render offline.

pub mod audio;
pub mod engine;
pub mod song;
pub mod synth;
pub mod wav;

pub use audio::{AudioError, AudioOutput};
pub use engine::{Command, Engine, EngineHandle};
pub use song::{Song, Source};
pub use synth::Waveform;
