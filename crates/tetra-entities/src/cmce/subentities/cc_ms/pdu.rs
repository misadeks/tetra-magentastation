use super::*;

#[cfg(test)]
pub(super) trait UplinkPdu {
    fn encode(&self) -> Result<tetra_ms_types::PackedBits, tetra_ms_core::cmce::CmceError>;
}

#[cfg(test)]
pub(super) fn default_speech_basic_service() -> BasicServiceInformation {
    BasicServiceInformation {
        circuit_mode_type: CircuitModeType::TchS,
        encryption_flag: false,
        communication_type: CommunicationType::P2Mp,
        slots_per_frame: None,
        speech_service: Some(0),
    }
}

pub(super) fn tncc_basic_from_pdu(basic: &BasicServiceInformation) -> Option<tncc::TnccBasicServiceInformation> {
    let communication_type = match basic.communication_type {
        CommunicationType::P2p => tncc::CommunicationType::PointToPoint,
        CommunicationType::P2Mp => tncc::CommunicationType::PointToMultipoint,
        CommunicationType::P2MpAcked => tncc::CommunicationType::PointToMultipointAcknowledged,
        CommunicationType::Broadcast => tncc::CommunicationType::Broadcast,
    };
    let encryption_flag = tncc::EncryptionFlag::from_bool(basic.encryption_flag);
    if basic.circuit_mode_type == CircuitModeType::TchS {
        let speech_service = match basic.speech_service? {
            0 => tncc::SpeechService::TetraEncodedOneTimeslotSpeech,
            3 => tncc::SpeechService::ProprietaryEncodedOneTimeslotSpeech,
            _ => return None,
        };
        Some(tncc::TnccBasicServiceInformation {
            circuit_mode_service: tncc::CircuitModeService::SpeechService,
            communication_type,
            data_service: None,
            data_call_capacity: None,
            encryption_flag,
            speech_service: Some(speech_service),
        })
    } else {
        let data_service = match basic.circuit_mode_type {
            CircuitModeType::Tch72 => tncc::DataService::Unprotected72KbitsNoInterleaving,
            CircuitModeType::Tch48n1 => tncc::DataService::LowProtection48KbitsShortInterleavingDepth1,
            CircuitModeType::Tch48n4 => tncc::DataService::LowProtection48KbitsMediumInterleavingDepth4,
            CircuitModeType::Tch48n8 => tncc::DataService::LowProtection48KbitsLongInterleavingDepth8,
            CircuitModeType::Tch24n1 => tncc::DataService::HighProtection24KbitsShortInterleavingDepth1,
            CircuitModeType::Tch24n4 => tncc::DataService::HighProtection24KbitsMediumInterleavingDepth4,
            CircuitModeType::Tch24n8 => tncc::DataService::HighProtection24KbitsLongInterleavingDepth8,
            CircuitModeType::TchS => return None,
        };
        let data_call_capacity = match basic.slots_per_frame? {
            0 => tncc::DataCallCapacity::OneTimeSlot,
            1 => tncc::DataCallCapacity::TwoTimeSlots,
            2 => tncc::DataCallCapacity::ThreeTimeSlots,
            3 => tncc::DataCallCapacity::FourTimeSlots,
            _ => return None,
        };
        Some(tncc::TnccBasicServiceInformation {
            circuit_mode_service: tncc::CircuitModeService::DataService,
            communication_type,
            data_service: Some(data_service),
            data_call_capacity: Some(data_call_capacity),
            encryption_flag,
            speech_service: None,
        })
    }
}

pub(super) fn pdu_basic_from_tncc(basic: &tncc::TnccBasicServiceInformation) -> Result<BasicServiceInformation, String> {
    let communication_type = match basic.communication_type {
        tncc::CommunicationType::PointToPoint => CommunicationType::P2p,
        tncc::CommunicationType::PointToMultipoint => CommunicationType::P2Mp,
        tncc::CommunicationType::PointToMultipointAcknowledged => CommunicationType::P2MpAcked,
        tncc::CommunicationType::Broadcast => CommunicationType::Broadcast,
    };
    match basic.circuit_mode_service {
        tncc::CircuitModeService::SpeechService => {
            let speech_service = match basic
                .speech_service
                .ok_or("speech_service is required for TNCC speech basic service")?
            {
                tncc::SpeechService::TetraEncodedOneTimeslotSpeech => 0,
                tncc::SpeechService::ProprietaryEncodedOneTimeslotSpeech => 3,
            };
            Ok(BasicServiceInformation {
                circuit_mode_type: CircuitModeType::TchS,
                encryption_flag: basic.encryption_flag.as_bool(),
                communication_type,
                slots_per_frame: None,
                speech_service: Some(speech_service),
            })
        }
        tncc::CircuitModeService::DataService => {
            let circuit_mode_type = match basic.data_service.ok_or("data_service is required for TNCC data basic service")? {
                tncc::DataService::Unprotected72KbitsNoInterleaving => CircuitModeType::Tch72,
                tncc::DataService::LowProtection48KbitsShortInterleavingDepth1 => CircuitModeType::Tch48n1,
                tncc::DataService::LowProtection48KbitsMediumInterleavingDepth4 => CircuitModeType::Tch48n4,
                tncc::DataService::LowProtection48KbitsLongInterleavingDepth8 => CircuitModeType::Tch48n8,
                tncc::DataService::HighProtection24KbitsShortInterleavingDepth1 => CircuitModeType::Tch24n1,
                tncc::DataService::HighProtection24KbitsMediumInterleavingDepth4 => CircuitModeType::Tch24n4,
                tncc::DataService::HighProtection24KbitsLongInterleavingDepth8 => CircuitModeType::Tch24n8,
            };
            let slots_per_frame = match basic
                .data_call_capacity
                .ok_or("data_call_capacity is required for TNCC data basic service")?
            {
                tncc::DataCallCapacity::OneTimeSlot => 0,
                tncc::DataCallCapacity::TwoTimeSlots => 1,
                tncc::DataCallCapacity::ThreeTimeSlots => 2,
                tncc::DataCallCapacity::FourTimeSlots => 3,
            };
            Ok(BasicServiceInformation {
                circuit_mode_type,
                encryption_flag: basic.encryption_flag.as_bool(),
                communication_type,
                slots_per_frame: Some(slots_per_frame),
                speech_service: None,
            })
        }
    }
}

pub(super) fn tncc_call_timeout(timeout: CallTimeout) -> tncc::CallTimeout {
    match timeout {
        CallTimeout::Infinite => tncc::CallTimeout::Infinite,
        CallTimeout::T30s => tncc::CallTimeout::Value1,
        CallTimeout::T45s => tncc::CallTimeout::Value2,
        CallTimeout::T60s => tncc::CallTimeout::Value3,
        CallTimeout::T2m => tncc::CallTimeout::Value4,
        CallTimeout::T3m => tncc::CallTimeout::Value5,
        CallTimeout::T4m => tncc::CallTimeout::Value6,
        CallTimeout::T5m => tncc::CallTimeout::Value7,
        CallTimeout::T6m => tncc::CallTimeout::Value8,
        CallTimeout::T8m => tncc::CallTimeout::Value9,
        CallTimeout::T10m => tncc::CallTimeout::Value10,
        CallTimeout::T12m => tncc::CallTimeout::Value11,
        CallTimeout::T15m => tncc::CallTimeout::Value12,
        CallTimeout::T20m => tncc::CallTimeout::Value13,
        CallTimeout::T30m => tncc::CallTimeout::Value14,
        CallTimeout::Reserved => tncc::CallTimeout::Value15,
    }
}

pub(super) fn tncc_setup_timeout(timeout: CallTimeoutSetupPhase) -> tncc::CallTimeoutSetupPhase {
    match timeout {
        CallTimeoutSetupPhase::Predefined => tncc::CallTimeoutSetupPhase::PreDefined,
        CallTimeoutSetupPhase::T1s => tncc::CallTimeoutSetupPhase::Value1,
        CallTimeoutSetupPhase::T2s => tncc::CallTimeoutSetupPhase::Value2,
        CallTimeoutSetupPhase::T5s => tncc::CallTimeoutSetupPhase::Value3,
        CallTimeoutSetupPhase::T10s => tncc::CallTimeoutSetupPhase::Value4,
        CallTimeoutSetupPhase::T20s => tncc::CallTimeoutSetupPhase::Value5,
        CallTimeoutSetupPhase::T30s => tncc::CallTimeoutSetupPhase::Value6,
        CallTimeoutSetupPhase::T60s => tncc::CallTimeoutSetupPhase::Value7,
    }
}

pub(super) fn tncc_transmission_grant(grant: TransmissionGrant) -> tncc::TransmissionGrant {
    match grant {
        TransmissionGrant::Granted => tncc::TransmissionGrant::TransmissionGranted,
        TransmissionGrant::NotGranted => tncc::TransmissionGrant::TransmissionNotGranted,
        TransmissionGrant::RequestQueued => tncc::TransmissionGrant::TransmissionRequestQueued,
        TransmissionGrant::GrantedToOtherUser => tncc::TransmissionGrant::TransmissionGrantedToAnotherUser,
    }
}

pub(super) fn tncc_transmission_status_from_grant(grant: TransmissionGrant) -> tncc::TransmissionStatus {
    match grant {
        TransmissionGrant::Granted => tncc::TransmissionStatus::TransmissionGranted,
        TransmissionGrant::NotGranted => tncc::TransmissionStatus::TransmissionNotGranted,
        TransmissionGrant::RequestQueued => tncc::TransmissionStatus::TransmissionRequestQueued,
        TransmissionGrant::GrantedToOtherUser => tncc::TransmissionStatus::TransmissionGrantedToAnotherUser,
    }
}

pub(super) fn tncc_call_status_raw(status: u8) -> Option<tncc::CallStatus> {
    Some(match status {
        0 => tncc::CallStatus::CallIsProgressing,
        1 => tncc::CallStatus::CallIsQueued,
        2 => tncc::CallStatus::RequestedSubscriberIsPaged,
        3 => tncc::CallStatus::CallContinue,
        4 => tncc::CallStatus::HangTimerHasExpired,
        _ => return None,
    })
}

/// Decode an External subscriber number type-3 element (cl. 14.8.20 / Table
/// 14.59) received in a downlink PDU into its dialled digit string, for
/// forwarding to the TN. `None` when the element is absent or malformed.
pub(super) fn decode_external_subscriber_number(field: &Option<Type3FieldGeneric>) -> Option<String> {
    let field = field.as_ref()?;
    if field.field_id != 2 {
        return None;
    }
    let bits = tetra_ms_types::BitView::new(&field.data, u16::try_from(field.len).ok()?).ok()?;
    let mut output = [0; 24];
    let len = tetra_ms_core::cmce::decode_external_number(bits, &mut output).ok()?;
    String::from_utf8(output[..usize::from(len)].to_vec()).ok()
}

/// Map a 4-bit DTMF digit code (Table 14.57) to its TNCC [`tncc::DtmfDigit`].
pub(super) fn dtmf_nibble_to_digit(nibble: u8) -> Option<tncc::DtmfDigit> {
    Some(match nibble {
        0x0 => tncc::DtmfDigit::Digit0,
        0x1 => tncc::DtmfDigit::Digit1,
        0x2 => tncc::DtmfDigit::Digit2,
        0x3 => tncc::DtmfDigit::Digit3,
        0x4 => tncc::DtmfDigit::Digit4,
        0x5 => tncc::DtmfDigit::Digit5,
        0x6 => tncc::DtmfDigit::Digit6,
        0x7 => tncc::DtmfDigit::Digit7,
        0x8 => tncc::DtmfDigit::Digit8,
        0x9 => tncc::DtmfDigit::Digit9,
        0xA => tncc::DtmfDigit::DigitStar,
        0xB => tncc::DtmfDigit::DigitHash,
        0xC => tncc::DtmfDigit::DigitA,
        0xD => tncc::DtmfDigit::DigitB,
        0xE => tncc::DtmfDigit::DigitC,
        0xF => tncc::DtmfDigit::DigitD,
        _ => return None,
    })
}

/// Build a TNCC-DTMF indication (Table 11.3, cl. 11.3.3.3) from a received DTMF
/// type-3 element (cl. 14.8.19, Table 14.58). Returns `None` when the element
/// does not decode or carries a reserved/unrecognised DTMF type, so an
/// unexpected downlink DTMF is dropped rather than surfaced as a malformed
/// indication.
pub(super) fn tncc_dtmf_indication_from_ie(field: &Type3FieldGeneric) -> Option<tncc::TnccDtmfIndication> {
    if field.field_id != 1 {
        return None;
    }
    let bits = tetra_ms_types::BitView::new(&field.data, u16::try_from(field.len).ok()?).ok()?;
    let ie = tetra_ms_core::cmce::decode_dtmf(bits).ok()?;
    match ie.kind {
        dtmf::DTMF_TYPE_TONE_START => {
            let digits: Vec<tncc::DtmfDigit> = (0..ie.digits.len() / 4)
                .map(|index| dtmf_nibble_to_digit(ie.digits.read(index * 4, 4).ok()? as u8))
                .collect::<Option<_>>()?;
            if digits.is_empty() {
                return None;
            }
            Some(tncc::TnccDtmfIndication {
                dtmf_tone_delimiter: Some(tncc::DtmfToneDelimiter::Dtmf),
                dtmf_result: None,
                number_of_dtmf_digits: Some(digits.len() as u8),
                dtmf_digits: Some(digits),
            })
        }
        dtmf::DTMF_TYPE_TONE_END => Some(tncc::TnccDtmfIndication {
            dtmf_tone_delimiter: Some(tncc::DtmfToneDelimiter::ToneEnd),
            dtmf_result: None,
            number_of_dtmf_digits: None,
            dtmf_digits: None,
        }),
        dtmf::DTMF_TYPE_NOT_SUPPORTED => Some(tncc::TnccDtmfIndication {
            dtmf_tone_delimiter: None,
            dtmf_result: Some(tncc::DtmfResult::DtmfNotSupported),
            number_of_dtmf_digits: None,
            dtmf_digits: None,
        }),
        dtmf::DTMF_TYPE_NOT_SUBSCRIBED => Some(tncc::TnccDtmfIndication {
            dtmf_tone_delimiter: None,
            dtmf_result: Some(tncc::DtmfResult::DtmfNotSubscribed),
            number_of_dtmf_digits: None,
            dtmf_digits: None,
        }),
        _ => None,
    }
}

pub(super) fn tncc_disconnect_cause(cause: DisconnectCause) -> tncc::DisconnectCause {
    match cause {
        DisconnectCause::CauseNotDefinedOrUnknown => tncc::DisconnectCause::CauseNotDefinedOrUnknown,
        DisconnectCause::UserRequestedDisconnection => tncc::DisconnectCause::UserRequestedDisconnection,
        DisconnectCause::NonCallOwnerRequestedDisconnection => tncc::DisconnectCause::NonCallOwnerRequestedDisconnection,
        DisconnectCause::CalledPartyBusy => tncc::DisconnectCause::CalledPartyBusy,
        DisconnectCause::CalledPartyNotReachable => tncc::DisconnectCause::CalledPartyNotReachable,
        DisconnectCause::CalledPartyDoesNotSupportEncryption => tncc::DisconnectCause::CalledPartyDoesNotSupportEncryption,
        DisconnectCause::CongestionInInfrastructure => tncc::DisconnectCause::CongestionInInfrastructure,
        DisconnectCause::NotAllowedTrafficCase => tncc::DisconnectCause::NotAllowedTrafficCase,
        DisconnectCause::IncompatibleTrafficCase => tncc::DisconnectCause::IncompatibleTrafficCase,
        DisconnectCause::RequestedServiceNotAvailable => tncc::DisconnectCause::RequestedServiceNotAvailable,
        DisconnectCause::PreEmptiveUseOfResource => tncc::DisconnectCause::PreEmptiveUseOfResource,
        DisconnectCause::InvalidCallIdentifier => tncc::DisconnectCause::InvalidCallIdentifier,
        DisconnectCause::CallRejectedByTheCalledParty => tncc::DisconnectCause::CallRejectedByTheCalledParty,
        DisconnectCause::NoIdleCcEntity => tncc::DisconnectCause::NoIdleCcEntity,
        DisconnectCause::ExpiryOfTimer => tncc::DisconnectCause::ExpiryOfTimer,
        DisconnectCause::SwmiRequestedDisconnection => tncc::DisconnectCause::SwmiRequestedDisconnection,
        DisconnectCause::AcknowledgedServiceNotComplete => tncc::DisconnectCause::AcknowledgedServiceNotCompleted,
        DisconnectCause::CalledPartyRequiresEncryption => tncc::DisconnectCause::CalledPartyRequiresEncryption,
        DisconnectCause::ConcurrentSetUpNotSupported => tncc::DisconnectCause::ConcurrentSetUpNotSupported,
        DisconnectCause::CalledPartyIsUnderTheSameDmGateOfTheCallingParty => {
            tncc::DisconnectCause::CalledPartyIsUnderTheSameDmGateOfTheCallingParty
        }
        DisconnectCause::UnknownTetraIdentity
        | DisconnectCause::SsSpecificDisconnection
        | DisconnectCause::UnknownExternalSubscriberIdentity
        | DisconnectCause::CallRestorationOfTheOtherUserFailed => tncc::DisconnectCause::CauseNotDefinedOrUnknown,
    }
}

pub(super) fn pdu_disconnect_cause_from_tncc(cause: tncc::DisconnectCause) -> Result<DisconnectCause, String> {
    Ok(match cause {
        tncc::DisconnectCause::CauseNotDefinedOrUnknown => DisconnectCause::CauseNotDefinedOrUnknown,
        tncc::DisconnectCause::UserRequestedDisconnection => DisconnectCause::UserRequestedDisconnection,
        tncc::DisconnectCause::NonCallOwnerRequestedDisconnection => DisconnectCause::NonCallOwnerRequestedDisconnection,
        tncc::DisconnectCause::CalledPartyBusy => DisconnectCause::CalledPartyBusy,
        tncc::DisconnectCause::CalledPartyNotReachable => DisconnectCause::CalledPartyNotReachable,
        tncc::DisconnectCause::CalledPartyDoesNotSupportEncryption => DisconnectCause::CalledPartyDoesNotSupportEncryption,
        tncc::DisconnectCause::CongestionInInfrastructure => DisconnectCause::CongestionInInfrastructure,
        tncc::DisconnectCause::NotAllowedTrafficCase => DisconnectCause::NotAllowedTrafficCase,
        tncc::DisconnectCause::IncompatibleTrafficCase => DisconnectCause::IncompatibleTrafficCase,
        tncc::DisconnectCause::RequestedServiceNotAvailable => DisconnectCause::RequestedServiceNotAvailable,
        tncc::DisconnectCause::PreEmptiveUseOfResource => DisconnectCause::PreEmptiveUseOfResource,
        tncc::DisconnectCause::InvalidCallIdentifier => DisconnectCause::InvalidCallIdentifier,
        tncc::DisconnectCause::CallRejectedByTheCalledParty => DisconnectCause::CallRejectedByTheCalledParty,
        tncc::DisconnectCause::NoIdleCcEntity => DisconnectCause::NoIdleCcEntity,
        tncc::DisconnectCause::ExpiryOfTimer => DisconnectCause::ExpiryOfTimer,
        tncc::DisconnectCause::SwmiRequestedDisconnection => DisconnectCause::SwmiRequestedDisconnection,
        tncc::DisconnectCause::AcknowledgedServiceNotCompleted => DisconnectCause::AcknowledgedServiceNotComplete,
        tncc::DisconnectCause::CalledPartyRequiresEncryption => DisconnectCause::CalledPartyRequiresEncryption,
        tncc::DisconnectCause::ConcurrentSetUpNotSupported => DisconnectCause::ConcurrentSetUpNotSupported,
        tncc::DisconnectCause::CalledPartyIsUnderTheSameDmGateOfTheCallingParty => {
            DisconnectCause::CalledPartyIsUnderTheSameDmGateOfTheCallingParty
        }
        tncc::DisconnectCause::LossOfResources | tncc::DisconnectCause::UsageMarkerFailure => {
            return Err("TNCC disconnect cause has no Phase-1 U-RELEASE/U-DISCONNECT mapping".to_string());
        }
    })
}
