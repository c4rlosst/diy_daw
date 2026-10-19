//! diy_daw audio engine: transport, a sequencer for MIDI clips, audio clip playback with
//! warp markers, a simple synth, and cpal output. The engine itself ([`Engine::render`]) is
//! independent of any audio device, so it can also render offline.

pub mod audio;
pub mod engine;
pub mod region;
pub mod sample;
pub mod song;
pub mod stretch;
pub mod synth;
pub mod wav;

pub use audio::{AudioError, AudioOutput};
pub use engine::{Command, Engine, EngineHandle};
pub use region::AudioRegion;
pub use sample::{Sample, SampleError, SampleResolver};
pub use song::{LoadReport, Song, Source};
pub use synth::Waveform;
