use super::*;
use tetra_ms_core::cmce::CmceEvent;
impl CcMsSubentity {
    pub fn new(telemetry: Option<TelemetrySink>) -> Self {
        Self {
            core: tetra_ms_core::cmce::CmceCore::new(),
            audio: std::collections::HashMap::new(),
            telemetry,
        }
    }
    pub fn new_with_config(config: SharedConfig, telemetry: Option<TelemetrySink>) -> Self {
        let mut entity = Self::new(telemetry);
        entity.set_config(config);
        entity
    }
    pub fn set_telemetry(&mut self, telemetry: Option<TelemetrySink>) {
        self.telemetry = telemetry;
    }
    pub(super) fn emit(&self, event: TelemetryEvent) {
        if let Some(sink) = &self.telemetry {
            sink.send(event);
        }
    }
    pub fn set_config(&mut self, config: SharedConfig) {
        let own = config.config().ms.as_ref().map(|ms| ms.issi);
        if let Err(error) = self.core.push_event(self.core.dltime, CmceEvent::Identity(own)) {
            tracing::warn!(?error, "invalid CMCE identity");
        }
    }
    pub fn call(&self, id: u16) -> Option<&MsCall> {
        self.core.calls.get(&id)
    }
    pub fn call_count(&self) -> usize {
        self.core.calls.len()
    }
    pub fn pending_origination_count(&self) -> usize {
        self.core.pending_originations.len()
    }
    pub fn handle_break(&mut self, queue: &mut MessageQueue) {
        self.dispatch(queue, CmceEvent::Break);
    }
    pub fn handle_reopen(&mut self, queue: &mut MessageQueue) {
        self.dispatch(queue, CmceEvent::Reopen);
    }
}
