use daw_engine::engine::Engine;
use daw_engine::song::{EventKind, Song, Source};
use daw_engine::synth::Waveform;
use daw_model::{Clip, ClipContent, ClipLoop, Note, Project, Track, TrackKind};

const SR: u32 = 48_000;

fn note(key: u8, time: f64, duration: f64) -> Note {
    Note {
        key,
        time,
        duration,
        velocity: 100.0,
    }
}

fn clip(len: f64, notes: Vec<Note>) -> Clip {
    Clip {
        name: String::new(),
        start: 0.0,
        end: len,
        looping: ClipLoop::default(),
        content: ClipContent::Midi { notes },
    }
}

fn project(clips: Vec<Clip>, tempo: f64) -> Project {
    let track = Track {
        id: 1,
        name: "t".into(),
        kind: TrackKind::Midi,
        volume: 1.0,
        devices: vec![],
        session_clips: clips.into_iter().map(Some).collect(),
        arrangement_clips: vec![],
    };
    Project {
        tempo,
        tracks: vec![track],
        return_tracks: vec![],
        scene_count: 1,
    }
}

/// Renders `seconds` of mono audio in odd-sized blocks, as a real callback would.
fn render(engine: &mut Engine, seconds: f64) -> Vec<f32> {
    let frames = (seconds * f64::from(SR)).round() as usize;
    let mut out = vec![0.0; frames];
    for block in out.chunks_mut(441) {
        engine.render(block, 1);
    }
    out
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0, |m, s| m.max(s.abs()))
}

#[test]
fn song_repeats_session_clip_and_sorts_note_offs_first() {
    let p = project(
        vec![clip(2.0, vec![note(60, 0.0, 1.0), note(60, 1.0, 1.0)])],
        120.0,
    );
    let song = Song::from_project(&p, Source::Scene(0));
    assert_eq!(song.length_beats, 2.0);
    // on@0, off@1, on@1, (off@2 is the loop end and is left to the wrap)
    let kinds: Vec<_> = song
        .events
        .iter()
        .map(|e| (e.beat, matches!(e.kind, EventKind::NoteOn { .. })))
        .collect();
    assert_eq!(kinds, vec![(0.0, true), (1.0, false), (1.0, true)]);
}

#[test]
fn song_arrangement_places_clips_on_the_timeline() {
    let mut p = project(vec![], 100.0);
    p.tracks[0].arrangement_clips = vec![Clip {
        start: 8.0,
        end: 12.0,
        ..clip(4.0, vec![note(64, 1.0, 1.0)])
    }];
    let song = Song::from_project(&p, Source::Arrangement);
    assert_eq!(song.length_beats, 12.0);
    assert_eq!(song.events[0].beat, 9.0);
    assert_eq!(song.tempo, 100.0);
}

#[test]
fn silent_until_the_first_note_then_sounds() {
    let (mut engine, handle) = Engine::new(SR);
    handle.load_song(Song::from_project(
        &project(vec![clip(8.0, vec![note(69, 2.0, 1.0)])], 120.0),
        Source::Scene(0),
    ));
    handle.play();
    // 120 bpm: beat 2 is at 1.0 s.
    assert_eq!(
        peak(&render(&mut engine, 0.98)),
        0.0,
        "sound before the note"
    );
    assert!(
        peak(&render(&mut engine, 0.1)) > 0.05,
        "no sound after the note started"
    );
}

#[test]
fn transport_follows_tempo() {
    let (mut engine, handle) = Engine::new(SR);
    handle.load_song(Song::from_project(
        &project(vec![clip(64.0, vec![])], 120.0),
        Source::Scene(0),
    ));
    handle.play();
    render(&mut engine, 1.0);
    assert!(
        (handle.position_beats() - 2.0).abs() < 1e-3,
        "{}",
        handle.position_beats()
    );
    handle.set_tempo(240.0);
    render(&mut engine, 1.0);
    assert!(
        (handle.position_beats() - 6.0).abs() < 1e-3,
        "{}",
        handle.position_beats()
    );
}

#[test]
fn loop_wraps_and_the_first_note_retriggers() {
    let (mut engine, handle) = Engine::new(SR);
    handle.load_song(Song::from_project(
        &project(vec![clip(4.0, vec![note(69, 0.0, 0.5)])], 120.0),
        Source::Scene(0),
    ));
    handle.set_waveform(Waveform::Sine);
    handle.play();
    // The loop is 2 s long. The note at beat 0 should sound at the start of every pass.
    for pass in 0..3 {
        let first_half_second = render(&mut engine, 0.25);
        assert!(
            peak(&first_half_second) > 0.05,
            "pass {pass}: first note missing"
        );
        let rest = render(&mut engine, 1.75);
        assert!(
            peak(&rest[4_800..]) < 1e-4,
            "pass {pass}: note should have ended"
        );
    }
    assert!(
        handle.position_beats() < 0.01,
        "{}",
        handle.position_beats()
    );
}

#[test]
fn stop_halts_then_rewinds_and_play_resumes() {
    let (mut engine, handle) = Engine::new(SR);
    handle.load_song(Song::from_project(
        &project(vec![clip(64.0, vec![note(60, 0.0, 60.0)])], 120.0),
        Source::Scene(0),
    ));
    handle.play();
    render(&mut engine, 0.5);
    handle.stop();
    let tail = render(&mut engine, 0.5);
    assert!(!handle.is_playing());
    assert_eq!(
        peak(&tail[tail.len() - 2_400..]),
        0.0,
        "release tail should have died out"
    );
    let paused_at = handle.position_beats();
    assert!((paused_at - 1.0).abs() < 1e-2, "{paused_at}");
    render(&mut engine, 0.5);
    assert_eq!(
        handle.position_beats(),
        paused_at,
        "must not advance while stopped"
    );

    handle.stop(); // second stop rewinds
    render(&mut engine, 0.01);
    assert_eq!(handle.position_beats(), 0.0);

    handle.play();
    assert!(peak(&render(&mut engine, 0.2)) > 0.05);
}

#[test]
fn output_never_exceeds_full_scale() {
    let notes: Vec<Note> = (0..40).map(|i| note(36 + i, 0.0, 4.0)).collect();
    let (mut engine, handle) = Engine::new(SR);
    handle.load_song(Song::from_project(
        &project(vec![clip(4.0, notes)], 120.0),
        Source::Scene(0),
    ));
    handle.play();
    let out = render(&mut engine, 2.0);
    assert!(out.iter().all(|s| s.is_finite()));
    assert!(peak(&out) <= 1.0);
}
