//! MS MLE selection, survey, service and layer-3 routing (clauses 17/18).
//!
//! The caller supplies codeplug enumeration, current policy and PHY heartbeat
//! events. There is no clock or allocator here. A rejected event leaves protocol
//! state and queued actions unchanged; drain actions before retrying QueueFull.

use tetra_ms_types::{BitError, BitView, FragmentBits, PackedBits};

use crate::queue::Queue;

pub const CANDIDATE_CAPACITY: usize = 256;
pub const GROUP_CAPACITY: usize = 64;
pub const ACTION_CAPACITY: usize = 8;
pub const SURVEY_SYSINFO_SYNC_LIMIT: u8 = 4;
pub const SURVEY_MONITOR_TICK_LIMIT: u16 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MleError {
    TruncatedPdu,
    InvalidProtocol(u8),
    UnsupportedProtocol(Protocol),
    UnsupportedMlePdu(u8),
    UnsupportedManagement,
    CapacityExceeded,
    QueueFull,
}

impl From<BitError> for MleError {
    fn from(error: BitError) -> Self {
        match error {
            BitError::Truncated => Self::TruncatedPdu,
            _ => Self::CapacityExceeded,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Network {
    pub mcc: u16,
    pub mnc: u16,
}

/// Borrowed policy, refreshed by the caller when its configuration changes.
/// The home network is always allowed, independently of the allowed-network list.
#[derive(Debug, Clone, Copy)]
pub struct MleConfig<'a> {
    pub home_network: Network,
    pub allowed_networks: &'a [Network],
    /// None (or a value outside 1..=16) preserves receive-only monitor behavior.
    pub subscriber_class: Option<u8>,
}

impl MleConfig<'_> {
    fn network_allowed(self, network: Network) -> bool {
        network == self.home_network || self.allowed_networks.contains(&network)
    }

    fn class_permitted(self, bitmap: u16) -> bool {
        match self.subscriber_class {
            Some(class @ 1..=16) => bitmap & (1 << (class - 1)) != 0,
            _ => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Protocol {
    Mm = 1,
    Cmce = 2,
    Sndcp = 4,
}

impl Protocol {
    /// Legacy destination mapping; SNDCP downlink is rejected before delivery
    /// until a separate packet-data entity is implemented.
    pub const fn destination(self) -> UpperLayer {
        match self {
            Self::Mm => UpperLayer::Mm,
            Self::Cmce | Self::Sndcp => UpperLayer::Cmce,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum UplinkSource {
    Mm { priority: Option<u8>, emergency: bool },
    Cmce { stealing_permission: bool },
}

#[derive(Debug, Clone, Copy)]
pub enum MleEvent<'a> {
    /// Both acknowledged and unacknowledged LLC delivery use identical routing.
    Downlink(BitView<'a>),
    Uplink {
        source: UplinkSource,
        sdu: BitView<'a>,
        endpoint_id: u32,
        link_id: u32,
    },
    Sync(BitView<'a>),
    Sysinfo(BitView<'a>),
    Monitor {
        downlink_available: bool,
        rssi_dbfs: Option<f32>,
    },
    /// Caller-owned scan-dwell expiry, not a timer started by this component.
    Dwell {
        candidates: &'a [u32],
    },
    SelectionMode {
        manual: bool,
    },
    Survey {
        start: bool,
        candidates: &'a [u32],
    },
    Camp {
        carrier_hz: u32,
        register: bool,
        candidates: &'a [u32],
    },
    Identities {
        issi: u32,
        attached_gssis: &'a [u32],
    },
    UPlane {
        switch_u_plane: bool,
        tx_grant: bool,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ServingCell {
    pub mcc: u16,
    pub mnc: u16,
    pub neighbor_cell_broadcast: u8,
    pub cell_load_ca: u8,
    pub late_entry_supported: bool,
    pub location_area: Option<u16>,
    pub subscriber_class: Option<u16>,
    pub registration_required: Option<bool>,
    pub system_wide_services: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FoundCell {
    pub carrier_hz: u32,
    pub mcc: u16,
    pub mnc: u16,
    pub location_area: Option<u16>,
    /// Colour code is not carried in D-MLE-SYNC; never synthesize one.
    pub colour_code: Option<u8>,
    pub rssi_dbfs: Option<f32>,
    pub registration_required: Option<bool>,
    pub late_entry_supported: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpperLayer {
    Cmce,
    Mm,
}

/// Lower-link request policy, including the existing unimplemented-field
/// defaults. Opaque platform reporters and channel-allocation records are not
/// protocol state and remain correlated by the caller.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LinkParameters {
    pub endpoint_id: u32,
    pub link_id: u32,
    pub priority: Option<u8>,
    pub emergency: bool,
    pub stealing_permission: bool,
    pub subscriber_class: i32,
    pub fcs: bool,
    pub air_interface_encryption: Option<i32>,
    pub stealing_repeats: Option<bool>,
    pub data_class: Option<i32>,
    pub request_handle: i32,
    pub graceful_degradation: Option<i32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeliveryParameters {
    pub handle: u32,
    pub channel_change_response_required: bool,
    pub channel_change_handle: Option<i32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MleAction {
    Tune {
        carrier_hz: u32,
    },
    ConfigureNetwork(Network),
    ConfigureIdentities {
        network: Network,
        issi: u32,
        groups: [u32; GROUP_CAPACITY],
        group_count: u16,
    },
    UPlane {
        switch_u_plane: bool,
        tx_grant: bool,
    },
    Activate {
        network: Network,
        location_area: u16,
        registration_required: bool,
        system_wide_services: bool,
        cell_type: i32,
    },
    Break {
        layer: UpperLayer,
        permitted_services: i32,
    },
    Reopen(UpperLayer),
    Rssi {
        rssi_dbfs: Option<f32>,
    },
    ScanResult(FoundCell),
    ScanComplete {
        found: u32,
        scanned: u32,
    },
    Deliver {
        protocol: Protocol,
        parameters: DeliveryParameters,
        sdu: FragmentBits,
    },
    Transmit {
        pdu: PackedBits,
        parameters: LinkParameters,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MleState {
    pub serving_cell: Option<ServingCell>,
    pub activate_confirmed: bool,
    pub out_of_service: bool,
    pub serving_cell_rssi_dbfs: Option<f32>,
    pub scanning: bool,
    pub scan_index: u16,
    pub selection_mode_manual: bool,
    pub camp_armed: bool,
    pub camp_force_register: bool,
    pub survey: bool,
    pub survey_index: u16,
    pub survey_pending: Option<FoundCell>,
    pub survey_sync_repeats: u8,
    pub survey_found: u32,
    pub survey_monitor_ticks: u16,
}

pub struct MleCore {
    state: MleState,
    scan_candidates: [u32; CANDIDATE_CAPACITY],
    scan_count: u16,
    survey_candidates: [u32; CANDIDATE_CAPACITY],
    survey_count: u16,
    actions: Queue<MleAction, ACTION_CAPACITY>,
}

/// Target-specific static storage sizes, without constructing the component.
#[derive(Debug, Clone, Copy)]
pub struct MemoryFootprint {
    pub core: u32,
    pub event: u32,
    pub action: u32,
    pub state: u32,
    pub action_queue: u32,
    pub scan_candidates: u32,
    pub survey_candidates: u32,
}

pub const MEMORY_FOOTPRINT: MemoryFootprint = MemoryFootprint {
    core: core::mem::size_of::<MleCore>() as u32,
    event: core::mem::size_of::<MleEvent<'static>>() as u32,
    action: core::mem::size_of::<MleAction>() as u32,
    state: core::mem::size_of::<MleState>() as u32,
    action_queue: core::mem::size_of::<Queue<MleAction, ACTION_CAPACITY>>() as u32,
    scan_candidates: core::mem::size_of::<[u32; CANDIDATE_CAPACITY]>() as u32,
    survey_candidates: core::mem::size_of::<[u32; CANDIDATE_CAPACITY]>() as u32,
};

impl Default for MleCore {
    fn default() -> Self {
        Self::new()
    }
}

impl MleCore {
    /// Initialize caller-owned storage without constructing a core-sized
    /// temporary. Reinitialization discards queued actions.
    pub fn init_in_place(out: &mut core::mem::MaybeUninit<Self>) -> &mut Self {
        let pointer = out.as_mut_ptr();
        // SAFETY: MaybeUninit supplies exclusively borrowed, aligned writable
        // storage. Every field is initialized before assuming the value valid.
        unsafe {
            core::ptr::addr_of_mut!((*pointer).state).write(MleState::default());
            core::ptr::write_bytes(
                core::ptr::addr_of_mut!((*pointer).scan_candidates).cast::<u32>(),
                0,
                CANDIDATE_CAPACITY,
            );
            core::ptr::addr_of_mut!((*pointer).scan_count).write(0);
            core::ptr::write_bytes(
                core::ptr::addr_of_mut!((*pointer).survey_candidates).cast::<u32>(),
                0,
                CANDIDATE_CAPACITY,
            );
            core::ptr::addr_of_mut!((*pointer).survey_count).write(0);
            Queue::init_at(core::ptr::addr_of_mut!((*pointer).actions));
            out.assume_init_mut()
        }
    }

    pub fn new() -> Self {
        Self {
            state: MleState::default(),
            scan_candidates: [0; CANDIDATE_CAPACITY],
            scan_count: 0,
            survey_candidates: [0; CANDIDATE_CAPACITY],
            survey_count: 0,
            actions: Queue::new(),
        }
    }

    pub fn state(&self) -> MleState {
        self.state
    }

    pub fn pop_action(&mut self) -> Option<MleAction> {
        self.actions.pop()
    }

    pub fn pending_actions(&self) -> u16 {
        self.actions.len()
    }

    /// Maximum actions produced by one event is four (RSSI + two BREAKs +
    /// cleared RSSI). Reserve before mutating state, including heartbeat counters.
    pub fn handle(&mut self, config: MleConfig<'_>, event: MleEvent<'_>) -> Result<(), MleError> {
        let needed = match event {
            MleEvent::Monitor { .. } => 4,
            MleEvent::Dwell { .. } | MleEvent::Sync(_) | MleEvent::Sysinfo(_) => 2,
            MleEvent::SelectionMode { .. } => 0,
            _ => 1,
        };
        if self.actions.free() < needed {
            return Err(MleError::QueueFull);
        }
        match event {
            MleEvent::Downlink(sdu) => self.downlink(sdu)?,
            MleEvent::Uplink {
                source,
                sdu,
                endpoint_id,
                link_id,
            } => {
                let (protocol, endpoint_id, link_id, priority, emergency, stealing_permission) = match source {
                    UplinkSource::Mm { priority, emergency } => (Protocol::Mm, 0, 0, priority, emergency, false),
                    UplinkSource::Cmce { stealing_permission } => (Protocol::Cmce, endpoint_id, link_id, None, false, stealing_permission),
                };
                let mut pdu = PackedBits::new();
                pdu.push(3, protocol as u32)?;
                pdu.append(sdu)?;
                self.actions.push(MleAction::Transmit {
                    pdu,
                    parameters: LinkParameters {
                        endpoint_id,
                        link_id,
                        priority,
                        emergency,
                        stealing_permission,
                        ..LinkParameters::default()
                    },
                });
            }
            MleEvent::Sync(sdu) => {
                let sync = Sync::parse(sdu)?;
                self.sync(config, sync);
            }
            MleEvent::Sysinfo(sdu) => {
                let info = Sysinfo::parse(sdu)?;
                self.sysinfo(config, info);
            }
            MleEvent::Monitor {
                downlink_available,
                rssi_dbfs,
            } => self.monitor(downlink_available, rssi_dbfs),
            MleEvent::Dwell { candidates } => {
                if self.state.survey {
                    self.survey_finalize();
                } else if self.state.serving_cell.is_none() && !self.state.selection_mode_manual {
                    if self.state.scanning {
                        self.advance_scan();
                    } else if !candidates.is_empty() {
                        Self::copy_candidates(&mut self.scan_candidates, candidates)?;
                        self.scan_count = candidates.len() as u16;
                        self.state.scan_index = 0;
                        self.state.scanning = true;
                        self.actions.push(MleAction::Tune { carrier_hz: candidates[0] });
                    }
                }
            }
            MleEvent::SelectionMode { manual } => {
                if self.state.selection_mode_manual != manual {
                    self.state.selection_mode_manual = manual;
                    self.state.camp_armed = false;
                    self.state.camp_force_register = false;
                    if manual {
                        self.state.scanning = false;
                    }
                }
            }
            MleEvent::Survey { start: true, candidates } => self.start_survey(candidates)?,
            MleEvent::Survey { start: false, .. } => {
                if self.state.survey {
                    self.state.survey = false;
                    self.state.survey_pending = None;
                    self.state.survey_sync_repeats = 0;
                    self.complete_survey(u32::from(self.state.survey_index));
                }
            }
            MleEvent::Camp {
                carrier_hz,
                register,
                candidates,
            } => {
                if candidates.is_empty() || candidates.contains(&carrier_hz) {
                    self.state.survey = false;
                    self.state.survey_pending = None;
                    self.state.scanning = false;
                    self.state.serving_cell = None;
                    self.state.activate_confirmed = false;
                    self.state.camp_armed = true;
                    self.state.camp_force_register = register;
                    self.actions.push(MleAction::Tune { carrier_hz });
                }
            }
            MleEvent::Identities { issi, attached_gssis } => {
                if attached_gssis.len() > GROUP_CAPACITY {
                    return Err(MleError::CapacityExceeded);
                }
                let mut groups = [0; GROUP_CAPACITY];
                groups[..attached_gssis.len()].copy_from_slice(attached_gssis);
                self.actions.push(MleAction::ConfigureIdentities {
                    network: config.home_network,
                    issi,
                    groups,
                    group_count: attached_gssis.len() as u16,
                });
            }
            MleEvent::UPlane { switch_u_plane, tx_grant } => {
                self.actions.push(MleAction::UPlane { switch_u_plane, tx_grant });
            }
        }
        Ok(())
    }

    fn copy_candidates(out: &mut [u32; CANDIDATE_CAPACITY], candidates: &[u32]) -> Result<(), MleError> {
        if candidates.len() > CANDIDATE_CAPACITY {
            return Err(MleError::CapacityExceeded);
        }
        out[..candidates.len()].copy_from_slice(candidates);
        Ok(())
    }

    fn downlink(&mut self, sdu: BitView<'_>) -> Result<(), MleError> {
        let raw = sdu.read(0, 3)? as u8;
        let payload = sdu.window(3, sdu.len() - 3)?;
        let protocol = match raw {
            1 => Protocol::Mm,
            2 => Protocol::Cmce,
            4 => return Err(MleError::UnsupportedProtocol(Protocol::Sndcp)),
            5 => return Err(MleError::UnsupportedMlePdu(payload.read(0, 3)? as u8)),
            6 => return Err(MleError::UnsupportedManagement),
            _ => return Err(MleError::InvalidProtocol(raw)),
        };
        self.actions.push(MleAction::Deliver {
            protocol,
            parameters: DeliveryParameters::default(),
            sdu: FragmentBits::from_view(payload)?,
        });
        Ok(())
    }

    fn advance_scan(&mut self) {
        if self.state.scanning && self.scan_count != 0 {
            self.state.scan_index = (self.state.scan_index + 1) % self.scan_count;
            self.actions.push(MleAction::Tune {
                carrier_hz: self.scan_candidates[usize::from(self.state.scan_index)],
            });
        }
    }

    fn start_survey(&mut self, candidates: &[u32]) -> Result<(), MleError> {
        Self::copy_candidates(&mut self.survey_candidates, candidates)?;
        self.survey_count = candidates.len() as u16;
        self.state.scanning = false;
        self.state.serving_cell = None;
        self.state.activate_confirmed = false;
        self.state.survey_pending = None;
        self.state.survey_sync_repeats = 0;
        self.state.survey_found = 0;
        self.state.survey_index = 0;
        self.state.survey_monitor_ticks = 0;
        self.state.survey = !candidates.is_empty();
        if let Some(carrier_hz) = candidates.first() {
            self.actions.push(MleAction::Tune { carrier_hz: *carrier_hz });
        } else {
            self.complete_survey(0);
        }
        Ok(())
    }

    fn complete_survey(&mut self, scanned: u32) {
        self.actions.push(MleAction::ScanComplete {
            found: self.state.survey_found,
            scanned,
        });
    }

    fn survey_finalize(&mut self) {
        if let Some(mut cell) = self.state.survey_pending.take() {
            cell.rssi_dbfs = self.state.serving_cell_rssi_dbfs;
            self.state.survey_found += 1;
            self.actions.push(MleAction::ScanResult(cell));
        }
        self.state.survey_sync_repeats = 0;
        self.state.survey_index += 1;
        self.state.survey_monitor_ticks = 0;
        if self.state.survey_index >= self.survey_count {
            self.state.survey = false;
            self.complete_survey(u32::from(self.survey_count));
        } else {
            self.actions.push(MleAction::Tune {
                carrier_hz: self.survey_candidates[usize::from(self.state.survey_index)],
            });
        }
    }

    fn monitor(&mut self, available: bool, rssi: Option<f32>) {
        if rssi.is_some() && self.state.serving_cell_rssi_dbfs != rssi {
            self.state.serving_cell_rssi_dbfs = rssi;
            self.actions.push(MleAction::Rssi { rssi_dbfs: rssi });
        }
        if self.state.survey {
            self.state.survey_monitor_ticks = self.state.survey_monitor_ticks.saturating_add(1);
            if self.state.survey_monitor_ticks >= SURVEY_MONITOR_TICK_LIMIT {
                self.survey_finalize();
            }
            return;
        }
        match (available, self.state.out_of_service) {
            (false, false) => {
                self.state.out_of_service = true;
                self.actions.push(MleAction::Break {
                    layer: UpperLayer::Cmce,
                    permitted_services: 0,
                });
                self.actions.push(MleAction::Break {
                    layer: UpperLayer::Mm,
                    permitted_services: 0,
                });
                self.state.serving_cell = None;
                self.state.activate_confirmed = false;
                if self.state.serving_cell_rssi_dbfs.take().is_some() {
                    self.actions.push(MleAction::Rssi { rssi_dbfs: None });
                }
            }
            (true, true) => {
                self.state.out_of_service = false;
                self.actions.push(MleAction::Reopen(UpperLayer::Cmce));
                self.actions.push(MleAction::Reopen(UpperLayer::Mm));
            }
            _ => {}
        }
    }

    fn sync(&mut self, config: MleConfig<'_>, sync: Sync) {
        let network = Network {
            mcc: sync.mcc,
            mnc: sync.mnc,
        };
        let newly_selected = self
            .state
            .serving_cell
            .is_none_or(|cell| cell.mcc != sync.mcc || cell.mnc != sync.mnc);
        if self.state.survey {
            self.state.survey_monitor_ticks = 0;
            match self.state.survey_pending {
                Some(cell) if cell.mcc == sync.mcc && cell.mnc == sync.mnc => {
                    self.state.survey_sync_repeats = self.state.survey_sync_repeats.saturating_add(1);
                    if self.state.survey_sync_repeats >= SURVEY_SYSINFO_SYNC_LIMIT {
                        self.survey_finalize();
                    }
                }
                _ => {
                    self.state.survey_sync_repeats = 0;
                    self.state.survey_pending = Some(FoundCell {
                        carrier_hz: self.survey_candidates[usize::from(self.state.survey_index)],
                        mcc: sync.mcc,
                        mnc: sync.mnc,
                        location_area: None,
                        colour_code: None,
                        rssi_dbfs: self.state.serving_cell_rssi_dbfs,
                        registration_required: None,
                        late_entry_supported: sync.late_entry_supported,
                    });
                }
            }
            return;
        }
        if self.state.selection_mode_manual && newly_selected && !self.state.camp_armed {
            return;
        }
        if newly_selected && !config.network_allowed(network) {
            self.advance_scan();
            return;
        }
        let mut cell = if newly_selected {
            ServingCell::default()
        } else {
            self.state.serving_cell.unwrap_or_default()
        };
        cell.mcc = sync.mcc;
        cell.mnc = sync.mnc;
        cell.neighbor_cell_broadcast = sync.neighbor_cell_broadcast;
        cell.cell_load_ca = sync.cell_load_ca;
        cell.late_entry_supported = sync.late_entry_supported;
        self.state.serving_cell = Some(cell);
        if newly_selected {
            self.state.activate_confirmed = false;
            self.actions.push(MleAction::ConfigureNetwork(network));
        }
    }

    fn sysinfo(&mut self, config: MleConfig<'_>, info: Sysinfo) {
        if self.state.survey {
            if let Some(cell) = self.state.survey_pending.as_mut() {
                cell.location_area = Some(info.location_area);
                cell.registration_required = Some(info.registration_required);
                self.survey_finalize();
            }
            return;
        }
        let Some(cell) = self.state.serving_cell.as_mut() else { return };
        let changed = cell.location_area != Some(info.location_area) || cell.system_wide_services != Some(info.system_wide_services);
        cell.location_area = Some(info.location_area);
        cell.subscriber_class = Some(info.subscriber_class);
        cell.registration_required = Some(info.registration_required);
        cell.system_wide_services = Some(info.system_wide_services);
        let network = Network {
            mcc: cell.mcc,
            mnc: cell.mnc,
        };
        if !self.state.activate_confirmed || changed {
            if !config.class_permitted(info.subscriber_class) {
                if self.state.scanning {
                    self.state.serving_cell = None;
                    self.state.activate_confirmed = false;
                    self.advance_scan();
                }
                return;
            }
            self.state.activate_confirmed = true;
            self.state.scanning = false;
            self.actions.push(MleAction::Activate {
                network,
                location_area: info.location_area,
                registration_required: info.registration_required,
                system_wide_services: info.system_wide_services,
                cell_type: 0,
            });
            self.state.camp_armed = false;
        }
    }
}

struct Sync {
    mcc: u16,
    mnc: u16,
    neighbor_cell_broadcast: u8,
    cell_load_ca: u8,
    late_entry_supported: bool,
}

impl Sync {
    fn parse(bits: BitView<'_>) -> Result<Self, MleError> {
        Ok(Self {
            mcc: bits.read(0, 10)? as u16,
            mnc: bits.read(10, 14)? as u16,
            neighbor_cell_broadcast: bits.read(24, 2)? as u8,
            cell_load_ca: bits.read(26, 2)? as u8,
            late_entry_supported: bits.read(28, 1)? != 0,
        })
    }
}

struct Sysinfo {
    location_area: u16,
    subscriber_class: u16,
    registration_required: bool,
    system_wide_services: bool,
}

impl Sysinfo {
    fn parse(bits: BitView<'_>) -> Result<Self, MleError> {
        // Consume all 12 BS-service bits, including the tolerated reserved bit.
        let services = bits.read(30, 12)?;
        Ok(Self {
            location_area: bits.read(0, 14)? as u16,
            subscriber_class: bits.read(14, 16)? as u16,
            registration_required: services & (1 << 11) != 0,
            system_wide_services: services & (1 << 6) != 0,
        })
    }
}
