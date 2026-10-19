//! Time-stretched playback of warped clips: pitch, timing and transposition, measured on
//! synthetic tones (no listening required).

#![cfg(feature = "stretch")]

use std::sync::Arc;

use daw_engine::engine::Engine;
use daw_engine::song::{Song, Source};
use daw_engine::Sample;
use daw_model::{
    Clip, ClipContent, ClipLoop, Project, SampleRef, Track, TrackKind, WarpMarker, WarpMode,
};

const SR: u32 = 48_000;
const HZ: f64 = 440.0;

/// A 440 Hz sine of amplitude 0.5, optionally only sounding between two times (seconds).
fn sine(sample_rate: u32, seconds: f64, burst: Option<(f64, f64)>) -> Arc<Sample> {
    let frames = (seconds * f64::from(sample_rate)) as usize;
    let data = (0..frames)
        .map(|i| {
            let t = i as f64 / f64::from(sample_rate);
            let on = burst.is_none_or(|(from, to)| t >= from && t < to);
            if on {
                (t * HZ * std::f64::consts::TAU).sin() as f32 * 0.5
            } else {
                0.0
            }
        })
        .collect();
    Arc::new(Sample::from_interleaved(sample_rate, 1, data))
}

/// A warped clip at 120 bpm (0.5 s of source per beat).
fn clip(len: f64, mode: WarpMode, pitch: f64, looping: Option<ClipLoop>) -> Clip {
    Clip {
        name: "tone".into(),
        start: 0.0,
        end: len,
        looping: looping.unwrap_or(ClipLoop {
            start: 0.0,
            end: len,
            start_relative: 0.0,
            enabled: false,
        }),
        content: ClipContent::Audio {
            sample: SampleRef::default(),
            sample_rate: 0,
            sample_frames: 0,
            warped: true,
            warp_markers: vec![
                WarpMarker {
                    sec_time: 0.0,
                    beat_time: 0.0,
                },
                WarpMarker {
                    sec_time: 0.5,
                    beat_time: 1.0,
                },
            ],
            warp_mode: mode,
            gain: 1.0,
            pitch_semitones: pitch,
        },
    }
}

fn song(clip: Clip, source: Source, sample: Arc<Sample>) -> Song {
    let (arrangement, session) = match source {
        Source::Arrangement => (vec![clip], vec![]),
        Source::Scene(_) => (vec![], vec![Some(clip)]),
    };
    let track = Track {
        id: 1,
        name: "a".into(),
        kind: TrackKind::Audio,
        volume: 1.0,
        devices: vec![],
        session_clips: session,
        arrangement_clips: arrangement,
    };
    let project = Project {
        tempo: 120.0,
        tracks: vec![track],
        return_tracks: vec![],
        scene_count: 1,
    };
    let mut song = Song::from_project(&project, source);
    for region in song.regions.iter_mut().flatten() {
        region.sample = Some(sample.clone());
    }
    song
}

fn render(engine: &mut Engine, seconds: f64) -> Vec<f32> {
    let frames = (seconds * f64::from(SR)).round() as usize;
    let mut out = vec![0.0; frames];
    for block in out.chunks_mut(509) {
        engine.render(block, 1);
    }
    out
}

/// Frequency of a roughly sinusoidal signal, from rising zero crossings.
fn frequency(samples: &[f32]) -> f64 {
    let crossings = samples
        .windows(2)
        .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
        .count();
    crossings as f64 / (samples.len() as f64 / f64::from(SR))
}

/// Time (s) at which the 5 ms RMS envelope first reaches half of its maximum.
fn onset(samples: &[f32]) -> f64 {
    let win = (0.005 * f64::from(SR)) as usize;
    let rms: Vec<f64> = samples
        .chunks(win)
        .map(|c| (c.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / c.len() as f64).sqrt())
        .collect();
    let peak = rms.iter().cloned().fold(0.0, f64::max);
    assert!(peak > 0.05, "no signal (peak rms {peak})");
    rms.iter().position(|&r| r > 0.5 * peak).unwrap() as f64 * 0.005
}

fn playing(song: Song, tempo: f64) -> (Engine, daw_engine::EngineHandle) {
    let (engine, handle) = Engine::new(SR);
    handle.load_song(song);
    handle.set_tempo(tempo);
    handle.play();
    (engine, handle)
}

fn assert_near(value: f64, expected: f64, tolerance: f64, what: &str) {
    assert!(
        (value - expected).abs() <= tolerance,
        "{what}: {value} (expected {expected} ± {tolerance})"
    );
}

fn biggest_step(samples: &[f32]) -> f32 {
    samples
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

#[test]
fn pitch_is_preserved_when_the_tempo_is_slower_or_faster() {
    for tempo in [96.0, 120.0, 150.0] {
        let s = song(
            clip(32.0, WarpMode::Complex, 0.0, None),
            Source::Arrangement,
            sine(SR, 16.0, None),
        );
        let (mut engine, _h) = playing(s, tempo);
        render(&mut engine, 1.0); // let the stream settle
        let hz = frequency(&render(&mut engine, 1.0));
        assert_near(hz, HZ, 0.01 * HZ, &format!("pitch at {tempo} bpm"));
    }
}

#[test]
fn re_pitch_mode_still_changes_pitch_with_tempo() {
    let s = song(
        clip(32.0, WarpMode::RePitch, 0.0, None),
        Source::Arrangement,
        sine(SR, 16.0, None),
    );
    let (mut engine, _h) = playing(s, 96.0);
    render(&mut engine, 0.5);
    assert_near(
        frequency(&render(&mut engine, 1.0)),
        HZ * 0.8,
        0.01 * HZ,
        "re-pitch at 96 bpm",
    );
}

#[test]
fn audio_stays_aligned_with_the_timeline() {
    // A burst starts 1.0 s into the file (beat 2 of the warp map).
    let burst = Some((1.0, 1.5));
    for tempo in [120.0, 96.0, 150.0] {
        let s = song(
            clip(32.0, WarpMode::Complex, 0.0, None),
            Source::Arrangement,
            sine(SR, 8.0, burst),
        );
        let (mut engine, _h) = playing(s, tempo);
        let out = render(&mut engine, 3.0);
        assert_near(
            onset(&out),
            2.0 * 60.0 / tempo,
            0.02,
            &format!("onset at {tempo} bpm"),
        );
    }
}

#[test]
fn audio_is_aligned_after_seeking_into_the_clip() {
    let s = song(
        clip(32.0, WarpMode::Complex, 0.0, None),
        Source::Arrangement,
        sine(SR, 8.0, Some((1.0, 1.5))),
    );
    let (mut engine, handle) = Engine::new(SR);
    handle.load_song(s);
    handle.seek(1.0); // source 0.5 s; the burst is 0.5 s ahead
    handle.play();
    let out = render(&mut engine, 2.0);
    assert_near(onset(&out), 0.5, 0.02, "onset after seek");
}

#[test]
fn clip_transposition_is_applied_without_changing_timing() {
    let s = song(
        clip(32.0, WarpMode::Complex, 12.0, None),
        Source::Arrangement,
        sine(SR, 8.0, Some((1.0, 2.0))),
    );
    let (mut engine, _h) = playing(s, 120.0);
    let out = render(&mut engine, 3.0);
    assert_near(onset(&out), 1.0, 0.02, "onset with +12 semitones");
    let burst = &out[(1.3 * f64::from(SR)) as usize..(1.9 * f64::from(SR)) as usize];
    assert_near(frequency(burst), 2.0 * HZ, 0.02 * HZ, "octave up");
}

#[test]
fn files_at_another_sample_rate_keep_their_pitch() {
    let s = song(
        clip(32.0, WarpMode::Complex, 0.0, None),
        Source::Arrangement,
        sine(44_100, 16.0, None),
    );
    let (mut engine, _h) = playing(s, 120.0);
    render(&mut engine, 1.0);
    assert_near(
        frequency(&render(&mut engine, 1.0)),
        HZ,
        0.01 * HZ,
        "44.1 kHz file on a 48 kHz engine",
    );
}

#[test]
fn tempo_changes_while_playing_keep_the_pitch_and_do_not_click() {
    let s = song(
        clip(64.0, WarpMode::Complex, 0.0, None),
        Source::Arrangement,
        sine(SR, 30.0, None),
    );
    let (mut engine, handle) = playing(s, 120.0);
    render(&mut engine, 1.0);
    let before = render(&mut engine, 1.0);
    handle.set_tempo(132.0); // +10%: below the re-sync threshold
    let during = render(&mut engine, 1.0);
    handle.set_tempo(180.0); // +36%: forces a re-sync
    render(&mut engine, 0.5);
    let after = render(&mut engine, 1.0);
    for (name, part) in [("before", &before), ("during", &during), ("after", &after)] {
        assert_near(frequency(part), HZ, 0.01 * HZ, name);
    }
    // A sine of amplitude 0.5 steps by at most 0.03 per sample; allow some slack for the
    // stretcher's own ripple but catch clicks.
    let (a, b) = (biggest_step(&before), biggest_step(&during));
    assert!(a < 0.06 && b < 0.06, "click: {a} {b}");
}

#[test]
fn looping_session_clip_stays_continuous_across_loop_and_song_wraps() {
    // 440 Hz for exactly 1 s makes a seamless 2-beat loop at 120 bpm.
    let looping = ClipLoop {
        start: 0.0,
        end: 2.0,
        start_relative: 0.0,
        enabled: true,
    };
    let s = song(
        clip(2.0, WarpMode::Complex, 0.0, Some(looping)),
        Source::Scene(0),
        sine(SR, 1.0, None),
    );
    assert_eq!(s.length_beats, 2.0);
    let (mut engine, _h) = playing(s, 120.0);
    render(&mut engine, 1.0);
    let out = render(&mut engine, 3.0); // crosses three song wraps
    assert_near(frequency(&out), HZ, 0.01 * HZ, "pitch across wraps");
    let step = biggest_step(&out);
    assert!(step < 0.08, "discontinuity at a wrap: step {step}");
    assert!(out.iter().all(|s| s.is_finite()));
}

#[test]
fn a_clip_that_starts_later_is_gated_to_its_place_on_the_timeline() {
    let mut c = clip(8.0, WarpMode::Complex, 0.0, None);
    c.start = 4.0; // timeline beat 4 = 2 s at 120 bpm
    c.end = 12.0;
    let s = song(c, Source::Arrangement, sine(SR, 8.0, None));
    let (mut engine, _h) = playing(s, 120.0);
    let out = render(&mut engine, 3.0);
    let quiet = out[..(1.9 * f64::from(SR)) as usize]
        .iter()
        .fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(quiet < 1e-4, "sound before the clip: {quiet}");
    assert_near(onset(&out), 2.0, 0.02, "clip start");
}
