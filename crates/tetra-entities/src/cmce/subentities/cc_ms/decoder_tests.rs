use super::*;
use tetra_ms_core::cmce::downlink;
use tetra_ms_types::{BitView, PackedBits};
use tetra_pdus::cmce::pdus::{
    d_alert::DAlert, d_call_proceeding::DCallProceeding, d_call_restore::DCallRestore, d_connect::DConnect, d_disconnect::DDisconnect,
    d_release::DRelease, d_tx_ceased::DTxCeased, d_tx_continue::DTxContinue, d_tx_interrupt::DTxInterrupt, d_tx_wait::DTxWait,
};

fn buffer(bits: BitView<'_>) -> BitBuffer {
    let mut buffer = BitBuffer::new_autoexpand(usize::from(bits.len()));
    for offset in 0..bits.len() {
        buffer.write_bit(bits.read(offset, 1).unwrap() as u8);
    }
    buffer.seek(0);
    buffer
}
fn historical_accepts(kind: u8, bits: BitView<'_>) -> bool {
    let mut bits = buffer(bits);
    macro_rules! parse {
        ($ty:ty) => {
            <$ty>::from_bitbuf(&mut bits)
                .map(|pdu| assert_eq!(pdu.call_identifier, 0x1234))
                .is_ok()
        };
    }
    match kind {
        0 => parse!(DAlert),
        1 => parse!(DCallProceeding),
        2 => parse!(DConnect),
        3 => parse!(DConnectAcknowledge),
        4 => parse!(DDisconnect),
        5 => parse!(DInfo),
        6 => parse!(DRelease),
        7 => parse!(DSetup),
        9 => parse!(DTxCeased),
        10 => parse!(DTxContinue),
        11 => parse!(DTxGranted),
        12 => parse!(DTxWait),
        13 => parse!(DTxInterrupt),
        14 => parse!(DCallRestore),
        _ => unreachable!(),
    }
}
fn minimum(kind: u8) -> PackedBits {
    let mut bits = PackedBits::new();
    bits.push(5, u32::from(kind)).unwrap();
    bits.push(14, 0x1234).unwrap();
    let fields: &[(u8, u32)] = match kind {
        0 => &[(3, 3), (1, 1), (1, 0), (1, 1)],
        1 => &[(3, 3), (1, 1), (1, 0)],
        2 => &[(4, 1), (1, 1), (1, 1), (2, 3), (1, 0), (1, 1)],
        3 => &[(4, 1), (2, 0), (1, 0)],
        4 | 6 => &[(5, 13)],
        5 => &[(1, 1), (1, 1)],
        7 => &[(4, 1), (1, 0), (1, 0), (8, 4), (2, 3), (1, 0), (4, 1)],
        9 | 12 => &[(1, 0)],
        10 => &[(1, 1), (1, 0)],
        11 | 13 => &[(2, 3), (1, 0), (1, 0), (1, 0)],
        14 => &[(2, 3), (1, 0), (1, 1)],
        _ => unreachable!(),
    };
    for &(width, value) in fields {
        bits.push(width, value).unwrap();
    }
    bits.push(1, 0).unwrap();
    bits
}
#[test]
fn all_fourteen_downlink_layouts_match_historical_acceptance_at_every_truncation() {
    for kind in [0, 1, 2, 3, 4, 5, 6, 7, 9, 10, 11, 12, 13, 14] {
        let bits = minimum(kind);
        assert!(historical_accepts(kind, bits.view()), "historical kind {kind}");
        let decoded = downlink::decode(bits.view()).unwrap();
        assert_eq!((decoded.kind, decoded.id), (kind, 0x1234));
        for len in 0..bits.len() {
            let prefix = bits.view().window(0, len).unwrap();
            assert!(!historical_accepts(kind, prefix), "historical accepted kind {kind} prefix {len}");
            assert!(downlink::decode(prefix).is_err(), "portable accepted kind {kind} prefix {len}");
        }
    }
}
#[test]
fn info_optional_fields_and_dtmf_offsets_match_the_original_codec() {
    let pdu = DInfo {
        call_identifier: 0x1234,
        reset_call_time_out_timer_t310_: true,
        poll_request: true,
        new_call_identifier: Some(0x2345),
        call_time_out: Some(2),
        call_time_out_set_up_phase_t301_t302_: Some(3),
        call_ownership: Some(1),
        modify: Some(0x123),
        call_status: Some(4),
        temporary_address: Some(0x123456),
        notification_indicator: Some(17),
        poll_response_percentage: Some(25),
        poll_response_number: Some(32),
        dtmf: dtmf::encode_tone_start_digits("12*#"),
        facility: None,
        poll_response_addresses: None,
        proprietary: None,
    };
    let mut bits = BitBuffer::new_autoexpand(256);
    pdu.to_bitbuf(&mut bits).unwrap();
    let view = bits.as_bit_view().unwrap();
    let p = downlink::decode(view).unwrap();
    assert_eq!((p.new_id, p.timeout, p.setup_timeout), (Some(0x2345), Some(2), Some(3)));
    assert_eq!((p.ownership, p.call_status, p.notification), (Some(true), Some(4), Some(17)));
    assert_eq!((p.poll_percentage, p.poll_number), (Some(25), Some(32)));
    let decoded = tetra_ms_core::cmce::decode_dtmf(p.dtmf.unwrap().view(view).unwrap()).unwrap();
    assert_eq!(decoded.digits.read(0, 16), Ok(0x12ab));
}
#[test]
fn tx_interrupt_retains_independently_present_address_fields() {
    let pdu = DTxInterrupt {
        call_identifier: 0x1234,
        transmission_grant: 3,
        transmission_request_permission: false,
        encryption_control: false,
        reserved: false,
        notification_indicator: Some(17),
        transmitting_party_type_identifier: Some(1),
        transmitting_party_address_ssi: Some(456),
        transmitting_party_extension: Some(789),
        external_subscriber_number: None,
        facility: None,
        dm_ms_address: None,
        proprietary: None,
    };
    let mut bits = BitBuffer::new_autoexpand(256);
    pdu.to_bitbuf(&mut bits).unwrap();
    let p = downlink::decode(bits.as_bit_view().unwrap()).unwrap();
    assert_eq!((p.speaker, p.extension, p.notification), (Some(456), Some(789), Some(17)));
}

#[test]
fn desktop_audio_sidecar_follows_portable_rekey_and_cleanup_actions() {
    let mut cc = CcMsSubentity::new(None);
    let mut queue = MessageQueue::new();
    cc.own_issi = Some(123);
    let setup = minimum(7);
    assert!(cc.dispatch(
        &mut queue,
        tetra_ms_core::cmce::CmceEvent::Downlink {
            sdu: setup.view(),
            route: CallRoute {
                main_address: TetraAddress::new(91, SsiType::Gssi),
                handle: 1,
                endpoint_id: 2,
                link_id: 0,
            }
        }
    ));
    cc.apply_transmission_grant(&mut queue, 0x1234, TransmissionGrant::Granted, None);
    cc.push_uplink_speech(0x1234, 274, &[1; 274]);
    assert_eq!(cc.audio.get(&0x1234).unwrap().frames.len(), 1);
    let pdu = DInfo {
        call_identifier: 0x1234,
        reset_call_time_out_timer_t310_: false,
        poll_request: false,
        new_call_identifier: Some(0x2345),
        call_time_out: None,
        call_time_out_set_up_phase_t301_t302_: None,
        call_ownership: None,
        modify: None,
        call_status: None,
        temporary_address: None,
        notification_indicator: None,
        poll_response_percentage: None,
        poll_response_number: None,
        dtmf: None,
        facility: None,
        poll_response_addresses: None,
        proprietary: None,
    };
    cc.rx_d_info(
        &mut queue,
        pdu,
        CallRoute {
            main_address: TetraAddress::new(91, SsiType::Gssi),
            handle: 1,
            endpoint_id: 2,
            link_id: 0,
        },
    );
    assert!(!cc.audio.contains_key(&0x1234));
    assert_eq!(cc.audio.get(&0x2345).unwrap().frames.len(), 1);
    let mut replacement = minimum(7);
    replacement.view_mut().write(5, 14, 0x2345).unwrap();
    replacement.view_mut().write(29, 2, 0).unwrap();
    assert!(cc.dispatch(
        &mut queue,
        tetra_ms_core::cmce::CmceEvent::Downlink {
            sdu: replacement.view(),
            route: CallRoute {
                main_address: TetraAddress::new(123, SsiType::Issi),
                handle: 1,
                endpoint_id: 2,
                link_id: 0,
            },
        }
    ));
    assert!(cc.audio.is_empty(), "a new call replacing the same ID discards old audio");
    cc.handle_reopen(&mut queue);
    assert!(cc.audio.is_empty());
    assert_eq!(cc.call_count(), 0);
}

#[test]
fn denied_wire_permission_is_negative_telemetry_and_gates_demand() {
    use crate::net_telemetry::channel::telemetry_channel;
    use tetra_ms_core::cmce::CmceEvent;
    for kind in [7, 2, 3, 9, 10, 11, 12, 13] {
        for denied in [false, true] {
            let (sink, rx) = telemetry_channel();
            let mut cc = CcMsSubentity::new(Some(sink));
            cc.own_issi = Some(123);
            let mut queue = MessageQueue::new();
            let route = CallRoute {
                main_address: TetraAddress::new(91, SsiType::Gssi),
                handle: 0,
                endpoint_id: 0,
                link_id: 0,
            };
            if kind != 7 {
                assert!(cc.dispatch(
                    &mut queue,
                    CmceEvent::Downlink {
                        sdu: minimum(7).view(),
                        route
                    }
                ));
            }
            while rx.try_recv().is_some() {}
            queue = MessageQueue::new();
            let mut bits = minimum(kind);
            let permission_offset = match kind {
                7 => 35,
                2 => 27,
                3 => 25,
                9 | 12 => 19,
                10 => 20,
                11 | 13 => 21,
                _ => unreachable!(),
            };
            bits.view_mut().write(permission_offset, 1, u32::from(denied)).unwrap();
            assert!(cc.dispatch(&mut queue, CmceEvent::Downlink { sdu: bits.view(), route }));
            let permission = std::iter::from_fn(|| rx.try_recv())
                .find_map(|event| match event {
                    TelemetryEvent::TnccSetupIndication { indication, .. } => Some(indication.transmission_request_permission),
                    TelemetryEvent::TnccSetupConfirm { confirm, .. } => Some(confirm.transmission_request_permission),
                    TelemetryEvent::TnccCompleteConfirm { confirm, .. } => Some(confirm.transmission_request_permission),
                    TelemetryEvent::TnccTxConfirm { confirm, .. } => Some(confirm.transmit_request_permission),
                    TelemetryEvent::TnccTxIndication { indication, .. } => Some(indication.transmit_request_permission),
                    _ => None,
                })
                .expect("TNCC permission indication");
            assert_eq!(
                permission,
                if denied {
                    tncc::TransmissionRequestPermission::NotAllowedToRequestForTransmission
                } else {
                    tncc::TransmissionRequestPermission::AllowedToRequestForTransmission
                }
            );
            queue = MessageQueue::new();
            let before = cc.call(0x1234).unwrap().status();
            assert_eq!(cc.dispatch(&mut queue, CmceEvent::Demand { id: 0x1234, priority: 0 }), !denied);
            if denied {
                assert_eq!(cc.call(0x1234).unwrap().status(), before);
                assert!(queue.iter().next().is_none());
            } else {
                assert!(cc.call(0x1234).unwrap().pending_tx_request);
                assert_eq!(queue.iter().count(), 1);
            }
        }
    }
}

#[test]
fn floor_loss_flushes_host_and_mac_audio_before_regrant() {
    use tetra_ms_core::cmce::CmceEvent;
    use tetra_ms_core::mac::Mac;

    fn bridge(queue: &mut MessageQueue, mac: &mut Mac<u32>) {
        while let Some(message) = queue.pop_front() {
            match message.msg {
                SapMsgInner::LcmcMleConfigureReq(config) => mac.configure_uplane(config.switch_u_plane, config.tx_grant),
                SapMsgInner::TmdCircuitDataReq(frame) => {
                    mac.queue_audio(BitView::new(&frame.data, 274).unwrap()).unwrap();
                }
                _ => {}
            }
        }
    }
    for loss in 0..8 {
        let mut cc = CcMsSubentity::new(None);
        cc.own_issi = Some(123);
        let mut queue = MessageQueue::new();
        let route = CallRoute {
            main_address: TetraAddress::new(91, SsiType::Gssi),
            handle: 0,
            endpoint_id: 0,
            link_id: 0,
        };
        assert!(cc.dispatch(
            &mut queue,
            CmceEvent::Downlink {
                sdu: minimum(7).view(),
                route
            }
        ));
        cc.apply_transmission_grant(&mut queue, 0x1234, TransmissionGrant::Granted, None);
        let mut mac = Mac::<u32> {
            scrambling_code: Some(123),
            assigned_traffic_slots: [true; 4],
            ..Mac::default()
        };
        bridge(&mut queue, &mut mac);
        cc.push_uplink_speech(0x1234, 274, &[1; 274]);
        cc.push_uplink_speech(0x1234, 274, &[1; 274]);
        cc.drive_uplink_source(&mut queue);
        bridge(&mut queue, &mut mac);
        assert_eq!(mac.uplink_audio.len(), 1);
        assert_eq!(cc.audio.get(&0x1234).unwrap().frames.len(), 1);
        let mut pdu = minimum(if loss == 4 { 13 } else { 11 });
        if loss >= 5 {
            pdu.view_mut().write(19, 2, if loss == 5 { 3 } else { loss - 5 }).unwrap();
        }
        let event = match loss {
            0 => CmceEvent::Cease { id: 0x1234 },
            1 => CmceEvent::Break,
            2 => CmceEvent::Disconnect { id: 0x1234, cause: 0 },
            3 => CmceEvent::Release { id: 0x1234, cause: 0 },
            _ => CmceEvent::Downlink { sdu: pdu.view(), route },
        };
        assert!(cc.dispatch(&mut queue, event));
        bridge(&mut queue, &mut mac);
        assert!(cc.audio.is_empty(), "host flush, loss {loss}");
        assert!(mac.uplink_audio.is_empty(), "MAC flush, loss {loss}");
        assert!(
            mac.queue_audio(BitView::new(&[0xff; 35], 274).unwrap()).unwrap(),
            "late speech dropped"
        );
        cc.apply_transmission_grant(&mut queue, 0x1234, TransmissionGrant::Granted, None);
        bridge(&mut queue, &mut mac);
        cc.drive_uplink_source(&mut queue);
        bridge(&mut queue, &mut mac);
        let burst = mac.traffic().expect("granted slot");
        assert_eq!(burst.speech.view().read(0, 32), Ok(0), "no old speech after loss {loss}");
        cc.push_uplink_speech(0x1234, 274, &[1; 274]);
        cc.drive_uplink_source(&mut queue);
        bridge(&mut queue, &mut mac);
        assert_eq!(mac.traffic().unwrap().speech.view().read(0, 32), Ok(u32::MAX));
    }
}

#[test]
fn mac_fragment_fill_matches_independent_uplink_pdu_decoders() {
    use tetra_ms_core::mac::wire;
    use tetra_pdus::umac::pdus::{mac_end_ul::MacEndUl, mac_frag_ul::MacFragUl};
    let bytes = [0xa5; 204];
    for len in 311..=1630 {
        let source = BitView::new(&bytes, len).unwrap();
        let (_, remainder, kind) = wire::access_fragment(123, source).unwrap();
        let wire::FragEndKind::MacFragUl { total_slots } = kind else {
            panic!("full slot plan");
        };
        let plan = wire::FragmentBlocks::build(remainder.view(), total_slots).unwrap();
        let mut recovered = tetra_ms_types::FragmentBits::new();
        for (index, block) in plan.as_slice().iter().enumerate() {
            let mut bits = buffer(block.view());
            let last = index + 1 == usize::from(total_slots);
            let (fill, end) = if last {
                let pdu = MacEndUl::from_bitbuf(&mut bits).unwrap();
                (pdu.fill_bits, usize::from(pdu.length_ind.unwrap()) * 8)
            } else {
                let pdu = MacFragUl::from_bitbuf(&mut bits).unwrap();
                (pdu.fill_bits, 268)
            };
            let header = bits.get_pos();
            let mut stop = end;
            if fill {
                while block.view().read((stop - 1) as u16, 1).unwrap() == 0 {
                    stop -= 1;
                }
                stop -= 1;
            }
            recovered
                .append(block.view().window(header as u16, (stop - header) as u16).unwrap())
                .unwrap();
        }
        assert_eq!(recovered, remainder, "SDU {len}");
    }
}

#[test]
fn clearing_deadline_does_not_grow_the_llc_adapter_queue_each_tick() {
    use tetra_ms_core::cmce::CmceEvent;
    let mut cc = CcMsSubentity::new(None);
    let mut queue = MessageQueue::new();
    let route = CallRoute {
        main_address: TetraAddress::new(91, SsiType::Gssi),
        handle: 0,
        endpoint_id: 0,
        link_id: 0,
    };
    assert!(cc.dispatch(
        &mut queue,
        CmceEvent::Downlink {
            sdu: minimum(7).view(),
            route
        }
    ));
    queue = MessageQueue::new();
    let deadline = cc.core.calls.get(&0x1234).unwrap().timers.call_deadline.unwrap();
    for tick in 0..100 {
        cc.core.push_event(deadline.add_timeslots(tick), CmceEvent::Tick).unwrap();
        cc.drain_actions(&mut queue);
    }
    assert_eq!(
        queue
            .iter()
            .filter(|message| matches!(message.msg, SapMsgInner::LcmcMleUnitdataReq(_)))
            .count(),
        1
    );
    assert_eq!(queue.iter().count(), 2);
}

#[test]
fn local_tx_confirm_reports_denied_permission_without_revoking_current_floor() {
    use crate::net_telemetry::channel::telemetry_channel;
    use tetra_ms_core::cmce::CmceEvent;
    let (sink, rx) = telemetry_channel();
    let mut cc = CcMsSubentity::new(Some(sink));
    cc.own_issi = Some(123);
    let mut queue = MessageQueue::new();
    let route = CallRoute {
        main_address: TetraAddress::new(91, SsiType::Gssi),
        handle: 0,
        endpoint_id: 0,
        link_id: 0,
    };
    assert!(cc.dispatch(
        &mut queue,
        CmceEvent::Downlink {
            sdu: minimum(7).view(),
            route
        }
    ));
    assert!(cc.dispatch(&mut queue, CmceEvent::Demand { id: 0x1234, priority: 0 }));
    while rx.try_recv().is_some() {}
    let mut grant = minimum(11);
    grant.view_mut().write(19, 2, 0).unwrap();
    grant.view_mut().write(21, 1, 1).unwrap();
    assert!(cc.dispatch(&mut queue, CmceEvent::Downlink { sdu: grant.view(), route }));
    let Some(TelemetryEvent::TnccTxConfirm { confirm, .. }) = rx.try_recv() else {
        panic!("TX confirm");
    };
    assert_eq!(
        confirm.transmit_request_permission,
        tncc::TransmissionRequestPermission::NotAllowedToRequestForTransmission
    );
    assert!(
        cc.core.calls.get(&0x1234).unwrap().accepts_uplink(),
        "permission controls future demand, not the grant itself"
    );
}
