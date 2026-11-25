//! Caller-owned SDS/SDS-TL ABI. See `include/tetra_sds.h`.
use crate::{
    ABI_MISMATCH, ARGUMENT, CAPACITY, EMPTY, INVALID_IDENTITY, QUEUE_FULL, TRUNCATED, UNINITIALIZED, UNSUPPORTED, boundary,
    disjoint_region, valid_ptr,
};
use core::{
    ffi::c_void,
    mem::{align_of, size_of},
    ptr,
};
use tetra_ms_core::{
    sds::{SdsAction, SdsCore, SdsError, SdsEvent, UserData},
    types::{BitError, BitView, MAX_PDU_BITS, MAX_PDU_BYTES, PackedBits, SsiType, TetraAddress},
};
pub const ABI_VERSION: u32 = 1;
pub const STORAGE_SIZE: u32 = 4096;
pub const STORAGE_ALIGN: u32 = 8;
pub const INVALID_VALUE: i32 = -301;
pub const INVALID_WIDTH: i32 = -302;
const MAGIC: u32 = 0x5344_5301;

#[repr(C)]
pub struct Config {
    pub size: u32,
    pub abi_version: u32,
}
#[repr(C)]
pub struct Event {
    pub size: u32,
    pub abi_version: u32,
    pub kind: u32,
    pub ssi: u32,
    pub ssi_type: u32,
    pub flags: u32,
    pub data_type: u32,
    pub status: u16,
    pub pdu_bits: u16,
    pub protocol_id: u8,
    pub reference: u8,
    pub report_request: u8,
    pub delivery_status: u8,
    pub pdu: [u8; MAX_PDU_BYTES as usize],
}
#[repr(C)]
pub struct Action {
    pub size: u32,
    pub abi_version: u32,
    pub kind: u32,
    pub ssi: u32,
    pub ssi_type: u32,
    pub flags: u32,
    pub data_type: u32,
    pub status: u16,
    pub pdu_bits: u16,
    pub protocol_id: u8,
    pub reference: u8,
    pub report_request: u8,
    pub delivery_status: u8,
    pub pdu: [u8; MAX_PDU_BYTES as usize],
}
#[repr(C)]
pub struct Status {
    pub size: u32,
    pub abi_version: u32,
    pub outstanding: u16,
    pub actions: u16,
    pub present: [u8; 32],
    pub destination: [u32; 256],
}
#[repr(C)]
struct State {
    magic: u32,
    core: SdsCore,
}
const _: () = {
    assert!(size_of::<State>() <= STORAGE_SIZE as usize && align_of::<State>() <= STORAGE_ALIGN as usize);
    assert!(!core::mem::needs_drop::<State>());
    assert!(size_of::<Config>() == 8 && size_of::<Event>() == 372 && size_of::<Action>() == 372 && size_of::<Status>() == 1068);
    assert!(align_of::<Event>() == 4 && align_of::<Action>() == 4 && align_of::<Status>() == 4);
    assert!(core::mem::offset_of!(Event, pdu) == 36 && core::mem::offset_of!(Action, pdu) == 36);
};
fn code(error: SdsError) -> i32 {
    match error {
        SdsError::Bits(BitError::Truncated) => TRUNCATED,
        SdsError::Bits(BitError::Capacity) => CAPACITY,
        SdsError::Bits(BitError::InvalidWidth) => INVALID_WIDTH,
        SdsError::Bits(BitError::InvalidValue) | SdsError::InvalidValue => INVALID_VALUE,
        SdsError::UnsupportedPdu => UNSUPPORTED,
        SdsError::OutputFull => QUEUE_FULL,
    }
}
fn validate_config(input: &Config) -> Result<(), i32> {
    if input.size != size_of::<Config>() as u32 || input.abi_version != ABI_VERSION {
        Err(ABI_MISMATCH)
    } else {
        Ok(())
    }
}
fn storage_ptr(storage: *const c_void) -> Result<*mut State, i32> {
    valid_ptr(storage.cast::<State>())?;
    if !(storage as usize).is_multiple_of(STORAGE_ALIGN as usize) {
        return Err(ARGUMENT);
    }
    Ok(storage.cast_mut().cast())
}
unsafe fn state_ptr(storage: *const c_void) -> Result<*mut State, i32> {
    let state = storage_ptr(storage)?;
    // SAFETY: caller provides readable storage; only initialized header is read.
    if unsafe { ptr::addr_of!((*state).magic).read() } != MAGIC {
        return Err(UNINITIALIZED);
    }
    Ok(state)
}
unsafe fn initialize(state: *mut State) {
    // SAFETY: caller guarantees exclusive aligned storage; initialize every field.
    unsafe {
        SdsCore::init_at(ptr::addr_of_mut!((*state).core));
        ptr::addr_of_mut!((*state).magic).write(MAGIC);
    }
}
fn destination(input: &Event) -> Result<TetraAddress, i32> {
    if input.ssi > 0xff_ffff {
        return Err(INVALID_IDENTITY);
    }
    let kind = match input.ssi_type {
        2 => SsiType::Issi,
        3 => SsiType::Gssi,
        _ => return Err(ARGUMENT),
    };
    Ok(TetraAddress::new(input.ssi, kind))
}
fn user_data(kind: u32, bits: BitView<'_>) -> Result<UserData, i32> {
    Ok(match kind {
        0 if bits.len() == 16 => UserData::Type1(bits.read(0, 16).map_err(|_| TRUNCATED)? as u16),
        1 if bits.len() == 32 => UserData::Type2(bits.read(0, 32).map_err(|_| TRUNCATED)?),
        2 if bits.len() == 64 => UserData::Type3(
            (u64::from(bits.read(0, 32).map_err(|_| TRUNCATED)?) << 32) | u64::from(bits.read(32, 32).map_err(|_| TRUNCATED)?),
        ),
        3 => UserData::Type4(PackedBits::from_view(bits).map_err(|error| code(SdsError::Bits(error)))?),
        _ => return Err(INVALID_VALUE),
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn tetra_sds_abi_version() -> u32 {
    ABI_VERSION
}
#[unsafe(no_mangle)]
pub extern "C" fn tetra_sds_core_size() -> u32 {
    STORAGE_SIZE
}
#[unsafe(no_mangle)]
pub extern "C" fn tetra_sds_core_align() -> u32 {
    STORAGE_ALIGN
}
/// # Safety
/// Storage/input must be live, aligned, sufficiently sized and disjoint.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_sds_init(storage: *mut c_void, storage_size: u32, input: *const Config) -> i32 {
    boundary(|| {
        let state = storage_ptr(storage)?;
        if storage_size < STORAGE_SIZE {
            return Err(CAPACITY);
        }
        disjoint_region(storage, STORAGE_SIZE, input)?;
        // SAFETY: readable config, validated pointer and non-overlap.
        validate_config(unsafe { &*input })?;
        // SAFETY: complete storage and configuration validated.
        unsafe { initialize(state) };
        Ok(())
    })
}
/// # Safety
/// Storage initialized/exclusive; input readable/aligned/disjoint.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_sds_reset(storage: *mut c_void, input: *const Config) -> i32 {
    boundary(|| {
        disjoint_region(storage, STORAGE_SIZE, input)?;
        // SAFETY: caller provides readable configuration.
        validate_config(unsafe { &*input })?;
        // SAFETY: only initialized header inspected before accessing the core.
        let state = unsafe { state_ptr(storage)? };
        // SAFETY: state has no Drop; initialize all fields in place.
        unsafe { initialize(state) };
        Ok(())
    })
}
/// # Safety
/// Storage initialized/exclusive; input fully initialized, readable, aligned and disjoint.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_sds_push_event(storage: *mut c_void, input: *const Event) -> i32 {
    boundary(|| {
        disjoint_region(storage, STORAGE_SIZE, input)?;
        // SAFETY: caller provides complete readable Event; no pointer retained.
        let input = unsafe { &*input };
        if input.size != size_of::<Event>() as u32 || input.abi_version != ABI_VERSION {
            return Err(ABI_MISMATCH);
        }
        if input.pdu_bits > MAX_PDU_BITS {
            return Err(CAPACITY);
        }
        if input.flags & !1 != 0 || (input.kind != 1 && input.flags != 0) {
            return Err(ARGUMENT);
        }
        if !matches!(input.kind, 1..=6) {
            return Err(UNSUPPORTED);
        }
        let data = BitView::new(&input.pdu, input.pdu_bits).map_err(|_| TRUNCATED)?;
        // Prepare every fallible conversion before touching state.
        let dest = if matches!(input.kind, 2..=4) {
            Some(destination(input)?)
        } else {
            None
        };
        let raw = if input.kind == 2 {
            Some(user_data(input.data_type, data)?)
        } else {
            None
        };
        if input.kind == 5 && input.ssi > 0xff_ffff {
            return Err(INVALID_IDENTITY);
        }
        // SAFETY: initialized/exclusive state and disjoint input were validated.
        let core = unsafe { &mut (*state_ptr(storage)?).core };
        match input.kind {
            1 => core
                .push_event(SdsEvent::Downlink {
                    sdu: data,
                    group: input.flags & 1 != 0,
                })
                .map_err(code),
            2 => core
                .push_event(SdsEvent::Unitdata {
                    destination: dest.ok_or(ARGUMENT)?,
                    data: raw.as_ref().ok_or(ARGUMENT)?,
                })
                .map_err(code),
            3 => core
                .push_event(SdsEvent::Status {
                    destination: dest.ok_or(ARGUMENT)?,
                    status: input.status,
                })
                .map_err(code),
            4 => core
                .push_event(SdsEvent::Message {
                    destination: dest.ok_or(ARGUMENT)?,
                    protocol_id: input.protocol_id,
                    report_request: input.report_request,
                    reference: input.reference,
                    data,
                })
                .map_err(code),
            5 => core
                .push_event(SdsEvent::Report {
                    destination: input.ssi,
                    reference: input.reference,
                    status: input.delivery_status,
                })
                .map_err(code),
            6 => {
                if core.cancel(input.reference) {
                    Ok(())
                } else {
                    Err(EMPTY)
                }
            }
            _ => Err(UNSUPPORTED),
        }
    })
}
fn payload(out: &mut Action, bits: &PackedBits) {
    out.pdu_bits = bits.len();
    out.pdu[..bits.as_bytes().len()].copy_from_slice(bits.as_bytes());
}
fn action(out: &mut Action, next: SdsAction) {
    match next {
        SdsAction::Uplink { destination, sdu } => {
            out.kind = 1;
            out.ssi = destination.ssi;
            out.ssi_type = destination.ssi_type as u32;
            payload(out, &sdu);
        }
        SdsAction::Unitdata { source, group, data } => {
            out.kind = 2;
            out.ssi = source;
            out.flags = u32::from(group);
            match data {
                UserData::Type1(value) => {
                    out.data_type = 0;
                    out.pdu_bits = 16;
                    out.pdu[..2].copy_from_slice(&value.to_be_bytes());
                }
                UserData::Type2(value) => {
                    out.data_type = 1;
                    out.pdu_bits = 32;
                    out.pdu[..4].copy_from_slice(&value.to_be_bytes());
                }
                UserData::Type3(value) => {
                    out.data_type = 2;
                    out.pdu_bits = 64;
                    out.pdu[..8].copy_from_slice(&value.to_be_bytes());
                }
                UserData::Type4(bits) => {
                    out.data_type = 3;
                    payload(out, &bits);
                }
            }
        }
        SdsAction::Status { source, status } => {
            out.kind = 3;
            out.ssi = source;
            out.status = status;
        }
        SdsAction::Message {
            source,
            group,
            protocol_id,
            report_request,
            reference,
            data,
        } => {
            out.kind = 4;
            out.ssi = source;
            out.flags = u32::from(group);
            out.protocol_id = protocol_id;
            out.report_request = report_request;
            out.reference = reference;
            payload(out, &data);
        }
        SdsAction::Report {
            source,
            reference,
            status,
            short,
        } => {
            out.kind = 5;
            out.ssi = source;
            out.reference = reference;
            out.delivery_status = status;
            out.flags = u32::from(short) << 1;
        }
    }
}
/// # Safety
/// Storage initialized/exclusive; output writable/aligned/disjoint with initialized headers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_sds_poll_action(storage: *mut c_void, output: *mut Action) -> i32 {
    boundary(|| {
        disjoint_region(storage, STORAGE_SIZE, output)?;
        // SAFETY: only initialized integer headers read.
        if unsafe { ptr::addr_of!((*output).size).read() } != size_of::<Action>() as u32
            || unsafe { ptr::addr_of!((*output).abi_version).read() } != ABI_VERSION
        {
            return Err(ABI_MISMATCH);
        }
        // SAFETY: state is initialized and exclusively accessible.
        let next = unsafe { (&mut *state_ptr(storage)?).core.pop_action() }.ok_or(EMPTY)?;
        // SAFETY: zero is valid for this complete integer-only output.
        let out = unsafe {
            ptr::write_bytes(output, 0, 1);
            &mut *output
        };
        out.size = size_of::<Action>() as u32;
        out.abi_version = ABI_VERSION;
        action(out, next);
        Ok(())
    })
}
/// # Safety
/// Storage readable/initialized without concurrent writes; output writable/aligned/disjoint.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_sds_get_status(storage: *const c_void, output: *mut Status) -> i32 {
    boundary(|| {
        disjoint_region(storage, STORAGE_SIZE, output)?;
        // SAFETY: caller initializes these two integer headers.
        if unsafe { ptr::addr_of!((*output).size).read() } != size_of::<Status>() as u32
            || unsafe { ptr::addr_of!((*output).abi_version).read() } != ABI_VERSION
        {
            return Err(ABI_MISMATCH);
        }
        // SAFETY: initialized shared storage, no concurrent writer.
        let core = unsafe { &(*state_ptr(storage)?).core };
        // SAFETY: zero is valid for all output fields.
        let out = unsafe {
            ptr::write_bytes(output, 0, 1);
            &mut *output
        };
        out.size = size_of::<Status>() as u32;
        out.abi_version = ABI_VERSION;
        let status = core.status();
        out.outstanding = status.outstanding;
        out.actions = status.queued_actions;
        for reference in 0..=u8::MAX {
            if let Some(destination) = core.outstanding(reference) {
                out.present[usize::from(reference) / 8] |= 1 << (reference % 8);
                out.destination[usize::from(reference)] = destination;
            }
        }

        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Config {
        Config {
            size: size_of::<Config>() as u32,
            abi_version: ABI_VERSION,
        }
    }
    fn event() -> Event {
        // SAFETY: all-zero is valid for this integer-only input.
        let mut event: Event = unsafe { core::mem::zeroed() };
        event.size = size_of::<Event>() as u32;
        event.abi_version = ABI_VERSION;
        event.kind = 4;
        event.ssi = 555;
        event.ssi_type = 2;
        event.protocol_id = 0x82;
        event.report_request = 1;
        event.reference = 7;
        event.pdu_bits = 5;
        event.pdu[0] = 0xa8;
        event
    }
    #[test]
    fn invalid_native_input_and_poll_do_not_consume_or_track() {
        let mut bytes = std::vec![0u64;STORAGE_SIZE as usize/8];
        let storage = bytes.as_mut_ptr().cast::<c_void>();
        let config = config();
        let mut event = event();
        let mut output = core::mem::MaybeUninit::<Action>::uninit();
        // SAFETY: live disjoint buffers; only initialized headers are read before output initialization.
        unsafe {
            assert_eq!(tetra_sds_init(storage, STORAGE_SIZE, &config), 0);
            assert_eq!(tetra_sds_push_event(storage, &event), 0);
            event.reference = 8;
            event.pdu_bits = MAX_PDU_BITS + 1;
            assert_eq!(tetra_sds_push_event(storage, &event), CAPACITY);
            event.pdu_bits = 5;
            event.report_request = 4;
            assert_eq!(tetra_sds_push_event(storage, &event), INVALID_VALUE);
            ptr::addr_of_mut!((*output.as_mut_ptr()).size).write(size_of::<Action>() as u32);
            ptr::addr_of_mut!((*output.as_mut_ptr()).abi_version).write(99);
            assert_eq!(tetra_sds_poll_action(storage, output.as_mut_ptr()), ABI_MISMATCH);
            assert_eq!(tetra_sds_poll_action(storage, storage.cast()), ARGUMENT);
            assert_eq!(
                tetra_sds_poll_action(
                    storage,
                    storage.cast::<u8>().add(STORAGE_SIZE as usize - size_of::<Action>()).cast()
                ),
                ARGUMENT
            );
            ptr::addr_of_mut!((*output.as_mut_ptr()).abi_version).write(ABI_VERSION);
            assert_eq!(tetra_sds_poll_action(storage, output.as_mut_ptr()), 0);
            let mut action = output.assume_init();
            assert_eq!((action.kind, action.ssi), (1, 555));
            let snapshot = action.pdu;
            assert_eq!(tetra_sds_poll_action(storage, &mut action), EMPTY);
            assert_eq!(action.pdu, snapshot);
            let mut status = core::mem::MaybeUninit::<Status>::uninit();
            ptr::addr_of_mut!((*status.as_mut_ptr()).size).write(size_of::<Status>() as u32);
            ptr::addr_of_mut!((*status.as_mut_ptr()).abi_version).write(ABI_VERSION);
            assert_eq!(tetra_sds_get_status(storage, status.as_mut_ptr()), 0);
            let status = status.assume_init();
            assert_eq!((status.outstanding, status.actions), (1, 0));
            assert_eq!(status.destination[7], 555);
            assert_eq!(status.present[1] & 1, 0);
            assert_eq!(tetra_sds_reset(storage, &config), 0);
            assert_eq!(tetra_sds_poll_action(storage, &mut action), EMPTY);
        }
    }
    #[test]
    fn bad_storage_and_configuration_are_rejected_without_writes() {
        let mut bytes = std::vec![0u64;STORAGE_SIZE as usize/8+1];
        let storage = bytes.as_mut_ptr().cast::<c_void>();
        let mut config = config();
        // SAFETY: invalid pointer shapes are rejected before any dereference.
        unsafe {
            assert_eq!(tetra_sds_init(ptr::null_mut(), STORAGE_SIZE, &config), ARGUMENT);
            assert_eq!(tetra_sds_init(storage.cast::<u8>().add(1).cast(), STORAGE_SIZE, &config), ARGUMENT);
            assert_eq!(tetra_sds_init(storage, STORAGE_SIZE, storage.cast()), ARGUMENT);
            config.abi_version = 99;
            assert_eq!(tetra_sds_init(storage, STORAGE_SIZE, &config), ABI_MISMATCH);
            assert!(bytes.iter().all(|word| *word == 0));
        }
    }
    #[cfg(feature = "c-smoke")]
    #[test]
    fn c_header_and_client() {
        unsafe extern "C" {
            fn tetra_sds_c_smoke() -> i32;
        }
        // SAFETY: the C client owns all buffers it passes.
        assert_eq!(unsafe { tetra_sds_c_smoke() }, 0);
    }
}
