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
