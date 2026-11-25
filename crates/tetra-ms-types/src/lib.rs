//! Platform-independent wire types. The default configuration needs no allocator.
#![no_std]
#![forbid(unsafe_code)]

#[cfg(test)]
extern crate std;

pub mod address;
pub mod bits;
pub mod frequency;
pub mod tdma_time;

pub use address::{SsiType, TetraAddress};
pub use bits::{BitError, BitStorage, BitView, BitViewMut, FragmentBits, MAX_PDU_BITS, MAX_PDU_BYTES, PackedBits};
pub use tdma_time::TdmaTime;
