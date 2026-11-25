//! PDU vocabulary conversion only; portable CMCE owns emitted wire bits.
use super::*;
use tetra_ms_core::cmce::{BasicService, CmceError, Uplink, encode_uplink};
use tetra_ms_types::{BitView, PackedBits};

fn basic(value: &BasicServiceInformation) -> Result<BasicService, CmceError> {
    let parameter = match (value.circuit_mode_type, value.slots_per_frame, value.speech_service) {
        (CircuitModeType::TchS, None, Some(speech)) => speech,
        (mode, Some(slots), None) if mode != CircuitModeType::TchS => slots,
        _ => return Err(CmceError::InvalidValue),
    };
    Ok(BasicService {
        circuit_mode: value.circuit_mode_type as u8,
        encrypted: value.encryption_flag,
        communication: value.communication_type as u8,
        parameter,
    })
}

fn field(value: &Type3FieldGeneric) -> Result<BitView<'_>, CmceError> {
    let len = u16::try_from(value.len).map_err(|_| CmceError::InvalidValue)?;
    Ok(BitView::new(&value.data, len)?)
}

impl pdu::UplinkPdu for USetup {
    fn encode(&self) -> Result<PackedBits, CmceError> {
        if self.area_selection != 0
            || self.call_priority != 0
            || self.clir_control != 0
            || self.called_party_type_identifier != PartyTypeIdentifier::Ssi
            || self.called_party_short_number_address.is_some()
            || self.called_party_extension.is_some()
            || self.facility.is_some()
            || self.dm_ms_address.is_some()
            || self.proprietary.is_some()
        {
            return Err(CmceError::NotAllowed);
        }
        encode_uplink(Uplink::Setup {
            called: u32::try_from(self.called_party_ssi.ok_or(CmceError::InvalidValue)?).map_err(|_| CmceError::InvalidValue)?,
            basic: basic(&self.basic_service_information)?,
            hook: self.hook_method_selection,
            duplex: self.simplex_duplex_selection,
            request_tx: self.request_to_transmit_send_data,
            external: self.external_subscriber_number.as_ref().map(field).transpose()?,
        })
    }
}
impl pdu::UplinkPdu for UAlert {
    fn encode(&self) -> Result<PackedBits, CmceError> {
        if !self.reserved || self.facility.is_some() || self.proprietary.is_some() {
            return Err(CmceError::NotAllowed);
        }
        encode_uplink(Uplink::Alert {
            id: self.call_identifier,
            duplex: self.simplex_duplex_selection,
            basic: basic(self.basic_service_information.as_ref().ok_or(CmceError::InvalidValue)?)?,
        })
    }
}
impl pdu::UplinkPdu for UConnect {
    fn encode(&self) -> Result<PackedBits, CmceError> {
        if self.facility.is_some() || self.proprietary.is_some() {
            return Err(CmceError::NotAllowed);
        }
        encode_uplink(Uplink::Connect {
            id: self.call_identifier,
            hook: self.hook_method_selection,
            duplex: self.simplex_duplex_selection,
            basic: basic(self.basic_service_information.as_ref().ok_or(CmceError::InvalidValue)?)?,
        })
    }
}
impl pdu::UplinkPdu for UDisconnect {
    fn encode(&self) -> Result<PackedBits, CmceError> {
        if self.facility.is_some() || self.proprietary.is_some() {
            return Err(CmceError::NotAllowed);
        }
        encode_uplink(Uplink::Disconnect {
            id: self.call_identifier,
            cause: self.disconnect_cause as u8,
        })
    }
}
impl pdu::UplinkPdu for URelease {
    fn encode(&self) -> Result<PackedBits, CmceError> {
        if self.facility.is_some() || self.proprietary.is_some() {
            return Err(CmceError::NotAllowed);
        }
        encode_uplink(Uplink::Release {
            id: self.call_identifier,
            cause: self.disconnect_cause as u8,
        })
    }
}
impl pdu::UplinkPdu for UCallRestore {
    fn encode(&self) -> Result<PackedBits, CmceError> {
        if self.other_party_type_identifier != 1
            || self.other_party_short_number_address.is_some()
            || self.other_party_extension.is_some()
            || self.facility.is_some()
            || self.dm_ms_address.is_some()
            || self.proprietary.is_some()
        {
            return Err(CmceError::NotAllowed);
        }
        encode_uplink(Uplink::Restore {
            id: self.call_identifier,
            other_party: u32::try_from(self.other_party_ssi.ok_or(CmceError::InvalidValue)?).map_err(|_| CmceError::InvalidValue)?,
            request_tx: self.request_to_transmit_send_data,
            basic: basic(self.basic_service_information.as_ref().ok_or(CmceError::InvalidValue)?)?,
        })
    }
}
impl pdu::UplinkPdu for UInfo {
    fn encode(&self) -> Result<PackedBits, CmceError> {
        if self.poll_response || self.modify.is_some() || self.facility.is_some() || self.proprietary.is_some() {
            return Err(CmceError::NotAllowed);
        }
        encode_uplink(Uplink::Info {
            id: self.call_identifier,
            dtmf: field(self.dtmf.as_ref().ok_or(CmceError::InvalidValue)?)?,
        })
    }
}
impl pdu::UplinkPdu for UTxDemand {
    fn encode(&self) -> Result<PackedBits, CmceError> {
        if self.reserved || self.facility.is_some() || self.dm_ms_address.is_some() || self.proprietary.is_some() {
            return Err(CmceError::NotAllowed);
        }
        encode_uplink(Uplink::TxDemand {
            id: self.call_identifier,
            priority: self.tx_demand_priority,
            encrypted: self.encryption_control,
        })
    }
}
impl pdu::UplinkPdu for UTxCeased {
    fn encode(&self) -> Result<PackedBits, CmceError> {
        if self.facility.is_some() || self.dm_ms_address.is_some() || self.proprietary.is_some() {
            return Err(CmceError::NotAllowed);
        }
        encode_uplink(Uplink::TxCeased { id: self.call_identifier })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdu::UplinkPdu;

    #[test]
    fn portable_wire_matches_historical_codecs_for_entire_generated_uplink_set() {
        macro_rules! equivalent {
            ($pdu:expr) => {{
                let pdu = $pdu;
                let mut reference = BitBuffer::new_autoexpand(128);
                pdu.to_bitbuf(&mut reference).unwrap();
                let expected = PackedBits::from_view(reference.as_bit_view().unwrap()).unwrap();
                assert_eq!(pdu.encode().unwrap(), expected);
            }};
        }
        let basic = default_speech_basic_service();
        equivalent!(USetup {
            area_selection: 0,
            hook_method_selection: true,
            simplex_duplex_selection: false,
            basic_service_information: basic.clone(),
            request_to_transmit_send_data: true,
            call_priority: 0,
            clir_control: 0,
            called_party_type_identifier: PartyTypeIdentifier::Ssi,
            called_party_short_number_address: None,
            called_party_ssi: Some(0x123456),
            called_party_extension: None,
            external_subscriber_number: external_subscriber_number::encode("+12*#"),
            facility: None,
            dm_ms_address: None,
            proprietary: None,
        });
        equivalent!(UAlert {
            call_identifier: 0x1234,
            reserved: true,
            simplex_duplex_selection: true,
            basic_service_information: Some(basic.clone()),
            facility: None,
            proprietary: None,
        });
        for hook in [false, true] {
            equivalent!(UConnect {
                call_identifier: 0x1234,
                hook_method_selection: hook,
                simplex_duplex_selection: true,
                basic_service_information: Some(basic.clone()),
                facility: None,
                proprietary: None,
            });
        }
        equivalent!(UDisconnect {
            call_identifier: 0x1234,
            disconnect_cause: DisconnectCause::ExpiryOfTimer,
            facility: None,
            proprietary: None,
        });
        equivalent!(URelease {
            call_identifier: 0x1234,
            disconnect_cause: DisconnectCause::UserRequestedDisconnection,
            facility: None,
            proprietary: None,
        });
        equivalent!(UCallRestore {
            call_identifier: 0x1234,
            request_to_transmit_send_data: true,
            other_party_type_identifier: 1,
            other_party_short_number_address: None,
            other_party_ssi: Some(91),
            other_party_extension: None,
            basic_service_information: Some(basic),
            facility: None,
            dm_ms_address: None,
            proprietary: None,
        });
        equivalent!(UInfo {
            call_identifier: 0x1234,
            poll_response: false,
            modify: None,
            dtmf: dtmf::encode_tone_start_digits("12*#"),
            facility: None,
            proprietary: None,
        });
        equivalent!(UTxDemand {
            call_identifier: 0x1234,
            tx_demand_priority: 2,
            encryption_control: false,
            reserved: false,
            facility: None,
            dm_ms_address: None,
            proprietary: None,
        });
        equivalent!(UTxCeased {
            call_identifier: 0x1234,
            facility: None,
            dm_ms_address: None,
            proprietary: None
        });
    }

    #[test]
    fn invalid_wire_or_full_pending_storage_does_not_create_phantom_originations() {
        let mut cc = CcMsSubentity::new(None);
        cc.own_issi = Some(123);
        let mut queue = MessageQueue::new();
        let invalid = BasicServiceInformation {
            circuit_mode_type: CircuitModeType::TchS,
            encryption_flag: false,
            communication_type: CommunicationType::P2Mp,
            slots_per_frame: None,
            speech_service: None,
        };
        assert!(!cc.originate_group_call(&mut queue, 91, invalid, true));
        assert_eq!(cc.pending_origination_count(), 0);
        assert!(queue.pop_front().is_none());
        for _ in 0..16 {
            assert!(cc.originate_group_call(&mut queue, 91, default_speech_basic_service(), true));
        }
        assert!(!cc.originate_group_call(&mut queue, 91, default_speech_basic_service(), true));
        assert_eq!(cc.pending_origination_count(), 16);
        for _ in 0..16 {
            queue.pop_front().expect("accepted setup");
        }
        assert!(queue.pop_front().is_none());
    }
}
