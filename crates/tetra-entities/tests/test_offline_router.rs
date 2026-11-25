use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, Ordering},
};

use tetra_config::magentastation::SharedConfig;
use tetra_core::{TdmaTime, tetra_entities::TetraEntity};
use tetra_entities::{MessageQueue, MessageRouter, TetraEntityTrait};
use tetra_saps::SapMsg;

#[path = "common/default_stack.rs"]
mod default_stack;

struct OfflineProbe {
    running: Arc<AtomicBool>,
    ticks: Arc<AtomicU32>,
    polls: Arc<AtomicU32>,
}

impl TetraEntityTrait for OfflineProbe {
    fn entity(&self) -> TetraEntity {
        TetraEntity::Mm
    }

    fn rx_prim(&mut self, _: &mut MessageQueue, _: SapMsg) {}

    fn tick_start(&mut self, _: &mut MessageQueue, _: TdmaTime) {
        self.ticks.fetch_add(1, Ordering::Relaxed);
    }

    fn tick_end(&mut self, _: &mut MessageQueue, _: TdmaTime) -> bool {
        self.ticks.fetch_add(1, Ordering::Relaxed);
        false
    }

    fn drive_offline_control(&mut self, _: &mut MessageQueue) {
        self.polls.fetch_add(1, Ordering::Relaxed);
        self.running.store(false, Ordering::Relaxed);
    }
}

#[test]
fn no_phy_services_offline_control_without_advancing_protocol_time() {
    let cfg = SharedConfig::from_parts(default_stack::default_test_config_ms(), None);
    let mut router = MessageRouter::new(cfg);
    let running = Arc::new(AtomicBool::new(true));
    let ticks = Arc::new(AtomicU32::new(0));
    let polls = Arc::new(AtomicU32::new(0));
    router.register_entity(Box::new(OfflineProbe {
        running: running.clone(),
        ticks: ticks.clone(),
        polls: polls.clone(),
    }));
    router.run_stack(None, Some(running));
    assert_eq!(polls.load(Ordering::Relaxed), 1);
    assert_eq!(ticks.load(Ordering::Relaxed), 0);
}
