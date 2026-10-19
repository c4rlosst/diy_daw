//! Loads real samples for a factory demo set, if a Live install is present.
//! Set `LIVE_LIBRARY` to the "Core Library" folder to override the default Windows trial path.

use std::path::Path;

use daw_engine::engine::Engine;
use daw_engine::{SampleResolver, Song, Source};
use daw_model::WarpMode;

fn library() -> Option<String> {
    let dir = std::env::var("LIVE_LIBRARY")
        .unwrap_or_else(|_| r"C:\ProgramData\Ableton\Live 12 Trial\Resources\Core Library".into());
    Path::new(&dir).is_dir().then_some(dir)
}

#[test]
fn demo_song_vocals_load_and_play() {
    let Some(lib) = library() else {
        eprintln!("no Live library found; skipping");
        return;
    };
    let set =
        Path::new(&lib).join("Lessons/Demo Songs/Chuck Sutton - Patience (Live 12 Suite Demo).als");
    let project = daw_als::read_file(&set).unwrap();
    let mut song = Song::from_project(&project, Source::Arrangement);

    let audio_tracks: Vec<_> = song.regions.iter().filter(|r| !r.is_empty()).collect();
    assert!(
        !audio_tracks.is_empty(),
        "no audio regions in the arrangement"
    );

    let report = song.load_samples(&SampleResolver::new().with_root(&lib));
    assert!(report.missing.is_empty(), "missing: {:?}", report.missing);
    assert!(report.failed.is_empty(), "failed: {:?}", report.failed);
    assert!(report.loaded >= 3, "only {} files loaded", report.loaded);
    assert!(song.regions.iter().flatten().all(|r| r.sample.is_some()));

    // Jump to a part of the song where the lead vocal is singing, and check there is sound.
    let (mut engine, handle) = Engine::new(48_000);
    handle.load_song(song);
    handle.seek(60.0);
    handle.play();
    let mut out = vec![0.0f32; 2 * 48_000];
    for block in out.chunks_mut(2 * 512) {
        engine.render(block, 2);
    }
    let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(
        peak.is_finite() && peak > 0.01 && peak <= 1.0,
        "peak {peak}"
    );
}

/// Renders the demo's "Vocal Main" clip at beat 323 on its own and compares it with the raw
/// file. The clip has loop start 3 beats and markers (31.6168 s = beat 0) and (31.6280 s =
/// beat 1/32), i.e. 167 bpm, the song's tempo, so it should play back its source unchanged,
/// starting where the warp markers and loop start say. Returns (correlation, fitted gain,
/// track gain).
fn vocal_vs_source(mode: WarpMode) -> Option<(f64, f64, f32)> {
    let lib = library()?;
    let set =
        Path::new(&lib).join("Lessons/Demo Songs/Chuck Sutton - Patience (Live 12 Suite Demo).als");
    let project = daw_als::read_file(&set).unwrap();
    let mut song = Song::from_project(&project, Source::Arrangement);
    song.load_samples(&SampleResolver::new().with_root(&lib));

    let track = song
        .track_names
        .iter()
        .position(|n| n == "Vocal Main")
        .unwrap();
    let gain = song.track_gains[track];
    let region = song.regions[track]
        .iter_mut()
        .find(|r| r.start == 323.0)
        .unwrap();
    region.warp_mode = mode;
    let region = region.clone();
    let source = region.sample.clone().unwrap();
    let seconds_per_beat = 0.011_227_544_910_179_64 / 0.03125;
    let first_second = 31.616_766_467_065_87 + 3.0 * seconds_per_beat;
    assert!((region.source_seconds(323.0, 167.0) - first_second).abs() < 1e-6);

    // Play only that clip.
    song.events.clear();
    for (i, regions) in song.regions.iter_mut().enumerate() {
        if i != track {
            regions.clear();
        }
    }
    let (mut engine, handle) = Engine::new(48_000);
    handle.load_song(song);
    handle.seek(323.0);
    handle.play();
    let frames = 48_000 * 3;
    let mut out = vec![0.0f32; 2 * frames];
    for block in out.chunks_mut(2 * 512) {
        engine.render(block, 2);
    }

    // Compare the left channel with the file, skipping the 2 ms fade-in.
    let (mut dot_os, mut dot_ss, mut dot_oo) = (0.0f64, 0.0f64, 0.0f64);
    for i in 480..frames {
        let o = f64::from(out[2 * i]);
        let s = f64::from(source.frame((first_second + i as f64 / 48_000.0) * 48_000.0)[0]);
        dot_os += o * s;
        dot_ss += s * s;
        dot_oo += o * o;
    }
    assert!(dot_ss > 1e-3, "source slice is silent; wrong position?");
    Some((dot_os / (dot_ss * dot_oo).sqrt(), dot_os / dot_ss, gain))
}

/// Re-Pitch mode reads the file by position, so at native tempo it must reproduce it exactly.
#[test]
fn resampled_vocal_matches_the_source_file() {
    let Some((correlation, fitted_gain, gain)) = vocal_vs_source(WarpMode::RePitch) else {
        eprintln!("no Live library found; skipping");
        return;
    };
    assert!(correlation > 0.999, "correlation {correlation}");
    assert!(
        (fitted_gain - f64::from(gain)).abs() < 0.01,
        "gain {fitted_gain} vs track {gain}"
    );
}

/// The time-stretcher is not bit-exact, but at native tempo on real singing it should still
/// follow the file closely. A timing error of even a millisecond would pull this well down.
#[cfg(feature = "stretch")]
#[test]
fn stretched_vocal_follows_the_source_file() {
    let Some((correlation, fitted_gain, gain)) = vocal_vs_source(WarpMode::Complex) else {
        eprintln!("no Live library found; skipping");
        return;
    };
    assert!(correlation > 0.99, "correlation {correlation}");
    assert!(
        (fitted_gain - f64::from(gain)).abs() < 0.05,
        "gain {fitted_gain} vs track {gain}"
    );
}
