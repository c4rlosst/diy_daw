//! Audio clip playback: warp maps, tempo, regions on the timeline, stop fades.

use std::sync::Arc;

use daw_engine::engine::Engine;
use daw_engine::song::{Song, Source};
use daw_engine::Sample;
use daw_model::{
    Clip, ClipContent, ClipLoop, Project, SampleRef, Track, TrackKind, WarpMarker, WarpMode,
};

const SR: u32 = 48_000;
const TONE_HZ: f64 = 440.0;

/// Mono sine, 4 seconds at 48 kHz, amplitude 0.5.
fn tone() -> Arc<Sample> {
    let data = (0..SR * 4)
        .map(|i| {
            (f64::from(i) / f64::from(SR) * TONE_HZ * std::f64::consts::TAU).sin() as f32 * 0.5
        })
        .collect();
    Arc::new(Sample::from_interleaved(SR, 1, data))
}

fn audio_clip(start: f64, end: f64, warped: bool, looping: ClipLoop) -> Clip {
    // 120 bpm: 0.5 s per beat.
    let warp_markers = vec![
        WarpMarker {
            sec_time: 0.0,
            beat_time: 0.0,
        },
        WarpMarker {
            sec_time: 0.5,
            beat_time: 1.0,
        },
    ];
    Clip {
        name: "tone".into(),
        start,
        end,
        looping,
        content: ClipContent::Audio {
            sample: SampleRef {
                path: "tone.wav".into(),
                pack: String::new(),
            },
            sample_rate: SR,
            sample_frames: u64::from(SR) * 4,
            warped,
            warp_markers,
            warp_mode: WarpMode::RePitch,
            gain: 1.0,
            pitch_semitones: 0.0,
        },
    }
}

fn play_song(
    arrangement: Vec<Clip>,
    session: Vec<Option<Clip>>,
    source: Source,
    tempo: f64,
) -> Song {
    let track = Track {
        id: 1,
        name: "audio".into(),
        kind: TrackKind::Audio,
        volume: 1.0,
        devices: vec![],
        session_clips: session,
        arrangement_clips: arrangement,
    };
    let project = Project {
        tempo,
        tracks: vec![track],
        return_tracks: vec![],
        scene_count: 1,
    };
    let mut song = Song::from_project(&project, source);
    for region in song.regions.iter_mut().flatten() {
        region.sample = Some(tone());
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

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0, |m, s| m.max(s.abs()))
}

fn rising_zero_crossings(samples: &[f32]) -> usize {
    samples
        .windows(2)
        .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
        .count()
}

fn no_loop(end: f64) -> ClipLoop {
    ClipLoop {
        start: 0.0,
        end,
        start_relative: 0.0,
        enabled: false,
    }
}

#[test]
fn warped_clip_plays_at_native_pitch_when_tempo_matches_its_markers() {
    let song = play_song(
        vec![audio_clip(0.0, 16.0, true, no_loop(16.0))],
        vec![],
        Source::Arrangement,
        120.0,
    );
    let (mut engine, handle) = Engine::new(SR);
    handle.load_song(song);
    handle.play();
    let out = render(&mut engine, 1.0);
    let crossings = rising_zero_crossings(&out);
    assert!(
        (438..=442).contains(&crossings),
        "{crossings} crossings in 1 s"
    );
    assert!(peak(&out) > 0.4);
}

#[test]
fn doubling_the_tempo_doubles_the_speed_and_pitch() {
    let song = play_song(
        vec![audio_clip(0.0, 16.0, true, no_loop(16.0))],
        vec![],
        Source::Arrangement,
        120.0,
    );
    let (mut engine, handle) = Engine::new(SR);
    handle.load_song(song);
    handle.set_tempo(240.0);
    handle.play();
    let crossings = rising_zero_crossings(&render(&mut engine, 1.0));
    assert!(
        (876..=884).contains(&crossings),
        "{crossings} crossings in 1 s"
    );
}

#[test]
fn unwarped_clip_ignores_the_tempo() {
    let song = play_song(
        vec![audio_clip(0.0, 16.0, false, no_loop(4.0))],
        vec![],
        Source::Arrangement,
        120.0,
    );
    let (mut engine, handle) = Engine::new(SR);
    handle.load_song(song);
    handle.set_tempo(240.0);
    handle.play();
    let crossings = rising_zero_crossings(&render(&mut engine, 0.5));
    // Native speed: 440 Hz for 0.5 s of output, even though the tempo doubled.
    assert!(
        (218..=222).contains(&crossings),
        "{crossings} crossings in 0.5 s"
    );
}

#[test]
fn clip_sounds_only_inside_its_place_on_the_timeline() {
    // Beats 2..4 at 120 bpm = 1.0 s..2.0 s. The song is longer so it does not wrap.
    let clips = vec![
        audio_clip(2.0, 4.0, true, no_loop(2.0)),
        audio_clip(20.0, 21.0, true, no_loop(1.0)),
    ];
    let song = play_song(clips, vec![], Source::Arrangement, 120.0);
    let (mut engine, handle) = Engine::new(SR);
    handle.load_song(song);
    handle.play();
    assert_eq!(
        peak(&render(&mut engine, 0.98)),
        0.0,
        "sound before the clip"
    );
    assert!(peak(&render(&mut engine, 0.1)) > 0.4, "clip did not start");
    render(&mut engine, 0.95); // now at 2.03 s; the clip ended at 2.0 s
    assert!(
        peak(&render(&mut engine, 0.1)) < 1e-6,
        "sound after the clip ended"
    );
}

#[test]
fn looping_session_clip_keeps_sounding_past_its_first_pass() {
    // 2-beat loop, started at the loop start: after the first 2 beats it repeats forever.
    let looping = ClipLoop {
        start: 0.0,
        end: 2.0,
        start_relative: 0.0,
        enabled: true,
    };
    let song = play_song(
        vec![],
        vec![Some(audio_clip(0.0, 2.0, true, looping))],
        Source::Scene(0),
        120.0,
    );
    assert!(song.regions[0][0].end.is_infinite());
    let (mut engine, handle) = Engine::new(SR);
    handle.set_looping(false); // let the playhead run on past the song length
    handle.load_song(song);
    handle.play();
    render(&mut engine, 3.0);
    assert!(
        peak(&render(&mut engine, 0.1)) > 0.4,
        "loop stopped after the first pass"
    );
}

#[test]
fn stop_fades_audio_out_without_a_click() {
    let song = play_song(
        vec![audio_clip(0.0, 16.0, true, no_loop(16.0))],
        vec![],
        Source::Arrangement,
        120.0,
    );
    let (mut engine, handle) = Engine::new(SR);
    handle.load_song(song);
    handle.play();
    let mut out = render(&mut engine, 0.5);
    handle.stop();
    out.extend(render(&mut engine, 0.1));
    // The loudest natural step of a 440 Hz sine at this level is about 0.03.
    let biggest_step = out
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(biggest_step < 0.05, "click of size {biggest_step}");
    assert_eq!(
        peak(&out[out.len() - 2_400..]),
        0.0,
        "audio did not fade to silence"
    );
    assert!(
        (handle.position_beats() - 1.0).abs() < 1e-3,
        "{}",
        handle.position_beats()
    );
}

#[test]
fn regions_without_a_sample_are_silent_not_fatal() {
    let mut song = play_song(
        vec![audio_clip(0.0, 4.0, true, no_loop(4.0))],
        vec![],
        Source::Arrangement,
        120.0,
    );
    song.regions[0][0].sample = None;
    let (mut engine, handle) = Engine::new(SR);
    handle.load_song(song);
    handle.play();
    assert_eq!(peak(&render(&mut engine, 0.5)), 0.0);
}

#[test]
fn song_length_covers_audio_clips() {
    let song = play_song(
        vec![audio_clip(8.0, 12.0, true, no_loop(4.0))],
        vec![],
        Source::Arrangement,
        120.0,
    );
    assert_eq!(song.length_beats, 12.0);
    assert_eq!(song.regions[0][0].start, 8.0);
    assert_eq!(song.regions[0][0].end, 12.0);
}

#[test]
fn stereo_output_keeps_left_and_right_separate() {
    // Left: sine; right: silence.
    let data: Vec<f32> = (0..SR * 2)
        .flat_map(|i| {
            [
                (f64::from(i) / f64::from(SR) * TONE_HZ * std::f64::consts::TAU).sin() as f32 * 0.5,
                0.0,
            ]
        })
        .collect();
    let mut song = play_song(
        vec![audio_clip(0.0, 16.0, true, no_loop(16.0))],
        vec![],
        Source::Arrangement,
        120.0,
    );
    song.regions[0][0].sample = Some(Arc::new(Sample::from_interleaved(SR, 2, data)));
    let (mut engine, handle) = Engine::new(SR);
    handle.load_song(song);
    handle.play();
    let mut out = vec![0.0f32; 2 * 4_800];
    engine.render(&mut out, 2);
    let left = out.iter().step_by(2).fold(0.0f32, |m, s| m.max(s.abs()));
    let right = out
        .iter()
        .skip(1)
        .step_by(2)
        .fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(left > 0.3 && right == 0.0, "left {left}, right {right}");
}
