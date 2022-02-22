use crate::metrics_util::measure_time;
use crate::processing::supplier::{RecorderEquipment, WriteAudioRequest, WriteMidiRequest};
use crate::processing::SlotInstruction::KeepSlot;
use crate::processing::{
    Clip, ClipChangedEvent, ClipPlayArgs, ClipPlayState, ClipProcessArgs, ClipRecordArgs,
    ClipRecordInput, ClipStopArgs, ClipStopBehavior, RecordBehavior, SlotInstruction,
};
use crate::timeline::{HybridTimeline, Timeline, TimelineMoment};
use crate::ClipEngineResult;
use helgoboss_learn::UnitValue;
use reaper_high::Project;
use reaper_medium::{Bpm, PlayState, ReaperVolumeValue};

#[derive(Debug, Default)]
pub struct Slot {
    clip: Option<Clip>,
    runtime_data: RuntimeData,
}

#[derive(Debug, Default)]
struct RuntimeData {
    last_play_state: ClipPlayState,
    last_play: Option<LastPlay>,
    stop_was_caused_by_transport_change: bool,
}

#[derive(Copy, Clone, Debug)]
pub struct LastPlay {
    was_synced_to_bar: bool,
}

impl Slot {
    pub fn fill(&mut self, clip: Clip) {
        // TODO-medium Suspend previous clip if playing.
        self.clip = Some(clip);
    }

    pub fn clip(&self) -> ClipEngineResult<&Clip> {
        self.get_clip()
    }

    pub fn clip_mut(&mut self) -> ClipEngineResult<&mut Clip> {
        self.get_clip_mut()
    }

    pub fn play_clip(&mut self, args: ClipPlayArgs) -> ClipEngineResult<()> {
        self.runtime_data.last_play = Some(LastPlay {
            was_synced_to_bar: args.from_bar.is_some(),
        });
        self.get_clip_mut()?.play(args);
        Ok(())
    }

    pub fn stop_clip(&mut self, args: ClipStopArgs) -> ClipEngineResult<()> {
        self.runtime_data.stop_was_caused_by_transport_change = false;
        if self.get_clip_mut()?.stop(args) == SlotInstruction::ClearSlot {
            self.clip = None;
        }
        Ok(())
    }

    pub fn set_clip_repeated(&mut self, repeated: bool) -> ClipEngineResult<()> {
        self.get_clip_mut()?.set_repeated(repeated);
        Ok(())
    }

    pub fn record_clip(
        &mut self,
        behavior: RecordBehavior,
        input: ClipRecordInput,
        project: Option<Project>,
        equipment: RecorderEquipment,
    ) -> ClipEngineResult<()> {
        use RecordBehavior::*;
        match behavior {
            Normal {
                play_after,
                timing,
                detect_downbeat,
            } => {
                let args = ClipRecordArgs {
                    play_after,
                    input,
                    timing,
                    detect_downbeat,
                };
                match &mut self.clip {
                    None => self.clip = Some(Clip::from_recording(args, project, equipment)),
                    Some(clip) => clip.record(args),
                }
            }
            MidiOverdub => {
                self.get_clip_mut()?.midi_overdub();
            }
        }
        Ok(())
    }

    pub fn pause_clip(&mut self) -> ClipEngineResult<()> {
        self.get_clip_mut()?.pause();
        Ok(())
    }

    pub fn seek_clip(&mut self, desired_pos: UnitValue) -> ClipEngineResult<()> {
        self.get_clip_mut()?.seek(desired_pos);
        Ok(())
    }

    pub fn clip_record_input(&self) -> Option<ClipRecordInput> {
        self.get_clip().ok()?.record_input()
    }

    pub fn write_clip_midi(&mut self, request: WriteMidiRequest) -> ClipEngineResult<()> {
        self.get_clip_mut()?.write_midi(request);
        Ok(())
    }

    pub fn write_clip_audio(&mut self, request: WriteAudioRequest) -> ClipEngineResult<()> {
        self.get_clip_mut()?.write_audio(request);
        Ok(())
    }

    pub fn set_clip_volume(
        &mut self,
        volume: ReaperVolumeValue,
    ) -> ClipEngineResult<ClipChangedEvent> {
        Ok(self.get_clip_mut()?.set_volume(volume))
    }

    pub fn process_transport_change(&mut self, args: &SlotProcessTransportChangeArgs) {
        let slot_instruction = {
            let clip = match &mut self.clip {
                None => return,
                Some(c) => c,
            };
            match args.change {
                TransportChange::PlayState(rel_change) => {
                    // We have a relevant transport change.
                    let last_play = match self.runtime_data.last_play {
                        None => return,
                        Some(a) => a,
                    };
                    // Clip was started at least once already.
                    let state = clip.play_state();
                    use ClipPlayState::*;
                    use RelevantPlayStateChange::*;
                    match rel_change {
                        PlayAfterStop => {
                            match state {
                                Stopped
                                    if self.runtime_data.stop_was_caused_by_transport_change =>
                                {
                                    // REAPER transport was started from stopped state. Clip is stopped
                                    // as well and was put in that state due to a previous transport
                                    // stop. Play the clip!
                                    let args = ClipPlayArgs {
                                        from_bar: Some(args.moment.next_bar()),
                                    };
                                    clip.play(args);
                                    SlotInstruction::KeepSlot
                                }
                                _ => {
                                    // Stop and forget (because we have a timeline switch).
                                    self.runtime_data.stop_clip_by_transport(clip, args, false)
                                }
                            }
                        }
                        StopAfterPlay => match state {
                            ScheduledForPlay | Playing | ScheduledForStop
                                if last_play.was_synced_to_bar =>
                            {
                                // Stop and memorize
                                self.runtime_data.stop_clip_by_transport(clip, args, true)
                            }
                            _ => {
                                // Stop and forget
                                self.runtime_data.stop_clip_by_transport(clip, args, false)
                            }
                        },
                        StopAfterPause => {
                            self.runtime_data.stop_clip_by_transport(clip, args, false)
                        }
                    }
                }
                TransportChange::PlayCursorJump => {
                    // The play cursor was repositioned.
                    let last_play = match self.runtime_data.last_play {
                        None => return,
                        Some(a) => a,
                    };
                    if !last_play.was_synced_to_bar {
                        return;
                    }
                    let play_state = clip.play_state();
                    use ClipPlayState::*;
                    if !matches!(play_state, ScheduledForPlay | Playing | ScheduledForStop) {
                        return;
                    }
                    clip.play(ClipPlayArgs {
                        from_bar: Some(args.moment.next_bar()),
                    });
                    KeepSlot
                }
            }
        };
        if slot_instruction == SlotInstruction::ClearSlot {
            self.clip = None;
        }
    }

    pub fn process(
        &mut self,
        args: &mut ClipProcessArgs<impl Timeline>,
    ) -> ClipEngineResult<Option<ClipPlayState>> {
        measure_time("slot.process.time", || {
            let clip = self.get_clip_mut()?;
            clip.process(args);
            let play_state = clip.play_state();
            let last_play_state = self.runtime_data.last_play_state;
            let changed_play_state = if play_state == last_play_state {
                None
            } else {
                debug!("Clip state changed: {:?}", play_state);
                self.runtime_data.last_play_state = play_state;
                Some(play_state)
            };
            Ok(changed_play_state)
        })
    }

    fn get_clip(&self) -> ClipEngineResult<&Clip> {
        self.clip.as_ref().ok_or(SLOT_NOT_FILLED)
    }

    fn get_clip_mut(&mut self) -> ClipEngineResult<&mut Clip> {
        self.clip.as_mut().ok_or(SLOT_NOT_FILLED)
    }
}

impl RuntimeData {
    fn stop_clip_by_transport(
        &mut self,
        clip: &mut Clip,
        args: &SlotProcessTransportChangeArgs,
        keep_starting_with_transport: bool,
    ) -> SlotInstruction {
        self.stop_was_caused_by_transport_change = keep_starting_with_transport;
        clip.stop(ClipStopArgs {
            stop_behavior: ClipStopBehavior::Immediately,
            timeline_cursor_pos: args.moment.cursor_pos(),
            timeline: args.timeline.clone(),
        })
    }
}

pub struct SlotPollArgs {
    pub timeline_tempo: Bpm,
}

#[derive(Clone, Debug)]
pub struct SlotProcessTransportChangeArgs {
    pub change: TransportChange,
    pub moment: TimelineMoment,
    pub timeline: HybridTimeline,
}

const SLOT_NOT_FILLED: &str = "slot not filled";

#[derive(Copy, Clone, Debug)]
pub enum TransportChange {
    PlayState(RelevantPlayStateChange),
    PlayCursorJump,
}
#[derive(Copy, Clone, Debug)]
pub enum RelevantPlayStateChange {
    PlayAfterStop,
    StopAfterPlay,
    StopAfterPause,
}

impl RelevantPlayStateChange {
    pub fn from_play_state_change(old: PlayState, new: PlayState) -> Option<Self> {
        use RelevantPlayStateChange::*;
        let change = if !old.is_paused && !old.is_playing && new.is_playing {
            PlayAfterStop
        } else if old.is_playing && !new.is_playing && !new.is_paused {
            StopAfterPlay
        } else if old.is_paused && !new.is_playing && !new.is_paused {
            StopAfterPause
        } else {
            return None;
        };
        Some(change)
    }
}