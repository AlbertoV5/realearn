use crate::application::{MappingModel, Preset, PresetManager, SharedMapping};
use crate::core::default_util::is_default;
use crate::domain::MappingCompartment;
use crate::infrastructure::data::MappingModelData;

use reaper_high::Reaper;
use rx_util::UnitEvent;
use rxrust::prelude::*;
use serde::de::DeserializeOwned;
use serde::export::PhantomData;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Debug;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[derive(Debug)]
pub struct FileBasedPresetManager<P: Preset, PD: PresetData<P = P>> {
    preset_dir_path: PathBuf,
    presets: Vec<P>,
    changed_subject: LocalSubject<'static, (), ()>,
    p: PhantomData<PD>,
}

pub trait ExtendedPresetManager {
    fn find_index_by_id(&self, id: &str) -> Option<usize>;
    fn find_id_by_index(&self, index: usize) -> Option<String>;
    fn remove_preset(&mut self, id: &str) -> Result<(), &'static str>;
}

impl<P: Preset, PD: PresetData<P = P>> FileBasedPresetManager<P, PD> {
    pub fn new(preset_dir_path: PathBuf) -> FileBasedPresetManager<P, PD> {
        let mut manager = FileBasedPresetManager {
            preset_dir_path,
            presets: vec![],
            changed_subject: Default::default(),
            p: PhantomData,
        };
        let _ = manager.load_presets();
        manager
    }

    pub fn load_presets(&mut self) -> Result<(), String> {
        let preset_file_paths = fs::read_dir(&self.preset_dir_path)
            .map_err(|_| "couldn't read preset directory".to_string())?
            .filter_map(|result| {
                let dir_entry = result.ok()?;
                let file_type = dir_entry.file_type().ok()?;
                if !file_type.is_file() {
                    return None;
                }
                let path = dir_entry.path();
                if !path.extension().contains(&"json") {
                    return None;
                };
                Some(path)
            });
        self.presets = preset_file_paths
            .filter_map(|p| Self::load_preset(p).ok())
            .collect();
        Ok(())
    }

    pub fn presets(&self) -> impl Iterator<Item = &P> + ExactSizeIterator {
        self.presets.iter()
    }

    pub fn find_by_index(&self, index: usize) -> Option<&P> {
        self.presets.get(index)
    }

    pub fn add_preset(&mut self, preset: P) -> Result<(), &'static str> {
        let path = self.get_preset_file_path(preset.id());
        fs::create_dir_all(&self.preset_dir_path)
            .map_err(|_| "couldn't create preset directory")?;
        let mut data = PD::from_model(&preset);
        // We don't want to have the ID in the file - because the file name itself is the ID
        data.clear_id();
        let json = serde_json::to_string_pretty(&data).map_err(|_| "couldn't serialize preset")?;
        fs::write(path, json).map_err(|_| "couldn't write preset file")?;
        self.notify_changed();
        Ok(())
    }

    pub fn update_preset(&mut self, preset: P) -> Result<(), &'static str> {
        self.add_preset(preset)
    }

    pub fn changed(&self) -> impl UnitEvent {
        self.changed_subject.clone()
    }

    pub fn log_debug_info(&self) {
        let msg = format!(
            "\n\
            # Preset manager\n\
            \n\
            - Preset count: {}\n\
            ",
            self.presets.len(),
        );
        Reaper::get().show_console_msg(msg);
    }

    fn notify_changed(&mut self) {
        let _ = self.load_presets();
        self.changed_subject.next(());
    }

    fn get_preset_file_path(&self, id: &str) -> PathBuf {
        self.preset_dir_path.join(format!("{}.json", id))
    }

    fn load_preset(path: impl AsRef<Path>) -> Result<P, String> {
        let id = path
            .as_ref()
            .file_stem()
            .ok_or_else(|| "preset file must have stem because it makes up the ID".to_string())?
            .to_string_lossy()
            .to_string();
        let json =
            fs::read_to_string(&path).map_err(|_| "couldn't read preset file".to_string())?;
        let data: PD = serde_json::from_str(&json).map_err(|e| {
            format!(
                "Preset file {:?} isn't valid. Details:\n\n{}",
                path.as_ref(),
                e
            )
        })?;
        Ok(data.to_model(id))
    }
}

impl<P: Preset, PD: PresetData<P = P>> ExtendedPresetManager for FileBasedPresetManager<P, PD> {
    fn find_index_by_id(&self, id: &str) -> Option<usize> {
        self.presets.iter().position(|p| p.id() == id)
    }

    fn find_id_by_index(&self, index: usize) -> Option<String> {
        let preset = self.find_by_index(index)?;
        Some(preset.id().to_string())
    }

    fn remove_preset(&mut self, id: &str) -> Result<(), &'static str> {
        let path = self.get_preset_file_path(id);
        fs::remove_file(path).map_err(|_| "couldn't delete preset file")?;
        self.notify_changed();
        Ok(())
    }
}

impl<P: Preset, PD: PresetData<P = P>> PresetManager for FileBasedPresetManager<P, PD> {
    type PresetType = P;

    fn find_by_id(&self, id: &str) -> Option<P> {
        self.presets.iter().find(|c| c.id() == id).cloned()
    }

    fn mappings_are_dirty(&self, id: &str, mappings: &[SharedMapping]) -> bool {
        let preset = match self.presets.iter().find(|c| c.id() == id) {
            None => return false,
            Some(c) => c,
        };
        if mappings.len() != preset.mappings().len() {
            return true;
        }
        mappings
            .iter()
            .zip(preset.mappings().iter())
            .any(|(actual_mapping, preset_mapping)| {
                let actual_mapping_data = MappingModelData::from_model(&actual_mapping.borrow());
                let preset_mapping_data = MappingModelData::from_model(preset_mapping);
                actual_mapping_data != preset_mapping_data
            })
    }
}

pub trait PresetData: Sized + Serialize + DeserializeOwned + Debug {
    type P: Preset;

    fn from_model(preset: &Self::P) -> Self;

    fn to_model(&self, id: String) -> Self::P;

    fn clear_id(&mut self);
}
