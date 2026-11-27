use super::*;
use crate::net_control::channel::make_control_link;
use tetra_core::{BitBuffer, SsiType, TetraAddress};
use tetra_saps::control::enums::sds_user_data::SdsUserData;
use tetra_saps::lcmc::LcmcMleUnitdataInd;
use tetra_saps::tnsds::{DeliveryReportRequest, TnsdsMessageRequest, TnsdsReportRequest, TnsdsStatusRequest, TnsdsUnitdataRequest};

const MS_TOML: &str = r#"
config_version = "0.6"
stack_mode = "Ms"
[phy_io]
backend = "SoapySdr"
[phy_io.soapysdr]
tx_freq = 439825000
rx_freq = 430425000
ppm_err = 0
device = "driver=sx"
sample_rate = 600000
rx_antenna = "RX"
tx_antenna = "TX"
rx_gain_lna = 48.0
rx_gain_pga = 8.0
tx_gain_dac = 0.0
tx_gain_mixer = 0.0
[net_info]
mcc = 901
mnc = 9999
[cell_info]
freq_band = 4
main_carrier = 1593
duplex_spacing = 7
custom_duplex_spacing = 9400000
freq_offset = 0
reverse_operation = false
location_area = 1
colour_code = 1
[ms]
issi = 1000001
subscriber_class = 1
attach_groups = []
"#;

fn cmce(control: Option<ControlEndpoint>) -> CmceMs {
    let config = tetra_config::magentastation::from_toml_str(MS_TOML).unwrap();
    CmceMs::new(SharedConfig::from_parts(config, None), None, control)
}

fn message_request(ssi: u32, bits: u16, data: Vec<u8>) -> TnsdsMessageRequest {
    TnsdsMessageRequest {
        called_party_ssi: ssi,
        called_party_is_group: false,
        protocol_id: 0x82,
        delivery_report_request: DeliveryReportRequest::Received,
        message_reference: 42,
        user_data: data,
        user_data_bits: bits,
    }
}

#[test]
fn unsupported_cmce_and_facility_are_dropped_without_calls_or_output() {
    let mut cmce = cmce(None);
    let mut queue = MessageQueue::new();
    for kind in [8, 15, 16, 31] {
        let mut sdu = BitBuffer::new(5);
        sdu.write_bits(kind, 5);
        sdu.seek(0);
        cmce.rx_unitdata_ind(
            &mut queue,
            SapMsg {
                sap: Sap::LcmcSap,
                src: TetraEntity::Mle,
                dest: TetraEntity::Cmce,
                msg: SapMsgInner::LcmcMleUnitdataInd(LcmcMleUnitdataInd {
                    sdu,
                    handle: 0,
                    endpoint_id: 0,
                    link_id: 0,
                    received_tetra_address: TetraAddress::new(91, SsiType::Gssi),
                    chan_change_resp_req: false,
                    chan_change_handle: None,
                }),
            },
        );
        assert!(queue.iter().next().is_none());
        assert_eq!(cmce.cc.call_count(), 0);
        assert_eq!(cmce.cc.pending_origination_count(), 0);
    }
}

#[test]
fn sds_admission_ack_reflects_invalid_lengths_and_exact_maxima() {
    for (message, bits, data, accepted) in [
        (false, 9, vec![0], false),
        (false, 2048, vec![0; 256], false),
        (false, 2047, vec![0; 256], true),
        (false, 0, vec![], true),
        (true, 9, vec![0], false),
        (true, 2024, vec![0; 253], false),
        (true, 2023, vec![0; 253], true),
        (true, 0, vec![], true),
    ] {
        let (dispatcher, endpoint) = make_control_link();
        let mut cmce = cmce(Some(endpoint));
        let mut queue = MessageQueue::new();
        let command = if message {
            ControlCommand::TnsdsSendMessage {
                handle: 7,
                request: message_request(555, bits, data),
            }
        } else {
            ControlCommand::TnsdsUnitdata {
                handle: 7,
                request: TnsdsUnitdataRequest {
                    called_party_ssi: 555,
                    called_party_is_group: false,
                    user_data: SdsUserData::Type4(bits, data),
                },
            }
        };
        cmce.handle_control_command(&mut queue, command);
        let Some(ControlResponse::TnsdsAck {
            handle,
            accepted: actual,
            detail,
        }) = dispatcher.try_recv_response()
        else {
            panic!("expected TNSDS admission ack");
        };
        assert_eq!((handle, actual), (7, accepted), "message={message}, bits={bits}");
        assert_eq!(detail.is_none(), accepted);
        assert_eq!(queue.iter().count(), usize::from(accepted));
        assert!(dispatcher.try_recv_response().is_none());
        if message {
            assert_eq!(
                cmce.sds.cancel(&tetra_saps::tnsds::TnsdsCancelRequest { message_reference: 42 }),
                accepted
            );
        }
    }
}

#[test]
fn all_sds_control_paths_reject_invalid_destinations_and_ack_valid_requests() {
    for ssi in [0, 0x100_0000, 0xff_ffff] {
        let commands = [
            ControlCommand::TnsdsUnitdata {
                handle: 1,
                request: TnsdsUnitdataRequest {
                    called_party_ssi: ssi,
                    called_party_is_group: false,
                    user_data: SdsUserData::Type1(1),
                },
            },
            ControlCommand::TnsdsStatus {
                handle: 2,
                request: TnsdsStatusRequest {
                    called_party_ssi: ssi,
                    called_party_is_group: true,
                    status_number: u16::MAX,
                },
            },
            ControlCommand::TnsdsSendMessage {
                handle: 3,
                request: message_request(ssi, 0, vec![]),
            },
            ControlCommand::TnsdsSendReport {
                handle: 4,
                request: TnsdsReportRequest {
                    called_party_ssi: ssi,
                    message_reference: 42,
                    delivery_status: u8::MAX,
                },
            },
        ];
        let (dispatcher, endpoint) = make_control_link();
        let mut cmce = cmce(Some(endpoint));
        let mut queue = MessageQueue::new();
        for command in commands {
            cmce.handle_control_command(&mut queue, command);
            let Some(ControlResponse::TnsdsAck { accepted, detail, .. }) = dispatcher.try_recv_response() else {
                panic!("SDS ack");
            };
            assert_eq!(accepted, ssi == 0xff_ffff);
            assert_eq!(detail.is_none(), accepted);
        }
        assert_eq!(queue.iter().count(), if ssi == 0xff_ffff { 4 } else { 0 });
    }
}
