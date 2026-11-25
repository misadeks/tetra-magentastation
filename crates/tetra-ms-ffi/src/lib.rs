//! Versioned, caller-owned C ABI for the extracted MS basic-link component.
//!
//! # Safety
//! All non-null pointers must designate accessible memory of the declared size,
//! with the documented alignment and no concurrent access. Storage must be
//! initialized by `tetra_ms_init` before any other operation. Arbitrary invalid
//! addresses cannot be validated by a C ABI. Buffer overlap is rejected.
#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

pub mod cmce;
pub mod mac;
pub mod mle;
pub mod mm;
pub mod reassembly;
pub mod sds;

#[cfg(any(feature = "std", test))]
extern crate std;

/// Standalone C firmware has no Rust application to provide a panic handler.
/// With panic=abort the handler halts; it never unwinds into a C frame.
#[cfg(all(feature = "standalone", not(feature = "std"), target_os = "none"))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

use core::{
    ffi::c_void,
    mem::{align_of, size_of},
    ptr,
};
use tetra_ms_core::{
    llc::{Completion, LlcAction, LlcConfig, LlcCore, LlcError, LlcEvent, Receipt, SendRequest, Service, TxFeedback},
    types::{BitView, MAX_PDU_BITS, MAX_PDU_BYTES, SsiType, TdmaTime, TetraAddress, tdma_time::TIME_INT_WRAP},
};

pub const ABI_VERSION: u32 = 1;
pub const STORAGE_SIZE: u32 = 16_384;
pub const STORAGE_ALIGN: u32 = 4;
const MAGIC: u32 = 0x544d_5301;

pub const OK: i32 = 0;
pub const EMPTY: i32 = 1;
pub const ARGUMENT: i32 = -1;
pub const ABI_MISMATCH: i32 = -2;
pub const UNINITIALIZED: i32 = -3;
pub const INVALID_IDENTITY: i32 = -4;
pub const INVALID_TIME: i32 = -5;
pub const INVALID_PDU: i32 = -6;
pub const TRUNCATED: i32 = -7;
pub const INVALID_FCS: i32 = -8;
pub const UNSUPPORTED: i32 = -9;
pub const CAPACITY: i32 = -10;
pub const QUEUE_FULL: i32 = -11;
pub const RECEIPT_BUSY: i32 = -12;
pub const STALE_RECEIPT: i32 = -13;
pub const INVALID_TRANSITION: i32 = -14;
pub const GENERATION_EXHAUSTED: i32 = -15;
pub const PANIC: i32 = -16;

#[repr(C)]
pub struct Config {
    pub size: u32,
    pub abi_version: u32,
    pub own_issi: u32,
}

#[repr(C)]
pub struct Event {
    pub size: u32,
    pub abi_version: u32,
    /// 1 Send, 2 Receive, 3 Transmission, 4 Tick.
    pub kind: u32,
    /// Absolute TDMA slot in 0..283115520, not a wall-clock duration.
    pub now: u32,
    pub command: u32,
    pub ssi: u32,
    pub ssi_type: u32,
    /// 0 Acknowledged, 1 Unacknowledged.
    pub service: u32,
    pub fcs: u32,
    pub receipt_epoch: u32,
    pub receipt_sequence: u32,
    /// 0 Transmitted, 1 Rejected, 2 Deferred.
    pub feedback: u32,
    pub pdu_bits: u16,
    pub reserved: u16,
    pub pdu: [u8; MAX_PDU_BYTES as usize],
}

#[repr(C)]
pub struct Action {
    pub size: u32,
    pub abi_version: u32,
    /// 1 Transmit, 2 Deliver, 3 Completed.
    pub kind: u32,
    pub command: u32,
    pub ssi: u32,
    pub ssi_type: u32,
    /// Both zero when no receipt is attached.
    pub receipt_epoch: u32,
    pub receipt_sequence: u32,
    /// 0 Not an automatic ACK, 1..4 original message timeslot.
    pub ack_slot: u32,
    pub group_data: u32,
    pub service: u32,
    pub fcs: u32,
    /// 0 Acknowledged, 1 RetryExhausted (only for kind 3).
    pub completion: u32,
    pub pdu_bits: u16,
    pub reserved: u16,
    pub pdu: [u8; MAX_PDU_BYTES as usize],
}

#[repr(C)]
pub struct Status {
    pub size: u32,
    pub abi_version: u32,
    pub pending: u16,
    pub unacknowledged: u16,
    pub scheduled_acks: u16,
    pub actions: u16,
    pub links: u16,
    pub reserved: u16,
    pub ignored_acks: u32,
}

#[repr(C)]
struct State {
    magic: u32,
    core: LlcCore,
}

const _: () = {
    assert!(size_of::<State>() <= STORAGE_SIZE as usize);
    assert!(align_of::<State>() <= STORAGE_ALIGN as usize);
    assert!(size_of::<Config>() == 12);
    assert!(size_of::<Event>() == 388);
    assert!(size_of::<Action>() == 392);
    assert!(size_of::<Status>() == 24);
    assert!(align_of::<Event>() == 4);
    assert!(align_of::<Action>() == 4);
    assert!(core::mem::offset_of!(Event, pdu) == 52);
    assert!(core::mem::offset_of!(Action, pdu) == 56);
};

fn boundary(operation: impl FnOnce() -> Result<(), i32>) -> i32 {
    #[cfg(feature = "std")]
    {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)).map_or(PANIC, |result| result.err().unwrap_or(OK))
    }
    #[cfg(not(feature = "std"))]
    operation().err().unwrap_or(OK)
}

fn code(error: LlcError) -> i32 {
    match error {
        LlcError::InvalidIdentity => INVALID_IDENTITY,
        LlcError::InvalidTime => INVALID_TIME,
        LlcError::InvalidPdu => INVALID_PDU,
        LlcError::TruncatedPdu => TRUNCATED,
        LlcError::InvalidFcs => INVALID_FCS,
        LlcError::UnsupportedPdu(_) => UNSUPPORTED,
        LlcError::CapacityExceeded => CAPACITY,
        LlcError::QueueFull => QUEUE_FULL,
        LlcError::ReceiptBusy => RECEIPT_BUSY,
        LlcError::StaleReceipt => STALE_RECEIPT,
        LlcError::InvalidTransition => INVALID_TRANSITION,
        LlcError::GenerationExhausted => GENERATION_EXHAUSTED,
    }
}

fn valid_ptr<T>(pointer: *const T) -> Result<(), i32> {
    if pointer.is_null() || !(pointer as usize).is_multiple_of(align_of::<T>()) {
        return Err(ARGUMENT);
    }
    Ok(())
}

fn disjoint<T>(storage: *const c_void, other: *const T) -> Result<(), i32> {
    disjoint_region(storage, STORAGE_SIZE, other)
}

fn disjoint_region<T>(storage: *const c_void, storage_size: u32, other: *const T) -> Result<(), i32> {
    valid_ptr(other)?;
    let start = storage as usize;
    let end = start.checked_add(storage_size as usize).ok_or(ARGUMENT)?;
    let other_start = other as usize;
    let other_end = other_start.checked_add(size_of::<T>()).ok_or(ARGUMENT)?;
    if start < other_end && other_start < end {
        return Err(ARGUMENT);
    }
    Ok(())
}

unsafe fn state_ptr(storage: *mut c_void) -> Result<*mut State, i32> {
    let state = storage.cast::<State>();
    valid_ptr(state)?;
    // SAFETY: The caller guarantees a readable initialized storage region;
    // alignment and null were checked above. Read only the header before casting.
    if unsafe { ptr::addr_of!((*state).magic).read() } != MAGIC {
        return Err(UNINITIALIZED);
    }
    Ok(state)
}

fn config(config: &Config) -> Result<LlcConfig, i32> {
    if config.size != size_of::<Config>() as u32 || config.abi_version != ABI_VERSION {
        return Err(ABI_MISMATCH);
    }
    let result = LlcConfig { own_issi: config.own_issi };
    result.validate().map_err(code)?;
    Ok(result)
}

fn address(ssi: u32, kind: u32) -> Result<TetraAddress, i32> {
    let kind = match kind {
        0 => SsiType::Unknown,
        1 => SsiType::Ssi,
        2 => SsiType::Issi,
        3 => SsiType::Gssi,
        4 => SsiType::Ussi,
        5 => SsiType::Smi,
        6 => SsiType::Esi,
        7 => SsiType::EventLabel,
        _ => return Err(ARGUMENT),
    };
    Ok(TetraAddress::new(ssi, kind))
}

fn address_kind(kind: SsiType) -> u32 {
    match kind {
        SsiType::Unknown => 0,
        SsiType::Ssi => 1,
        SsiType::Issi => 2,
        SsiType::Gssi => 3,
        SsiType::Ussi => 4,
        SsiType::Smi => 5,
        SsiType::Esi => 6,
        SsiType::EventLabel => 7,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn tetra_ms_abi_version() -> u32 {
    ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn tetra_ms_core_size() -> u32 {
    STORAGE_SIZE
}

#[unsafe(no_mangle)]
pub extern "C" fn tetra_ms_core_align() -> u32 {
    STORAGE_ALIGN
}

/// # Safety
/// `storage` must own at least `storage_size` writable bytes, aligned as queried.
/// `input` must be a readable Config. Both regions must be disjoint.
/// Reinitialization is allowed only with exclusive ownership and invalidates receipts.
/// Use reset rather than init for a live instance to preserve stale-receipt protection.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_ms_init(storage: *mut c_void, storage_size: u32, input: *const Config) -> i32 {
    boundary(|| {
        valid_ptr(storage.cast::<State>())?;
        if storage_size < STORAGE_SIZE {
            return Err(CAPACITY);
        }
        disjoint(storage, input)?;
        // SAFETY: Input is aligned, non-null, disjoint and readable per caller contract.
        let config = config(unsafe { &*input })?;
        let state = storage.cast::<State>();
        // SAFETY: The caller owns enough aligned storage for State. A
        // MaybeUninit<LlcCore> reference requires no initialized core fields.
        // Initialize the core field directly, then publish the valid header.
        unsafe {
            let core = ptr::addr_of_mut!((*state).core).cast::<core::mem::MaybeUninit<LlcCore>>();
            LlcCore::init_in_place(&mut *core, config).map_err(code)?;
            ptr::addr_of_mut!((*state).magic).write(MAGIC);
        }
        Ok(())
    })
}

/// # Safety
/// Storage must have been initialized, be exclusively accessible and disjoint
/// from the readable, aligned Config.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_ms_reset(storage: *mut c_void, input: *const Config) -> i32 {
    boundary(|| {
        disjoint(storage, input)?;
        // SAFETY: Inputs satisfy the function's caller contract; helpers check headers/alignment.
        let config = config(unsafe { &*input })?;
        let state = unsafe { state_ptr(storage)? };
        // SAFETY: State was initialized and is uniquely owned for this call.
        unsafe { &mut (*state).core }.reset(config).map_err(code)
    })
}

/// # Safety
/// Storage must be initialized and exclusively accessible. Event must be a
/// readable aligned object disjoint from storage. No input is retained.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_ms_push_event(storage: *mut c_void, input: *const Event) -> i32 {
    boundary(|| {
        disjoint(storage, input)?;
        // SAFETY: Input is non-null/aligned and readable per caller contract.
        let input = unsafe { &*input };
        if input.size != size_of::<Event>() as u32 || input.abi_version != ABI_VERSION {
            return Err(ABI_MISMATCH);
        }
        if input.now >= TIME_INT_WRAP as u32 {
            return Err(INVALID_TIME);
        }
        if input.pdu_bits > MAX_PDU_BITS {
            return Err(CAPACITY);
        }
        if input.reserved != 0 || input.fcs > 1 {
            return Err(ARGUMENT);
        }
        let pdu = BitView::new(&input.pdu, input.pdu_bits).map_err(|_| TRUNCATED)?;
        let event = match input.kind {
            1 => LlcEvent::Send(SendRequest {
                command: input.command,
                address: address(input.ssi, input.ssi_type)?,
                service: match input.service {
                    0 => Service::Acknowledged,
                    1 => Service::Unacknowledged,
                    _ => return Err(ARGUMENT),
                },
                fcs: input.fcs != 0,
                sdu: pdu,
            }),
            2 => LlcEvent::Receive {
                tag: input.command,
                address: address(input.ssi, input.ssi_type)?,
                pdu,
            },
            3 => LlcEvent::Transmission {
                receipt: Receipt {
                    epoch: input.receipt_epoch,
                    sequence: input.receipt_sequence,
                },
                feedback: match input.feedback {
                    0 => TxFeedback::Transmitted,
                    1 => TxFeedback::Rejected,
                    2 => TxFeedback::Deferred,
                    _ => return Err(ARGUMENT),
                },
            },
            4 => LlcEvent::Tick,
            _ => return Err(UNSUPPORTED),
        };
        // SAFETY: Storage is initialized/exclusive per caller; helper checks its header.
        let state = unsafe { state_ptr(storage)? };
        unsafe { &mut (*state).core }
            .handle(TdmaTime::from_int(input.now as i32), &event)
            .map_err(code)
    })
}

/// # Safety
/// Storage must be initialized/exclusive; output must be writable and aligned,
/// disjoint from storage, with size and ABI fields initialized by the caller.
/// EMPTY leaves output untouched. All errors leave the action queued.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_ms_poll_action(storage: *mut c_void, output: *mut Action) -> i32 {
    boundary(|| {
        disjoint(storage, output)?;
        // SAFETY: Pointer is aligned/non-null and points to initialized headers.
        if unsafe { (*output).size } != size_of::<Action>() as u32 || unsafe { (*output).abi_version } != ABI_VERSION {
            return Err(ABI_MISMATCH);
        }
        // SAFETY: Storage is initialized and exclusively owned.
        let state = unsafe { state_ptr(storage)? };
        let Some(action) = unsafe { &mut (*state).core }.poll_action() else {
            return Err(EMPTY);
        };
        let mut result = Action {
            size: size_of::<Action>() as u32,
            abi_version: ABI_VERSION,
            kind: 0,
            command: 0,
            ssi: 0,
            ssi_type: 0,
            receipt_epoch: 0,
            receipt_sequence: 0,
            ack_slot: 0,
            group_data: 0,
            service: 0,
            fcs: 0,
            completion: 0,
            pdu_bits: 0,
            reserved: 0,
            pdu: [0; MAX_PDU_BYTES as usize],
        };
        match action {
            LlcAction::Transmit {
                command,
                address,
                receipt,
                ack_slot,
                group_data,
                pdu,
            } => {
                result.kind = 1;
                result.command = command;
                result.ssi = address.ssi;
                result.ssi_type = address_kind(address.ssi_type);
                if let Some(receipt) = receipt {
                    result.receipt_epoch = receipt.epoch;
                    result.receipt_sequence = receipt.sequence;
                }
                result.ack_slot = u32::from(ack_slot.unwrap_or(0));
                result.group_data = u32::from(group_data);
                result.pdu_bits = pdu.len();
                result.pdu[..pdu.as_bytes().len()].copy_from_slice(pdu.as_bytes());
            }
            LlcAction::Deliver { tag, service, fcs, sdu } => {
                result.kind = 2;
                result.command = tag;
                result.service = match service {
                    Service::Acknowledged => 0,
                    Service::Unacknowledged => 1,
                };
                result.fcs = u32::from(fcs);
                result.pdu_bits = sdu.len();
                result.pdu[..sdu.as_bytes().len()].copy_from_slice(sdu.as_bytes());
            }
            LlcAction::Completed {
                command,
                result: completion,
            } => {
                result.kind = 3;
                result.command = command;
                result.completion = match completion {
                    Completion::Acknowledged => 0,
                    Completion::RetryExhausted => 1,
                };
            }
        }
        // SAFETY: Output is writable, sufficiently sized/aligned and disjoint.
        unsafe { output.write(result) };
        Ok(())
    })
}

/// # Safety
/// Storage must be initialized/readable and not concurrently mutated; output
/// must be writable/aligned/disjoint with initialized size and ABI headers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_ms_get_status(storage: *const c_void, output: *mut Status) -> i32 {
    boundary(|| {
        disjoint(storage, output)?;
        // SAFETY: Pointer is aligned/non-null and headers are initialized by caller.
        if unsafe { (*output).size } != size_of::<Status>() as u32 || unsafe { (*output).abi_version } != ABI_VERSION {
            return Err(ABI_MISMATCH);
        }
        // SAFETY: Helper only reads; caller guarantees no concurrent mutable access.
        let state = unsafe { state_ptr(storage.cast_mut())? };
        let status = unsafe { &(*state).core }.status();
        let result = Status {
            size: size_of::<Status>() as u32,
            abi_version: ABI_VERSION,
            pending: status.pending,
            unacknowledged: status.unacknowledged,
            scheduled_acks: status.scheduled_acks,
            actions: status.actions,
            links: status.links,
            reserved: 0,
            ignored_acks: status.ignored_acks,
        };
        // SAFETY: Output is writable and disjoint from storage.
        unsafe { output.write(result) };
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[repr(C, align(4))]
    struct Storage([u8; STORAGE_SIZE as usize]);

    fn config() -> Config {
        Config {
            size: size_of::<Config>() as u32,
            abi_version: ABI_VERSION,
            own_issi: 1,
        }
    }

    fn event() -> Event {
        Event {
            size: size_of::<Event>() as u32,
            abi_version: ABI_VERSION,
            kind: 1,
            now: 0,
            command: 1,
            ssi: 1,
            ssi_type: 2,
            service: 0,
            fcs: 0,
            receipt_epoch: 0,
            receipt_sequence: 0,
            feedback: 0,
            pdu_bits: 5,
            reserved: 0,
            pdu: [0; MAX_PDU_BYTES as usize],
        }
    }

    #[test]
    fn initialization_layout_errors_and_in_place_reset() {
        let mut storage = Storage([0; STORAGE_SIZE as usize]);
        let pointer = storage.0.as_mut_ptr().cast();
        let cfg = config();
        let mut input = event();
        // SAFETY: All pointers refer to correctly sized, aligned, disjoint local objects.
        unsafe {
            assert_eq!(tetra_ms_push_event(pointer, &input), UNINITIALIZED);
            assert_eq!(tetra_ms_init(ptr::null_mut(), STORAGE_SIZE, &cfg), ARGUMENT);
            assert_eq!(tetra_ms_init(pointer, STORAGE_SIZE - 1, &cfg), CAPACITY);
            assert_eq!(tetra_ms_init(pointer, STORAGE_SIZE, ptr::null()), ARGUMENT);
            assert_eq!(tetra_ms_init(storage.0.as_mut_ptr().add(1).cast(), STORAGE_SIZE, &cfg), ARGUMENT);
            assert_eq!(tetra_ms_init(pointer, STORAGE_SIZE, &cfg), OK);
            assert_eq!(tetra_ms_push_event(pointer, &input), OK);
            input.size = 0;
            assert_eq!(tetra_ms_push_event(pointer, &input), ABI_MISMATCH);
            input.size = size_of::<Event>() as u32;
            input.pdu_bits = MAX_PDU_BITS + 1;
            assert_eq!(tetra_ms_push_event(pointer, &input), CAPACITY);
            input.pdu_bits = 5;
            input.now = TIME_INT_WRAP as u32;
            assert_eq!(tetra_ms_push_event(pointer, &input), INVALID_TIME);
            assert_eq!(tetra_ms_reset(pointer, &cfg), OK);
            input.now = 0;
            assert_eq!(tetra_ms_push_event(pointer, &input), OK);
            assert_eq!(tetra_ms_push_event(pointer, pointer.cast()), ARGUMENT);
        }
    }

    #[test]
    fn polling_validates_output_before_consuming() {
        let mut storage = Storage([0; STORAGE_SIZE as usize]);
        let pointer = storage.0.as_mut_ptr().cast();
        let mut input = event();
        let mut output = core::mem::MaybeUninit::<Action>::zeroed();
        // SAFETY: Action consists solely of integer fields/arrays, for which zero is valid.
        let output = unsafe { output.assume_init_mut() };
        // SAFETY: All objects are initialized, properly aligned and disjoint.
        unsafe {
            assert_eq!(tetra_ms_init(pointer, STORAGE_SIZE, &config()), OK);
            assert_eq!(tetra_ms_push_event(pointer, &input), OK);
            input.kind = 4;
            assert_eq!(tetra_ms_push_event(pointer, &input), OK);
            assert_eq!(tetra_ms_poll_action(pointer, output), ABI_MISMATCH);
            output.size = size_of::<Action>() as u32;
            output.abi_version = ABI_VERSION;
            assert_eq!(tetra_ms_poll_action(pointer, output), OK);
            assert_eq!(output.kind, 1);
            assert_eq!(output.pdu_bits, 10);
            assert_ne!(output.receipt_sequence, 0);
            assert_eq!(tetra_ms_poll_action(pointer, output), EMPTY);
            let receipt = (output.receipt_epoch, output.receipt_sequence);
            assert_eq!(tetra_ms_reset(pointer, &config()), OK);
            input.kind = 3;
            input.receipt_epoch = receipt.0;
            input.receipt_sequence = receipt.1;
            assert_eq!(tetra_ms_push_event(pointer, &input), STALE_RECEIPT);
        }
    }

    #[cfg(feature = "c-smoke")]
    #[test]
    fn linked_c_caller() {
        unsafe extern "C" {
            fn tetra_ms_c_smoke() -> i32;
        }
        // SAFETY: The compiled C test has no arguments and exercises the documented API.
        assert_eq!(unsafe { tetra_ms_c_smoke() }, 0);
    }

    #[cfg(feature = "std")]
    #[test]
    fn unwinding_is_converted_to_a_boundary_error() {
        assert_eq!(boundary(|| panic!("injected boundary failure")), PANIC);
    }
}
