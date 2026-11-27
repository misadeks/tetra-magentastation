//! Allocator-free mobile-station mobility management.
//!
//! Time is supplied as elapsed recovered downlink slots. Drain actions before
//! submitting more events; an event is rejected atomically on capacity errors.
//! Security procedures and unsupported MM messages remain outside this engine.
//!
//! Owns the registration/temporary-registration conditions, T351 retries, N351
//! rejection lockout, standalone group reconciliation/T353, and bounded ITSI
//! detach drain from the MS application implementation (TS 100 392-2, clause 16).
//! Group membership is limited to 63 unique GSSIs; an on-air request or response
//! also has at most 63 repeated elements. Confirmations preserve wire order,
//! while the authoritative address-filter set is sorted and deduplicated.
//!
//! Compatibility includes expiry on the slot after T351/T353 reaches zero,
//! minimal initial demands followed by standalone group affiliation, and minimal
//! retry demands even for an infrastructure-command-triggered registration.
//! Unsolicited accepts and repeated downlink groups retain application behavior.
//! This is not a new implementation of previously unsupported security,
//! D-LOCATION-UPDATE-PROCEEDING, or network-initiated group attachment procedures:
//! those inputs return [`Error::UnsupportedPdu`] rather than changing state.
//!
//! The caller owns codeplug resolution, persistence, logging, transport receipts,
//! and conversion of actions to SAP/TNMM primitives. The supplied detach transfer
//! state must describe the receipt requested by the current detach Send action.
//! Batched time must be split when that external receipt changes. A batch that
//! cannot preserve every resulting action returns [`Error::ActionCapacity`]
//! without advancing time; drain actions and retry with smaller batches.
//! Decoding borrows validated group elements; only the final queued actions own
//! group arrays. Encoders write into one staged PDU before procedure state commits.

mod wire;

use crate::queue::Queue;
use tetra_ms_types::bits::{BitError, BitView, PackedBits};

pub const T351_TIMEOUT_SLOTS: u32 = 706;
pub const T353_TIMEOUT_SLOTS: u32 = 706;
/// Original implementation guard, approximately two multiframes, not a wire timer.
pub const DETACH_DRAIN_SLOTS: u32 = 144;
pub const MAX_REGISTRATION_ATTEMPTS: u8 = 5;
pub const N351_MAX_SYSTEM_REJECTIONS: u8 = 4;
pub const GROUP_CLASS_OF_USAGE: u8 = 4;
pub const MAX_GROUPS: usize = 63;
pub const ACTION_CAPACITY: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Bits(BitError),
    GroupCapacity,
    ActionCapacity,
    InvalidIdentity,
    InvalidPdu,
    UnsupportedPdu(u8),
    NotRegistered,
    GroupOperationPending,
    EmptyGroups,
    RegistrationInProgress,
    AlreadyRegistered,
    Detaching,
    LeftSystem,
}

impl From<BitError> for Error {
    fn from(value: BitError) -> Self {
        Self::Bits(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegState {
    Idle,
    Registering,
    Registered,
    Detaching,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellId {
    pub mcc: u16,
    pub mnc: u16,
    pub la: u16,
}

impl CellId {
    fn validate(self) -> Result<(), Error> {
        if self.mcc > 0x3ff || self.mnc > 0x3fff || self.la > 0x3fff {
            return Err(Error::InvalidIdentity);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LocationUpdateType {
    RoamingLocationUpdating = 0,
    MigratingLocationUpdating = 1,
    PeriodicLocationUpdating = 2,
    ItsiAttach = 3,
    ServiceRestorationRoamingLocationUpdating = 4,
    ServiceRestorationMigratingLocationUpdating = 5,
    DemandLocationUpdating = 6,
    DisabledMsUpdating = 7,
}

impl LocationUpdateType {
    pub fn priority(self) -> u8 {
        match self {
            Self::RoamingLocationUpdating | Self::MigratingLocationUpdating | Self::PeriodicLocationUpdating => 3,
            Self::ServiceRestorationRoamingLocationUpdating | Self::ServiceRestorationMigratingLocationUpdating => 5,
            _ => 6,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceStatus {
    OutOfService,
    WaitingForRegistration,
    InService,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferState {
    Pending,
    Transmitted,
    Acknowledged,
    Discarded,
    Lost,
}

/// Sorted, unique GSSIs, bounded by the on-air six-bit repetition count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupSet {
    values: [u32; MAX_GROUPS],
    len: u8,
}

impl Default for GroupSet {
    fn default() -> Self {
        Self::new()
    }
}

impl GroupSet {
    pub const fn new() -> Self {
        Self {
            values: [0; MAX_GROUPS],
            len: 0,
        }
    }

    pub fn from_slice(groups: &[u32]) -> Result<Self, Error> {
        let mut set = Self::new();
        for &gssi in groups {
            set.insert(gssi)?;
        }
        Ok(set)
    }

    pub fn as_slice(&self) -> &[u32] {
        &self.values[..usize::from(self.len)]
    }

    pub fn iter(&self) -> core::slice::Iter<'_, u32> {
        self.as_slice().iter()
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn contains(&self, gssi: &u32) -> bool {
        self.as_slice().binary_search(gssi).is_ok()
    }

    pub fn insert(&mut self, gssi: u32) -> Result<(), Error> {
        if gssi > 0xff_ffff {
            return Err(Error::InvalidIdentity);
        }
        let Err(pos) = self.as_slice().binary_search(&gssi) else {
            return Ok(());
        };
        let len = usize::from(self.len);
        if len == MAX_GROUPS {
            return Err(Error::GroupCapacity);
        }
        self.values.copy_within(pos..len, pos + 1);
        self.values[pos] = gssi;
        self.len += 1;
        Ok(())
    }

    fn remove(&mut self, gssi: &u32) {
        if let Ok(pos) = self.as_slice().binary_search(gssi) {
            let len = usize::from(self.len);
            self.values.copy_within(pos + 1..len, pos);
            self.len -= 1;
            self.values[usize::from(self.len)] = 0;
        }
    }
}

/// Wire-order identities used in confirmations, including repeated entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupList {
    values: [u32; MAX_GROUPS],
    len: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupAttachment {
    pub gssi: u32,
    pub class_of_usage: u8,
    pub lifetime: u8,
}

impl GroupAttachment {
    const GSSI_MASK: u32 = 0xff_ffff;
    const CLASS_SHIFT: u32 = 24;
    const LIFETIME_SHIFT: u32 = 27;

    pub const fn new(gssi: u32, class_of_usage: u8, lifetime: u8) -> Result<Self, Error> {
        if gssi > Self::GSSI_MASK {
            return Err(Error::InvalidIdentity);
        }
        if class_of_usage > 7 || lifetime > 3 {
            return Err(Error::InvalidPdu);
        }
        Ok(Self {
            gssi,
            class_of_usage,
            lifetime,
        })
    }

    fn packed(self) -> u32 {
        self.gssi | (u32::from(self.class_of_usage) << Self::CLASS_SHIFT) | (u32::from(self.lifetime) << Self::LIFETIME_SHIFT)
    }

    fn from_packed(packed: u32) -> Self {
        Self {
            gssi: packed & Self::GSSI_MASK,
            class_of_usage: ((packed >> Self::CLASS_SHIFT) & 7) as u8,
            lifetime: ((packed >> Self::LIFETIME_SHIFT) & 3) as u8,
        }
    }
}

/// Wire-order attachment metadata packed into the unused high bits of each
/// 24-bit identity, preserving the existing bounded action/storage footprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GroupAttachments(GroupList);

impl GroupAttachments {
    pub const fn new() -> Self {
        Self(GroupList::new())
    }

    pub fn from_slice(groups: &[GroupAttachment]) -> Result<Self, Error> {
        let mut result = Self::new();
        for group in groups {
            result.push(group.gssi, group.lifetime, group.class_of_usage)?;
        }
        Ok(result)
    }

    pub fn len(&self) -> usize {
        self.0.as_slice().len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = GroupAttachment> + '_ {
        self.0.iter().copied().map(GroupAttachment::from_packed)
    }

    fn push(&mut self, gssi: u32, lifetime: u8, class_of_usage: u8) -> Result<(), Error> {
        self.0.push(GroupAttachment::new(gssi, class_of_usage, lifetime)?.packed())
    }
}

impl Default for GroupList {
    fn default() -> Self {
        Self::new()
    }
}

impl GroupList {
    pub const fn new() -> Self {
        Self {
            values: [0; MAX_GROUPS],
            len: 0,
        }
    }

    pub fn as_slice(&self) -> &[u32] {
        &self.values[..usize::from(self.len)]
    }

    pub fn iter(&self) -> core::slice::Iter<'_, u32> {
        self.as_slice().iter()
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) fn push(&mut self, gssi: u32) -> Result<(), Error> {
        let len = usize::from(self.len);
        if len == MAX_GROUPS {
            return Err(Error::GroupCapacity);
        }
        self.values[len] = gssi;
        self.len += 1;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupRequest {
    pub gssi: u32,
    /// Attachment has a class of usage (0..7); detachment has a reason (0..3).
    pub operation: GroupOperation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupOperation {
    Attach { class_of_usage: u8 },
    Detach { reason: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingGroupOp {
    pub detach_all: bool,
    pub report_requested: bool,
    pub handle: u32,
    pub t353_countdown: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MmState {
    pub reg_state: RegState,
    pub t351_countdown: u32,
    pub attempts: u8,
    pub system_rejection_count: u8,
    pub left_system: bool,
    pub detach_countdown: u32,
    pub current_cell: Option<CellId>,
    pub registered_cell: Option<CellId>,
    pub pending_cell: Option<CellId>,
    pub pending_lu_type: LocationUpdateType,
    pub temporary_registration: bool,
    pub serving_la: u16,
    pub out_of_service: bool,
    pub attached_gssis: GroupSet,
    pub pending_group_op: Option<PendingGroupOp>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    pub issi: u32,
    pub mcc: u16,
    pub mnc: u16,
    pub initial_la: u16,
}

impl Config {
    pub fn validate(self) -> Result<(), Error> {
        if self.issi == 0 || self.issi > 0xff_ffff {
            return Err(Error::InvalidIdentity);
        }
        CellId {
            mcc: self.mcc,
            mnc: self.mnc,
            la: self.initial_la,
        }
        .validate()
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Event<'a> {
    Activate {
        cell: CellId,
        registration_required: bool,
        system_wide_services: bool,
        force_register: bool,
    },
    Register,
    Downlink(BitView<'a>),
    LinkBreak,
    LinkReopen,
    Groups {
        handle: u32,
        detach_all: bool,
        report_requested: bool,
        requests: &'a [GroupRequest],
    },
    Deregister,
    Tick {
        elapsed_slots: u32,
        detach_transfer: TransferState,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Send {
        sdu: PackedBits,
        priority: u8,
        detach_receipt: bool,
    },
    Service(ServiceStatus),
    RegistrationSuccess {
        attached: GroupAttachments,
    },
    RegistrationFailure {
        cause: Option<u8>,
    },
    Identities {
        attached: GroupSet,
        detached: GroupList,
    },
    GroupConfirm {
        op: PendingGroupOp,
        attached: GroupAttachments,
        detached: GroupList,
    },
    DetachReport {
        successful: bool,
    },
    DetachComplete,
}

pub struct MmCore {
    config: Config,
    state: MmState,
    actions: Queue<Action, ACTION_CAPACITY>,
}

const _: () = assert!(!core::mem::needs_drop::<MmCore>());

impl MmCore {
    fn initial_state(config: Config, groups: &[u32]) -> Result<MmState, Error> {
        config.validate()?;
        Ok(MmState {
            reg_state: RegState::Idle,
            t351_countdown: 0,
            attempts: 0,
            system_rejection_count: 0,
            left_system: false,
            detach_countdown: 0,
            current_cell: None,
            registered_cell: None,
            pending_cell: None,
            pending_lu_type: LocationUpdateType::ItsiAttach,
            temporary_registration: false,
            serving_la: config.initial_la,
            out_of_service: false,
            attached_gssis: GroupSet::from_slice(groups)?,
            pending_group_op: None,
        })
    }

    pub fn new(config: Config, groups: &[u32]) -> Result<Self, Error> {
        Ok(Self {
            config,
            state: Self::initial_state(config, groups)?,
            actions: Queue::new(),
        })
    }

    /// Initialize static/caller-owned storage without a complete core temporary.
    /// Invalid configuration leaves every byte of the destination untouched.
    pub fn init_in_place<'s>(storage: &'s mut core::mem::MaybeUninit<Self>, config: Config, groups: &[u32]) -> Result<&'s mut Self, Error> {
        config.validate()?;
        let mut attached = GroupSet::new();
        for &gssi in groups {
            attached.insert(gssi)?;
        }
        let out = storage.as_mut_ptr();
        // SAFETY: MaybeUninit provides exclusive aligned storage. Initialize
        // every state field and queue slot, never a complete temporary core or
        // state. The compile-time no-Drop assertion permits resetting this way.
        unsafe {
            core::ptr::addr_of_mut!((*out).config).write(config);
            core::ptr::addr_of_mut!((*out).state.reg_state).write(RegState::Idle);
            core::ptr::addr_of_mut!((*out).state.t351_countdown).write(0);
            core::ptr::addr_of_mut!((*out).state.attempts).write(0);
            core::ptr::addr_of_mut!((*out).state.system_rejection_count).write(0);
            core::ptr::addr_of_mut!((*out).state.left_system).write(false);
            core::ptr::addr_of_mut!((*out).state.detach_countdown).write(0);
            core::ptr::addr_of_mut!((*out).state.current_cell).write(None);
            core::ptr::addr_of_mut!((*out).state.registered_cell).write(None);
            core::ptr::addr_of_mut!((*out).state.pending_cell).write(None);
            core::ptr::addr_of_mut!((*out).state.pending_lu_type).write(LocationUpdateType::ItsiAttach);
            core::ptr::addr_of_mut!((*out).state.temporary_registration).write(false);
            core::ptr::addr_of_mut!((*out).state.serving_la).write(config.initial_la);
            core::ptr::addr_of_mut!((*out).state.out_of_service).write(false);
            core::ptr::addr_of_mut!((*out).state.attached_gssis).write(attached);
            core::ptr::addr_of_mut!((*out).state.pending_group_op).write(None);
            Queue::init_at(core::ptr::addr_of_mut!((*out).actions));
            Ok(storage.assume_init_mut())
        }
    }

    /// Cancel pending procedures/actions and reset in place. Invalid input is atomic.
    /// The caller must discard external detach receipts associated with the old state.
    pub fn reset(&mut self, config: Config, groups: &[u32]) -> Result<(), Error> {
        let storage = core::ptr::from_mut(self).cast::<core::mem::MaybeUninit<Self>>();
        // SAFETY: Self is aligned/exclusive and has no Drop resources (asserted
        // above). Invalid input leaves it untouched; success initializes every
        // field before exposing Self again.
        Self::init_in_place(unsafe { &mut *storage }, config, groups).map(|_| ())
    }

    pub fn state(&self) -> &MmState {
        &self.state
    }

    /// Update caller-provided identity configuration without resetting protocol timers.
    pub fn configure(&mut self, config: Config) -> Result<(), Error> {
        config.validate()?;
        self.config = config;
        Ok(())
    }

    /// Replace the caller's desired groups before registration (or restored state).
    pub fn configure_groups(&mut self, groups: &[u32]) -> Result<(), Error> {
        self.state.attached_gssis = GroupSet::from_slice(groups)?;
        Ok(())
    }

    pub fn poll_action(&mut self) -> Option<Action> {
        self.actions.pop()
    }

    pub fn action_count(&self) -> u16 {
        self.actions.len()
    }

    pub fn service_status(&self) -> ServiceStatus {
        if self.state.out_of_service {
            return ServiceStatus::OutOfService;
        }
        match self.state.reg_state {
            RegState::Registered => ServiceStatus::InService,
            RegState::Registering => ServiceStatus::WaitingForRegistration,
            _ => ServiceStatus::OutOfService,
        }
    }

    pub fn deregistration_pending(&self) -> bool {
        self.state.reg_state == RegState::Detaching && self.state.detach_countdown > 0
    }

    /// On error neither protocol state nor previously queued actions change.
    #[inline(never)]
    pub fn handle(&mut self, event: Event<'_>) -> Result<(), Error> {
        // Reserve before parsing to avoid a whole-engine transactional stack copy.
        // Time batches preflight their exact action count separately.
        if !matches!(event, Event::Tick { .. }) && self.actions.free() < 5 {
            return Err(Error::ActionCapacity);
        }
        match event {
            Event::Activate {
                cell,
                registration_required,
                system_wide_services,
                force_register,
            } => self.activate(cell, registration_required, system_wide_services, force_register),
            Event::Register => self.register(),
            Event::Downlink(bits) => self.receive(bits),
            Event::LinkBreak => self.link_change(true),
            Event::LinkReopen => self.link_change(false),
            Event::Groups {
                handle,
                detach_all,
                report_requested,
                requests,
            } => self.begin_groups(handle, detach_all, report_requested, requests),
            Event::Deregister => self.deregister(),
            Event::Tick {
                elapsed_slots,
                detach_transfer,
            } => self.tick(elapsed_slots, detach_transfer),
        }
    }

    // Keep procedure-specific PDU/action temporaries out of the dispatcher's
    // frame. These boundaries are intentional, including in firmware LTO builds.
    #[inline(never)]
    fn register(&mut self) -> Result<(), Error> {
        if self.state.left_system {
            return Err(Error::LeftSystem);
        }
        match self.state.reg_state {
            RegState::Idle => {}
            RegState::Registering => return Err(Error::RegistrationInProgress),
            RegState::Registered => return Err(Error::AlreadyRegistered),
            RegState::Detaching => return Err(Error::Detaching),
        }
        let mut bits = PackedBits::new();
        wire::location_update(&mut bits, LocationUpdateType::ItsiAttach, None, &[], false)?;
        self.state.attempts = 0;
        self.send_registration(&bits, LocationUpdateType::ItsiAttach, 6);
        Ok(())
    }

    #[inline(never)]
    fn link_change(&mut self, broken: bool) -> Result<(), Error> {
        if self.state.out_of_service != broken {
            self.state.out_of_service = broken;
            self.actions.push(Action::Service(self.service_status()));
        }
        Ok(())
    }

    #[inline(never)]
    fn deregister(&mut self) -> Result<(), Error> {
        if self.state.reg_state != RegState::Registered {
            return Err(Error::NotRegistered);
        }
        if self.state.out_of_service {
            return Ok(());
        }
        let sdu = wire::detach()?;
        self.actions.push(Action::Send {
            sdu,
            priority: 3,
            detach_receipt: true,
        });
        self.state.reg_state = RegState::Detaching;
        self.state.detach_countdown = DETACH_DRAIN_SLOTS;
        self.actions.push(Action::Service(ServiceStatus::OutOfService));
        Ok(())
    }

    #[inline(never)]
    fn activate(&mut self, cell: CellId, required: bool, wide: bool, force: bool) -> Result<(), Error> {
        cell.validate()?;
        let s = &self.state;
        if s.left_system || matches!(s.reg_state, RegState::Detaching | RegState::Registering) {
            self.state.current_cell = Some(cell);
            self.state.serving_la = cell.la;
            return Ok(());
        }
        let restricted = !wide;
        let lu = if s.reg_state == RegState::Registered {
            match s.registered_cell {
                Some(reg) if reg == cell => {
                    if s.temporary_registration && wide {
                        Some(LocationUpdateType::PeriodicLocationUpdating)
                    } else if s.temporary_registration || restricted || force {
                        Some(LocationUpdateType::RoamingLocationUpdating)
                    } else {
                        None
                    }
                }
                Some(reg) if reg.mcc != cell.mcc || reg.mnc != cell.mnc => Some(LocationUpdateType::MigratingLocationUpdating),
                _ if required || restricted || s.temporary_registration || force => Some(LocationUpdateType::RoamingLocationUpdating),
                _ => None,
            }
        } else if required || restricted || force {
            Some(LocationUpdateType::ItsiAttach)
        } else {
            None
        };
        let mut bits = PackedBits::new();
        if let Some(lu) = lu {
            wire::location_update(&mut bits, lu, None, &[], false)?;
        }
        self.state.current_cell = Some(cell);
        self.state.serving_la = cell.la;
        if let Some(lu) = lu {
            self.state.pending_cell = Some(cell);
            self.state.attempts = 0;
            self.send_registration(&bits, lu, lu.priority());
        }
        Ok(())
    }

    #[inline(never)]
    fn send_registration(&mut self, sdu: &PackedBits, lu: LocationUpdateType, priority: u8) {
        self.actions.push(Action::Send {
            sdu: sdu.clone(),
            priority,
            detach_receipt: false,
        });
        self.state.pending_lu_type = lu;
        self.state.reg_state = RegState::Registering;
        self.state.t351_countdown = T351_TIMEOUT_SLOTS;
        self.state.attempts += 1;
        self.actions.push(Action::Service(ServiceStatus::WaitingForRegistration));
    }

    #[inline(never)]
    fn begin_groups(&mut self, handle: u32, detach_all: bool, report_requested: bool, requests: &[GroupRequest]) -> Result<(), Error> {
        if self.state.reg_state != RegState::Registered {
            return Err(Error::NotRegistered);
        }
        if self.state.pending_group_op.is_some() {
            return Err(Error::GroupOperationPending);
        }
        if requests.is_empty() {
            return Err(Error::EmptyGroups);
        }
        let mut sdu = PackedBits::new();
        wire::groups(&mut sdu, detach_all, requests)?;
        self.actions.push(Action::Send {
            sdu,
            priority: 3,
            detach_receipt: false,
        });
        self.state.pending_group_op = Some(PendingGroupOp {
            detach_all,
            report_requested,
            handle,
            t353_countdown: T353_TIMEOUT_SLOTS,
        });
        Ok(())
    }

    #[inline(never)]
    fn receive(&mut self, bits: BitView<'_>) -> Result<(), Error> {
        let pdu = wire::decode(bits)?;
        self.downlink(&pdu)
    }

    fn downlink(&mut self, pdu: &wire::Downlink<'_>) -> Result<(), Error> {
        match pdu {
            wire::Downlink::Command { report, mni } => self.downlink_command(*report, *mni),
            wire::Downlink::Accept { temporary, groups } => self.downlink_accept(*temporary, groups.as_ref()),
            wire::Downlink::Reject { cause } => self.downlink_reject(*cause),
            wire::Downlink::GroupAck { groups } => self.downlink_group_ack(groups),
        }
    }

    #[inline(never)]
    fn downlink_command(&mut self, report: bool, mni: Option<u32>) -> Result<(), Error> {
        if mni.is_some_and(|mni| mni != (u32::from(self.config.mcc) << 14 | u32::from(self.config.mnc)))
            || self.state.left_system
            || self.state.reg_state == RegState::Detaching
            || (self.state.reg_state == RegState::Registering && self.state.t351_countdown > 0)
        {
            return Ok(());
        }
        let mut sdu = PackedBits::new();
        wire::location_update(
            &mut sdu,
            LocationUpdateType::DemandLocationUpdating,
            Some(self.config),
            self.state.attached_gssis.as_slice(),
            report,
        )?;
        self.state.attempts = 0;
        self.state.pending_cell = self.state.current_cell;
        self.send_registration(&sdu, LocationUpdateType::DemandLocationUpdating, if report { 3 } else { 6 });
        Ok(())
    }

    #[inline(never)]
    fn downlink_accept(&mut self, temporary: bool, groups: Option<&wire::GroupElements<'_>>) -> Result<(), Error> {
        if self.state.reg_state == RegState::Registered {
            return Ok(());
        }
        let mut attached = GroupAttachments::new();
        let mut new_groups = self.state.attached_gssis;
        if let Some(groups) = groups {
            groups.attachments(&mut attached)?;
            new_groups = GroupSet::new();
            for group in attached.iter() {
                new_groups.insert(group.gssi)?;
            }
        }
        let post_attach = !new_groups.is_empty() && self.state.pending_group_op.is_none();
        let mut sdu = PackedBits::new();
        if post_attach {
            wire::attach_set(&mut sdu, &new_groups)?;
        }
        self.state.temporary_registration = temporary;
        self.state.reg_state = RegState::Registered;
        if self.state.pending_cell.is_some() {
            self.state.registered_cell = self.state.pending_cell;
        }
        self.state.t351_countdown = 0;
        self.state.system_rejection_count = 0;
        self.state.attached_gssis = new_groups;
        self.queue_registration_identities(&new_groups);
        self.queue_registration_success(&attached);
        if post_attach {
            self.actions.push(Action::Send {
                sdu,
                priority: 3,
                detach_receipt: false,
            });
            self.state.pending_group_op = Some(PendingGroupOp {
                detach_all: false,
                report_requested: false,
                handle: 0,
                t353_countdown: T353_TIMEOUT_SLOTS,
            });
        }
        Ok(())
    }

    #[inline(never)]
    fn downlink_reject(&mut self, cause: u8) -> Result<(), Error> {
        if self.state.reg_state != RegState::Registering {
            return Ok(());
        }
        self.state.t351_countdown = 0;
        match cause {
            5 | 6 | 9 | 10 if self.state.attempts < MAX_REGISTRATION_ATTEMPTS => return Ok(()),
            1 | 2 | 12 => {
                self.state.system_rejection_count = self.state.system_rejection_count.saturating_add(1);
                if self.state.system_rejection_count >= N351_MAX_SYSTEM_REJECTIONS {
                    self.state.left_system = true;
                }
            }
            _ => {}
        }
        self.state.reg_state = RegState::Idle;
        self.actions.push(Action::RegistrationFailure { cause: Some(cause) });
        Ok(())
    }

    #[inline(never)]
    fn downlink_group_ack(&mut self, groups: &wire::GroupElements<'_>) -> Result<(), Error> {
        let Some(op) = self.state.pending_group_op else { return Ok(()) };
        let mut attached = GroupAttachments::new();
        let mut detached = GroupList::new();
        groups.collect(&mut attached, &mut detached)?;
        let mut next = self.state.attached_gssis;
        if op.detach_all {
            next = GroupSet::new();
            for group in attached.iter() {
                next.insert(group.gssi)?;
            }
        } else {
            for gssi in detached.iter() {
                next.remove(gssi);
            }
            for group in attached.iter() {
                if !detached.as_slice().contains(&group.gssi) {
                    next.insert(group.gssi)?;
                }
            }
        }
        self.state.attached_gssis = next;
        self.state.pending_group_op = None;
        self.queue_identities(&next, &detached);
        self.queue_group_confirm(op, &attached, &detached);
        Ok(())
    }

    // Keep distinct large enum constructors out of the procedures' live frames.
    #[inline(never)]
    fn queue_identities(&mut self, attached: &GroupSet, detached: &GroupList) {
        self.actions.push(Action::Identities {
            attached: *attached,
            detached: *detached,
        });
    }

    #[inline(never)]
    fn queue_registration_identities(&mut self, attached: &GroupSet) {
        self.actions.push(Action::Identities {
            attached: *attached,
            detached: GroupList::new(),
        });
    }

    #[inline(never)]
    fn queue_registration_success(&mut self, attached: &GroupAttachments) {
        self.actions.push(Action::RegistrationSuccess { attached: *attached });
    }

    #[inline(never)]
    fn queue_group_confirm(&mut self, op: PendingGroupOp, attached: &GroupAttachments, detached: &GroupList) {
        self.actions.push(Action::GroupConfirm {
            op,
            attached: *attached,
            detached: *detached,
        });
    }

    #[inline(never)]
    fn tick(&mut self, mut elapsed: u32, transfer: TransferState) -> Result<(), Error> {
        let mut available_slots = elapsed;
        let mut needed = 0;
        if self.state.reg_state == RegState::Detaching {
            let until_complete = if matches!(
                transfer,
                TransferState::Acknowledged | TransferState::Discarded | TransferState::Lost
            ) {
                1
            } else {
                self.state.detach_countdown.max(1)
            };
            if available_slots < until_complete {
                needed = 0;
                available_slots = 0;
            } else {
                needed += 2;
                available_slots -= until_complete;
            }
        }
        if self.state.pending_group_op.is_some_and(|op| available_slots > op.t353_countdown) {
            needed += 1;
        }
        if self.state.reg_state == RegState::Registering {
            let mut until_retry = self.state.t351_countdown + 1;
            let mut attempts = self.state.attempts;
            while available_slots >= until_retry && attempts < MAX_REGISTRATION_ATTEMPTS {
                needed += 2;
                available_slots -= until_retry;
                attempts += 1;
                until_retry = T351_TIMEOUT_SLOTS + 1;
            }
        }
        if needed > self.actions.free() {
            return Err(Error::ActionCapacity);
        }
        while elapsed > 0 {
            if self.state.reg_state != RegState::Detaching
                && self.state.reg_state != RegState::Registering
                && self.state.pending_group_op.is_none()
            {
                break;
            }
            self.tick_once(transfer)?;
            elapsed -= 1;
        }
        Ok(())
    }

    #[inline(never)]
    fn tick_once(&mut self, transfer: TransferState) -> Result<(), Error> {
        if self.state.reg_state == RegState::Detaching {
            let final_result = match transfer {
                TransferState::Acknowledged => Some(true),
                TransferState::Discarded | TransferState::Lost => Some(false),
                _ => None,
            };
            if final_result.is_none() && self.state.detach_countdown > 1 {
                self.state.detach_countdown -= 1;
                return Ok(());
            }
            self.state.detach_countdown = 0;
            self.state.reg_state = RegState::Idle;
            self.actions.push(Action::DetachReport {
                successful: final_result.unwrap_or(transfer == TransferState::Transmitted),
            });
            self.actions.push(Action::DetachComplete);
            return Ok(());
        }
        // The original adapter expires T351/T353 on the tick *after* the
        // countdown reaches zero. Preserve that boundary and its retry shape.
        let retry = self.state.reg_state == RegState::Registering
            && self.state.t351_countdown == 0
            && self.state.attempts < MAX_REGISTRATION_ATTEMPTS;
        let mut resend = PackedBits::new();
        if retry {
            wire::location_update(&mut resend, self.state.pending_lu_type, None, &[], false)?;
        }
        if let Some(op) = &mut self.state.pending_group_op {
            if op.t353_countdown > 0 {
                op.t353_countdown -= 1;
            } else {
                let op = *op;
                self.state.pending_group_op = None;
                self.actions.push(Action::GroupConfirm {
                    op,
                    attached: GroupAttachments::new(),
                    detached: GroupList::new(),
                });
            }
        }
        if self.state.reg_state == RegState::Registering {
            if self.state.t351_countdown > 0 {
                self.state.t351_countdown -= 1;
            } else if retry {
                self.send_registration(&resend, self.state.pending_lu_type, self.state.pending_lu_type.priority());
            } else {
                self.state.reg_state = RegState::Idle;
            }
        }
        Ok(())
    }
}
