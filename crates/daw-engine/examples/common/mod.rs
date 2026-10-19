//! Argument parsing and sample loading shared by the examples.

use std::path::{Path, PathBuf};

use daw_engine::{SampleResolver, Song, Source, Waveform};

pub struct Options {
    pub set: String,
    /// Positional arguments after the set path (e.g. the output file for `render`).
    pub rest: Vec<String>,
    pub source: Source,
    pub seconds: f64,
    pub tempo: Option<f64>,
    pub waveform: Waveform,
    pub from_beat: f64,
    pub library: Option<PathBuf>,
}

const USAGE: &str = "<set.als> [--scene N | --arrangement] [--seconds S] [--from BEAT] \
                     [--tempo BPM] [--wave sine|saw] [--library DIR]";

impl Options {
    pub fn parse(usage_prefix: &str) -> Options {
        let mut args = std::env::args().skip(1);
        let set = args
            .next()
            .unwrap_or_else(|| panic!("usage: {usage_prefix} {USAGE}"));
        let mut options = Options {
            set,
            rest: Vec::new(),
            source: Source::Scene(0),
            seconds: 15.0,
            tempo: None,
            waveform: Waveform::Saw,
            from_beat: 0.0,
            library: None,
        };
        let number = |flag: &str, args: &mut dyn Iterator<Item = String>| -> f64 {
            args.next()
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(|| panic!("{flag} needs a number"))
        };
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--scene" => options.source = Source::Scene(number(&arg, &mut args) as usize),
                "--arrangement" => options.source = Source::Arrangement,
                "--seconds" => options.seconds = number(&arg, &mut args),
                "--from" => options.from_beat = number(&arg, &mut args),
                "--tempo" => options.tempo = Some(number(&arg, &mut args)),
                "--library" => {
                    options.library = Some(args.next().expect("--library needs a folder").into())
                }
                "--wave" => {
                    options.waveform = match args.next().as_deref() {
                        Some("sine") => Waveform::Sine,
                        Some("saw") => Waveform::Saw,
                        other => panic!("--wave must be sine or saw, got {other:?}"),
                    }
                }
                flag if flag.starts_with("--") => panic!("unknown option {flag:?}"),
                _ => options.rest.push(arg),
            }
        }
        options
    }

    /// Reads the set, builds the song for the chosen source, and loads its audio files.
    pub fn load_song(&self) -> Song {
        let project = daw_als::read_file(&self.set).unwrap_or_else(|e| panic!("{}: {e}", self.set));
        let mut song = Song::from_project(&project, self.source);

        let mut resolver = SampleResolver::new();
        if let Some(dir) = Path::new(&self.set).parent() {
            resolver = resolver.with_root(dir);
        }
        if let Some(library) = self.library_dir() {
            resolver = resolver.with_root(library);
        }
        let audio_regions: usize = song.regions.iter().map(Vec::len).sum();
        let report = song.load_samples(&resolver);
        println!(
            "{} tracks, {} MIDI events, {} audio clips ({} files loaded), {} beats at {} bpm",
            song.track_names.len(),
            song.events.len(),
            audio_regions,
            report.loaded,
            song.length_beats,
            song.tempo
        );
        for sample in &report.missing {
            eprintln!(
                "  missing sample: {} (library {:?}); pass --library DIR",
                sample.path, sample.pack
            );
        }
        for (path, error) in &report.failed {
            eprintln!("  could not decode {}: {error}", path.display());
        }
        song
    }

    /// `--library`, else `$LIVE_LIBRARY`, else the Live 12 Trial's Core Library on Windows.
    fn library_dir(&self) -> Option<PathBuf> {
        self.library
            .clone()
            .or_else(|| std::env::var_os("LIVE_LIBRARY").map(PathBuf::from))
            .or_else(|| {
                let trial =
                    PathBuf::from(r"C:\ProgramData\Ableton\Live 12 Trial\Resources\Core Library");
                trial.is_dir().then_some(trial)
            })
    }
}
