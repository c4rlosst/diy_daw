//! Plays a Live Set (MIDI clips and audio clips) through the default audio device.
//!
//! `cargo run --release -p daw-engine --example play -- <set.als> [--scene N | --arrangement]
//!     [--seconds S] [--from BEAT] [--tempo BPM] [--wave sine|saw] [--library DIR]`

mod common;

use std::time::{Duration, Instant};

use daw_engine::AudioOutput;

fn main() {
    let options = common::Options::parse("play");
    let song = options.load_song();

    let (output, engine) = AudioOutput::start().expect("could not open an audio output");
    println!(
        "playing on {} ({} Hz, {} ch)",
        output.device_name, output.sample_rate, output.channels
    );
    engine.load_song(song);
    engine.set_waveform(options.waveform);
    if let Some(bpm) = options.tempo {
        engine.set_tempo(bpm);
    }
    engine.seek(options.from_beat);
    engine.play();

    let start = Instant::now();
    while start.elapsed().as_secs_f64() < options.seconds {
        std::thread::sleep(Duration::from_millis(500));
        println!("  beat {:7.2}", engine.position_beats());
    }
    engine.stop();
    std::thread::sleep(Duration::from_millis(200));
}
