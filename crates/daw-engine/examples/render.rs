//! Renders a Live Set's MIDI clips to a 16-bit WAV file without touching an audio device.
//!
//! `cargo run -p daw-engine --example render -- <set.als> <out.wav> [seconds] [scene N | arrangement]`

use daw_engine::engine::Engine;
use daw_engine::{wav, Song, Source};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(path), Some(out)) = (args.next(), args.next()) else {
        panic!("usage: render <set.als> <out.wav> [seconds] [scene N | arrangement]");
    };
    let seconds: f64 = args
        .next()
        .map_or(10.0, |v| v.parse().expect("seconds must be a number"));
    let source: Source = match args.next().as_deref() {
        None => Source::Scene(0),
        Some("scene") => args.next().expect("scene needs a number").parse().unwrap(),
        Some(other) => other.parse().unwrap(),
    };

    const SR: u32 = 48_000;
    let project = daw_als::read_file(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let song = Song::from_project(&project, source);
    println!(
        "{} events over {} beats at {} bpm",
        song.events.len(),
        song.length_beats,
        song.tempo
    );

    let (mut engine, handle) = Engine::new(SR);
    handle.load_song(song);
    handle.play();

    let frames = (seconds * f64::from(SR)) as usize;
    let mut samples = vec![0.0f32; frames * 2];
    for block in samples.chunks_mut(512 * 2) {
        engine.render(block, 2);
    }
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    wav::write_wav_i16(&out, SR, 2, &samples).expect("could not write wav");
    println!(
        "wrote {out}: {seconds} s, peak {peak:.3}, ended at beat {:.2}",
        handle.position_beats()
    );
}
