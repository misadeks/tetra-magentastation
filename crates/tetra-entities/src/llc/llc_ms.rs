//! Desktop entity boundary for the authoritative portable MS LLC implementation.
use tetra_config::magentastation::SharedConfig;
use tetra_core::{TdmaTime, tetra_entities::TetraEntity};
use tetra_saps::SapMsg;

use super::ms_adapter::MsLlcAdapter;
use crate::{MessageQueue, TetraEntityTrait};

pub struct Llc {
    adapter: MsLlcAdapter,
}

impl Llc {
    pub fn new(config: SharedConfig) -> Self {
        let own_issi = config.config().ms.as_ref().expect("validated MS configuration").issi;
        Self {
            adapter: MsLlcAdapter::new(own_issi).expect("validated MS ISSI"),
        }
    }
}

impl TetraEntityTrait for Llc {
    fn entity(&self) -> TetraEntity {
        TetraEntity::Llc
    }

    fn set_config(&mut self, config: SharedConfig) {
        let own_issi = config.config().ms.as_ref().expect("validated MS configuration").issi;
        if let Err(error) = self.adapter.reconfigure(own_issi) {
            tracing::error!(?error, "MS LLC reconfiguration rejected; previous configuration retained");
        }
    }

    fn rx_prim(&mut self, queue: &mut MessageQueue, message: SapMsg) {
        self.adapter.receive(queue, message);
    }

    fn tick_start(&mut self, _queue: &mut MessageQueue, time: TdmaTime) {
        self.adapter.tick_start(time);
    }

    fn tick_end(&mut self, queue: &mut MessageQueue, _time: TdmaTime) -> bool {
        self.adapter.tick_end(queue)
    }
}
