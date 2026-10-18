# Ableton Live 12 Set format (`.als`) — observations

Observed from the factory sets that ship with the Live 12 Trial install
(`Resources\Core Library\Templates`, `Lessons\Demo Songs`, `Resources\Builtin\Templates\DefaultLiveSet.als`).
This is behavior documentation for designing our own format, not a copy of Ableton code.

## Container
- A `.als` file is a **gzip-compressed XML** document (magic `1f 8b`). `DefaultLiveSet.als`: 13 KB gzipped, ~180 KB XML.
- Root element: `<Ableton MajorVersion="5" MinorVersion="12.0_12402" SchemaChangeCount=".." Creator="Ableton Live 12.4d1" Revision="<git sha>">`.
- Single child `<LiveSet>`.

## `LiveSet` top level (order as written)
`NextPointeeId`, `OverwriteProtectionNumber`, `LomId`, `Tracks`, `MainTrack`, `PreHearTrack`, `SendsPre`,
`Scenes`, `Transport`, `GlobalQuantisation`, `Grid`, `ScaleInformation`, `Locators`, `GroovePool`,
`AutomationMode`, `CrossfadeCurve`, `TuningSystems`, `ViewStates`, ... plus many `ViewState*` UI fields.

- Return tracks live in `Tracks` as `ReturnTrack` (visible tracks vs. returns are separated by list wrappers).
- Tempo is an automatable param on the main track mixer: `MainTrack/DeviceChain/Mixer/Tempo/Manual`.
- Every parameter is wrapped as `<Param><LomId/><Manual Value=".."/><AutomationTarget Id=".."/><ModulationTarget Id=".."/></Param>` —
  `Manual` is the static value, the targets are ids that automation envelopes and modulation point at (`NextPointeeId` is the id counter).

## Tracks
Track elements: `MidiTrack`, `AudioTrack`, `GroupTrack`, `ReturnTrack` (each with `Id`).
Common children: `Name{EffectiveName,UserName,Annotation,MemorizedFirstClipName}`, `Color`, `TrackGroupId` (-1 = none),
`TrackDelay`, `Freeze`, `DeviceChain`.

`DeviceChain` holds, in order: `AutomationLanes`, `AudioInputRouting`/`MidiInputRouting`/`AudioOutputRouting`/`MidiOutputRouting`
(5 children each), `Mixer` (~30 children: `On`, `Volume`, `Pan`, `PanMode`, `Sends`, `Speaker`, `SoloSink`, `CrossFadeState` ...), `MainSequencer`, `FreezeSequencer`,
and a nested `DeviceChain/Devices` list.

## Devices
Stored as typed child elements of `Devices`, tag = device class: `Reverb`, `Delay`, `Compressor2`, `AutoFilter`, `StereoGain`,
`Drift`, `OriginalSimpler`, and wrappers `DrumGroupDevice`, `InstrumentGroupDevice`, `AudioEffectGroupDevice` (racks).

## Session vs arrangement
- Session clips: `MainSequencer/ClipSlotList/ClipSlot/ClipSlot/Value/<clip>` (one `ClipSlot` per scene row; empty `Value` = empty slot).
- Arrangement clips: `MainSequencer/ClipTimeable/ArrangerAutomation/Events/<clip>`.
- `Scenes` has one entry per scene (Quick Start Beat: 8, Patience demo: 13).
- Time unit is **beats** (quarter notes), as floats. A 16 beat clip: `CurrentStart=0`, `CurrentEnd=16`.

## MidiClip
Attributes `Id`, `Time`. Notable fields: `CurrentStart/End`, `Loop{LoopStart,LoopEnd,StartRelative,LoopOn,OutMarker,HiddenLoopStart,HiddenLoopEnd}`,
`Name`, `Color` (palette index), `LaunchMode`, `LaunchQuantisation`, `FollowAction`, `Legato`, `VelocityAmount`, `IsWarped`,
`Envelopes` (clip automation), `Grid`, `ScaleInformation`/`IsInKey`, and `Notes`.

### Notes encoding
```
Notes
  KeyTracks            one KeyTrack per pitch that has notes
    KeyTrack Id=..
      Notes
        MidiNoteEvent Time="0" Duration="0.5" Velocity="100" OffVelocity="64" NoteId="13"
      MidiKey Value="36"      (MIDI note number)
  PerNoteEventStore    per-note expression (MPE)
  NoteProbabilityGroups, ProbabilityGroupIdGenerator, NoteIdGenerator
```
Notes are grouped by pitch, not in a flat time-ordered list. `Time` is in beats relative to the clip start.

## Sample sets surveyed
| Set | Tempo | Tracks | Scenes |
|---|---|---|---|
| Quick Start Beat | 147 | 4 MIDI + 2 return | 8 |
| Patience (Suite Demo) | 167 | 11 MIDI + 4 audio + 2 return | 13 |

## Implications for diy_daw
- Plain XML in gzip is a good model: easy to diff, but verbose. We could use JSON and keep the same concepts.
- Worth copying conceptually: unified parameter wrapper (value + automation target id), beat-based time, session grid + arrangement timeline sharing one clip type, note storage keyed by pitch.
- Worth skipping: huge amount of UI view state mixed into the document, and per-field LomIds.

## Not yet examined
`.adv`/`.adg` device and rack presets, `.alc` clips, `.asd` sample analysis files (warp markers), audio clip + warp marker schema,
automation envelope encoding, and the native engine (`Ableton Live Engine.dll`).
Remote Scripts under `Resources\MIDI Remote Scripts` ship as `.pyc` only.
