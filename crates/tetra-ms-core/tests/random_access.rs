use tetra_ms_core::{
    random_access::*,
    types::{TdmaTime, tdma_time::TIME_INT_WRAP},
};

fn params(imm: u8) -> RandomAccessParams {
    RandomAccessParams {
        imm,
        wt: 1,
        nu: 3,
        fl_factor: false,
        ts_ptr: 0,
        min_pdu_prio: 0,
        subscriber_class: None,
        gssi: None,
    }
}

fn assign(marker: bool) -> SlotAccessAssign {
    SlotAccessAssign {
        subslot1: SubslotAccess {
            access_code: Some(AccessCode::A),
            frame_len: if marker {
                BaseFrameLength::FrameMarker(2)
            } else {
                BaseFrameLength::OngoingFrame
            },
        },
        subslot2: SubslotAccess {
            access_code: Some(AccessCode::A),
            frame_len: BaseFrameLength::OngoingFrame,
        },
    }
}

#[test]
fn entropy_is_requested_without_mutation_and_invalid_choices_are_atomic() {
    let mut core = MsRandomAccess::new();
    let now = TdmaTime::default();
    let params = params(0);
    core.initiate(now, AccessCode::A, &params, 0, false).unwrap();
    assert_eq!(
        core.entropy_needed(now, &assign(true), true, &params),
        Ok(Some(EntropyRequest::SubslotIndex(2)))
    );
    assert_eq!(
        core.poll_downlink_slot(now, &assign(true), true, &params, &mut RaChoices::new(3, Subslot::One)),
        Err(RaFailure::InvalidEntropy)
    );
    assert_eq!(
        core.entropy_needed(now, &assign(true), true, &params),
        Ok(Some(EntropyRequest::SubslotIndex(2)))
    );
    assert_eq!(
        core.poll_downlink_slot(now, &assign(true), true, &params, &mut RaChoices::new(2, Subslot::One)),
        Ok(Some(RaAction::Transmit {
            ul_time: now.add_timeslots(2),
            subslot: Subslot::Two
        }))
    );
    assert_eq!(core.initiate(now, AccessCode::A, &params, 0, false), Err(RaFailure::Busy));
}

#[test]
fn first_try_window_expires_across_hyperframe_wrap() {
    let mut core = MsRandomAccess::new();
    let start = TdmaTime::from_int(TIME_INT_WRAP - 4);
    let params = params(1);
    core.initiate(start, AccessCode::A, &params, 0, false).unwrap();
    let now = start.add_timeslots(8);
    assert_eq!(
        core.poll_downlink_slot(now, &assign(false), true, &params, &mut RaChoices::new(1, Subslot::One)),
        Ok(None)
    );
    assert_eq!(
        core.entropy_needed(now, &assign(true), true, &params),
        Ok(Some(EntropyRequest::SubslotIndex(2)))
    );
}

#[test]
fn malformed_time_and_parameters_are_rejected_before_starting() {
    let mut core = MsRandomAccess::new();
    let now = TdmaTime::default();
    assert_eq!(
        core.initiate(TdmaTime { t: 0, ..now }, AccessCode::A, &params(15), 0, false),
        Err(RaFailure::InvalidTime)
    );
    let mut invalid = params(15);
    invalid.nu = 255;
    assert_eq!(
        core.initiate(now, AccessCode::A, &invalid, 0, true),
        Err(RaFailure::InvalidParameters)
    );
    assert!(!core.is_active());
}
