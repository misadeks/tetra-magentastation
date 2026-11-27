use super::*;
impl CcMsSubentity {
    #[cfg(test)]
    pub(super) fn resolve_floor_grant(&self, grant: TransmissionGrant, speaker: Option<u32>) -> TransmissionGrant {
        TransmissionGrant::try_from(u64::from(
            tetra_ms_core::cmce::resolve_floor_grant(self.core.own_issi, grant as u8, speaker).unwrap(),
        ))
        .unwrap()
    }
    #[cfg(test)]
    pub(super) fn apply_transmission_grant(&mut self, queue: &mut MessageQueue, id: u16, grant: TransmissionGrant, speaker: Option<u32>) {
        self.core.apply_grant(id, grant as u8, speaker).expect("fixture grant");
        self.drain_actions(queue);
    }
    pub fn rx_downlink_traffic(&mut self, ts: u8, bfi: bool, _usage_marker: Option<u8>, owner: Option<u32>, data: &[u8]) {
        let Some(speech) = self.core.receive_traffic(owner) else { return };
        self.emit(TelemetryEvent::MsSpeechFrame {
            call_identifier: speech.id,
            timeslot: ts,
            sequence: speech.sequence,
            transmitting_party_ssi: speech.speaker,
            frame_bits: data.len() as u16,
            bad_frame: bfi,
            data: data.to_vec(),
        });
    }
    pub fn push_uplink_speech(&mut self, id: u16, frame_bits: u16, data: &[u8]) {
        let Some(call) = self.core.calls.get_mut(&id) else { return };
        if !call.accepts_uplink() {
            return;
        }
        let Some(data) = pack_tch_s_type1(frame_bits, data) else { return };
        let audio = self.audio.entry(id).or_default();
        if audio.frames.len() >= UPLINK_SOURCE_MAX_FRAMES {
            audio.frames.pop_front();
        }
        audio.frames.push_back(data);
    }
    pub fn drive_uplink_source(&mut self, queue: &mut MessageQueue) {
        let Some(id) = self.core.transmit_target() else { return };
        let Some(data) = self.audio.get_mut(&id).and_then(|audio| audio.frames.pop_front()) else {
            return;
        };
        self.core.transmitted(id).expect("portable transmit target");
        queue.push_back(SapMsg {
            sap: Sap::TmdSap,
            src: TetraEntity::Cmce,
            dest: TetraEntity::Umac,
            msg: SapMsgInner::TmdCircuitDataReq(TmdCircuitDataReq { ts: 0, data }),
        });
    }
}
pub(super) const UPLINK_SOURCE_MAX_FRAMES: usize = 4;
const TCH_S_TYPE1_BITS: usize = 274;
fn pack_tch_s_type1(frame_bits: u16, data: &[u8]) -> Option<Vec<u8>> {
    const PACKED_BYTES: usize = TCH_S_TYPE1_BITS.div_ceil(8);
    if data.len() == PACKED_BYTES {
        return Some(data.to_vec());
    }
    if usize::from(frame_bits) < TCH_S_TYPE1_BITS || data.len() < TCH_S_TYPE1_BITS {
        return None;
    }
    let mut packed = vec![0; PACKED_BYTES];
    for (offset, bit) in data.iter().take(TCH_S_TYPE1_BITS).enumerate() {
        if bit & 1 != 0 {
            packed[offset / 8] |= 1 << (7 - offset % 8);
        }
    }
    Some(packed)
}
