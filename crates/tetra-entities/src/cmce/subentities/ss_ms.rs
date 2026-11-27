use tetra_saps::SapMsg;

use crate::MessageQueue;

/// Clause 12 Supplementary Services CMCE sub-entity
pub struct SsMsSubentity {}

impl Default for SsMsSubentity {
    fn default() -> Self {
        Self::new()
    }
}

impl SsMsSubentity {
    pub fn new() -> Self {
        SsMsSubentity {}
    }

    pub fn route_re_deliver(&mut self, _queue: &mut MessageQueue, _message: SapMsg) {
        tracing::warn!(error = ?tetra_ms_core::cmce::CmceError::Unsupported, "CMCE-MS: D-FACILITY supplementary service dropped");
    }
}
