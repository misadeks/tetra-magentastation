//! Portable TETRA logical-channel error-control algorithms.
//!
//! All bit slices use one bit per byte, except the explicitly packed CRC API.
//! Storage is fixed-size or caller-owned. This crate has no allocator, I/O,
//! sample processing, device, timing, protocol primitive, or cryptography dependency.
//!
//! [`errorcontrol`] checks input dimensions and bit values before writing output;
//! [`viterbi::ViterbiDecoder::decode_into`] reports insufficient caller storage.
//! The low-level CRC, puncturing, interleaving and encoder entry points retain
//! their existing preconditions and panic/truncation behavior.
//!
//! # Storage and scope
//!
//! Control blocks contain at most 268 type-1 bits, 288 type-2 bits and 432
//! coded bits. TCH/S uses 274 codec bits and 432 coded bits. Decoding requires
//! an explicit reusable [`errorcontrol::Workspace`] (2,880 bytes, alignment 2).
//! Put it in caller/core-owned static storage on constrained targets; initialize
//! once with its const constructor. For runtime-owned uninitialized storage,
//! use its in-place `initialize` method: constructing the multi-field workspace
//! by value can otherwise require a workspace-sized temporary. Decode calls
//! overwrite scratch in place without constructing another workspace.
//! Encoding still uses bounded local
//! arrays; target stack usage must be measured by the embedding application.
//!
//! A rate-1/N decoder stores 16*N bytes of polynomial lookup data. Its
//! caller-owned output and traceback require respectively one byte and two
//! bytes per decoded bit. [`viterbi::Workspace`] owns only the two-byte-per-bit
//! traceback, and [`viterbi::Sb1Workspace`] is 1,728 bytes with alignment 2.
//! [`viterbi::dec_sb1_with_workspace`] supports at most 864 decoded bits, requires
//! caller-owned workspace and converts hard bits during metric calculation,
//! without a soft-bit array. No portable decoding API implicitly constructs
//! large scratch. Legacy application signatures allocate their workspace outside
//! this crate.
//!
//! AACH retains the existing single-bit error correction, not full RM decoding.
//! Unimplemented data/linearization channels remain outside this crate; no
//! demodulator, equalizer, sample stream, SDR or hardware operation is added.
//!
//! ```
//! use tetra_phy_core::{errorcontrol, errorcontrol_params::BSCH_PARAMS};
//! let input = [0; 60];
//! let mut coded = [0; 120];
//! errorcontrol::encode_cp(&BSCH_PARAMS, &input, 3, &mut coded).unwrap();
//! let mut decoded = [0; 60];
//! // The application can instead place this caller-owned storage in a static.
//! let mut storage = core::mem::MaybeUninit::uninit();
//! let workspace = errorcontrol::Workspace::initialize(&mut storage);
//! assert!(errorcontrol::decode_cp(&BSCH_PARAMS, &coded, 3, &mut decoded, workspace).unwrap());
//! assert_eq!(decoded, input);
//! ```
#![no_std]
#![deny(unsafe_code)]

#[cfg(test)]
extern crate std;

pub mod convenc;
pub mod crc16;
pub mod errorcontrol;
pub mod errorcontrol_params;
pub mod interleaver;
pub mod rm3014;
pub mod scrambler;
pub mod tch_reorder;
pub mod viterbi;
