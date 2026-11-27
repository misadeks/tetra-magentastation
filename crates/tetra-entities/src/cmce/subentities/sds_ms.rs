use tetra_config::magentastation::SharedConfig;
use tetra_core::{BitBuffer, Layer2Service, Sap, SsiType, TetraAddress, tetra_entities::TetraEntity};
#[cfg(test)]
use tetra_pdus::cmce::fields::sds_tl;
#[cfg(test)]
use tetra_pdus::cmce::{
    enums::party_type_identifier::PartyTypeIdentifier,
    enums::pre_coded_status::PreCodedStatus,
    pdus::{u_sds_data::USdsData, u_status::UStatus},
};
use tetra_saps::control::enums::sds_user_data::SdsUserData;
use tetra_saps::lcmc::LcmcMleUnitdataReq;
use tetra_saps::tnsds::{
    DeliveryReportRequest, TnsdsCancelRequest, TnsdsMessageIndication, TnsdsMessageRequest, TnsdsReportIndication, TnsdsReportRequest,
    TnsdsStatusIndication, TnsdsUnitdataIndication,
};
use tetra_saps::{SapMsg, SapMsgInner};

use crate::MessageQueue;
use crate::net_telemetry::{TelemetryEvent, channel::TelemetrySink};
use tetra_ms_core::sds::{SdsCore, SdsError, SdsEvent};
use tetra_ms_types::BitView;

#[path = "cc_ms/sds_adapter.rs"]
mod adapter;

/// Clause 13 Short Data Service CMCE sub-entity (MS side).
///
/// Receive: decodes D-SDS-DATA (cl. 14.7.1.10) / D-STATUS (cl. 14.7.1.11) and
/// surfaces them to the user application as TNSDS-UNITDATA / TNSDS-STATUS
/// indications (cl. 13.3.2) over the telemetry SAP.
///
/// Transmit: builds U-SDS-DATA (cl. 14.7.2.8) / U-STATUS (cl. 14.7.2.7) from
/// TNSDS-UNITDATA / TNSDS-STATUS requests and sends them uplink via the
/// LCMC-SAP MLE-UNITDATA request (mirrors the BS `sds_bs.rs`). The calling
/// party (own ISSI) is carried by the MAC source address, so it is not encoded
/// in the uplink PDU. Protocol state, wire PDUs and SDS-TL delivery reporting
/// are owned by `tetra_ms_core::sds`.
pub struct SdsMsSubentity {
    #[allow(dead_code)]
    config: SharedConfig,
    telemetry: Option<TelemetrySink>,
    /// Portable protocol state; this adapter only translates SAPs/telemetry.
    core: SdsCore,
}

impl SdsMsSubentity {
    /// Create a new instance of the SDS sub-entity.
    pub fn new(config: SharedConfig, telemetry: Option<TelemetrySink>) -> Self {
        SdsMsSubentity {
            config,
            telemetry,
            core: SdsCore::new(),
        }
    }

    pub fn rx_sds_data(&mut self, queue: &mut MessageQueue, mut message: SapMsg) {
        let SapMsgInner::LcmcMleUnitdataInd(prim) = &mut message.msg else {
            return;
        };
        let Ok(sdu) = prim.sdu.as_bit_view() else { return };
        let Ok(offset) = u16::try_from(prim.sdu.get_pos()) else { return };
        let Ok(sdu) = sdu.window(offset, sdu.len().saturating_sub(offset)) else {
            return;
        };
        let _ = adapter::dispatch(
            &mut self.core,
            self.telemetry.as_ref(),
            queue,
            SdsEvent::Downlink {
                sdu,
                group: prim.received_tetra_address.ssi_type == SsiType::Gssi,
            },
        );
    }

    pub fn rx_status(&mut self, queue: &mut MessageQueue, message: SapMsg) {
        self.rx_sds_data(queue, message);
    }

    /// Send a U-SDS-DATA PDU uplink (cl. 14.7.2.8) — TNSDS-UNITDATA request
    /// (Table 13.3, cl. 13.3.2.3). Mirrors `sds_bs::send_d_sds_data`.
    /// Success means local admission, not over-the-air delivery.
    pub fn send_u_sds_data(
        &mut self,
        queue: &mut MessageQueue,
        dest_ssi: u32,
        dest_is_group: bool,
        user_defined_data: SdsUserData,
    ) -> Result<(), SdsError> {
        let data = adapter::data_to_core(&user_defined_data)?;
        adapter::dispatch(
            &mut self.core,
            self.telemetry.as_ref(),
            queue,
            SdsEvent::Unitdata {
                destination: TetraAddress::new(dest_ssi, if dest_is_group { SsiType::Gssi } else { SsiType::Issi }),
                data: &data,
            },
        )
    }

    /// Send a U-STATUS PDU uplink (cl. 14.7.2.7) — TNSDS-STATUS request
    /// (Table 13.1, cl. 13.3.2.1). Mirrors `sds_bs::send_d_status`.
    pub fn send_u_status(
        &mut self,
        queue: &mut MessageQueue,
        dest_ssi: u32,
        dest_is_group: bool,
        status_number: u16,
    ) -> Result<(), SdsError> {
        adapter::dispatch(
            &mut self.core,
            self.telemetry.as_ref(),
            queue,
            SdsEvent::Status {
                destination: TetraAddress::new(dest_ssi, if dest_is_group { SsiType::Gssi } else { SsiType::Issi }),
                status: status_number,
            },
        )
    }

    /// TNSDS-UNITDATA request carrying an SDS-TL SDS-TRANSFER (cl. 29.4.2.4).
    /// Wraps the application message in an SDS-TRANSFER PDU (message reference +
    /// delivery-report request) and sends it as Type-4 user data in a
    /// U-SDS-DATA. When a report is requested the message reference is tracked
    /// (for TNSDS-CANCEL and to correlate the returning report).
    pub fn send_message(&mut self, queue: &mut MessageQueue, req: &TnsdsMessageRequest) -> Result<(), SdsError> {
        let data = BitView::new(&req.user_data, req.user_data_bits)?;
        adapter::dispatch(
            &mut self.core,
            self.telemetry.as_ref(),
            queue,
            SdsEvent::Message {
                destination: TetraAddress::new(
                    req.called_party_ssi,
                    if req.called_party_is_group { SsiType::Gssi } else { SsiType::Issi },
                ),
                protocol_id: req.protocol_id,
                report_request: req.delivery_report_request.to_bits(),
                reference: req.message_reference,
                data,
            },
        )
    }

    /// TNSDS-REPORT request (Table 13.2, cl. 13.3.2.2): send an SDS-TL delivery/
    /// read report (SDS-REPORT) for a received message — e.g. a "consumed" report
    /// once the user application reads the message.
    pub fn send_report(&mut self, queue: &mut MessageQueue, req: &TnsdsReportRequest) -> Result<(), SdsError> {
        adapter::dispatch(
            &mut self.core,
            self.telemetry.as_ref(),
            queue,
            SdsEvent::Report {
                destination: req.called_party_ssi,
                reference: req.message_reference,
                status: req.delivery_status,
            },
        )
    }

    /// TNSDS-CANCEL: stop tracking a locally-outstanding SDS-TL message. Returns
    /// `true` if a matching outstanding message reference was found.
    pub fn cancel(&mut self, req: &TnsdsCancelRequest) -> bool {
        self.core.cancel(req.message_reference)
    }

    /// Poor man's rx_prim, as this is a subcomponent and not governed by the MessageRouter
    /// If need be, we can deviate from the standard's subentity ranking and make this a full-fledged component
    /// See Figure 14.2: Block view of CMCE-MS
    pub fn route_rf_deliver(&mut self, queue: &mut MessageQueue, message: SapMsg) {
        self.rx_sds_data(queue, message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net_telemetry::channel::telemetry_channel;
    use tetra_config::magentastation::{SharedConfig, from_toml_str};
    use tetra_pdus::cmce::pdus::{d_sds_data::DSdsData, d_status::DStatus};
    use tetra_saps::lcmc::LcmcMleUnitdataInd;

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

    fn make_sds(sink: Option<TelemetrySink>) -> SdsMsSubentity {
        let cfg = from_toml_str(MS_TOML).expect("valid MS test config");
        SdsMsSubentity::new(SharedConfig::from_parts(cfg, None), sink)
    }

    fn deliver(sdu: BitBuffer, received: TetraAddress) -> SapMsg {
        SapMsg {
            sap: Sap::LcmcSap,
            src: TetraEntity::Mle,
            dest: TetraEntity::Cmce,
            msg: SapMsgInner::LcmcMleUnitdataInd(LcmcMleUnitdataInd {
                sdu,
                handle: 0,
                endpoint_id: 0,
                link_id: 0,
                received_tetra_address: received,
                chan_change_resp_req: false,
                chan_change_handle: None,
            }),
        }
    }

    // --- Transmit (U-SDS-DATA / U-STATUS) decoded by the BS's own parsers ---

    #[test]
    fn u_sds_data_roundtrips_through_bs_parser() {
        let mut sds = make_sds(None);
        let mut q = MessageQueue::new();
        let payload = SdsUserData::Type4(16, vec![0xAB, 0xCD]);
        sds.send_u_sds_data(&mut q, 1234, false, payload.clone()).unwrap();

        let msg = q.pop_front().expect("U-SDS-DATA queued");
        let SapMsgInner::LcmcMleUnitdataReq(mut prim) = msg.msg else {
            panic!("expected LcmcMleUnitdataReq");
        };
        // Individual (ISSI) destination => acknowledged basic link (cl. 14.7.2).
        assert_eq!(prim.main_address.ssi, 1234);
        assert!(matches!(prim.main_address.ssi_type, SsiType::Issi));
        assert!(matches!(prim.layer2service, Layer2Service::Acknowledged));

        let pdu = USdsData::from_bitbuf(&mut prim.sdu).expect("BS decodes U-SDS-DATA");
        assert_eq!(pdu.called_party_ssi, Some(1234));
        assert!(matches!(pdu.called_party_type_identifier, PartyTypeIdentifier::Ssi));
        assert_eq!(pdu.user_defined_data, payload);
    }

    #[test]
    fn u_status_roundtrips_through_bs_parser_group() {
        let mut sds = make_sds(None);
        let mut q = MessageQueue::new();
        let status = PreCodedStatus::from(0x8002).into_raw();
        sds.send_u_status(&mut q, 91, true, status).unwrap();

        let msg = q.pop_front().expect("U-STATUS queued");
        let SapMsgInner::LcmcMleUnitdataReq(mut prim) = msg.msg else {
            panic!("expected LcmcMleUnitdataReq");
        };
        // Group (GSSI) destination => unacknowledged (cl. 14.7.2).
        assert_eq!(prim.main_address.ssi, 91);
        assert!(matches!(prim.main_address.ssi_type, SsiType::Gssi));
        assert!(matches!(prim.layer2service, Layer2Service::Unacknowledged));

        let pdu = UStatus::from_bitbuf(&mut prim.sdu).expect("BS decodes U-STATUS");
        assert_eq!(pdu.called_party_ssi, Some(91));
        assert_eq!(pdu.pre_coded_status.into_raw(), status);
    }

    // --- Receive emits exactly one TNSDS indication from a BS-built PDU ---

    fn build_d_sds_data(source: u32, data: SdsUserData) -> BitBuffer {
        let pdu = DSdsData {
            calling_party_type_identifier: PartyTypeIdentifier::Ssi,
            calling_party_address_ssi: Some(source as u64),
            calling_party_extension: None,
            user_defined_data: data,
            external_subscriber_number: None,
            dm_ms_address: None,
        };
        let mut sdu = BitBuffer::new_autoexpand(128);
        pdu.to_bitbuf(&mut sdu).unwrap();
        sdu.seek(0);
        sdu
    }

    fn build_d_status(source: u32, status: PreCodedStatus) -> BitBuffer {
        let pdu = DStatus {
            calling_party_type_identifier: PartyTypeIdentifier::Ssi,
            calling_party_address_ssi: Some(source as u64),
            calling_party_extension: None,
            pre_coded_status: status,
            external_subscriber_number: None,
            dm_ms_address: None,
        };
        let mut sdu = BitBuffer::new_autoexpand(64);
        pdu.to_bitbuf(&mut sdu).unwrap();
        sdu.seek(0);
        sdu
    }

    #[test]
    fn rx_d_sds_data_emits_unitdata_indication() {
        let (sink, source_rx) = telemetry_channel();
        let mut sds = make_sds(Some(sink));
        let mut q = MessageQueue::new();

        let payload = SdsUserData::Type4(16, vec![0x12, 0x34]);
        let sdu = build_d_sds_data(555, payload.clone());
        // Delivered to our own ISSI => individual (not group).
        let msg = deliver(sdu, TetraAddress::new(1000001, SsiType::Issi));
        sds.route_rf_deliver(&mut q, msg);

        let ev = source_rx.try_recv().expect("one telemetry event");
        match ev {
            TelemetryEvent::TnsdsUnitdataIndication(ind) => {
                assert_eq!(ind.calling_party_ssi, 555);
                assert!(!ind.called_party_is_group);
                assert_eq!(ind.user_data, payload);
            }
            other => panic!("unexpected event: {:?}", other),
        }
        assert!(source_rx.try_recv().is_none(), "exactly one indication");
    }

    #[test]
    fn rx_d_sds_data_group_addressed_sets_group_flag() {
        let (sink, source_rx) = telemetry_channel();
        let mut sds = make_sds(Some(sink));
        let mut q = MessageQueue::new();

        let sdu = build_d_sds_data(777, SdsUserData::Type1(5));
        let msg = deliver(sdu, TetraAddress::new(91, SsiType::Gssi));
        sds.route_rf_deliver(&mut q, msg);

        let ev = source_rx.try_recv().expect("one telemetry event");
        match ev {
            TelemetryEvent::TnsdsUnitdataIndication(ind) => {
                assert_eq!(ind.calling_party_ssi, 777);
                assert!(ind.called_party_is_group);
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[test]
    fn rx_d_status_emits_status_indication() {
        let (sink, source_rx) = telemetry_channel();
        let mut sds = make_sds(Some(sink));
        let mut q = MessageQueue::new();

        let status = PreCodedStatus::from(0x8003);
        let sdu = build_d_status(2200699, status);
        let msg = deliver(sdu, TetraAddress::new(1000001, SsiType::Issi));
        sds.route_rf_deliver(&mut q, msg);

        let ev = source_rx.try_recv().expect("one telemetry event");
        match ev {
            TelemetryEvent::TnsdsStatusIndication(ind) => {
                assert_eq!(ind.calling_party_ssi, 2200699);
                assert_eq!(ind.status_number, status.into_raw());
            }
            other => panic!("unexpected event: {:?}", other),
        }
        assert!(source_rx.try_recv().is_none(), "exactly one indication");
    }

    #[test]
    fn rx_without_sink_does_not_panic() {
        let mut sds = make_sds(None);
        let mut q = MessageQueue::new();
        let sdu = build_d_sds_data(555, SdsUserData::Type1(1));
        let msg = deliver(sdu, TetraAddress::new(1000001, SsiType::Issi));
        sds.route_rf_deliver(&mut q, msg);
        assert!(q.pop_front().is_none());
    }

    // --- SDS-TL (cl. 29): message reference + delivery reporting ---

    fn take_type4(msg: SapMsg) -> (u32, SsiType, u16, Vec<u8>) {
        let SapMsgInner::LcmcMleUnitdataReq(mut prim) = msg.msg else {
            panic!("expected LcmcMleUnitdataReq");
        };
        let ssi = prim.main_address.ssi;
        let ty = prim.main_address.ssi_type;
        let pdu = USdsData::from_bitbuf(&mut prim.sdu).expect("BS decodes U-SDS-DATA");
        let SdsUserData::Type4(len_bits, bytes) = pdu.user_defined_data else {
            panic!("expected Type-4 user data");
        };
        (ssi, ty, len_bits, bytes)
    }

    #[test]
    fn send_message_builds_sds_transfer_and_tracks_outstanding() {
        let mut sds = make_sds(None);
        let mut q = MessageQueue::new();
        sds.send_message(
            &mut q,
            &TnsdsMessageRequest {
                called_party_ssi: 2200699,
                called_party_is_group: false,
                protocol_id: sds_tl::PROTOCOL_ID_TEXT_MESSAGING,
                delivery_report_request: DeliveryReportRequest::ReceivedAndConsumed,
                message_reference: 42,
                user_data: vec![0x01, b'H', b'i'],
                user_data_bits: 24,
            },
        )
        .unwrap();
        let (ssi, ty, len_bits, bytes) = take_type4(q.pop_front().expect("U-SDS-DATA queued"));
        assert_eq!(ssi, 2200699);
        assert!(matches!(ty, SsiType::Issi));
        // Decode the SDS-TL PDU the same way the peer MS would.
        let tl = sds_tl::decode(len_bits, &bytes).expect("SDS-TL transfer");
        match tl {
            sds_tl::SdsTlPdu::Transfer(t) => {
                assert_eq!(t.message_reference, 42);
                assert_eq!(t.delivery_report_request, sds_tl::DRR_RECEIVED_AND_CONSUMED);
                assert_eq!(t.user_data, vec![0x01, b'H', b'i']);
            }
            other => panic!("expected transfer, got {other:?}"),
        }
        // Outstanding tracked (a report was requested) → cancel finds it.
        assert!(sds.cancel(&TnsdsCancelRequest { message_reference: 42 }));
        assert!(!sds.cancel(&TnsdsCancelRequest { message_reference: 42 }), "already cleared");
    }

    #[test]
    fn rx_transfer_indicates_and_auto_sends_received_report() {
        let (sink, rx) = telemetry_channel();
        let mut sds = make_sds(Some(sink));
        let mut q = MessageQueue::new();

        let (len_bits, bytes) = sds_tl::SdsTransfer {
            protocol_id: sds_tl::PROTOCOL_ID_TEXT_MESSAGING,
            delivery_report_request: sds_tl::DRR_RECEIVED,
            service_selection: false,
            message_reference: 7,
            user_data: vec![0x01, b'Y', b'o'],
            user_data_bits: 24,
        }
        .encode();
        let sdu = build_d_sds_data(555, SdsUserData::Type4(len_bits, bytes));
        sds.route_rf_deliver(&mut q, deliver(sdu, TetraAddress::new(1000001, SsiType::Issi)));

        // Message surfaced to the UI.
        match rx.try_recv().expect("message indication") {
            TelemetryEvent::TnsdsMessageIndication(ind) => {
                assert_eq!(ind.calling_party_ssi, 555);
                assert_eq!(ind.message_reference, 7);
                assert_eq!(ind.delivery_report_request, DeliveryReportRequest::Received);
                assert_eq!(ind.user_data, vec![0x01, b'Y', b'o']);
            }
            other => panic!("unexpected event: {other:?}"),
        }
        // Auto "received" SDS-REPORT emitted back to the sender.
        let (ssi, _ty, lb, b) = take_type4(q.pop_front().expect("auto SDS-REPORT queued"));
        assert_eq!(ssi, 555, "report addressed to the original sender");
        match sds_tl::decode(lb, &b).expect("SDS-TL report") {
            sds_tl::SdsTlPdu::Report(r) => {
                assert_eq!(r.message_reference, 7);
                assert_eq!(r.delivery_status, sds_tl::DELIVERY_RECEIPT_ACK_BY_DEST);
            }
            other => panic!("expected report, got {other:?}"),
        }
    }

    #[test]
    fn rx_group_transfer_does_not_auto_report() {
        let (sink, rx) = telemetry_channel();
        let mut sds = make_sds(Some(sink));
        let mut q = MessageQueue::new();
        let (len_bits, bytes) = sds_tl::SdsTransfer {
            protocol_id: sds_tl::PROTOCOL_ID_TEXT_MESSAGING,
            delivery_report_request: sds_tl::DRR_RECEIVED,
            service_selection: true,
            message_reference: 9,
            user_data: vec![b'!'],
            user_data_bits: 8,
        }
        .encode();
        let sdu = build_d_sds_data(777, SdsUserData::Type4(len_bits, bytes));
        sds.route_rf_deliver(&mut q, deliver(sdu, TetraAddress::new(91, SsiType::Gssi)));
        assert!(matches!(rx.try_recv(), Some(TelemetryEvent::TnsdsMessageIndication(_))));
        assert!(q.pop_front().is_none(), "group message must not trigger an individual report");
    }

    #[test]
    fn rx_report_emits_report_indication_and_clears_outstanding() {
        let (sink, rx) = telemetry_channel();
        let mut sds = make_sds(Some(sink));
        let mut q = MessageQueue::new();
        sds.send_message(
            &mut q,
            &TnsdsMessageRequest {
                called_party_ssi: 555,
                called_party_is_group: false,
                protocol_id: sds_tl::PROTOCOL_ID_TEXT_MESSAGING,
                delivery_report_request: DeliveryReportRequest::Received,
                message_reference: 7,
                user_data: vec![],
                user_data_bits: 0,
            },
        )
        .unwrap();
        q.pop_front().expect("transfer queued");
        let (len_bits, bytes) = sds_tl::SdsReport {
            protocol_id: sds_tl::PROTOCOL_ID_TEXT_MESSAGING,
            ack_required: false,
            delivery_status: sds_tl::DELIVERY_CONSUMED_BY_DEST,
            message_reference: 7,
        }
        .encode();
        let sdu = build_d_sds_data(555, SdsUserData::Type4(len_bits, bytes));
        sds.route_rf_deliver(&mut q, deliver(sdu, TetraAddress::new(1000001, SsiType::Issi)));
        match rx.try_recv().expect("report indication") {
            TelemetryEvent::TnsdsReportIndication(ind) => {
                assert_eq!(ind.message_reference, 7);
                assert_eq!(ind.delivery_status, sds_tl::DELIVERY_CONSUMED_BY_DEST);
                assert!(!ind.short_form);
            }
            other => panic!("unexpected event: {other:?}"),
        }
        assert!(
            !sds.cancel(&TnsdsCancelRequest { message_reference: 7 }),
            "outstanding cleared on report"
        );
    }

    #[test]
    fn rx_short_report_status_emits_report_indication() {
        let (sink, rx) = telemetry_channel();
        let mut sds = make_sds(Some(sink));
        let mut q = MessageQueue::new();
        let status = sds_tl::SdsShortReport {
            short_report_type: sds_tl::SHORT_REPORT_MESSAGE_CONSUMED,
            message_reference: 33,
        }
        .encode_status();
        let sdu = build_d_status(2200699, PreCodedStatus::from(status));
        sds.route_rf_deliver(&mut q, deliver(sdu, TetraAddress::new(1000001, SsiType::Issi)));
        match rx.try_recv().expect("report indication") {
            TelemetryEvent::TnsdsReportIndication(ind) => {
                assert_eq!(ind.calling_party_ssi, 2200699);
                assert_eq!(ind.message_reference, 33);
                assert_eq!(ind.delivery_status, sds_tl::DELIVERY_CONSUMED_BY_DEST);
                assert!(ind.short_form);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn send_report_builds_sds_report() {
        let mut sds = make_sds(None);
        let mut q = MessageQueue::new();
        sds.send_report(
            &mut q,
            &TnsdsReportRequest {
                called_party_ssi: 555,
                message_reference: 12,
                delivery_status: sds_tl::DELIVERY_CONSUMED_BY_DEST,
            },
        )
        .unwrap();
        let (ssi, ty, lb, b) = take_type4(q.pop_front().expect("U-SDS-DATA queued"));
        assert_eq!(ssi, 555);
        assert!(matches!(ty, SsiType::Issi));
        match sds_tl::decode(lb, &b).expect("SDS-TL report") {
            sds_tl::SdsTlPdu::Report(r) => {
                assert_eq!(r.message_reference, 12);
                assert_eq!(r.delivery_status, sds_tl::DELIVERY_CONSUMED_BY_DEST);
            }
            other => panic!("expected report, got {other:?}"),
        }
    }
}
