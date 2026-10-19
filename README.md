# diy_daw

A DAW written in Rust. Early stage.

| Crate | What it does |
|---|---|
| `daw-model` | Format-independent project model (tracks, clips, notes, warp markers) |
| `daw-als` | Reads Ableton Live Set (`.als`) files into the model |
| `daw-engine` | Realtime engine: transport (play/stop/tempo/seek/loop), MIDI clip sequencer with a sine/saw synth, audio clip playback (WAV/AIFF/Ogg/FLAC/MP3) following warp markers and loop settings, cpal output |

```bash
# Play the first scene of a Live Set through the default audio device
cargo run --release -p daw-engine --example play -- path/to/set.als --scene 0 --seconds 15

# Play the arrangement from beat 296 (e.g. the vocals of the Patience demo)
cargo run --release -p daw-engine --example play -- path/to/set.als --arrangement --from 296

# Render to a WAV file without an audio device
cargo run --release -p daw-engine --example render -- path/to/set.als out.wav --scene 0 --seconds 10

# Print what was read from a set
cargo run -p daw-als --example dump -- path/to/set.als
```

Audio clips are found relative to the set's folder and to a library folder: pass `--library DIR`, set `LIVE_LIBRARY`,
or rely on the default Live 12 Trial location on Windows.

Notes on the Live Set format are in [`docs/live12`](docs/live12).

Current limits: audio clips play by resampling, so their pitch follows the tempo (like Live's Re-Pitch mode; time-stretching is next),
clip pitch/transposition is ignored, devices, sends and automation are not played, MIDI clip loop settings are approximated,
and every MIDI track uses the same simple synth.
