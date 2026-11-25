//! Platform/SAP value conversion. No call-state or timer decisions live here.
use super::*;
use tetra_ms_core::cmce::{BasicService, CmceAction, CmceError, CmceEvent, CmceNotice, NoticeKind};
use tetra_ms_types::{BitView, PackedBits};

pub(super) fn basic_to_core(value: &BasicServiceInformation) -> Result<BasicService, CmceError> {
    let parameter = match (value.circuit_mode_type, value.slots_per_frame, value.speech_service) {
        (CircuitModeType::TchS, None, Some(speech)) => speech,
        (mode, Some(slots), None) if mode != CircuitModeType::TchS => slots,
        _ => return Err(CmceError::InvalidValue),
    };
    if parameter > 3 {
        return Err(CmceError::InvalidValue);
    }
    Ok(BasicService {
        circuit_mode: value.circuit_mode_type as u8,
        encrypted: value.encryption_flag,
        communication: value.communication_type as u8,
        parameter,
    })
}
pub(super) fn basic_from_core(value: BasicService) -> BasicServiceInformation {
    BasicServiceInformation {
        circuit_mode_type: CircuitModeType::try_from(u64::from(value.circuit_mode)).expect("portable mode"),
        encryption_flag: value.encrypted,
        communication_type: CommunicationType::try_from(u64::from(value.communication)).expect("portable communication type"),
        slots_per_frame: (value.circuit_mode != 0).then_some(value.parameter),
        speech_service: (value.circuit_mode == 0).then_some(value.parameter),
    }
}
fn timeout(value: u8) -> CallTimeout {
    CallTimeout::try_from(u64::from(value)).expect("portable call timeout")
}
fn setup(value: u8) -> CallTimeoutSetupPhase {
    CallTimeoutSetupPhase::try_from(u64::from(value)).expect("portable setup timeout")
}
fn grant(value: u8) -> TransmissionGrant {
    TransmissionGrant::try_from(u64::from(value)).expect("portable grant")
}
fn field(span: Option<tetra_ms_core::cmce::downlink::Span>, bits: BitView<'_>, id: u64) -> Option<Type3FieldGeneric> {
    let view = span?.view(bits).ok()?;
    let data = PackedBits::from_view(view).ok()?;
    Some(Type3FieldGeneric {
        field_id: id,
        len: usize::from(data.len()),
        data: data.as_bytes().to_vec(),
    })
}
fn bitbuffer(bits: &PackedBits) -> BitBuffer {
    let mut buffer = BitBuffer::new_autoexpand(usize::from(bits.len()));
    for offset in 0..bits.len() {
        buffer.write_bit(bits.view().read(offset, 1).expect("portable bits") as u8);
    }
    buffer.seek(0);
    buffer
}
impl CcMsSubentity {
    pub(super) fn dispatch(&mut self, queue: &mut MessageQueue, event: CmceEvent<'_>) -> bool {
        let accepted = match self.core.push_event(self.core.dltime, event) {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(?error, "CMCE event rejected");
                false
            }
        };
        self.drain_actions(queue);
        accepted
    }
    pub(super) fn drain_actions(&mut self, queue: &mut MessageQueue) {
        while let Some(action) = self.core.pop_action() {
            match action {
                CmceAction::Uplink {
                    sdu,
                    route,
                    stealing,
                    repeats,
                } => queue.push_back(SapMsg {
                    sap: Sap::LcmcSap,
                    src: TetraEntity::Cmce,
                    dest: TetraEntity::Mle,
                    msg: SapMsgInner::LcmcMleUnitdataReq(LcmcMleUnitdataReq {
                        sdu: bitbuffer(&sdu),
                        handle: route.handle,
                        endpoint_id: route.endpoint_id,
                        link_id: route.link_id,
                        layer2service: Layer2Service::Todo,
                        pdu_prio: 0,
                        layer2_qos: 0,
                        stealing_permission: stealing,
                        stealing_repeats_flag: repeats,
                        main_address: route.main_address,
                        chan_alloc: None,
                        tx_reporter: None,
                    }),
                }),
                CmceAction::Configure { route, basic, state, .. } => queue.push_back(SapMsg {
                    sap: Sap::LcmcSap,
                    src: TetraEntity::Cmce,
                    dest: TetraEntity::Mle,
                    msg: SapMsgInner::LcmcMleConfigureReq(LcmcMleConfigureReq {
                        endpoint_id: route.endpoint_id,
                        chan_change_accepted: None,
                        chan_change_handle: 0 as Todo,
                        call_release: if state.switch_u_plane { None } else { Some(0 as Todo) },
                        encryption_flag: basic.encrypted,
                        circuit_mode_type: basic_from_core(basic).circuit_mode_type,
                        add_temp_gssi: None,
                        del_temp_gssi: None,
                        simplex_duplex: state.simplex_duplex,
                        tx_grant: state.tx_grant,
                        switch_u_plane: state.switch_u_plane,
                    }),
                }),
                CmceAction::Notify(notice) => self.emit_notice(notice),
                CmceAction::Speech(_) => {}
                CmceAction::Call { change, id, new_id } => {
                    use tetra_ms_core::cmce::CallChange;
                    match change {
                        CallChange::Created | CallChange::Removed => {
                            self.audio.remove(&id);
                        }
                        CallChange::Rekeyed => {
                            let audio = self.audio.remove(&id);
                            self.audio.remove(&new_id);
                            if let Some(audio) = audio {
                                self.audio.insert(new_id, audio);
                            }
                        }
                    }
                }
            }
            self.audio.retain(|id, _| self.core.calls.contains_key(id));
        }
    }
    fn emit_notice(&self, notice: CmceNotice) {
        let p = notice.fields;
        let id = notice.id;
        let basic = basic_from_core(notice.basic);
        let offered = p.basic.and_then(|basic| tncc_basic_from_pdu(&basic_from_core(basic)));
        let notification = p.notification;
        let permission = tncc::TransmissionRequestPermission::from_bool(p.permission_denied);
        let external = field(p.external, notice.sdu.view(), 2);
        let event = match notice.kind {
            NoticeKind::Setup => {
                let Some(basic) = tncc_basic_from_pdu(&basic) else { return };
                TelemetryEvent::TnccSetupIndication {
                    call_identifier: id,
                    indication: Box::new(tncc::TnccSetupIndication {
                        basic_service_information: basic,
                        call_priority: p
                            .priority
                            .and_then(tncc::CallPriority::from_raw)
                            .unwrap_or(tncc::CallPriority::PriorityNotDefined),
                        call_time_out: tncc_call_timeout(timeout(p.timeout.unwrap_or(0))),
                        called_party_ssi: notice.called_party,
                        called_party_extension: None,
                        calling_party_ssi: p.speaker,
                        calling_party_extension: p.extension,
                        external_subscriber_number_calling: decode_external_subscriber_number(&external),
                        clir_control: None,
                        hook_method_selection: tncc::HookMethodSelection::from_bool(p.hook),
                        notification_indicator: notification,
                        simplex_duplex_selection: tncc::SimplexDuplexSelection::from_bool(p.duplex),
                        transmission_grant: tncc_transmission_grant(grant(notice.grant)),
                        transmission_request_permission: permission,
                    }),
                }
            }
            NoticeKind::Proceed => TelemetryEvent::TnccProceedIndication {
                call_identifier: id,
                indication: tncc::TnccProceedIndication {
                    basic_service_information_offered: offered,
                    call_status: p.call_status.and_then(tncc_call_status_raw),
                    hook_method: Some(tncc::HookMethodSelection::from_bool(p.hook)),
                    notification_indicator: notification,
                    simplex_duplex: Some(tncc::SimplexDuplexSelection::from_bool(p.duplex)),
                },
            },
            NoticeKind::Alert => TelemetryEvent::TnccAlertIndication {
                call_identifier: id,
                indication: tncc::TnccAlertIndication {
                    basic_service_information_offered: offered,
                    call_queued: Some(if p.queued {
                        tncc::CallQueued::CallIsQueued
                    } else {
                        tncc::CallQueued::CallIsNotQueued
                    }),
                    call_time_out_set_up_phase: tncc_setup_timeout(setup(p.setup_timeout.unwrap_or(0))),
                    notification_indicator: notification,
                    simplex_duplex: tncc::SimplexDuplexSelection::from_bool(p.duplex),
                },
            },
            NoticeKind::SetupConfirm => {
                let Some(basic) = tncc_basic_from_pdu(&basic) else { return };
                TelemetryEvent::TnccSetupConfirm {
                    call_identifier: id,
                    confirm: Box::new(tncc::TnccSetupConfirm {
                        basic_service_information: basic,
                        call_priority: p.priority.and_then(tncc::CallPriority::from_raw),
                        call_ownership: if p.ownership.unwrap_or(false) {
                            tncc::CallOwnership::ACallOwner
                        } else {
                            tncc::CallOwnership::NotACallOwner
                        },
                        call_amalgamation: tncc::CallAmalgamation::CallNotAmalgamated,
                        call_time_out: tncc_call_timeout(timeout(p.timeout.unwrap_or(0))),
                        hook_method_selection: tncc::HookMethodSelection::from_bool(p.hook),
                        notification_indicator: notification,
                        simplex_duplex_selection: tncc::SimplexDuplexSelection::from_bool(p.duplex),
                        transmission_grant: tncc_transmission_grant(grant(notice.grant)),
                        transmission_request_permission: permission,
                    }),
                }
            }
            NoticeKind::CompleteConfirm => TelemetryEvent::TnccCompleteConfirm {
                call_identifier: id,
                confirm: tncc::TnccCompleteConfirm {
                    call_time_out: tncc_call_timeout(timeout(notice.timeout)),
                    notification_indicator: notification,
                    transmission_grant: tncc_transmission_grant(grant(notice.grant)),
                    transmission_request_permission: permission,
                    transmission_status: tncc_transmission_status_from_grant(grant(notice.grant)),
                },
            },
            NoticeKind::TxConfirm => TelemetryEvent::TnccTxConfirm {
                call_identifier: id,
                confirm: tncc::TnccTxConfirm {
                    encryption_flag: tncc::EncryptionFlag::from_bool(p.encrypted),
                    transmit_request_permission: permission,
                    transmission_status: tncc_transmission_status_from_grant(grant(notice.grant)),
                },
            },
            NoticeKind::TxGranted | NoticeKind::TxCeased | NoticeKind::TxWait | NoticeKind::TxContinue | NoticeKind::TxInterrupt => {
                let floor = matches!(notice.kind, NoticeKind::TxGranted | NoticeKind::TxInterrupt);
                let status = match notice.kind {
                    NoticeKind::TxCeased => tncc::TransmissionStatus::TransmissionCeased,
                    NoticeKind::TxWait => tncc::TransmissionStatus::TransmissionWait,
                    NoticeKind::TxContinue if !p.resume => tncc::TransmissionStatus::TransmissionCeased,
                    NoticeKind::TxContinue => tncc::TransmissionStatus::TransmissionGranted,
                    NoticeKind::TxInterrupt => tncc::TransmissionStatus::TransmissionInterrupt,
                    _ => tncc_transmission_status_from_grant(grant(notice.grant)),
                };
                TelemetryEvent::TnccTxIndication {
                    call_identifier: id,
                    indication: tncc::TnccTxIndication {
                        encryption_flag: tncc::EncryptionFlag::from_bool(p.encrypted && notice.kind == NoticeKind::TxGranted),
                        notification_indicator: notification,
                        transmitting_party_ssi: if floor { notice.speaker } else { None },
                        transmitting_party_extension: if floor { p.extension } else { None },
                        external_subscriber_number: if floor {
                            decode_external_subscriber_number(&external)
                        } else {
                            None
                        },
                        transmit_request_permission: permission,
                        transmission_status: status,
                    },
                }
            }
            NoticeKind::ReleaseIndication => TelemetryEvent::TnccReleaseIndication {
                call_identifier: id,
                indication: tncc::TnccReleaseIndication {
                    disconnect_cause: tncc_disconnect_cause(DisconnectCause::try_from(u64::from(p.cause)).expect("portable cause")),
                    notification_indicator: notification,
                },
            },
            NoticeKind::ReleaseConfirm => TelemetryEvent::TnccReleaseConfirm {
                call_identifier: id,
                confirm: tncc::TnccReleaseConfirm {
                    disconnect_cause: tncc_disconnect_cause(DisconnectCause::try_from(u64::from(p.cause)).expect("portable cause")),
                    disconnect_status: tncc::DisconnectStatus::DisconnectionSuccessful,
                    notification_indicator: notification,
                },
            },
            NoticeKind::Info => {
                self.emit(TelemetryEvent::TnccNotifyIndication {
                    call_identifier: id,
                    indication: tncc::TnccNotifyIndication {
                        call_status: p.call_status.and_then(tncc_call_status_raw),
                        call_time_out_in_set_up_phase: p.setup_timeout.map(|v| tncc_setup_timeout(setup(v))),
                        call_time_out: p.timeout.map(|v| tncc_call_timeout(timeout(v))),
                        call_ownership: p.ownership.map(|v| {
                            if v {
                                tncc::CallOwnership::ACallOwner
                            } else {
                                tncc::CallOwnership::NotACallOwner
                            }
                        }),
                        notification_indicator: notification,
                        poll_response_percentage: p.poll_percentage,
                        poll_response_number: p.poll_number,
                        poll_response_addresses: None,
                        poll_request: Some(p.poll),
                    },
                });
                if let Some(indication) = field(p.dtmf, notice.sdu.view(), 1).as_ref().and_then(tncc_dtmf_indication_from_ie) {
                    self.emit(TelemetryEvent::TnccDtmfIndication {
                        call_identifier: id,
                        indication,
                    });
                }
                return;
            }
        };
        self.emit(event);
    }
}
