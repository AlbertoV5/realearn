//! This is the API for building clip engine presets.
//!
//! It is designed using the following conventions:
//!
//! - Fields are optional only if they have a totally natural default or are an optional override
//!   of an otherwise inherited value.
//! - Fat enum variants are used to distinguish between multiple alternatives, but not as a general
//!   rule. For UI purposes, it's sometimes desirable to save data even it's not actually in use.
//!   In the processing layer this would be different.
//! - For the same reasons, boolean data types are allowed and not in general substituted by On/Off
//!   enums.
//! - Only a subset of the possible Rust data structuring possibilities are used. The ones that
//!   work well with ReaLearn Script (Lua).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Matrix {
    /// All columns from left to right.
    pub columns: Vec<Column>,
    /// All rows from top to bottom.
    pub rows: Vec<Row>,
    pub clip_play_settings: MatrixClipPlaySettings,
    pub clip_record_settings: MatrixClipRecordSettings,
    pub common_tempo_range: TempoRange,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TempoRange {
    pub min: Bpm,
    pub max: Bpm,
}

/// Matrix-global settings related to playing clips.
#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MatrixClipPlaySettings {
    pub start_timing: ClipPlayStartTiming,
    pub stop_timing: ClipPlayStopTiming,
    pub audio_settings: MatrixClipPlayAudioSettings,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MatrixClipPlayAudioSettings {
    pub time_stretch_mode: AudioTimeStretchMode,
}

/// Matrix-global settings related to recording clips.
#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MatrixClipRecordSettings {
    pub start_timing: ClipRecordStartTiming,
    pub stop_timing: ClipRecordStopTiming,
    pub duration: RecordLength,
    pub play_start_timing: ClipSettingOverrideAfterRecording<ClipPlayStartTiming>,
    pub play_stop_timing: ClipSettingOverrideAfterRecording<ClipPlayStopTiming>,
    pub time_base: ClipRecordTimeBase,
    /// If `true`, starts playing the clip right after recording.
    pub play_after: bool,
    /// If `true`, sets the global tempo to the tempo of this clip right after recording.
    pub lead_tempo: bool,
    pub midi_settings: MatrixClipRecordMidiSettings,
    pub audio_settings: MatrixClipRecordAudioSettings,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MatrixClipRecordMidiSettings {
    pub record_mode: MidiClipRecordMode,
    /// If `true`, attempts to detect the actual start of the recorded MIDI material and derives
    /// the downbeat position from that.
    pub detect_downbeat: bool,
    /// Makes the global record button work for MIDI by allowing global input detection.
    pub detect_input: bool,
    /// Applies quantization while recording using the current quantization settings.
    pub auto_quantize: bool,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MatrixClipRecordAudioSettings {
    /// If `true`, attempts to detect the actual start of the recorded audio material and derives
    /// the downbeat position from that.
    pub detect_downbeat: bool,
    /// Makes the global record button work for audio by allowing global input detection.
    pub detect_input: bool,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind")]
pub enum RecordLength {
    /// Records open-ended until the user decides to stop.
    OpenEnd,
    /// Records exactly as much material as defined by the given quantization.
    Quantized(EvenQuantization),
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum ClipRecordTimeBase {
    /// Sets the time base of the recorded clip to [`ClipTimeBase::Time`].
    Time,
    /// Sets the time base of the recorded clip to [`ClipTimeBase::Beat`].
    Beat,
    /// Derives the time base of the resulting clip from the clip start/stop timing.
    DeriveFromRecordTiming,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind")]
pub enum ClipRecordStartTiming {
    /// Uses the global clip play start timing.
    LikeClipPlayStartTiming,
    /// Starts recording immediately.
    Immediately,
    /// Starts recording according to the given quantization.
    Quantized(EvenQuantization),
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind")]
pub enum ClipRecordStopTiming {
    /// Uses the record start timing.
    LikeClipRecordStartTiming,
    /// Stops recording immediately.
    Immediately,
    /// Stops recording according to the given quantization.
    Quantized(EvenQuantization),
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum MidiClipRecordMode {
    /// Creates an empty clip and records MIDI material in it.
    Normal,
    /// Records more material onto an existing clip, leaving existing material in place.
    Overdub,
    /// Records more material onto an existing clip, overwriting existing material.
    Replace,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind")]
pub enum ClipPlayStartTiming {
    /// Starts playing immediately.
    Immediately,
    /// Starts playing according to the given quantization.
    Quantized(EvenQuantization),
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind")]
pub enum ClipPlayStopTiming {
    /// Uses the play start timing.
    LikeClipStartTiming,
    /// Stops playing immediately.
    Immediately,
    /// Stops playing according to the given quantization.
    Quantized(EvenQuantization),
    /// Keeps playing until the end of the clip.
    UntilEndOfClip,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind")]
pub enum ClipSettingOverrideAfterRecording<T> {
    /// Doesn't apply any override.
    Inherit,
    /// Overrides the setting with the given value.
    Override(Override<T>),
    /// Overrides the setting with a value derived from the record timing.
    OverrideFromRecordTiming,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Override<T> {
    pub value: T,
}

/// An even quantization.
///
/// Even in the sense of that's it's not swing or dotted.
#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvenQuantization {
    /// The number of bars.
    ///
    /// Must not be zero.
    pub numerator: u32,
    /// Defines the fraction of a bar.
    ///
    /// Must not be zero.
    ///
    /// If the numerator is > 1, this must be 1.
    pub denominator: u32,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Column {
    pub clip_play_settings: ColumnClipPlaySettings,
    pub clip_record_settings: ColumnClipRecordSettings,
    /// Slots in this column.
    ///
    /// Only filled slots need to be mentioned here.
    pub slots: Vec<Slot>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ColumnClipPlaySettings {
    /// REAPER track used for playing back clips in this column.
    ///
    /// Usually, each column should have a play track. But events might occur that leave a column
    /// in a "track-less" state, e.g. the deletion of a track. This column will be unusable until
    /// the user sets a play track again. We still want to be able to save the matrix in such a
    /// state, otherwise it could be really annoying. So we allow `None`.
    pub track: Option<TrackId>,
    /// Start timing override.
    ///
    /// `None` means it uses the matrix-global start timing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_timing: Option<ClipPlayStartTiming>,
    /// Stop timing override.
    ///
    /// `None` means it uses the matrix-global stop timing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_timing: Option<ClipPlayStopTiming>,
    pub audio_settings: ColumnClipPlayAudioSettings,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ColumnClipRecordSettings {
    /// By default, Playtime records from the play track but this settings allows to override that.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track: Option<TrackId>,
    pub origin: TrackRecordOrigin,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ColumnClipPlayAudioSettings {
    /// Overrides the matrix-global audio time stretch mode for clips in this column.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_stretch_mode: Option<AudioTimeStretchMode>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub scene: Scene,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    pub name: Option<String>,
    /// An optional tempo associated with this row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tempo: Option<Bpm>,
    /// An optional time signature associated with this row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_signature: Option<TimeSignature>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind")]
pub enum AudioTimeStretchMode {
    /// Doesn't just stretch/squeeze the material but also changes the pitch.
    ///
    /// Comparatively fast.
    VariSpeed(VariSpeedMode),
    /// Applies a real time-stretch algorithm to the material which keeps the pitch.
    ///
    /// Comparatively slow.
    KeepingPitch(TimeStretchMode),
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VariSpeedMode {
    pub mode: VirtualVariSpeedMode,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind")]
pub enum VirtualVariSpeedMode {
    /// Uses the resample mode set as default for this REAPER project.
    ProjectDefault,
    /// Uses a specific resample mode.
    ReaperMode(ReaperResampleMode),
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReaperResampleMode {
    pub mode: u32,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimeStretchMode {
    pub mode: VirtualTimeStretchMode,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind")]
pub enum VirtualTimeStretchMode {
    /// Uses the pitch shift mode set as default for this REAPER project.
    ProjectDefault,
    /// Uses a specific REAPER pitch shift mode.
    ReaperMode(ReaperPitchShiftMode),
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReaperPitchShiftMode {
    pub mode: u32,
    pub sub_mode: u32,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum TrackRecordOrigin {
    /// Records using the hardware input set for the track (MIDI or stereo).
    TrackInput,
    /// Captures audio from the output of the track.
    TrackAudioOutput,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Slot {
    /// Slot index within the column (= row), starting at zero.
    pub row: usize,
    /// Clip which currently lives in this slot.
    pub clip: Option<Clip>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    /// Source of the audio/MIDI material of this clip.
    pub source: Source,
    /// Time base of the material provided by that source.
    pub time_base: ClipTimeBase,
    /// Start timing override.
    ///
    /// `None` means it uses the column start timing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_timing: Option<ClipPlayStartTiming>,
    /// Stop timing override.
    ///
    /// `None` means it uses the column stop timing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_timing: Option<ClipPlayStopTiming>,
    /// Whether the clip should be played repeatedly or as a single shot.
    pub looped: bool,
    /// Color of the clip.
    pub color: ClipColor,
    /// Defines which portion of the original source should be played.
    pub section: Section,
    pub audio_settings: ClipAudioSettings,
    pub midi_settings: ClipMidiSettings,
    // /// Defines the total amount of time this clip should consume and where within that range the
    // /// portion of the original source is located.
    // ///
    // /// This allows one to insert silence at the beginning and the end
    // /// as well as changing the source section without affecting beat
    // /// alignment.
    // ///
    // /// `None` means the canvas will have the same size as the source
    // /// section.
    // canvas: Option<Canvas>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipAudioSettings {
    /// Whether to cache audio in memory.
    pub cache_behavior: AudioCacheBehavior,
    /// Defines whether to apply automatic fades in order to fix potentially non-optimized source
    /// material.
    ///
    /// ## `false`
    ///
    /// Doesn't apply automatic fades for fixing non-optimized source material.
    ///
    /// This only prevents source-fix fades. Fades that are not about fixing the source will still
    /// be applied if necessary in order to ensure a smooth playback, such as:
    ///
    /// - Section fades (start fade-in, end fade-out)
    /// - Interaction fades (resume-after-pause fade-in, immediate stop fade-out)
    ///
    /// Fades don't overlap. Here's the order of priority (for fade-in and fade-out separately):
    ///
    /// - Interaction fades
    /// - Section fades
    /// - Source-fix fades
    ///
    /// ## `true`
    ///
    /// Applies automatic fades to fix non-optimized source material, if necessary.
    pub apply_source_fades: bool,
    /// Defines how to adjust audio material.
    ///
    /// This is usually used with the beat time base to match the tempo of the clip to the global
    /// tempo.
    ///
    /// `None` means it uses the column time stretch mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_stretch_mode: Option<AudioTimeStretchMode>,
}

// struct Canvas {
//     /// Should be long enough to let the source section fit in.
//     length: DurationInSeconds,
//     /// Position of the source section.
//     section_pos: PositionInSeconds,
// }

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipMidiSettings {
    /// For fixing the source itself.
    pub source_reset_settings: MidiResetMessageRange,
    /// For fine-tuning the section.
    pub section_reset_settings: MidiResetMessageRange,
    /// For fine-tuning the complete loop.
    pub loop_reset_settings: MidiResetMessageRange,
    /// For fine-tuning instant start/stop of a MIDI clip when in the middle of a source or section.
    pub interaction_reset_settings: MidiResetMessageRange,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MidiResetMessageRange {
    /// Which MIDI reset messages to apply at the beginning.
    pub start: MidiResetMessages,
    /// Which MIDI reset messages to apply at the beginning.
    pub end: MidiResetMessages,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MidiResetMessages {
    pub all_notes_off: bool,
    pub all_sounds_off: bool,
    pub sustain_off: bool,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Section {
    /// Position in the source from which to start.
    ///
    /// If this is greater than zero, a fade-in will be used to avoid clicks.
    pub start_pos: Seconds,
    /// Length of the material to be played, starting from `start_pos`.
    ///
    /// - `None` means until original source end.
    /// - May exceed the end of the source.
    /// - If this makes the section end be located before the original source end, a fade-out will
    ///   be used to avoid clicks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length: Option<Seconds>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum AudioCacheBehavior {
    /// Loads directly from the disk.
    ///
    /// Might still pre-buffer some blocks but definitely won't put the complete audio data into
    /// memory.
    DirectFromDisk,
    /// Loads the complete audio data into memory.
    CacheInMemory,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind")]
pub enum ClipColor {
    /// Inherits the color of the column's play track.
    PlayTrackColor,
    /// Assigns a very specific custom color.
    CustomColor(CustomClipColor),
    /// Uses a certain color from a palette.
    PaletteColor(PaletteClipColor),
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CustomClipColor {
    pub value: u32,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PaletteClipColor {
    pub index: u32,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind")]
pub enum Source {
    /// Takes content from a media file on the file system (audio or MIDI).
    File(FileSource),
    /// Embedded MIDI data.
    MidiChunk(MidiChunkSource),
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileSource {
    /// Path to the media file.
    ///
    /// If it's a relative path, it will be interpreted as relative to the REAPER project directory.
    pub path: PathBuf,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MidiChunkSource {
    /// MIDI data in the same format that REAPER uses for in-project MIDI.
    pub chunk: String,
}

/// Decides if the clip will be adjusted to the current tempo.
#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind")]
pub enum ClipTimeBase {
    /// Material which doesn't need to be adjusted to the current tempo.
    Time,
    /// Material which needs to be adjusted to the current tempo.
    Beat(BeatTimeBase),
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BeatTimeBase {
    /// The clip's native tempo.
    ///
    /// Must be set for audio. Is ignored for MIDI.
    ///
    /// This information is used by the clip engine to determine how much to speed up or
    /// slow down the material depending on the current project tempo.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_bpm: Option<Bpm>,
    /// The time signature of this clip.
    ///
    /// If provided, This information is used for certain aspects of the user interface.
    pub time_signature: TimeSignature,
    /// Defines which position (in beats) is the downbeat.
    pub downbeat: f64,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimeSignature {
    pub numerator: u32,
    pub denominator: u32,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TrackId(pub String);

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Bpm(pub f64);

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Seconds(pub f64);

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RgbColor(pub u8, pub u8, pub u8);