//! Version 1 MM ABI. No C object contains a Rust enum, pointer, bool, or usize.
//! Inputs are borrowed only during a call; output errors never consume actions.

use core::{
    ffi::c_void,
    mem::{MaybeUninit, align_of, size_of},
    ptr,
};
use tetra_ms_core::{
    mm::{
        Action as CoreAction, CellId, Config as CoreConfig, Error, Event as CoreEvent, GroupOperation, GroupRequest as CoreGroupRequest,
        MAX_GROUPS, MmCore, RegState, ServiceStatus, TransferState,
    },
    types::{BitError, BitView, MAX_PDU_BITS, MAX_PDU_BYTES},
};

use crate::{ABI_MISMATCH, ARGUMENT, CAPACITY, EMPTY, INVALID_IDENTITY, INVALID_PDU, QUEUE_FULL, TRUNCATED, UNINITIALIZED, UNSUPPORTED};
use crate::{boundary, disjoint_region, valid_ptr};

pub const ABI_VERSION: u32 = 1;
pub const STORAGE_SIZE: u32 = 4608;
pub const STORAGE_ALIGN: u32 = 4;
const MAGIC: u32 = 0x544d_4d01;

pub const INVALID_WIDTH: i32 = -101;
pub const INVALID_VALUE: i32 = -102;
pub const GROUP_CAPACITY: i32 = -103;
pub const NOT_REGISTERED: i32 = -104;
pub const GROUP_OPERATION_PENDING: i32 = -105;
pub const EMPTY_GROUPS: i32 = -106;
pub const REGISTRATION_IN_PROGRESS: i32 = -107;
pub const ALREADY_REGISTERED: i32 = -108;
pub const DETACHING: i32 = -109;
pub const LEFT_SYSTEM: i32 = -110;

pub const ACTIVATE: u32 = 1;
pub const REGISTER: u32 = 2;
pub const DOWNLINK: u32 = 3;
pub const LINK_BREAK: u32 = 4;
pub const LINK_REOPEN: u32 = 5;
pub const GROUPS: u32 = 6;
pub const DEREGISTER: u32 = 7;
pub const TICK: u32 = 8;
pub const SET_GROUPS: u32 = 9;
pub const SET_IDENTITY: u32 = 10;

pub const REGISTRATION_REQUIRED: u32 = 1;
pub const SYSTEM_WIDE_SERVICES: u32 = 2;
pub const FORCE_REGISTER: u32 = 4;
pub const DETACH_ALL: u32 = 8;
pub const REPORT_REQUESTED: u32 = 16;

pub const SEND: u32 = 1;
pub const SERVICE: u32 = 2;
pub const REGISTRATION_SUCCESS: u32 = 3;
pub const REGISTRATION_FAILURE: u32 = 4;
pub const IDENTITIES: u32 = 5;
pub const GROUP_CONFIRM: u32 = 6;
pub const DETACH_REPORT: u32 = 7;
pub const DETACH_COMPLETE: u32 = 8;

pub const HAS_DETACH_RECEIPT: u32 = 1;
pub const ACTION_DETACH_ALL: u32 = 2;
pub const ACTION_REPORT_REQUESTED: u32 = 4;
pub const TRANSFER_SUCCESSFUL: u32 = 8;
pub const HAS_REJECT_CAUSE: u32 = 16;

pub const HAS_LEFT_SYSTEM: u32 = 1;
pub const TEMPORARY_REGISTRATION: u32 = 2;
pub const OUT_OF_SERVICE: u32 = 4;
pub const HAS_GROUP_OPERATION: u32 = 8;
pub const STATUS_DETACH_ALL: u32 = 16;
pub const STATUS_REPORT_REQUESTED: u32 = 32;
pub const HAS_CURRENT_CELL: u32 = 64;
pub const HAS_REGISTERED_CELL: u32 = 128;
pub const HAS_PENDING_CELL: u32 = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct GroupRequest {
    pub gssi: u32,
    /// 0 attach, 1 detach. Value is the raw class (0..7) or reason (0..3).
    pub operation: u8,
    pub value: u8,
    pub reserved: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct Cell {
    pub mcc: u16,
    pub mnc: u16,
    pub la: u16,
    pub reserved: u16,
}

#[repr(C)]
pub struct Config {
    pub size: u32,
    pub abi_version: u32,
    pub own_issi: u32,
    pub mcc: u16,
    pub mnc: u16,
    pub initial_la: u16,
    pub group_count: u16,
    pub groups: [u32; MAX_GROUPS],
}

#[repr(C)]
pub struct Event {
    pub size: u32,
    pub abi_version: u32,
    pub kind: u32,
    pub handle: u32,
    pub elapsed_slots: u32,
    pub own_issi: u32,
    pub cell_mcc: u16,
    pub cell_mnc: u16,
    pub cell_la: u16,
    pub group_count: u16,
    pub flags: u32,
    /// 0 pending, 1 transmitted, 2 acknowledged, 3 discarded, 4 lost.
    pub transfer_state: u32,
    pub pdu_bits: u16,
    pub reserved: u16,
    pub pdu: [u8; MAX_PDU_BYTES as usize],
    pub groups: [GroupRequest; MAX_GROUPS],
}

#[repr(C)]
pub struct Action {
    pub size: u32,
    pub abi_version: u32,
    pub kind: u32,
    pub handle: u32,
    pub flags: u32,
    pub service: u32,
    pub reject_cause: u32,
    pub t353_countdown: u32,
    pub priority: u8,
    pub reserved: u8,
    pub pdu_bits: u16,
    pub attached_count: u16,
    pub detached_count: u16,
    pub pdu: [u8; MAX_PDU_BYTES as usize],
    pub attached: [u32; MAX_GROUPS],
    pub detached: [u32; MAX_GROUPS],
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[repr(C)]
pub struct Status {
    pub size: u32,
    pub abi_version: u32,
    pub registration: u32,
    pub service: u32,
    pub t351_countdown: u32,
    pub detach_countdown: u32,
    pub group_handle: u32,
    pub t353_countdown: u32,
    pub flags: u32,
    pub attempts: u8,
    pub system_rejection_count: u8,
    pub pending_lu_type: u8,
    pub reserved: u8,
    pub serving_la: u16,
    pub attached_count: u16,
    pub actions: u16,
    pub reserved2: u16,
    pub current_cell: Cell,
    pub registered_cell: Cell,
    pub pending_cell: Cell,
    pub attached: [u32; MAX_GROUPS],
}

#[repr(C)]
struct State {
    magic: u32,
    core: MmCore,
}

const _: () = {
    assert!(size_of::<State>() <= STORAGE_SIZE as usize);
    assert!(align_of::<State>() == STORAGE_ALIGN as usize);
    assert!(size_of::<Config>() == 272);
    assert!(size_of::<GroupRequest>() == 8);
    assert!(size_of::<Cell>() == 8);
    assert!(size_of::<Event>() == 884);
    assert!(size_of::<Action>() == 880);
    assert!(size_of::<Status>() == 324);
    assert!(align_of::<Event>() == 4);
    assert!(align_of::<Action>() == 4);
    assert!(core::mem::offset_of!(Event, pdu) == 44);
    assert!(core::mem::offset_of!(Event, groups) == 380);
    assert!(core::mem::offset_of!(Action, pdu) == 40);
    assert!(core::mem::offset_of!(Action, attached) == 376);
    assert!(core::mem::offset_of!(Status, attached) == 72);
};

fn code(error: Error) -> i32 {
    match error {
        Error::Bits(BitError::Truncated) => TRUNCATED,
        Error::Bits(BitError::Capacity) => CAPACITY,
        Error::Bits(BitError::InvalidWidth) => INVALID_WIDTH,
        Error::Bits(BitError::InvalidValue) => INVALID_VALUE,
        Error::GroupCapacity => GROUP_CAPACITY,
        Error::ActionCapacity => QUEUE_FULL,
        Error::InvalidIdentity => INVALID_IDENTITY,
        Error::InvalidPdu => INVALID_PDU,
        Error::UnsupportedPdu(_) => UNSUPPORTED,
        Error::NotRegistered => NOT_REGISTERED,
        Error::GroupOperationPending => GROUP_OPERATION_PENDING,
        Error::EmptyGroups => EMPTY_GROUPS,
        Error::RegistrationInProgress => REGISTRATION_IN_PROGRESS,
        Error::AlreadyRegistered => ALREADY_REGISTERED,
        Error::Detaching => DETACHING,
        Error::LeftSystem => LEFT_SYSTEM,
    }
}

fn config(input: &Config) -> Result<(CoreConfig, &[u32]), i32> {
    if input.size != size_of::<Config>() as u32 || input.abi_version != ABI_VERSION {
        return Err(ABI_MISMATCH);
    }
    if usize::from(input.group_count) > MAX_GROUPS {
        return Err(GROUP_CAPACITY);
    }
    Ok((
        CoreConfig {
            issi: input.own_issi,
            mcc: input.mcc,
            mnc: input.mnc,
            initial_la: input.initial_la,
        },
        &input.groups[..usize::from(input.group_count)],
    ))
}

unsafe fn state_ptr(storage: *const c_void) -> Result<*mut State, i32> {
    let state = storage.cast::<State>();
    valid_ptr(state)?;
    // SAFETY: Caller guarantees readable storage; only its integer magic is read
    // before treating the core as initialized. Null/alignment are checked.
    if unsafe { ptr::addr_of!((*state).magic).read() } != MAGIC {
        return Err(UNINITIALIZED);
    }
    Ok(state.cast_mut())
}

fn service(status: ServiceStatus) -> u32 {
    match status {
        ServiceStatus::OutOfService => 0,
        ServiceStatus::WaitingForRegistration => 1,
        ServiceStatus::InService => 2,
    }
}

fn cell(value: CellId) -> Cell {
    Cell {
        mcc: value.mcc,
        mnc: value.mnc,
        la: value.la,
        reserved: 0,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn tetra_ms_mm_abi_version() -> u32 {
    ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn tetra_ms_mm_storage_size() -> u32 {
    STORAGE_SIZE
}

#[unsafe(no_mangle)]
pub extern "C" fn tetra_ms_mm_storage_align() -> u32 {
    STORAGE_ALIGN
}

/// # Safety
/// Storage must own at least storage_size writable bytes with queried alignment.
/// Input must be a readable Config, disjoint from the entire storage budget.
/// No concurrent access is allowed. Invalid configuration leaves storage untouched.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_ms_mm_init(storage: *mut c_void, storage_size: u32, input: *const Config) -> i32 {
    boundary(|| {
        valid_ptr(storage.cast::<State>())?;
        if storage_size < STORAGE_SIZE {
            return Err(CAPACITY);
        }
        disjoint_region(storage, STORAGE_SIZE, input)?;
        // SAFETY: Config is readable, aligned and disjoint per checked contract.
        let (config, groups) = config(unsafe { &*input })?;
        let state = storage.cast::<State>();
        // SAFETY: Initialize directly in caller-owned storage, publishing the
        // magic only after every field/queue slot has been initialized.
        unsafe {
            let core = ptr::addr_of_mut!((*state).core).cast::<MaybeUninit<MmCore>>();
            MmCore::init_in_place(&mut *core, config, groups).map_err(code)?;
            ptr::addr_of_mut!((*state).magic).write(MAGIC);
        }
        Ok(())
    })
}

/// # Safety
/// Storage must be initialized and exclusively accessible. Config must be
/// readable/aligned/disjoint. Reset cancels queued actions and old detach receipts.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_ms_mm_reset(storage: *mut c_void, input: *const Config) -> i32 {
    boundary(|| {
        disjoint_region(storage, STORAGE_SIZE, input)?;
        // SAFETY: Input is aligned/disjoint; initialized storage is checked below.
        let (config, groups) = config(unsafe { &*input })?;
        let state = unsafe { state_ptr(storage)? };
        // SAFETY: Caller exclusively owns the initialized MM instance.
        unsafe { &mut (*state).core }.reset(config, groups).map_err(code)
    })
}

/// # Safety
/// Storage must be initialized/exclusive; input must be a readable aligned Event,
/// disjoint from storage. Event arrays are borrowed only during this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_ms_mm_push_event(storage: *mut c_void, input: *const Event) -> i32 {
    boundary(|| {
        disjoint_region(storage, STORAGE_SIZE, input)?;
        // SAFETY: Input points to a readable aligned Event per caller contract.
        let input = unsafe { &*input };
        if input.size != size_of::<Event>() as u32 || input.abi_version != ABI_VERSION {
            return Err(ABI_MISMATCH);
        }
        if input.reserved != 0 {
            return Err(ARGUMENT);
        }
        let allowed_flags = match input.kind {
            ACTIVATE => REGISTRATION_REQUIRED | SYSTEM_WIDE_SERVICES | FORCE_REGISTER,
            GROUPS => DETACH_ALL | REPORT_REQUESTED,
            _ => 0,
        };
        if input.flags & !allowed_flags != 0 {
            return Err(ARGUMENT);
        }
        // SAFETY: Validate the header before obtaining exclusive core access.
        let state = unsafe { state_ptr(storage)? };
        let core = unsafe { &mut (*state).core };
        match input.kind {
            ACTIVATE => core
                .handle(CoreEvent::Activate {
                    cell: CellId {
                        mcc: input.cell_mcc,
                        mnc: input.cell_mnc,
                        la: input.cell_la,
                    },
                    registration_required: input.flags & REGISTRATION_REQUIRED != 0,
                    system_wide_services: input.flags & SYSTEM_WIDE_SERVICES != 0,
                    force_register: input.flags & FORCE_REGISTER != 0,
                })
                .map_err(code),
            REGISTER => core.handle(CoreEvent::Register).map_err(code),
            DOWNLINK => {
                if input.pdu_bits > MAX_PDU_BITS {
                    return Err(CAPACITY);
                }
                let bits = BitView::new(&input.pdu, input.pdu_bits).map_err(|error| code(error.into()))?;
                core.handle(CoreEvent::Downlink(bits)).map_err(code)
            }
            LINK_BREAK => core.handle(CoreEvent::LinkBreak).map_err(code),
            LINK_REOPEN => core.handle(CoreEvent::LinkReopen).map_err(code),
            GROUPS => {
                if usize::from(input.group_count) > MAX_GROUPS {
                    return Err(GROUP_CAPACITY);
                }
                let mut requests = [CoreGroupRequest {
                    gssi: 0,
                    operation: GroupOperation::Attach { class_of_usage: 0 },
                }; MAX_GROUPS];
                for (out, request) in requests.iter_mut().zip(&input.groups[..usize::from(input.group_count)]) {
                    if request.reserved != 0 {
                        return Err(ARGUMENT);
                    }
                    *out = CoreGroupRequest {
                        gssi: request.gssi,
                        operation: match request.operation {
                            0 => GroupOperation::Attach {
                                class_of_usage: request.value,
                            },
                            1 => GroupOperation::Detach { reason: request.value },
                            _ => return Err(ARGUMENT),
                        },
                    };
                }
                core.handle(CoreEvent::Groups {
                    handle: input.handle,
                    detach_all: input.flags & DETACH_ALL != 0,
                    report_requested: input.flags & REPORT_REQUESTED != 0,
                    requests: &requests[..usize::from(input.group_count)],
                })
                .map_err(code)
            }
            DEREGISTER => core.handle(CoreEvent::Deregister).map_err(code),
            TICK => {
                let detach_transfer = match input.transfer_state {
                    0 => TransferState::Pending,
                    1 => TransferState::Transmitted,
                    2 => TransferState::Acknowledged,
                    3 => TransferState::Discarded,
                    4 => TransferState::Lost,
                    _ => return Err(ARGUMENT),
                };
                core.handle(CoreEvent::Tick {
                    elapsed_slots: input.elapsed_slots,
                    detach_transfer,
                })
                .map_err(code)
            }
            SET_GROUPS => {
                if usize::from(input.group_count) > MAX_GROUPS {
                    return Err(GROUP_CAPACITY);
                }
                let mut groups = [0; MAX_GROUPS];
                for (out, request) in groups.iter_mut().zip(&input.groups[..usize::from(input.group_count)]) {
                    if request.reserved != 0 || request.operation != 0 || request.value != 0 {
                        return Err(ARGUMENT);
                    }
                    *out = request.gssi;
                }
                core.configure_groups(&groups[..usize::from(input.group_count)]).map_err(code)
            }
            SET_IDENTITY => core
                .configure(CoreConfig {
                    issi: input.own_issi,
                    mcc: input.cell_mcc,
                    mnc: input.cell_mnc,
                    initial_la: input.cell_la,
                })
                .map_err(code),
            _ => Err(UNSUPPORTED),
        }
    })
}

/// # Safety
/// Storage must be initialized/exclusive. Output must be writable/aligned/disjoint
/// with initialized size/version fields. EMPTY and errors do not modify output or
/// consume an action. All other output fields may initially be uninitialized.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_ms_mm_poll_action(storage: *mut c_void, output: *mut Action) -> i32 {
    boundary(|| {
        disjoint_region(storage, STORAGE_SIZE, output)?;
        // SAFETY: Caller initializes these two integer headers.
        if unsafe { ptr::addr_of!((*output).size).read() } != size_of::<Action>() as u32
            || unsafe { ptr::addr_of!((*output).abi_version).read() } != ABI_VERSION
        {
            return Err(ABI_MISMATCH);
        }
        let state = unsafe { state_ptr(storage)? };
        let Some(action) = unsafe { &mut (*state).core }.poll_action() else {
            return Err(EMPTY);
        };
        // SAFETY: Output is writable/disjoint. Zero every integer before forming
        // a reference, supporting C callers that initialized only the headers.
        unsafe { ptr::write_bytes(output, 0, 1) };
        let out = unsafe { &mut *output };
        out.size = size_of::<Action>() as u32;
        out.abi_version = ABI_VERSION;
        match action {
            CoreAction::Send {
                sdu,
                priority,
                detach_receipt,
            } => {
                out.kind = SEND;
                out.priority = priority;
                out.flags = if detach_receipt { HAS_DETACH_RECEIPT } else { 0 };
                out.pdu_bits = sdu.len();
                out.pdu[..sdu.as_bytes().len()].copy_from_slice(sdu.as_bytes());
            }
            CoreAction::Service(status) => {
                out.kind = SERVICE;
                out.service = service(status);
            }
            CoreAction::RegistrationSuccess { attached } => {
                out.kind = REGISTRATION_SUCCESS;
                out.attached_count = attached.len() as u16;
                for (output, group) in out.attached.iter_mut().zip(attached.iter()) {
                    *output = group.gssi;
                }
            }
            CoreAction::RegistrationFailure { cause } => {
                out.kind = REGISTRATION_FAILURE;
                if let Some(cause) = cause {
                    out.flags = HAS_REJECT_CAUSE;
                    out.reject_cause = u32::from(cause);
                }
            }
            CoreAction::Identities { attached, detached } => {
                out.kind = IDENTITIES;
                out.attached_count = attached.as_slice().len() as u16;
                out.detached_count = detached.as_slice().len() as u16;
                out.attached[..attached.as_slice().len()].copy_from_slice(attached.as_slice());
                out.detached[..detached.as_slice().len()].copy_from_slice(detached.as_slice());
            }
            CoreAction::GroupConfirm { op, attached, detached } => {
                out.kind = GROUP_CONFIRM;
                out.handle = op.handle;
                out.t353_countdown = op.t353_countdown;
                out.flags =
                    if op.detach_all { ACTION_DETACH_ALL } else { 0 } | if op.report_requested { ACTION_REPORT_REQUESTED } else { 0 };
                out.attached_count = attached.len() as u16;
                out.detached_count = detached.as_slice().len() as u16;
                for (output, group) in out.attached.iter_mut().zip(attached.iter()) {
                    *output = group.gssi;
                }
                out.detached[..detached.as_slice().len()].copy_from_slice(detached.as_slice());
            }
            CoreAction::DetachReport { successful } => {
                out.kind = DETACH_REPORT;
                out.flags = if successful { TRANSFER_SUCCESSFUL } else { 0 };
            }
            CoreAction::DetachComplete => out.kind = DETACH_COMPLETE,
        }
        Ok(())
    })
}

/// # Safety
/// Storage must be initialized/readable with no concurrent mutation. Output must
/// be writable/aligned/disjoint with initialized size/version headers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_ms_mm_get_status(storage: *const c_void, output: *mut Status) -> i32 {
    boundary(|| {
        disjoint_region(storage, STORAGE_SIZE, output)?;
        // SAFETY: Caller initializes these two integer headers.
        if unsafe { ptr::addr_of!((*output).size).read() } != size_of::<Status>() as u32
            || unsafe { ptr::addr_of!((*output).abi_version).read() } != ABI_VERSION
        {
            return Err(ABI_MISMATCH);
        }

        let state = unsafe { state_ptr(storage)? };
        // SAFETY: No concurrent mutation is permitted by the caller contract.
        let core = unsafe { &(*state).core };
        let state = core.state();
        // SAFETY: All fields are fixed-width integers; initialize before borrowing.
        unsafe { ptr::write_bytes(output, 0, 1) };
        let out = unsafe { &mut *output };
        out.size = size_of::<Status>() as u32;
        out.abi_version = ABI_VERSION;
        out.registration = match state.reg_state {
            RegState::Idle => 0,
            RegState::Registering => 1,
            RegState::Registered => 2,
            RegState::Detaching => 3,
        };
        out.service = service(core.service_status());
        out.t351_countdown = state.t351_countdown;
        out.detach_countdown = state.detach_countdown;
        out.flags = if state.left_system { HAS_LEFT_SYSTEM } else { 0 }
            | if state.temporary_registration { TEMPORARY_REGISTRATION } else { 0 }
            | if state.out_of_service { OUT_OF_SERVICE } else { 0 };
        out.attempts = state.attempts;
        out.system_rejection_count = state.system_rejection_count;
        out.pending_lu_type = state.pending_lu_type as u8;
        out.serving_la = state.serving_la;
        out.actions = core.action_count();
        out.attached_count = state.attached_gssis.as_slice().len() as u16;
        out.attached[..state.attached_gssis.as_slice().len()].copy_from_slice(state.attached_gssis.as_slice());
        if let Some(op) = state.pending_group_op {
            out.flags |= HAS_GROUP_OPERATION
                | if op.detach_all { STATUS_DETACH_ALL } else { 0 }
                | if op.report_requested { STATUS_REPORT_REQUESTED } else { 0 };
            out.group_handle = op.handle;
            out.t353_countdown = op.t353_countdown;
        }
        if let Some(value) = state.current_cell {
            out.flags |= HAS_CURRENT_CELL;
            out.current_cell = cell(value);
        }
        if let Some(value) = state.registered_cell {
            out.flags |= HAS_REGISTERED_CELL;
            out.registered_cell = cell(value);
        }
        if let Some(value) = state.pending_cell {
            out.flags |= HAS_PENDING_CELL;
            out.pending_cell = cell(value);
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OK;
    use tetra_ms_core::types::PackedBits;

    #[repr(C, align(4))]
    struct Storage([u8; STORAGE_SIZE as usize]);

    fn config() -> Config {
        Config {
            size: size_of::<Config>() as u32,
            abi_version: ABI_VERSION,
            own_issi: 1_000_001,
            mcc: 234,
            mnc: 5678,
            initial_la: 100,
            group_count: 0,
            groups: [0; MAX_GROUPS],
        }
    }

    fn event(kind: u32) -> Event {
        Event {
            size: size_of::<Event>() as u32,
            abi_version: ABI_VERSION,
            kind,
            handle: 0,
            elapsed_slots: 0,
            own_issi: 0,
            cell_mcc: 234,
            cell_mnc: 5678,
            cell_la: 100,
            group_count: 0,
            flags: 0,
            transfer_state: 0,
            pdu_bits: 0,
            reserved: 0,
            pdu: [0; MAX_PDU_BYTES as usize],
            groups: [GroupRequest {
                gssi: 0,
                operation: 0,
                value: 0,
                reserved: 0,
            }; MAX_GROUPS],
        }
    }

    fn action() -> Action {
        // SAFETY: Every field is an integer or integer array; all-zero is valid.
        let mut output: Action = unsafe { MaybeUninit::zeroed().assume_init() };
        output.size = size_of::<Action>() as u32;
        output.abi_version = ABI_VERSION;
        output
    }

    fn status(storage: *const c_void) -> Status {
        // SAFETY: All-zero is valid for these integer fields. Tests supply a live
        // initialized storage pointer, with disjoint output and no concurrency.
        unsafe {
            let mut output: Status = MaybeUninit::zeroed().assume_init();
            output.size = size_of::<Status>() as u32;
            output.abi_version = ABI_VERSION;
            assert_eq!(tetra_ms_mm_get_status(storage, &mut output), OK);
            output
        }
    }

    fn drain(storage: *mut c_void) -> std::vec::Vec<Action> {
        let mut result = std::vec::Vec::new();
        loop {
            let mut output = action();
            // SAFETY: Test storage is initialized/exclusive; output is live/disjoint.
            match unsafe { tetra_ms_mm_poll_action(storage, &mut output) } {
                OK => result.push(output),
                EMPTY => return result,
                error => panic!("poll failed: {error}"),
            }
        }
    }

    fn downlink(text: &str) -> Event {
        let mut input = event(DOWNLINK);
        input.pdu_bits = text.len() as u16;
        for (pos, bit) in text.bytes().enumerate() {
            input.pdu[pos / 8] |= (bit - b'0') << (7 - pos % 8);
        }
        input
    }

    fn group_ack(groups: &[u32]) -> Event {
        group_ack_metadata(groups, 0, 0)
    }

    fn group_ack_metadata(groups: &[u32], class: u8, lifetime: u8) -> Event {
        let mut payload = PackedBits::new();
        payload.push(6, groups.len() as u32).unwrap();
        for &gssi in groups {
            payload.push(1, 0).unwrap();
            payload.push(2, u32::from(lifetime)).unwrap();
            payload.push(3, u32::from(class)).unwrap();
            payload.push(2, 0).unwrap();
            payload.push(24, gssi).unwrap();
        }
        let mut bits = PackedBits::new();
        bits.push(7, 0b1011001).unwrap();
        bits.push(1, 1).unwrap();
        bits.push(4, 7).unwrap();
        bits.push(11, u32::from(payload.len())).unwrap();
        bits.append(payload.view()).unwrap();
        bits.push(1, 0).unwrap();
        let mut input = event(DOWNLINK);
        input.pdu_bits = bits.len();
        input.pdu[..bits.as_bytes().len()].copy_from_slice(bits.as_bytes());
        input
    }

    #[test]
    fn v1_c_actions_and_status_never_expose_packed_attachment_metadata() {
        for class in 0..8 {
            for lifetime in 0..4 {
                let mut storage = Storage([0; STORAGE_SIZE as usize]);
                let pointer = storage.0.as_mut_ptr().cast();
                let mut activate = event(ACTIVATE);
                activate.flags = REGISTRATION_REQUIRED | SYSTEM_WIDE_SERVICES;
                // SAFETY: Storage and every input/output are live, exclusive and disjoint.
                unsafe {
                    assert_eq!(tetra_ms_mm_init(pointer, STORAGE_SIZE, &config()), OK);
                    assert_eq!(tetra_ms_mm_push_event(pointer, &activate), OK);
                    drain(pointer);
                    assert_eq!(tetra_ms_mm_push_event(pointer, &downlink("01010110")), OK);
                    drain(pointer);
                    let mut groups = event(GROUPS);
                    groups.group_count = 1;
                    groups.groups[0] = GroupRequest {
                        gssi: 0xff_ffff,
                        operation: 0,
                        value: class,
                        reserved: 0,
                    };
                    assert_eq!(tetra_ms_mm_push_event(pointer, &groups), OK);
                    drain(pointer);
                    assert_eq!(
                        tetra_ms_mm_push_event(pointer, &group_ack_metadata(&[0xff_ffff], class, lifetime)),
                        OK
                    );
                    let output = drain(pointer);
                    assert_eq!(output[1].kind, GROUP_CONFIRM);
                    assert_eq!(output[1].attached_count, 1);
                    assert_eq!(output[1].attached[0], 0xff_ffff);
                    assert!(output[1].attached[1..].iter().all(|gssi| *gssi == 0));
                    let before = status(pointer);
                    assert_eq!(before.attached[0], 0xff_ffff);
                    groups.groups[0].value = 8;
                    assert_eq!(tetra_ms_mm_push_event(pointer, &groups), INVALID_VALUE);
                    assert_eq!(status(pointer), before);
                    assert_eq!(tetra_ms_mm_reset(pointer, &config()), OK);
                    assert_eq!(status(pointer).attached_count, 0);
                    assert!(drain(pointer).is_empty());
                }
            }
        }
    }

    #[test]
    fn caller_storage_validation_failed_reset_and_poll_are_atomic() {
        let mut storage = Storage([0xa5; STORAGE_SIZE as usize]);
        let pointer = storage.0.as_mut_ptr().cast();
        let mut cfg = config();
        let mut input = event(REGISTER);
        let mut output = action();
        // SAFETY: Every pointer is live, aligned, and disjoint unless deliberately
        // rejected before dereferencing; malformed headers do not shorten objects.
        unsafe {
            assert_eq!(tetra_ms_mm_push_event(pointer, &input), UNINITIALIZED);
            assert_eq!(tetra_ms_mm_init(ptr::null_mut(), STORAGE_SIZE, &cfg), ARGUMENT);
            assert_eq!(tetra_ms_mm_init(pointer, STORAGE_SIZE, ptr::null()), ARGUMENT);
            assert_eq!(tetra_ms_mm_init(pointer, STORAGE_SIZE - 1, &cfg), CAPACITY);
            assert_eq!(tetra_ms_mm_init(storage.0.as_mut_ptr().add(1).cast(), STORAGE_SIZE, &cfg), ARGUMENT);
            cfg.own_issi = 0x100_0000;
            assert_eq!(tetra_ms_mm_init(pointer, STORAGE_SIZE, &cfg), INVALID_IDENTITY);
            assert!(storage.0.iter().all(|&value| value == 0xa5));
            cfg.own_issi = 1_000_001;
            assert_eq!(tetra_ms_mm_init(pointer, STORAGE_SIZE, &cfg), OK);
            let (native_config, native_groups) = super::config(&cfg).unwrap();
            let expected = MmCore::new(native_config, native_groups).unwrap();
            assert_eq!((*state_ptr(pointer).unwrap()).core.state(), expected.state());
            assert_eq!(tetra_ms_mm_push_event(pointer, &input), OK);
            let before = status(pointer);
            cfg.group_count = 64;
            assert_eq!(tetra_ms_mm_reset(pointer, &cfg), GROUP_CAPACITY);
            assert_eq!(status(pointer), before);
            output.size = 0;
            assert_eq!(tetra_ms_mm_poll_action(pointer, &mut output), ABI_MISMATCH);
            assert_eq!(status(pointer), before);
            assert_eq!(tetra_ms_mm_poll_action(pointer, pointer.cast()), ARGUMENT);
            assert_eq!(tetra_ms_mm_push_event(pointer, pointer.cast()), ARGUMENT);
            assert_eq!(tetra_ms_mm_reset(pointer, pointer.cast()), ARGUMENT);
            input.size = 0;
            assert_eq!(tetra_ms_mm_push_event(pointer, &input), ABI_MISMATCH);
            assert_eq!(status(pointer), before);
            cfg.group_count = 2;
            cfg.groups[..2].copy_from_slice(&[220, 91]);
            assert_eq!(tetra_ms_mm_reset(pointer, &cfg), OK);
            let (native_config, native_groups) = super::config(&cfg).unwrap();
            let expected = MmCore::new(native_config, native_groups).unwrap();
            assert_eq!((*state_ptr(pointer).unwrap()).core.state(), expected.state());
            cfg.groups[0] = 999;
            let reset = status(pointer);
            assert_eq!(reset.registration, 0);
            assert_eq!(reset.actions, 0);
            assert_eq!(reset.attached_count, 2);
            assert_eq!(&reset.attached[..2], &[91, 220]);
            assert_ne!(reset.attached[0], cfg.groups[0]);
        }
    }

    #[test]
    fn group_wire_order_cells_and_complete_status_survive_the_abi() {
        let mut storage = Storage([0; STORAGE_SIZE as usize]);
        let pointer = storage.0.as_mut_ptr().cast();
        let mut activate = event(ACTIVATE);
        activate.flags = REGISTRATION_REQUIRED | SYSTEM_WIDE_SERVICES;
        // SAFETY: Tests own initialized, properly aligned, disjoint objects.
        unsafe {
            assert_eq!(tetra_ms_mm_init(pointer, STORAGE_SIZE, &config()), OK);
            assert_eq!(tetra_ms_mm_push_event(pointer, &activate), OK);
            let sent = drain(pointer);
            assert_eq!(sent[0].kind, SEND);
            assert_eq!(sent[0].pdu_bits, 10);
            assert_eq!(sent[0].pdu[0], 0x26);
            assert_eq!(tetra_ms_mm_push_event(pointer, &downlink("01010110")), OK);
            let accepted = drain(pointer);
            assert_eq!(accepted[0].kind, IDENTITIES);
            assert_eq!(accepted[1].kind, REGISTRATION_SUCCESS);
            let mut groups = event(GROUPS);
            groups.handle = 42;
            groups.flags = DETACH_ALL | REPORT_REQUESTED;
            groups.group_count = 2;
            groups.groups[0] = GroupRequest {
                gssi: 900,
                operation: 0,
                value: 3,
                reserved: 0,
            };
            groups.groups[1] = GroupRequest {
                gssi: 300,
                operation: 0,
                value: 4,
                reserved: 0,
            };
            assert_eq!(tetra_ms_mm_push_event(pointer, &groups), OK);
            let pending = status(pointer);
            assert_eq!(pending.registration, 2);
            assert_eq!(pending.group_handle, 42);
            assert_eq!(pending.t353_countdown, 706);
            assert_eq!(
                pending.flags,
                HAS_GROUP_OPERATION
                    | STATUS_DETACH_ALL
                    | STATUS_REPORT_REQUESTED
                    | HAS_CURRENT_CELL
                    | HAS_REGISTERED_CELL
                    | HAS_PENDING_CELL
            );
            assert_eq!(
                pending.current_cell,
                Cell {
                    mcc: 234,
                    mnc: 5678,
                    la: 100,
                    reserved: 0
                }
            );
            assert_eq!(pending.registered_cell, pending.current_cell);
            assert_eq!(pending.pending_cell, pending.current_cell);
            assert_eq!(tetra_ms_mm_push_event(pointer, &groups), GROUP_OPERATION_PENDING);
            drain(pointer);
            let mut ack = group_ack(&[900, 300, 900]);
            assert_eq!(tetra_ms_mm_push_event(pointer, &ack), OK);
            ack.pdu.fill(0xff);
            let results = drain(pointer);
            assert_eq!(results[0].kind, IDENTITIES);
            assert_eq!(results[0].attached_count, 2);
            assert_eq!(&results[0].attached[..2], &[300, 900]);
            assert_eq!(results[1].kind, GROUP_CONFIRM);
            assert_eq!(results[1].handle, 42);
            assert_eq!(results[1].flags, ACTION_DETACH_ALL | ACTION_REPORT_REQUESTED);
            assert_eq!(results[1].attached_count, 3);
            assert_eq!(&results[1].attached[..3], &[900, 300, 900]);
            assert_eq!(status(pointer).flags & HAS_GROUP_OPERATION, 0);
        }
    }

    #[test]
    fn protocol_errors_rejection_lockout_and_backpressure_are_not_collapsed() {
        let mut storage = Storage([0; STORAGE_SIZE as usize]);
        let pointer = storage.0.as_mut_ptr().cast();
        // SAFETY: Test pointers refer to disjoint live objects, initialized below.
        unsafe {
            assert_eq!(tetra_ms_mm_init(pointer, STORAGE_SIZE, &config()), OK);
            assert_eq!(tetra_ms_mm_push_event(pointer, &event(DEREGISTER)), NOT_REGISTERED);
            assert_eq!(tetra_ms_mm_push_event(pointer, &event(REGISTER)), OK);
            assert_eq!(tetra_ms_mm_push_event(pointer, &event(REGISTER)), REGISTRATION_IN_PROGRESS);
            assert_eq!(tetra_ms_mm_push_event(pointer, &downlink("1001")), UNSUPPORTED);
            assert_eq!(tetra_ms_mm_push_event(pointer, &downlink("010")), TRUNCATED);
            assert_eq!(tetra_ms_mm_push_event(pointer, &event(LINK_BREAK)), OK);
            assert_eq!(tetra_ms_mm_push_event(pointer, &event(LINK_REOPEN)), OK);
            let before = status(pointer);
            assert_eq!(tetra_ms_mm_push_event(pointer, &event(LINK_BREAK)), QUEUE_FULL);
            assert_eq!(status(pointer), before);
            assert_eq!(tetra_ms_mm_reset(pointer, &config()), OK);
            for count in 1..=4 {
                assert_eq!(tetra_ms_mm_push_event(pointer, &event(REGISTER)), OK);
                drain(pointer);
                assert_eq!(tetra_ms_mm_push_event(pointer, &downlink("01110110000100")), OK);
                let failed = drain(pointer);
                assert_eq!(failed.len(), 1);
                assert_eq!(failed[0].kind, REGISTRATION_FAILURE);
                assert_eq!(failed[0].flags, HAS_REJECT_CAUSE);
                assert_eq!(failed[0].reject_cause, 1);
                assert_eq!(status(pointer).system_rejection_count, count);
            }
            assert_ne!(status(pointer).flags & HAS_LEFT_SYSTEM, 0);
            assert_eq!(tetra_ms_mm_push_event(pointer, &event(REGISTER)), LEFT_SYSTEM);
            assert_eq!(tetra_ms_mm_reset(pointer, &config()), OK);
            assert_eq!(status(pointer).flags, 0);
        }
    }

    #[test]
    fn header_only_initialized_output_is_fully_written_on_success() {
        let mut storage = Storage([0; STORAGE_SIZE as usize]);
        let pointer = storage.0.as_mut_ptr().cast();
        let mut output = MaybeUninit::<Action>::uninit();
        let mut snapshot = MaybeUninit::<Status>::uninit();
        // SAFETY: Only integer headers are initialized before the call. The API
        // guarantees all remaining output fields are written before OK returns.
        unsafe {
            assert_eq!(tetra_ms_mm_init(pointer, STORAGE_SIZE, &config()), crate::OK);
            assert_eq!(tetra_ms_mm_push_event(pointer, &event(REGISTER)), crate::OK);
            ptr::addr_of_mut!((*output.as_mut_ptr()).size).write(size_of::<Action>() as u32);
            ptr::addr_of_mut!((*output.as_mut_ptr()).abi_version).write(ABI_VERSION);
            assert_eq!(tetra_ms_mm_poll_action(pointer, output.as_mut_ptr()), crate::OK);
            let output = output.assume_init();
            assert_eq!(output.kind, SEND);
            assert_eq!(output.pdu_bits, 10);
            assert!(output.pdu[2..].iter().all(|&value| value == 0));
            assert!(output.attached.iter().all(|&value| value == 0));
            ptr::addr_of_mut!((*snapshot.as_mut_ptr()).size).write(size_of::<Status>() as u32);
            ptr::addr_of_mut!((*snapshot.as_mut_ptr()).abi_version).write(ABI_VERSION);
            assert_eq!(tetra_ms_mm_get_status(pointer, snapshot.as_mut_ptr()), crate::OK);
            let snapshot = snapshot.assume_init();
            assert_eq!(snapshot.registration, 1);
            assert_eq!(snapshot.actions, 1);
            assert_eq!(snapshot.flags, 0);
            assert_eq!(
                snapshot.current_cell,
                Cell {
                    mcc: 0,
                    mnc: 0,
                    la: 0,
                    reserved: 0
                }
            );
        }
    }

    #[test]
    fn caller_time_group_expiry_retry_and_failed_detach_preserve_core_semantics() {
        let mut storage = Storage([0; STORAGE_SIZE as usize]);
        let pointer = storage.0.as_mut_ptr().cast();
        let mut tick = event(TICK);
        // SAFETY: All objects are live, aligned and disjoint; storage is
        // initialized below and used serially throughout this test.
        unsafe {
            assert_eq!(tetra_ms_mm_init(pointer, STORAGE_SIZE, &config()), OK);
            assert_eq!(tetra_ms_mm_push_event(pointer, &event(REGISTER)), OK);
            drain(pointer);
            tick.elapsed_slots = 706;
            assert_eq!(tetra_ms_mm_push_event(pointer, &tick), OK);
            assert!(drain(pointer).is_empty());
            assert_eq!(status(pointer).t351_countdown, 0);
            assert_eq!(status(pointer).attempts, 1);
            tick.elapsed_slots = 1;
            assert_eq!(tetra_ms_mm_push_event(pointer, &tick), OK);
            let retry = drain(pointer);
            assert_eq!(retry[0].kind, SEND);
            assert_eq!(status(pointer).attempts, 2);
            assert_eq!(tetra_ms_mm_push_event(pointer, &downlink("01010110")), OK);
            drain(pointer);
            let mut groups = event(GROUPS);
            groups.handle = 17;
            groups.group_count = 1;
            groups.groups[0] = GroupRequest {
                gssi: 900,
                operation: 0,
                value: 8,
                reserved: 0,
            };
            assert_eq!(tetra_ms_mm_push_event(pointer, &groups), INVALID_VALUE);
            assert_eq!(status(pointer).flags & HAS_GROUP_OPERATION, 0);
            groups.groups[0].operation = 1;
            groups.groups[0].value = 2;
            assert_eq!(tetra_ms_mm_push_event(pointer, &groups), OK);
            assert_eq!(drain(pointer)[0].pdu_bits, 59);
            tick.elapsed_slots = 707;
            assert_eq!(tetra_ms_mm_push_event(pointer, &tick), OK);
            let expired = drain(pointer);
            assert_eq!(expired.len(), 1);
            assert_eq!(expired[0].kind, GROUP_CONFIRM);
            assert_eq!(expired[0].handle, 17);
            assert_eq!(expired[0].t353_countdown, 0);
            assert_eq!(expired[0].attached_count, 0);
            assert_eq!(expired[0].detached_count, 0);
            assert_eq!(tetra_ms_mm_push_event(pointer, &event(DEREGISTER)), OK);
            drain(pointer);
            tick.elapsed_slots = 1;
            tick.transfer_state = 3;
            assert_eq!(tetra_ms_mm_push_event(pointer, &tick), OK);
            let failed = drain(pointer);
            assert_eq!(failed[0].kind, DETACH_REPORT);
            assert_eq!(failed[0].flags, 0);
            assert_eq!(failed[1].kind, DETACH_COMPLETE);
            assert_eq!(status(pointer).registration, 0);
        }
    }

    #[cfg(feature = "c-smoke")]
    #[test]
    fn linked_mm_c_caller() {
        unsafe extern "C" {
            fn tetra_ms_mm_c_smoke() -> i32;
        }
        // SAFETY: Linked test has no parameters and uses only the public C API.
        assert_eq!(unsafe { tetra_ms_mm_c_smoke() }, 0);
    }
}
