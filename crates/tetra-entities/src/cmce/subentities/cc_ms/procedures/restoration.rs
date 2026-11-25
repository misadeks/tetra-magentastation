use super::*;
impl CcMsSubentity {
    pub fn request_call_restore(&mut self, queue: &mut MessageQueue, id: u16) -> bool {
        self.dispatch(queue, tetra_ms_core::cmce::CmceEvent::Restore { id })
    }
}
