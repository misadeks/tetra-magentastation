//! Application SAP/receipt translation. No LLC sequence or retry decisions live here.
use std::collections::BTreeMap;

use tetra_core::{BitBuffer, Sap, TdmaTime, TxReporter, TxState, tetra_entities::TetraEntity};
use tetra_ms_core::llc::{Completion, LlcAction, LlcConfig, LlcCore, LlcError, LlcEvent, Receipt, SendRequest, Service, TxFeedback};
use tetra_ms_types::{BitView, MAX_PDU_BITS, PackedBits};
use tetra_saps::{
    SapMsg, SapMsgInner,
    lcmc::{
        enums::{alloc_type::ChanAllocType, ul_dl_assignment::UlDlAssignment},
        fields::chan_alloc_req::CmceChanAllocReq,
    },
    tla::{TlaTlDataIndBl, TlaTlUnitdataIndBl},
    tma::{TmaUnitdataInd, TmaUnitdataReq},
};

use crate::MessageQueue;

struct Metadata {
    request: TmaUnitdataReq,
    active: Option<(Receipt, TxReporter)>,
    reported: bool,
}

pub(super) struct MsLlcAdapter {
    core: LlcCore,
    own_issi: u32,
    now: TdmaTime,
    next_command: u32,
    metadata: BTreeMap<u32, Metadata>,
}

fn remaining(source: &BitBuffer) -> Result<BitView<'_>, LlcError> {
    let len = source.get_len_remaining();
    if len > usize::from(MAX_PDU_BITS) {
        return Err(LlcError::CapacityExceeded);
    }
    let offset = u16::try_from(source.get_pos()).map_err(|_| LlcError::CapacityExceeded)?;
    Ok(source.as_bit_view()?.window(offset, len as u16)?)
}

fn desktop(source: &PackedBits) -> BitBuffer {
    let mut result = BitBuffer::from_bytes(source.as_bytes());
    result.set_raw_end(usize::from(source.len()));
    result
}

impl MsLlcAdapter {
    pub(super) fn new(own_issi: u32) -> Result<Self, LlcError> {
        Ok(Self {
            core: LlcCore::new(LlcConfig { own_issi })?,
            own_issi,
            now: TdmaTime::default(),
            next_command: 0,
            metadata: BTreeMap::new(),
        })
    }

    pub(super) fn reconfigure(&mut self, own_issi: u32) -> Result<(), LlcError> {
        if self.own_issi != own_issi {
            self.core.reset(LlcConfig { own_issi })?;
            for entry in self.metadata.values() {
                if let Some(reporter) = &entry.request.tx_reporter {
                    if reporter.is_transmitted() {
                        reporter.try_mark_lost();
                    } else {
                        reporter.try_mark_discarded();
                    }
                }
            }
            if !self.metadata.is_empty() {
                tracing::warn!("MS LLC identity changed: cancelled {} pending commands", self.metadata.len());
            }
            self.metadata.clear();
            self.own_issi = own_issi;
        }
        Ok(())
    }

    pub(super) fn tick_start(&mut self, time: TdmaTime) {
        self.now = time;
    }

    pub(super) fn receive(&mut self, queue: &mut MessageQueue, message: SapMsg) {
        let result = match message.msg {
            SapMsgInner::TlaTlDataReqBl(prim) => {
                let fcs = prim.fcs_flag;
                let request = TmaUnitdataReq {
                    req_handle: prim.req_handle,
                    pdu: prim.tl_sdu,
                    main_address: prim.main_address,
                    link_id: prim.link_id,
                    endpoint_id: prim.endpoint_id,
                    pdu_priority: prim.pdu_priority,
                    is_emergency: prim.is_emergency,
                    stealing_permission: prim.stealing_permission,
                    subscriber_class: prim.subscriber_class,
                    air_interface_encryption: prim.air_interface_encryption,
                    stealing_repeats_flag: prim.stealing_repeats_flag,
                    data_category: prim.data_class_info,
                    chan_alloc: prim.chan_alloc,
                    tx_reporter: prim.tx_reporter,
                };
                self.submit(request, Service::Acknowledged, fcs)
            }
            SapMsgInner::TlaTlUnitdataReqBl(prim) => {
                let request = TmaUnitdataReq {
                    req_handle: prim.req_handle,
                    pdu: prim.tl_sdu,
                    main_address: prim.main_address,
                    link_id: prim.link_id,
                    endpoint_id: prim.endpoint_id,
                    pdu_priority: None,
                    is_emergency: false,
                    stealing_permission: prim.stealing_permission,
                    subscriber_class: prim.subscriber_class,
                    air_interface_encryption: prim.air_interface_encryption,
                    stealing_repeats_flag: None,
                    data_category: prim.data_class_info,
                    chan_alloc: prim.chan_alloc,
                    tx_reporter: prim.tx_reporter,
                };
                self.submit(request, Service::Unacknowledged, false)
            }
            SapMsgInner::TmaUnitdataInd(prim) => {
                let result = prim.pdu.as_ref().ok_or(LlcError::InvalidPdu).and_then(remaining).and_then(|pdu| {
                    self.core.handle(
                        self.now,
                        &LlcEvent::Receive {
                            tag: 0,
                            address: prim.main_address,
                            pdu,
                        },
                    )
                });
                if result.is_ok() {
                    self.drain(queue, Some(prim));
                }
                result
            }
            SapMsgInner::TmaReportInd(_) => Ok(()), // MAC completion is carried by TxReporter, as before.
            _ => Err(LlcError::UnsupportedPdu(0xff)),
        };
        if let Err(error) = result {
            tracing::warn!(?error, "MS LLC rejected SAP input");
        }
    }

    fn submit(&mut self, mut request: TmaUnitdataReq, service: Service, fcs: bool) -> Result<(), LlcError> {
        let result = self.stage(&mut request, service, fcs);
        if result.is_err() {
            if let Some(reporter) = request.tx_reporter {
                reporter.try_mark_discarded();
            }
        }
        result
    }

    fn stage(&mut self, request: &mut TmaUnitdataReq, service: Service, fcs: bool) -> Result<(), LlcError> {
        let command = self.next_command.checked_add(1).ok_or(LlcError::GenerationExhausted)?;
        let sdu = remaining(&request.pdu)?;
        self.core.handle(
            self.now,
            &LlcEvent::Send(SendRequest {
                command,
                address: request.main_address,
                service,
                fcs,
                sdu,
            }),
        )?;
        self.next_command = command;
        if service == Service::Acknowledged && request.main_address.ssi_type != tetra_core::SsiType::Gssi && request.tx_reporter.is_none() {
            request.tx_reporter = Some(TxReporter::new());
        }
        request.pdu = BitBuffer::new(0);
        self.metadata.insert(
            command,
            Metadata {
                request: request.clone(),
                active: None,
                reported: false,
            },
        );
        Ok(())
    }

    pub(super) fn tick_end(&mut self, queue: &mut MessageQueue) -> bool {
        for entry in self.metadata.values_mut() {
            if entry.reported {
                continue;
            }
            let Some((receipt, mac)) = &entry.active else { continue };
            let feedback = match mac.get_state() {
                TxState::Transmitted => TxFeedback::Transmitted,
                TxState::Discarded => TxFeedback::Rejected,
                _ => continue,
            };
            match self.core.handle(
                self.now,
                &LlcEvent::Transmission {
                    receipt: *receipt,
                    feedback,
                },
            ) {
                Ok(()) => {
                    entry.reported = true;
                    if let Some(upper) = &entry.request.tx_reporter {
                        match feedback {
                            TxFeedback::Transmitted => {
                                upper.try_mark_transmitted();
                            }
                            TxFeedback::Rejected => {
                                upper.try_mark_discarded();
                            }
                            TxFeedback::Deferred => {}
                        }
                    }
                }
                Err(error) => tracing::error!(?error, "MS LLC receipt translation failed"),
            }
        }
        if let Err(error) = self.core.handle(self.now, &LlcEvent::Tick) {
            tracing::error!(?error, "MS LLC tick failed");
            return false;
        }
        self.drain(queue, None)
    }

    fn drain(&mut self, queue: &mut MessageQueue, mut downlink: Option<TmaUnitdataInd>) -> bool {
        let mut activity = false;
        while let Some(action) = self.core.poll_action() {
            activity = true;
            match action {
                LlcAction::Transmit {
                    command,
                    address,
                    receipt,
                    ack_slot,
                    group_data,
                    pdu,
                } => {
                    let request = if let Some(slot) = ack_slot {
                        let steal = matches!(slot, 2..=4);
                        let chan_alloc = if steal {
                            let mut timeslots = [false; 4];
                            timeslots[usize::from(slot - 1)] = true;
                            Some(CmceChanAllocReq {
                                usage: None,
                                timeslots,
                                alloc_type: ChanAllocType::Replace,
                                ul_dl_assigned: UlDlAssignment::Both,
                                carrier: None,
                            })
                        } else {
                            None
                        };
                        TmaUnitdataReq {
                            req_handle: 0,
                            pdu: desktop(&pdu),
                            main_address: address,
                            link_id: if steal { u32::from(slot) } else { 0 },
                            endpoint_id: 0,
                            pdu_priority: None,
                            is_emergency: false,
                            stealing_permission: steal,
                            subscriber_class: 0,
                            air_interface_encryption: None,
                            stealing_repeats_flag: None,
                            data_category: None,
                            chan_alloc,
                            tx_reporter: None,
                        }
                    } else {
                        let Some(entry) = self.metadata.get_mut(&command) else {
                            tracing::error!(command, "MS LLC transmit metadata missing");
                            continue;
                        };
                        let mut request = entry.request.clone();
                        request.pdu = desktop(&pdu);
                        if let Some(receipt) = receipt {
                            if let Some(upper) = &request.tx_reporter {
                                upper.reset();
                            }
                            let reporter = TxReporter::new();
                            request.tx_reporter = Some(reporter.clone());
                            entry.active = Some((receipt, reporter));
                            entry.reported = false;
                        } else {
                            self.metadata.remove(&command);
                        }
                        if group_data {
                            request.link_id = 0;
                        }
                        request
                    };
                    queue.push_back(SapMsg::new(
                        Sap::TmaSap,
                        TetraEntity::Llc,
                        TetraEntity::Umac,
                        SapMsgInner::TmaUnitdataReq(request),
                    ));
                }
                LlcAction::Completed { command, result } => {
                    let Some(entry) = self.metadata.remove(&command) else {
                        tracing::error!(command, "MS LLC completion metadata missing");
                        continue;
                    };
                    if let Some(reporter) = entry.request.tx_reporter {
                        match result {
                            Completion::Acknowledged => {
                                // MAC can complete and deliver the ACK before tick_end
                                // propagates its reporter. A genuinely pending MAC must
                                // retain the historical early-ACK behavior.
                                if entry
                                    .active
                                    .as_ref()
                                    .is_some_and(|(_, mac)| mac.get_state() == TxState::Transmitted)
                                {
                                    reporter.try_mark_transmitted();
                                }
                                if reporter.get_state() == TxState::Transmitted {
                                    reporter.mark_acknowledged();
                                }
                            }
                            Completion::RetryExhausted => {
                                reporter.try_mark_lost();
                                tracing::warn!(command, "MS LLC exhausted retransmissions");
                            }
                        }
                    }
                }
                LlcAction::Deliver { service, fcs, sdu, .. } => {
                    let Some(prim) = downlink.take() else {
                        tracing::error!("MS LLC downlink metadata missing");
                        continue;
                    };
                    let sdu = if sdu.is_empty() { None } else { Some(desktop(&sdu)) };
                    let message = match service {
                        Service::Acknowledged => SapMsgInner::TlaTlDataIndBl(TlaTlDataIndBl {
                            main_address: prim.main_address,
                            link_id: 0,
                            endpoint_id: prim.endpoint_id,
                            new_endpoint_id: prim.new_endpoint_id,
                            css_endpoint_id: prim.css_endpoint_id,
                            tl_sdu: sdu,
                            scrambling_code: prim.scrambling_code,
                            fcs_flag: fcs,
                            air_interface_encryption: prim.air_interface_encryption,
                            chan_change_resp_req: prim.chan_change_response_req,
                            chan_change_handle: prim.chan_change_handle,
                            chan_info: prim.chan_info,
                            req_handle: 0,
                        }),
                        Service::Unacknowledged => SapMsgInner::TlaTlUnitdataIndBl(TlaTlUnitdataIndBl {
                            main_address: prim.main_address,
                            link_id: 0,
                            endpoint_id: prim.endpoint_id,
                            new_endpoint_id: prim.new_endpoint_id,
                            css_endpoint_id: prim.css_endpoint_id,
                            tl_sdu: sdu,
                            scrambling_code: prim.scrambling_code,
                            fcs_flag: fcs,
                            air_interface_encryption: prim.air_interface_encryption,
                            chan_change_resp_req: prim.chan_change_response_req,
                            chan_change_handle: prim.chan_change_handle,
                            chan_info: prim.chan_info,
                            report: None,
                        }),
                    };
                    queue.push_back(SapMsg::new(Sap::TlaSap, TetraEntity::Llc, TetraEntity::Mle, message));
                }
            }
        }
        activity
    }
}
