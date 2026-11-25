use super::*;
impl CcMsSubentity {
    pub fn tick_start(&mut self, queue: &mut MessageQueue, time: TdmaTime) {
        if let Err(error) = self.core.push_event(time, tetra_ms_core::cmce::CmceEvent::Tick) {
            tracing::warn!(?error, "CMCE tick rejected");
        }
        self.drain_actions(queue);
    }
}
#[cfg(test)]
pub(super) fn seconds_to_timeslots(seconds: i32) -> i32 {
    tetra_ms_core::cmce::seconds_to_timeslots(seconds)
}
#[cfg(test)]
pub(super) fn setup_timeout_to_timeslots(value: CallTimeoutSetupPhase) -> Option<i32> {
    tetra_ms_core::cmce::setup_timeout_to_timeslots(value.into_raw() as u8)
}
#[cfg(test)]
pub(super) fn call_timeout_to_timeslots(value: CallTimeout) -> Option<i32> {
    tetra_ms_core::cmce::call_timeout_to_timeslots(value.into_raw() as u8)
}
