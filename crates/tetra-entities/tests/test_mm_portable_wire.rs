//! Characterization against the pre-extraction PDU codecs, without the MS adapter.
use tetra_core::{BitBuffer, typed_pdu_fields::Type3FieldGeneric};
use tetra_ms_core::mm::{Action, Config, Event, MmCore, RegState};
use tetra_pdus::mm::{
    enums::location_update_type::LocationUpdateType,
    fields::{
        class_of_ms::ClassOfMs, group_identity_location_demand::GroupIdentityLocationDemand, group_identity_uplink::GroupIdentityUplink,
    },
    pdus::{d_location_update_command::DLocationUpdateCommand, u_location_update_demand::ULocationUpdateDemand},
};

#[test]
fn command_response_wire_matches_original_encoder_at_group_boundaries() {
    let mni = 234 << 14 | 5678;
    for groups in [vec![], vec![91], vec![91, 220], (1..=63).collect()] {
        for report in [false, true] {
            for cipher_control in [false, true] {
                let mut engine = MmCore::new(
                    Config {
                        issi: 1_000_001,
                        mcc: 234,
                        mnc: 5678,
                        initial_la: 100,
                    },
                    &groups,
                )
                .unwrap();
                let command = DLocationUpdateCommand {
                    group_identity_report: report,
                    cipher_control,
                    ciphering_parameters: cipher_control.then_some(0x3ff),
                    address_extension: Some(mni),
                    cell_type_control: None,
                    proprietary: None,
                };
                let mut downlink = BitBuffer::new_autoexpand(128);
                command.to_bitbuf(&mut downlink).unwrap();
                downlink.seek(0);
                engine.handle(Event::Downlink(downlink.as_bit_view().unwrap())).unwrap();
                let Some(Action::Send {
                    sdu,
                    priority,
                    detach_receipt,
                }) = engine.poll_action()
                else {
                    panic!()
                };
                assert_eq!(priority, if report { 3 } else { 6 });
                assert!(!detach_receipt);
                let original = ULocationUpdateDemand {
                    location_update_type: LocationUpdateType::DemandLocationUpdating,
                    request_to_append_la: false,
                    cipher_control: false,
                    ciphering_parameters: None,
                    class_of_ms: Some(ClassOfMs::from_bitbuf(&mut BitBuffer::from_bitstr("000110000000000001100000")).unwrap()),
                    energy_saving_mode: None,
                    la_information: None,
                    ssi: Some(1_000_001),
                    address_extension: Some(mni),
                    group_identity_location_demand: (!groups.is_empty()).then(|| GroupIdentityLocationDemand {
                        group_identity_attach_detach_mode: 1,
                        group_identity_uplink: Some(
                            groups
                                .iter()
                                .map(|&gssi| GroupIdentityUplink {
                                    class_of_usage: Some(4),
                                    group_identity_detachment_uplink: None,
                                    gssi: Some(gssi),
                                    address_extension: None,
                                    vgssi: None,
                                })
                                .collect(),
                        ),
                    }),
                    group_report_response: report.then(|| Type3FieldGeneric {
                        field_id: 4,
                        len: 1,
                        data: vec![0],
                    }),
                    authentication_uplink: None,
                    extended_capabilities: None,
                    proprietary: None,
                };
                let mut expected = BitBuffer::new_autoexpand(256);
                original.to_bitbuf(&mut expected).unwrap();
                let actual: String = (0..sdu.len())
                    .map(|pos| if sdu.view().read(pos, 1).unwrap() == 0 { '0' } else { '1' })
                    .collect();
                assert_eq!(actual, expected.to_bitstr(), "groups={} report={report}", groups.len());
            }
        }
    }
}

#[test]
fn original_command_optional_elements_are_parsed_and_foreign_mni_ignored() {
    for mni in [234 << 14 | 5678, 1] {
        let mut engine = MmCore::new(
            Config {
                issi: 1_000_001,
                mcc: 234,
                mnc: 5678,
                initial_la: 100,
            },
            &[],
        )
        .unwrap();
        let command = DLocationUpdateCommand {
            group_identity_report: false,
            cipher_control: false,
            ciphering_parameters: None,
            address_extension: Some(mni),
            cell_type_control: Some(Type3FieldGeneric {
                field_id: 13,
                len: 96,
                data: vec![0xa5; 12],
            }),
            proprietary: Some(Type3FieldGeneric {
                field_id: 15,
                len: 3,
                data: vec![0xa0],
            }),
        };
        let mut downlink = BitBuffer::new_autoexpand(256);
        command.to_bitbuf(&mut downlink).unwrap();
        downlink.seek(0);
        engine.handle(Event::Downlink(downlink.as_bit_view().unwrap())).unwrap();
        if mni == 1 {
            assert_eq!(engine.state().reg_state, RegState::Idle);
            assert!(engine.poll_action().is_none());
        } else {
            assert_eq!(engine.state().reg_state, RegState::Registering);
            assert!(matches!(engine.poll_action(), Some(Action::Send { .. })));
        }
    }
}
