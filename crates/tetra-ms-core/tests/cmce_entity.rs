use tetra_ms_core::cmce::*;
use tetra_ms_types::{BitView, PackedBits, SsiType, TdmaTime, TetraAddress};

fn route(ssi: u32) -> CallRoute {
    CallRoute {
        main_address: TetraAddress::new(ssi, SsiType::Gssi),
        handle: 1,
        endpoint_id: 2,
        link_id: 0,
    }
}
fn drain(core: &mut CmceCore) -> Vec<CmceAction> {
    let mut actions = Vec::new();
    while let Some(action) = core.pop_action() {
        actions.push(action);
    }
    actions
}
fn push(bits: &mut PackedBits, width: u8, value: u32) {
    bits.push(width, value).unwrap();
}
fn setup(id: u16, communication: u32, hook: bool, grant: u32, caller: Option<u32>) -> PackedBits {
    let mut bits = PackedBits::new();
    for (width, value) in [
        (5, 7),
        (14, u32::from(id)),
        (4, 1),
        (1, u32::from(hook)),
        (1, 0),
        (3, 0),
        (1, 0),
        (2, communication),
        (2, 0),
        (2, grant),
        (1, 0),
        (4, 0),
        (1, u32::from(caller.is_some())),
    ] {
        push(&mut bits, width, value);
    }
    if let Some(caller) = caller {
        push(&mut bits, 1, 0); // notification absent
        push(&mut bits, 1, 0); // temporary address absent
        push(&mut bits, 1, 1);
        push(&mut bits, 2, 1);
        push(&mut bits, 24, caller);
        push(&mut bits, 1, 0);
    }
    bits
}
fn receive(core: &mut CmceCore, bits: &PackedBits, address: u32) -> Result<(), CmceError> {
    core.push_event(
        TdmaTime::default(),
        CmceEvent::Downlink {
            sdu: bits.view(),
            route: route(address),
        },
    )
}
fn floor(id: u16, kind: u32, speaker: Option<u32>) -> PackedBits {
    let mut bits = PackedBits::new();
    for (width, value) in [
        (5, 11),
        (14, u32::from(id)),
        (2, kind),
        (1, 0),
        (1, 0),
        (1, 0),
        (1, u32::from(speaker.is_some())),
    ] {
        push(&mut bits, width, value);
    }
    if let Some(speaker) = speaker {
        push(&mut bits, 1, 0);
        push(&mut bits, 1, 1);
        push(&mut bits, 2, 1);
        push(&mut bits, 24, speaker);
        push(&mut bits, 1, 0);
    }
    bits
}

#[test]
fn private_answer_hook_sequence_and_connect_ack_are_entity_owned() {
    let mut core = CmceCore::new();
    receive(&mut core, &setup(7, 0, true, 1, Some(456)), 123).unwrap();
    assert_eq!(core.calls.get(&7).unwrap().state, MsCcState::MtCallSetup);
    drain(&mut core);
    core.calls.get_mut(&7).unwrap().timers.setup_timeout = Some(3);
    core.push_event(
        TdmaTime::default(),
        CmceEvent::Answer {
            id: 7,
            basic: None,
            duplex: true,
            complete: false,
        },
    )
    .unwrap();
    let actions = drain(&mut core);
    assert!(matches!(&actions[0], CmceAction::Uplink { sdu, .. } if sdu.view().read(0,5) == Ok(0)));
    assert!(core.calls.get(&7).unwrap().timers.setup_phase_deadline.is_none());
    core.push_event(
        TdmaTime::default(),
        CmceEvent::Answer {
            id: 7,
            basic: None,
            duplex: true,
            complete: true,
        },
    )
    .unwrap();
    assert!(core.calls.get(&7).unwrap().timers.setup_phase_deadline.is_some());
    drain(&mut core);
    let mut ack = PackedBits::new();
    for (width, value) in [(5, 3), (14, 7), (4, 1), (2, 0), (1, 0), (1, 0)] {
        push(&mut ack, width, value);
    }
    receive(&mut core, &ack, 123).unwrap();
    let call = core.calls.get(&7).unwrap();
    assert_eq!(call.state, MsCcState::CallActive);
    assert!(call.timers.setup_phase_deadline.is_none());
    assert_eq!(call.tx_grant_state, MsTxGrantState::GrantedSelf);
}

#[test]
fn late_entry_repeats_do_not_recreate_or_revoke_the_local_floor() {
    let mut core = CmceCore::new();
    core.push_event(TdmaTime::default(), CmceEvent::Identity(Some(123))).unwrap();
    receive(&mut core, &setup(7, 1, false, 0, Some(456)), 91).unwrap();
    let actions = drain(&mut core);
    assert_eq!(
        actions
            .iter()
            .filter(|a| matches!(a,CmceAction::Notify(n) if n.kind==NoticeKind::Setup))
            .count(),
        1
    );
    assert_eq!(core.calls.get(&7).unwrap().tx_grant_state, MsTxGrantState::GrantedOther);
    receive(&mut core, &setup(7, 1, false, 0, Some(456)), 91).unwrap();
    assert!(!drain(&mut core).iter().any(|a| matches!(a, CmceAction::Notify(_))));
    core.push_event(TdmaTime::default(), CmceEvent::Demand { id: 7, priority: 1 })
        .unwrap();
    drain(&mut core);
    receive(&mut core, &floor(7, 0, Some(123)), 123).unwrap();
    assert!(
        drain(&mut core)
            .iter()
            .any(|a| matches!(a,CmceAction::Notify(n) if n.kind==NoticeKind::TxConfirm))
    );
    receive(&mut core, &setup(7, 1, false, 3, Some(456)), 91).unwrap();
    assert!(drain(&mut core).is_empty());
    assert_eq!(core.calls.get(&7).unwrap().tx_grant_state, MsTxGrantState::GrantedSelf);
    receive(&mut core, &setup(7, 1, false, 3, Some(123)), 91).unwrap();
    assert!(drain(&mut core).is_empty());
    assert_eq!(core.calls.len(), 1);
}

#[test]
fn private_call_arbitration_and_marker_demultiplexing_are_portable() {
    let mut core = CmceCore::new();
    receive(&mut core, &setup(1, 0, false, 1, Some(456)), 123).unwrap();
    drain(&mut core);
    core.configure(
        1,
        MsUPlaneState {
            switch_u_plane: true,
            tx_grant: false,
            simplex_duplex: true,
        },
    )
    .unwrap();
    drain(&mut core);
    receive(&mut core, &setup(2, 1, false, 0, Some(456)), 91).unwrap();
    assert!(core.calls.get(&2).unwrap().last_uplane.is_none());
    assert_eq!(core.receive_traffic(Some(91)).unwrap().id, 1); // historical sole-call fallback
    drain(&mut core);
    core.calls.remove(&1);
    receive(&mut core, &setup(2, 1, false, 0, Some(456)), 91).unwrap();
    assert_eq!(core.receive_traffic(Some(91)).unwrap().id, 2);
    drain(&mut core);
    receive(&mut core, &setup(3, 1, false, 0, Some(789)), 92).unwrap();
    drain(&mut core);
    assert_eq!(core.receive_traffic(Some(92)).unwrap().id, 3);
    assert!(core.receive_traffic(None).is_none());
    assert!(core.receive_traffic(Some(999)).is_none());
}

#[test]
fn proceeding_connect_and_info_rename_retain_lifo_and_timer_behavior() {
    let mut core = CmceCore::new();
    let now = TdmaTime::default();
    core.push_event(now, CmceEvent::Identity(Some(123))).unwrap();
    for communication in [0, 1] {
        let basic = BasicService {
            communication,
            ..DEFAULT_BASIC
        };
        core.push_event(
            now,
            CmceEvent::Originate {
                called: route(91).main_address,
                basic,
                duplex: false,
                request_tx: true,
                hook: false,
                external: None,
            },
        )
        .unwrap();
        drain(&mut core);
    }
    let mut proceeding = PackedBits::new();
    for (w, v) in [(5, 1), (14, 7), (3, 3), (1, 0), (1, 0), (1, 0)] {
        push(&mut proceeding, w, v);
    }
    receive(&mut core, &proceeding, 123).unwrap();
    assert_eq!(core.calls.get(&7).unwrap().kind, MsCallKind::Group);
    assert_eq!(core.pending_originations.len(), 1);
    drain(&mut core);
    let mut info = PackedBits::new();
    for (w, v) in [
        (5, 5),
        (14, 7),
        (1, 1),
        (1, 0),
        (1, 1),
        (1, 1),
        (14, 8),
        (1, 1),
        (4, 2),
        (1, 0),
        (1, 0),
        (1, 0),
        (1, 0),
        (1, 0),
        (1, 0),
        (1, 0),
        (1, 0),
        (1, 0),
    ] {
        push(&mut info, w, v);
    }
    receive(&mut core, &info, 123).unwrap();
    assert!(core.calls.get(&7).is_none());
    assert_eq!(core.calls.get(&8).unwrap().call_identifier, 8);
    assert_eq!(core.calls.get(&8).unwrap().timers.call_timeout, 2);
    assert!(matches!(drain(&mut core).last(),Some(CmceAction::Notify(n)) if n.id==8 && n.kind==NoticeKind::Info));
}

#[test]
fn malformed_input_full_tables_and_backpressure_leave_state_intact() {
    let mut core = CmceCore::new();
    let bits = setup(7, 1, false, 0, Some(456));
    for len in 0..bits.len() {
        assert!(receive(&mut core, &PackedBits::from_view(bits.view().window(0, len).unwrap()).unwrap(), 91).is_err());
        assert_eq!(core.status().calls, 0);
        assert_eq!(core.status().queued_actions, 0);
    }
    for id in 0..MAX_CALLS {
        receive(&mut core, &setup(id as u16, 1, false, 1, None), 91).unwrap();
        drain(&mut core);
    }
    assert_eq!(receive(&mut core, &setup(99, 1, false, 1, None), 91), Err(CmceError::Capacity));
    assert_eq!(core.calls.len(), MAX_CALLS);
    for _ in 0..MAX_ACTIONS / 2 {
        core.push_event(TdmaTime::default(), CmceEvent::Release { id: 0, cause: 1 })
            .unwrap();
    }
    let before = core.calls.get(&1).unwrap().status();
    assert_eq!(
        core.push_event(TdmaTime::default(), CmceEvent::Disconnect { id: 1, cause: 1 }),
        Err(CmceError::OutputFull)
    );
    assert_eq!(core.calls.get(&1).unwrap().status(), before);
}

#[test]
fn timer_disconnect_and_restoration_lifecycle_emit_original_wire_and_actions() {
    let mut core = CmceCore::new();
    receive(&mut core, &setup(7, 1, false, 0, Some(456)), 91).unwrap();
    drain(&mut core);
    core.push_event(TdmaTime::default(), CmceEvent::Break).unwrap();
    assert_eq!(core.calls.get(&7).unwrap().state, MsCcState::Restore);
    drain(&mut core);
    core.push_event(TdmaTime::default(), CmceEvent::Restore { id: 7 }).unwrap();
    assert!(matches!(drain(&mut core).first(),Some(CmceAction::Uplink{sdu,..}) if sdu.view().read(0,5)==Ok(14)));
    let deadline = core.calls.get(&7).unwrap().timers.call_deadline.unwrap();
    core.push_event(deadline, CmceEvent::Tick).unwrap();
    assert!(matches!(drain(&mut core).first(),Some(CmceAction::Uplink{sdu,..}) if sdu.view().read(19,5)==Ok(13)));
    core.push_event(deadline, CmceEvent::Reopen).unwrap();
    assert!(core.calls.is_empty());
}

#[test]
fn unsupported_downlink_is_explicit_and_never_treated_as_a_call() {
    let mut core = CmceCore::new();
    for kind in [8, 15, 16, 31] {
        let bytes = [kind << 3];
        assert_eq!(
            core.push_event(
                TdmaTime::default(),
                CmceEvent::Downlink {
                    sdu: BitView::new(&bytes, 5).unwrap(),
                    route: route(91)
                }
            ),
            Err(CmceError::Unsupported)
        );
    }
    assert_eq!(core.status().calls, 0);
}

#[test]
fn in_place_initialization_and_reset_leave_no_stale_calls_or_actions() {
    let mut storage = core::mem::MaybeUninit::<CmceCore>::uninit();
    // SAFETY: exclusively owned aligned uninitialized storage for the whole core.
    unsafe {
        CmceCore::init_at(storage.as_mut_ptr());
    }
    // SAFETY: init_at initialized every field.
    let mut core = unsafe { storage.assume_init() };
    core.push_event(TdmaTime::default(), CmceEvent::Identity(Some(123))).unwrap();
    core.push_event(
        TdmaTime::default(),
        CmceEvent::Originate {
            called: route(91).main_address,
            basic: DEFAULT_BASIC,
            duplex: false,
            request_tx: true,
            hook: false,
            external: None,
        },
    )
    .unwrap();
    assert_eq!(core.status().originations, 1);
    core.reset();
    assert_eq!(
        core.status(),
        CmceStatus {
            own_issi: None,
            calls: 0,
            originations: 0,
            queued_actions: 0
        }
    );
    assert!(core.pop_action().is_none());
    receive(&mut core, &setup(7, 1, false, 3, Some(456)), 91).unwrap();
    assert_eq!(core.status().calls, 1);
}
