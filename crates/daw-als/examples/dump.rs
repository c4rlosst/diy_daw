//! Prints a summary of a Live Set: `cargo run -p daw-als --example dump -- path/to/set.als`

use daw_model::ClipContent;

fn main() {
    let path = std::env::args().nth(1).expect("usage: dump <file.als>");
    let project = daw_als::read_file(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    println!(
        "tempo {} bpm, {} scenes",
        project.tempo, project.scene_count
    );
    for t in project.tracks.iter().chain(&project.return_tracks) {
        let notes: usize = t
            .session_clips
            .iter()
            .flatten()
            .chain(&t.arrangement_clips)
            .map(|c| c.note_count())
            .sum();
        let audio = t
            .session_clips
            .iter()
            .flatten()
            .chain(&t.arrangement_clips)
            .filter(|c| matches!(c.content, ClipContent::Audio { .. }))
            .count();
        println!(
            "{:?} {:?}: vol {:.3}, devices {:?}, session clips {}, arrangement clips {}, notes {}, audio clips {}",
            t.kind,
            t.name,
            t.volume,
            t.devices,
            t.session_clip_count(),
            t.arrangement_clips.len(),
            notes,
            audio
        );
    }
}
