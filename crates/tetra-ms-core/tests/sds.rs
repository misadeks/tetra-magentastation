use tetra_ms_core::sds::{SdsAction, SdsCore, SdsError, SdsEvent, TEXT_PROTOCOL_ID, UserData};
use tetra_ms_types::{BitView, PackedBits, SsiType, TetraAddress};

fn address(group: bool) -> TetraAddress {
    TetraAddress::new(555, if group { SsiType::Gssi } else { SsiType::Issi })
}

fn downlink(data: &[u8], len: u16) -> PackedBits {
    let mut bits = PackedBits::new();
    bits.push(5, 15).unwrap();
    bits.push(2, 1).unwrap();
    bits.push(24, 555).unwrap();
    bits.push(2, 3).unwrap();
    bits.push(11, u32::from(len)).unwrap();
    bits.append(BitView::new(data, len).unwrap()).unwrap();
    bits.push(1, 0).unwrap();
    bits
}

#[test]
fn uplink_non_byte_payload_matches_historical_field_order() {
    let mut core = SdsCore::new();
    let payload = PackedBits::from_view(BitView::new(&[0xac], 6).unwrap()).unwrap();
    core.push_event(SdsEvent::Unitdata {
        destination: address(false),
        data: &UserData::Type4(payload),
    })
    .unwrap();
    let SdsAction::Uplink { destination, sdu } = core.pop_action().unwrap() else {
        panic!()
    };
    assert_eq!(destination, address(false));
    assert_eq!(sdu.len(), 55);
    assert_eq!(sdu.view().read(0, 5), Ok(15));
    assert_eq!(sdu.view().read(5, 4), Ok(0));
    assert_eq!(sdu.view().read(9, 2), Ok(1));
    assert_eq!(sdu.view().read(11, 24), Ok(555));
    assert_eq!(sdu.view().read(35, 13), Ok((3 << 11) | 6));
    assert_eq!(sdu.view().read(48, 7), Ok(0b1010110));
}

#[test]
fn transfer_received_report_is_individual_only_and_keeps_partial_byte() {
    let bits = downlink(&[TEXT_PROTOCOL_ID, 0x0c, 42, 0xa8], 29);
    for group in [false, true] {
        let mut core = SdsCore::new();
        core.push_event(SdsEvent::Downlink { sdu: bits.view(), group }).unwrap();
        let SdsAction::Message {
            source,
            group: received_group,
            report_request,
            reference,
            data,
            ..
        } = core.pop_action().unwrap()
        else {
            panic!()
        };
        assert_eq!((source, received_group, report_request, reference), (555, group, 3, 42));
        assert_eq!((data.len(), data.as_bytes()), (5, &[0xa8][..]));
        if group {
            assert!(core.pop_action().is_none());
        } else {
            let SdsAction::Uplink { destination, sdu } = core.pop_action().unwrap() else {
                panic!()
            };
            assert_eq!(destination, address(false));
            assert_eq!(sdu.view().read(48, 32), Ok(0x8210_002a));
            assert!(core.pop_action().is_none());
        }
    }
}

#[test]
fn all_references_fit_and_reuse_replaces_destination() {
    let mut core = SdsCore::new();
    let empty = BitView::new(&[], 0).unwrap();
    for reference in 0..=u8::MAX {
        core.push_event(SdsEvent::Message {
            destination: address(true),
            protocol_id: TEXT_PROTOCOL_ID,
            report_request: 1,
            reference,
            data: empty,
        })
        .unwrap();
        core.pop_action().unwrap();
        assert_eq!(core.outstanding(reference), Some(555));
    }
    core.push_event(SdsEvent::Message {
        destination: TetraAddress::new(777, SsiType::Issi),
        protocol_id: TEXT_PROTOCOL_ID,
        report_request: 2,
        reference: 42,
        data: empty,
    })
    .unwrap();
    assert_eq!(core.outstanding(42), Some(777));
    assert!(core.cancel(42));
    assert!(!core.cancel(42));
    assert_eq!(core.outstanding(41), Some(555));
}

#[test]
fn report_ack_and_short_report_follow_legacy_reference_tracking() {
    let mut core = SdsCore::new();
    let empty = BitView::new(&[], 0).unwrap();
    core.push_event(SdsEvent::Message {
        destination: address(false),
        protocol_id: TEXT_PROTOCOL_ID,
        report_request: 1,
        reference: 7,
        data: empty,
    })
    .unwrap();
    core.pop_action();
    let bits = downlink(&[TEXT_PROTOCOL_ID, 0x18, 2, 7], 32);
    core.push_event(SdsEvent::Downlink {
        sdu: bits.view(),
        group: false,
    })
    .unwrap();
    assert_eq!(
        core.pop_action(),
        Some(SdsAction::Report {
            source: 555,
            reference: 7,
            status: 2,
            short: false
        })
    );
    assert_eq!(core.outstanding(7), None);
    let SdsAction::Uplink { sdu, .. } = core.pop_action().unwrap() else {
        panic!()
    };
    assert_eq!(sdu.view().read(48, 32), Ok(0x8220_0207));
    let mut status = PackedBits::new();
    status.push(5, 8).unwrap();
    status.push(2, 1).unwrap();
    status.push(24, 555).unwrap();
    status.push(16, 0x7d2a).unwrap();
    status.push(1, 0).unwrap();
    core.push_event(SdsEvent::Downlink {
        sdu: status.view(),
        group: false,
    })
    .unwrap();
    assert_eq!(
        core.pop_action(),
        Some(SdsAction::Report {
            source: 555,
            reference: 42,
            status: 0x52,
            short: true
        })
    );
}

#[test]
fn output_backpressure_is_atomic_for_both_report_actions_and_tracking() {
    let mut core = SdsCore::new();
    for _ in 0..3 {
        core.push_event(SdsEvent::Status {
            destination: address(false),
            status: 0x8002,
        })
        .unwrap();
    }
    let bits = downlink(&[TEXT_PROTOCOL_ID, 0x04, 7], 24);
    assert_eq!(
        core.push_event(SdsEvent::Downlink {
            sdu: bits.view(),
            group: false
        }),
        Err(SdsError::OutputFull)
    );
    core.push_event(SdsEvent::Status {
        destination: address(false),
        status: 0x8002,
    })
    .unwrap();
    assert_eq!(
        core.push_event(SdsEvent::Message {
            destination: address(false),
            protocol_id: TEXT_PROTOCOL_ID,
            report_request: 1,
            reference: 42,
            data: BitView::new(&[], 0).unwrap(),
        }),
        Err(SdsError::OutputFull)
    );
    assert_eq!(core.outstanding(42), None);
    for _ in 0..4 {
        assert!(matches!(core.pop_action(), Some(SdsAction::Uplink { .. })));
    }
    assert!(core.pop_action().is_none());
}

#[test]
fn malformed_downlink_never_emits_an_ack_or_indication() {
    let bits = downlink(&[TEXT_PROTOCOL_ID, 0x04, 7, 0xa8], 29);
    for len in 0..bits.len() {
        let mut core = SdsCore::new();
        assert!(
            core.push_event(SdsEvent::Downlink {
                sdu: bits.view().window(0, len).unwrap(),
                group: false
            })
            .is_err()
        );
        assert!(core.pop_action().is_none());
    }
}

#[test]
fn unsupported_tl_variants_remain_opaque_and_invalid_destinations_do_not_track() {
    let mut core = SdsCore::new();
    for bytes in [[0x82, 0x80, 1, 2], [0x82, 0x01, 1, 2], [0x82, 0x30, 1, 2]] {
        let bits = downlink(&bytes, 32);
        core.push_event(SdsEvent::Downlink {
            sdu: bits.view(),
            group: false,
        })
        .unwrap();
        assert!(matches!(
            core.pop_action(),
            Some(SdsAction::Unitdata {
                data: UserData::Type4(_),
                ..
            })
        ));
    }
    assert_eq!(
        core.push_event(SdsEvent::Message {
            destination: TetraAddress::new(0x100_0000, SsiType::Issi),
            protocol_id: TEXT_PROTOCOL_ID,
            report_request: 1,
            reference: 8,
            data: BitView::new(&[], 0).unwrap(),
        }),
        Err(SdsError::InvalidValue)
    );
    assert_eq!(core.outstanding(8), None);
}

#[test]
fn in_place_reset_clears_outstanding_reports_and_pending_actions() {
    let mut storage = core::mem::MaybeUninit::<SdsCore>::uninit();
    // SAFETY: exclusively owned aligned storage for the whole SDS core.
    unsafe {
        SdsCore::init_at(storage.as_mut_ptr());
    }
    // SAFETY: init_at initialized every field.
    let mut core = unsafe { storage.assume_init() };
    core.push_event(SdsEvent::Message {
        destination: address(false),
        protocol_id: TEXT_PROTOCOL_ID,
        report_request: 1,
        reference: 7,
        data: BitView::new(&[], 0).unwrap(),
    })
    .unwrap();
    assert_eq!(core.outstanding(7), Some(555));
    core.reset();
    assert_eq!(core.outstanding(7), None);
    assert!(core.pop_action().is_none());
}

#[test]
fn largest_transfer_and_opaque_payload_do_not_need_staged_owned_data() {
    let mut core = SdsCore::new();
    let bytes = [0xa5; 256];
    let body = BitView::new(&bytes, 2023).unwrap();
    core.push_event(SdsEvent::Message {
        destination: address(false),
        protocol_id: TEXT_PROTOCOL_ID,
        report_request: 1,
        reference: 17,
        data: body,
    })
    .unwrap();
    let SdsAction::Uplink { sdu, .. } = core.pop_action().unwrap() else {
        panic!()
    };
    assert_eq!(sdu.len(), 2096);
    assert_eq!(sdu.view().read(37, 11), Ok(2047));
    assert_eq!(sdu.view().read(48, 24), Ok(0x820411));
    let user = PackedBits::from_view(sdu.view().window(48, 2047).unwrap()).unwrap();
    let received = downlink(user.as_bytes(), user.len());
    core.push_event(SdsEvent::Downlink {
        sdu: received.view(),
        group: false,
    })
    .unwrap();
    let SdsAction::Message { data, reference, .. } = core.pop_action().unwrap() else {
        panic!()
    };
    assert_eq!(reference, 17);
    assert_eq!(data, PackedBits::from_view(body).unwrap());
    assert!(matches!(core.pop_action(), Some(SdsAction::Uplink { .. })));

    let opaque = downlink(&bytes, 2047); // application-defined TL type, passed through
    core.push_event(SdsEvent::Downlink {
        sdu: opaque.view(),
        group: true,
    })
    .unwrap();
    assert_eq!(
        core.pop_action(),
        Some(SdsAction::Unitdata {
            source: 555,
            group: true,
            data: UserData::Type4(PackedBits::from_view(BitView::new(&bytes, 2047).unwrap()).unwrap()),
        })
    );
    assert!(core.pop_action().is_none());
}

#[test]
fn report_backpressure_does_not_clear_tracking_and_retry_preserves_fifo_order() {
    let mut core = SdsCore::new();
    core.push_event(SdsEvent::Message {
        destination: address(false),
        protocol_id: TEXT_PROTOCOL_ID,
        report_request: 1,
        reference: 7,
        data: BitView::new(&[], 0).unwrap(),
    })
    .unwrap();
    core.pop_action();
    for status in 0x8000..0x8003 {
        core.push_event(SdsEvent::Status {
            destination: address(false),
            status,
        })
        .unwrap();
    }
    let report = downlink(&[TEXT_PROTOCOL_ID, 0x18, 2, 7], 32);
    assert_eq!(
        core.push_event(SdsEvent::Downlink {
            sdu: report.view(),
            group: false
        }),
        Err(SdsError::OutputFull)
    );
    assert_eq!(core.outstanding(7), Some(555));
    let SdsAction::Uplink { sdu, .. } = core.pop_action().unwrap() else {
        panic!()
    };
    assert_eq!(sdu.view().read(35, 16), Ok(0x8000));
    core.push_event(SdsEvent::Downlink {
        sdu: report.view(),
        group: false,
    })
    .unwrap();
    assert_eq!(core.outstanding(7), None);
    for status in 0x8001..0x8003 {
        let SdsAction::Uplink { sdu, .. } = core.pop_action().unwrap() else {
            panic!()
        };
        assert_eq!(sdu.view().read(35, 16), Ok(status));
    }
    assert_eq!(
        core.pop_action(),
        Some(SdsAction::Report {
            source: 555,
            reference: 7,
            status: 2,
            short: false
        })
    );
    let SdsAction::Uplink { sdu, .. } = core.pop_action().unwrap() else {
        panic!()
    };
    assert_eq!(sdu.view().read(48, 32), Ok(0x8220_0207));
    assert!(core.pop_action().is_none());
}

#[test]
fn borrowed_parser_preserves_all_fixed_data_types_and_tsi_calling_addresses() {
    let mut core = SdsCore::new();
    for data in [
        UserData::Type1(0x8001),
        UserData::Type2(0x89ab_cdef),
        UserData::Type3(0x0123_4567_89ab_cdef),
    ] {
        let mut bits = PackedBits::new();
        bits.push(5, 15).unwrap();
        bits.push(2, 2).unwrap();
        bits.push(24, 555).unwrap();
        bits.push(24, 0x123456).unwrap();
        match data {
            UserData::Type1(value) => {
                bits.push(2, 0).unwrap();
                bits.push(16, u32::from(value)).unwrap();
            }
            UserData::Type2(value) => {
                bits.push(2, 1).unwrap();
                bits.push(32, value).unwrap();
            }
            UserData::Type3(value) => {
                bits.push(2, 2).unwrap();
                bits.push(32, (value >> 32) as u32).unwrap();
                bits.push(32, value as u32).unwrap();
            }
            _ => unreachable!(),
        }
        bits.push(1, 0).unwrap();
        core.push_event(SdsEvent::Downlink {
            sdu: bits.view(),
            group: false,
        })
        .unwrap();
        assert_eq!(
            core.pop_action(),
            Some(SdsAction::Unitdata {
                source: 555,
                group: false,
                data
            })
        );
    }
}
