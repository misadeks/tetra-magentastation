//! Caller-driven, allocation-free MS protocol components.
#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(test)]
extern crate std;

pub mod cmce;
pub mod fcs;
pub mod llc;
pub mod mac;
pub mod mle;
pub mod mm;
mod queue;
pub mod random_access;
pub mod reassembly;
pub mod sds;

pub use tetra_ms_types as types;

const COMPONENT_STATE_SIZE: usize = core::mem::size_of::<llc::LlcCore>()
    + core::mem::size_of::<mac::MsMac>()
    + core::mem::size_of::<random_access::MsRandomAccess>()
    + core::mem::size_of::<random_access::AccessParamStore>()
    + core::mem::size_of::<reassembly::MsReassembly>()
    + core::mem::size_of::<mle::MleCore>()
    + core::mem::size_of::<mm::MmCore>()
    + core::mem::size_of::<cmce::CmceCore>()
    + core::mem::size_of::<sds::SdsCore>();

/// Sum of default protocol-component objects, excluding caller configuration,
/// inter-object alignment, PHY workspace, actions in flight and platform state.
pub const DEFAULT_COMPONENT_STATE_BYTES: u32 = COMPONENT_STATE_SIZE as u32;

const _: () = assert!(COMPONENT_STATE_SIZE <= 65_536);

#[cfg(target_pointer_width = "32")]
const _: [(); 65_476] = [(); COMPONENT_STATE_SIZE];
