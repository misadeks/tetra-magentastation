//! Caller-owned MAC reassembly ABI; Rust layout remains private.
use super::{
    ABI_MISMATCH, ABI_VERSION, ARGUMENT, CAPACITY, EMPTY, GENERATION_EXHAUSTED, INVALID_IDENTITY, INVALID_TIME, INVALID_TRANSITION,
    UNINITIALIZED, address, address_kind, boundary, disjoint_region, valid_ptr,
};
use core::{
    ffi::c_void,
    mem::{MaybeUninit, align_of, size_of},
    ptr,
};
use tetra_ms_core::{
    reassembly::{FragmentError, MsReassembly},
    types::{BitView, TdmaTime, tdma_time::TIME_INT_WRAP},
};

pub const STORAGE_SIZE: u32 = 2304;
pub const REPLACED: i32 = 2;
pub const EXPIRED_MASK: i32 = 0x100;
pub const OVERFLOW_ABORTED: i32 = -17;
const MAGIC: u32 = 0x4d52_4101;

#[repr(C)]
pub struct Event {
    pub size: u32,
    pub abi_version: u32,
    pub kind: u32,
    pub now: u32,
    pub ssi: u32,
    pub ssi_type: u32,
    pub aie_valid: u32,
    pub aie_info: i32,
    pub pdu_bits: u16,
    pub reserved: u16,
    pub pdu: [u8; 512],
}

#[repr(C)]
pub struct Action {
    pub size: u32,
    pub abi_version: u32,
    pub ssi: u32,
    pub ssi_type: u32,
    pub first: u32,
    pub last: u32,
    pub aie_valid: u32,
    pub aie_info: i32,
    pub pdu_bits: u16,
    pub fragments: u16,
    pub pdu: [u8; 512],
}

#[repr(C)]
struct State {
    magic: u32,
    core: MsReassembly,
}

const _: () = {
    assert!(size_of::<State>() <= STORAGE_SIZE as usize);
    assert!(align_of::<State>() == 4);
    assert!(size_of::<Event>() == 548);
    assert!(size_of::<Action>() == 548);
    assert!(core::mem::offset_of!(Event, pdu) == 36);
    assert!(core::mem::offset_of!(Action, pdu) == 36);
};

fn time(raw: u32) -> Result<TdmaTime, i32> {
    if raw >= TIME_INT_WRAP as u32 {
        return Err(INVALID_TIME);
    }
    Ok(TdmaTime::from_int(raw as i32))
}

fn code(error: FragmentError) -> i32 {
    match error {
        FragmentError::InvalidTime => INVALID_TIME,
        FragmentError::InvalidAddress => INVALID_IDENTITY,
        FragmentError::NotActive | FragmentError::NotComplete => INVALID_TRANSITION,
        FragmentError::CapacityExceeded => CAPACITY,
        FragmentError::OverflowAborted => OVERFLOW_ABORTED,
        FragmentError::CounterExhausted => GENERATION_EXHAUSTED,
    }
}

unsafe fn state(storage: *mut c_void) -> Result<*mut State, i32> {
    let out = storage.cast::<State>();
    valid_ptr(out)?;
    // SAFETY: The C contract requires live initialized storage before this call.
    if unsafe { ptr::addr_of!((*out).magic).read() } != MAGIC {
        return Err(UNINITIALIZED);
    }
    Ok(out)
}

#[unsafe(no_mangle)]
pub extern "C" fn tetra_ms_reassembly_size() -> u32 {
    STORAGE_SIZE
}

/// # Safety
/// Storage must designate at least size bytes of live, aligned, exclusive memory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_ms_reassembly_init(storage: *mut c_void, size: u32) -> i32 {
    boundary(|| {
        let out = storage.cast::<State>();
        valid_ptr(out)?;
        if size < STORAGE_SIZE {
            return Err(CAPACITY);
        }
        (storage as usize).checked_add(STORAGE_SIZE as usize).ok_or(ARGUMENT)?;
        // SAFETY: Caller owns sufficient aligned storage. Field projection does
        // not create a reference to the uninitialized outer State.
        unsafe {
            let core = &mut *ptr::addr_of_mut!((*out).core).cast::<MaybeUninit<MsReassembly>>();
            MsReassembly::init_in_place(core);
            ptr::addr_of_mut!((*out).magic).write(MAGIC);
        }
        Ok(())
    })
}

/// # Safety
/// Storage must be initialized and exclusively owned for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_ms_reassembly_reset(storage: *mut c_void) -> i32 {
    boundary(|| {
        // SAFETY: Caller guarantees initialized exclusive storage.
        unsafe {
            (*state(storage)?).core.reset();
        }
        Ok(())
    })
}

/// # Safety
/// Storage and event must be live, aligned, disjoint objects; serialize all calls.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_ms_reassembly_push(storage: *mut c_void, event: *const Event) -> i32 {
    let mut outcome = 0;
    let result = boundary(|| {
        disjoint_region(storage, STORAGE_SIZE, event)?;
        // SAFETY: Read only the header before validating the complete input size.
        let (size, version) = unsafe { (ptr::addr_of!((*event).size).read(), ptr::addr_of!((*event).abi_version).read()) };
        if size != size_of::<Event>() as u32 || version != ABI_VERSION {
            return Err(ABI_MISMATCH);
        }
        // SAFETY: Caller guarantees the declared live region; alignment,
        // full layout/version and non-overlap were checked before borrowing it.
        let (core, event) = unsafe { (&mut (*state(storage)?).core, &*event) };
        if event.reserved != 0 || event.aie_valid > 1 {
            return Err(ARGUMENT);
        }
        let now = time(event.now)?;
        if event.pdu_bits > 4096 {
            return Err(CAPACITY);
        }
        let bits = BitView::new(&event.pdu, event.pdu_bits).map_err(|_| CAPACITY)?;
        match event.kind {
            1 => {
                let address = address(event.ssi, event.ssi_type)?;
                let replaced = core
                    .first(now, address, (event.aie_valid == 1).then_some(event.aie_info), bits)
                    .map_err(code)?;
                if replaced {
                    outcome = REPLACED;
                }
            }
            2 | 3 => core.next(now, bits, event.kind == 3).map_err(code)?,
            4 => {
                let expired = core.age(now).map_err(code)?;
                if expired != 0 {
                    outcome = EXPIRED_MASK | i32::from(expired);
                }
            }
            _ => return Err(ARGUMENT),
        }
        Ok(())
    });
    if result == 0 { outcome } else { result }
}

/// # Safety
/// Storage and action must be live, aligned, disjoint objects. Initialize the
/// action size/version fields before calling. EMPTY does not modify the output.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_ms_reassembly_poll(storage: *mut c_void, now: u32, action: *mut Action) -> i32 {
    boundary(|| {
        disjoint_region(storage, STORAGE_SIZE, action)?;
        let now = time(now)?;
        // SAFETY: Caller guarantees live objects; alignment and overlap checked.
        let core = unsafe { &mut (*state(storage)?).core };
        // SAFETY: Only the caller-initialized header is read, not the output payload.
        let (size, version) = unsafe { (ptr::addr_of!((*action).size).read(), ptr::addr_of!((*action).abi_version).read()) };
        if size != size_of::<Action>() as u32 || version != ABI_VERSION {
            return Err(ABI_MISMATCH);
        }
        let (metadata, bits) = match core.completed(now) {
            Ok(value) => value,
            Err(FragmentError::NotComplete) => return Err(EMPTY),
            Err(error) => return Err(code(error)),
        };
        let mut result = Action {
            size: size_of::<Action>() as u32,
            abi_version: ABI_VERSION,
            ssi: metadata.address.ssi,
            ssi_type: address_kind(metadata.address.ssi_type),
            first: metadata.first.to_int() as u32,
            last: metadata.last.to_int() as u32,
            aie_valid: u32::from(metadata.aie_info.is_some()),
            aie_info: metadata.aie_info.unwrap_or(0),
            pdu_bits: bits.len(),
            fragments: metadata.fragments,
            pdu: [0; 512],
        };
        for offset in 0..bits.len() {
            let bit = bits.read(offset, 1).map_err(|_| ARGUMENT)? as u8;
            result.pdu[usize::from(offset / 8)] |= bit << (7 - offset % 8);
        }

        core.release(now).map_err(code)?;
        // SAFETY: Caller owns sufficient writable storage; overlap/alignment checked.
        unsafe {
            action.write(result);
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "c-smoke")]
    #[test]
    fn linked_c_reassembly_caller() {
        unsafe extern "C" {
            fn tetra_ms_reassembly_c_smoke() -> i32;
        }
        // SAFETY: The C smoke function owns all of its disjoint local objects.
        assert_eq!(unsafe { tetra_ms_reassembly_c_smoke() }, 0);
    }

    #[test]
    fn polling_needs_only_an_initialized_output_header_and_errors_preserve_the_chain() {
        let mut storage = MaybeUninit::<[u32; STORAGE_SIZE as usize / 4]>::uninit();
        let storage = storage.as_mut_ptr().cast();
        let mut output = MaybeUninit::<Action>::uninit();
        let out = output.as_mut_ptr();
        let mut event = Event {
            size: size_of::<Event>() as u32,
            abi_version: ABI_VERSION,
            kind: 1,
            now: 0,
            ssi: 1234,
            ssi_type: 2,
            aie_valid: 0,
            aie_info: 0,
            pdu_bits: 0,
            reserved: 0,
            pdu: [0; 512],
        };
        // SAFETY: Storage is sufficiently large/aligned and exclusively borrowed.
        // Output payload is deliberately uninitialized: only its header is read
        // before a successful poll initializes the complete Action.
        unsafe {
            assert_eq!(tetra_ms_reassembly_init(storage, STORAGE_SIZE), 0);
            ptr::addr_of_mut!((*out).size).write(size_of::<Action>() as u32);
            ptr::addr_of_mut!((*out).abi_version).write(ABI_VERSION);
            assert_eq!(tetra_ms_reassembly_poll(storage, 0, out), EMPTY);
            assert_eq!(tetra_ms_reassembly_push(storage, &event), 0);
            event.kind = 3;
            assert_eq!(tetra_ms_reassembly_push(storage, &event), 0);
            ptr::addr_of_mut!((*out).size).write(0);
            assert_eq!(tetra_ms_reassembly_poll(storage, 0, out), ABI_MISMATCH);
            ptr::addr_of_mut!((*out).size).write(size_of::<Action>() as u32);
            assert_eq!(tetra_ms_reassembly_poll(storage, 0, out), 0);
            let output = output.assume_init();
            assert_eq!(output.fragments, 2);
            assert_eq!(output.ssi, 1234);
            assert_eq!(output.pdu_bits, 0);
        }
    }
}
