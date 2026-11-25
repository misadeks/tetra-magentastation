use tetra_ms_core::cmce::*;
use tetra_ms_types::{SsiType, TdmaTime, TetraAddress};

fn call() -> CallState {
    CallState::new(7, MsCcState::MtCallSetup, MsCallKind::Group, false, true, 0).unwrap()
}

fn commit(call: &mut CallState, action: CallAction) {
    if let CallAction::Configure(u) = action {
        call.configure(u);
    }
}

#[test]
fn foreign_granted_talker_does_not_grant_self_and_not_granted_keeps_uplane() {
    let mut call = call();
    let now = TdmaTime::default();
    call.push_event(now, CallEvent::Demand { priority: 2 }).unwrap();
    let action = call
        .push_event(
            now,
            CallEvent::Grant {
                own: Some(123),
                grant: 0,
                speaker: Some(456),
            },
        )
        .unwrap();
    commit(&mut call, action);
    assert_eq!(call.tx_grant_state, MsTxGrantState::GrantedOther);
    assert!(call.pending_tx_request);
    assert!(call.accepts_downlink());
    assert!(!call.accepts_uplink());
    assert_eq!(
        call.push_event(
            now,
            CallEvent::Grant {
                own: Some(123),
                grant: 1,
                speaker: None
            }
        ),
        Ok(CallAction::None)
    );
    assert!(call.last_uplane.unwrap().switch_u_plane);
    assert_eq!(call.tx_grant_state, MsTxGrantState::None);
}

#[test]
fn wait_continue_restores_floor_and_false_continue_retains_saved_snapshot() {
    let mut call = call();
    let now = TdmaTime::default();
    let action = call
        .push_event(
            now,
            CallEvent::Grant {
                own: Some(123),
                grant: 0,
                speaker: None,
            },
        )
        .unwrap();
    commit(&mut call, action);
    assert!(call.accepts_uplink());
    assert!(!call.accepts_downlink());
    let action = call.push_event(now, CallEvent::Wait { permission_denied: true }).unwrap();
    commit(&mut call, action);
    assert_eq!(call.state, MsCcState::Wait);
    assert!(!call.transmission_request_allowed);
    assert_eq!(
        call.push_event(
            now,
            CallEvent::Continue {
                permission_denied: false,
                resume: false
            }
        ),
        Ok(CallAction::None)
    );
    assert!(call.uplane_before_wait.is_some());
    let action = call
        .push_event(
            now,
            CallEvent::Continue {
                permission_denied: false,
                resume: true,
            },
        )
        .unwrap();
    commit(&mut call, action);
    assert_eq!(call.tx_grant_state, MsTxGrantState::GrantedSelf);
    assert!(call.uplane_before_wait.is_none());
    assert!(call.accepts_uplink());
}

#[test]
fn caller_time_drives_setup_call_and_info_reset_deadlines() {
    let mut call = call();
    let now = TdmaTime::default();
    call.push_event(now, CallEvent::Proceed { timeout: 3 }).unwrap();
    assert!(!call.expired(now.add_timeslots(351)));
    assert!(call.expired(now.add_timeslots(352)));
    call.push_event(
        now,
        CallEvent::Connect {
            timeout: 1,
            permission_denied: false,
            duplex: Some(true),
        },
    )
    .unwrap();
    assert!(call.timers.setup_phase_deadline.is_none());
    let original = call.timers.call_deadline;
    call.push_event(
        now.add_timeslots(100),
        CallEvent::Info {
            setup: None,
            timeout: Some(2),
            reset: false,
        },
    )
    .unwrap();
    assert_eq!(call.timers.call_deadline, original);
    call.push_event(
        now.add_timeslots(100),
        CallEvent::Info {
            setup: None,
            timeout: None,
            reset: true,
        },
    )
    .unwrap();
    assert_eq!(call.timers.call_deadline, Some(now.add_timeslots(100 + seconds_to_timeslots(45))));
    call.push_event(
        now,
        CallEvent::Info {
            setup: Some(0),
            timeout: Some(15),
            reset: true,
        },
    )
    .unwrap();
    assert_eq!(call.timers.setup_phase_deadline, None);
    assert_eq!(call.timers.call_deadline, None);
}

#[test]
fn invalid_events_leave_state_and_deadlines_unchanged() {
    let mut call = call();
    let now = TdmaTime::default();
    let before = call.status();
    for event in [
        CallEvent::Demand { priority: 4 },
        CallEvent::Grant {
            own: Some(123),
            grant: 4,
            speaker: Some(456),
        },
        CallEvent::Connect {
            timeout: 16,
            permission_denied: true,
            duplex: Some(true),
        },
        CallEvent::Info {
            setup: Some(3),
            timeout: Some(16),
            reset: true,
        },
        CallEvent::Clear { release: true, cause: 24 },
    ] {
        assert_eq!(call.push_event(now, event), Err(CmceError::InvalidValue));
        assert_eq!(call.status(), before);
        assert!(call.timers.setup_phase_deadline.is_none());
    }
}

#[test]
fn table_capacity_replacement_and_remap_do_not_evict_unrelated_calls() {
    let mut calls = CallTable::<u8, 2>::new();
    assert_eq!(calls.insert(1, 11), Ok(None));
    assert_eq!(calls.insert(2, 22), Ok(None));
    assert_eq!(calls.insert(3, 33), Err(CmceError::Capacity));
    assert_eq!(calls.insert(0x4000, 33), Err(CmceError::InvalidValue));
    assert_eq!(calls.insert(1, 12), Ok(Some(11)));
    assert_eq!(calls.remap(1, 0x4000), Err(CmceError::InvalidValue));
    assert_eq!(calls.get(&1), Some(&12));
    assert_eq!(calls.remap(1, 2), Ok(Some(22)));
    assert_eq!(calls.get(&2), Some(&12));
    assert_eq!(calls.len(), 1);
}

#[test]
fn bounded_pending_stack_preserves_historical_lifo_order() {
    let mut pending = PendingStack::<u8, 2>::new();
    pending.push(1).unwrap();
    pending.push(2).unwrap();
    assert_eq!(pending.push(3), Err(CmceError::Capacity));
    assert_eq!(pending.pop(), Some(2));
    assert_eq!(pending.pop(), Some(1));
    assert_eq!(pending.pop(), None);
}

#[test]
fn floor_wire_and_routing_retain_own_issi_tch_associated_ack_identity() {
    let mut call = call();
    call.last_uplane = Some(call.uplane(true, false));
    let (address, link, stealing) = floor_route(TetraAddress::new(91, SsiType::Gssi), Some(123), 0, call.last_uplane);
    assert_eq!((address, link, stealing), (TetraAddress::new(123, SsiType::Issi), 2, true));
    let bits = encode_uplink(Uplink::TxDemand {
        id: 7,
        priority: 3,
        encrypted: false,
    })
    .unwrap();
    assert_eq!(bits.len(), 24);
    assert_eq!(bits.as_bytes(), &[0x50, 0x00, 0xf8]);
    let bits = encode_uplink(Uplink::TxCeased { id: 7 }).unwrap();
    assert_eq!((bits.len(), bits.as_bytes()), (20, &[0x48, 0x00, 0xe0][..]));
    assert!(
        encode_uplink(Uplink::TxDemand {
            id: 7,
            priority: 4,
            encrypted: false
        })
        .is_err()
    );
    assert!(encode_uplink(Uplink::TxCeased { id: 0x4000 }).is_err());
}

#[test]
fn group_rebroadcast_and_private_uplane_arbitration_preserve_active_floor() {
    let mut private = call();
    private.kind = MsCallKind::Individual;
    private.configure(private.uplane(true, true));
    let mut group = call();
    group.call_identifier = 8;
    group.tx_grant_state = MsTxGrantState::GrantedSelf;
    assert!(!group.group_rebroadcast(false));
    assert_eq!(group.tx_grant_state, MsTxGrantState::GrantedSelf);
    assert!(private.blocks_uplane_of(group.call_identifier, group.kind));
    private.configure(private.uplane(false, false));
    assert!(!private.blocks_uplane_of(group.call_identifier, group.kind));
    assert!(ignore_group_setup(MsCallKind::Group, Some(123), Some(123)));
    assert!(!ignore_group_setup(MsCallKind::Group, None, None));
}

#[test]
fn granted_event_confirms_only_explicit_local_floor_and_preserves_ambiguous_other_grant() {
    let mut call = call();
    call.request_tx(1).unwrap();
    let foreign = call.granted_event(Some(123), 0, Some(456), false).unwrap();
    assert!(!foreign.confirm_request);
    assert_eq!(foreign.grant, 3);
    let ambiguous = call.granted_event(Some(123), 3, Some(123), false).unwrap();
    assert!(ambiguous.ignored);
    assert_eq!(call.tx_grant_state, MsTxGrantState::GrantedOther);
    assert!(call.pending_tx_request);
    let granted = call.granted_event(Some(123), 0, Some(123), false).unwrap();
    assert!(granted.confirm_request);
    assert_eq!(call.tx_grant_state, MsTxGrantState::GrantedSelf);
    assert!(!call.pending_tx_request);
    let snapshot = call.status();
    assert_eq!(
        call.granted_event(Some(123), 0, Some(0x100_0000), false),
        Err(CmceError::InvalidValue)
    );
    assert_eq!(call.status(), snapshot);
}

#[test]
fn gateway_digits_and_dtmf_keep_original_encoding_limits_and_padding() {
    let digits = encode_external_number(" +385\u{2003}*12# ").unwrap();
    assert_eq!(digits.len(), 32);
    assert_eq!(digits.as_bytes(), &[0xc3, 0x85, 0xa1, 0x2b]);
    let mut output = [0; 24];
    assert_eq!(decode_external_number(digits.view(), &mut output), Ok(8));
    assert_eq!(&output[..8], b"+385*12#");
    assert!(encode_external_number(" \u{2003} ").is_err());
    assert!(encode_external_number("12A").is_err());
    assert!(encode_external_number("1234567890123456789012345").is_err());
    let before = output;
    let invalid = tetra_ms_types::BitView::new(&[0xd0], 4).unwrap();
    assert!(decode_external_number(invalid, &mut output).is_err());
    assert_eq!(output, before);
    let dtmf = encode_dtmf(Some(&[1, 2, 10, 11])).unwrap();
    assert_eq!((dtmf.len(), dtmf.as_bytes()), (19, &[0x02, 0x55, 0x60][..]));
    let decoded = decode_dtmf(dtmf.view()).unwrap();
    assert_eq!(decoded.kind, 0);
    assert_eq!(decoded.digits.read(0, 16), Ok(0x12ab));
    assert_eq!(decode_dtmf(encode_dtmf(None).unwrap().view()).unwrap().kind, 1);
    assert_eq!(encode_dtmf(Some(&[9; 254])).unwrap().len(), 1019);
    assert!(encode_dtmf(Some(&[9; 255])).is_err());
    assert!(encode_dtmf(Some(&[])).is_err());
    assert!(encode_dtmf(Some(&[16])).is_err());
}
