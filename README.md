# diy_daw

A DAW written in Rust. Early stage.

| Crate | What it does |
|---|---|
| `daw-model` | Format-independent project model (tracks, clips, notes, warp markers) |
| `daw-als` | Reads Ableton Live Set (`.als`) files into the model |
| `daw-engine` | Realtime engine: transport (play/stop/tempo/seek/loop), MIDI clip sequencer, sine/saw synth, cpal output |

```bash
# Play the first scene of a Live Set through the default audio device
cargo run --release -p daw-engine --example play -- path/to/set.als --scene 0 --seconds 15

# Render to a WAV file without an audio device
cargo run --release -p daw-engine --example render -- path/to/set.als out.wav 10 scene 0

# Print what was read from a set
cargo run -p daw-als --example dump -- path/to/set.als
```

Notes on the Live Set format are in [`docs/live12`](docs/live12).

Current limits: only MIDI clips are played (audio clips, devices, sends and automation are not),
every MIDI track uses the same simple synth, and clip loop settings are approximated.
