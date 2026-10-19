# Live 12 presets, audio clips, warp analysis, automation — observations

Companion to [als-format.md](als-format.md). Source: factory content in the Live 12 Trial install
(`Resources\Core Library`). Behavior documentation only.

## File families
| Ext | Count | Container | Contents |
|---|---|---|---|
| `.als` Live Set, `.alc` clip, `.adv` device preset, `.adg` rack/group preset | many | gzip XML, same `<Ableton ...>` root | same schema family as sets |
| `.asd` | 6135 | custom binary (self-describing) | per-sample analysis: waveform overview, onsets, tempo, default clip settings, warp markers |
| `.agr` | 219 | custom binary, magic `ab 1e 56 78` | groove |
| `.ascl` | 152 | plain text | Scala-style tuning spec (the file documents its own format) |

## Device presets (`.adv`)
- Same root element as a set. The single child is the device, tag = device class (`AutoFilter2`, ...).
- A device has common header fields (`On`, `IsExpanded`, `Pointee`, `LastPresetRef`, `UserName`, `Annotation`, `MpePitchBendUsesTuning`, `OverwriteProtectionNumber`)
  followed by one element per parameter, named after the parameter, e.g. `Filter_Frequency`, `Filter_Resonance`, `Filter_Type`, `Filter_Drive`.
- Parameters are the `Manual` value + automation/modulation target wrapper (same as in sets): `Filter_Frequency/Manual = 9999.998`.
- Takeaway: presets are just a device node serialized on its own, so a set's device chain and a preset file share one schema.

## Racks (`.adg`)
`GroupDevicePreset > Device > DrumGroupDevice` (177 child fields) + `BranchPresets > DrumBranchPreset[...]` (one per pad, 11 fields each)
+ `ReturnBranchPresets`. (Only the drum rack was inspected; other rack types are unverified.)

## Audio clips (`AudioClip` in a set)
- Attributes `Id`, `Time`. Fields seen: `CurrentStart/End` (beats), `Name`, `Color`, `IsWarped`, `WarpMode` (int enum),
  `PitchCoarse`, `PitchFine`, `SampleVolume`, `Fade`, `Loop`, `WarpMarkers`, `SampleRef`.
- **Warp markers:** `WarpMarkers > WarpMarker Id SecTime BeatTime` — seconds in the source file mapped to beats on the timeline.
  The clip inspected ("Vocal Main") has 2: `(0 s, 0 beats)` and a second one a tiny distance later (`0.01123 s ↔ 0.03125 beats`, i.e. 1/32 beat).
  Presumably tempo between markers is interpolated and the last slope continues past the end, which makes warping a piecewise-linear sec→beat map (not verified against the engine).
- `SampleRef`: `FileRef` (`RelativePathType`, `RelativePath`, absolute `Path`, `Type`, `LivePackName`, `LivePackId`, `OriginalFileSize`, `OriginalCrc`),
  `LastModDate`, `DefaultDuration` (frames), `DefaultSampleRate`, `SamplesToAutoWarp`.
  Files are referenced by path + size + CRC, not embedded.

## Automation envelopes
- Set contains `AutomationEnvelope` elements (83 in the Patience demo). Each has `EnvelopeTarget > PointeeId` (the `AutomationTarget Id` of the parameter)
  and `Automation > Events`.
- Events: `FloatEvent Id Time Value`, plus `BoolEvent`, `EnumEvent` for non-float params. Time is in beats.
- In the envelope inspected, the first event is a sentinel at `Time="-63072000"` (a very large negative time) that appears to hold the parameter's value before any breakpoint; I only checked one envelope.
- Some events carry `CurveControl1X/1Y/2X/2Y` (values 0..1, apparently a cubic Bézier shape for the segment); the ones without them look linear. Which end of the segment owns the control points is not confirmed.
- Note-expression data is `PerNoteEvent` (66k in the demo) and shares the same event idea.

## Sample analysis files (`.asd`)
- Binary, named `<sample file name>.asd` next to the sample (e.g. `Clap Finger.wav.asd`).
- Starts with a table of increasing u32 offsets (overview waveform chunk boundaries). Followed by a **self-describing serialized object stream**:
  length-prefixed UTF-16 field names, short ASCII type names (`RemoteableDouble`, `RemoteableInt`, `RemoteableBool`, `RemoteableEnum`, `RemoteableArray`, `RemoteableList`, `SampleData`, `SampleOverView`, `OnsetArray`, `AufTaktData`...), then the values.
- Field names recovered (in order): `OverViewLevels` (`SamplesPerBinLog2`, `ChannelCount`, `InterleavedBinData2`), `UserOnsets`/`OnSets`/`OnsetArray`,
  `AufTaktData` (`Numerator`, `Denominator`, `Positions`, `TransitionEnergies`, `UnbiasedTempoEstimate`, `PreprocessedDataChunk`),
  clip defaults (`LoopStart`, `LoopEnd`, `HiddenLoopStart/End`, `OutMarker`, `SampleOffset`, `Sync`, `HiQ`, `Fade`, `IsWarped`, `SampleVolume`, `PitchCoarse/Fine`, `WarpMode`, `TransientResolution`, `GranularityTones`, `GranularityTexture`, `FluctuationTexture`, `ComplexProFormants`, `ComplexProEnvelope`, `TimeSignature`, `LaunchMode`, `LoopOn`, `LaunchQuantisation`),
  `WarpMarkers` (`MarkersGenerated`, `SecTime`, `BeatTime`), `ExtraLength`, `OriginalFileSize`.
- Verified decode: in the 83 bpm loop "Oracle Electric Flankers Kit", the marker pair stored is `(SecTime 5.783152, BeatTime 8.0)` and `(5.805742, 8.03125)`.
  8 beats / 5.783152 s = 82.99 bpm, matching the 83 bpm in the name. Values are little-endian float64.
- Not fully decoded: exact binary framing between fields, the overview bin data layout, and the onset/tempo data structures.
- Takeaway for diy_daw: analysis (overview waveform, onsets, tempo guess) is cached in a sidecar so the DAW never re-analyzes a file; default warp settings come from the sidecar when a sample is first dropped in.

## Still not examined
`.agr` groove binary layout, `PerNoteEvent` encoding details, the native engine (`Ableton Live Engine.dll`), and the device DLLs in `Resources\Devices\Pc`.

## Audio clip loop and warp semantics (worked out from the Patience demo set)

Clip fields: `CurrentStart`/`CurrentEnd` (timeline extent for arrangement clips; `0..length` for session clips) and `Loop{LoopStart, LoopEnd, StartRelative, LoopOn}`.

- **Units.** For warped clips (`IsWarped=true`) the loop fields are in *sample beats*, which the warp markers map to seconds in the file.
  For unwarped clips they are *seconds* of source audio (e.g. `LoopEnd=44.55` for a 44.55 s file) and the clip plays at native speed;
  its timeline length in beats was simply derived from the tempo it was saved at.
- **Warp map.** Piecewise linear through the markers `(SecTime, BeatTime)`; beyond the first/last marker the slope of the end segment continues.
  Beat 0 is *not* necessarily the file start: one clip has marker `(31.6168 s, beat 0)`. Seconds per beat = `60/tempo` (0.35928 = 167 bpm there).
- **Negative positions** are silence before the file starts (`LoopStart=-8` means 8 beats of silence, then the audio).
- **Where playback starts:** `LoopStart + StartRelative`.
- **Loop off:** plays `CurrentEnd - CurrentStart` beats from there (`CurrentEnd-CurrentStart == LoopEnd-start` held in every clip checked).
- **Loop on:** first pass runs from the start marker to `LoopEnd` (length `LoopEnd - (LoopStart+StartRelative)`, which matched `CurrentEnd-CurrentStart`
  in both session clips with `LoopOn=true`), then it repeats `LoopStart..LoopEnd`.
- `OutMarker` was ignored; its meaning is unknown.

Verified by rendering the arrangement clip "Vocal Main" at beat 323 (loop start 3, marker `(31.6168 s, beat 0)`) through the engine and comparing it
with the raw WAV: correlation > 0.999 at the position these rules predict. The loop-on rules are consistent with the data but untested by ear.

Not verified: warp mode ids other than 4, `PitchCoarse`/`PitchFine`/`SampleVolume` units, whether arrangement clips with loop on follow the same rule.
