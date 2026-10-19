//! Renders a Live Set (MIDI clips and audio clips) to a 16-bit WAV file without touching an
//! audio device.
//!
//! `cargo run --release -p daw-engine --example render -- <set.als> <out.wav>
//!     [--scene N | --arrangement] [--seconds S] [--from BEAT] [--tempo BPM]
//!     [--wave sine|saw] [--library DIR]`

mod common;

use daw_engine::engine::Engine;
use daw_engine::wav;

fn main() {
    let options = common::Options::parse("render");
    let out = options
        .rest
        .first()
        .expect("usage: render <set.als> <out.wav> [options]")
        .clone();
    let song = options.load_song();

    const SR: u32 = 48_000;
    let (mut engine, handle) = Engine::new(SR);
    handle.load_song(song);
    handle.set_waveform(options.waveform);
    if let Some(bpm) = options.tempo {
        handle.set_tempo(bpm);
    }
    handle.seek(options.from_beat);
    handle.play();

    let frames = (options.seconds * f64::from(SR)) as usize;
    let mut samples = vec![0.0f32; frames * 2];
    for block in samples.chunks_mut(512 * 2) {
        engine.render(block, 2);
    }
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    wav::write_wav_i16(&out, SR, 2, &samples).expect("could not write wav");
    println!(
        "wrote {out}: {} s, peak {peak:.3}, ended at beat {:.2}",
        options.seconds,
        handle.position_beats()
    );
}
