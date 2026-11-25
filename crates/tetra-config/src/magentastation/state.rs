/// Mutable desktop state shared by MS entities.
#[derive(Debug, Clone, Default)]
pub struct StackState {
    /// Downlink frequency last camped on, used to bias re-acquisition.
    pub last_camped_freq_hz: Option<u32>,
}
