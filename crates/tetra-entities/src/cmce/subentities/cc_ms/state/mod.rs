#[cfg(test)]
use super::*;
use std::collections::VecDeque;
pub use tetra_ms_core::cmce::{CallRoute, MsCallKind, MsCcState, MsTxGrantState, MsUPlaneState};
pub type MsCall = tetra_ms_core::cmce::CmceCall;
pub type MsCallTimers = tetra_ms_core::cmce::MsCallTimers<u8, u8>;

#[derive(Clone, Debug, Default)]
pub struct AudioSource {
    pub frames: VecDeque<Vec<u8>>,
}

#[cfg(test)]
pub trait MsCallFixture {
    fn new(
        id: u16,
        state: MsCcState,
        kind: MsCallKind,
        basic: BasicServiceInformation,
        duplex: bool,
        route: CallRoute,
        allowed: bool,
    ) -> Self;
}
#[cfg(test)]
impl MsCallFixture for MsCall {
    fn new(
        id: u16,
        state: MsCcState,
        kind: MsCallKind,
        basic: BasicServiceInformation,
        duplex: bool,
        route: CallRoute,
        allowed: bool,
    ) -> Self {
        let mut call = Self::create(
            id,
            state,
            adapter::basic_to_core(&basic).expect("fixture basic service"),
            duplex,
            route,
            allowed,
        )
        .expect("fixture call");
        call.kind = kind;
        call
    }
}
