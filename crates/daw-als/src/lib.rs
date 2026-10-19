//! Reads Ableton Live Set (`.als`) files into [`daw_model::Project`].
//!
//! A `.als` file is gzip-compressed XML. Only the parts that map onto the
//! project model are read; everything else is ignored. See `docs/live12`.

use std::io::Read;
use std::path::Path;

use daw_model::{Clip, ClipContent, Note, Project, Track, TrackKind, WarpMarker};
use flate2::read::GzDecoder;
use roxmltree::{Document, Node};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("xml error: {0}")]
    Xml(#[from] roxmltree::Error),
    #[error("not a Live Set: {0}")]
    Format(String),
}

pub fn read_file(path: impl AsRef<Path>) -> Result<Project, Error> {
    read_bytes(&std::fs::read(path)?)
}

pub fn read_bytes(bytes: &[u8]) -> Result<Project, Error> {
    let mut xml = String::new();
    if bytes.starts_with(&[0x1f, 0x8b]) {
        GzDecoder::new(bytes).read_to_string(&mut xml)?;
    } else {
        xml = String::from_utf8_lossy(bytes).into_owned();
    }
    read_xml(&xml)
}

pub fn read_xml(xml: &str) -> Result<Project, Error> {
    let doc = Document::parse(xml)?;
    let root = doc.root_element();
    if root.tag_name().name() != "Ableton" {
        return Err(Error::Format(format!(
            "root element is <{}>",
            root.tag_name().name()
        )));
    }
    let live_set =
        child(root, "LiveSet").ok_or_else(|| Error::Format("missing <LiveSet>".into()))?;

    let tempo = child_path(
        live_set,
        &["MainTrack", "DeviceChain", "Mixer", "Tempo", "Manual"],
    )
    .and_then(value_f64)
    .unwrap_or(120.0);
    let scene_count =
        child(live_set, "Scenes").map_or(0, |s| s.children().filter(Node::is_element).count());

    let mut project = Project {
        tempo,
        scene_count,
        ..Project::default()
    };
    if let Some(tracks) = child(live_set, "Tracks") {
        for t in tracks.children().filter(Node::is_element) {
            let kind = match t.tag_name().name() {
                "MidiTrack" => TrackKind::Midi,
                "AudioTrack" => TrackKind::Audio,
                "GroupTrack" => TrackKind::Group,
                "ReturnTrack" => TrackKind::Return,
                _ => continue,
            };
            let track = read_track(t, kind, scene_count);
            if kind == TrackKind::Return {
                project.return_tracks.push(track);
            } else {
                project.tracks.push(track);
            }
        }
    }
    Ok(project)
}

fn read_track(t: Node, kind: TrackKind, scene_count: usize) -> Track {
    let chain = child(t, "DeviceChain");
    let devices = chain
        .and_then(|c| child_path(c, &["DeviceChain", "Devices"]))
        .map(|d| {
            d.children()
                .filter(Node::is_element)
                .map(|n| n.tag_name().name().to_string())
                .collect()
        })
        .unwrap_or_default();
    let sequencer = chain.and_then(|c| child(c, "MainSequencer"));

    let mut session_clips: Vec<Option<Clip>> = sequencer
        .and_then(|s| child(s, "ClipSlotList"))
        .map(|list| {
            list.children()
                .filter(|n| n.has_tag_name("ClipSlot"))
                .map(|slot| {
                    child_path(slot, &["ClipSlot", "Value"])
                        .and_then(|v| v.children().find(Node::is_element))
                        .and_then(read_clip)
                })
                .collect()
        })
        .unwrap_or_default();
    if session_clips.len() < scene_count {
        session_clips.resize(scene_count, None);
    }

    let arrangement_clips = sequencer
        .and_then(|s| {
            s.descendants()
                .find(|n| n.has_tag_name("ArrangerAutomation"))
        })
        .and_then(|a| child(a, "Events"))
        .map(|e| {
            e.children()
                .filter(Node::is_element)
                .filter_map(read_clip)
                .collect()
        })
        .unwrap_or_default();

    Track {
        id: t.attribute("Id").and_then(|v| v.parse().ok()).unwrap_or(-1),
        name: child_path(t, &["Name", "EffectiveName"])
            .and_then(|n| n.attribute("Value"))
            .unwrap_or("")
            .to_string(),
        kind,
        volume: chain
            .and_then(|c| child_path(c, &["Mixer", "Volume", "Manual"]))
            .and_then(value_f64)
            .unwrap_or(1.0),
        devices,
        session_clips,
        arrangement_clips,
    }
}

fn read_clip(c: Node) -> Option<Clip> {
    let content = match c.tag_name().name() {
        "MidiClip" => ClipContent::Midi {
            notes: read_notes(c),
        },
        "AudioClip" => ClipContent::Audio {
            sample_path: child_path(c, &["SampleRef", "FileRef", "RelativePath"])
                .and_then(|n| n.attribute("Value"))
                .unwrap_or("")
                .to_string(),
            warped: child(c, "IsWarped").and_then(|n| n.attribute("Value")) == Some("true"),
            warp_markers: child(c, "WarpMarkers")
                .map(|w| {
                    w.children()
                        .filter(|n| n.has_tag_name("WarpMarker"))
                        .filter_map(|m| {
                            Some(WarpMarker {
                                sec_time: m.attribute("SecTime")?.parse().ok()?,
                                beat_time: m.attribute("BeatTime")?.parse().ok()?,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
        },
        _ => return None,
    };
    Some(Clip {
        name: child(c, "Name")
            .and_then(|n| n.attribute("Value"))
            .unwrap_or("")
            .to_string(),
        start: child(c, "CurrentStart").and_then(value_f64).unwrap_or(0.0),
        end: child(c, "CurrentEnd").and_then(value_f64).unwrap_or(0.0),
        content,
    })
}

fn read_notes(clip: Node) -> Vec<Note> {
    let mut notes = Vec::new();
    let Some(key_tracks) = child_path(clip, &["Notes", "KeyTracks"]) else {
        return notes;
    };
    for kt in key_tracks.children().filter(|n| n.has_tag_name("KeyTrack")) {
        let Some(key) = child(kt, "MidiKey")
            .and_then(|n| n.attribute("Value"))
            .and_then(|v| v.parse::<u8>().ok())
        else {
            continue;
        };
        let Some(events) = child(kt, "Notes") else {
            continue;
        };
        for e in events
            .children()
            .filter(|n| n.has_tag_name("MidiNoteEvent"))
        {
            let f = |name: &str| e.attribute(name).and_then(|v| v.parse::<f64>().ok());
            if let (Some(time), Some(duration), Some(velocity)) =
                (f("Time"), f("Duration"), f("Velocity"))
            {
                notes.push(Note {
                    key,
                    time,
                    duration,
                    velocity: velocity as f32,
                });
            }
        }
    }
    notes.sort_by(|a, b| a.time.total_cmp(&b.time).then(a.key.cmp(&b.key)));
    notes
}

fn child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| c.has_tag_name(name))
}

fn child_path<'a, 'i>(mut n: Node<'a, 'i>, path: &[&str]) -> Option<Node<'a, 'i>> {
    for name in path {
        n = child(n, name)?;
    }
    Some(n)
}

fn value_f64(n: Node) -> Option<f64> {
    n.attribute("Value")?.parse().ok()
}
