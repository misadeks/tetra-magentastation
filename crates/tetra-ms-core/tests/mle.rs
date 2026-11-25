use tetra_ms_core::mle::*;
use tetra_ms_types::{BitView, FragmentBits, MAX_PDU_BITS, PackedBits};

const HOME: Network = Network { mcc: 901, mnc: 9999 };
const CARRIERS: [u32; 2] = [390_000_000, 396_000_000];

fn config() -> MleConfig<'static> {
    MleConfig {
        home_network: HOME,
        allowed_networks: &[],
        subscriber_class: Some(1),
    }
}

fn sync(network: Network) -> PackedBits {
    let mut bits = PackedBits::new();
    bits.push(10, u32::from(network.mcc)).unwrap();
    bits.push(14, u32::from(network.mnc)).unwrap();
    bits.push(2, 2).unwrap();
    bits.push(2, 3).unwrap();
    bits.push(1, 1).unwrap();
    bits
}

fn sysinfo(la: u16, class: u16, services: u16) -> PackedBits {
    let mut bits = PackedBits::new();
    bits.push(14, u32::from(la)).unwrap();
    bits.push(16, u32::from(class)).unwrap();
    bits.push(12, u32::from(services)).unwrap();
    bits
}

fn drain(core: &mut MleCore) -> Vec<MleAction> {
    std::iter::from_fn(|| core.pop_action()).collect()
}

fn event(core: &mut MleCore, event: MleEvent<'_>) -> Vec<MleAction> {
    core.handle(config(), event).unwrap();
    drain(core)
}

#[test]
fn cell_selection_preserves_sysinfo_and_reconfirms_only_la_or_sws_changes() {
    let mut core = MleCore::new();
    let sync = sync(HOME);
    assert_eq!(event(&mut core, MleEvent::Sync(sync.view())), [MleAction::ConfigureNetwork(HOME)]);
    let info = sysinfo(9, 1, 0x840);
    assert!(matches!(
        event(&mut core, MleEvent::Sysinfo(info.view()))[..],
        [MleAction::Activate {
            location_area: 9,
            registration_required: true,
            system_wide_services: true,
            ..
        }]
    ));
    assert!(event(&mut core, MleEvent::Sync(sync.view())).is_empty());
    assert_eq!(core.state().serving_cell.unwrap().location_area, Some(9));
    assert!(event(&mut core, MleEvent::Sysinfo(info.view())).is_empty());
    // Registration/subscriber bitmap changes alone did not historically re-confirm.
    let flags_only = sysinfo(9, 0, 0x40);
    assert!(event(&mut core, MleEvent::Sysinfo(flags_only.view())).is_empty());
    let sws = sysinfo(9, 1, 0);
    assert!(matches!(
        event(&mut core, MleEvent::Sysinfo(sws.view()))[..],
        [MleAction::Activate {
            system_wide_services: false,
            registration_required: false,
            ..
        }]
    ));
    let new_la = sysinfo(10, 1, 0);
    assert_eq!(event(&mut core, MleEvent::Sysinfo(new_la.view())).len(), 1);
}

#[test]
fn scan_snapshots_candidates_wraps_rejects_foreign_and_barred_cells() {
    let mut core = MleCore::new();
    assert_eq!(
        event(&mut core, MleEvent::Dwell { candidates: &CARRIERS }),
        [MleAction::Tune { carrier_hz: CARRIERS[0] }]
    );
    // A changed caller list must not alter an in-progress scan.
    assert_eq!(
        event(&mut core, MleEvent::Dwell { candidates: &[1] }),
        [MleAction::Tune { carrier_hz: CARRIERS[1] }]
    );
    let foreign = sync(Network { mcc: 238, mnc: 6 });
    assert_eq!(
        event(&mut core, MleEvent::Sync(foreign.view())),
        [MleAction::Tune { carrier_hz: CARRIERS[0] }]
    );
    let home = sync(HOME);
    event(&mut core, MleEvent::Sync(home.view()));
    let barred = sysinfo(1, 2, 0x840);
    assert_eq!(
        event(&mut core, MleEvent::Sysinfo(barred.view())),
        [MleAction::Tune { carrier_hz: CARRIERS[1] }]
    );
    assert!(core.state().serving_cell.is_none());
    event(&mut core, MleEvent::Sync(home.view()));
    event(&mut core, MleEvent::Sysinfo(sysinfo(1, 1, 0x840).view()));
    assert!(!core.state().scanning);
    assert!(event(&mut core, MleEvent::Dwell { candidates: &CARRIERS }).is_empty());
}

#[test]
fn manual_camp_and_receive_only_survey_preserve_completion_counts() {
    let mut core = MleCore::new();
    event(&mut core, MleEvent::SelectionMode { manual: true });
    let home = sync(HOME);
    assert!(event(&mut core, MleEvent::Sync(home.view())).is_empty());
    assert!(
        event(
            &mut core,
            MleEvent::Camp {
                carrier_hz: 123,
                register: true,
                candidates: &CARRIERS,
            }
        )
        .is_empty()
    );
    event(
        &mut core,
        MleEvent::Survey {
            start: true,
            candidates: &CARRIERS,
        },
    );
    let foreign = sync(Network { mcc: 238, mnc: 6 });
    event(&mut core, MleEvent::Sync(foreign.view()));
    let actions = event(&mut core, MleEvent::Sysinfo(sysinfo(7, 0, 0x800).view()));
    assert!(matches!(
        &actions[..],
        [
            MleAction::ScanResult(FoundCell {
                mcc: 238,
                location_area: Some(7),
                registration_required: Some(true),
                ..
            }),
            MleAction::Tune { .. }
        ]
    ));
    assert!(core.state().serving_cell.is_none());
    event(&mut core, MleEvent::Sync(home.view()));
    assert_eq!(
        event(
            &mut core,
            MleEvent::Survey {
                start: false,
                candidates: &[]
            }
        ),
        [MleAction::ScanComplete { found: 1, scanned: 1 }]
    );
    event(
        &mut core,
        MleEvent::Camp {
            carrier_hz: CARRIERS[1],
            register: true,
            candidates: &CARRIERS,
        },
    );
    event(&mut core, MleEvent::Sync(home.view()));
    assert_eq!(event(&mut core, MleEvent::Sysinfo(sysinfo(1, 1, 0).view())).len(), 1);
    assert!(!core.state().camp_armed);
}

#[test]
fn survey_partial_timeout_and_monitor_backstop_never_break_service() {
    let mut core = MleCore::new();
    event(
        &mut core,
        MleEvent::Survey {
            start: true,
            candidates: &CARRIERS,
        },
    );
    let sync = sync(HOME);
    event(&mut core, MleEvent::Sync(sync.view()));
    for _ in 0..SURVEY_SYSINFO_SYNC_LIMIT - 1 {
        assert!(event(&mut core, MleEvent::Sync(sync.view())).is_empty());
    }
    let partial = event(&mut core, MleEvent::Sync(sync.view()));
    assert!(matches!(
        &partial[..],
        [
            MleAction::ScanResult(FoundCell {
                location_area: None,
                colour_code: None,
                registration_required: None,
                ..
            }),
            MleAction::Tune { .. }
        ]
    ));
    for _ in 0..SURVEY_MONITOR_TICK_LIMIT - 1 {
        assert!(
            event(
                &mut core,
                MleEvent::Monitor {
                    downlink_available: false,
                    rssi_dbfs: None
                }
            )
            .is_empty()
        );
    }
    assert_eq!(
        event(
            &mut core,
            MleEvent::Monitor {
                downlink_available: false,
                rssi_dbfs: None
            }
        ),
        [MleAction::ScanComplete { found: 1, scanned: 2 }]
    );
    assert!(!core.state().out_of_service);
}

#[test]
fn monitor_rssi_break_reopen_order_and_idempotence() {
    let mut core = MleCore::new();
    event(&mut core, MleEvent::Sync(sync(HOME).view()));
    assert_eq!(
        event(
            &mut core,
            MleEvent::Monitor {
                downlink_available: false,
                rssi_dbfs: Some(-12.5)
            }
        ),
        [
            MleAction::Rssi { rssi_dbfs: Some(-12.5) },
            MleAction::Break {
                layer: UpperLayer::Cmce,
                permitted_services: 0
            },
            MleAction::Break {
                layer: UpperLayer::Mm,
                permitted_services: 0
            },
            MleAction::Rssi { rssi_dbfs: None },
        ]
    );
    assert!(core.state().serving_cell.is_none());
    assert!(
        event(
            &mut core,
            MleEvent::Monitor {
                downlink_available: false,
                rssi_dbfs: None
            }
        )
        .is_empty()
    );
    assert_eq!(
        event(
            &mut core,
            MleEvent::Monitor {
                downlink_available: true,
                rssi_dbfs: None
            }
        ),
        [MleAction::Reopen(UpperLayer::Cmce), MleAction::Reopen(UpperLayer::Mm)]
    );
}

#[test]
fn routing_and_uplink_headers_are_bit_exact_for_all_bit_alignments() {
    for protocol in [Protocol::Mm, Protocol::Cmce, Protocol::Sndcp] {
        for len in 0..32_u16 {
            let mut sdu = PackedBits::new();
            sdu.push(3, protocol as u32).unwrap();
            sdu.push(len as u8, if len == 0 { 0 } else { u32::MAX >> (32 - len) }).unwrap();
            let expected = PackedBits::from_view(sdu.view().window(3, len).unwrap()).unwrap();
            let mut core = MleCore::new();
            assert_eq!(
                event(&mut core, MleEvent::Downlink(sdu.view())),
                [MleAction::Deliver {
                    protocol,
                    parameters: DeliveryParameters::default(),
                    sdu: FragmentBits::from_view(expected.view()).unwrap(),
                }]
            );
            if protocol == Protocol::Sndcp {
                continue;
            }
            let source = if protocol == Protocol::Mm {
                UplinkSource::Mm {
                    priority: Some(7),
                    emergency: true,
                }
            } else {
                UplinkSource::Cmce { stealing_permission: true }
            };
            let out = event(
                &mut core,
                MleEvent::Uplink {
                    source,
                    sdu: expected.view(),
                    endpoint_id: 19,
                    link_id: 3,
                },
            );
            let [MleAction::Transmit { pdu, parameters }] = &out[..] else {
                panic!("one transmit action");
            };
            assert_eq!(pdu, &sdu);
            assert_eq!(
                (parameters.endpoint_id, parameters.link_id),
                if protocol == Protocol::Mm { (0, 0) } else { (19, 3) }
            );
            assert_eq!(
                (parameters.priority, parameters.emergency, parameters.stealing_permission),
                if protocol == Protocol::Mm {
                    (Some(7), true, false)
                } else {
                    (None, false, true)
                }
            );
        }
    }
}

#[test]
fn truncated_wire_capacity_and_queue_errors_are_atomic() {
    let mut core = MleCore::new();
    for len in 0..29 {
        let bits = sync(HOME);
        assert_eq!(
            core.handle(config(), MleEvent::Sync(bits.view().window(0, len).unwrap())),
            Err(MleError::TruncatedPdu)
        );
        assert_eq!(core.state(), MleState::default());
    }
    for len in 0..42 {
        let bits = sysinfo(1, 1, 0x840);
        assert_eq!(
            core.handle(config(), MleEvent::Sysinfo(bits.view().window(0, len).unwrap())),
            Err(MleError::TruncatedPdu)
        );
    }
    let oversized = [1; CANDIDATE_CAPACITY + 1];
    assert_eq!(
        core.handle(
            config(),
            MleEvent::Survey {
                start: true,
                candidates: &oversized
            }
        ),
        Err(MleError::CapacityExceeded)
    );
    assert_eq!(core.state(), MleState::default());
    let full = [0; MAX_PDU_BITS as usize / 8];
    assert_eq!(
        core.handle(
            config(),
            MleEvent::Uplink {
                source: UplinkSource::Mm {
                    priority: None,
                    emergency: false
                },
                sdu: BitView::new(&full, MAX_PDU_BITS).unwrap(),
                endpoint_id: 0,
                link_id: 0,
            }
        ),
        Err(MleError::CapacityExceeded)
    );
    assert_eq!(core.pending_actions(), 0);
    for _ in 0..ACTION_CAPACITY {
        core.handle(
            config(),
            MleEvent::UPlane {
                switch_u_plane: true,
                tx_grant: false,
            },
        )
        .unwrap();
    }
    assert_eq!(
        core.handle(
            config(),
            MleEvent::Survey {
                start: true,
                candidates: &CARRIERS
            }
        ),
        Err(MleError::QueueFull)
    );
    assert_eq!(core.state(), MleState::default());
    drain(&mut core);
    assert_eq!(
        event(
            &mut core,
            MleEvent::Survey {
                start: true,
                candidates: &CARRIERS
            }
        )
        .len(),
        1
    );
    assert_eq!(
        core.handle(
            config(),
            MleEvent::Identities {
                issi: 1,
                attached_gssis: &[1; GROUP_CAPACITY + 1]
            }
        ),
        Err(MleError::CapacityExceeded)
    );
}

#[test]
fn identities_preserve_home_network_and_all_group_order() {
    let mut core = MleCore::new();
    let groups = [91, 220];
    let out = event(
        &mut core,
        MleEvent::Identities {
            issi: 1,
            attached_gssis: &groups,
        },
    );
    let [
        MleAction::ConfigureIdentities {
            network,
            issi,
            groups: out,
            group_count,
        },
    ] = &out[..]
    else {
        panic!("one configure action");
    };
    assert_eq!((*network, *issi, *group_count), (HOME, 1, 2));
    assert_eq!(&out[..2], &groups);
}

#[test]
fn large_reassembled_downlink_is_delivered_without_packed_pdu_truncation() {
    let mut core = MleCore::new();
    let mut sdu = FragmentBits::new();
    sdu.push(3, Protocol::Cmce as u32).unwrap();
    for _ in 0..127 {
        sdu.push(32, 0x1234_5678).unwrap();
    }
    sdu.push(29, 0x123_4567).unwrap();
    assert_eq!(sdu.len(), 4096);
    let out = event(&mut core, MleEvent::Downlink(sdu.view()));
    let [
        MleAction::Deliver {
            protocol: Protocol::Cmce,
            sdu: payload,
            ..
        },
    ] = &out[..]
    else {
        panic!("one CMCE delivery");
    };
    assert_eq!(payload.len(), 4093);
    assert_eq!(payload.view().read(0, 32).unwrap(), 0x1234_5678);
    assert_eq!(payload.view().read(4064, 29).unwrap(), 0x123_4567);
}

#[test]
fn reserved_internal_and_management_pdus_are_explicit_and_never_routed() {
    let mut core = MleCore::new();
    for raw in 0..8 {
        if [1, 2, 4].contains(&raw) {
            continue;
        }
        let mut bits = PackedBits::new();
        bits.push(3, raw).unwrap();
        bits.push(3, 7).unwrap();
        let expected = match raw {
            5 => MleError::UnsupportedMlePdu(7),
            6 => MleError::UnsupportedManagement,
            _ => MleError::InvalidProtocol(raw as u8),
        };
        assert_eq!(core.handle(config(), MleEvent::Downlink(bits.view())), Err(expected));
        assert_eq!(core.pending_actions(), 0);
    }
    for len in 0..3 {
        let bits = BitView::new(&[0], len).unwrap();
        assert_eq!(core.handle(config(), MleEvent::Downlink(bits)), Err(MleError::TruncatedPdu));
    }
}

#[test]
fn failed_monitor_reservation_does_not_consume_survey_heartbeat() {
    let mut core = MleCore::new();
    event(
        &mut core,
        MleEvent::Survey {
            start: true,
            candidates: &CARRIERS,
        },
    );
    for _ in 0..ACTION_CAPACITY - 3 {
        core.handle(
            config(),
            MleEvent::UPlane {
                switch_u_plane: true,
                tx_grant: true,
            },
        )
        .unwrap();
    }
    let before = core.state();
    let monitor = MleEvent::Monitor {
        downlink_available: false,
        rssi_dbfs: Some(-1.0),
    };
    assert_eq!(core.handle(config(), monitor), Err(MleError::QueueFull));
    assert_eq!(core.state(), before);
    drain(&mut core);
    assert_eq!(event(&mut core, monitor), [MleAction::Rssi { rssi_dbfs: Some(-1.0) }]);
    assert_eq!(core.state().survey_monitor_ticks, 1);
}

#[test]
fn caller_policy_admits_additional_networks_and_class_sixteen() {
    let foreign = Network { mcc: 238, mnc: 6 };
    let policy = MleConfig {
        allowed_networks: &[foreign],
        subscriber_class: Some(16),
        ..config()
    };
    let mut core = MleCore::new();
    let sync = sync(foreign);
    core.handle(policy, MleEvent::Sync(sync.view())).unwrap();
    assert_eq!(drain(&mut core), [MleAction::ConfigureNetwork(foreign)]);
    let info = sysinfo(4, 0x8000, 0x840);
    core.handle(policy, MleEvent::Sysinfo(info.view())).unwrap();
    assert!(matches!(drain(&mut core)[..], [MleAction::Activate { .. }]));
}
