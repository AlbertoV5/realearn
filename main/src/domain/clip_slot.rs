use crate::core::default_util::is_default;
use crate::domain::ClipChangedEvent;
use enumflags2::BitFlags;
use reaper_high::{Item, OwnedSource, Project, Reaper, Source, Track};
use reaper_low::raw;
use reaper_low::raw::preview_register_t;
use reaper_medium::{
    BufferingBehavior, DurationInSeconds, MeasureAlignment, MediaItem, MidiImportBehavior,
    OwnedPreviewRegister, PcmSource, PositionInSeconds, ReaperFunctionError, ReaperLockError,
    ReaperMutex, ReaperMutexGuard, ReaperVolumeValue,
};
use serde::{Deserialize, Serialize};
use std::mem;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::ptr::{null_mut, NonNull};
use std::sync::Arc;

type SharedRegister = Arc<ReaperMutex<OwnedPreviewRegister>>;

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct SlotDescriptor {
    #[serde(rename = "volume", default, skip_serializing_if = "is_default")]
    pub volume: ReaperVolumeValue,
    #[serde(rename = "repeat", default, skip_serializing_if = "is_default")]
    pub repeat: bool,
    #[serde(rename = "content", default, skip_serializing_if = "is_default")]
    pub content: Option<SlotContent>,
}

impl Default for SlotDescriptor {
    fn default() -> Self {
        Self {
            volume: ReaperVolumeValue::ZERO_DB,
            repeat: false,
            content: None,
        }
    }
}

impl SlotDescriptor {
    pub fn is_filled(&self) -> bool {
        self.content.is_some()
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SlotContent {
    File {
        #[serde(rename = "file")]
        file: PathBuf,
    },
}

impl SlotContent {
    pub fn create_source(&self, project: Option<Project>) -> Result<OwnedSource, &'static str> {
        match self {
            SlotContent::File { file } => {
                let absolute_file = if file.is_relative() {
                    project
                        .ok_or("slot source given as relative file but without project")?
                        .make_path_absolute(file)
                        .ok_or("couldn't make clip source path absolute")?
                } else {
                    file.clone()
                };
                OwnedSource::from_file(&absolute_file, MidiImportBehavior::UsePreference)
            }
        }
    }
}

#[derive(Debug)]
pub struct ClipSlot {
    descriptor: SlotDescriptor,
    register: SharedRegister,
    state: State,
}

impl Default for ClipSlot {
    fn default() -> Self {
        let descriptor = SlotDescriptor::default();
        let register = create_shared_register(&descriptor);
        Self {
            descriptor,
            register,
            state: State::Empty,
        }
    }
}

fn create_shared_register(descriptor: &SlotDescriptor) -> SharedRegister {
    let mut register = OwnedPreviewRegister::default();
    register.set_volume(descriptor.volume);
    Arc::new(ReaperMutex::new(register))
}

impl ClipSlot {
    pub fn descriptor(&self) -> &SlotDescriptor {
        &self.descriptor
    }

    /// Resets all slot data to the defaults (including volume, repeat etc.).
    pub fn reset(&mut self) -> Result<Vec<ClipChangedEvent>, &'static str> {
        self.load(Default::default(), None)
    }

    /// Stops playback if necessary and loads all slot settings including the contained clip from
    /// the given descriptor.
    pub fn load(
        &mut self,
        descriptor: SlotDescriptor,
        project: Option<Project>,
    ) -> Result<Vec<ClipChangedEvent>, &'static str> {
        self.clear()?;
        // Using a completely new register saves us from cleaning up.
        self.register = create_shared_register(&descriptor);
        self.descriptor = descriptor;
        // If we can't load now, don't complain. Maybe media is missing just temporarily. Don't
        // mess up persistent data.
        let _ = self.load_content_from_descriptor(project);
        let events = vec![
            self.play_state_changed_event(),
            self.volume_changed_event(),
            self.repeat_changed_event(),
        ];
        Ok(events)
    }

    fn load_content_from_descriptor(
        &mut self,
        project: Option<Project>,
    ) -> Result<(), &'static str> {
        let source = if let Some(content) = self.descriptor.content.as_ref() {
            content.create_source(project)?
        } else {
            // Nothing to load
            return Ok(());
        };
        self.fill_with_source(source)?;
        Ok(())
    }

    pub fn fill_with_source_from_item(&mut self, item: Item) -> Result<(), &'static str> {
        let source_file = item
            .active_take()
            .ok_or("item has no active take")?
            .source()
            .ok_or("take has no source")?
            .root_source()
            .file_name()
            .ok_or("root source doesn't have a file name")?;
        let project = item.project();
        let content = SlotContent::File {
            file: project
                .and_then(|p| p.make_path_relative_if_in_project_directory(&source_file))
                .unwrap_or(source_file),
        };
        self.fill(content, project)
    }

    pub fn fill(
        &mut self,
        content: SlotContent,
        project: Option<Project>,
    ) -> Result<(), &'static str> {
        let source = content.create_source(project)?;
        self.fill_with_source(source)?;
        // Here it's important to not set the descriptor (change things) unless load was successful.
        self.descriptor.content = Some(content);
        Ok(())
    }

    pub fn clip_info(&self) -> Option<ClipInfo> {
        let source = self.state.source()?;
        let guard = self.register.lock().ok()?;
        let info = ClipInfo {
            r#type: source.r#type(),
            file_name: source.file_name(),
            length: source.length(),
        };
        // TODO-medium This is probably necessary to make sure the mutex is not unlocked before the
        //  PCM source operations are done. How can we solve this in a better way API-wise? On the
        //  other hand, we are on our own anyway when it comes to PCM source thread safety ...
        std::mem::drop(guard);
        Some(info)
    }

    /// Should be called regularly to detect stops.
    pub fn poll(&mut self) -> Option<ClipChangedEvent> {
        if self.play_state() != ClipPlayState::Playing {
            return None;
        }
        let (current_pos, length) = {
            let guard = self.register.lock().ok()?;
            let source = guard.src()?;
            let length = unsafe { source.get_length() };
            (guard.cur_pos(), length)
        };
        match length {
            Some(l) if current_pos.get() > l.get() => {
                self.stop().ok()?;
                Some(ClipChangedEvent::PlayStateChanged(ClipPlayState::Stopped))
            }
            _ => Some(ClipChangedEvent::ClipPositionChanged(current_pos)),
        }
    }

    pub fn is_filled(&self) -> bool {
        self.descriptor.is_filled()
    }

    pub fn source_is_loaded(&self) -> bool {
        !matches!(self.state, State::Empty)
    }

    pub fn play_state(&self) -> ClipPlayState {
        use State::*;
        match &self.state {
            Empty => ClipPlayState::Stopped,
            Suspended(s) => {
                if s.is_paused {
                    ClipPlayState::Paused
                } else {
                    ClipPlayState::Stopped
                }
            }
            Playing(_) => ClipPlayState::Playing,
            Transitioning => unreachable!(),
        }
    }

    pub fn play_state_changed_event(&self) -> ClipChangedEvent {
        ClipChangedEvent::PlayStateChanged(self.play_state())
    }

    fn fill_with_source(&mut self, source: OwnedSource) -> Result<(), &'static str> {
        let result = self
            .start_transition()
            .fill_with_source(source, &self.register);
        self.finish_transition(result)
    }

    pub fn play(
        &mut self,
        track: Option<&Track>,
        options: SlotPlayOptions,
    ) -> Result<ClipChangedEvent, &'static str> {
        let result = self.start_transition().play(&self.register, options);
        self.finish_transition(result)?;
        Ok(self.play_state_changed_event())
    }

    /// Stops playback if necessary, destroys the contained source and resets the playback position
    /// to zero.
    pub fn clear(&mut self) -> Result<(), &'static str> {
        let result = self.start_transition().clear(&self.register);
        self.finish_transition(result)
    }

    pub fn stop(&mut self) -> Result<ClipChangedEvent, &'static str> {
        let result = self.start_transition().stop(&self.register);
        self.finish_transition(result)?;
        Ok(self.play_state_changed_event())
    }

    pub fn pause(&mut self) -> Result<ClipChangedEvent, &'static str> {
        let result = self.start_transition().pause();
        self.finish_transition(result)?;
        Ok(self.play_state_changed_event())
    }

    pub fn is_repeated(&self) -> bool {
        self.descriptor.repeat
    }

    pub fn repeat_changed_event(&self) -> ClipChangedEvent {
        ClipChangedEvent::ClipRepeatedChanged(self.descriptor.repeat)
    }

    pub fn toggle_repeat(&mut self) -> ClipChangedEvent {
        let new_value = !self.descriptor.repeat;
        self.descriptor.repeat = new_value;
        lock(&self.register).set_looped(new_value);
        self.repeat_changed_event()
    }

    pub fn volume(&self) -> ReaperVolumeValue {
        self.descriptor.volume
    }

    pub fn volume_changed_event(&self) -> ClipChangedEvent {
        ClipChangedEvent::ClipVolumeChanged(self.descriptor.volume)
    }

    pub fn set_volume(&mut self, volume: ReaperVolumeValue) -> ClipChangedEvent {
        self.descriptor.volume = volume;
        lock(&self.register).set_volume(volume);
        self.volume_changed_event()
    }

    fn start_transition(&mut self) -> State {
        std::mem::replace(&mut self.state, State::Transitioning)
    }

    fn finish_transition(&mut self, result: TransitionResult) -> Result<(), &'static str> {
        let (next_state, result) = match result {
            Ok(s) => (s, Ok(())),
            Err((s, msg)) => (s, Err(msg)),
        };
        self.state = next_state;
        result
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum ClipPlayState {
    Stopped,
    Playing,
    Paused,
}

type TransitionResult = Result<State, (State, &'static str)>;

#[derive(Debug)]
enum State {
    Empty,
    Suspended(SuspendedState),
    Playing(PlayingState),
    Transitioning,
}

impl State {
    pub fn source(&self) -> Option<&OwnedSource> {
        use State::*;
        match self {
            Suspended(s) => Some(&s.source),
            Playing(s) => Some(&s.source),
            _ => None,
        }
    }

    pub fn play(self, reg: &SharedRegister, options: SlotPlayOptions) -> TransitionResult {
        use State::*;
        match self {
            Empty => Err((Empty, "slot is empty")),
            Suspended(s) => s.play(reg, options),
            Playing(s) => s.play(reg),
            Transitioning => unreachable!(),
        }
    }

    pub fn stop(self, reg: &SharedRegister) -> TransitionResult {
        use State::*;
        match self {
            Empty => Ok(Empty),
            Suspended(s) => s.stop(reg),
            Playing(s) => s.stop(reg),
            Transitioning => unreachable!(),
        }
    }

    pub fn pause(self) -> TransitionResult {
        use State::*;
        match self {
            s @ Empty | s @ Suspended(_) => Ok(s),
            Playing(s) => s.pause(),
            Transitioning => unreachable!(),
        }
    }

    pub fn clear(self, reg: &SharedRegister) -> TransitionResult {
        use State::*;
        match self {
            Empty => Ok(Empty),
            Suspended(s) => s.clear(reg),
            Playing(s) => s.clear(reg),
            Transitioning => unreachable!(),
        }
    }

    pub fn fill_with_source(self, source: OwnedSource, reg: &SharedRegister) -> TransitionResult {
        use State::*;
        match self {
            Empty | Suspended(_) => {
                let mut g = lock(reg);
                g.set_src(Some(source.raw()));
                g.set_cur_pos(PositionInSeconds::new(0.0));
                Ok(Suspended(SuspendedState {
                    source,
                    is_paused: false,
                }))
            }
            Playing(s) => s.fill_with_source(source, reg),
            Transitioning => unreachable!(),
        }
    }
}

#[derive(Debug)]
struct SuspendedState {
    source: OwnedSource,
    is_paused: bool,
}

impl SuspendedState {
    pub fn play(self, register: &SharedRegister, options: SlotPlayOptions) -> TransitionResult {
        let result = unsafe {
            Reaper::get().medium_session().play_preview_ex(
                register.clone(),
                if options.buffered {
                    BitFlags::from_flag(BufferingBehavior::BufferSource)
                } else {
                    BitFlags::empty()
                },
                if options.next_bar {
                    MeasureAlignment::AlignWithMeasureStart
                } else {
                    MeasureAlignment::PlayImmediately
                },
            )
        };
        match result {
            Ok(handle) => {
                let next_state = PlayingState {
                    source: self.source,
                    handle,
                };
                Ok(State::Playing(next_state))
            }
            Err(_) => Err((State::Suspended(self), "couldn't play preview")),
        }
    }

    pub fn stop(self, reg: &SharedRegister) -> TransitionResult {
        let next_state = State::Suspended(self);
        let mut g = lock(reg);
        // Reset position!
        g.set_cur_pos(PositionInSeconds::new(0.0));
        Ok(next_state)
    }

    pub fn clear(self, reg: &SharedRegister) -> TransitionResult {
        let mut g = lock(reg);
        g.set_src(None);
        g.set_cur_pos(PositionInSeconds::new(0.0));
        Ok(State::Empty)
    }
}

#[derive(Debug)]
struct PlayingState {
    source: OwnedSource,
    handle: NonNull<raw::preview_register_t>,
}

impl PlayingState {
    pub fn play(self, reg: &SharedRegister) -> TransitionResult {
        let mut g = lock(reg);
        // Retrigger!
        g.set_cur_pos(PositionInSeconds::new(0.0));
        Ok(State::Playing(self))
    }

    pub fn fill_with_source(self, source: OwnedSource, reg: &SharedRegister) -> TransitionResult {
        let mut g = lock(reg);
        g.set_src(Some(source.raw()));
        Ok(State::Playing(PlayingState {
            source,
            handle: self.handle,
        }))
    }

    pub fn stop(self, reg: &SharedRegister) -> TransitionResult {
        let next_state = State::Suspended(self.suspend(false));
        let mut g = lock(reg);
        // Reset position!
        g.set_cur_pos(PositionInSeconds::new(0.0));
        Ok(next_state)
    }

    pub fn clear(self, reg: &SharedRegister) -> TransitionResult {
        self.suspend(false).clear(reg)
    }

    pub fn pause(self) -> TransitionResult {
        Ok(State::Suspended(self.suspend(true)))
    }

    fn suspend(self, pause: bool) -> SuspendedState {
        let next_state = SuspendedState {
            source: self.source,
            is_paused: pause,
        };
        // If not successful this probably means it was stopped already, so okay.
        let _ = unsafe { Reaper::get().medium_session().stop_preview(self.handle) };
        next_state
    }
}

pub struct ClipInfo {
    pub r#type: String,
    pub file_name: Option<PathBuf>,
    pub length: Option<DurationInSeconds>,
}

#[derive(Copy, Clone, PartialEq, Debug, Default)]
pub struct SlotPlayOptions {
    pub next_bar: bool,
    pub buffered: bool,
}

fn lock(reg: &SharedRegister) -> ReaperMutexGuard<OwnedPreviewRegister> {
    reg.lock().expect("couldn't acquire lock")
}