//! Caller-owned, pointer-free CMCE ABI. See `include/tetra_cmce.h`.
use crate::{
    ABI_MISMATCH, ARGUMENT, CAPACITY, EMPTY, INVALID_IDENTITY, INVALID_TIME, QUEUE_FULL, TRUNCATED, UNINITIALIZED, UNSUPPORTED, boundary,
    disjoint_region, valid_ptr,
};
use core::{
    ffi::c_void,
    mem::{align_of, size_of},
    ptr,
};
use tetra_ms_core::{
    cmce::{
        BasicService, CallChange, CallRoute, CmceAction, CmceCore, CmceError, CmceEvent, MAX_CALLS, MAX_ORIGINATIONS, downlink::Downlink,
    },
    types::{BitError, BitView, MAX_PDU_BITS, MAX_PDU_BYTES, PackedBits, SsiType, TdmaTime, TetraAddress, tdma_time::TIME_INT_WRAP},
};

pub const ABI_VERSION: u32 = 1;
pub const STORAGE_SIZE: u32 = 20_480;
pub const STORAGE_ALIGN: u32 = 8;
const MAGIC: u32 = 0x4343_4501;
pub const UNKNOWN_CALL: i32 = -201;
pub const NOT_ALLOWED: i32 = -202;
pub const INVALID_VALUE: i32 = -203;
pub const INVALID_WIDTH: i32 = -204;
pub const HAS_IDENTITY: u32 = 1;
pub const DUPLEX: u32 = 2;
pub const REQUEST_TX: u32 = 4;
pub const HOOK: u32 = 8;
pub const BASIC_PRESENT: u32 = 16;
pub const COMPLETE: u32 = 32;
pub const OWNER_PRESENT: u32 = 64;

#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct Basic {
    pub circuit_mode: u8,
    pub encrypted: u8,
    pub communication: u8,
    pub parameter: u8,
}
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct Route {
    pub ssi: u32,
    pub ssi_type: u32,
    pub handle: u32,
    pub endpoint_id: u32,
    pub link_id: u32,
}
#[repr(C)]
pub struct Config {
    pub size: u32,
    pub abi_version: u32,
    pub own_issi: u32,
    pub flags: u32,
}
#[repr(C)]
pub struct Event {
    pub size: u32,
    pub abi_version: u32,
    pub kind: u32,
    pub now: u32,
    pub route: Route,
    pub ssi: u32,
    pub flags: u32,
    pub id: u16,
    pub pdu_bits: u16,
    pub basic: Basic,
    pub priority: u8,
    pub cause: u8,
    pub reserved: u16,
    pub pdu: [u8; MAX_PDU_BYTES as usize],
}
#[repr(C)]
pub struct Fields {
    pub present: u32,
    pub flags: u32,
    pub speaker: u32,
    pub extension: u32,
    pub id: u16,
    pub new_id: u16,
    pub external_offset: u16,
    pub external_bits: u16,
    pub dtmf_offset: u16,
    pub dtmf_bits: u16,
    pub basic: Basic,
    pub kind: u8,
    pub timeout: u8,
    pub setup_timeout: u8,
    pub grant: u8,
    pub priority: u8,
    pub notification: u8,
    pub call_status: u8,
    pub poll_percentage: u8,
    pub poll_number: u8,
    pub cause: u8,
    pub reserved: u16,
}
#[repr(C)]
pub struct Action {
    pub size: u32,
    pub abi_version: u32,
    pub kind: u32,
    pub flags: u32,
    pub route: Route,
    pub basic: Basic,
    pub id: u16,
    pub new_id: u16,
    pub sequence: u32,
    pub speaker: u32,
    pub called_party: u32,
    pub notice_kind: u8,
    pub grant: u8,
    pub timeout: u8,
    pub change: u8,
    pub fields: Fields,
    pub pdu_bits: u16,
    pub reserved: u16,
    pub pdu: [u8; MAX_PDU_BYTES as usize],
}
#[repr(C)]
pub struct CallStatus {
    pub route: Route,
    pub basic: Basic,
    pub id: u16,
    pub state: u8,
    pub kind: u8,
    pub flags: u32,
    pub speaker: u32,
    pub setup_deadline: u32,
    pub call_deadline: u32,
    pub rx_frames: u32,
    pub tx_frames: u32,
    pub grant: u8,
    pub timeout: u8,
    pub setup_timeout: u8,
    pub cause: u8,
}
#[repr(C)]
pub struct Pending {
    pub ssi: u32,
    pub ssi_type: u32,
    pub basic: Basic,
    pub duplex: u8,
    pub reserved: [u8; 3],
}
#[repr(C)]
pub struct Status {
    pub size: u32,
    pub abi_version: u32,
    pub flags: u32,
    pub own_issi: u32,
    pub now: u32,
    pub calls: u16,
    pub originations: u16,
    pub actions: u16,
    pub transmit_call: u16,
    pub call: [CallStatus; MAX_CALLS],
    pub pending: [Pending; MAX_ORIGINATIONS],
}
#[repr(C)]
struct State {
    magic: u32,
    core: CmceCore,
}
const _: () = {
    assert!(size_of::<State>() <= STORAGE_SIZE as usize && align_of::<State>() <= STORAGE_ALIGN as usize);
    assert!(!core::mem::needs_drop::<State>());
    assert!(size_of::<Config>() == 16 && size_of::<Basic>() == 4 && size_of::<Route>() == 20);
    assert!(size_of::<Event>() == 392 && size_of::<Fields>() == 44 && size_of::<Action>() == 444);
    assert!(align_of::<Event>() == 4 && align_of::<Action>() == 4 && align_of::<Status>() == 4);
    assert!(size_of::<CallStatus>() == 56 && size_of::<Pending>() == 16 && size_of::<Status>() == 1180);
    assert!(core::mem::offset_of!(Event, pdu) == 56 && core::mem::offset_of!(Action, pdu) == 108);
};
fn code(error: CmceError) -> i32 {
    match error {
        CmceError::InvalidValue | CmceError::Bits(BitError::InvalidValue) => INVALID_VALUE,
        CmceError::UnknownCall => UNKNOWN_CALL,
        CmceError::NotAllowed => NOT_ALLOWED,
        CmceError::Capacity | CmceError::Bits(BitError::Capacity) => CAPACITY,
        CmceError::Bits(BitError::Truncated) => TRUNCATED,
        CmceError::Bits(BitError::InvalidWidth) => INVALID_WIDTH,
        CmceError::Unsupported => UNSUPPORTED,
        CmceError::OutputFull => QUEUE_FULL,
    }
}
fn basic(input: Basic) -> Result<BasicService, i32> {
    if input.circuit_mode > 7 || input.encrypted > 1 || input.communication > 3 || input.parameter > 3 {
        return Err(INVALID_VALUE);
    }
    Ok(BasicService {
        circuit_mode: input.circuit_mode,
        encrypted: input.encrypted != 0,
        communication: input.communication,
        parameter: input.parameter,
    })
}
fn native_basic(input: BasicService) -> Basic {
    Basic {
        circuit_mode: input.circuit_mode,
        encrypted: u8::from(input.encrypted),
        communication: input.communication,
        parameter: input.parameter,
    }
}
fn address(ssi: u32, kind: u32) -> Result<TetraAddress, i32> {
    if ssi > 0xff_ffff {
        return Err(INVALID_IDENTITY);
    }
    let ssi_type = match kind {
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
    Ok(TetraAddress::new(ssi, ssi_type))
}
fn native_route(route: CallRoute) -> Route {
    Route {
        ssi: route.main_address.ssi,
        ssi_type: route.main_address.ssi_type as u32,
        handle: route.handle,
        endpoint_id: route.endpoint_id,
        link_id: route.link_id,
    }
}
fn validate_config(input: &Config) -> Result<Option<u32>, i32> {
    if input.size != size_of::<Config>() as u32 || input.abi_version != ABI_VERSION {
        return Err(ABI_MISMATCH);
    }
    if input.flags & !HAS_IDENTITY != 0 {
        return Err(ARGUMENT);
    }
    if input.flags & HAS_IDENTITY != 0 {
        if input.own_issi == 0 || input.own_issi > 0xff_ffff {
            return Err(INVALID_IDENTITY);
        }
        Ok(Some(input.own_issi))
    } else {
        Ok(None)
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
    // SAFETY: caller provides readable storage; only its initialized header is read.
    if unsafe { ptr::addr_of!((*state).magic).read() } != MAGIC {
        return Err(UNINITIALIZED);
    }
    Ok(state)
}
unsafe fn initialize(state: *mut State, own: Option<u32>) {
    // SAFETY: caller supplies exclusive aligned full storage. No complete-core temporary.
    unsafe {
        CmceCore::init_at(ptr::addr_of_mut!((*state).core));
        ptr::addr_of_mut!((*state).core.own_issi).write(own);
        ptr::addr_of_mut!((*state).magic).write(MAGIC);
    }
}
fn event(input: &Event) -> Result<CmceEvent<'_>, i32> {
    if input.size != size_of::<Event>() as u32 || input.abi_version != ABI_VERSION {
        return Err(ABI_MISMATCH);
    }
    if input.reserved != 0 {
        return Err(ARGUMENT);
    }
    if input.pdu_bits > MAX_PDU_BITS {
        return Err(CAPACITY);
    }
    if input.now >= TIME_INT_WRAP as u32 {
        return Err(INVALID_TIME);
    }
    let allowed = match input.kind {
        1 => HAS_IDENTITY,
        2 => 0,
        3 => DUPLEX | REQUEST_TX | HOOK,
        4 => DUPLEX | BASIC_PRESENT,
        5 => DUPLEX,
        6 => DUPLEX | BASIC_PRESENT | COMPLETE,
        7..=12 | 14..=17 => 0,
        13 => OWNER_PRESENT,
        _ => return Err(UNSUPPORTED),
    };
    if input.flags & !allowed != 0 {
        return Err(ARGUMENT);
    }
    let flags = input.flags;
    let id = input.id;
    if id > 0x3fff {
        return Err(INVALID_VALUE);
    }
    let data = BitView::new(&input.pdu, input.pdu_bits).map_err(|_| TRUNCATED)?;
    let duplex = flags & DUPLEX != 0;
    Ok(match input.kind {
        1 => {
            if flags & HAS_IDENTITY != 0 && (input.ssi == 0 || input.ssi > 0xff_ffff) {
                return Err(INVALID_IDENTITY);
            }
            CmceEvent::Identity((flags & HAS_IDENTITY != 0).then_some(input.ssi))
        }
        2 => CmceEvent::Downlink {
            sdu: data,
            route: CallRoute {
                main_address: address(input.route.ssi, input.route.ssi_type)?,
                handle: input.route.handle,
                endpoint_id: input.route.endpoint_id,
                link_id: input.route.link_id,
            },
        },
        3 => {
            let called = address(input.route.ssi, input.route.ssi_type)?;
            if !matches!(called.ssi_type, SsiType::Issi | SsiType::Gssi) {
                return Err(NOT_ALLOWED);
            }
            if !data.is_empty() {
                tetra_ms_core::cmce::decode_external_number(data, &mut [0; 24]).map_err(code)?;
            }
            CmceEvent::Originate {
                called,
                basic: basic(input.basic)?,
                duplex,
                request_tx: flags & REQUEST_TX != 0,
                hook: flags & HOOK != 0,
                external: (!data.is_empty()).then_some(data),
            }
        }
        4 => CmceEvent::Alert {
            id,
            basic: if flags & BASIC_PRESENT != 0 {
                Some(basic(input.basic)?)
            } else {
                None
            },
            duplex,
        },
        5 => CmceEvent::Connect {
            id,
            basic: basic(input.basic)?,
            duplex,
        },
        6 => CmceEvent::Answer {
            id,
            basic: if flags & BASIC_PRESENT != 0 {
                Some(basic(input.basic)?)
            } else {
                None
            },
            duplex,
            complete: flags & COMPLETE != 0,
        },
        7 => CmceEvent::Demand {
            id,
            priority: input.priority,
        },
        8 => CmceEvent::Cease { id },
        9 => CmceEvent::Disconnect { id, cause: input.cause },
        10 => CmceEvent::Release { id, cause: input.cause },
        11 => CmceEvent::Restore { id },
        12 => CmceEvent::Dtmf { id, data },
        13 => {
            if flags & OWNER_PRESENT != 0 && input.ssi > 0xff_ffff {
                return Err(INVALID_IDENTITY);
            }
            CmceEvent::Traffic {
                owner: (flags & OWNER_PRESENT != 0).then_some(input.ssi),
            }
        }
        14 => CmceEvent::Tick,
        15 => CmceEvent::Break,
        16 => CmceEvent::Reopen,
        17 => CmceEvent::Transmitted { id },
        _ => return Err(UNSUPPORTED),
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn tetra_cmce_abi_version() -> u32 {
    ABI_VERSION
}
#[unsafe(no_mangle)]
pub extern "C" fn tetra_cmce_core_size() -> u32 {
    STORAGE_SIZE
}
#[unsafe(no_mangle)]
pub extern "C" fn tetra_cmce_core_align() -> u32 {
    STORAGE_ALIGN
}
/// # Safety
/// Storage and input must be live, aligned, sufficiently sized and disjoint.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_cmce_init(storage: *mut c_void, storage_size: u32, input: *const Config) -> i32 {
    boundary(|| {
        let state = storage_ptr(storage)?;
        if storage_size < STORAGE_SIZE {
            return Err(CAPACITY);
        }
        disjoint_region(storage, STORAGE_SIZE, input)?;
        // SAFETY: input is readable by contract; pointer/overlap checked.
        let own = validate_config(unsafe { &*input })?;
        // SAFETY: full storage and all configuration values have been validated.
        unsafe { initialize(state, own) };
        Ok(())
    })
}
/// # Safety
/// Storage must be initialized/exclusive; input readable/aligned/disjoint.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_cmce_reset(storage: *mut c_void, input: *const Config) -> i32 {
    boundary(|| {
        disjoint_region(storage, STORAGE_SIZE, input)?;
        // SAFETY: input readable; pointer and disjointness validated.
        let own = validate_config(unsafe { &*input })?;
        // SAFETY: caller supplies initialized, exclusive storage.
        let state = unsafe { state_ptr(storage)? };
        // SAFETY: fixed primitive state has no Drop; complete initialization is in-place.
        unsafe { initialize(state, own) };
        Ok(())
    })
}
/// # Safety
/// Storage must be initialized/exclusive; event fully initialized/readable/disjoint.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_cmce_push_event(storage: *mut c_void, input: *const Event) -> i32 {
    boundary(|| {
        disjoint_region(storage, STORAGE_SIZE, input)?;
        // SAFETY: caller supplies readable event after pointer/overlap validation.
        let input = unsafe { &*input };
        let event = event(input)?;
        // SAFETY: state header validated before borrowing protocol fields.
        let state = unsafe { &mut *state_ptr(storage)? };
        state.core.push_event(TdmaTime::from_int(input.now as i32), event).map_err(code)
    })
}
fn fields(out: &mut Fields, p: Downlink) {
    out.kind = p.kind;
    out.id = p.id;
    out.grant = p.grant;
    out.cause = p.cause;
    out.flags = u32::from(p.hook)
        | (u32::from(p.duplex) << 1)
        | (u32::from(p.permission_denied) << 2)
        | (u32::from(p.ownership.unwrap_or(false)) << 3)
        | (u32::from(p.reset) << 4)
        | (u32::from(p.poll) << 5)
        | (u32::from(p.queued) << 6)
        | (u32::from(p.resume) << 7)
        | (u32::from(p.encrypted) << 8);
    macro_rules! opt {
        ($value:expr,$field:ident,$mask:expr) => {
            if let Some(value) = $value {
                out.present |= $mask;
                out.$field = value;
            }
        };
    }
    opt!(p.timeout, timeout, 1);
    opt!(p.setup_timeout, setup_timeout, 2);
    if let Some(b) = p.basic {
        out.present |= 4;
        out.basic = native_basic(b);
    }
    opt!(p.priority, priority, 8);
    if p.ownership.is_some() {
        out.present |= 16;
    }
    opt!(p.speaker, speaker, 32);
    opt!(p.extension, extension, 64);
    opt!(p.notification, notification, 128);
    opt!(p.call_status, call_status, 256);
    opt!(p.new_id, new_id, 512);
    opt!(p.poll_percentage, poll_percentage, 1024);
    opt!(p.poll_number, poll_number, 2048);
    if let Some(span) = p.external {
        out.present |= 4096;
        out.external_offset = span.offset;
        out.external_bits = span.len;
    }
    if let Some(span) = p.dtmf {
        out.present |= 8192;
        out.dtmf_offset = span.offset;
        out.dtmf_bits = span.len;
    }
}
fn payload(out: &mut Action, pdu: &PackedBits) {
    out.pdu_bits = pdu.len();
    out.pdu[..pdu.as_bytes().len()].copy_from_slice(pdu.as_bytes());
}
fn action(out: &mut Action, next: CmceAction) {
    match next {
        CmceAction::Uplink {
            sdu,
            route,
            stealing,
            repeats,
        } => {
            out.kind = 1;
            out.route = native_route(route);
            out.flags = u32::from(stealing) | (u32::from(repeats) << 1);
            payload(out, &sdu);
        }
        CmceAction::Configure { id, route, basic, state } => {
            out.kind = 2;
            out.id = id;
            out.route = native_route(route);
            out.basic = native_basic(basic);
            out.flags = (u32::from(state.switch_u_plane) << 2) | (u32::from(state.tx_grant) << 3) | (u32::from(state.simplex_duplex) << 4);
        }
        CmceAction::Notify(n) => {
            out.kind = 3;
            out.id = n.id;
            out.route = native_route(n.route);
            out.notice_kind = n.kind as u8;
            out.basic = native_basic(n.basic);
            out.timeout = n.timeout;
            out.grant = n.grant;
            out.called_party = n.called_party;
            if let Some(speaker) = n.speaker {
                out.flags |= 32;
                out.speaker = speaker;
            }
            fields(&mut out.fields, n.fields);
            payload(out, &n.sdu);
        }
        CmceAction::Speech(s) => {
            out.kind = 4;
            out.id = s.id;
            out.sequence = s.sequence;
            if let Some(speaker) = s.speaker {
                out.flags |= 32;
                out.speaker = speaker;
            }
        }
        CmceAction::Call { change, id, new_id } => {
            out.kind = 5;
            out.id = id;
            out.new_id = new_id;
            out.change = match change {
                CallChange::Created => 0,
                CallChange::Rekeyed => 1,
                CallChange::Removed => 2,
            };
        }
    }
}
/// # Safety
/// Storage is initialized/exclusive; output writable/aligned/disjoint with initialized headers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_cmce_poll_action(storage: *mut c_void, output: *mut Action) -> i32 {
    boundary(|| {
        disjoint_region(storage, STORAGE_SIZE, output)?;
        // SAFETY: only initialized integer headers read before full initialization.
        if unsafe { ptr::addr_of!((*output).size).read() } != size_of::<Action>() as u32
            || unsafe { ptr::addr_of!((*output).abi_version).read() } != ABI_VERSION
        {
            return Err(ABI_MISMATCH);
        }
        // SAFETY: caller supplies initialized exclusive state.
        let next = unsafe { (&mut *state_ptr(storage)?).core.pop_action() }.ok_or(EMPTY)?;
        // SAFETY: output contains only integers and arrays, so zero initializes it completely.
        let output = unsafe {
            ptr::write_bytes(output, 0, 1);
            &mut *output
        };
        output.size = size_of::<Action>() as u32;
        output.abi_version = ABI_VERSION;
        action(output, next);
        Ok(())
    })
}
/// # Safety
/// Storage readable/initialized without concurrent writes; output writable/aligned/disjoint.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_cmce_get_status(storage: *const c_void, output: *mut Status) -> i32 {
    boundary(|| {
        disjoint_region(storage, STORAGE_SIZE, output)?;
        // SAFETY: caller initializes these two integer headers only.
        if unsafe { ptr::addr_of!((*output).size).read() } != size_of::<Status>() as u32
            || unsafe { ptr::addr_of!((*output).abi_version).read() } != ABI_VERSION
        {
            return Err(ABI_MISMATCH);
        }
        // SAFETY: initialized shared storage, no concurrent writer.
        let core = unsafe { &(*state_ptr(storage)?).core };
        // SAFETY: all-zero is valid for this complete integer-only output.
        let out = unsafe {
            ptr::write_bytes(output, 0, 1);
            &mut *output
        };
        out.size = size_of::<Status>() as u32;
        out.abi_version = ABI_VERSION;
        out.now = core.dltime.to_int() as u32;
        if let Some(own) = core.own_issi {
            out.flags |= 1;
            out.own_issi = own;
        }
        if let Some(id) = core.transmit_target() {
            out.flags |= 2;
            out.transmit_call = id;
        }
        let status = core.status();
        out.calls = status.calls;
        out.originations = status.originations;
        out.actions = status.queued_actions;
        for (out, call) in out.call.iter_mut().zip(core.calls.values()) {
            out.route = native_route(call.route);
            out.basic = native_basic(call.basic_service);
            out.id = call.call_identifier;
            out.state = call.state as u8;
            out.kind = call.kind as u8;
            out.grant = call.tx_grant_state as u8;
            out.timeout = call.timers.call_timeout;
            out.rx_frames = call.rx_speech_frames;
            out.tx_frames = call.tx_speech_frames;
            if let Some(s) = call.current_speaker_ssi {
                out.flags |= 1;
                out.speaker = s;
            }
            if let Some(t) = call.timers.setup_phase_deadline {
                out.flags |= 2;
                out.setup_deadline = t.to_int() as u32;
            }
            if let Some(t) = call.timers.call_deadline {
                out.flags |= 4;
                out.call_deadline = t.to_int() as u32;
            }
            if let Some(t) = call.timers.setup_timeout {
                out.flags |= 8;
                out.setup_timeout = t;
            }
            if let Some(c) = call.disconnect_cause {
                out.flags |= 16;
                out.cause = c;
            }
            out.flags |= (u32::from(call.transmission_request_allowed) << 5)
                | (u32::from(call.pending_tx_request) << 6)
                | (u32::from(call.hook_on_off) << 7)
                | (u32::from(call.simplex_duplex_selection) << 8);
            if let Some(u) = call.last_uplane {
                out.flags |=
                    512 | (u32::from(u.switch_u_plane) << 10) | (u32::from(u.tx_grant) << 11) | (u32::from(u.simplex_duplex) << 16);
            }
            if let Some(u) = call.uplane_before_wait {
                out.flags |=
                    4096 | (u32::from(u.switch_u_plane) << 13) | (u32::from(u.tx_grant) << 14) | (u32::from(u.simplex_duplex) << 15);
            }
        }

        for (out, pending) in out.pending.iter_mut().zip(core.pending_originations.iter()) {
            out.ssi = pending.called_party.ssi;
            out.ssi_type = pending.called_party.ssi_type as u32;
            out.basic = native_basic(pending.basic_service);
            out.duplex = u8::from(pending.simplex_duplex_selection);
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
            own_issi: 123,
            flags: HAS_IDENTITY,
        }
    }
    fn event() -> Event {
        // SAFETY: Event contains only integers and byte arrays.
        let mut event: Event = unsafe { core::mem::zeroed() };
        event.size = size_of::<Event>() as u32;
        event.abi_version = ABI_VERSION;
        event.kind = 3;
        event.route.ssi = 91;
        event.route.ssi_type = 3;
        event.basic.communication = 1;
        event
    }
    #[test]
    fn invalid_events_and_output_headers_preserve_state_and_action() {
        let mut bytes = std::vec![0u64;STORAGE_SIZE as usize/8];
        let storage = bytes.as_mut_ptr().cast::<c_void>();
        let mut event = event();
        let config = config();
        let mut output = core::mem::MaybeUninit::<Action>::uninit();
        // SAFETY: caller-owned aligned live buffers are disjoint; only initialized headers read.
        unsafe {
            assert_eq!(tetra_cmce_init(storage, STORAGE_SIZE, &config), 0);
            assert_eq!(tetra_cmce_push_event(storage, &event), 0);
            event.now = TIME_INT_WRAP as u32;
            assert_eq!(tetra_cmce_push_event(storage, &event), INVALID_TIME);
            event.now = 1;
            event.flags = 1 << 31;
            assert_eq!(tetra_cmce_push_event(storage, &event), ARGUMENT);
            ptr::addr_of_mut!((*output.as_mut_ptr()).size).write(size_of::<Action>() as u32);
            ptr::addr_of_mut!((*output.as_mut_ptr()).abi_version).write(99);
            assert_eq!(tetra_cmce_poll_action(storage, output.as_mut_ptr()), ABI_MISMATCH);
            assert_eq!(tetra_cmce_poll_action(storage, storage.cast()), ARGUMENT);
            assert_eq!(
                tetra_cmce_poll_action(
                    storage,
                    storage.cast::<u8>().add(STORAGE_SIZE as usize - size_of::<Action>()).cast()
                ),
                ARGUMENT
            );
            ptr::addr_of_mut!((*output.as_mut_ptr()).abi_version).write(ABI_VERSION);
            assert_eq!(tetra_cmce_poll_action(storage, output.as_mut_ptr()), 0);
            let mut action = output.assume_init();
            assert_eq!((action.kind, action.route.ssi, action.route.ssi_type), (1, 123, 2));
            let snapshot = action.pdu;
            assert_eq!(tetra_cmce_poll_action(storage, &mut action), EMPTY);
            assert_eq!(action.pdu, snapshot);
            let mut status = core::mem::MaybeUninit::<Status>::uninit();
            ptr::addr_of_mut!((*status.as_mut_ptr()).size).write(size_of::<Status>() as u32);
            ptr::addr_of_mut!((*status.as_mut_ptr()).abi_version).write(ABI_VERSION);
            assert_eq!(tetra_cmce_get_status(storage, status.as_mut_ptr()), 0);
            let status = status.assume_init();
            assert_eq!((status.now, status.originations, status.actions), (0, 1, 0));
            assert_eq!(status.pending[0].ssi, 91);
            assert_eq!(tetra_cmce_reset(storage, &config), 0);
            assert_eq!(tetra_cmce_poll_action(storage, &mut action), EMPTY);
        }
    }
    #[test]
    fn null_misaligned_alias_and_invalid_config_never_initialize() {
        let mut bytes = std::vec![0u64;STORAGE_SIZE as usize/8+1];
        let storage = bytes.as_mut_ptr().cast::<c_void>();
        let mut config = config();
        // SAFETY: bad pointer shapes are rejected before dereference; valid buffers are live/disjoint.
        unsafe {
            assert_eq!(tetra_cmce_init(ptr::null_mut(), STORAGE_SIZE, &config), ARGUMENT);
            assert_eq!(tetra_cmce_init(storage.cast::<u8>().add(1).cast(), STORAGE_SIZE, &config), ARGUMENT);
            assert_eq!(tetra_cmce_init(storage, STORAGE_SIZE, storage.cast()), ARGUMENT);
            config.own_issi = 0x100_0000;
            assert_eq!(tetra_cmce_init(storage, STORAGE_SIZE, &config), INVALID_IDENTITY);
            assert!(bytes.iter().all(|word| *word == 0));
        }
    }
    #[cfg(feature = "c-smoke")]
    #[test]
    fn c_header_and_client() {
        unsafe extern "C" {
            fn tetra_cmce_c_smoke() -> i32;
        }
        // SAFETY: the C smoke client owns and validates all buffers it passes.
        assert_eq!(unsafe { tetra_cmce_c_smoke() }, 0);
    }
}
