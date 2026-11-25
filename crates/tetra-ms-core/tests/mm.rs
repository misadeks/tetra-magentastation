use tetra_ms_core::mm::{
    Action, CellId, Config, DETACH_DRAIN_SLOTS, Error, Event, GroupOperation, GroupRequest, MAX_GROUPS, MAX_REGISTRATION_ATTEMPTS, MmCore,
    RegState, T351_TIMEOUT_SLOTS, T353_TIMEOUT_SLOTS, TransferState,
};
use tetra_ms_types::bits::{BitView, PackedBits};

#[test]
fn zero_issi_is_rejected_by_all_configuration_entry_points() {
    let invalid = Config {
        issi: 0,
        mcc: 234,
        mnc: 5678,
        initial_la: 100,
    };
    assert!(matches!(MmCore::new(invalid, &[]), Err(Error::InvalidIdentity)));
    let mut storage = core::mem::MaybeUninit::<MmCore>::uninit();
    assert!(matches!(
        MmCore::init_in_place(&mut storage, invalid, &[]),
        Err(Error::InvalidIdentity)
    ));
    let mut engine = core(&[]);
    assert_eq!(engine.configure(invalid), Err(Error::InvalidIdentity));
    activate(&mut engine);
    assert_eq!(engine.state().reg_state, RegState::Registering);
}

#[test]
fn malformed_cell_identifiers_do_not_start_or_mutate_registration() {
    let mut engine = core(&[]);
    let valid = CellId {
        mcc: 234,
        mnc: 5678,
        la: 100,
    };
    for cell in [
        CellId { mcc: 1024, ..valid },
        CellId { mnc: 16384, ..valid },
        CellId { la: 16384, ..valid },
    ] {
        assert_eq!(
            engine.handle(Event::Activate {
                cell,
                registration_required: true,
                system_wide_services: true,
                force_register: false,
            }),
            Err(Error::InvalidIdentity)
        );
        assert_eq!(engine.state().current_cell, None);
        assert_eq!(engine.action_count(), 0);
    }
    let invalid = Config {
        issi: 1_000_001,
        mcc: 234,
        mnc: 5678,
        initial_la: 16384,
    };
    assert!(matches!(MmCore::new(invalid, &[]), Err(Error::InvalidIdentity)));
    assert_eq!(engine.configure(invalid), Err(Error::InvalidIdentity));
    activate(&mut engine);
    assert_eq!(engine.state().reg_state, RegState::Registering);
}

fn core(groups: &[u32]) -> MmCore {
    MmCore::new(
        Config {
            issi: 1_000_001,
            mcc: 234,
            mnc: 5678,
            initial_la: 100,
        },
        groups,
    )
    .unwrap()
}

fn bits(text: &str) -> PackedBits {
    let mut result = PackedBits::new();
    for bit in text.bytes() {
        result.push(1, u32::from(bit - b'0')).unwrap();
    }
    result
}

fn bit_string(bits: &PackedBits) -> String {
    (0..bits.len())
        .map(|pos| if bits.view().read(pos, 1).unwrap() == 0 { '0' } else { '1' })
        .collect()
}

fn drain(core: &mut MmCore) -> Vec<Action> {
    std::iter::from_fn(|| core.poll_action()).collect()
}

fn activate(core: &mut MmCore) {
    core.handle(Event::Activate {
        cell: CellId {
            mcc: 234,
            mnc: 5678,
            la: 100,
        },
        registration_required: true,
        system_wide_services: true,
        force_register: false,
    })
    .unwrap();
}

fn register(core: &mut MmCore) {
    activate(core);
    drain(core);
    core.handle(Event::Downlink(bits("01010110").view())).unwrap();
    drain(core);
}

fn tick(core: &mut MmCore, transfer: TransferState) {
    core.handle(Event::Tick {
        elapsed_slots: 1,
        detach_transfer: transfer,
    })
    .unwrap();
}

fn group_ack(attached: &[u32], detached: &[u32]) -> PackedBits {
    let mut payload = PackedBits::new();
    payload.push(6, (attached.len() + detached.len()) as u32).unwrap();
    for (groups, detachment) in [(attached, false), (detached, true)] {
        for &gssi in groups {
            payload.push(1, u32::from(detachment)).unwrap();
            payload.push(if detachment { 2 } else { 5 }, 0).unwrap();
            payload.push(2, 0).unwrap();
            payload.push(24, gssi).unwrap();
        }
    }
    let mut result = bits("1011001");
    result.push(1, 1).unwrap();
    result.push(4, 7).unwrap();
    result.push(11, u32::from(payload.len())).unwrap();
    result.append(payload.view()).unwrap();
    result.push(1, 0).unwrap();
    result
}

#[test]
fn original_registration_group_and_detach_golden_vectors() {
    eprintln!(
        "MM footprint: core={} state={} action={}",
        core::mem::size_of::<MmCore>(),
        core::mem::size_of::<tetra_ms_core::mm::MmState>(),
        core::mem::size_of::<Action>(),
    );
    let mut engine = core(&[91, 220]);
    activate(&mut engine);
    let actions = drain(&mut engine);
    let Action::Send {
        sdu,
        priority,
        detach_receipt,
    } = &actions[0]
    else {
        panic!()
    };
    assert_eq!(bit_string(sdu), "0010011000");
    assert_eq!((*priority, *detach_receipt), (6, false));
    engine.handle(Event::Downlink(bits("01010110").view())).unwrap();
    let actions = drain(&mut engine);
    let Action::Send { sdu, priority, .. } = &actions[2] else {
        panic!()
    };
    assert_eq!(
        bit_string(sdu),
        "011100111000000010000100000100100000000000000000000010110110100000000000000000000110111000"
    );
    assert_eq!(*priority, 3);
    engine.handle(Event::Deregister).unwrap();
    let actions = drain(&mut engine);
    let Action::Send {
        sdu,
        priority,
        detach_receipt,
    } = &actions[0]
    else {
        panic!()
    };
    assert_eq!(bit_string(sdu), "00010");
    assert_eq!((*priority, *detach_receipt), (3, true));
}

#[test]
fn retry_boundaries_and_caps_match_recovered_slot_timing() {
    let mut engine = core(&[]);
    activate(&mut engine);
    drain(&mut engine);
    for attempt in 1..=MAX_REGISTRATION_ATTEMPTS {
        for _ in 0..T351_TIMEOUT_SLOTS {
            tick(&mut engine, TransferState::Pending);
            assert!(drain(&mut engine).is_empty());
        }
        assert_eq!(engine.state().attempts, attempt);
        tick(&mut engine, TransferState::Pending);
        let actions = drain(&mut engine);
        if attempt < MAX_REGISTRATION_ATTEMPTS {
            assert!(matches!(actions[0], Action::Send { .. }));
            assert_eq!(engine.state().attempts, attempt + 1);
        } else {
            assert!(actions.is_empty());
            assert_eq!(engine.state().reg_state, RegState::Idle);
        }
    }
}

#[test]
fn malformed_downlinks_and_bounded_group_errors_are_atomic() {
    let mut engine = core(&[]);
    activate(&mut engine);
    drain(&mut engine);
    let before = engine.state().clone();
    // Type-4 group-list length 5 cannot even contain its six-bit count.
    let mut malformed = bits("1011001");
    malformed.push(1, 1).unwrap();
    malformed.push(4, 7).unwrap();
    malformed.push(11, 5).unwrap();
    malformed.push(5, 0).unwrap();
    malformed.push(1, 0).unwrap();
    assert!(engine.handle(Event::Downlink(malformed.view())).is_err());
    assert_eq!(engine.state(), &before);
    assert!(drain(&mut engine).is_empty());

    register(&mut engine);
    let before = engine.state().clone();
    let request = GroupRequest {
        gssi: 1,
        operation: GroupOperation::Attach { class_of_usage: 4 },
    };
    let requests = [request; MAX_GROUPS + 1];
    assert_eq!(
        engine.handle(Event::Groups {
            handle: 1,
            detach_all: false,
            report_requested: false,
            requests: &requests
        }),
        Err(Error::GroupCapacity)
    );
    assert_eq!(engine.state(), &before);
    assert!(drain(&mut engine).is_empty());
    assert_eq!(engine.configure_groups(&(0..64).collect::<Vec<_>>()), Err(Error::GroupCapacity));
    assert_eq!(engine.state(), &before);
}

#[test]
fn output_backpressure_preserves_state_and_previous_actions() {
    let mut engine = core(&[]);
    activate(&mut engine);
    // Each event reserves room for its worst case before changing state.
    engine.handle(Event::LinkBreak).unwrap();
    engine.handle(Event::LinkReopen).unwrap();
    let before = engine.state().clone();
    assert_eq!(engine.handle(Event::LinkBreak), Err(Error::ActionCapacity));
    assert_eq!(engine.state(), &before);
    assert_eq!(drain(&mut engine).len(), 4);
    engine.handle(Event::LinkBreak).unwrap();
}

#[test]
fn command_registration_retry_preserves_original_minimal_demand_shape() {
    let mut engine = core(&[91, 220]);
    engine.handle(Event::Downlink(bits("0110100").view())).unwrap();
    drain(&mut engine);
    // D-LOCATION-UPDATE-REJECT: demand updating, network failure, clear, no options.
    engine.handle(Event::Downlink(bits("01111100010100").view())).unwrap();
    assert!(drain(&mut engine).is_empty());
    tick(&mut engine, TransferState::Pending);
    let actions = drain(&mut engine);
    let Action::Send { sdu, priority, .. } = &actions[0] else {
        panic!()
    };
    // The historical resend intentionally omits the original class/MNI/groups.
    assert_eq!(bit_string(sdu), "0010110000");
    assert_eq!(*priority, 6);
    assert_eq!(engine.state().attempts, 2);
}

#[test]
fn detach_final_receipt_and_drain_fallback_report_once() {
    for (transfer, ticks, successful) in [
        (TransferState::Acknowledged, 1, true),
        (TransferState::Discarded, 1, false),
        (TransferState::Lost, 1, false),
        (TransferState::Transmitted, DETACH_DRAIN_SLOTS, true),
        (TransferState::Pending, DETACH_DRAIN_SLOTS, false),
    ] {
        let mut engine = core(&[]);
        register(&mut engine);
        engine.handle(Event::Deregister).unwrap();
        drain(&mut engine);
        for step in 0..ticks {
            tick(&mut engine, transfer);
            if step + 1 < ticks {
                assert!(drain(&mut engine).is_empty());
            }
        }
        assert_eq!(
            drain(&mut engine),
            vec![Action::DetachReport { successful }, Action::DetachComplete]
        );
        assert_eq!(engine.state().reg_state, RegState::Idle);
        tick(&mut engine, transfer);
        assert!(drain(&mut engine).is_empty());
    }
}

#[test]
fn group_timeout_keeps_confirmed_set_and_reports_empty_result() {
    let mut engine = core(&[]);
    register(&mut engine);
    let requests = [GroupRequest {
        gssi: 300,
        operation: GroupOperation::Attach { class_of_usage: 3 },
    }];
    engine
        .handle(Event::Groups {
            handle: 42,
            detach_all: true,
            report_requested: true,
            requests: &requests,
        })
        .unwrap();
    drain(&mut engine);
    for _ in 0..T353_TIMEOUT_SLOTS {
        tick(&mut engine, TransferState::Pending);
        assert!(drain(&mut engine).is_empty());
    }
    tick(&mut engine, TransferState::Pending);
    let actions = drain(&mut engine);
    let [Action::GroupConfirm { op, attached, detached }] = actions.as_slice() else {
        panic!()
    };
    assert_eq!(op.handle, 42);
    assert!(op.detach_all && op.report_requested);
    assert!(attached.is_empty() && detached.is_empty());
    assert!(engine.state().attached_gssis.is_empty());
}

#[test]
fn every_truncated_downlink_prefix_leaves_state_unchanged() {
    let examples = [bits("01010110"), bits("0110000"), bits("01110110011000"), bits("1011000")];
    for pdu in examples {
        for len in 0..pdu.len() {
            let mut engine = core(&[]);
            activate(&mut engine);
            drain(&mut engine);
            let before = engine.state().clone();
            let view = BitView::new(pdu.as_bytes(), len).unwrap();
            assert!(engine.handle(Event::Downlink(view)).is_err(), "prefix length {len}");
            assert_eq!(engine.state(), &before);
            assert!(drain(&mut engine).is_empty());
        }
    }
}

#[test]
fn malformed_group_body_is_validated_even_without_an_outstanding_operation() {
    let mut engine = core(&[]);
    let mut malformed = bits("1011001");
    malformed.push(1, 1).unwrap();
    malformed.push(4, 7).unwrap();
    malformed.push(11, 7).unwrap();
    malformed.push(6, 1).unwrap();
    malformed.push(1, 0).unwrap();
    malformed.push(1, 0).unwrap();
    let before = engine.state().clone();
    assert_eq!(
        engine.handle(Event::Downlink(malformed.view())),
        Err(Error::Bits(tetra_ms_types::bits::BitError::Truncated))
    );
    assert_eq!(engine.state(), &before);
    assert!(drain(&mut engine).is_empty());
}

#[test]
fn maximum_group_ack_borrows_a_non_aligned_window_without_retaining_input() {
    let mut engine = core(&[]);
    register(&mut engine);
    let groups: Vec<u32> = (1..=MAX_GROUPS as u32).rev().collect();
    let requests: Vec<GroupRequest> = groups
        .iter()
        .map(|&gssi| GroupRequest {
            gssi,
            operation: GroupOperation::Attach { class_of_usage: 4 },
        })
        .collect();
    engine
        .handle(Event::Groups {
            handle: 10,
            detach_all: true,
            report_requested: false,
            requests: &requests,
        })
        .unwrap();
    drain(&mut engine);
    let ack = group_ack(&groups, &[]);
    let mut input = PackedBits::new();
    input.push(3, 0b101).unwrap();
    input.append(ack.view()).unwrap();
    engine.handle(Event::Downlink(input.view().window(3, ack.len()).unwrap())).unwrap();
    input.clear();
    let actions = drain(&mut engine);
    let Action::Identities { attached, .. } = &actions[0] else {
        panic!()
    };
    assert_eq!(attached.as_slice(), &(1..=MAX_GROUPS as u32).collect::<Vec<_>>());
    let Action::GroupConfirm { attached, .. } = &actions[1] else {
        panic!()
    };
    assert_eq!(attached.as_slice(), groups);
}

#[test]
fn group_confirm_preserves_wire_order_but_filter_is_sorted_and_unique() {
    let mut engine = core(&[]);
    register(&mut engine);
    let requests = [GroupRequest {
        gssi: 900,
        operation: GroupOperation::Attach { class_of_usage: 3 },
    }];
    engine
        .handle(Event::Groups {
            handle: 10,
            detach_all: false,
            report_requested: false,
            requests: &requests,
        })
        .unwrap();
    drain(&mut engine);
    engine.handle(Event::Downlink(group_ack(&[900, 300, 900], &[]).view())).unwrap();
    let actions = drain(&mut engine);
    let Action::GroupConfirm { attached, .. } = &actions[1] else {
        panic!()
    };
    assert_eq!(attached.as_slice(), [900, 300, 900]);
    assert_eq!(engine.state().attached_gssis.as_slice(), [300, 900]);
}

#[test]
fn full_group_filter_amendment_checks_final_capacity_atomically() {
    let mut engine = core(&[]);
    register(&mut engine);
    engine.configure_groups(&(0..63).collect::<Vec<_>>()).unwrap();
    let requests = [GroupRequest {
        gssi: 64,
        operation: GroupOperation::Attach { class_of_usage: 3 },
    }];
    engine
        .handle(Event::Groups {
            handle: 10,
            detach_all: false,
            report_requested: false,
            requests: &requests,
        })
        .unwrap();
    drain(&mut engine);
    let before = engine.state().clone();
    assert_eq!(
        engine.handle(Event::Downlink(group_ack(&[64], &[]).view())),
        Err(Error::GroupCapacity)
    );
    assert_eq!(engine.state(), &before);
    assert!(drain(&mut engine).is_empty());
    engine.handle(Event::Downlink(group_ack(&[64], &[0]).view())).unwrap();
    assert_eq!(engine.state().attached_gssis.as_slice().len(), MAX_GROUPS);
    assert!(!engine.state().attached_gssis.contains(&0));
    assert!(engine.state().attached_gssis.contains(&64));
}

#[test]
fn batched_caller_time_matches_individual_slots_and_is_bounded() {
    let mut stepped = core(&[]);
    let mut batched = core(&[]);
    activate(&mut stepped);
    activate(&mut batched);
    drain(&mut stepped);
    drain(&mut batched);
    let elapsed = T351_TIMEOUT_SLOTS + 4;
    for _ in 0..elapsed {
        tick(&mut stepped, TransferState::Pending);
    }
    batched
        .handle(Event::Tick {
            elapsed_slots: elapsed,
            detach_transfer: TransferState::Pending,
        })
        .unwrap();
    assert_eq!(stepped.state(), batched.state());
    assert_eq!(drain(&mut stepped), drain(&mut batched));
    // Three queued retry transmissions do not fit behind these service actions.
    batched.handle(Event::LinkBreak).unwrap();
    batched.handle(Event::LinkReopen).unwrap();
    batched.handle(Event::LinkBreak).unwrap();
    let before = batched.state().clone();
    assert_eq!(
        batched.handle(Event::Tick {
            elapsed_slots: u32::MAX,
            detach_transfer: TransferState::Pending
        }),
        Err(Error::ActionCapacity)
    );
    assert_eq!(batched.state(), &before);
}
