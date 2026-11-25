//! Explicit unsupported packet-data boundary; no SNDCP service is implemented.
use crate::{MessageQueue, TetraEntityTrait};
use tetra_config::magentastation::SharedConfig;
use tetra_core::{Sap, tetra_entities::TetraEntity, unimplemented_log};
use tetra_saps::SapMsg;

pub struct Sndcp;

impl Sndcp {
    pub fn new(_config: SharedConfig) -> Self {
        Self
    }
}

impl TetraEntityTrait for Sndcp {
    fn entity(&self) -> TetraEntity {
        TetraEntity::Sndcp
    }

    fn rx_prim(&mut self, _queue: &mut MessageQueue, message: SapMsg) {
        assert_eq!(message.sap, Sap::TlpdSap);
        unimplemented_log!("SNDCP not implemented");
    }
}
