//! Loads every `.als` found under a Live install's resources, if one is present.
//! Set `LIVE_RESOURCES` to override the default Windows trial path. Skips when absent.

use std::path::{Path, PathBuf};

fn find_als(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            find_als(&p, out);
        } else if p.extension().is_some_and(|x| x == "als") {
            out.push(p);
        }
    }
}

#[test]
fn all_factory_sets_load() {
    let root = std::env::var("LIVE_RESOURCES")
        .unwrap_or_else(|_| r"C:\ProgramData\Ableton\Live 12 Trial\Resources".into());
    let mut files = Vec::new();
    find_als(Path::new(&root), &mut files);
    if files.is_empty() {
        eprintln!("no .als files under {root}; skipping");
        return;
    }
    for f in &files {
        let p = daw_als::read_file(f).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        assert!(p.tempo >= 20.0 && p.tempo <= 999.0, "{}: tempo {}", f.display(), p.tempo);
    }
    eprintln!("loaded {} sets", files.len());
}

#[test]
fn minimal_set_parses() {
    let xml = r#"<Ableton><LiveSet>
        <MainTrack><DeviceChain><Mixer><Tempo><Manual Value="128"/></Tempo></Mixer></DeviceChain></MainTrack>
        <Tracks><MidiTrack Id="7"><Name><EffectiveName Value="Keys"/></Name><DeviceChain>
          <Mixer><Volume><Manual Value="0.5"/></Volume></Mixer>
          <MainSequencer><ClipSlotList><ClipSlot Id="0"><ClipSlot><Value>
            <MidiClip><CurrentStart Value="0"/><CurrentEnd Value="4"/><Name Value="c"/>
              <Notes><KeyTracks><KeyTrack Id="0"><Notes>
                <MidiNoteEvent Time="1" Duration="0.5" Velocity="100"/>
                <MidiNoteEvent Time="0" Duration="1" Velocity="90"/>
              </Notes><MidiKey Value="60"/></KeyTrack></KeyTracks></Notes>
            </MidiClip></Value></ClipSlot></ClipSlot></ClipSlotList></MainSequencer>
        </DeviceChain></MidiTrack></Tracks>
        <Scenes><Scene/><Scene/></Scenes></LiveSet></Ableton>"#;
    let p = daw_als::read_xml(xml).unwrap();
    assert_eq!(p.tempo, 128.0);
    assert_eq!(p.scene_count, 2);
    let t = &p.tracks[0];
    assert_eq!((t.id, t.name.as_str(), t.volume), (7, "Keys", 0.5));
    assert_eq!(t.session_clips.len(), 2);
    let clip = t.session_clips[0].as_ref().unwrap();
    assert_eq!(clip.note_count(), 2);
    assert!(t.session_clips[1].is_none());
    match &clip.content {
        daw_model::ClipContent::Midi { notes } => assert_eq!(notes[0].time, 0.0),
        _ => panic!("expected midi"),
    }
}
