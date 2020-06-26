use crate::core::MovingAverageCalculator;
use crate::domain::{
    MainProcessorTask, MappingId, MidiClockCalculator, MidiControlInput, MidiFeedbackOutput,
    MidiSourceScanner, RealTimeProcessorMapping,
};
use helgoboss_learn::{Bpm, MidiSource, MidiSourceValue};
use helgoboss_midi::{
    ControlChange14BitMessage, ControlChange14BitMessageScanner, MessageMainCategory,
    ParameterNumberMessage, ParameterNumberMessageScanner, RawShortMessage, ShortMessage,
    ShortMessageFactory, ShortMessageType, U7,
};
use reaper_high::Reaper;
use reaper_medium::{Hz, MidiFrameOffset, SendMidiTime};
use slog::{debug, info};
use std::collections::HashMap;
use std::convert::{TryFrom, TryInto};
use std::ptr::null_mut;
use vst::api::{Event, EventType, Events, MidiEvent, TimeInfo};
use vst::host::Host;
use vst::plugin::HostCallback;

const BULK_SIZE: usize = 100;

#[derive(PartialEq, Debug)]
pub(crate) enum ControlState {
    Controlling,
    LearningSource,
}

pub struct RealTimeProcessor {
    // Synced processing settings
    pub(crate) control_state: ControlState,
    pub(crate) midi_control_input: MidiControlInput,
    pub(crate) midi_feedback_output: Option<MidiFeedbackOutput>,
    pub(crate) mappings: HashMap<MappingId, RealTimeProcessorMapping>,
    pub(crate) let_matched_events_through: bool,
    pub(crate) let_unmatched_events_through: bool,
    // Inter-thread communication
    pub(crate) receiver: crossbeam_channel::Receiver<RealTimeProcessorTask>,
    pub(crate) main_processor_sender: crossbeam_channel::Sender<MainProcessorTask>,
    // Host communication
    pub(crate) host: HostCallback,
    // Scanners for more complex MIDI message types
    pub(crate) nrpn_scanner: ParameterNumberMessageScanner,
    pub(crate) cc_14_bit_scanner: ControlChange14BitMessageScanner,
    // For detecting play state changes
    pub(crate) was_playing_in_last_cycle: bool,
    // For source learning
    pub(crate) source_scanner: MidiSourceScanner,
    // For MIDI timing clock calculations
    pub(crate) midi_clock_calculator: MidiClockCalculator,
}

impl RealTimeProcessor {
    pub fn new(
        receiver: crossbeam_channel::Receiver<RealTimeProcessorTask>,
        main_processor_sender: crossbeam_channel::Sender<MainProcessorTask>,
        host_callback: HostCallback,
    ) -> RealTimeProcessor {
        RealTimeProcessor {
            control_state: ControlState::Controlling,
            receiver,
            main_processor_sender: main_processor_sender,
            mappings: Default::default(),
            let_matched_events_through: false,
            let_unmatched_events_through: false,
            nrpn_scanner: Default::default(),
            cc_14_bit_scanner: Default::default(),
            midi_control_input: MidiControlInput::FxInput,
            midi_feedback_output: None,
            host: host_callback,
            was_playing_in_last_cycle: false,
            source_scanner: Default::default(),
            midi_clock_calculator: Default::default(),
        }
    }

    pub fn process_incoming_midi_from_fx_input(
        &mut self,
        frame_offset: MidiFrameOffset,
        msg: RawShortMessage,
    ) {
        if self.midi_control_input == MidiControlInput::FxInput {
            let transport_is_starting = !self.was_playing_in_last_cycle && self.is_now_playing();
            if transport_is_starting && msg.r#type() == ShortMessageType::NoteOff {
                // Ignore note off messages which are a result of starting the transport. They
                // are generated by REAPER in order to stop instruments from sounding. But ReaLearn
                // is not an instrument in the classical sense. We don't want to reset target values
                // just because play has been pressed!
                self.process_unmatched_short(msg);
                return;
            }
            self.process_incoming_midi(frame_offset, msg);
        }
    }

    /// Should be called regularly in real-time audio thread.
    pub fn idle(&mut self, sample_count: usize) {
        // Increase MIDI clock calculator's sample counter
        self.midi_clock_calculator
            .increase_sample_counter_by(sample_count as u64);
        // Process tasks sent from other thread (probably main thread)
        for task in self.receiver.try_iter().take(BULK_SIZE) {
            use RealTimeProcessorTask::*;
            match task {
                UpdateAllMappings(mappings) => {
                    debug!(
                        Reaper::get().logger(),
                        "Real-time processor: Updating all mappings"
                    );
                    self.mappings = mappings.into_iter().map(|m| (m.id(), m)).collect();
                }
                UpdateSingleMapping { id, mapping } => {
                    debug!(
                        Reaper::get().logger(),
                        "Real-time processor: Updating mapping {:?}...", id
                    );
                    match mapping {
                        None => self.mappings.remove(&id),
                        Some(m) => self.mappings.insert(id, m),
                    };
                }
                UpdateSettings {
                    let_matched_events_through,
                    let_unmatched_events_through,
                    midi_control_input,
                    midi_feedback_output,
                } => {
                    debug!(
                        Reaper::get().logger(),
                        "Real-time processor: Updating settings"
                    );
                    self.let_matched_events_through = let_matched_events_through;
                    self.let_unmatched_events_through = let_unmatched_events_through;
                    self.midi_control_input = midi_control_input;
                    self.midi_feedback_output = midi_feedback_output;
                }
                UpdateSampleRate(sample_rate) => {
                    debug!(
                        Reaper::get().logger(),
                        "Real-time processor: Updating sample rate"
                    );
                    self.midi_clock_calculator.update_sample_rate(sample_rate);
                }
                StartLearnSource => {
                    debug!(
                        Reaper::get().logger(),
                        "Real-time processor: Start learn source"
                    );
                    self.control_state = ControlState::LearningSource;
                    self.nrpn_scanner.reset();
                    self.cc_14_bit_scanner.reset();
                    self.source_scanner.reset();
                }
                StopLearnSource => {
                    debug!(
                        Reaper::get().logger(),
                        "Real-time processor: Stop learn source"
                    );
                    self.control_state = ControlState::Controlling;
                    self.nrpn_scanner.reset();
                    self.cc_14_bit_scanner.reset();
                }
                Feedback(source_value) => {
                    self.feedback(source_value);
                }
                LogDebugInfo => {
                    self.log_debug_info();
                }
            }
        }
        // Get current time information so we can detect changes in play state reliably
        // (TimeInfoFlags::TRANSPORT_CHANGED doesn't work the way we want it).
        self.was_playing_in_last_cycle = self.is_now_playing();
        // Read MIDI events from devices
        if let MidiControlInput::Device(dev) = self.midi_control_input {
            dev.with_midi_input(|mi| {
                for evt in mi.get_read_buf().enum_items(0) {
                    self.process_incoming_midi(evt.frame_offset(), evt.message().to_other());
                }
            });
        }
        // Poll source scanner if we are learning a source currently
        if self.control_state == ControlState::LearningSource {
            self.poll_source_scanner()
        }
    }

    fn log_debug_info(&self) {
        info!(
            Reaper::get().logger(),
            "\n\
                        # Real-time processor\n\
                        \n\
                        - Control state: {:?} \n\
                        - Task queue length: {} \n\
                        - Mapping count: {} \n\
                        ",
            // self.mappings.values(),
            self.control_state,
            self.receiver.len(),
            self.mappings.len(),
        );
    }

    fn is_now_playing(&self) -> bool {
        use vst::api::TimeInfoFlags;
        let time_info = self
            .host
            .get_time_info(TimeInfoFlags::TRANSPORT_PLAYING.bits());
        match time_info {
            None => false,
            Some(ti) => {
                let flags = TimeInfoFlags::from_bits_truncate(ti.flags);
                flags.intersects(TimeInfoFlags::TRANSPORT_PLAYING)
            }
        }
    }

    fn process_incoming_midi(&mut self, frame_offset: MidiFrameOffset, msg: RawShortMessage) {
        use ShortMessageType::*;
        match msg.r#type() {
            NoteOff
            | NoteOn
            | PolyphonicKeyPressure
            | ControlChange
            | ProgramChange
            | ChannelPressure
            | PitchBendChange
            | Start
            | Continue
            | Stop => {
                self.process_incoming_midi_normal(msg);
            }
            SystemExclusiveStart
            | TimeCodeQuarterFrame
            | SongPositionPointer
            | SongSelect
            | SystemCommonUndefined1
            | SystemCommonUndefined2
            | TuneRequest
            | SystemExclusiveEnd
            | SystemRealTimeUndefined1
            | SystemRealTimeUndefined2
            | ActiveSensing
            | SystemReset => {
                // ReaLearn doesn't process those. Forward them if user wants it.
                self.process_unmatched_short(msg);
            }
            TimingClock => {
                // Timing clock messages are treated special (calculates BPM).
                if let Some(bpm) = self.midi_clock_calculator.feed(frame_offset) {
                    let source_value = MidiSourceValue::<RawShortMessage>::Tempo(bpm);
                    self.control(source_value);
                }
            }
        };
    }

    fn process_incoming_midi_normal(&mut self, msg: RawShortMessage) {
        // TODO-low This is probably unnecessary optimization, but we could switch off NRPN/CC14
        //  scanning if there's no such source.
        if let Some(nrpn_msg) = self.nrpn_scanner.feed(&msg) {
            self.process_incoming_midi_normal_nrpn(nrpn_msg);
        }
        if let Some(cc14_msg) = self.cc_14_bit_scanner.feed(&msg) {
            self.process_incoming_midi_normal_cc14(cc14_msg);
        }
        self.process_incoming_midi_normal_plain(msg);
    }

    fn process_incoming_midi_normal_nrpn(&mut self, msg: ParameterNumberMessage) {
        let source_value = MidiSourceValue::<RawShortMessage>::ParameterNumber(msg);
        match self.control_state {
            ControlState::Controlling => {
                let matched = self.control(source_value);
                if self.midi_control_input != MidiControlInput::FxInput {
                    return;
                }
                if (matched && self.let_matched_events_through)
                    || (!matched && self.let_unmatched_events_through)
                {
                    for m in msg
                        .to_short_messages::<RawShortMessage>()
                        .into_iter()
                        .flatten()
                    {
                        self.forward_midi(*m);
                    }
                }
            }
            ControlState::LearningSource => {
                self.feed_source_scanner(source_value);
            }
        }
    }

    fn poll_source_scanner(&mut self) {
        if let Some(source) = self.source_scanner.poll() {
            self.learn_source(source);
        }
    }

    fn feed_source_scanner(&mut self, value: MidiSourceValue<RawShortMessage>) {
        if let Some(source) = self.source_scanner.feed(value) {
            self.learn_source(source);
        }
    }

    fn learn_source(&mut self, source: MidiSource) {
        self.main_processor_sender
            .send(MainProcessorTask::LearnSource(source));
    }

    fn process_incoming_midi_normal_cc14(&mut self, msg: ControlChange14BitMessage) {
        let source_value = MidiSourceValue::<RawShortMessage>::ControlChange14Bit(msg);
        match self.control_state {
            ControlState::Controlling => {
                let matched = self.control(source_value);
                if self.midi_control_input != MidiControlInput::FxInput {
                    return;
                }
                if (matched && self.let_matched_events_through)
                    || (!matched && self.let_unmatched_events_through)
                {
                    for m in msg.to_short_messages::<RawShortMessage>().into_iter() {
                        self.forward_midi(*m);
                    }
                }
            }
            ControlState::LearningSource => {
                self.feed_source_scanner(source_value);
            }
        }
    }

    fn process_incoming_midi_normal_plain(&mut self, msg: RawShortMessage) {
        let source_value = MidiSourceValue::Plain(msg);
        match self.control_state {
            ControlState::Controlling => {
                if self.is_consumed(msg) {
                    return;
                }
                let matched = self.control(source_value);
                if matched {
                    self.process_matched_short(msg);
                } else {
                    self.process_unmatched_short(msg);
                }
            }
            ControlState::LearningSource => {
                self.feed_source_scanner(source_value);
            }
        }
    }

    /// Returns whether this source value matched one of the mappings.
    fn control(&self, value: MidiSourceValue<RawShortMessage>) -> bool {
        let mut matched = false;
        for m in self.mappings.values() {
            if let Some(control_value) = m.control(&value) {
                let main_processor_task = MainProcessorTask::Control {
                    mapping_id: m.id(),
                    value: control_value,
                };
                self.main_processor_sender.send(main_processor_task);
                matched = true;
            }
        }
        matched
    }

    fn process_matched_short(&self, msg: RawShortMessage) {
        if self.midi_control_input != MidiControlInput::FxInput {
            return;
        }
        if !self.let_matched_events_through {
            return;
        }
        self.forward_midi(msg);
    }

    fn process_unmatched_short(&self, msg: RawShortMessage) {
        if self.midi_control_input != MidiControlInput::FxInput {
            return;
        }
        if !self.let_unmatched_events_through {
            return;
        }
        self.forward_midi(msg);
    }

    fn is_consumed(&self, msg: RawShortMessage) -> bool {
        self.mappings.values().any(|m| m.consumes(msg))
    }

    fn feedback(&self, value: MidiSourceValue<RawShortMessage>) {
        if let Some(output) = self.midi_feedback_output {
            let shorts = value.to_short_messages();
            if shorts[0].is_none() {
                return;
            }
            match output {
                MidiFeedbackOutput::FxOutput => {
                    for short in shorts.into_iter().flatten() {
                        self.forward_midi(*short);
                    }
                }
                MidiFeedbackOutput::Device(dev) => {
                    dev.with_midi_output(|mo| {
                        for short in shorts.into_iter().flatten() {
                            mo.send(*short, SendMidiTime::Instantly);
                        }
                    });
                }
            };
        }
    }

    fn forward_midi(&self, msg: RawShortMessage) {
        let bytes = msg.to_bytes();
        let mut event = MidiEvent {
            event_type: EventType::Midi,
            byte_size: std::mem::size_of::<MidiEvent>() as _,
            delta_frames: 0,
            flags: vst::api::MidiEventFlags::REALTIME_EVENT.bits(),
            note_length: 0,
            note_offset: 0,
            midi_data: [bytes.0, bytes.1.get(), bytes.2.get()],
            _midi_reserved: 0,
            detune: 0,
            note_off_velocity: 0,
            _reserved1: 0,
            _reserved2: 0,
        };
        let events = Events {
            num_events: 1,
            _reserved: 0,
            events: [&mut event as *mut MidiEvent as _, null_mut()],
        };
        self.host.process_events(&events);
    }
}

#[derive(Debug)]
pub enum RealTimeProcessorTask {
    UpdateAllMappings(Vec<RealTimeProcessorMapping>),
    UpdateSingleMapping {
        id: MappingId,
        mapping: Option<RealTimeProcessorMapping>,
    },
    UpdateSettings {
        let_matched_events_through: bool,
        let_unmatched_events_through: bool,
        midi_control_input: MidiControlInput,
        midi_feedback_output: Option<MidiFeedbackOutput>,
    },
    LogDebugInfo,
    UpdateSampleRate(Hz),
    // TODO-low Is it better for performance to push a vector (smallvec) here?
    Feedback(MidiSourceValue<RawShortMessage>),
    StartLearnSource,
    StopLearnSource,
}
