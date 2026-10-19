# diy_daw

A DAW written in Rust. Early stage.

| Crate | What it does |
|---|---|
| `daw-model` | Format-independent project model (tracks, clips, notes, warp markers) |
| `daw-als` | Reads Ableton Live Set (`.als`) files into the model |
| `daw-engine` | Realtime engine: transport (play/stop/tempo/seek/loop), MIDI clip sequencer with a sine/saw synth, audio clip playback (WAV/AIFF/Ogg/FLAC/MP3) following warp markers and loop settings, pitch-preserving time-stretch, cpal output |

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

## Time-stretching

Warped audio clips follow the tempo without changing pitch, using [Signalsmith Stretch](https://github.com/Signalsmith-Audio/signalsmith-stretch)
(MIT) through the `ssstretch` crate. Clips in Re-Pitch mode and unwarped clips are resampled instead (pitch follows speed, as in Live).

The stretcher is C++ and is built with your C++ toolchain (MSVC Build Tools on Windows, `cc`/`c++` elsewhere).
`.cargo/config.toml` passes `/Zc:__cplusplus` for MSVC, without which the crate fails to compile. To build without it:
`cargo build -p daw-engine --no-default-features` (warped clips are then resampled).

## Current limits

- Devices, sends and automation are not played; every MIDI track uses the same simple synth; MIDI clip loop settings are approximated.
- All warp modes except Re-Pitch use the same stretch algorithm (Beats, Tones, Texture, Complex and Complex Pro are not distinguished).
- The stretcher delays its output by ~120 ms, so the engine feeds it audio from ahead of the playhead. Seeks, starting playback inside a clip,
  and tempo jumps of more than 15% restart the stream (a brief glitch); smaller tempo changes are followed smoothly.
- Loop and song-loop wraps are spliced in the stretcher's input without a crossfade, so they land on the beat but may be audible on some material.
- Clip transposition (`PitchCoarse`/`PitchFine`) works for stretched clips only.
- Timing of stretched audio is verified to within 20 ms on synthetic tones and by correlation (0.997) against a real vocal; it has not been checked by ear.
