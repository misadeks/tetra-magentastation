//! A board-independent, no_std adapter fragment. The firmware owns storage,
//! synchronization and transport; no board runtime or panic handler is imposed.
#![no_std]

use core::mem::MaybeUninit;

use tetra_ms_core::{
    llc::{LlcAction, LlcConfig, LlcCore, LlcError, LlcEvent, Receipt, TxFeedback},
    types::{BitView, TdmaTime, TetraAddress},
};

pub fn initialize(storage: &mut MaybeUninit<LlcCore>, own_issi: u32) -> Result<&mut LlcCore, LlcError> {
    LlcCore::init_in_place(storage, LlcConfig { own_issi })
}

/// Input bytes may be reused after this call. Drain every action before the
/// next input, retaining rejected/deferred transport requests in the adapter.
pub fn downlink(core: &mut LlcCore, now: TdmaTime, address: TetraAddress, bytes: &[u8], bits: u16) -> Result<(), LlcError> {
    let pdu = BitView::new(bytes, bits)?;
    core.handle(now, &LlcEvent::Receive { tag: 0, address, pdu })
}

/// Call when MAC actually transmits or rejects the receipt's specific attempt,
/// not merely when the board queues the request.
pub fn transmission(core: &mut LlcCore, now: TdmaTime, receipt: Receipt, feedback: TxFeedback) -> Result<(), LlcError> {
    core.handle(now, &LlcEvent::Transmission { receipt, feedback })
}

pub fn end_of_tick(core: &mut LlcCore, now: TdmaTime) -> Result<(), LlcError> {
    core.handle(now, &LlcEvent::Tick)
}

/// Execute or retain the returned request outside the protocol call stack.
pub fn next_action(core: &mut LlcCore) -> Option<LlcAction> {
    core.poll_action()
}
