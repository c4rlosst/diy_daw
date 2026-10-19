//! Plays a Live Set's MIDI clips through the default audio device.
//!
//! `cargo run -p daw-engine --example play -- <set.als> [--scene N | --arrangement]
//!     [--seconds S] [--tempo BPM] [--wave sine|saw]`

use std::time::{Duration, Instant};

use daw_engine::{AudioOutput, Song, Source, Waveform};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: play <set.als> [--scene N | --arrangement] [--seconds S] [--tempo BPM] [--wave sine|saw]");
    let mut source = Source::Scene(0);
    let mut seconds = 15.0;
    let mut tempo = None;
    let mut waveform = Waveform::Saw;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--scene" => {
                source = Source::Scene(
                    args.next()
                        .and_then(|v| v.parse().ok())
                        .expect("--scene needs a number"),
                )
            }
            "--arrangement" => source = Source::Arrangement,
            "--seconds" => {
                seconds = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .expect("--seconds needs a number")
            }
            "--tempo" => {
                tempo = Some(
                    args.next()
                        .and_then(|v| v.parse().ok())
                        .expect("--tempo needs a number"),
                )
            }
            "--wave" => {
                waveform = match args.next().as_deref() {
                    Some("sine") => Waveform::Sine,
                    Some("saw") => Waveform::Saw,
                    other => panic!("--wave must be sine or saw, got {other:?}"),
                }
            }
            other => panic!("unknown argument {other:?}"),
        }
    }

    let project = daw_als::read_file(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let song = Song::from_project(&project, source);
    println!(
        "{} tracks, {} events, {} beats at {} bpm",
        song.track_names.len(),
        song.events.len(),
        song.length_beats,
        song.tempo
    );

    let (output, engine) = AudioOutput::start().expect("could not open an audio output");
    println!(
        "playing on {} ({} Hz, {} ch)",
        output.device_name, output.sample_rate, output.channels
    );
    engine.load_song(song);
    engine.set_waveform(waveform);
    if let Some(bpm) = tempo {
        engine.set_tempo(bpm);
    }
    engine.play();

    let start = Instant::now();
    while start.elapsed().as_secs_f64() < seconds {
        std::thread::sleep(Duration::from_millis(500));
        println!("  beat {:7.2}", engine.position_beats());
    }
    engine.stop();
    std::thread::sleep(Duration::from_millis(200));
}
