//! Platform/SAP translation for the portable SDS engine.
use super::*;
use tetra_ms_core::sds::{SdsAction, SdsCore, SdsError, SdsEvent, UserData};
use tetra_ms_types::{BitView, PackedBits};

pub(super) fn data_to_core(data: &SdsUserData) -> Result<UserData, SdsError> {
    Ok(match data {
        SdsUserData::Type1(value) => UserData::Type1(*value),
        SdsUserData::Type2(value) => UserData::Type2(*value),
        SdsUserData::Type3(value) => UserData::Type3(*value),
        SdsUserData::Type4(len, bytes) => UserData::Type4(PackedBits::from_view(BitView::new(bytes, *len)?)?),
    })
}

pub(super) fn data_from_core(data: UserData) -> SdsUserData {
    match data {
        UserData::Type1(value) => SdsUserData::Type1(value),
        UserData::Type2(value) => SdsUserData::Type2(value),
        UserData::Type3(value) => SdsUserData::Type3(value),
        UserData::Type4(bits) => SdsUserData::Type4(bits.len(), bits.as_bytes().to_vec()),
    }
}

pub(super) fn dispatch(core: &mut SdsCore, telemetry: Option<&TelemetrySink>, queue: &mut MessageQueue, event: SdsEvent<'_>) {
    if let Err(error) = core.push_event(event) {
        tracing::warn!(?error, "CMCE-MS SDS rejected event");
    }
    drain(core, telemetry, queue);
}

pub(super) fn drain(core: &mut SdsCore, telemetry: Option<&TelemetrySink>, queue: &mut MessageQueue) {
    while let Some(action) = core.pop_action() {
        let indication = match action {
            SdsAction::Uplink { destination, sdu } => {
                let mut buffer = BitBuffer::new_autoexpand(usize::from(sdu.len()));
                for bit in 0..sdu.len() {
                    buffer.write_bit(sdu.view().read(bit, 1).expect("bounded core bits") as u8);
                }
                buffer.seek(0);
                queue.push_back(SapMsg {
                    sap: Sap::LcmcSap,
                    src: TetraEntity::Cmce,
                    dest: TetraEntity::Mle,
                    msg: SapMsgInner::LcmcMleUnitdataReq(LcmcMleUnitdataReq {
                        sdu: buffer,
                        handle: 0,
                        endpoint_id: 0,
                        link_id: 0,
                        layer2service: if destination.ssi_type == SsiType::Gssi {
                            Layer2Service::Unacknowledged
                        } else {
                            Layer2Service::Acknowledged
                        },
                        pdu_prio: 0,
                        layer2_qos: 0,
                        stealing_permission: false,
                        stealing_repeats_flag: false,
                        chan_alloc: None,
                        main_address: destination,
                        tx_reporter: None,
                    }),
                });
                continue;
            }
            SdsAction::Unitdata { source, group, data } => TelemetryEvent::TnsdsUnitdataIndication(TnsdsUnitdataIndication {
                calling_party_ssi: source,
                called_party_is_group: group,
                user_data: data_from_core(data),
            }),
            SdsAction::Status { source, status } => TelemetryEvent::TnsdsStatusIndication(TnsdsStatusIndication {
                calling_party_ssi: source,
                status_number: status,
            }),
            SdsAction::Message {
                source,
                group,
                protocol_id,
                report_request,
                reference,
                data,
            } => TelemetryEvent::TnsdsMessageIndication(TnsdsMessageIndication {
                calling_party_ssi: source,
                called_party_is_group: group,
                protocol_id,
                delivery_report_request: DeliveryReportRequest::from_bits(report_request),
                message_reference: reference,
                user_data: data.as_bytes().to_vec(),
                user_data_bits: data.len(),
            }),
            SdsAction::Report {
                source,
                reference,
                status,
                short,
            } => TelemetryEvent::TnsdsReportIndication(TnsdsReportIndication {
                calling_party_ssi: source,
                message_reference: reference,
                delivery_status: status,
                short_form: short,
            }),
        };
        if let Some(sink) = telemetry {
            sink.send(indication);
        }
    }
}
