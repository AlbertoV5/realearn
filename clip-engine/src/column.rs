use crate::{
    CacheRequest, ClipChangedEvent, ClipRecordTask, ColumnFillSlotArgs, ColumnPauseClipArgs,
    ColumnPlayClipArgs, ColumnPollSlotArgs, ColumnSeekClipArgs, ColumnSetClipRepeatedArgs,
    ColumnSetClipVolumeArgs, ColumnSource, ColumnSourceTask, ColumnStopClipArgs, RecordBehavior,
    RecordKind, RecorderEquipment, RecorderRequest, SharedColumnSource, Slot,
    SlotProcessTransportChangeArgs, Timeline, TimelineMoment, TransportChange,
};
use crossbeam_channel::Sender;
use enumflags2::BitFlags;
use helgoboss_learn::UnitValue;
use reaper_high::{BorrowedSource, Project, Reaper, Track};
use reaper_low::raw::preview_register_t;
use reaper_medium::{
    create_custom_owned_pcm_source, BorrowedPcmSource, CustomPcmSource, FlexibleOwnedPcmSource,
    MeasureAlignment, OwnedPreviewRegister, PositionInSeconds, ReaperMutex, ReaperMutexGuard,
    ReaperVolumeValue,
};
use std::ptr::NonNull;
use std::sync::Arc;

pub type SharedRegister = Arc<ReaperMutex<OwnedPreviewRegister>>;

#[derive(Clone, Debug)]
pub struct Column {
    track: Option<Track>,
    column_source: SharedColumnSource,
    preview_register: PlayingPreviewRegister,
    task_sender: Sender<ColumnSourceTask>,
}

#[derive(Clone, Debug)]
struct PlayingPreviewRegister {
    preview_register: SharedRegister,
    play_handle: NonNull<preview_register_t>,
}

impl Column {
    pub fn new(track: Option<Track>) -> Self {
        let (task_sender, task_receiver) = crossbeam_channel::bounded(500);
        let source = ColumnSource::new(track.as_ref().map(|t| t.project()), task_receiver);
        let shared_source = SharedColumnSource::new(source);
        Self {
            preview_register: {
                PlayingPreviewRegister::new(shared_source.clone(), track.as_ref())
            },
            track,
            column_source: shared_source,
            task_sender,
        }
    }

    pub fn fill_slot(&mut self, args: ColumnFillSlotArgs) {
        self.with_source_mut(|s| s.fill_slot(args));
    }

    pub fn poll_slot(&mut self, args: ColumnPollSlotArgs) -> Option<ClipChangedEvent> {
        self.with_source_mut(|s| s.poll_slot(args))
    }

    pub fn with_slot<R>(
        &self,
        index: usize,
        f: impl FnOnce(&Slot) -> Result<R, &'static str>,
    ) -> Result<R, &'static str> {
        self.with_source(|s| s.with_slot(index, f))
    }

    pub fn play_clip(&mut self, args: ColumnPlayClipArgs) {
        self.send_source_task(ColumnSourceTask::PlayClip(args));
    }

    pub fn stop_clip(&mut self, args: ColumnStopClipArgs) {
        self.send_source_task(ColumnSourceTask::StopClip(args));
    }

    pub fn set_clip_repeated(&mut self, args: ColumnSetClipRepeatedArgs) {
        self.send_source_task(ColumnSourceTask::SetClipRepeated(args));
    }

    pub fn toggle_clip_repeated(&mut self, index: usize) -> Result<ClipChangedEvent, &'static str> {
        self.with_source_mut(|s| s.toggle_clip_repeated(index))
    }

    pub fn record_clip(
        &mut self,
        index: usize,
        behavior: RecordBehavior,
        equipment: RecorderEquipment,
    ) -> Result<ClipRecordTask, &'static str> {
        self.with_source_mut(|s| s.record_clip(index, behavior, equipment))?;
        let task = ClipRecordTask {
            column_source: self.column_source.clone(),
            slot_index: index,
        };
        Ok(task)
    }

    pub fn pause_clip(&mut self, index: usize) {
        let args = ColumnPauseClipArgs { index };
        self.send_source_task(ColumnSourceTask::PauseClip(args));
    }

    pub fn seek_clip(&mut self, index: usize, desired_pos: UnitValue) {
        let args = ColumnSeekClipArgs { index, desired_pos };
        self.send_source_task(ColumnSourceTask::SeekClip(args));
    }

    pub fn set_clip_volume(&mut self, index: usize, volume: ReaperVolumeValue) {
        let args = ColumnSetClipVolumeArgs { index, volume };
        self.send_source_task(ColumnSourceTask::SetClipVolume(args));
    }

    /// This method should be called whenever REAPER's play state changes. It will make the clip
    /// start/stop synchronized with REAPER's transport.
    pub fn process_transport_change(&mut self, args: SlotProcessTransportChangeArgs) {
        self.send_source_task(ColumnSourceTask::ProcessTransportChange(args));
    }

    fn with_source<R>(&self, f: impl FnOnce(&ColumnSource) -> R) -> R {
        let guard = self.column_source.lock();
        f(&guard)
    }

    fn with_source_mut<R>(&mut self, f: impl FnOnce(&mut ColumnSource) -> R) -> R {
        let mut guard = self.column_source.lock();
        f(&mut guard)
    }

    fn send_source_task(&self, task: ColumnSourceTask) {
        self.task_sender.try_send(task).unwrap();
    }
}

fn lock(reg: &SharedRegister) -> ReaperMutexGuard<OwnedPreviewRegister> {
    reg.lock().expect("couldn't acquire lock")
}

impl Drop for Column {
    fn drop(&mut self) {
        self.preview_register
            .stop_playing_preview(self.track.as_ref());
    }
}
impl PlayingPreviewRegister {
    pub fn new(source: impl CustomPcmSource + 'static, track: Option<&Track>) -> Self {
        let mut register = OwnedPreviewRegister::default();
        register.set_volume(ReaperVolumeValue::ZERO_DB);
        let (out_chan, preview_track) = if let Some(t) = track {
            (-1, Some(t.raw()))
        } else {
            (0, None)
        };
        register.set_out_chan(out_chan);
        register.set_preview_track(preview_track);
        let source = create_custom_owned_pcm_source(source);
        register.set_src(Some(FlexibleOwnedPcmSource::Custom(source)));
        let preview_register = Arc::new(ReaperMutex::new(register));
        let play_handle = start_playing_preview(&preview_register, track);
        Self {
            preview_register,
            play_handle,
        }
    }

    fn stop_playing_preview(&mut self, track: Option<&Track>) {
        if let Some(track) = track {
            // Check prevents error message on project close.
            let project = track.project();
            if project.is_available() {
                // If not successful this probably means it was stopped already, so okay.
                let _ = Reaper::get()
                    .medium_session()
                    .stop_track_preview_2(project.context(), self.play_handle);
            }
        } else {
            // If not successful this probably means it was stopped already, so okay.
            let _ = Reaper::get()
                .medium_session()
                .stop_preview(self.play_handle);
        };
    }
}

fn start_playing_preview(
    reg: &SharedRegister,
    track: Option<&Track>,
) -> NonNull<preview_register_t> {
    let buffering_behavior = BitFlags::empty();
    let measure_alignment = MeasureAlignment::PlayImmediately;
    let result = if let Some(track) = track {
        Reaper::get().medium_session().play_track_preview_2_ex(
            track.project().context(),
            reg.clone(),
            buffering_behavior,
            measure_alignment,
        )
    } else {
        Reaper::get().medium_session().play_preview_ex(
            reg.clone(),
            buffering_behavior,
            measure_alignment,
        )
    };
    result.unwrap()
}
