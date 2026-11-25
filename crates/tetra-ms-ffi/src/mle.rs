//! Caller-owned MS MLE ABI. See the maintained `include/tetra_mle.h`.

use core::{
    ffi::c_void,
    mem::{align_of, size_of},
    ptr,
};
use tetra_ms_core::{
    mle::{self, MleAction, MleConfig, MleCore, MleError, MleEvent, Network, UplinkSource, UpperLayer},
    types::BitView,
};

use super::{
    ABI_MISMATCH, ARGUMENT, CAPACITY, EMPTY, QUEUE_FULL, TRUNCATED, UNINITIALIZED, UNSUPPORTED, boundary, disjoint_region, valid_ptr,
};

pub const ABI_VERSION: u32 = 1;
pub const STORAGE_SIZE: u32 = 8192;
pub const STORAGE_ALIGN: u32 = 4;
pub const NETWORK_CAPACITY: usize = 64;
pub const PDU_BYTES: usize = 512;
pub const INVALID_PROTOCOL_BASE: i32 = -32;
pub const UNSUPPORTED_PDU_BASE: i32 = -48;
pub const UNSUPPORTED_MANAGEMENT: i32 = -64;
const MAGIC: u32 = 0x4d4c_4501;

pub const ENABLED: u32 = 1;
pub const RSSI_PRESENT: u32 = 2;
pub const PRIORITY_PRESENT: u32 = 4;
pub const EMERGENCY: u32 = 8;
pub const STEALING_PERMISSION: u32 = 16;
pub const TX_GRANT: u32 = 32;

pub const A_LOCATION_PRESENT: u32 = 1;
pub const A_COLOUR_PRESENT: u32 = 1 << 1;
pub const A_RSSI_PRESENT: u32 = 1 << 2;
pub const A_REGISTRATION_PRESENT: u32 = 1 << 3;
pub const A_REGISTRATION_REQUIRED: u32 = 1 << 4;
pub const A_LATE_ENTRY: u32 = 1 << 5;
pub const A_SYSTEM_WIDE_SERVICES: u32 = 1 << 6;
pub const A_SWITCH_U_PLANE: u32 = 1 << 7;
pub const A_TX_GRANT: u32 = 1 << 8;
pub const A_PRIORITY_PRESENT: u32 = 1 << 9;
pub const A_EMERGENCY: u32 = 1 << 10;
pub const A_STEALING_PERMISSION: u32 = 1 << 11;
pub const A_FCS: u32 = 1 << 12;
pub const A_ENCRYPTION_PRESENT: u32 = 1 << 13;
pub const A_REPEATS_PRESENT: u32 = 1 << 14;
pub const A_REPEATS: u32 = 1 << 15;
pub const A_DATA_CLASS_PRESENT: u32 = 1 << 16;
pub const A_GRACEFUL_PRESENT: u32 = 1 << 17;
pub const A_CHANNEL_RESPONSE: u32 = 1 << 18;
pub const A_CHANNEL_HANDLE_PRESENT: u32 = 1 << 19;

#[derive(Clone, Copy)]
#[repr(C)]
pub struct CNetwork {
    pub mcc: u16,
    pub mnc: u16,
}

#[repr(C)]
pub struct Config {
    pub size: u32,
    pub abi_version: u32,
    pub home_mcc: u16,
    pub home_mnc: u16,
    /// Zero means receive-only/no subscriber-class restriction.
    pub subscriber_class: u16,
    pub network_count: u16,
    pub networks: [CNetwork; NETWORK_CAPACITY],
}

#[repr(C)]
pub struct Event {
    pub size: u32,
    pub abi_version: u32,
    pub kind: u32,
    pub source: u32,
    pub endpoint_id: u32,
    pub link_id: u32,
    pub carrier_hz: u32,
    pub issi: u32,
    pub flags: u32,
    pub priority: u32,
    /// IEEE-754 binary32 bits, not an integer dB measurement.
    pub rssi_dbfs_bits: u32,
    pub pdu_bits: u16,
    pub value_count: u16,
    pub pdu: [u8; PDU_BYTES],
    /// Candidate carriers, or attached GSSIs for Identities.
    pub values: [u32; mle::CANDIDATE_CAPACITY],
}

#[repr(C)]
pub struct Action {
    pub size: u32,
    pub abi_version: u32,
    pub kind: u32,
    pub target: u32,
    pub protocol: u32,
    pub flags: u32,
    pub carrier_hz: u32,
    pub issi: u32,
    pub endpoint_id: u32,
    pub link_id: u32,
    pub mcc: u32,
    pub mnc: u32,
    pub location_area: u32,
    pub colour_code: u32,
    pub rssi_dbfs_bits: u32,
    pub found: u32,
    pub scanned: u32,
    pub priority: u32,
    pub subscriber_class: i32,
    pub request_handle: i32,
    pub air_interface_encryption: i32,
    pub data_class: i32,
    pub graceful_degradation: i32,
    pub channel_change_handle: i32,
    pub cell_type: i32,
    pub permitted_services: i32,
    pub handle: u32,
    pub pdu_bits: u16,
    pub group_count: u16,
    pub pdu: [u8; PDU_BYTES],
    pub groups: [u32; mle::GROUP_CAPACITY],
}

#[repr(C)]
pub struct Cell {
    pub flags: u32,
    pub carrier_hz: u32,
    pub mcc: u32,
    pub mnc: u32,
    pub neighbor_cell_broadcast: u32,
    pub cell_load_ca: u32,
    pub location_area: u32,
    pub subscriber_class: u32,
    pub colour_code: u32,
    pub rssi_dbfs_bits: u32,
}

#[repr(C)]
pub struct Status {
    pub size: u32,
    pub abi_version: u32,
    pub flags: u32,
    pub rssi_dbfs_bits: u32,
    pub survey_found: u32,
    pub queued_actions: u16,
    pub scan_index: u16,
    pub survey_index: u16,
    pub survey_monitor_ticks: u16,
    pub survey_sync_repeats: u16,
    pub reserved: u16,
    pub serving_cell: Cell,
    pub survey_pending: Cell,
}

#[repr(C)]
struct State {
    magic: u32,
    home_network: Network,
    networks: [Network; NETWORK_CAPACITY],
    network_count: u16,
    subscriber_class: u8,
    reserved: u8,
    core: MleCore,
}

const _: () = {
    assert!(size_of::<State>() <= STORAGE_SIZE as usize);
    assert!(align_of::<State>() <= STORAGE_ALIGN as usize);
    assert!(size_of::<CNetwork>() == 4);
    assert!(size_of::<Config>() == 272);
    assert!(size_of::<Event>() == 1584);
    assert!(size_of::<Action>() == 880);
    assert!(size_of::<Cell>() == 40);
    assert!(size_of::<Status>() == 112);
    assert!(align_of::<Event>() == 4 && align_of::<Action>() == 4);
    assert!(core::mem::offset_of!(Event, pdu) == 48);
    assert!(core::mem::offset_of!(Event, values) == 560);
    assert!(core::mem::offset_of!(Action, pdu) == 112);
    assert!(core::mem::offset_of!(Action, groups) == 624);
    assert!(core::mem::offset_of!(Status, serving_cell) == 32);
};

fn code(error: MleError) -> i32 {
    match error {
        MleError::TruncatedPdu => TRUNCATED,
        MleError::InvalidProtocol(value) => INVALID_PROTOCOL_BASE - i32::from(value),
        MleError::UnsupportedMlePdu(value) => UNSUPPORTED_PDU_BASE - i32::from(value),
        MleError::UnsupportedManagement => UNSUPPORTED_MANAGEMENT,
        MleError::CapacityExceeded => CAPACITY,
        MleError::QueueFull => QUEUE_FULL,
    }
}

fn validate_config(input: &Config) -> Result<(), i32> {
    if input.size != size_of::<Config>() as u32 || input.abi_version != ABI_VERSION {
        return Err(ABI_MISMATCH);
    }
    if usize::from(input.network_count) > NETWORK_CAPACITY {
        return Err(CAPACITY);
    }
    if input.home_mcc > 1023 || input.home_mnc > 16383 || input.subscriber_class > 16 {
        return Err(ARGUMENT);
    }
    if input.networks[..usize::from(input.network_count)]
        .iter()
        .any(|n| n.mcc > 1023 || n.mnc > 16383)
    {
        return Err(ARGUMENT);
    }
    Ok(())
}

unsafe fn state_ptr(storage: *const c_void) -> Result<*mut State, i32> {
    let state = storage.cast::<State>();
    valid_ptr(state)?;
    // SAFETY: The caller guarantees readable storage; only its aligned,
    // initialized integer header is inspected before accessing core fields.
    if unsafe { ptr::addr_of!((*state).magic).read() } != MAGIC {
        return Err(UNINITIALIZED);
    }
    Ok(state.cast_mut())
}

unsafe fn configure(state: *mut State, config: &Config) {
    // SAFETY: The caller supplies exclusive, disjoint, aligned storage for State
    // and a validated Config. No field is read before it is initialized.
    unsafe {
        ptr::addr_of_mut!((*state).home_network).write(Network {
            mcc: config.home_mcc,
            mnc: config.home_mnc,
        });
        let networks = ptr::addr_of_mut!((*state).networks).cast::<Network>();
        for i in 0..NETWORK_CAPACITY {
            let network = if i < usize::from(config.network_count) {
                Network {
                    mcc: config.networks[i].mcc,
                    mnc: config.networks[i].mnc,
                }
            } else {
                Network { mcc: 0, mnc: 0 }
            };
            networks.add(i).write(network);
        }
        ptr::addr_of_mut!((*state).network_count).write(config.network_count);
        ptr::addr_of_mut!((*state).subscriber_class).write(config.subscriber_class as u8);
        ptr::addr_of_mut!((*state).reserved).write(0);
    }
}

unsafe fn initialize(state: *mut State, config: &Config) {
    // SAFETY: The caller guarantees exclusive aligned State storage and a
    // validated, disjoint Config. Publish the magic only after full initialization.
    unsafe {
        ptr::addr_of_mut!((*state).magic).write(0);
        configure(state, config);
        let core = ptr::addr_of_mut!((*state).core).cast::<core::mem::MaybeUninit<MleCore>>();
        MleCore::init_in_place(&mut *core);
        ptr::addr_of_mut!((*state).magic).write(MAGIC);
    }
}

fn event(input: &Event) -> Result<MleEvent<'_>, i32> {
    if input.size != size_of::<Event>() as u32 || input.abi_version != ABI_VERSION {
        return Err(ABI_MISMATCH);
    }
    let value_capacity = if input.kind == 10 {
        mle::GROUP_CAPACITY
    } else {
        mle::CANDIDATE_CAPACITY
    };
    if usize::from(input.pdu_bits) > PDU_BYTES * 8 || usize::from(input.value_count) > value_capacity {
        return Err(CAPACITY);
    }
    let allowed_flags = match input.kind {
        1 | 3 | 4 | 6 | 10 => 0,
        2 => match input.source {
            1 => PRIORITY_PRESENT | EMERGENCY,
            2 => STEALING_PERMISSION,
            _ => return Err(ARGUMENT),
        },
        5 => ENABLED | RSSI_PRESENT,
        7..=9 => ENABLED,
        11 => ENABLED | TX_GRANT,
        _ => return Err(UNSUPPORTED),
    };
    if input.flags & !allowed_flags != 0 {
        return Err(ARGUMENT);
    }
    let bits = BitView::new(&input.pdu, input.pdu_bits).map_err(|_| TRUNCATED)?;
    let values = &input.values[..usize::from(input.value_count)];
    let enabled = input.flags & ENABLED != 0;
    Ok(match input.kind {
        1 => MleEvent::Downlink(bits),
        2 => MleEvent::Uplink {
            source: match input.source {
                1 => UplinkSource::Mm {
                    priority: if input.flags & PRIORITY_PRESENT != 0 {
                        Some(u8::try_from(input.priority).map_err(|_| ARGUMENT)?)
                    } else {
                        None
                    },
                    emergency: input.flags & EMERGENCY != 0,
                },
                2 => UplinkSource::Cmce {
                    stealing_permission: input.flags & STEALING_PERMISSION != 0,
                },
                _ => return Err(ARGUMENT),
            },
            sdu: bits,
            endpoint_id: input.endpoint_id,
            link_id: input.link_id,
        },
        3 => MleEvent::Sync(bits),
        4 => MleEvent::Sysinfo(bits),
        5 => MleEvent::Monitor {
            downlink_available: enabled,
            rssi_dbfs: (input.flags & RSSI_PRESENT != 0).then_some(f32::from_bits(input.rssi_dbfs_bits)),
        },
        6 => MleEvent::Dwell { candidates: values },
        7 => MleEvent::SelectionMode { manual: enabled },
        8 => MleEvent::Survey {
            start: enabled,
            candidates: values,
        },
        9 => MleEvent::Camp {
            carrier_hz: input.carrier_hz,
            register: enabled,
            candidates: values,
        },
        10 => MleEvent::Identities {
            issi: input.issi,
            attached_gssis: values,
        },
        11 => MleEvent::UPlane {
            switch_u_plane: enabled,
            tx_grant: input.flags & TX_GRANT != 0,
        },
        _ => return Err(UNSUPPORTED),
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn tetra_mle_abi_version() -> u32 {
    ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn tetra_mle_core_size() -> u32 {
    STORAGE_SIZE
}

#[unsafe(no_mangle)]
pub extern "C" fn tetra_mle_core_align() -> u32 {
    STORAGE_ALIGN
}

/// # Safety
/// Storage must own at least storage_size aligned writable bytes. Config must
/// designate a readable aligned object disjoint from the entire storage budget.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_mle_init(storage: *mut c_void, storage_size: u32, input: *const Config) -> i32 {
    boundary(|| {
        valid_ptr(storage.cast::<State>())?;
        if storage_size < STORAGE_SIZE {
            return Err(CAPACITY);
        }
        disjoint_region(storage, STORAGE_SIZE, input)?;
        // SAFETY: Caller guarantees a readable Config; helpers checked alignment,
        // null and overlap before any reference is formed.
        let input = unsafe { &*input };
        validate_config(input)?;
        // SAFETY: Storage capacity/alignment and complete Config were validated.
        unsafe { initialize(storage.cast(), input) };
        Ok(())
    })
}

/// # Safety
/// Storage must be initialized and exclusively accessible. Config must be
/// readable/aligned and disjoint. Reset cancels every queued action.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_mle_reset(storage: *mut c_void, input: *const Config) -> i32 {
    boundary(|| {
        disjoint_region(storage, STORAGE_SIZE, input)?;
        // SAFETY: Config validity and non-overlap are the caller contract,
        // checked for alignment/null/overlap before forming this reference.
        let input = unsafe { &*input };
        validate_config(input)?;
        // SAFETY: The caller owns live storage; the helper checks its header.
        let state = unsafe { state_ptr(storage)? };
        // SAFETY: State is exclusive and all Config fields are validated.
        unsafe { initialize(state, input) };
        Ok(())
    })
}

/// # Safety
/// Storage must be initialized/exclusive; Config must be readable, aligned and
/// disjoint. Unlike reset, policy updates preserve protocol state and actions.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_mle_configure(storage: *mut c_void, input: *const Config) -> i32 {
    boundary(|| {
        disjoint_region(storage, STORAGE_SIZE, input)?;
        // SAFETY: Config is readable and the helper checked pointer/overlap.
        let input = unsafe { &*input };
        validate_config(input)?;
        // SAFETY: The caller supplies live, exclusively accessed State storage.
        let state = unsafe { state_ptr(storage)? };
        // SAFETY: Config and State were validated and are disjoint.
        unsafe { configure(state, input) };
        Ok(())
    })
}

/// # Safety
/// Storage must be initialized/exclusive; Event must be readable/aligned and
/// disjoint. All borrowed bytes and lists are consumed during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_mle_push_event(storage: *mut c_void, input: *const Event) -> i32 {
    boundary(|| {
        disjoint_region(storage, STORAGE_SIZE, input)?;
        // SAFETY: Caller supplies a readable event, validated for null/alignment
        // and overlap before forming a shared reference.
        let input = unsafe { &*input };
        let event = event(input)?;
        // SAFETY: Storage is live and exclusively accessible per caller.
        let state = unsafe { &mut *state_ptr(storage)? };
        let policy = MleConfig {
            home_network: state.home_network,
            allowed_networks: &state.networks[..usize::from(state.network_count)],
            subscriber_class: (state.subscriber_class != 0).then_some(state.subscriber_class),
        };
        state.core.handle(policy, event).map_err(code)
    })
}

fn target(layer: UpperLayer) -> u32 {
    match layer {
        UpperLayer::Mm => 1,
        UpperLayer::Cmce => 2,
    }
}

fn bit(value: bool, mask: u32) -> u32 {
    u32::from(value) * mask
}

fn optional<T: Copy + Into<u32>>(flags: &mut u32, flag: u32, output: &mut u32, value: Option<T>) {
    if let Some(value) = value {
        *flags |= flag;
        *output = value.into();
    }
}

fn optional_i32(flags: &mut u32, flag: u32, output: &mut i32, value: Option<i32>) {
    if let Some(value) = value {
        *flags |= flag;
        *output = value;
    }
}

fn action(output: &mut Action, action: &MleAction) {
    match action {
        MleAction::Tune { carrier_hz } => {
            output.kind = 1;
            output.target = 5;
            output.carrier_hz = *carrier_hz;
        }
        MleAction::ConfigureNetwork(network) => {
            output.kind = 2;
            output.target = 5;
            output.mcc = u32::from(network.mcc);
            output.mnc = u32::from(network.mnc);
        }
        MleAction::ConfigureIdentities {
            network,
            issi,
            groups,
            group_count,
        } => {
            output.kind = 3;
            output.target = 5;
            output.issi = *issi;
            output.mcc = u32::from(network.mcc);
            output.mnc = u32::from(network.mnc);
            output.group_count = *group_count;
            output.groups[..usize::from(*group_count)].copy_from_slice(&groups[..usize::from(*group_count)]);
        }
        MleAction::UPlane { switch_u_plane, tx_grant } => {
            output.kind = 4;
            output.target = 5;
            output.flags = bit(*switch_u_plane, A_SWITCH_U_PLANE) | bit(*tx_grant, A_TX_GRANT);
        }
        MleAction::Activate {
            network,
            location_area,
            registration_required,
            system_wide_services,
            cell_type,
        } => {
            output.kind = 5;
            output.target = 1;
            output.mcc = u32::from(network.mcc);
            output.mnc = u32::from(network.mnc);
            output.location_area = u32::from(*location_area);
            output.cell_type = *cell_type;
            output.flags = A_LOCATION_PRESENT
                | A_REGISTRATION_PRESENT
                | bit(*registration_required, A_REGISTRATION_REQUIRED)
                | bit(*system_wide_services, A_SYSTEM_WIDE_SERVICES);
        }
        MleAction::Break { layer, permitted_services } => {
            output.kind = 6;
            output.target = target(*layer);
            output.permitted_services = *permitted_services;
        }
        MleAction::Reopen(layer) => {
            output.kind = 7;
            output.target = target(*layer);
        }
        MleAction::Rssi { rssi_dbfs } => {
            output.kind = 8;
            output.target = 1;
            optional(
                &mut output.flags,
                A_RSSI_PRESENT,
                &mut output.rssi_dbfs_bits,
                rssi_dbfs.map(f32::to_bits),
            );
        }
        MleAction::ScanResult(cell) => {
            output.kind = 9;
            output.target = 1;
            output.carrier_hz = cell.carrier_hz;
            output.mcc = u32::from(cell.mcc);
            output.mnc = u32::from(cell.mnc);
            optional(&mut output.flags, A_LOCATION_PRESENT, &mut output.location_area, cell.location_area);
            optional(&mut output.flags, A_COLOUR_PRESENT, &mut output.colour_code, cell.colour_code);
            optional(
                &mut output.flags,
                A_RSSI_PRESENT,
                &mut output.rssi_dbfs_bits,
                cell.rssi_dbfs.map(f32::to_bits),
            );
            if let Some(required) = cell.registration_required {
                output.flags |= A_REGISTRATION_PRESENT | bit(required, A_REGISTRATION_REQUIRED);
            }
            output.flags |= bit(cell.late_entry_supported, A_LATE_ENTRY);
        }
        MleAction::ScanComplete { found, scanned } => {
            output.kind = 10;
            output.target = 1;
            output.found = *found;
            output.scanned = *scanned;
        }
        MleAction::Deliver { protocol, parameters, sdu } => {
            output.kind = 11;
            output.target = target(protocol.destination());
            output.protocol = *protocol as u32;
            output.handle = parameters.handle;
            output.flags |= bit(parameters.channel_change_response_required, A_CHANNEL_RESPONSE);
            optional_i32(
                &mut output.flags,
                A_CHANNEL_HANDLE_PRESENT,
                &mut output.channel_change_handle,
                parameters.channel_change_handle,
            );
            output.pdu_bits = sdu.len();
            output.pdu[..sdu.as_bytes().len()].copy_from_slice(sdu.as_bytes());
        }
        MleAction::Transmit { pdu, parameters } => {
            output.kind = 12;
            output.target = 3;
            output.endpoint_id = parameters.endpoint_id;
            output.link_id = parameters.link_id;
            optional(&mut output.flags, A_PRIORITY_PRESENT, &mut output.priority, parameters.priority);
            output.flags |= bit(parameters.emergency, A_EMERGENCY)
                | bit(parameters.stealing_permission, A_STEALING_PERMISSION)
                | bit(parameters.fcs, A_FCS);
            output.subscriber_class = parameters.subscriber_class;
            output.request_handle = parameters.request_handle;
            optional_i32(
                &mut output.flags,
                A_ENCRYPTION_PRESENT,
                &mut output.air_interface_encryption,
                parameters.air_interface_encryption,
            );
            optional_i32(
                &mut output.flags,
                A_DATA_CLASS_PRESENT,
                &mut output.data_class,
                parameters.data_class,
            );
            optional_i32(
                &mut output.flags,
                A_GRACEFUL_PRESENT,
                &mut output.graceful_degradation,
                parameters.graceful_degradation,
            );
            if let Some(repeats) = parameters.stealing_repeats {
                output.flags |= A_REPEATS_PRESENT | bit(repeats, A_REPEATS);
            }
            output.pdu_bits = pdu.len();
            output.pdu[..pdu.as_bytes().len()].copy_from_slice(pdu.as_bytes());
        }
    }
}

/// # Safety
/// Storage must be live/exclusive; output must be writable/aligned/disjoint with
/// initialized size/version fields. EMPTY/errors leave output/action queue intact.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_mle_poll_action(storage: *mut c_void, output: *mut Action) -> i32 {
    boundary(|| {
        disjoint_region(storage, STORAGE_SIZE, output)?;
        // SAFETY: Output is aligned and its initialized headers are readable.
        if unsafe { (*output).size } != size_of::<Action>() as u32 || unsafe { (*output).abi_version } != ABI_VERSION {
            return Err(ABI_MISMATCH);
        }
        // SAFETY: Storage is initialized/exclusively accessible per caller.
        let state = unsafe { &mut *state_ptr(storage)? };
        let next = state.core.pop_action();
        let Some(next) = next.as_ref() else { return Err(EMPTY) };
        // SAFETY: Action is entirely fixed-width integers/byte arrays: all-zero
        // is valid. Write directly to avoid an additional action-sized temporary.
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
/// Storage must be initialized/readable and not concurrently mutated. Output
/// must be writable/aligned/disjoint with initialized size/version headers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tetra_mle_get_status(storage: *const c_void, output: *mut Status) -> i32 {
    boundary(|| {
        disjoint_region(storage, STORAGE_SIZE, output)?;
        // SAFETY: Output's header is initialized/readable by caller contract.
        if unsafe { (*output).size } != size_of::<Status>() as u32 || unsafe { (*output).abi_version } != ABI_VERSION {
            return Err(ABI_MISMATCH);
        }
        // SAFETY: Only shared reads; caller guarantees no concurrent writer.
        let state = unsafe { &*state_ptr(storage)? };
        let status = state.core.state();
        // SAFETY: Status/Cell contain only integers and arrays, so zero is valid.
        let output = unsafe {
            ptr::write_bytes(output, 0, 1);
            &mut *output
        };
        output.size = size_of::<Status>() as u32;
        output.abi_version = ABI_VERSION;
        output.flags = bit(status.serving_cell.is_some(), 1)
            | bit(status.activate_confirmed, 1 << 1)
            | bit(status.out_of_service, 1 << 2)
            | bit(status.serving_cell_rssi_dbfs.is_some(), 1 << 3)
            | bit(status.scanning, 1 << 4)
            | bit(status.selection_mode_manual, 1 << 5)
            | bit(status.camp_armed, 1 << 6)
            | bit(status.camp_force_register, 1 << 7)
            | bit(status.survey, 1 << 8)
            | bit(status.survey_pending.is_some(), 1 << 9);
        output.rssi_dbfs_bits = status.serving_cell_rssi_dbfs.map_or(0, f32::to_bits);
        output.queued_actions = state.core.pending_actions();
        output.scan_index = status.scan_index;
        output.survey_index = status.survey_index;
        output.survey_monitor_ticks = status.survey_monitor_ticks;
        output.survey_sync_repeats = u16::from(status.survey_sync_repeats);
        output.survey_found = status.survey_found;
        if let Some(cell) = status.serving_cell {
            let out = &mut output.serving_cell;
            out.mcc = u32::from(cell.mcc);
            out.mnc = u32::from(cell.mnc);
            out.neighbor_cell_broadcast = u32::from(cell.neighbor_cell_broadcast);
            out.cell_load_ca = u32::from(cell.cell_load_ca);
            optional(&mut out.flags, 1, &mut out.location_area, cell.location_area);
            optional(&mut out.flags, 1 << 1, &mut out.subscriber_class, cell.subscriber_class);
            if let Some(required) = cell.registration_required {
                out.flags |= (1 << 2) | bit(required, 1 << 3);
            }
            if let Some(supported) = cell.system_wide_services {
                out.flags |= (1 << 4) | bit(supported, 1 << 5);
            }
            out.flags |= bit(cell.late_entry_supported, 1 << 6);
            optional(
                &mut out.flags,
                1 << 8,
                &mut out.rssi_dbfs_bits,
                status.serving_cell_rssi_dbfs.map(f32::to_bits),
            );
        }
        if let Some(cell) = status.survey_pending {
            let out = &mut output.survey_pending;
            out.carrier_hz = cell.carrier_hz;
            out.mcc = u32::from(cell.mcc);
            out.mnc = u32::from(cell.mnc);
            optional(&mut out.flags, 1, &mut out.location_area, cell.location_area);
            if let Some(required) = cell.registration_required {
                out.flags |= (1 << 2) | bit(required, 1 << 3);
            }
            out.flags |= bit(cell.late_entry_supported, 1 << 6);
            optional(&mut out.flags, 1 << 7, &mut out.colour_code, cell.colour_code);
            optional(&mut out.flags, 1 << 8, &mut out.rssi_dbfs_bits, cell.rssi_dbfs.map(f32::to_bits));
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
            home_mcc: 901,
            home_mnc: 9999,
            subscriber_class: 1,
            network_count: 0,
            networks: [CNetwork { mcc: 0, mnc: 0 }; NETWORK_CAPACITY],
        }
    }

    fn event(kind: u32) -> Event {
        Event {
            size: size_of::<Event>() as u32,
            abi_version: ABI_VERSION,
            kind,
            source: 0,
            endpoint_id: 0,
            link_id: 0,
            carrier_hz: 0,
            issi: 0,
            flags: 0,
            priority: 0,
            rssi_dbfs_bits: 0,
            pdu_bits: 0,
            value_count: 0,
            pdu: [0; PDU_BYTES],
            values: [0; mle::CANDIDATE_CAPACITY],
        }
    }

    fn wire(kind: u32, fields: &[(u8, u32)]) -> Event {
        let mut bits = PackedBits::new();
        for &(width, value) in fields {
            bits.push(width, value).unwrap();
        }
        let mut event = event(kind);
        event.pdu_bits = bits.len();
        event.pdu[..bits.as_bytes().len()].copy_from_slice(bits.as_bytes());
        event
    }

    fn action_output() -> Action {
        // SAFETY: Action consists entirely of fixed-width integer fields/arrays.
        let mut result = unsafe { core::mem::MaybeUninit::<Action>::zeroed().assume_init() };
        result.size = size_of::<Action>() as u32;
        result.abi_version = ABI_VERSION;
        result
    }

    fn status_output() -> Status {
        // SAFETY: Status/Cell consist entirely of fixed-width integer fields.
        let mut result = unsafe { core::mem::MaybeUninit::<Status>::zeroed().assume_init() };
        result.size = size_of::<Status>() as u32;
        result.abi_version = ABI_VERSION;
        result
    }

    struct Session(Storage);

    impl Session {
        fn new(config: &Config) -> Self {
            let mut session = Self(Storage([0xa5; STORAGE_SIZE as usize]));
            // SAFETY: Storage is aligned/writable; Config is readable and disjoint.
            assert_eq!(unsafe { tetra_mle_init(session.pointer(), STORAGE_SIZE, config) }, OK);
            session
        }

        fn pointer(&mut self) -> *mut c_void {
            self.0.0.as_mut_ptr().cast()
        }

        fn push(&mut self, input: &Event) -> i32 {
            // SAFETY: Session owns initialized storage, distinct from input.
            unsafe { tetra_mle_push_event(self.pointer(), input) }
        }

        fn poll(&mut self) -> Action {
            let mut output = action_output();
            // SAFETY: Session and writable output are aligned, live and disjoint.
            assert_eq!(unsafe { tetra_mle_poll_action(self.pointer(), &mut output) }, OK);
            output
        }

        fn status(&mut self) -> Status {
            let mut output = status_output();
            // SAFETY: Session and writable output are aligned, live and disjoint.
            assert_eq!(unsafe { tetra_mle_get_status(self.pointer(), &mut output) }, OK);
            output
        }

        fn reset(&mut self, config: &Config) -> i32 {
            // SAFETY: Initialized session and readable config are disjoint.
            unsafe { tetra_mle_reset(self.pointer(), config) }
        }
    }

    #[test]
    fn lifecycle_headers_aliases_and_reset_preserve_canaries() {
        #[repr(C)]
        struct Guard {
            before: [u32; 4],
            storage: Storage,
            after: [u32; 4],
        }
        let mut guard = Guard {
            before: [0x1234; 4],
            storage: Storage([0; STORAGE_SIZE as usize]),
            after: [0x5678; 4],
        };
        let storage = guard.storage.0.as_mut_ptr().cast::<c_void>();
        let mut config = config();
        let mut output = status_output();
        output.queued_actions = 777;
        // SAFETY: Buffers are live/aligned/disjoint. Deliberately invalid
        // pointers/overlap are rejected before creating any references.
        unsafe {
            assert_eq!(tetra_mle_get_status(storage, &mut output), UNINITIALIZED);
            assert_eq!(output.queued_actions, 777);
            assert_eq!(tetra_mle_init(ptr::null_mut(), STORAGE_SIZE, &config), ARGUMENT);
            assert_eq!(tetra_mle_init(storage, STORAGE_SIZE - 1, &config), CAPACITY);
            assert_eq!(tetra_mle_init(storage, STORAGE_SIZE, ptr::null()), ARGUMENT);
            assert_eq!(
                tetra_mle_init(guard.storage.0.as_mut_ptr().add(1).cast(), STORAGE_SIZE, &config),
                ARGUMENT
            );
            config.abi_version += 1;
            assert_eq!(tetra_mle_init(storage, STORAGE_SIZE, &config), ABI_MISMATCH);
            config.abi_version = ABI_VERSION;
            config.subscriber_class = 17;
            assert_eq!(tetra_mle_init(storage, STORAGE_SIZE, &config), ARGUMENT);
            config.subscriber_class = 1;
            assert_eq!(tetra_mle_init(storage, STORAGE_SIZE, &config), OK);
            assert_eq!(tetra_mle_push_event(storage, &event(11)), OK);
            config.network_count = 65;
            assert_eq!(tetra_mle_reset(storage, &config), CAPACITY);
            assert_eq!(tetra_mle_get_status(storage, &mut output), OK);
            assert_eq!(output.queued_actions, 1);
            config.network_count = 0;
            assert_eq!(tetra_mle_reset(storage, &config), OK);
            assert_eq!(tetra_mle_get_status(storage, &mut output), OK);
            assert_eq!((output.flags, output.queued_actions, output.survey_found), (0, 0, 0));
            let tail = guard.storage.0.as_mut_ptr().add(STORAGE_SIZE as usize - size_of::<Action>());
            assert_eq!(tetra_mle_poll_action(storage, tail.cast()), ARGUMENT);
            assert_eq!(tetra_mle_push_event(storage, storage.cast()), ARGUMENT);
            assert_eq!(tetra_mle_reset(storage, storage.cast()), ARGUMENT);
            assert_eq!(tetra_mle_get_status(storage, storage.cast()), ARGUMENT);
            let mut action = action_output();
            action.kind = 0x1234_5678;
            assert_eq!(tetra_mle_poll_action(storage, &mut action), EMPTY);
            assert_eq!(action.kind, 0x1234_5678);
            output.abi_version = 0;
            output.flags = 0x1234_5678;
            assert_eq!(tetra_mle_get_status(storage, &mut output), ABI_MISMATCH);
            assert_eq!(output.flags, 0x1234_5678);
        }
        assert_eq!(guard.before, [0x1234; 4]);
        assert_eq!(guard.after, [0x5678; 4]);
    }

    #[test]
    fn polling_and_status_accept_header_only_initialized_output() {
        let mut session = Session::new(&config());
        let mut action = core::mem::MaybeUninit::<Action>::uninit();
        let mut status = core::mem::MaybeUninit::<Status>::uninit();
        // SAFETY: Only initialized header fields are read on EMPTY. Successful
        // calls initialize the entire integer-only output before assume_init.
        unsafe {
            let output = action.as_mut_ptr();
            ptr::addr_of_mut!((*output).size).write(size_of::<Action>() as u32);
            ptr::addr_of_mut!((*output).abi_version).write(ABI_VERSION);
            assert_eq!(tetra_mle_poll_action(session.pointer(), output), EMPTY);
            assert_eq!(ptr::addr_of!((*output).abi_version).read(), ABI_VERSION);
            assert_eq!(session.push(&event(11)), OK);
            assert_eq!(tetra_mle_poll_action(session.pointer(), output), OK);
            assert_eq!(action.assume_init().kind, 4);
            let output = status.as_mut_ptr();
            ptr::addr_of_mut!((*output).size).write(size_of::<Status>() as u32);
            ptr::addr_of_mut!((*output).abi_version).write(ABI_VERSION);
            assert_eq!(tetra_mle_get_status(session.pointer(), output), OK);
            assert_eq!(status.assume_init().queued_actions, 0);
        }
    }

    #[test]
    fn survey_camp_and_service_actions_preserve_all_presence_bits() {
        let mut config = config();
        config.network_count = 1;
        config.networks[0] = CNetwork { mcc: 238, mnc: 6 };
        let mut session = Session::new(&config);
        let mut survey = event(8);
        survey.flags = ENABLED;
        survey.value_count = 2;
        survey.values[..2].copy_from_slice(&[390_000_000, 396_000_000]);
        assert_eq!(session.push(&survey), OK);
        assert_eq!(session.poll().carrier_hz, 390_000_000);
        let sync = wire(3, &[(10, 238), (14, 6), (2, 2), (2, 3), (1, 1)]);
        assert_eq!(session.push(&sync), OK);
        let status = session.status();
        assert_eq!(status.flags, (1 << 8) | (1 << 9));
        assert_eq!(status.survey_pending.mcc, 238);
        assert_eq!(status.survey_pending.flags, 1 << 6);
        let info = wire(4, &[(14, 9), (16, 1), (12, 0x840)]);
        assert_eq!(session.push(&info), OK);
        let result = session.poll();
        assert_eq!(
            (result.kind, result.target, result.mcc, result.mnc, result.location_area),
            (9, 1, 238, 6, 9)
        );
        assert_eq!(
            result.flags,
            A_LOCATION_PRESENT | A_REGISTRATION_PRESENT | A_REGISTRATION_REQUIRED | A_LATE_ENTRY
        );
        assert_eq!(session.poll().carrier_hz, 396_000_000);
        assert_eq!(session.push(&event(6)), OK);
        let complete = session.poll();
        assert_eq!((complete.kind, complete.found, complete.scanned), (10, 1, 2));
        let mut mode = event(7);
        mode.flags = ENABLED;
        assert_eq!(session.push(&mode), OK);
        assert_eq!(session.push(&sync), OK);
        assert_eq!(session.status().queued_actions, 0);
        let mut camp = event(9);
        camp.carrier_hz = 396_000_000;
        camp.flags = ENABLED;
        assert_eq!(session.push(&camp), OK);
        assert_eq!(session.poll().kind, 1);
        assert_eq!(session.push(&sync), OK);
        let configure = session.poll();
        assert_eq!((configure.kind, configure.target, configure.mcc, configure.mnc), (2, 5, 238, 6));
        assert_eq!(session.push(&info), OK);
        let activate = session.poll();
        assert_eq!(activate.kind, 5);
        assert_eq!(
            activate.flags,
            A_LOCATION_PRESENT | A_REGISTRATION_PRESENT | A_REGISTRATION_REQUIRED | A_SYSTEM_WIDE_SERVICES
        );
        let status = session.status();
        assert_eq!(status.flags & ((1 << 0) | (1 << 1) | (1 << 6)), 3);
        assert_eq!((status.serving_cell.location_area, status.serving_cell.subscriber_class), (9, 1));
        assert_eq!(
            (status.serving_cell.neighbor_cell_broadcast, status.serving_cell.cell_load_ca),
            (2, 3)
        );
        assert_eq!(status.serving_cell.flags, 0x7f);
        let mut monitor = event(5);
        monitor.flags = RSSI_PRESENT;
        monitor.rssi_dbfs_bits = (-12.5_f32).to_bits();
        assert_eq!(session.push(&monitor), OK);
        let rssi = session.poll();
        assert_eq!(
            (rssi.kind, rssi.flags, rssi.rssi_dbfs_bits),
            (8, A_RSSI_PRESENT, monitor.rssi_dbfs_bits)
        );
        let cmce = session.poll();
        let mm = session.poll();
        let cleared = session.poll();
        assert_eq!(
            (cmce.kind, cmce.target, mm.kind, mm.target, cleared.kind, cleared.flags),
            (6, 2, 6, 1, 8, 0)
        );
        assert_ne!(session.status().flags & (1 << 2), 0);
        monitor.flags = ENABLED;
        assert_eq!(session.push(&monitor), OK);
        let cmce = session.poll();
        let mm = session.poll();
        assert_eq!((cmce.kind, cmce.target, mm.kind, mm.target), (7, 2, 7, 1));
        let mut identities = event(10);
        identities.issi = 42;
        identities.value_count = 2;
        identities.values[..2].copy_from_slice(&[91, 220]);
        assert_eq!(session.push(&identities), OK);
        let identities = session.poll();
        assert_eq!(
            (
                identities.kind,
                identities.issi,
                identities.mcc,
                identities.mnc,
                identities.group_count
            ),
            (3, 42, 901, 9999, 2)
        );
        assert_eq!(&identities.groups[..2], &[91, 220]);
        let mut uplane = event(11);
        uplane.flags = ENABLED | TX_GRANT;
        assert_eq!(session.push(&uplane), OK);
        let uplane = session.poll();
        assert_eq!((uplane.kind, uplane.flags), (4, A_SWITCH_U_PLANE | A_TX_GRANT));
    }

    #[test]
    fn uplink_downlink_large_payload_and_unused_output_bytes() {
        let mut session = Session::new(&config());
        for source in [1, 2] {
            let mut input = wire(2, &[(5, 0x12)]);
            input.source = source;
            input.endpoint_id = 19;
            input.link_id = 7;
            input.priority = 6;
            input.flags = if source == 1 {
                PRIORITY_PRESENT | EMERGENCY
            } else {
                STEALING_PERMISSION
            };
            assert_eq!(session.push(&input), OK);
            let action = session.poll();
            assert_eq!((action.kind, action.target, action.pdu_bits), (12, 3, 8));
            assert_eq!(action.pdu[0], if source == 1 { 0x32 } else { 0x52 });
            assert_eq!((action.endpoint_id, action.link_id), if source == 1 { (0, 0) } else { (19, 7) });
            assert_eq!(
                action.flags,
                if source == 1 {
                    A_PRIORITY_PRESENT | A_EMERGENCY
                } else {
                    A_STEALING_PERMISSION
                }
            );
            assert!(action.pdu[1..].iter().all(|&b| b == 0));
            assert!(action.groups.iter().all(|&g| g == 0));
            input.pdu_bits = 2686;
            assert_eq!(session.push(&input), CAPACITY);
        }
        for protocol in [1, 2, 4] {
            let input = wire(1, &[(3, protocol), (5, 0x12)]);
            assert_eq!(session.push(&input), OK);
            let action = session.poll();
            assert_eq!(
                (action.kind, action.protocol, action.target, action.pdu_bits),
                (11, protocol, if protocol == 1 { 1 } else { 2 }, 5)
            );
            assert_eq!(action.pdu[0], 0x90);
        }
        let mut large = event(1);
        large.pdu_bits = 4096;
        large.pdu[0] = 0x40;
        assert_eq!(session.push(&large), OK);
        let action = session.poll();
        assert_eq!((action.kind, action.pdu_bits), (11, 4093));
        assert_eq!(action.pdu[511] & 7, 0);
        assert_eq!(session.status().queued_actions, 0);
    }

    #[test]
    fn malformed_inputs_unknown_protocol_details_and_backpressure_are_explicit() {
        let mut session = Session::new(&config());
        let mut input = event(99);
        assert_eq!(session.push(&input), UNSUPPORTED);
        input.kind = 11;
        input.flags = 1 << 31;
        assert_eq!(session.push(&input), ARGUMENT);
        input.flags = 0;
        input.abi_version += 1;
        assert_eq!(session.push(&input), ABI_MISMATCH);
        input.abi_version = ABI_VERSION;
        input.pdu_bits = 4097;
        assert_eq!(session.push(&input), CAPACITY);
        input.pdu_bits = 0;
        input.value_count = 257;
        assert_eq!(session.push(&input), CAPACITY);
        input.value_count = 65;
        input.kind = 10;
        assert_eq!(session.push(&input), CAPACITY);
        input.value_count = 0;
        input.kind = 2;
        input.source = 99;
        assert_eq!(session.push(&input), ARGUMENT);
        input.source = 1;
        input.flags = PRIORITY_PRESENT;
        input.priority = 256;
        assert_eq!(session.push(&input), ARGUMENT);
        for raw in [0, 3, 7] {
            assert_eq!(session.push(&wire(1, &[(3, raw)])), INVALID_PROTOCOL_BASE - raw as i32);
        }
        for raw in 0..8 {
            assert_eq!(session.push(&wire(1, &[(3, 5), (3, raw)])), UNSUPPORTED_PDU_BASE - raw as i32);
        }
        assert_eq!(session.push(&wire(1, &[(3, 6)])), UNSUPPORTED_MANAGEMENT);
        for (kind, len) in [(3, 29), (4, 42)] {
            for bits in 0..len {
                let mut truncated = event(kind);
                truncated.pdu_bits = bits;
                assert_eq!(session.push(&truncated), TRUNCATED);
            }
        }
        assert_eq!(session.status().flags, 0);
        for _ in 0..mle::ACTION_CAPACITY {
            assert_eq!(session.push(&event(11)), OK);
        }
        assert_eq!(session.push(&event(11)), QUEUE_FULL);
        let mut output = action_output();
        output.abi_version = 0;
        output.kind = 0x1234;
        // SAFETY: Session/output are live and disjoint; malformed header is checked.
        assert_eq!(unsafe { tetra_mle_poll_action(session.pointer(), &mut output) }, ABI_MISMATCH);
        assert_eq!(output.kind, 0x1234);
        assert_eq!(session.status().queued_actions, 8);
        session.poll();
        assert_eq!(session.status().queued_actions, 7);
        assert_eq!(session.reset(&config()), OK);
        assert_eq!(session.status().queued_actions, 0);
    }

    #[test]
    fn configuration_and_candidate_inputs_are_copied_not_retained() {
        let mut config = config();
        config.network_count = 1;
        config.networks[0] = CNetwork { mcc: 238, mnc: 6 };
        let mut session = Session::new(&config);
        config.networks[0] = CNetwork { mcc: 1, mnc: 1 };
        let mut dwell = event(6);
        dwell.value_count = 2;
        dwell.values[..2].copy_from_slice(&[390_000_000, 396_000_000]);
        assert_eq!(session.push(&dwell), OK);
        assert_eq!(session.poll().carrier_hz, 390_000_000);
        dwell.values[..2].copy_from_slice(&[1, 2]);
        assert_eq!(session.push(&dwell), OK);
        assert_eq!(session.poll().carrier_hz, 396_000_000);
        assert_eq!(session.push(&wire(3, &[(10, 238), (14, 6), (5, 0)])), OK);
        assert_eq!(session.poll().kind, 2, "original copied allowed-network policy is still active");
        let flags = session.status().flags;
        assert_eq!(session.push(&event(11)), OK);
        // SAFETY: Session and its new policy are live, aligned and disjoint.
        assert_eq!(unsafe { tetra_mle_configure(session.pointer(), &config) }, OK);
        let status = session.status();
        assert_eq!((status.flags, status.queued_actions), (flags, 1));
        assert_eq!(session.poll().kind, 4);
        assert_eq!(session.push(&wire(3, &[(10, 1), (14, 1), (5, 0)])), OK);
        assert_eq!(session.poll().mcc, 1, "policy updates do not require discarding protocol state");
        assert_eq!(session.reset(&config), OK);
        assert_eq!(session.push(&wire(3, &[(10, 238), (14, 6), (5, 0)])), OK);
        assert_eq!(session.status().flags, 0, "reset adopted the changed allowed-network policy");
    }

    #[cfg(feature = "c-smoke")]
    #[test]
    fn linked_real_c_mle_caller() {
        unsafe extern "C" {
            fn tetra_mle_c_smoke() -> i32;
        }
        // SAFETY: The linked C smoke test owns its storage/inputs/outputs.
        assert_eq!(unsafe { tetra_mle_c_smoke() }, 0);
    }
}
