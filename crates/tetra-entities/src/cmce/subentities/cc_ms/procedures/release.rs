use super::*;
impl CcMsSubentity {
    pub fn disconnect_call(&mut self, queue: &mut MessageQueue, id: u16, cause: DisconnectCause) -> bool {
        self.dispatch(queue, tetra_ms_core::cmce::CmceEvent::Disconnect { id, cause: cause as u8 })
    }
    pub fn release_call(&mut self, queue: &mut MessageQueue, id: u16, cause: DisconnectCause) -> bool {
        self.dispatch(queue, tetra_ms_core::cmce::CmceEvent::Release { id, cause: cause as u8 })
    }
}
