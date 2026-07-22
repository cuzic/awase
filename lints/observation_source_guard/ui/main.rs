// UI test: verify RESTRICTED_INPUT_MODE_OBSERVATION_SOURCE fires on
// `ImeEvent::InputModeObserved { source: ObservationSource::X, .. }` where X is
// disguised (ImmGetOpenStatus, never allowed) or out-of-place (ConvBitsInference,
// only allowed in `apply_idle_conv_check`).

/// Minimal inline mock matching the shape of
/// `awase_windows::state::ime_event::{ImeEvent, ObservationSource}`.
mod ime_event {
    #[derive(Debug, Clone, Copy)]
    pub enum ObservationSource {
        ImmGetOpenStatus,
        ConvBitsInference,
        GjiIoInference,
        FocusProbe,
    }

    #[derive(Debug, Clone, Copy)]
    pub enum ObservationConfidence {
        High,
    }

    #[derive(Debug, Clone)]
    pub enum ImeEvent {
        ObserverReported {
            open: bool,
            source: ObservationSource,
            confidence: ObservationConfidence,
        },
        InputModeObserved {
            source: ObservationSource,
            confidence: ObservationConfidence,
        },
    }
}

use ime_event::{ImeEvent, ObservationConfidence, ObservationSource};

fn apply_idle_conv_check() -> ImeEvent {
    // Should NOT trigger: this is the designated function for ConvBitsInference.
    ImeEvent::InputModeObserved {
        source: ObservationSource::ConvBitsInference,
        confidence: ObservationConfidence::High,
    }
}

fn some_other_helper_reusing_conv_bits() -> ImeEvent {
    // Should trigger: ConvBitsInference constructed outside its designated function.
    ImeEvent::InputModeObserved {
        source: ObservationSource::ConvBitsInference, //~ WARN constructing `InputModeObserved`
        confidence: ObservationConfidence::High,
    }
}

fn reset_stale_input_mode_for_some_new_reason() -> ImeEvent {
    // Should trigger: ImmGetOpenStatus is never a legitimate InputModeObserved
    // source (that API reports open/close, not conversion mode) — this is the
    // exact disguise pattern the lint exists to catch.
    ImeEvent::InputModeObserved {
        source: ObservationSource::ImmGetOpenStatus, //~ WARN constructing `InputModeObserved`
        confidence: ObservationConfidence::High,
    }
}

fn ir_stage_observe() -> ImeEvent {
    // Should NOT trigger: this is the designated function for GjiIoInference.
    ImeEvent::InputModeObserved {
        source: ObservationSource::GjiIoInference,
        confidence: ObservationConfidence::High,
    }
}

fn some_new_recovery_path_reusing_gji_inference() -> ImeEvent {
    // Should trigger: GjiIoInference constructed outside its designated function.
    ImeEvent::InputModeObserved {
        source: ObservationSource::GjiIoInference, //~ WARN constructing `InputModeObserved`
        confidence: ObservationConfidence::High,
    }
}

fn legitimate_open_close_observation() -> ImeEvent {
    // Should NOT trigger: ImmGetOpenStatus paired with ObserverReported (a
    // different event variant) is the real, honest use of that source.
    ImeEvent::ObserverReported {
        open: true,
        source: ObservationSource::ImmGetOpenStatus,
        confidence: ObservationConfidence::High,
    }
}

fn unrelated_source_is_fine() -> ImeEvent {
    // Should NOT trigger: FocusProbe is not in the restricted list.
    ImeEvent::InputModeObserved {
        source: ObservationSource::FocusProbe,
        confidence: ObservationConfidence::High,
    }
}

fn main() {
    let _ = apply_idle_conv_check();
    let _ = some_other_helper_reusing_conv_bits();
    let _ = reset_stale_input_mode_for_some_new_reason();
    let _ = ir_stage_observe();
    let _ = some_new_recovery_path_reusing_gji_inference();
    let _ = legitimate_open_close_observation();
    let _ = unrelated_source_is_fine();
}
