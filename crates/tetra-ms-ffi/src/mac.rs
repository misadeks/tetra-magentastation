//! Versioned MAC/ALOHA boundary. Reassembly deliberately remains a separate ABI.
use crate::{
    ABI_MISMATCH, ARGUMENT, CAPACITY, EMPTY, INVALID_IDENTITY, INVALID_TIME, QUEUE_FULL, TRUNCATED, UNINITIALIZED, UNSUPPORTED, address,
    address_kind, boundary, disjoint_region, valid_ptr,
};
use core::{
    ffi::c_void,
    mem::{MaybeUninit, align_of, size_of},
    ptr,
};
use tetra_ms_core::{
    mac::{self, AccessCompletion, AllocationResult, Channel, FragmentAction, GrantCapacity, MsMac, ReservedAction, downlink, wire},
    random_access::{self as ra, AccessCode, AccessParamStore, MsRandomAccess, RaAction, RaFailure},
    types::{BitError, BitView, TdmaTime, tdma_time::TIME_INT_WRAP},
};

pub const ABI_VERSION: u32 = 1;
pub const STORAGE_SIZE: u32 = 32768;
pub const STORAGE_ALIGN: u32 = 4;
pub const PDU_BYTES: usize = 512;
pub const ACTION_CAPACITY: usize = 8;
pub const BIT_INVALID_WIDTH: i32 = -260;
pub const BIT_INVALID_VALUE: i32 = -261;
const MAGIC: u32 = 0x4d41_4301;

pub const SUBMIT: u32 = 1;
pub const TICK: u32 = 2;
pub const AACH: u32 = 3;
pub const DOWNLINK: u32 = 4;
pub const SYNC: u32 = 5;
pub const IDENTITIES: u32 = 6;
pub const UPLANE: u32 = 7;
pub const AUDIO: u32 = 8;
pub const ALLOCATE: u32 = 9;
pub const BIND_MARKER: u32 = 10;
pub const TRAFFIC_OWNER: u32 = 11;
pub const CANCEL_ACCESS: u32 = 12;
pub const SET_TIME: u32 = 13;
pub const SET_PARAMS: u32 = 14;
pub const RA_INITIATE: u32 = 15;
pub const RA_POLL: u32 = 16;
pub const RA_RESPONSE: u32 = 17;
pub const RA_CANCEL: u32 = 18;
pub const GRANT_FRAGMENT: u32 = 19;
pub const POLL_RESERVED: u32 = 20;
pub const POLL_TRAFFIC: u32 = 21;
pub const PROMOTE: u32 = 22;
pub const ADVANCE: u32 = 23;
pub const CHECK_ADDRESS: u32 = 24;
pub const ENCODE: u32 = 25;
pub const SCRAMBLING: u32 = 26;
pub const CONTINUE_BLOCK: u32 = 27;

pub const E_RECEIPT: u32 = 1;
pub const E_PRIORITY: u32 = 2;
pub const E_EMERGENCY: u32 = 4;
pub const E_STEALING: u32 = 8;
pub const E_INDIVIDUAL: u32 = 16;
pub const E_GROUPS: u32 = 32;
pub const E_SWITCH: u32 = 64;
pub const E_TX_GRANT: u32 = 128;
pub const E_UL_VALID: u32 = 256;
pub const E_ADDRESS_MATCH: u32 = 512;
pub const E_RA_SUCCESS: u32 = 1024;
pub const E_RECOVERED: u32 = 2048;
pub const E_CLEAR_RECOVERED: u32 = 4096;

pub const C_FILTERED: u32 = 1;
pub const C_INDIVIDUAL: u32 = 2;
pub const C_NETWORK: u32 = 4;
pub const C_COLOUR: u32 = 8;
pub const C_SCRAMBLING: u32 = 16;
pub const C_CARRIER: u32 = 32;
pub const C_RECOVERED: u32 = 64;

pub const TRANSMIT: u32 = 1;
pub const SUBMITTED: u32 = 2;
pub const DISCARDED_TRANSFER: u32 = 3;
pub const DISCARDED_STOLEN: u32 = 4;
pub const RA_TRANSMIT: u32 = 5;
pub const RA_SUCCEEDED: u32 = 6;
pub const RA_FAILED: u32 = 7;
pub const RESOURCE: u32 = 8;
pub const FRAGMENT: u32 = 9;
pub const END: u32 = 10;
pub const SYNCED: u32 = 11;
pub const SYSINFO: u32 = 12;
pub const ACCESS_PARAMS: u32 = 13;
pub const CONFIGURE: u32 = 14;
pub const ALLOCATION: u32 = 15;
pub const OWNER: u32 = 16;
pub const AUDIO_DROPPED: u32 = 17;
pub const CANCELLED: u32 = 18;
pub const FRAGMENT_REJECTED: u32 = 19;
pub const PLAN_INSTALLED: u32 = 20;
pub const RESERVED_MISSED: u32 = 21;
pub const ADDRESS_RESULT: u32 = 22;
pub const ENCODED: u32 = 23;
pub const AACH_INFO: u32 = 24;
pub const NEXT_PDU: u32 = 25;

pub const F_RECEIPT: u32 = 1;
pub const F_RECEIPT2: u32 = 2;
pub const F_RESERVED: u32 = 4;
pub const F_TRAFFIC: u32 = 8;
pub const F_SECOND: u32 = 16;
pub const F_PRIORITY: u32 = 32;
pub const F_EMERGENCY: u32 = 64;
pub const F_ADDRESS: u32 = 128;
pub const F_EVENT_LABEL: u32 = 256;
pub const F_MARKER: u32 = 512;
pub const F_GRANT: u32 = 1024;
pub const F_ALLOCATION: u32 = 2048;
pub const F_TRUNCATED: u32 = 4096;
pub const F_SECOND_STOLEN: u32 = 8192;
pub const F_ACKS_OWN: u32 = 16384;
pub const F_HYPERFRAME: u32 = 32768;
pub const F_DEFAULT_ACCESS: u32 = 65536;
pub const F_APPLIED: u32 = 131072;
pub const F_SCRAMBLING: u32 = 262144;
pub const F_MAC_BLOCK: u32 = 524288;
pub const F_ASSIGNED_CONTROL: u32 = 1048576;
pub const F_RANDOM_ACCESS: u32 = 2097152;
pub const F_FILL: u32 = 4194304;
pub const F_ALLOCATION_RESULT: u32 = 8388608;
pub const F_ACCESS: u32 = 16777216;
pub const S_RECOVERED: u32 = 1;
pub const S_INDIVIDUAL: u32 = 2;
pub const S_NETWORK: u32 = 4;
pub const S_COLOUR: u32 = 8;
pub const S_SCRAMBLING: u32 = 16;
pub const S_CARRIER: u32 = 32;
pub const S_TRAFFIC_OWNER: u32 = 64;
pub const S_MARKER: u32 = 128;
pub const S_PENDING: u32 = 256;
pub const S_FRAGMENT: u32 = 512;
pub const S_RESERVED: u32 = 1024;
pub const S_PENDING_RECEIPT: u32 = 2048;
pub const S_FRAGMENT_RECEIPT: u32 = 4096;
pub const S_PRIORITY: u32 = 8192;
pub const S_RA_ACTIVE: u32 = 16384;
pub const S_FILTERED: u32 = 32768;
pub const S_RESERVED_RECEIPT: u32 = 65536;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Params {
    /// bit 0 subscriber class present; bit 1 GSSI present.
    pub flags: u32,
    pub imm: u32,
    pub wt: u32,
    pub nu: u32,
    pub fl_factor: u32,
    pub ts_ptr: u32,
    pub min_priority: u32,
    pub subscriber_class: u32,
    pub gssi: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct SlotAccess {
    pub code_present: u32,
    pub code: u32,
    /// 0 reserved, 1 CLCH, 2 ongoing frame, 3 frame marker.
    pub frame_kind: u32,
    pub frame_length: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Grant {
    /// 0 first subslot, 1 second subslot, 2 full slots.
    pub kind: u32,
    pub slots: u32,
    pub next_opportunity: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Allocation {
    pub carrier: u32,
    /// bit 0 extended, bit 1 cell change, bit 2 downlink bearing.
    pub flags: u32,
    /// 0 replace, 1 additional.
    pub kind: u32,
    pub slot_bitmap: u32,
    pub owner: u32,
}

#[repr(C)]
pub struct Config {
    pub size: u32,
    pub abi_version: u32,
    pub own_issi: u32,
    pub flags: u32,
    pub now: u32,
    pub recovered_now: u32,
    pub mcc: u32,
    pub mnc: u32,
    pub colour: u32,
    pub scrambling_code: u32,
    pub serving_carrier: u32,
    pub group_count: u32,
    pub groups: [u32; mac::MAX_ATTACHED_GROUPS],
}

#[repr(C)]
pub struct Event {
    pub size: u32,
    pub abi_version: u32,
    pub kind: u32,
    pub now: u32,
    pub flags: u32,
    pub receipt: u32,
    pub ssi: u32,
    pub ssi_type: u32,
    pub priority: u32,
    pub code: u32,
    pub scrambling_code: u32,
    pub mcc: u32,
    pub mnc: u32,
    pub source: u32,
    pub value: u32,
    pub choice_index: u32,
    pub choice_subslot: u32,
    pub group_count: u32,
    pub pdu_bits: u32,
    pub allocation: Allocation,
    pub params: Params,
    pub grant: Grant,
    pub subslots: [SlotAccess; 2],
    pub groups: [u32; mac::MAX_ATTACHED_GROUPS],
    pub pdu: [u8; PDU_BYTES],
}

#[repr(C)]
pub struct Action {
    pub size: u32,
    pub abi_version: u32,
    pub kind: u32,
    pub flags: u32,
    pub now: u32,
    pub receipt: u32,
    pub receipt2: u32,
    pub reason: i32,
    pub code: u32,
    pub subslot: u32,
    pub channel: u32,
    pub channel2: u32,
    pub scrambling_code: u32,
    pub scrambling_code2: u32,
    pub route: u32,
    pub ssi: u32,
    pub ssi_type: u32,
    pub event_label: u32,
    pub marker: u32,
    pub encryption: u32,
    pub length_ind: u32,
    pub header_bits: u32,
    pub payload_end: u32,
    pub next_bits: u32,
    pub fill_bits: u32,
    pub allocation_result: u32,
    pub mcc: u32,
    pub mnc: u32,
    pub colour: u32,
    pub carrier: u32,
    pub band: u32,
    pub offset: u32,
    pub duplex: u32,
    pub reverse: u32,
    pub option: u32,
    pub option_bits: u32,
    pub fragment_kind: u32,
    pub fragment_slots: u32,
    pub pdu_bits: u32,
    pub pdu2_bits: u32,
    pub hyperframe: u32,
    pub grant: Grant,
    pub allocation: Allocation,
    pub params: Params,
    pub subslots: [SlotAccess; 2],
    pub pdu: [u8; PDU_BYTES],
    pub pdu2: [u8; PDU_BYTES],
}

#[repr(C)]
pub struct Entropy {
    pub size: u32,
    pub abi_version: u32,
    /// 0 none, 1 uniform index in 1..=maximum, 2 choose subslot 1 or 2.
    pub kind: u32,
    pub maximum: u32,
    /// Nonzero when automatic admission will fail, not an entropy failure.
    pub admission_failure: i32,
}

#[derive(Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Status {
    pub size: u32,
    pub abi_version: u32,
    pub flags: u32,
    pub now: u32,
    pub recovered_now: u32,
    pub own_issi: u32,
    pub individual_ssi: u32,
    pub mcc: u32,
    pub mnc: u32,
    pub colour: u32,
    pub scrambling_code: u32,
    pub serving_carrier: u32,
    pub traffic_owner: u32,
    pub current_marker: u32,
    pub transfer_status: u32,
    pub queued_uplinks: u32,
    pub queued_stolen: u32,
    pub queued_audio: u32,
    pub assigned_slot_bitmap: u32,
    pub tx_granted: u32,
    pub actions: u32,
    pub ra_phase: u32,
    pub ra_code: u32,
    pub ra_emergency: u32,
    pub ra_transmissions: u32,
    pub group_count: u32,
    pub defined_mask: u32,
    pub sysinfo_present: u32,
    pub param_mask: u32,
    pub pending_receipt: u32,
    pub fragment_receipt: u32,
    pub pending_priority: u32,
    pub pending_emergency: u32,
    pub fragment_kind: u32,
    pub fragment_slots: u32,
    pub pending_bits: u32,
    pub fragment_bits: u32,
    pub reserved_count: u32,
    pub pending_channel: u32,
    pub pending_scrambling_code: u32,
    pub fragment_scrambling_code: u32,
    pub reserved_scrambling_code: u32,
    pub reserved_receipt: u32,
    pub reserved_times: [u32; wire::MAX_UL_FRAG_SLOTS],
    pub params: [Params; 4],
    pub groups: [u32; mac::MAX_ATTACHED_GROUPS],
    pub marker_mask: [u32; 8],
    pub marker_owners: [u32; 256],
    pub pending_pdu: [u8; PDU_BYTES],
    pub fragment_pdu: [u8; PDU_BYTES],
}

#[repr(C)]
struct State {
    magic: u32,
    own_issi: u32,
    filtered: u32,
    defined_mask: u32,
    sysinfo_present: u32,
    core: MsMac,
    ra: MsRandomAccess,
    params: AccessParamStore,
    actions: [MaybeUninit<Action>; ACTION_CAPACITY],
    head: u8,
    len: u8,
}

const _: () = {
    assert!(!core::mem::needs_drop::<State>());
    assert!(size_of::<State>() <= STORAGE_SIZE as usize);
    assert!(align_of::<State>() <= STORAGE_ALIGN as usize);
    assert!(align_of::<Action>() == 4 && align_of::<Event>() == 4);
    assert!(size_of::<Params>() == 36 && size_of::<SlotAccess>() == 16);
    assert!(size_of::<Grant>() == 12 && size_of::<Allocation>() == 20);
    assert!(size_of::<Config>() == 1072);
    assert!(size_of::<Event>() == 1712);
    assert!(size_of::<Action>() == 1288);
    assert!(size_of::<Entropy>() == 20);
    assert!(size_of::<Status>() == 3444);
    assert!(core::mem::offset_of!(Config, groups) == 48);
    assert!(core::mem::offset_of!(Event, pdu) == 1200);
    assert!(core::mem::offset_of!(Action, pdu) == 264);
    assert!(core::mem::offset_of!(Action, pdu2) == 776);
    assert!(core::mem::offset_of!(Status, groups) == 340);
    assert!(core::mem::offset_of!(Status, pending_pdu) == 2420);
};

fn bit_error(error: BitError) -> i32 {
    match error {
        BitError::Truncated => TRUNCATED,
        BitError::Capacity => CAPACITY,
        BitError::InvalidWidth => BIT_INVALID_WIDTH,
        BitError::InvalidValue => BIT_INVALID_VALUE,
    }
}

fn ra_error(error: RaFailure) -> i32 {
    -200 - match error {
        RaFailure::InvalidTime => 0,
        RaFailure::InvalidParameters => 1,
        RaFailure::InvalidEntropy => 2,
        RaFailure::Busy => 3,
        RaFailure::NoValidAccessCode => 4,
        RaFailure::AccessCodeNotPermitted => 5,
        RaFailure::MaxTransmissions => 6,
    }
}

fn decode_error(error: downlink::DecodeError) -> i32 {
    match error {
        downlink::DecodeError::Bits(error) => bit_error(error),
        downlink::DecodeError::Unsupported(kind) => -240 - kind as i32,
    }
}

fn time(raw: u32) -> Result<TdmaTime, i32> {
    if raw >= TIME_INT_WRAP as u32 {
        Err(INVALID_TIME)
    } else {
        Ok(TdmaTime::from_int(raw as i32))
    }
}

fn params(input: Params) -> Result<ra::RandomAccessParams, i32> {
    if input.flags & !3 != 0
        || input.fl_factor > 1
        || input.subscriber_class > u16::MAX as u32
        || input.imm > 15
        || input.wt > 15
        || input.nu > 15
        || input.ts_ptr > 15
        || input.min_priority > 7
        || input.gssi > 0xff_ffff
    {
        return Err(ARGUMENT);
    }
    Ok(ra::RandomAccessParams {
        imm: input.imm as u8,
        wt: input.wt as u8,
        nu: input.nu as u8,
        fl_factor: input.fl_factor != 0,
        ts_ptr: input.ts_ptr as u8,
        min_pdu_prio: input.min_priority as u8,
        subscriber_class: (input.flags & 1 != 0).then_some(input.subscriber_class as u16),
        gssi: (input.flags & 2 != 0).then_some(input.gssi),
    })
}

fn c_params(input: &ra::RandomAccessParams) -> Params {
    Params {
        flags: u32::from(input.subscriber_class.is_some()) | (u32::from(input.gssi.is_some()) << 1),
        imm: u32::from(input.imm),
        wt: u32::from(input.wt),
        nu: u32::from(input.nu),
        fl_factor: u32::from(input.fl_factor),
        ts_ptr: u32::from(input.ts_ptr),
        min_priority: u32::from(input.min_pdu_prio),
        subscriber_class: u32::from(input.subscriber_class.unwrap_or(0)),
        gssi: input.gssi.unwrap_or(0),
    }
}

fn access_code(raw: u32) -> Result<AccessCode, i32> {
    if raw > 3 {
        return Err(ARGUMENT);
    }
    AccessCode::from_raw(raw as u8).ok_or(ARGUMENT)
}

fn grant(input: Grant) -> Result<mac::Grant, i32> {
    if input.next_opportunity > 1 {
        return Err(ARGUMENT);
    }
    let capacity = match input.kind {
        0 => GrantCapacity::FirstSubslot,
        1 => GrantCapacity::SecondSubslot,
        2 if input.slots <= u8::MAX as u32 => GrantCapacity::Slots(input.slots as u8),
        _ => return Err(ARGUMENT),
    };
    Ok(mac::Grant {
        next_opportunity: input.next_opportunity != 0,
        capacity,
    })
}

fn c_grant(input: mac::Grant) -> Grant {
    let (kind, slots) = match input.capacity {
        GrantCapacity::FirstSubslot => (0, 0),
        GrantCapacity::SecondSubslot => (1, 0),
        GrantCapacity::Slots(count) => (2, u32::from(count)),
    };
    Grant {
        kind,
        slots,
        next_opportunity: u32::from(input.next_opportunity),
    }
}

fn allocation(input: Allocation) -> Result<mac::Allocation, i32> {
    if input.carrier > 4095 || input.flags & !7 != 0 || input.kind > 1 || input.slot_bitmap > 15 || input.owner > 0xff_ffff {
        return Err(ARGUMENT);
    }
    Ok(mac::Allocation {
        carrier: input.carrier as u16,
        extended: input.flags & 1 != 0,
        cell_change: input.flags & 2 != 0,
        downlink: input.flags & 4 != 0,
        kind: if input.kind == 0 {
            mac::AllocationType::Replace
        } else {
            mac::AllocationType::Additional
        },
        slots: core::array::from_fn(|i| input.slot_bitmap & (8 >> i) != 0),
        owner: input.owner,
    })
}

fn c_allocation(input: mac::Allocation) -> Allocation {
    Allocation {
        carrier: u32::from(input.carrier),
        flags: u32::from(input.extended) | (u32::from(input.cell_change) << 1) | (u32::from(input.downlink) << 2),
        kind: u32::from(input.kind == mac::AllocationType::Additional),
        slot_bitmap: input.slots.iter().fold(0, |bits, &slot| (bits << 1) | u32::from(slot)),
        owner: input.owner,
    }
}

fn allocation_result(result: AllocationResult) -> u32 {
    match result {
        AllocationResult::Applied => 0,
        AllocationResult::CrossCell => 1,
        AllocationResult::CrossCarrier => 2,
        AllocationResult::UnknownCarrier => 3,
        AllocationResult::UplinkOnly => 4,
        AllocationResult::IndividualEngaged => 5,
    }
}

fn channel(channel: Channel) -> u32 {
    match channel {
        Channel::SchHu => 1,
        Channel::SchF => 2,
        Channel::Stch => 3,
        Channel::TchS => 4,
    }
}

fn fragment_kind(kind: wire::FragEndKind) -> (u32, u32) {
    match kind {
        wire::FragEndKind::MacEndHu => (1, 0),
        wire::FragEndKind::MacEndUl => (2, 1),
        wire::FragEndKind::MacFragUl { total_slots } => (3, u32::from(total_slots)),
    }
}

fn copy_bits(out: &mut [u8; PDU_BYTES], bits: BitView<'_>) -> u32 {
    for (index, byte) in out[..usize::from(bits.len()).div_ceil(8)].iter_mut().enumerate() {
        let offset = index as u16 * 8;
        let width = (bits.len() - offset).min(8) as u8;
        *byte = (bits.read(offset, width).expect("checked MAC bit window") << (8 - width)) as u8;
    }
    u32::from(bits.len())
}

impl State {
    fn reserve(&self, count: u8) -> Result<(), i32> {
        if usize::from(self.len) + usize::from(count) > ACTION_CAPACITY {
            Err(QUEUE_FULL)
        } else {
            Ok(())
        }
    }

    fn action(&mut self, kind: u32) -> &mut Action {
        debug_assert!(usize::from(self.len) < ACTION_CAPACITY);
        let index = (usize::from(self.head) + usize::from(self.len)) % ACTION_CAPACITY;
        let out = self.actions[index].as_mut_ptr();
        // SAFETY: Reserved queue capacity and exclusive State access designate a
        // writable slot. All Action fields are zero-valid fixed-width integers.
        unsafe {
            ptr::write_bytes(out, 0, 1);
            self.len += 1;
            (*out).size = size_of::<Action>() as u32;
            (*out).abi_version = ABI_VERSION;
            (*out).kind = kind;
            &mut *out
        }
    }
}

unsafe fn state_ptr(storage: *const c_void) -> Result<*mut State, i32> {
    let state = storage.cast::<State>();
    valid_ptr(state)?;
    // SAFETY: Only the aligned initialized integer header is read initially.
    if unsafe { ptr::addr_of!((*state).magic).read() } != MAGIC {
        return Err(UNINITIALIZED);
    }
    Ok(state.cast_mut())
}

unsafe fn check_header<T>(input: *const T) -> Result<(), i32> {
    #[repr(C)]
    struct Header {
        size: u32,
        version: u32,
    }
    valid_ptr(input)?;
    let header = input.cast::<Header>();
    // SAFETY: The caller guarantees readable initialized integer headers.
    if unsafe { ptr::addr_of!((*header).size).read() } != size_of::<T>() as u32
        || unsafe { ptr::addr_of!((*header).version).read() } != ABI_VERSION
    {
        return Err(ABI_MISMATCH);
    }
    Ok(())
}

fn validate_config(input: &Config) -> Result<(), i32> {
    if input.flags & !(C_FILTERED | C_INDIVIDUAL | C_NETWORK | C_COLOUR | C_SCRAMBLING | C_CARRIER | C_RECOVERED) != 0 {
        return Err(ARGUMENT);
    }
    if input.own_issi > 0xff_ffff || input.groups[..(input.group_count.min(256) as usize)].iter().any(|&v| v > 0xff_ffff) {
        return Err(INVALID_IDENTITY);
    }
    if input.group_count > 256 {
        return Err(CAPACITY);
    }
    if input.mcc > 1023 || input.mnc > 16383 || input.colour > 63 || input.serving_carrier > 4095 {
        return Err(ARGUMENT);
    }
    time(input.now)?;
    if input.flags & C_RECOVERED != 0 {
        time(input.recovered_now)?;
    }
    Ok(())
}

unsafe fn initialize(state: *mut State, config: &Config) {
    // SAFETY: Config is validated and disjoint, and the caller owns sufficient
    // aligned storage. Initialize fields without a full State/MAC temporary.
    unsafe {
        ptr::addr_of_mut!((*state).own_issi).write(config.own_issi);
        ptr::addr_of_mut!((*state).filtered).write(u32::from(config.flags & C_FILTERED != 0));
        ptr::addr_of_mut!((*state).defined_mask).write(0);
        ptr::addr_of_mut!((*state).sysinfo_present).write(0);
        MsMac::init_in_place(&mut *ptr::addr_of_mut!((*state).core).cast::<MaybeUninit<MsMac>>());
        ptr::addr_of_mut!((*state).ra).write(MsRandomAccess::new());
        ptr::addr_of_mut!((*state).params).write(AccessParamStore::new());
        let actions = ptr::addr_of_mut!((*state).actions).cast::<MaybeUninit<Action>>();
        for i in 0..ACTION_CAPACITY {
            actions.add(i).write(MaybeUninit::uninit());
        }
        ptr::addr_of_mut!((*state).head).write(0);
        ptr::addr_of_mut!((*state).len).write(0);
        let core = &mut (*state).core;
        core.dltime = TdmaTime::from_int(config.now as i32);
        core.last_recovered_dltime = (config.flags & C_RECOVERED != 0).then(|| TdmaTime::from_int(config.recovered_now as i32));
        core.valid_individual_ssi = (config.flags & C_INDIVIDUAL != 0).then_some(config.own_issi);
        core.valid_group_ssis
            .replace_bounded(&config.groups[..config.group_count as usize])
            .expect("validated group capacity");
        if config.flags & C_NETWORK != 0 {
            core.mcc = Some(config.mcc as u16);
            core.mnc = Some(config.mnc as u16);
        }
        core.cc = (config.flags & C_COLOUR != 0).then_some(config.colour as u8);
        core.update_scrambling();
        if config.flags & C_SCRAMBLING != 0 {
            core.scrambling_code = Some(config.scrambling_code);
        }
        core.serving_carrier_num = (config.flags & C_CARRIER != 0).then_some(config.serving_carrier as u16);
        ptr::addr_of_mut!((*state).magic).write(MAGIC);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn tetra_ms_mac_abi_version() -> u32 {
    ABI_VERSION
}
#[unsafe(no_mangle)]
pub extern "C" fn tetra_ms_mac_storage_size() -> u32 {
    STORAGE_SIZE
}
#[unsafe(no_mangle)]
pub extern "C" fn tetra_ms_mac_storage_align() -> u32 {
    STORAGE_ALIGN
}

/// # Safety
/// Caller owns sufficiently large/aligned writable storage and a fully initialized
/// readable Config. Regions must be disjoint; serialize all calls per instance.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_ms_mac_init(storage: *mut c_void, storage_size: u32, input: *const Config) -> i32 {
    boundary(|| {
        valid_ptr(storage.cast::<State>())?;
        if storage_size < STORAGE_SIZE {
            return Err(CAPACITY);
        }
        disjoint_region(storage, STORAGE_SIZE, input)?;
        unsafe {
            check_header(input)?;
        }
        let config = unsafe { &*input };
        validate_config(config)?;
        unsafe { initialize(storage.cast(), config) };
        Ok(())
    })
}

/// # Safety
/// Storage is initialized/exclusive; Config is fully initialized, readable,
/// aligned and disjoint. Reset cancels all old external receipts and actions.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_ms_mac_reset(storage: *mut c_void, input: *const Config) -> i32 {
    boundary(|| {
        disjoint_region(storage, STORAGE_SIZE, input)?;
        unsafe {
            check_header(input)?;
        }
        let config = unsafe { &*input };
        validate_config(config)?;
        let state = unsafe { state_ptr(storage)? };
        unsafe { initialize(state, config) };
        Ok(())
    })
}

/// # Safety
/// Storage is initialized/exclusive; output is writable/aligned/disjoint. Only
/// size and abi_version need initialization. Errors/EMPTY consume nothing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_ms_mac_poll_action(storage: *mut c_void, output: *mut Action) -> i32 {
    boundary(|| {
        disjoint_region(storage, STORAGE_SIZE, output)?;
        unsafe {
            check_header(output)?;
        }
        let state = unsafe { &mut *state_ptr(storage)? };
        if state.len == 0 {
            return Err(EMPTY);
        }
        let action = state.actions[usize::from(state.head)].as_ptr();
        // SAFETY: A committed queue slot is initialized; regions are disjoint.
        unsafe { ptr::copy_nonoverlapping(action, output, 1) };
        state.head = ((usize::from(state.head) + 1) % ACTION_CAPACITY) as u8;
        state.len -= 1;
        Ok(())
    })
}

fn slot(input: SlotAccess) -> Result<ra::SubslotAccess, i32> {
    if input.code_present > 1 || input.frame_length > u16::MAX as u32 {
        return Err(ARGUMENT);
    }
    let access_code = if input.code_present != 0 {
        Some(access_code(input.code)?)
    } else {
        None
    };
    let frame_len = match input.frame_kind {
        0 => ra::BaseFrameLength::ReservedSubslot,
        1 => ra::BaseFrameLength::ClchSubslot,
        2 => ra::BaseFrameLength::OngoingFrame,
        3 => ra::BaseFrameLength::FrameMarker(input.frame_length as u16),
        _ => return Err(ARGUMENT),
    };
    Ok(ra::SubslotAccess { access_code, frame_len })
}

fn c_slot(input: ra::SubslotAccess) -> SlotAccess {
    let (frame_kind, frame_length) = match input.frame_len {
        ra::BaseFrameLength::ReservedSubslot => (0, 0),
        ra::BaseFrameLength::ClchSubslot => (1, 0),
        ra::BaseFrameLength::OngoingFrame => (2, 0),
        ra::BaseFrameLength::FrameMarker(length) => (3, u32::from(length)),
    };
    SlotAccess {
        code_present: u32::from(input.access_code.is_some()),
        code: input.access_code.map_or(0, |code| code as u32),
        frame_kind,
        frame_length,
    }
}

fn receipt(out: &mut Action, first: Option<u32>, second: Option<u32>) {
    if let Some(value) = first {
        out.flags |= F_RECEIPT;
        out.receipt = value;
    }
    if let Some(value) = second {
        out.flags |= F_RECEIPT2;
        out.receipt2 = value;
    }
}

fn payload(out: &mut Action, input: BitView<'_>, window: downlink::Payload) {
    out.header_bits = u32::from(window.start);
    out.payload_end = u32::from(window.end);
    out.next_bits = u32::from(window.next);
    out.fill_bits = u32::from(window.fill);
    if window.truncated {
        out.flags |= F_TRUNCATED;
    }
    out.pdu_bits = copy_bits(
        &mut out.pdu,
        input.window(window.start, window.end - window.start).expect("decoded payload"),
    );
    out.pdu2_bits = copy_bits(&mut out.pdu2, input);
}

fn grant_error(error: mac::GrantError) -> i32 {
    match error {
        mac::GrantError::Missing => -220,
        mac::GrantError::Delay => -221,
        mac::GrantError::Capacity => -222,
        mac::GrantError::MissedSlot => -223,
        mac::GrantError::Bits(error) => bit_error(error),
    }
}

impl State {
    fn emit_burst(&mut self, burst: mac::Burst<u32>) {
        let out = self.action(TRANSMIT);
        out.flags = F_RESERVED | F_MAC_BLOCK;
        out.now = burst.time.to_int() as u32;
        out.channel = channel(burst.channel);
        out.scrambling_code = burst.scrambling_code;
        out.pdu_bits = copy_bits(&mut out.pdu, burst.block.view());
        receipt(out, burst.receipt, None);
    }

    fn emit_traffic(&mut self, burst: mac::TrafficBurst<u32>) {
        let out = self.action(TRANSMIT);
        out.flags = F_TRAFFIC | F_MAC_BLOCK;
        out.now = burst.time.to_int() as u32;
        match burst.stolen {
            Some(stolen) => {
                out.flags |= F_SECOND;
                out.channel = channel(Channel::Stch);
                out.channel2 = channel(Channel::TchS);
                out.scrambling_code = stolen.scrambling_code;
                out.scrambling_code2 = burst.scrambling_code;
                out.pdu_bits = copy_bits(&mut out.pdu, stolen.mac_block.view());
                out.pdu2_bits = copy_bits(&mut out.pdu2, burst.speech.view());
                receipt(out, stolen.tx_reporter, None);
            }
            None => {
                out.channel = channel(Channel::TchS);
                out.scrambling_code = burst.scrambling_code;
                out.pdu_bits = copy_bits(&mut out.pdu, burst.speech.view());
            }
        }
    }

    fn emit_reserved(&mut self, action: ReservedAction<u32>) {
        match action {
            ReservedAction::Waiting => {}
            ReservedAction::Transmit(burst) => self.emit_burst(burst),
            ReservedAction::Missed { receipt: token } => {
                let out = self.action(RESERVED_MISSED);
                out.reason = -223;
                receipt(out, token, None);
            }
        }
    }

    fn emit_fragment(&mut self, action: FragmentAction<u32>) {
        match action {
            FragmentAction::Rejected { reason, receipt: token } => {
                let out = self.action(FRAGMENT_REJECTED);
                out.reason = grant_error(reason);
                receipt(out, token, None);
            }
            FragmentAction::Transmit(burst) => self.emit_burst(burst),
            FragmentAction::Reserved => {
                self.action(PLAN_INSTALLED);
                let action = self.core.reserved();
                self.emit_reserved(action);
            }
        }
    }

    fn emit_ra(&mut self, action: Option<RaAction>) {
        match action {
            None => {}
            Some(RaAction::Transmit { ul_time, subslot }) => {
                // Only one bounded MAC block is copied; the pending transfer is
                // deliberately retained for native ALOHA retransmission.
                let pending = self
                    .core
                    .pending_uplink
                    .as_ref()
                    .map(|pending| (pending.mac_block.clone(), pending.logical_channel, pending.scrambling_code));
                let out = self.action(RA_TRANSMIT);
                out.now = ul_time.to_int() as u32;
                out.subslot = match subslot {
                    ra::Subslot::One => 1,
                    ra::Subslot::Two => 2,
                };
                if let Some((bits, logical, scrambling)) = pending {
                    out.flags = F_MAC_BLOCK;
                    out.channel = channel(logical);
                    out.scrambling_code = scrambling;
                    out.pdu_bits = copy_bits(&mut out.pdu, bits.view());
                }
            }
            Some(RaAction::Succeeded) => {
                self.action(RA_SUCCEEDED);
            }
            Some(RaAction::Failed(reason)) => {
                let (first, tail) = self.core.abandon_access();
                let out = self.action(RA_FAILED);
                out.reason = ra_error(reason);
                receipt(out, first, tail);
            }
        }
    }

    fn emit_resource(&mut self, bits: BitView<'_>, event: mac::ResourceEvent<u32>) {
        match event.access {
            AccessCompletion::None => {}
            AccessCompletion::Complete { receipt: token } => receipt(self.action(RA_SUCCEEDED), token, None),
            AccessCompletion::Fragment(action) => {
                self.action(RA_SUCCEEDED);
                self.emit_fragment(action);
            }
        }
        let outcome = event.outcome;
        let header = outcome.header;
        let out = self.action(RESOURCE);
        payload(out, bits, outcome.payload);
        out.flags |= if header.fill_bits { F_FILL } else { 0 }
            | if header.random_access_flag { F_RANDOM_ACCESS } else { 0 }
            | if outcome.second_half_stolen { F_SECOND_STOLEN } else { 0 }
            | if outcome.acknowledges_own { F_ACKS_OWN } else { 0 };
        out.route = match outcome.route {
            mac::ResourceRoute::Drop(mac::DropReason::NoAddress) => 0,
            mac::ResourceRoute::Drop(mac::DropReason::Encrypted) => 1,
            mac::ResourceRoute::Drop(mac::DropReason::ForeignAddress) => 2,
            mac::ResourceRoute::Empty => 3,
            mac::ResourceRoute::Deliver => 4,
            mac::ResourceRoute::FragmentStart => 5,
        };
        out.encryption = u32::from(header.encryption_mode);
        out.length_ind = u32::from(header.length_ind);
        if let Some(address) = header.addr {
            out.flags |= F_ADDRESS;
            out.ssi = address.ssi;
            out.ssi_type = address_kind(address.ssi_type);
        }
        if let Some(label) = header.event_label {
            out.flags |= F_EVENT_LABEL;
            out.event_label = u32::from(label);
        }
        if let Some(marker) = header.usage_marker {
            out.flags |= F_MARKER;
            out.marker = u32::from(marker);
        }
        if let Some(grant) = header.grant {
            out.flags |= F_GRANT;
            out.grant = c_grant(grant);
        }
        if let Some(allocation) = header.allocation {
            out.flags |= F_ALLOCATION;
            out.allocation = c_allocation(allocation);
        }
        if let Some(result) = outcome.allocation {
            out.flags |= F_ALLOCATION_RESULT;
            out.allocation_result = allocation_result(result);
        }
    }

    #[inline(never)]
    fn submit(&mut self, event: &Event, bits: BitView<'_>) -> Result<(), i32> {
        self.reserve(2)?;
        let result = self
            .core
            .submit_sdu(
                mac::SubmitRequest {
                    issi: self.own_issi,
                    sdu: bits,
                    stealing: event.flags & E_STEALING != 0,
                    priority: (event.flags & E_PRIORITY != 0).then_some(event.priority as u8),
                    emergency: event.flags & E_EMERGENCY != 0,
                    receipt: (event.flags & E_RECEIPT != 0).then_some(event.receipt),
                },
                self.ra.is_active(),
            )
            .map_err(|error| bit_error(error.reason))?;
        self.action(SUBMITTED).route = result.route as u32;
        if let Some(discarded) = result.discarded {
            let out = self.action(DISCARDED_TRANSFER);
            out.reason = QUEUE_FULL;
            out.flags = F_MAC_BLOCK
                | if discarded.pending.pdu_priority.is_some() { F_PRIORITY } else { 0 }
                | if discarded.pending.is_emergency { F_EMERGENCY } else { 0 };
            out.code = u32::from(discarded.pending.pdu_priority.unwrap_or(0));
            out.channel = channel(discarded.pending.logical_channel);
            out.scrambling_code = discarded.pending.scrambling_code;
            out.pdu_bits = copy_bits(&mut out.pdu, discarded.pending.mac_block.view());
            receipt(out, discarded.pending.tx_reporter, None);
            if let Some(fragment) = discarded.fragment {
                out.flags |= F_SECOND;
                out.scrambling_code2 = fragment.scrambling_code;
                (out.fragment_kind, out.fragment_slots) = fragment_kind(fragment.end_kind);
                out.pdu2_bits = copy_bits(&mut out.pdu2, fragment.remainder.view());
                receipt(out, None, fragment.tx_reporter);
            }
        }
        if let Some(discarded) = result.discarded_stolen {
            let out = self.action(DISCARDED_STOLEN);
            out.reason = QUEUE_FULL;
            out.channel = channel(Channel::Stch);
            out.scrambling_code = discarded.scrambling_code;
            out.pdu_bits = copy_bits(&mut out.pdu, discarded.mac_block.view());
            receipt(out, discarded.tx_reporter, None);
        }
        Ok(())
    }
}

struct PollPlan {
    assignment: Option<ra::SlotAccessAssign>,
    params: Option<ra::RandomAccessParams>,
    now: TdmaTime,
    needed: Option<ra::EntropyRequest>,
    admission_failure: Option<RaFailure>,
    marker: Option<u8>,
}

fn poll_plan(state: &State, event: &Event, bits: BitView<'_>) -> Result<PollPlan, i32> {
    let automatic = event.kind == AACH;
    let (assignment, marker, now) = if automatic {
        let aach = downlink::aach(bits, state.core.dltime.f == 18).map_err(decode_error)?;
        (aach.access, aach.traffic_marker, state.core.dltime)
    } else {
        (
            Some(ra::SlotAccessAssign {
                subslot1: slot(event.subslots[0])?,
                subslot2: slot(event.subslots[1])?,
            }),
            None,
            time(event.now)?,
        )
    };
    let mut probe = state.ra.clone();
    let code = if automatic {
        probe.active_code().unwrap_or(AccessCode::A)
    } else {
        probe.active_code().unwrap_or(access_code(event.code)?)
    };
    let params = state.params.params_for(code).cloned();
    let mut admission_failure = None;
    if assignment.is_some()
        && automatic
        && !probe.is_active()
        && let (Some(params), Some(candidate)) = (params.as_ref(), state.core.access_candidate(false))
    {
        admission_failure = probe
            .initiate(
                now,
                AccessCode::A,
                params,
                candidate.pdu_priority.unwrap_or(params.min_pdu_prio),
                candidate.is_emergency,
            )
            .err();
    }
    if !automatic && params.is_none() {
        return Err(ra_error(RaFailure::NoValidAccessCode));
    }
    let needed = if admission_failure.is_none() {
        match (assignment.as_ref(), params.as_ref()) {
            (Some(assign), Some(params)) => probe
                .entropy_needed(now, assign, automatic || event.flags & E_UL_VALID != 0, params)
                .map_err(ra_error)?,
            _ => None,
        }
    } else {
        None
    };
    Ok(PollPlan {
        assignment,
        params,
        now,
        needed,
        admission_failure,
        marker,
    })
}

fn choices(event: &Event, needed: Option<ra::EntropyRequest>) -> Result<ra::RaChoices, i32> {
    let index = match needed {
        Some(ra::EntropyRequest::SubslotIndex(maximum)) => {
            if event.choice_index == 0 || event.choice_index > u32::from(maximum) {
                return Err(ra_error(RaFailure::InvalidEntropy));
            }
            event.choice_index as u16
        }
        _ => 1,
    };
    let subslot = match (needed, event.choice_subslot) {
        (Some(ra::EntropyRequest::OneOfTwo), 1) => ra::Subslot::One,
        (Some(ra::EntropyRequest::OneOfTwo), 2) => ra::Subslot::Two,
        (Some(ra::EntropyRequest::OneOfTwo), _) => return Err(ra_error(RaFailure::InvalidEntropy)),
        (_, 2) => ra::Subslot::Two,
        _ => ra::Subslot::One,
    };
    Ok(ra::RaChoices::new(index, subslot))
}

fn validate_event(event: &Event) -> Result<(), i32> {
    if event.pdu_bits > 4096 || event.group_count > 256 {
        return Err(CAPACITY);
    }
    let allowed = match event.kind {
        SUBMIT => E_RECEIPT | E_PRIORITY | E_EMERGENCY | E_STEALING,
        IDENTITIES => E_INDIVIDUAL | E_GROUPS,
        UPLANE => E_SWITCH | E_TX_GRANT,
        SET_TIME => E_RECOVERED | E_CLEAR_RECOVERED,
        RA_INITIATE => E_EMERGENCY,
        RA_POLL => E_UL_VALID,
        RA_RESPONSE => E_ADDRESS_MATCH | E_RA_SUCCESS,
        TICK | AACH | DOWNLINK | SYNC | AUDIO | ALLOCATE | BIND_MARKER | TRAFFIC_OWNER | CANCEL_ACCESS | SET_PARAMS | RA_CANCEL
        | GRANT_FRAGMENT | POLL_RESERVED | POLL_TRAFFIC | PROMOTE | ADVANCE | CHECK_ADDRESS | ENCODE | SCRAMBLING | CONTINUE_BLOCK => 0,
        _ => return Err(UNSUPPORTED),
    };
    if event.flags & !allowed != 0 {
        return Err(ARGUMENT);
    }
    match event.kind {
        SUBMIT if event.flags & E_PRIORITY != 0 && event.priority > 7 => return Err(ARGUMENT),
        IDENTITIES => {
            if event.mcc > 1023 || event.mnc > 16383 {
                return Err(ARGUMENT);
            }
            if event.ssi > 0xff_ffff || event.groups[..event.group_count as usize].iter().any(|&v| v > 0xff_ffff) {
                return Err(INVALID_IDENTITY);
            }
        }
        TICK | ADVANCE | RA_INITIATE | RA_POLL => {
            time(event.now)?;
        }
        SET_TIME => {
            time(event.now)?;
            if event.flags & E_RECOVERED != 0 {
                time(event.value)?;
            }
            if event.flags & (E_RECOVERED | E_CLEAR_RECOVERED) == E_RECOVERED | E_CLEAR_RECOVERED {
                return Err(ARGUMENT);
            }
        }
        ALLOCATE => {
            allocation(event.allocation)?;
        }
        BIND_MARKER if event.value > 255 || event.ssi > 0xff_ffff => return Err(ARGUMENT),
        DOWNLINK if event.source > 1 => return Err(ARGUMENT),
        SET_PARAMS => {
            params(event.params)?;
            access_code(event.code)?;
            if event.source > 2 || (event.source == 0 && event.code != 0) {
                return Err(ARGUMENT);
            }
        }
        GRANT_FRAGMENT => {
            if event.source > 1 {
                return Err(ARGUMENT);
            }
            if event.source == 1 {
                grant(event.grant)?;
            }
        }
        SCRAMBLING if event.source > 1 => return Err(ARGUMENT),
        CONTINUE_BLOCK if event.value > u16::MAX as u32 => return Err(ARGUMENT),
        CHECK_ADDRESS => {
            if event.ssi > 0xff_ffff || event.value > 2 {
                return Err(ARGUMENT);
            }
            address(event.ssi, event.ssi_type)?;
        }
        ENCODE if event.ssi > 0xff_ffff => return Err(INVALID_IDENTITY),
        _ => {}
    }
    if event.kind == RA_INITIATE && event.priority > 7 {
        return Err(ARGUMENT);
    }
    if matches!(event.kind, RA_INITIATE | RA_POLL) {
        access_code(event.code)?;
    }
    Ok(())
}

impl State {
    #[inline(never)]
    fn poll_access(&mut self, event: &Event, bits: BitView<'_>) -> Result<(), i32> {
        let plan = poll_plan(self, event, bits)?;
        let mut choices = choices(event, plan.needed)?;
        self.reserve(if event.kind == AACH { 2 } else { 1 })?;
        if event.kind == AACH {
            self.core.aach(bits).expect("preflight decoded AACH");
            let out = self.action(AACH_INFO);
            out.now = plan.now.to_int() as u32;
            if let Some(marker) = plan.marker {
                out.flags |= F_MARKER;
                out.marker = u32::from(marker);
            }
            if let Some(assign) = plan.assignment {
                out.flags |= F_ACCESS;
                out.subslots = [c_slot(assign.subslot1), c_slot(assign.subslot2)];
            } else {
                return Ok(());
            }
            match self.core.begin_access(&mut self.ra, plan.params.as_ref()) {
                Ok(false) => return Ok(()),
                Ok(true) => {}
                Err(error) => {
                    let out = self.action(RA_FAILED);
                    out.reason = ra_error(error.reason);
                    receipt(out, error.first_receipt, error.fragment_receipt);
                    return Ok(());
                }
            }
        }
        let action = self
            .ra
            .poll_downlink_slot(
                plan.now,
                plan.assignment.as_ref().expect("preflight access"),
                event.kind == AACH || event.flags & E_UL_VALID != 0,
                plan.params.as_ref().expect("preflight access parameters"),
                &mut choices,
            )
            .expect("preflight entropy");
        self.emit_ra(action);
        Ok(())
    }

    #[inline(never)]
    fn downlink(&mut self, event: &Event, bits: BitView<'_>) -> Result<(), i32> {
        match downlink::kind(bits, event.source != 0).map_err(decode_error)? {
            downlink::PduKind::Resource => {
                downlink::resource(bits).map_err(decode_error)?.payload(bits).map_err(bit_error)?;
                self.reserve(3)?;
                let result = self
                    .core
                    .resource_event(&mut self.ra, bits, self.own_issi, self.filtered != 0)
                    .expect("preflight MAC-RESOURCE");
                self.emit_resource(bits, result);
            }
            downlink::PduKind::Fragment | downlink::PduKind::End => {
                let ending = matches!(downlink::kind(bits, event.source != 0), Ok(downlink::PduKind::End));
                let decoded = if ending { downlink::end(bits) } else { downlink::fragment(bits) }.map_err(decode_error)?;
                self.reserve(1)?;
                payload(self.action(if ending { END } else { FRAGMENT }), bits, decoded);
            }
            downlink::PduKind::Sysinfo => {
                let info = downlink::sysinfo(bits).map_err(decode_error)?;
                self.reserve(2)?;
                if let Some(params) = &info.default_access {
                    self.params.update_sysinfo_default_a(params.clone());
                    self.sysinfo_present = 1;
                }
                if let Some(correction) = self.core.adopt_sysinfo(&info) {
                    let out = self.action(CONFIGURE);
                    out.flags = F_HYPERFRAME;
                    out.now = correction.to_int() as u32;
                    out.hyperframe = u32::from(correction.h);
                }
                let out = self.action(SYSINFO);
                out.carrier = u32::from(info.main_carrier);
                out.band = u32::from(info.freq_band);
                out.offset = u32::from(info.freq_offset_index);
                out.duplex = u32::from(info.duplex_spacing);
                out.reverse = u32::from(info.reverse_operation);
                out.option = u32::from(info.option);
                out.option_bits = info.option_bits;
                out.header_bits = u32::from(info.sdu_offset);
                out.pdu_bits = copy_bits(
                    &mut out.pdu,
                    bits.window(info.sdu_offset, bits.len() - info.sdu_offset).expect("SYSINFO SDU"),
                );
                out.pdu2_bits = copy_bits(&mut out.pdu2, bits);
                if let Some(hyperframe) = info.hyperframe {
                    out.flags |= F_HYPERFRAME;
                    out.hyperframe = u32::from(hyperframe);
                }
                if let Some(params) = info.default_access {
                    out.flags |= F_DEFAULT_ACCESS;
                    out.params = c_params(&params);
                }
            }
            downlink::PduKind::AccessDefine => {
                let definition = downlink::access_define(bits).map_err(decode_error)?;
                self.reserve(1)?;
                let applied = !definition.assigned_control;
                if applied {
                    self.params.update_access_define(definition.code, definition.params.clone());
                    self.defined_mask |= 1 << definition.code as u32;
                }
                let out = self.action(ACCESS_PARAMS);
                out.code = definition.code as u32;
                out.params = c_params(&definition.params);
                out.flags = if applied { F_APPLIED } else { F_ASSIGNED_CONTROL };
                out.pdu_bits = copy_bits(&mut out.pdu, bits);
            }
        }
        Ok(())
    }

    #[inline(never)]
    fn encode(&mut self, event: &Event, bits: BitView<'_>) -> Result<(), i32> {
        if event.value == 6 {
            let (block, tail, kind) = wire::access_fragment(event.ssi, bits).map_err(bit_error)?;
            self.reserve(1)?;
            let out = self.action(ENCODED);
            out.route = event.value;
            out.channel = channel(Channel::SchHu);
            (out.fragment_kind, out.fragment_slots) = fragment_kind(kind);
            out.pdu_bits = copy_bits(&mut out.pdu, block.view());
            out.pdu2_bits = copy_bits(&mut out.pdu2, tail.view());
            return Ok(());
        }
        if event.value == 7 {
            let slots: u8 = event.source.try_into().map_err(|_| CAPACITY)?;
            let blocks = wire::FragmentBlocks::build(bits, slots).map_err(bit_error)?;
            self.reserve(slots)?;
            for (index, block) in blocks.as_slice().iter().enumerate() {
                let out = self.action(ENCODED);
                out.route = event.value;
                out.channel = channel(Channel::SchF);
                out.code = index as u32;
                out.fragment_slots = u32::from(slots);
                out.pdu_bits = copy_bits(&mut out.pdu, block.view());
            }
            return Ok(());
        }
        let (block, logical) = match event.value {
            1 => (wire::access(event.ssi, bits), Channel::SchHu),
            2 => (wire::stolen_data(event.ssi, bits), Channel::Stch),
            3 => (wire::fragment(bits), Channel::SchF),
            4 => (wire::end_hu(bits), Channel::SchHu),
            5 => (wire::end_ul(bits), Channel::SchF),
            _ => return Err(UNSUPPORTED),
        };
        let block = block.map_err(bit_error)?;
        self.reserve(1)?;
        let out = self.action(ENCODED);
        out.route = event.value;
        out.channel = channel(logical);
        out.pdu_bits = copy_bits(&mut out.pdu, block.view());
        Ok(())
    }

    // Keep unrelated event temporaries out of the common C entry's stack frame.
    #[inline(never)]
    fn handle(&mut self, event: &Event, bits: BitView<'_>) -> Result<(), i32> {
        match event.kind {
            SUBMIT => self.submit(event, bits)?,
            TICK => {
                self.reserve(2)?;
                let actions = self.core.tick(time(event.now)?).map_err(bit_error)?;
                self.emit_reserved(actions.reserved);
                if let Some(traffic) = actions.traffic {
                    self.emit_traffic(traffic);
                }
            }
            AACH | RA_POLL => self.poll_access(event, bits)?,
            DOWNLINK => self.downlink(event, bits)?,
            SYNC => {
                let sync = downlink::sync(bits).map_err(decode_error)?;
                self.reserve(1)?;
                self.core.adopt_sync(sync);
                let out = self.action(SYNCED);
                out.now = sync.time.to_int() as u32;
                out.colour = u32::from(sync.colour_code);
                out.header_bits = u32::from(sync.sdu_offset);
                out.pdu_bits = copy_bits(
                    &mut out.pdu,
                    bits.window(sync.sdu_offset, bits.len() - sync.sdu_offset).expect("SYNC SDU"),
                );
                out.pdu2_bits = copy_bits(&mut out.pdu2, bits);
            }
            IDENTITIES => {
                self.reserve(1)?;
                let code = self
                    .core
                    .configure_identities(
                        event.mcc as u16,
                        event.mnc as u16,
                        (event.flags & E_INDIVIDUAL != 0).then_some(event.ssi),
                        (event.flags & E_GROUPS != 0).then_some(&event.groups[..event.group_count as usize]),
                    )
                    .map_err(bit_error)?;
                let colour = self.core.cc;
                let out = self.action(CONFIGURE);
                out.mcc = event.mcc;
                out.mnc = event.mnc;
                out.colour = u32::from(colour.unwrap_or(0));
                if let Some(code) = code {
                    out.flags |= F_SCRAMBLING;
                    out.scrambling_code = code;
                }
            }
            UPLANE => self
                .core
                .configure_uplane(event.flags & E_SWITCH != 0, event.flags & E_TX_GRANT != 0),
            AUDIO => {
                self.reserve(1)?;
                if self.core.queue_audio(bits).map_err(bit_error)? {
                    self.action(AUDIO_DROPPED);
                }
            }
            ALLOCATE => {
                let allocation = allocation(event.allocation)?;
                self.reserve(1)?;
                let result = self.core.allocate(self.own_issi, allocation);
                let out = self.action(ALLOCATION);
                out.allocation = c_allocation(allocation);
                out.allocation_result = allocation_result(result);
            }
            BIND_MARKER => self.core.active_traffic_usage_markers.insert(event.value as u8, event.ssi),
            TRAFFIC_OWNER => {
                self.reserve(1)?;
                if let Some((marker, owner)) = self.core.downlink_traffic_owner() {
                    let out = self.action(OWNER);
                    out.marker = u32::from(marker);
                    out.ssi = owner;
                }
            }
            CANCEL_ACCESS => {
                self.reserve(1)?;
                self.ra.cancel();
                let (first, tail) = self.core.abandon_access();
                receipt(self.action(CANCELLED), first, tail);
            }
            SET_TIME => {
                self.core.dltime = time(event.now)?;
                if event.flags & E_RECOVERED != 0 {
                    self.core.last_recovered_dltime = Some(time(event.value)?);
                }
                if event.flags & E_CLEAR_RECOVERED != 0 {
                    self.core.last_recovered_dltime = None;
                }
            }
            SET_PARAMS => {
                let params = params(event.params)?;
                let c_params = c_params(&params);
                let code = access_code(event.code)?;
                self.reserve(1)?;
                match event.source {
                    0 => {
                        self.params.update_sysinfo_default_a(params);
                        self.sysinfo_present = 1;
                    }
                    1 => {
                        self.params.update_access_define(code, params);
                        self.defined_mask |= 1 << event.code;
                    }
                    _ => {}
                }
                let out = self.action(ACCESS_PARAMS);
                out.code = event.code;
                out.params = c_params;
                out.flags = if event.source == 2 { F_ASSIGNED_CONTROL } else { F_APPLIED };
            }
            RA_INITIATE => {
                let code = access_code(event.code)?;
                let params = self.params.params_for(code).ok_or_else(|| ra_error(RaFailure::NoValidAccessCode))?;
                self.ra
                    .initiate(time(event.now)?, code, params, event.priority as u8, event.flags & E_EMERGENCY != 0)
                    .map_err(ra_error)?;
            }
            RA_RESPONSE => {
                self.reserve(1)?;
                let action = self
                    .ra
                    .on_mac_resource(event.flags & E_ADDRESS_MATCH != 0, event.flags & E_RA_SUCCESS != 0);
                self.emit_ra(action);
            }
            RA_CANCEL => self.ra.cancel(),
            GRANT_FRAGMENT => {
                let grant = if event.source == 0 { None } else { Some(grant(event.grant)?) };
                self.reserve(2)?;
                if let Some(action) = self.core.grant_fragment(grant) {
                    self.emit_fragment(action);
                }
            }
            POLL_RESERVED => {
                self.reserve(1)?;
                let action = self.core.reserved();
                self.emit_reserved(action);
            }
            POLL_TRAFFIC => {
                self.reserve(1)?;
                if let Some(action) = self.core.traffic() {
                    self.emit_traffic(action);
                }
            }
            PROMOTE => self.core.promote(self.ra.is_active()),
            ADVANCE => {
                self.core.advance(time(event.now)?).map_err(bit_error)?;
            }
            CHECK_ADDRESS => {
                let address = address(event.ssi, event.ssi_type)?;
                let filtered = if event.value == 2 { self.filtered != 0 } else { event.value != 0 };
                self.reserve(1)?;
                let accepted = self.core.accepts(filtered, address);
                self.action(ADDRESS_RESULT).route = u32::from(accepted);
            }
            ENCODE => self.encode(event, bits)?,
            SCRAMBLING => {
                self.reserve(1)?;
                self.core.scrambling_code = (event.source != 0).then_some(event.scrambling_code);
                let out = self.action(CONFIGURE);
                if event.source != 0 {
                    out.flags = F_SCRAMBLING;
                    out.scrambling_code = event.scrambling_code;
                }
            }
            CONTINUE_BLOCK => {
                let mut cursor = downlink::BlockCursor::new(bits.len());
                if cursor.advance(event.value as u16).map_err(bit_error)? {
                    self.reserve(1)?;
                    let remaining = bits
                        .window(event.value as u16, bits.len() - event.value as u16)
                        .expect("checked MAC cursor");
                    let out = self.action(NEXT_PDU);
                    out.header_bits = event.value;
                    out.payload_end = u32::from(bits.len());
                    out.pdu_bits = copy_bits(&mut out.pdu, remaining);
                }
            }
            _ => return Err(UNSUPPORTED),
        }
        Ok(())
    }
}

/// # Safety
/// Storage is initialized/exclusive; input is fully initialized, readable,
/// aligned and disjoint. Errors never consume an event or queued output.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_ms_mac_push_event(storage: *mut c_void, input: *const Event) -> i32 {
    boundary(|| {
        disjoint_region(storage, STORAGE_SIZE, input)?;
        unsafe {
            check_header(input)?;
        }
        let input = unsafe { &*input };
        validate_event(input)?;
        let bits = BitView::new(&input.pdu, input.pdu_bits as u16).map_err(bit_error)?;
        let state = unsafe { &mut *state_ptr(storage)? };
        state.handle(input, bits)
    })
}

/// # Safety
/// Storage is initialized/readable. Input is fully initialized/readable; output
/// is writable with initialized size/version only. All three regions are disjoint.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_ms_mac_entropy_needed(storage: *const c_void, input: *const Event, output: *mut Entropy) -> i32 {
    boundary(|| {
        disjoint_region(storage, STORAGE_SIZE, input)?;
        disjoint_region(storage, STORAGE_SIZE, output)?;
        disjoint_region(input.cast(), size_of::<Event>() as u32, output)?;
        unsafe {
            check_header(input)?;
            check_header(output)?;
        }
        let input = unsafe { &*input };
        validate_event(input)?;
        if !matches!(input.kind, AACH | RA_POLL) {
            return Err(ARGUMENT);
        }
        let state = unsafe { &*state_ptr(storage)? };
        let bits = BitView::new(&input.pdu, input.pdu_bits as u16).map_err(bit_error)?;
        let plan = poll_plan(state, input, bits)?;
        let (kind, maximum) = match plan.needed {
            None => (0, 0),
            Some(ra::EntropyRequest::SubslotIndex(maximum)) => (1, u32::from(maximum)),
            Some(ra::EntropyRequest::OneOfTwo) => (2, 2),
        };
        // SAFETY: All checks completed. Output may have uninitialized payload.
        unsafe {
            output.write(Entropy {
                size: size_of::<Entropy>() as u32,
                abi_version: ABI_VERSION,
                kind,
                maximum,
                admission_failure: plan.admission_failure.map_or(0, ra_error),
            })
        };
        Ok(())
    })
}

/// # Safety
/// Storage is initialized/readable without concurrent mutation. Output is
/// writable, aligned/disjoint, with initialized size/version headers only.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_ms_mac_get_status(storage: *const c_void, output: *mut Status) -> i32 {
    boundary(|| {
        disjoint_region(storage, STORAGE_SIZE, output)?;
        unsafe {
            check_header(output)?;
        }
        let state = unsafe { &*state_ptr(storage)? };
        let core = &state.core;
        let snapshot = core.snapshot();
        let ra = state.ra.status();
        // SAFETY: All validation precedes writing; every Status field is an
        // integer/array, so zeroing initializes even a header-only C output.
        unsafe { ptr::write_bytes(output, 0, 1) };
        let out = unsafe { &mut *output };
        out.size = size_of::<Status>() as u32;
        out.abi_version = ABI_VERSION;
        out.now = core.dltime.to_int() as u32;
        out.own_issi = state.own_issi;
        out.flags = if state.filtered != 0 { S_FILTERED } else { 0 };
        if let Some(time) = core.last_recovered_dltime {
            out.flags |= S_RECOVERED;
            out.recovered_now = time.to_int() as u32;
        }
        if let Some(ssi) = core.valid_individual_ssi {
            out.flags |= S_INDIVIDUAL;
            out.individual_ssi = ssi;
        }
        if let (Some(mcc), Some(mnc)) = (core.mcc, core.mnc) {
            out.flags |= S_NETWORK;
            out.mcc = u32::from(mcc);
            out.mnc = u32::from(mnc);
        }
        if let Some(colour) = core.cc {
            out.flags |= S_COLOUR;
            out.colour = u32::from(colour);
        }
        if let Some(code) = core.scrambling_code {
            out.flags |= S_SCRAMBLING;
            out.scrambling_code = code;
        }
        if let Some(carrier) = core.serving_carrier_num {
            out.flags |= S_CARRIER;
            out.serving_carrier = u32::from(carrier);
        }
        if let Some(owner) = core.traffic_channel_owner {
            out.flags |= S_TRAFFIC_OWNER;
            out.traffic_owner = owner;
        }
        if let Some(marker) = core.current_traffic_usage_marker {
            out.flags |= S_MARKER;
            out.current_marker = u32::from(marker);
        }
        out.transfer_status = snapshot.status as u32;
        out.queued_uplinks = u32::from(snapshot.queued_uplinks);
        out.queued_stolen = u32::from(snapshot.queued_stolen);
        out.queued_audio = u32::from(snapshot.queued_audio);
        out.assigned_slot_bitmap = u32::from(snapshot.assigned_slot_bitmap);
        out.tx_granted = u32::from(snapshot.tx_granted);
        out.actions = u32::from(state.len);
        out.ra_phase = match ra.phase {
            ra::AccessPhase::Idle => 0,
            ra::AccessPhase::FirstTry => 1,
            ra::AccessPhase::AwaitFrameMarker => 2,
            ra::AccessPhase::Counting => 3,
            ra::AccessPhase::AwaitResponse => 4,
        };
        if let Some(code) = ra.code {
            out.flags |= S_RA_ACTIVE;
            out.ra_code = code as u32;
        }
        out.ra_emergency = u32::from(ra.emergency);
        out.ra_transmissions = u32::from(ra.transmissions);
        out.defined_mask = state.defined_mask;
        out.sysinfo_present = state.sysinfo_present;
        for code in [AccessCode::A, AccessCode::B, AccessCode::C, AccessCode::D] {
            if let Some(params) = state.params.params_for(code) {
                out.param_mask |= 1 << code as u32;
                out.params[code.index()] = c_params(params);
            }
        }
        let groups = core.valid_group_ssis.as_slice();
        out.group_count = groups.len() as u32;
        out.groups[..groups.len()].copy_from_slice(groups);
        for marker in 0..256 {
            if let Some(&owner) = core.active_traffic_usage_markers.get(&(marker as u8)) {
                out.marker_mask[marker / 32] |= 1 << (marker % 32);
                out.marker_owners[marker] = owner;
            }
        }
        if let Some(pending) = &core.pending_uplink {
            out.flags |= S_PENDING;
            out.pending_channel = channel(pending.logical_channel);
            out.pending_scrambling_code = pending.scrambling_code;
            out.pending_bits = copy_bits(&mut out.pending_pdu, pending.mac_block.view());
            if let Some(token) = pending.tx_reporter {
                out.flags |= S_PENDING_RECEIPT;
                out.pending_receipt = token;
            }
            if let Some(priority) = pending.pdu_priority {
                out.flags |= S_PRIORITY;
                out.pending_priority = u32::from(priority);
            }
            out.pending_emergency = u32::from(pending.is_emergency);
        }
        if let Some(fragment) = &core.pending_fragment {
            out.flags |= S_FRAGMENT;
            out.fragment_scrambling_code = fragment.scrambling_code;
            out.fragment_bits = copy_bits(&mut out.fragment_pdu, fragment.remainder.view());
            (out.fragment_kind, out.fragment_slots) = fragment_kind(fragment.end_kind);
            if let Some(token) = fragment.tx_reporter {
                out.flags |= S_FRAGMENT_RECEIPT;
                out.fragment_receipt = token;
            }
        }
        if let Some(plan) = &core.reserved_tx {
            out.flags |= S_RESERVED;
            out.reserved_scrambling_code = plan.scrambling_code;
            if let Some(token) = plan.tx_reporter {
                out.flags |= S_RESERVED_RECEIPT;
                out.reserved_receipt = token;
            }
            let slots = plan.remaining_slots();
            out.reserved_count = slots.len() as u32;
            for (out, slot) in out.reserved_times.iter_mut().zip(slots) {
                *out = slot.to_int() as u32;
            }
        }
        Ok(())
    })
}

#[cfg(test)]
#[path = "mac_tests.rs"]
mod tests;
