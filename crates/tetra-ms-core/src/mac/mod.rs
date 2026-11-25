//! Caller-driven MS MAC policy and bounded state. Receipts are opaque caller
//! tokens: the core returns them; only the transport adapter reports delivery.
//! Tokens must be `Copy`: protocol state cannot own a destructor, and no token
//! cloning is performed. Application adapters use `u32` handles, never reporters.
//!
//! Use [`MsMac`] for fixed-width `u32` receipt tokens. Supply recovered downlink
//! time to [`Mac::tick`], decoded SYNC/SYSINFO to their adoption methods, and
//! LLC bits to [`Mac::submit_sdu`]. [`Mac::traffic`] and [`Mac::reserved`] return
//! at most one burst each per caller tick. A reserved burst's time is exact;
//! random-access retries are driven by the separate `random_access` component,
//! with [`Mac::begin_access`] and [`Mac::acknowledge_access`] owning transfer
//! admission/completion. Downlink wire parsing lives in [`downlink`]; the caller
//! routes its typed outcomes into the separate `reassembly` and LLC components.
//!
//! Storage bounds: one active access transfer, 16 queued transfers, six
//! reserved full slots, four packed speech frames, 16 stolen-signalling
//! blocks, 256 attached groups and all 256 possible marker byte values.
//! Queue overflow returns the displaced transfer/block (or an audio-drop flag);
//! callers must finish every displaced/rejected receipt. Group replacement is
//! atomic on overflow. Encoders return checked bit errors rather than partial
//! blocks. Extended/augmented allocations, supplementary MAC/U-SIGNAL, and
//! other broadcast types remain explicitly unsupported; encrypted resources
//! are identified and not delivered. No cryptographic or radio work is done.
pub mod downlink;
pub mod wire;

use crate::queue::Queue;
use crate::types::{BitError, BitStorage, BitView, FragmentBits, TdmaTime, TetraAddress};
pub use wire::{FragEndKind, MacBlock};

pub const MAX_UPLINK_QUEUE: usize = 16;
pub const UPLINK_AUDIO_MAX_FRAMES: usize = 4;
pub const MAX_ATTACHED_GROUPS: usize = 256;
pub const MAX_STOLEN_SIGNALLING: usize = 16;
pub const TCH_S_BITS: u16 = 274;
pub type SpeechFrame = BitStorage<35>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    SchHu,
    SchF,
    Stch,
    TchS,
}

#[derive(Debug)]
pub struct PendingUplink<R: Copy> {
    pub mac_block: MacBlock,
    pub logical_channel: Channel,
    pub scrambling_code: u32,
    pub tx_reporter: Option<R>,
    pub pdu_priority: Option<u8>,
    pub is_emergency: bool,
}

#[derive(Debug)]
pub struct UplinkFragment<R: Copy> {
    pub remainder: FragmentBits,
    pub scrambling_code: u32,
    pub end_kind: FragEndKind,
    pub tx_reporter: Option<R>,
}

#[derive(Debug)]
pub struct QueuedUplink<R: Copy> {
    pub pending: PendingUplink<R>,
    pub fragment: Option<UplinkFragment<R>>,
}

#[derive(Debug)]
pub struct StolenSignalling<R: Copy> {
    pub mac_block: MacBlock,
    pub scrambling_code: u32,
    pub tx_reporter: Option<R>,
}

/// Capacity is checked before mutation. Full queues explicitly return ownership
/// of the displaced item so the caller can finish its receipt.
pub struct BoundedQueue<T, const N: usize>(Queue<T, N>);

impl<T, const N: usize> Default for BoundedQueue<T, N> {
    fn default() -> Self {
        assert!(N != 0 && N <= u16::MAX as usize);
        Self(Queue::new())
    }
}

impl<T, const N: usize> BoundedQueue<T, N> {
    pub fn len(&self) -> usize {
        usize::from(self.0.len())
    }

    pub fn is_empty(&self) -> bool {
        self.0.len() == 0
    }

    pub fn push_drop_oldest(&mut self, value: T) -> Option<T> {
        assert!(N != 0 && N <= u16::MAX as usize);
        let dropped = if self.0.free() == 0 { self.0.pop() } else { None };
        self.0.push(value);
        dropped
    }

    pub fn pop_front(&mut self) -> Option<T> {
        self.0.pop()
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }
}

#[derive(Debug, Clone)]
pub struct IdentitySet {
    values: [u32; MAX_ATTACHED_GROUPS],
    len: u16,
}

impl Default for IdentitySet {
    fn default() -> Self {
        Self {
            values: [0; MAX_ATTACHED_GROUPS],
            len: 0,
        }
    }
}

impl IdentitySet {
    pub fn as_slice(&self) -> &[u32] {
        &self.values[..usize::from(self.len)]
    }

    /// Atomic replacement for an already bounded input list, without a second
    /// identity array on the stack. Larger raw lists use `replace`, which also
    /// accepts duplicate-heavy lists whose unique set still fits.
    pub fn replace_bounded(&mut self, values: &[u32]) -> Result<(), BitError> {
        if values.len() > MAX_ATTACHED_GROUPS {
            return Err(BitError::Capacity);
        }
        self.len = 0;
        for &value in values {
            if !self.contains(&value) {
                self.values[usize::from(self.len)] = value;
                self.len += 1;
            }
        }
        Ok(())
    }

    pub fn replace(&mut self, values: &[u32]) -> Result<(), BitError> {
        let mut next = Self::default();
        for &value in values {
            if next.contains(&value) {
                continue;
            }
            if usize::from(next.len) == MAX_ATTACHED_GROUPS {
                return Err(BitError::Capacity);
            }
            next.values[usize::from(next.len)] = value;
            next.len += 1;
        }
        *self = next;
        Ok(())
    }

    pub fn contains(&self, value: &u32) -> bool {
        self.values[..usize::from(self.len)].contains(value)
    }
}

#[derive(Debug)]
pub struct UsageMarkers([Option<u32>; 256]);

impl Default for UsageMarkers {
    fn default() -> Self {
        Self([None; 256])
    }
}

impl UsageMarkers {
    pub fn insert(&mut self, marker: u8, owner: u32) {
        self.0[usize::from(marker)] = Some(owner);
    }

    pub fn get(&self, marker: &u8) -> Option<&u32> {
        self.0[usize::from(*marker)].as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocationType {
    Additional,
    Replace,
}

#[derive(Debug, Clone, Copy)]
pub struct Allocation {
    pub carrier: u16,
    pub extended: bool,
    pub cell_change: bool,
    pub downlink: bool,
    pub kind: AllocationType,
    pub slots: [bool; 4],
    pub owner: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocationResult {
    Applied,
    CrossCell,
    CrossCarrier,
    UnknownCarrier,
    UplinkOnly,
    IndividualEngaged,
}

#[derive(Debug, Clone, Copy)]
pub enum GrantCapacity {
    FirstSubslot,
    SecondSubslot,
    Slots(u8),
}

#[derive(Debug, Clone, Copy)]
pub struct Grant {
    pub next_opportunity: bool,
    pub capacity: GrantCapacity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantError {
    Missing,
    Delay,
    Capacity,
    MissedSlot,
    Bits(BitError),
}

#[derive(Debug)]
pub struct ReservedTxPlan<R: Copy> {
    blocks: wire::FragmentBlocks,
    slots: [TdmaTime; wire::MAX_UL_FRAG_SLOTS],
    next: u8,
    pub scrambling_code: u32,
    pub tx_reporter: Option<R>,
}

pub fn reserved_slot_sequence(first: TdmaTime, total: u8) -> Result<[TdmaTime; wire::MAX_UL_FRAG_SLOTS], BitError> {
    if !first.is_valid() {
        return Err(BitError::InvalidValue);
    }
    if usize::from(total) > wire::MAX_UL_FRAG_SLOTS {
        return Err(BitError::Capacity);
    }
    let mut slots = [first; wire::MAX_UL_FRAG_SLOTS];
    let mut candidate = first;
    for slot in slots.iter_mut().take(usize::from(total)) {
        while candidate.is_mandatory_clch() {
            candidate = candidate.add_timeslots(4);
        }
        *slot = candidate;
        candidate = candidate.add_timeslots(4);
    }
    Ok(slots)
}

impl<R: Copy> ReservedTxPlan<R> {
    pub fn remaining_slots(&self) -> &[TdmaTime] {
        &self.slots[usize::from(self.next)..self.blocks.as_slice().len()]
    }

    pub fn new(remainder: BitView<'_>, total: u8, first: TdmaTime, scrambling_code: u32, tx_reporter: Option<R>) -> Result<Self, BitError> {
        let blocks = wire::FragmentBlocks::build(remainder, total)?;
        let slots = reserved_slot_sequence(first, total)?;
        Ok(Self {
            blocks,
            slots,
            next: 0,
            scrambling_code,
            tx_reporter,
        })
    }
}

pub struct Burst<R: Copy> {
    pub time: TdmaTime,
    pub block: MacBlock,
    pub channel: Channel,
    pub scrambling_code: u32,
    pub receipt: Option<R>,
}

pub enum ReservedAction<R: Copy> {
    Waiting,
    Missed { receipt: Option<R> },
    Transmit(Burst<R>),
}

pub enum FragmentAction<R: Copy> {
    Rejected {
        reason: GrantError,
        receipt: Option<R>,
    },
    /// A plan is installed but its first slot has not been emitted. The
    /// high-level `resource_event` polls it immediately; callers of the lower
    /// level `grant_fragment` must call `reserved` before advancing time.
    Reserved,
    Transmit(Burst<R>),
}

pub struct TrafficBurst<R: Copy> {
    pub time: TdmaTime,
    pub speech: SpeechFrame,
    pub scrambling_code: u32,
    pub stolen: Option<StolenSignalling<R>>,
}

/// Execute the reserved action first, then the optional traffic burst.
pub struct TickActions<R: Copy> {
    pub reserved: ReservedAction<R>,
    pub traffic: Option<TrafficBurst<R>>,
}

pub struct SubmitRequest<'a, R: Copy> {
    pub issi: u32,
    pub sdu: BitView<'a>,
    pub stealing: bool,
    pub priority: Option<u8>,
    pub emergency: bool,
    pub receipt: Option<R>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SubmitRoute {
    Access,
    FragmentedAccess,
    Stolen,
}

#[derive(Debug)]
pub struct SubmitResult<R: Copy> {
    pub route: SubmitRoute,
    pub discarded: Option<QueuedUplink<R>>,
    pub discarded_stolen: Option<StolenSignalling<R>>,
}

#[derive(Debug)]
pub struct SubmitError<R: Copy> {
    pub reason: BitError,
    pub receipt: Option<R>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Status {
    Idle,
    AccessPending,
    AwaitingFragmentGrant,
    Reserved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct Snapshot {
    pub status: Status,
    pub queued_uplinks: u8,
    pub queued_stolen: u8,
    pub queued_audio: u8,
    pub assigned_slot_bitmap: u8,
    pub tx_granted: u8,
}

pub type MsMac = Mac<u32>;
const _: () = assert!(!core::mem::needs_drop::<MsMac>());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DropReason {
    NoAddress,
    Encrypted,
    ForeignAddress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ResourceRoute {
    Drop(DropReason),
    Empty,
    Deliver,
    FragmentStart,
}

#[derive(Debug, Clone, Copy)]
pub struct ResourceOutcome {
    pub header: downlink::Resource,
    pub payload: downlink::Payload,
    pub route: ResourceRoute,
    pub allocation: Option<AllocationResult>,
    pub second_half_stolen: bool,
    pub acknowledges_own: bool,
}

#[derive(Debug)]
pub struct AccessStartError<R: Copy> {
    pub reason: crate::random_access::RaFailure,
    pub first_receipt: Option<R>,
    pub fragment_receipt: Option<R>,
}

pub enum AccessCompletion<R: Copy> {
    None,
    Complete { receipt: Option<R> },
    Fragment(FragmentAction<R>),
}

/// Execute access completion before routing the resource SDU/stealing signal.
pub struct ResourceEvent<R: Copy> {
    pub outcome: ResourceOutcome,
    pub access: AccessCompletion<R>,
}

/// All storage is bounded and owned by the caller. No entropy, clock, radio,
/// allocation or receipt reporting is performed here.
///
/// Receipt owners with destructors cannot enter protocol storage:
/// ```compile_fail
/// use tetra_ms_core::mac::Mac;
/// struct ApplicationReceipt;
/// impl Drop for ApplicationReceipt { fn drop(&mut self) {} }
/// let _ = Mac::<ApplicationReceipt>::default();
/// ```
pub struct Mac<R: Copy> {
    pub dltime: TdmaTime,
    pub last_recovered_dltime: Option<TdmaTime>,
    pub mcc: Option<u16>,
    pub mnc: Option<u16>,
    pub cc: Option<u8>,
    pub scrambling_code: Option<u32>,
    pub pending_uplink: Option<PendingUplink<R>>,
    pub pending_fragment: Option<UplinkFragment<R>>,
    pub uplink_queue: BoundedQueue<QueuedUplink<R>, MAX_UPLINK_QUEUE>,
    pub reserved_tx: Option<ReservedTxPlan<R>>,
    pub valid_individual_ssi: Option<u32>,
    pub valid_group_ssis: IdentitySet,
    pub serving_carrier_num: Option<u16>,
    pub assigned_traffic_slots: [bool; 4],
    pub traffic_channel_owner: Option<u32>,
    pub active_traffic_usage_markers: UsageMarkers,
    pub current_traffic_usage_marker: Option<u8>,
    pub uplink_tx_granted: bool,
    pub uplink_audio: BoundedQueue<SpeechFrame, UPLINK_AUDIO_MAX_FRAMES>,
    pub pending_stolen_signalling: BoundedQueue<StolenSignalling<R>, MAX_STOLEN_SIGNALLING>,
}

impl<R: Copy> Default for Mac<R> {
    fn default() -> Self {
        Self {
            dltime: TdmaTime::default(),
            last_recovered_dltime: None,
            mcc: None,
            mnc: None,
            cc: None,
            scrambling_code: None,
            pending_uplink: None,
            pending_fragment: None,
            uplink_queue: BoundedQueue::default(),
            reserved_tx: None,
            valid_individual_ssi: None,
            valid_group_ssis: IdentitySet::default(),
            serving_carrier_num: None,
            assigned_traffic_slots: [false; 4],
            traffic_channel_owner: None,
            active_traffic_usage_markers: UsageMarkers::default(),
            current_traffic_usage_marker: None,
            uplink_tx_granted: false,
            uplink_audio: BoundedQueue::default(),
            pending_stolen_signalling: BoundedQueue::default(),
        }
    }
}

impl<R: Copy> Mac<R> {
    /// The next candidate without promoting or mutating FIFO state. Used when
    /// querying entropy before committing a random-access input.
    pub fn access_candidate(&self, random_access_active: bool) -> Option<&PendingUplink<R>> {
        self.pending_uplink.as_ref().or_else(|| {
            if self.uplink_in_flight(random_access_active) {
                None
            } else {
                self.uplink_queue.0.iter().next().map(|transfer| &transfer.pending)
            }
        })
    }

    pub fn tick(&mut self, recovered: TdmaTime) -> Result<TickActions<R>, BitError> {
        self.advance(recovered)?;
        Ok(TickActions {
            reserved: self.reserved(),
            traffic: self.traffic(),
        })
    }

    pub fn resource_event(
        &mut self,
        random_access: &mut crate::random_access::MsRandomAccess,
        bits: BitView<'_>,
        own_issi: u32,
        filtered: bool,
    ) -> Result<ResourceEvent<R>, downlink::DecodeError> {
        let outcome = self.resource(bits, own_issi, filtered)?;
        let access = match self.acknowledge_access(random_access, &outcome) {
            AccessCompletion::Fragment(FragmentAction::Reserved) => match self.reserved() {
                ReservedAction::Transmit(burst) => AccessCompletion::Fragment(FragmentAction::Transmit(burst)),
                ReservedAction::Waiting => AccessCompletion::Fragment(FragmentAction::Reserved),
                ReservedAction::Missed { receipt } => AccessCompletion::Fragment(FragmentAction::Rejected {
                    reason: GrantError::MissedSlot,
                    receipt,
                }),
            },
            other => other,
        };
        Ok(ResourceEvent { outcome, access })
    }

    pub fn aach(&mut self, bits: BitView<'_>) -> Result<downlink::Aach, downlink::DecodeError> {
        self.current_traffic_usage_marker = None;
        let assignment = downlink::aach(bits, self.dltime.f == 18)?;
        self.current_traffic_usage_marker = assignment.traffic_marker;
        Ok(assignment)
    }

    pub fn snapshot(&self) -> Snapshot {
        let assigned_slot_bitmap = self
            .assigned_traffic_slots
            .iter()
            .fold(0, |bitmap, &slot| (bitmap << 1) | u8::from(slot));
        Snapshot {
            status: self.status(),
            queued_uplinks: self.uplink_queue.len() as u8,
            queued_stolen: self.pending_stolen_signalling.len() as u8,
            queued_audio: self.uplink_audio.len() as u8,
            assigned_slot_bitmap,
            tx_granted: u8::from(self.uplink_tx_granted),
        }
    }

    pub fn adopt_sync(&mut self, sync: downlink::Sync) {
        self.cc = Some(sync.colour_code);
        self.dltime = sync.time;
    }

    /// The historical MAC informs LMAC about a hyperframe correction, but does
    /// not re-seed its own downlink clock until SYNC.
    pub fn adopt_sysinfo(&mut self, info: &downlink::Sysinfo) -> Option<TdmaTime> {
        self.serving_carrier_num = Some(info.main_carrier);
        info.hyperframe
            .filter(|&h| h != self.dltime.h)
            .map(|h| TdmaTime { h, ..self.dltime })
    }

    pub fn configure_identities(
        &mut self,
        mcc: u16,
        mnc: u16,
        individual: Option<u32>,
        groups: Option<&[u32]>,
    ) -> Result<Option<u32>, BitError> {
        if let Some(groups) = groups {
            self.valid_group_ssis.replace(groups)?;
        }
        if let Some(individual) = individual {
            self.valid_individual_ssi = Some(individual);
        }
        self.mcc = Some(mcc);
        self.mnc = Some(mnc);
        Ok(self.update_scrambling())
    }

    pub fn begin_access(
        &mut self,
        random_access: &mut crate::random_access::MsRandomAccess,
        params: Option<&crate::random_access::RandomAccessParams>,
    ) -> Result<bool, AccessStartError<R>> {
        self.promote(random_access.is_active());
        let Some(params) = params else { return Ok(false) };
        if random_access.is_active() {
            return Ok(true);
        }
        let Some(pending) = self.pending_uplink.as_ref() else {
            return Ok(false);
        };
        match random_access.initiate(
            self.dltime,
            crate::random_access::AccessCode::A,
            params,
            pending.pdu_priority.unwrap_or(params.min_pdu_prio),
            pending.is_emergency,
        ) {
            Ok(()) => Ok(true),
            Err(reason) => {
                let (first_receipt, fragment_receipt) = self.abandon_access();
                Err(AccessStartError {
                    reason,
                    first_receipt,
                    fragment_receipt,
                })
            }
        }
    }

    pub fn acknowledge_access(
        &mut self,
        random_access: &mut crate::random_access::MsRandomAccess,
        resource: &ResourceOutcome,
    ) -> AccessCompletion<R> {
        if random_access.on_mac_resource(resource.acknowledges_own, resource.header.random_access_flag)
            != Some(crate::random_access::RaAction::Succeeded)
        {
            return AccessCompletion::None;
        }
        let pending = self.pending_uplink.take();
        if self.pending_fragment.is_some() {
            match self.grant_fragment(resource.header.grant) {
                Some(action) => AccessCompletion::Fragment(action),
                None => AccessCompletion::None,
            }
        } else {
            AccessCompletion::Complete {
                receipt: pending.and_then(|pending| pending.tx_reporter),
            }
        }
    }

    pub fn resource(&mut self, bits: BitView<'_>, own_issi: u32, filtered: bool) -> Result<ResourceOutcome, downlink::DecodeError> {
        let header = downlink::resource(bits)?;
        let payload = header.payload(bits)?;
        let acknowledges_own = header.addr.is_some_and(|address| address.ssi == own_issi);
        let mut allocation = None;
        let route = match header.addr {
            None => ResourceRoute::Drop(DropReason::NoAddress),
            Some(_) if header.encryption_mode != 0 => ResourceRoute::Drop(DropReason::Encrypted),
            Some(address) if !self.accepts(filtered, address) => ResourceRoute::Drop(DropReason::ForeignAddress),
            Some(address) => {
                if let Some(assignment) = header.allocation {
                    allocation = Some(self.allocate(own_issi, assignment));
                }
                if let Some(marker) = header.usage_marker {
                    self.active_traffic_usage_markers.insert(marker, address.ssi);
                }
                if header.length_ind == 63 {
                    ResourceRoute::FragmentStart
                } else if header.length_ind == 0 || payload.end == header.header_bits {
                    ResourceRoute::Empty
                } else {
                    ResourceRoute::Deliver
                }
            }
        };
        Ok(ResourceOutcome {
            header,
            payload,
            route,
            allocation,
            acknowledges_own,
            second_half_stolen: header.length_ind == 62 && matches!(route, ResourceRoute::Deliver | ResourceRoute::Empty),
        })
    }

    pub fn status(&self) -> Status {
        if self.reserved_tx.is_some() {
            Status::Reserved
        } else if self.pending_fragment.is_some() && self.pending_uplink.is_none() {
            Status::AwaitingFragmentGrant
        } else if self.pending_uplink.is_some() {
            Status::AccessPending
        } else {
            Status::Idle
        }
    }

    pub fn submit_sdu(&mut self, request: SubmitRequest<'_, R>, random_access_active: bool) -> Result<SubmitResult<R>, SubmitError<R>> {
        let Some(scrambling_code) = self.scrambling_code else {
            return Err(SubmitError {
                reason: BitError::InvalidValue,
                receipt: request.receipt,
            });
        };
        if request.stealing
            && self.has_assigned_traffic_slot()
            && let Ok(mac_block) = wire::stolen_data(request.issi, request.sdu)
        {
            let discarded_stolen = self.pending_stolen_signalling.push_drop_oldest(StolenSignalling {
                mac_block,
                scrambling_code,
                tx_reporter: request.receipt,
            });
            return Ok(SubmitResult {
                route: SubmitRoute::Stolen,
                discarded: None,
                discarded_stolen,
            });
        }
        let (mac_block, fragment, route) = match wire::access(request.issi, request.sdu) {
            Ok(block) => (block, None, SubmitRoute::Access),
            Err(_) => match wire::access_fragment(request.issi, request.sdu) {
                Ok((block, remainder, end_kind)) => (block, Some((remainder, end_kind)), SubmitRoute::FragmentedAccess),
                Err(reason) => {
                    return Err(SubmitError {
                        reason,
                        receipt: request.receipt,
                    });
                }
            },
        };
        let (tx_reporter, fragment) = match fragment {
            None => (request.receipt, None),
            Some((remainder, end_kind)) => (
                None,
                Some(UplinkFragment {
                    remainder,
                    end_kind,
                    scrambling_code,
                    tx_reporter: request.receipt,
                }),
            ),
        };
        let discarded = self.submit(
            QueuedUplink {
                pending: PendingUplink {
                    mac_block,
                    logical_channel: Channel::SchHu,
                    scrambling_code,
                    tx_reporter,
                    pdu_priority: request.priority,
                    is_emergency: request.emergency,
                },
                fragment,
            },
            random_access_active,
        );
        Ok(SubmitResult {
            route,
            discarded,
            discarded_stolen: None,
        })
    }

    /// Cancel both halves together, including when a priority gate refuses to
    /// start random access. The caller must report every returned receipt.
    pub fn abandon_access(&mut self) -> (Option<R>, Option<R>) {
        let first = self.pending_uplink.take().and_then(|pending| pending.tx_reporter);
        let tail = self.pending_fragment.take().and_then(|fragment| fragment.tx_reporter);
        (first, tail)
    }

    pub fn advance(&mut self, recovered: TdmaTime) -> Result<TdmaTime, BitError> {
        if !recovered.is_valid() {
            return Err(BitError::InvalidValue);
        }
        let delta = self.last_recovered_dltime.map_or(1, |prev| recovered.diff(prev));
        self.dltime = self.dltime.add_timeslots(delta);
        self.last_recovered_dltime = Some(recovered);
        self.current_traffic_usage_marker = None;
        Ok(self.dltime)
    }

    pub fn update_scrambling(&mut self) -> Option<u32> {
        let (mcc, mnc, cc) = (self.mcc?, self.mnc?, self.cc?);
        let code = ((u32::from(cc) | (u32::from(mnc) << 6) | (u32::from(mcc) << 20)) << 2) | 3;
        self.scrambling_code = Some(code);
        Some(code)
    }

    pub fn accepts(&self, filtered: bool, address: TetraAddress) -> bool {
        !filtered
            || address.ssi == 0xff_ffff
            || self.valid_individual_ssi == Some(address.ssi)
            || self.valid_group_ssis.contains(&address.ssi)
    }

    pub fn allocate(&mut self, own: u32, allocation: Allocation) -> AllocationResult {
        if allocation.extended || allocation.cell_change {
            return AllocationResult::CrossCell;
        }
        match self.serving_carrier_num {
            None => return AllocationResult::UnknownCarrier,
            Some(carrier) if carrier != allocation.carrier => return AllocationResult::CrossCarrier,
            _ => {}
        }
        if !allocation.downlink {
            return AllocationResult::UplinkOnly;
        }
        if self.traffic_channel_owner == Some(own) && allocation.owner != own {
            return AllocationResult::IndividualEngaged;
        }
        match allocation.kind {
            AllocationType::Additional => {
                for (assigned, additional) in self.assigned_traffic_slots.iter_mut().zip(allocation.slots) {
                    *assigned |= additional;
                }
            }
            AllocationType::Replace => self.assigned_traffic_slots = allocation.slots,
        }
        self.traffic_channel_owner = Some(allocation.owner);
        AllocationResult::Applied
    }

    pub fn has_assigned_traffic_slot(&self) -> bool {
        self.assigned_traffic_slots.iter().any(|&assigned| assigned)
    }

    pub fn configure_uplane(&mut self, switch: bool, grant: bool) {
        self.uplink_tx_granted = switch && grant;
        if !self.uplink_tx_granted {
            self.uplink_audio.clear();
        }
        if !switch {
            self.traffic_channel_owner = None;
        }
    }

    pub fn queue_audio(&mut self, frame: BitView<'_>) -> Result<bool, BitError> {
        let frame = SpeechFrame::from_view(frame.window(0, frame.len().min(TCH_S_BITS))?)?;
        Ok(self.uplink_audio.push_drop_oldest(frame).is_some())
    }

    pub fn traffic(&mut self) -> Option<TrafficBurst<R>> {
        let paired = self.dltime;
        if paired.f == 18
            || !(1..=4).contains(&paired.t)
            || !self.assigned_traffic_slots[usize::from(paired.t - 1)]
            || (!self.uplink_tx_granted && self.pending_stolen_signalling.is_empty())
        {
            return None;
        }
        let scrambling_code = self.scrambling_code?;
        // Historical behavior consumes one buffered frame even for a
        // listening-only stolen burst; it never sends that frame without grant.
        let speech = match self.uplink_audio.pop_front() {
            Some(frame) if self.uplink_tx_granted && !frame.is_empty() => frame,
            _ => SpeechFrame::from_view(BitView::new(&[0; 35], TCH_S_BITS).ok()?).ok()?,
        };
        Some(TrafficBurst {
            time: paired.add_timeslots(2),
            speech,
            scrambling_code,
            stolen: self.pending_stolen_signalling.pop_front(),
        })
    }

    pub fn downlink_traffic_owner(&self) -> Option<(u8, u32)> {
        let marker = self.current_traffic_usage_marker?;
        Some((marker, *self.active_traffic_usage_markers.get(&marker)?))
    }

    pub fn uplink_in_flight(&self, random_access_active: bool) -> bool {
        self.pending_uplink.is_some() || self.pending_fragment.is_some() || random_access_active || self.reserved_tx.is_some()
    }

    pub fn submit(&mut self, transfer: QueuedUplink<R>, random_access_active: bool) -> Option<QueuedUplink<R>> {
        if self.uplink_in_flight(random_access_active) {
            self.uplink_queue.push_drop_oldest(transfer)
        } else {
            self.pending_uplink = Some(transfer.pending);
            self.pending_fragment = transfer.fragment;
            None
        }
    }

    pub fn promote(&mut self, random_access_active: bool) {
        if !self.uplink_in_flight(random_access_active)
            && let Some(next) = self.uplink_queue.pop_front()
        {
            self.pending_uplink = Some(next.pending);
            self.pending_fragment = next.fragment;
        }
    }

    pub fn grant_fragment(&mut self, grant: Option<Grant>) -> Option<FragmentAction<R>> {
        let fragment = self.pending_fragment.take()?;
        let result = self.prepare_fragment(&fragment, grant);
        Some(match result {
            Err(reason) => FragmentAction::Rejected {
                reason,
                receipt: fragment.tx_reporter,
            },
            Ok(Some((block, channel))) => FragmentAction::Transmit(Burst {
                time: self.dltime.add_timeslots(2),
                block,
                channel,
                scrambling_code: fragment.scrambling_code,
                receipt: fragment.tx_reporter,
            }),
            Ok(None) => {
                if let Some(plan) = self.reserved_tx.as_mut() {
                    plan.tx_reporter = fragment.tx_reporter;
                }
                FragmentAction::Reserved
            }
        })
    }

    fn prepare_fragment(&mut self, fragment: &UplinkFragment<R>, grant: Option<Grant>) -> Result<Option<(MacBlock, Channel)>, GrantError> {
        let grant = grant.ok_or(GrantError::Missing)?;
        if !grant.next_opportunity {
            return Err(GrantError::Delay);
        }
        let bits = fragment.remainder.view();
        match (fragment.end_kind, grant.capacity) {
            (FragEndKind::MacEndHu, GrantCapacity::FirstSubslot) => {
                Ok(Some((wire::end_hu(bits).map_err(GrantError::Bits)?, Channel::SchHu)))
            }
            (FragEndKind::MacEndUl, GrantCapacity::Slots(1)) => Ok(Some((wire::end_ul(bits).map_err(GrantError::Bits)?, Channel::SchF))),
            (FragEndKind::MacFragUl { total_slots }, GrantCapacity::Slots(granted)) if granted >= total_slots => {
                self.reserved_tx = Some(
                    ReservedTxPlan::new(bits, total_slots, self.dltime.add_timeslots(2), fragment.scrambling_code, None)
                        .map_err(GrantError::Bits)?,
                );
                Ok(None)
            }
            _ => Err(GrantError::Capacity),
        }
    }

    pub fn reserved(&mut self) -> ReservedAction<R> {
        let Some(mut plan) = self.reserved_tx.take() else {
            return ReservedAction::Waiting;
        };
        let time = plan.slots[usize::from(plan.next)];
        let delta = self.dltime.add_timeslots(2).diff(time);
        if delta < 0 {
            self.reserved_tx = Some(plan);
            return ReservedAction::Waiting;
        }
        if delta > 0 {
            return ReservedAction::Missed { receipt: plan.tx_reporter };
        }
        let block = plan.blocks.as_slice()[usize::from(plan.next)].clone();
        plan.next += 1;
        let complete = usize::from(plan.next) == plan.blocks.as_slice().len();
        let receipt = if complete { plan.tx_reporter.take() } else { None };
        let burst = Burst {
            time,
            block,
            channel: Channel::SchF,
            scrambling_code: plan.scrambling_code,
            receipt,
        };
        if !complete {
            self.reserved_tx = Some(plan);
        }
        ReservedAction::Transmit(burst)
    }
}

impl Mac<u32> {
    /// Initialize directly in caller-owned storage without a full MAC temporary.
    /// Integer receipts own no resources; reset has the same in-place property.
    pub fn init_in_place(out: &mut core::mem::MaybeUninit<Self>) -> &mut Self {
        // SAFETY: MaybeUninit supplies sufficient aligned exclusive storage.
        unsafe {
            Self::initialize(out.as_mut_ptr());
            out.assume_init_mut()
        }
    }

    pub fn reset(&mut self) {
        // SAFETY: All stored receipt values are integers, and no field owns a
        // resource requiring Drop. Reinitialize every field in place.
        unsafe { Self::initialize(self) }
    }

    #[inline(never)]
    unsafe fn initialize(out: *mut Self) {
        // SAFETY: The caller guarantees aligned writable storage for all fields.
        // No reference to the outer MAC is formed until initialization completes.
        unsafe {
            core::ptr::addr_of_mut!((*out).dltime).write(TdmaTime::default());
            core::ptr::addr_of_mut!((*out).last_recovered_dltime).write(None);
            core::ptr::addr_of_mut!((*out).mcc).write(None);
            core::ptr::addr_of_mut!((*out).mnc).write(None);
            core::ptr::addr_of_mut!((*out).cc).write(None);
            core::ptr::addr_of_mut!((*out).scrambling_code).write(None);
            core::ptr::addr_of_mut!((*out).pending_uplink).write(None);
            core::ptr::addr_of_mut!((*out).pending_fragment).write(None);
            Queue::init_at(core::ptr::addr_of_mut!((*out).uplink_queue.0));
            core::ptr::addr_of_mut!((*out).reserved_tx).write(None);
            core::ptr::addr_of_mut!((*out).valid_individual_ssi).write(None);
            let groups = core::ptr::addr_of_mut!((*out).valid_group_ssis.values).cast::<u32>();
            for i in 0..MAX_ATTACHED_GROUPS {
                groups.add(i).write(0);
            }
            core::ptr::addr_of_mut!((*out).valid_group_ssis.len).write(0);
            core::ptr::addr_of_mut!((*out).serving_carrier_num).write(None);
            core::ptr::addr_of_mut!((*out).assigned_traffic_slots).write([false; 4]);
            core::ptr::addr_of_mut!((*out).traffic_channel_owner).write(None);
            let markers = core::ptr::addr_of_mut!((*out).active_traffic_usage_markers.0).cast::<Option<u32>>();
            for i in 0..256 {
                markers.add(i).write(None);
            }
            core::ptr::addr_of_mut!((*out).current_traffic_usage_marker).write(None);
            core::ptr::addr_of_mut!((*out).uplink_tx_granted).write(false);
            Queue::init_at(core::ptr::addr_of_mut!((*out).uplink_audio.0));
            Queue::init_at(core::ptr::addr_of_mut!((*out).pending_stolen_signalling.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SsiType;

    fn bits(len: u16) -> FragmentBits {
        FragmentBits::from_view(BitView::new(&[0xa5; 512], len).unwrap()).unwrap()
    }

    fn request(sdu: &FragmentBits, receipt: u32) -> SubmitRequest<'_, u32> {
        SubmitRequest {
            issi: 1_000_001,
            sdu: sdu.view(),
            stealing: false,
            priority: Some(3),
            emergency: false,
            receipt: Some(receipt),
        }
    }

    #[test]
    fn receipt_copy_never_invokes_a_custom_clone_callback() {
        #[derive(Debug, Copy)]
        struct Handle(u32);
        #[allow(clippy::non_canonical_clone_impl)]
        impl Clone for Handle {
            fn clone(&self) -> Self {
                panic!("receipt cloning must stay outside protocol operations")
            }
        }
        let mut mac = Mac::<Handle> {
            scrambling_code: Some(123),
            ..Mac::default()
        };
        let sdu = bits(456);
        mac.submit_sdu(
            SubmitRequest {
                issi: 1_000_001,
                sdu: sdu.view(),
                stealing: false,
                priority: None,
                emergency: false,
                receipt: Some(Handle(7)),
            },
            false,
        )
        .unwrap();
        mac.pending_uplink = None;
        assert!(matches!(
            mac.grant_fragment(Some(Grant {
                next_opportunity: true,
                capacity: GrantCapacity::Slots(2),
            })),
            Some(FragmentAction::Reserved)
        ));
        assert!(matches!(mac.reserved(), ReservedAction::Transmit(Burst { receipt: None, .. })));
        mac.dltime = mac.dltime.add_timeslots(4);
        let ReservedAction::Transmit(last) = mac.reserved() else {
            panic!("last fragment due")
        };
        assert_eq!(last.receipt.unwrap().0, 7);
    }

    #[test]
    fn transfer_queue_overflow_returns_receipt_and_keeps_active() {
        let mut mac = Mac::<u32> {
            scrambling_code: Some(123),
            ..Mac::default()
        };
        let sdu = bits(20);
        assert!(mac.submit_sdu(request(&sdu, 0), false).is_ok());
        for token in 1..=16 {
            assert!(mac.submit_sdu(request(&sdu, token), false).unwrap().discarded.is_none());
        }
        let displaced = mac.submit_sdu(request(&sdu, 17), false).unwrap().discarded.unwrap();
        assert_eq!(displaced.pending.tx_reporter, Some(1));
        assert_eq!(mac.pending_uplink.as_ref().unwrap().tx_reporter, Some(0));
        assert_eq!(mac.abandon_access(), (Some(0), None));
        mac.promote(false);
        assert_eq!(mac.pending_uplink.as_ref().unwrap().tx_reporter, Some(2));
    }

    #[test]
    fn fragmented_receipt_completes_only_on_final_reserved_slot() {
        let mut mac = Mac::<u32> {
            scrambling_code: Some(123),
            ..Mac::default()
        };
        let sdu = bits(456);
        assert_eq!(
            mac.submit_sdu(request(&sdu, 7), false).unwrap().route,
            SubmitRoute::FragmentedAccess
        );
        assert_eq!(mac.pending_uplink.as_ref().unwrap().tx_reporter, None);
        mac.pending_uplink = None;
        assert!(matches!(
            mac.grant_fragment(Some(Grant {
                next_opportunity: true,
                capacity: GrantCapacity::Slots(2),
            })),
            Some(FragmentAction::Reserved)
        ));
        let ReservedAction::Transmit(first) = mac.reserved() else {
            panic!("first block due")
        };
        assert_eq!(first.receipt, None);
        assert_eq!(first.block.view().read(0, 4), Ok(0b0100));
        mac.dltime = mac.dltime.add_timeslots(4);
        let ReservedAction::Transmit(last) = mac.reserved() else {
            panic!("last block due")
        };
        assert_eq!(last.receipt, Some(7));
        assert_eq!(last.block.view().read(0, 3), Ok(0b011));
        assert!(mac.reserved_tx.is_none());
    }

    #[test]
    fn missing_mismatched_and_late_grants_discard_tail_receipt() {
        for grant in [
            None,
            Some(Grant {
                next_opportunity: false,
                capacity: GrantCapacity::FirstSubslot,
            }),
            Some(Grant {
                next_opportunity: true,
                capacity: GrantCapacity::SecondSubslot,
            }),
            Some(Grant {
                next_opportunity: true,
                capacity: GrantCapacity::Slots(1),
            }),
        ] {
            let mut mac = Mac::<u32> {
                scrambling_code: Some(123),
                ..Mac::default()
            };
            assert!(mac.submit_sdu(request(&bits(100), 7), false).is_ok());
            assert!(matches!(
                mac.grant_fragment(grant),
                Some(FragmentAction::Rejected { receipt: Some(7), .. })
            ));
        }
        let mut mac = Mac::<u32> {
            scrambling_code: Some(123),
            ..Mac::default()
        };
        assert!(mac.submit_sdu(request(&bits(456), 7), false).is_ok());
        assert!(matches!(
            mac.grant_fragment(Some(Grant {
                next_opportunity: true,
                capacity: GrantCapacity::Slots(2),
            })),
            Some(FragmentAction::Reserved)
        ));
        mac.dltime = mac.dltime.add_timeslots(1);
        assert!(matches!(mac.reserved(), ReservedAction::Missed { receipt: Some(7) }));
        assert!(mac.reserved_tx.is_none());
    }

    #[test]
    fn traffic_uses_paired_slot_and_flushes_audio_on_degrant() {
        let mut mac = Mac::<u32> {
            scrambling_code: Some(123),
            uplink_tx_granted: true,
            assigned_traffic_slots: [false, false, true, false],
            dltime: TdmaTime { t: 3, f: 17, m: 1, h: 0 },
            ..Mac::default()
        };
        assert!(mac.queue_audio(bits(274).view()).is_ok());
        let burst = mac.traffic().unwrap();
        assert_eq!(burst.time.t, 1);
        assert_eq!(burst.time.f, 18);
        assert_eq!(burst.speech.view().read(0, 8), Ok(0xa5));
        mac.dltime.f = 18;
        assert!(mac.traffic().is_none());
        mac.queue_audio(bits(274).view()).unwrap();
        mac.configure_uplane(true, false);
        assert!(mac.uplink_audio.is_empty());
        assert!(!mac.uplink_tx_granted);
    }

    #[test]
    fn listening_steal_is_bounded_and_reports_displacement() {
        let mut mac = Mac::<u32> {
            scrambling_code: Some(123),
            assigned_traffic_slots: [true, false, false, false],
            ..Mac::default()
        };
        let sdu = bits(20);
        for token in 0..=16 {
            let mut request = request(&sdu, token);
            request.stealing = true;
            let result = mac.submit_sdu(request, false).unwrap();
            assert_eq!(result.route, SubmitRoute::Stolen);
            if token == 16 {
                assert_eq!(result.discarded_stolen.unwrap().tx_reporter, Some(0));
            } else {
                assert!(result.discarded_stolen.is_none());
            }
        }
        let burst = mac.traffic().unwrap();
        assert_eq!(burst.stolen.unwrap().tx_reporter, Some(1));
        assert_eq!(burst.speech.view().read(0, 32), Ok(0));
        assert!(mac.pending_uplink.is_none());
    }

    #[test]
    fn filter_replacement_is_atomic_and_individual_allocation_wins() {
        let mut mac = Mac::<u32> {
            valid_individual_ssi: Some(1),
            serving_carrier_num: Some(1593),
            ..Mac::default()
        };
        mac.valid_group_ssis.replace(&[2, 2, 3]).unwrap();
        let too_many = core::array::from_fn::<_, { MAX_ATTACHED_GROUPS + 1 }, _>(|i| i as u32 + 100);
        assert_eq!(mac.valid_group_ssis.replace(&too_many), Err(BitError::Capacity));
        assert!(mac.accepts(
            true,
            TetraAddress {
                ssi: 2,
                ssi_type: SsiType::Gssi
            }
        ));
        assert!(!mac.accepts(
            true,
            TetraAddress {
                ssi: 100,
                ssi_type: SsiType::Ssi
            }
        ));
        let mut allocation = Allocation {
            carrier: 1593,
            extended: false,
            cell_change: false,
            downlink: true,
            kind: AllocationType::Replace,
            slots: [false, true, false, false],
            owner: 1,
        };
        assert_eq!(mac.allocate(1, allocation), AllocationResult::Applied);
        allocation.owner = 2;
        allocation.slots = [false, false, true, false];
        assert_eq!(mac.allocate(1, allocation), AllocationResult::IndividualEngaged);
        assert_eq!(mac.assigned_traffic_slots, [false, true, false, false]);
        mac.configure_uplane(false, false);
        assert_eq!(mac.allocate(1, allocation), AllocationResult::Applied);
    }

    #[test]
    fn ack_only_resource_returns_receipt_without_an_sdu() {
        let mut mac = Mac::<u32> {
            scrambling_code: Some(123),
            ..Mac::default()
        };
        let mut random_access = crate::random_access::MsRandomAccess::new();
        let params = crate::random_access::RandomAccessParams {
            imm: 15,
            wt: 6,
            nu: 4,
            fl_factor: false,
            ts_ptr: 0,
            min_pdu_prio: 0,
            subscriber_class: None,
            gssi: None,
        };
        mac.submit_sdu(request(&bits(20), 7), false).unwrap();
        assert!(mac.begin_access(&mut random_access, Some(&params)).unwrap());
        let mut resource = MacBlock::new();
        resource.push(6, 0).unwrap();
        resource.push(1, 1).unwrap();
        resource.push(6, 2).unwrap(); // under-declared octet length
        resource.push(3, 1).unwrap();
        resource.push(24, 1_000_001).unwrap();
        resource.push(3, 0).unwrap();
        let event = mac.resource_event(&mut random_access, resource.view(), 1_000_001, false).unwrap();
        assert_eq!(event.outcome.route, ResourceRoute::Empty);
        assert_eq!(event.outcome.payload.end, 43);
        assert_eq!(event.outcome.payload.next, 16);
        assert!(matches!(event.access, AccessCompletion::Complete { receipt: Some(7) }));
        assert!(!random_access.is_active());
        assert!(mac.pending_uplink.is_none());
    }

    #[test]
    fn priority_rejection_returns_fragment_receipt_and_unblocks_fifo() {
        let mut mac = Mac::<u32> {
            scrambling_code: Some(123),
            ..Mac::default()
        };
        let mut random_access = crate::random_access::MsRandomAccess::new();
        let params = crate::random_access::RandomAccessParams {
            imm: 15,
            wt: 6,
            nu: 4,
            fl_factor: false,
            ts_ptr: 0,
            min_pdu_prio: 5,
            subscriber_class: None,
            gssi: None,
        };
        mac.submit_sdu(request(&bits(100), 7), false).unwrap();
        mac.submit_sdu(request(&bits(20), 8), false).unwrap();
        let error = mac.begin_access(&mut random_access, Some(&params)).unwrap_err();
        assert_eq!(error.first_receipt, None);
        assert_eq!(error.fragment_receipt, Some(7));
        assert_eq!(mac.status(), Status::Idle);
        mac.promote(false);
        assert_eq!(mac.pending_uplink.as_ref().unwrap().tx_reporter, Some(8));
    }

    #[test]
    fn invalid_time_and_failed_submission_do_not_mutate_state() {
        let mut mac = Mac::<u32>::default();
        let old = mac.dltime;
        assert_eq!(mac.advance(TdmaTime { t: 0, ..old }), Err(BitError::InvalidValue));
        assert_eq!(mac.dltime, old);
        let error = mac.submit_sdu(request(&bits(20), 8), false).err().unwrap();
        assert_eq!(error.receipt, Some(8));
        assert_eq!(mac.status(), Status::Idle);
        mac.scrambling_code = Some(123);
        let error = mac.submit_sdu(request(&bits(2000), 9), false).err().unwrap();
        assert_eq!(error.reason, BitError::Capacity);
        assert_eq!(error.receipt, Some(9));
        assert_eq!(mac.status(), Status::Idle);
    }
}
