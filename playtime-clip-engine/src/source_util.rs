use crate::file_util::get_path_for_new_media_file;
use crate::rt::supplier::MIDI_BASE_BPM;
use crate::{rt, ClipEngineResult};
use playtime_api as api;
use playtime_api::{FileSource, MidiChunkSource};
use reaper_high::{BorrowedSource, Item, OwnedSource, Project, ReaperSource};
use reaper_medium::{MidiImportBehavior, OwnedPcmSource};
use std::error::Error;
use std::path::{Path, PathBuf};

/// Creates slot content based on the audio/MIDI file used by the given item.
///
/// If the item uses pooled MIDI instead of a file, this method exports the MIDI data to a new
/// file in the recording directory and uses that one.
#[allow(dead_code)]
pub fn create_api_source_from_item(
    item: Item,
    force_export_to_file: bool,
) -> Result<api::Source, Box<dyn Error>> {
    let active_take = item.active_take().ok_or("item has no active take")?;
    let root_pcm_source = active_take
        .source()
        .ok_or("take has no source")?
        .root_source();
    let root_pcm_source = ReaperSource::new(root_pcm_source);
    let mode = if force_export_to_file {
        CreateApiSourceMode::ForceExportToFile {
            file_base_name: active_take.name(),
        }
    } else {
        CreateApiSourceMode::AllowEmbeddedData
    };
    create_api_source_from_pcm_source(&root_pcm_source, mode, item.project())
}

#[allow(dead_code)]
pub enum CreateApiSourceMode {
    AllowEmbeddedData,
    ForceExportToFile { file_base_name: String },
}

/// Project is used for making a file path relative and/or for determining the directory of a file
/// to be exported.
pub fn create_api_source_from_pcm_source(
    pcm_source: &BorrowedSource,
    mode: CreateApiSourceMode,
    project: Option<Project>,
) -> Result<api::Source, Box<dyn Error>> {
    let pcm_source_type = pcm_source.r#type();
    if let Some(source_file) = pcm_source.file_name() {
        Ok(create_file_api_source(project, &source_file))
    } else if matches!(pcm_source_type.as_str(), "MIDI" | "MIDIPOOL") {
        use CreateApiSourceMode::*;
        let api_source = match mode {
            AllowEmbeddedData => create_midi_chunk_source(pcm_source.state_chunk()),
            ForceExportToFile { file_base_name } => {
                let file_name = get_path_for_new_media_file(&file_base_name, "mid", project);
                pcm_source
                    .export_to_file(&file_name)
                    .map_err(|_| "couldn't export MIDI source to file")?;
                create_file_api_source(project, &file_name)
            }
        };
        Ok(api_source)
    } else {
        Err(format!("item source incompatible (type {})", pcm_source_type).into())
    }
}

/// Takes care of making the path project-relative (if a project is given).
pub fn create_file_api_source(project: Option<Project>, file: &Path) -> api::Source {
    api::Source::File(api::FileSource {
        path: make_relative(project, file),
    })
}

fn create_midi_chunk_source(chunk: String) -> api::Source {
    api::Source::MidiChunk(api::MidiChunkSource { chunk })
}

/// Creates a REAPER PCM source from the given API source.
///
/// If no project is given, the path will not be relative.
pub fn create_pcm_source_from_api_source(
    api_source: &api::Source,
    project_for_relative_path: Option<Project>,
) -> ClipEngineResult<OwnedPcmSource> {
    use api::Source::*;
    let pcm_source = match api_source {
        File(s) => create_pcm_source_from_file_based_api_source(project_for_relative_path, s)?,
        MidiChunk(s) => create_pcm_source_from_midi_chunk_based_api_source(s.clone())?,
    };
    Ok(pcm_source.into_raw())
}

pub fn create_pcm_source_from_midi_chunk_based_api_source(
    MidiChunkSource { mut chunk }: MidiChunkSource,
) -> ClipEngineResult<OwnedSource> {
    let mut source = OwnedSource::from_type("MIDI").unwrap();
    chunk += ">\n";
    source.set_state_chunk("<SOURCE MIDI\n", chunk)?;
    // Make sure we don't have any association to some item on the timeline (or in
    // another slot) because that could lead to unpleasant surprises.
    source.remove_from_midi_pool().map_err(|e| e.message())?;
    source
        .set_preview_tempo(MIDI_BASE_BPM)
        .map_err(|e| e.message())?;
    post_process_midi_source(&source);
    Ok(source)
}

pub fn create_pcm_source_from_file_based_api_source(
    project_for_relative_path: Option<Project>,
    FileSource { path }: &FileSource,
) -> ClipEngineResult<OwnedSource> {
    let absolute_file = if path.is_relative() {
        project_for_relative_path
            .ok_or("slot source given as relative file but without project")?
            .make_path_absolute(path)
            .ok_or("couldn't make clip source path absolute")?
    } else {
        path.clone()
    };
    // We don't import as in-project events, otherwise we would end up with a MIDI chunk
    // source on save, which would be unexpected. It's worth to point out that MIDI overdub
    // is not possible with file-based MIDI sources. So as soon as the user does MIDI
    // overdub, we need to go "MIDI chunk".
    let source = OwnedSource::from_file(&absolute_file, MidiImportBehavior::ForceNoMidiImport)?;
    if rt::source_util::pcm_source_is_midi(source.as_ref().as_raw()) {
        post_process_midi_source(&source);
    }
    Ok(source)
}

/// Returns an empty MIDI source prepared for recording.
pub fn create_empty_midi_source() -> OwnedSource {
    let mut source = OwnedSource::from_type("MIDI").unwrap();
    // The following seems to be the absolute minimum to create the shortest possible MIDI clip
    // (which still is longer than zero).
    let chunk = "\
        HASDATA 1 960 QN\n\
        E 1 b0 7b 00\n\
    >\n\
    ";
    source
        .set_state_chunk("<SOURCE MIDI\n", String::from(chunk))
        .unwrap();
    post_process_midi_source(&source);
    source
}

fn post_process_midi_source(source: &BorrowedSource) {
    // Setting the source preview tempo is absolutely essential. It decouples the playing
    // of the source from REAPER's project position and tempo. We set it to a constant tempo
    // because we control the tempo on-the-fly by modifying the frame rate when requesting
    // material.
    // There are alternatives to setting the preview tempo, in particular doing the
    // following when playing the material:
    //
    //      transfer.set_force_bpm(MIDI_BASE_BPM);
    //      transfer.set_absolute_time_s(PositionInSeconds::ZERO);
    //
    // However, now we set the constant preview tempo at source creation time, which makes
    // the source completely project tempo/pos-independent, also when doing recording via
    // midi_realtime_write_struct_t. So that's not necessary anymore.
    source.set_preview_tempo(MIDI_BASE_BPM).unwrap();
}

fn make_relative(project: Option<Project>, file: &Path) -> PathBuf {
    project
        .and_then(|p| p.make_path_relative_if_in_project_directory(file))
        .unwrap_or_else(|| file.to_owned())
}