//! Application configuration, logging and SAP translation for the portable MS MLE.
//! Selection, survey, service and wire/routing decisions belong to MleCore.

use crate::{MessageQueue, TetraEntityTrait};
use tetra_config::magentastation::SharedConfig;
use tetra_core::{BitBuffer, Sap, TetraAddress, tetra_entities::TetraEntity};
use tetra_ms_core::mle::{MleAction, MleConfig, MleCore, MleError, MleEvent, Network, Protocol, UplinkSource, UpperLayer};
use tetra_ms_types::{BitView, PackedBits};
use tetra_saps::lcmc::{LcmcMleBreakInd, LcmcMleReopenInd, LcmcMleUnitdataInd};
use tetra_saps::lmm::{
    LmmMleActivateConf, LmmMleBreakInd, LmmMleReopenInd, LmmMleRssiInd, LmmMleScanCompleteInd, LmmMleScanResultInd, LmmMleUnitdataInd,
};
use tetra_saps::tla::TlaTlDataReqBl;
use tetra_saps::tlmc::{TlmcConfigureReq, TlmcTuneReq, TlmcUPlaneConfigureReq, TlmcValidAddress};
use tetra_saps::{SapMsg, SapMsgInner};

pub struct MleMs {
    config: SharedConfig,
    core: MleCore,
}

fn remaining(source: &BitBuffer) -> Result<BitView<'_>, MleError> {
    let offset = u16::try_from(source.get_pos()).map_err(|_| MleError::CapacityExceeded)?;
    let len = u16::try_from(source.get_len_remaining()).map_err(|_| MleError::CapacityExceeded)?;
    Ok(source.as_bit_view()?.window(offset, len)?)
}

fn desktop(source: &PackedBits) -> BitBuffer {
    let mut result = BitBuffer::from_bytes(source.as_bytes());
    result.set_raw_end(usize::from(source.len()));
    result
}

impl MleMs {
    pub fn new(config: SharedConfig) -> Self {
        Self {
            config,
            core: MleCore::new(),
        }
    }

    #[cfg(test)]
    const SURVEY_SYSINFO_SYNC_LIMIT: u8 = tetra_ms_core::mle::SURVEY_SYSINFO_SYNC_LIMIT;
    #[cfg(test)]
    const SURVEY_MONITOR_TICK_LIMIT: u16 = tetra_ms_core::mle::SURVEY_MONITOR_TICK_LIMIT;

    fn actions(&mut self, event: MleEvent<'_>) -> Vec<MleAction> {
        let cfg = self.config.config();
        let networks: Vec<_> = cfg.codeplug.networks.iter().map(|n| Network { mcc: n.mcc, mnc: n.mnc }).collect();
        let policy = MleConfig {
            home_network: Network {
                mcc: cfg.net.mcc,
                mnc: cfg.net.mnc,
            },
            allowed_networks: &networks,
            subscriber_class: cfg.ms.as_ref().map(|ms| ms.subscriber_class),
        };
        if let Err(error) = self.core.handle(policy, event) {
            tracing::warn!("MLE-MS: portable event rejected: {error:?}");
        }
        std::iter::from_fn(|| self.core.pop_action()).collect()
    }

    fn control(&mut self, queue: &mut MessageQueue, event: MleEvent<'_>) {
        for action in self.actions(event) {
            let (sap, dest, msg) = match action {
                MleAction::Tune { carrier_hz } => {
                    tracing::info!("MLE: requesting downlink retune to {carrier_hz} Hz");
                    (
                        Sap::TlmcSap,
                        TetraEntity::Umac,
                        SapMsgInner::TlmcTuneReq(TlmcTuneReq { carrier_hz }),
                    )
                }
                MleAction::ConfigureNetwork(network) => (
                    Sap::TlmcSap,
                    TetraEntity::Umac,
                    SapMsgInner::TlmcConfigureReq(TlmcConfigureReq {
                        valid_addresses: Some(TlmcValidAddress {
                            mcc: network.mcc,
                            mnc: network.mnc,
                            individual_ssi: None,
                            group_ssis: None,
                        }),
                        ..Default::default()
                    }),
                ),
                MleAction::ConfigureIdentities {
                    network,
                    issi,
                    groups,
                    group_count,
                } => (
                    Sap::TlmcSap,
                    TetraEntity::Umac,
                    SapMsgInner::TlmcConfigureReq(TlmcConfigureReq {
                        valid_addresses: Some(TlmcValidAddress {
                            mcc: network.mcc,
                            mnc: network.mnc,
                            individual_ssi: Some(issi),
                            group_ssis: Some(groups[..usize::from(group_count)].to_vec()),
                        }),
                        ..Default::default()
                    }),
                ),
                MleAction::UPlane { switch_u_plane, tx_grant } => (
                    Sap::TlmcSap,
                    TetraEntity::Umac,
                    SapMsgInner::TlmcUPlaneConfigureReq(TlmcUPlaneConfigureReq { switch_u_plane, tx_grant }),
                ),
                MleAction::Activate {
                    network,
                    location_area,
                    registration_required,
                    system_wide_services,
                    cell_type,
                } => (
                    Sap::LmmSap,
                    TetraEntity::Mm,
                    SapMsgInner::LmmMleActivateConf(LmmMleActivateConf {
                        mcc: network.mcc,
                        mnc: network.mnc,
                        la: location_area,
                        registration_required,
                        system_wide_services,
                        cell_type,
                    }),
                ),
                MleAction::Break {
                    layer: UpperLayer::Cmce,
                    permitted_services,
                } => (
                    Sap::LcmcSap,
                    TetraEntity::Cmce,
                    SapMsgInner::LcmcMleBreakInd(LcmcMleBreakInd {
                        permitted_services_in_ms_graceful_service_degradation_mode: permitted_services,
                    }),
                ),
                MleAction::Break { layer: UpperLayer::Mm, .. } => {
                    (Sap::LmmSap, TetraEntity::Mm, SapMsgInner::LmmMleBreakInd(LmmMleBreakInd {}))
                }
                MleAction::Reopen(UpperLayer::Cmce) => {
                    (Sap::LcmcSap, TetraEntity::Cmce, SapMsgInner::LcmcMleReopenInd(LcmcMleReopenInd {}))
                }
                MleAction::Reopen(UpperLayer::Mm) => (Sap::LmmSap, TetraEntity::Mm, SapMsgInner::LmmMleReopenInd(LmmMleReopenInd {})),
                MleAction::Rssi { rssi_dbfs } => (
                    Sap::LmmSap,
                    TetraEntity::Mm,
                    SapMsgInner::LmmMleRssiInd(LmmMleRssiInd { rssi_dbfs }),
                ),
                MleAction::ScanResult(cell) => (
                    Sap::LmmSap,
                    TetraEntity::Mm,
                    SapMsgInner::LmmMleScanResultInd(LmmMleScanResultInd {
                        carrier_hz: cell.carrier_hz,
                        mcc: cell.mcc,
                        mnc: cell.mnc,
                        location_area: cell.location_area,
                        colour_code: cell.colour_code,
                        rssi_dbfs: cell.rssi_dbfs,
                        registration_required: cell.registration_required,
                        late_entry_supported: cell.late_entry_supported,
                    }),
                ),
                MleAction::ScanComplete { found, scanned } => (
                    Sap::LmmSap,
                    TetraEntity::Mm,
                    SapMsgInner::LmmMleScanCompleteInd(LmmMleScanCompleteInd { found, scanned }),
                ),
                MleAction::Deliver { .. } | MleAction::Transmit { .. } => unreachable!("data action on a control event"),
            };
            queue.push_back(SapMsg {
                sap,
                src: TetraEntity::Mle,
                dest,
                msg,
            });
        }
    }

    fn rx_tla_prim(&mut self, queue: &mut MessageQueue, message: SapMsg) {
        let (sdu, address, endpoint_id, link_id) = match message.msg {
            SapMsgInner::TlaTlDataIndBl(mut prim) => (prim.tl_sdu.take(), prim.main_address, prim.endpoint_id, prim.link_id),
            SapMsgInner::TlaTlUnitdataIndBl(mut prim) => (prim.tl_sdu.take(), prim.main_address, prim.endpoint_id, prim.link_id),
            _ => panic!("unexpected TLA primitive"),
        };
        let Some(mut original_sdu) = sdu else {
            tracing::warn!("MLE-MS: downlink has no TL-SDU");
            return;
        };
        let Ok(bits) = remaining(&original_sdu) else {
            tracing::warn!("MLE-MS: downlink exceeds portable bit capacity");
            return;
        };
        let actions = self.actions(MleEvent::Downlink(bits));
        if let Some(action) = actions.into_iter().next() {
            let MleAction::Deliver {
                protocol,
                parameters,
                sdu: payload,
            } = action
            else {
                unreachable!("downlink action")
            };
            // Retain BitBuffer's original window/cursor representation for SAP
            // users, while the portable core owns discriminator consumption.
            let consumed = original_sdu.get_len_remaining() - usize::from(payload.len());
            original_sdu.seek_rel(consumed as isize);
            let sdu = original_sdu;
            let dest = match protocol.destination() {
                UpperLayer::Mm => TetraEntity::Mm,
                UpperLayer::Cmce => TetraEntity::Cmce,
            };
            let (sap, msg) = match protocol {
                Protocol::Mm => (
                    Sap::LmmSap,
                    SapMsgInner::LmmMleUnitdataInd(LmmMleUnitdataInd {
                        sdu,
                        handle: parameters.handle,
                        received_address: address,
                    }),
                ),
                Protocol::Cmce => (
                    Sap::LcmcSap,
                    SapMsgInner::LcmcMleUnitdataInd(LcmcMleUnitdataInd {
                        sdu,
                        handle: parameters.handle,
                        received_tetra_address: address,
                        endpoint_id,
                        link_id,
                        chan_change_resp_req: parameters.channel_change_response_required,
                        chan_change_handle: parameters.channel_change_handle,
                    }),
                ),
                Protocol::Sndcp => {
                    tracing::warn!("MLE-MS: unsupported SNDCP downlink dropped");
                    return;
                }
            };
            queue.push_back(SapMsg {
                sap,
                src: TetraEntity::Mle,
                dest,
                msg,
            });
        }
    }

    fn rx_tlmb_prim(&mut self, queue: &mut MessageQueue, message: SapMsg) {
        match message.msg {
            SapMsgInner::TlmbSyncInd(_) => self.rx_tlmb_tl_sync_ind(queue, message),
            SapMsgInner::TlmbSysinfoInd(_) => self.rx_tlmb_tl_sysinfo_ind(queue, message),
            SapMsgInner::TlmbMonitorInd(inner) => self.control(
                queue,
                MleEvent::Monitor {
                    downlink_available: inner.downlink_available,
                    rssi_dbfs: inner.rssi_dbfs,
                },
            ),
            SapMsgInner::TlmbScanDwellInd(_) => {
                let candidates = self.config.config().codeplug.scan_candidate_frequencies();
                self.control(queue, MleEvent::Dwell { candidates: &candidates });
            }
            _ => panic!("unexpected TLMB primitive"),
        }
    }

    #[cfg(test)]
    fn rx_tlmb_monitor_ind(&mut self, queue: &mut MessageQueue, downlink_available: bool) {
        self.control(
            queue,
            MleEvent::Monitor {
                downlink_available,
                rssi_dbfs: None,
            },
        );
    }

    pub fn rx_tlmb_tl_sync_ind(&mut self, queue: &mut MessageQueue, message: SapMsg) {
        let SapMsgInner::TlmbSyncInd(inner) = message.msg else {
            panic!("expected SYNC")
        };
        match remaining(&inner.tl_sdu) {
            Ok(bits) => self.control(queue, MleEvent::Sync(bits)),
            Err(error) => tracing::warn!("MLE-MS: invalid SYNC: {error:?}"),
        }
    }

    pub fn rx_tlmb_tl_sysinfo_ind(&mut self, queue: &mut MessageQueue, message: SapMsg) {
        let SapMsgInner::TlmbSysinfoInd(inner) = message.msg else {
            panic!("expected SYSINFO")
        };
        match remaining(&inner.tl_sdu) {
            Ok(bits) => self.control(queue, MleEvent::Sysinfo(bits)),
            Err(error) => tracing::warn!("MLE-MS: invalid SYSINFO: {error:?}"),
        }
    }

    fn rx_lmm_prim(&mut self, queue: &mut MessageQueue, message: SapMsg) {
        match message.msg {
            SapMsgInner::LmmMleUnitdataReq(prim) => {
                let source = UplinkSource::Mm {
                    priority: prim.pdu_priority,
                    emergency: prim.is_emergency,
                };
                let Ok(sdu) = remaining(&prim.sdu) else {
                    tracing::warn!("MLE-MS: MM uplink exceeds portable bit capacity");
                    return;
                };
                let actions = self.actions(MleEvent::Uplink {
                    source,
                    sdu,
                    endpoint_id: 0,
                    link_id: 0,
                });
                self.emit_uplink(queue, actions, prim.address, None, prim.tx_reporter);
            }
            SapMsgInner::LmmMleIdentitiesReq(prim) => self.control(
                queue,
                MleEvent::Identities {
                    issi: prim.issi,
                    attached_gssis: &prim.attached_gssis,
                },
            ),
            SapMsgInner::LmmMleSelectionModeReq(prim) => self.control(queue, MleEvent::SelectionMode { manual: prim.manual }),
            SapMsgInner::LmmMleScanReq(prim) => {
                let candidates = if prim.start {
                    self.config.config().codeplug.scan_candidate_frequencies()
                } else {
                    Vec::new()
                };
                self.control(
                    queue,
                    MleEvent::Survey {
                        start: prim.start,
                        candidates: &candidates,
                    },
                );
            }
            SapMsgInner::LmmMleCampReq(prim) => {
                let candidates = self.config.config().codeplug.scan_candidate_frequencies();
                self.control(
                    queue,
                    MleEvent::Camp {
                        carrier_hz: prim.carrier_hz,
                        register: prim.register,
                        candidates: &candidates,
                    },
                );
            }
            _ => panic!("unexpected LMM primitive"),
        }
    }

    fn emit_uplink(
        &self,
        queue: &mut MessageQueue,
        actions: Vec<MleAction>,
        address: TetraAddress,
        chan_alloc: Option<tetra_saps::lcmc::fields::chan_alloc_req::CmceChanAllocReq>,
        tx_reporter: Option<tetra_core::TxReporter>,
    ) {
        if let Some(action) = actions.into_iter().next() {
            let MleAction::Transmit { pdu, parameters } = action else {
                unreachable!("uplink action")
            };
            queue.push_back(SapMsg {
                sap: Sap::TlaSap,
                src: TetraEntity::Mle,
                dest: TetraEntity::Llc,
                msg: SapMsgInner::TlaTlDataReqBl(TlaTlDataReqBl {
                    main_address: address,
                    tl_sdu: desktop(&pdu),
                    endpoint_id: parameters.endpoint_id,
                    link_id: parameters.link_id,
                    pdu_priority: parameters.priority,
                    is_emergency: parameters.emergency,
                    stealing_permission: parameters.stealing_permission,
                    subscriber_class: parameters.subscriber_class,
                    fcs_flag: parameters.fcs,
                    air_interface_encryption: parameters.air_interface_encryption,
                    stealing_repeats_flag: parameters.stealing_repeats,
                    data_class_info: parameters.data_class,
                    req_handle: parameters.request_handle,
                    graceful_degradation: parameters.graceful_degradation,
                    chan_alloc,
                    tx_reporter,
                }),
            });
        } else if let Some(reporter) = &tx_reporter {
            reporter.try_mark_discarded();
        }
    }

    fn rx_lcmc_prim(&mut self, queue: &mut MessageQueue, message: SapMsg) {
        match message.msg {
            SapMsgInner::LcmcMleUnitdataReq(prim) => {
                let Ok(sdu) = remaining(&prim.sdu) else {
                    tracing::warn!("MLE-MS: CMCE uplink exceeds portable bit capacity");
                    return;
                };
                let actions = self.actions(MleEvent::Uplink {
                    source: UplinkSource::Cmce {
                        stealing_permission: prim.stealing_permission,
                    },
                    sdu,
                    endpoint_id: prim.endpoint_id,
                    link_id: prim.link_id,
                });
                self.emit_uplink(queue, actions, prim.main_address, prim.chan_alloc, prim.tx_reporter);
            }
            SapMsgInner::LcmcMleConfigureReq(prim) => self.control(
                queue,
                MleEvent::UPlane {
                    switch_u_plane: prim.switch_u_plane,
                    tx_grant: prim.tx_grant,
                },
            ),
            _ => panic!("unexpected LCMC primitive"),
        }
    }
}

impl TetraEntityTrait for MleMs {
    fn entity(&self) -> TetraEntity {
        TetraEntity::Mle
    }

    fn rx_prim(&mut self, queue: &mut MessageQueue, message: SapMsg) {
        match message.sap {
            Sap::TlaSap => self.rx_tla_prim(queue, message),
            Sap::TlmbSap => self.rx_tlmb_prim(queue, message),
            Sap::LmmSap => self.rx_lmm_prim(queue, message),
            Sap::LcmcSap => self.rx_lcmc_prim(queue, message),
            Sap::TlmcSap => unimplemented!("rx_tlmc_prim"),
            Sap::TlpdSap => unimplemented!("rx_tlpd_prim"),
            _ => panic!("unexpected MLE SAP"),
        }
    }
}

#[cfg(test)]
#[path = "mle_ms_tests.rs"]
mod tests;
