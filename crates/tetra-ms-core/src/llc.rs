//! BlueStation MS basic-link procedure (clause 22.3.2.3).
//!
//! Time, MAC receipts and SAP metadata belong to the caller. The wire encoding,
//! ACK ownership, per-SSI sequencing, stop-and-wait and T251/N252 live here.
//! Existing behavior is intentional: duplicate downlink SDUs are delivered and
//! outbound FCS flags do not cause an FCS to be appended by LLC.

use tetra_ms_types::{BitError, BitView, MAX_PDU_BITS, PackedBits, SsiType, TdmaTime, TetraAddress};

use crate::{fcs, queue::Queue};

pub const PENDING_CAPACITY: u16 = 8;
pub const ACK_CAPACITY: u16 = 8;
pub const UDATA_CAPACITY: u16 = 8;
pub const LINK_CAPACITY: u16 = 8;
pub const ACTION_CAPACITY: u16 = PENDING_CAPACITY + ACK_CAPACITY + UDATA_CAPACITY;
pub const T251_SLOTS: i32 = 16;
pub const N252_RETRIES: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlcError {
    InvalidIdentity,
    InvalidTime,
    InvalidPdu,
    TruncatedPdu,
    InvalidFcs,
    UnsupportedPdu(u8),
    CapacityExceeded,
    QueueFull,
    ReceiptBusy,
    StaleReceipt,
    InvalidTransition,
    GenerationExhausted,
}

impl From<BitError> for LlcError {
    fn from(error: BitError) -> Self {
        match error {
            BitError::Truncated => Self::TruncatedPdu,
            BitError::Capacity => Self::CapacityExceeded,
            BitError::InvalidWidth | BitError::InvalidValue => Self::InvalidPdu,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LlcConfig {
    pub own_issi: u32,
}

impl LlcConfig {
    pub fn validate(self) -> Result<(), LlcError> {
        if self.own_issi == 0 || self.own_issi > 0xff_ffff {
            return Err(LlcError::InvalidIdentity);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Acknowledged,
    Unacknowledged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Receipt {
    pub epoch: u32,
    pub sequence: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxFeedback {
    Transmitted,
    Rejected,
    Deferred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completion {
    Acknowledged,
    RetryExhausted,
}

#[derive(Debug, Clone, Copy)]
pub struct SendRequest<'a> {
    /// Opaque caller metadata key, unique until a Completed/Transmit action releases it.
    pub command: u32,
    pub address: TetraAddress,
    pub service: Service,
    pub fcs: bool,
    pub sdu: BitView<'a>,
}

#[derive(Debug, Clone, Copy)]
pub enum LlcEvent<'a> {
    Send(SendRequest<'a>),
    Receive {
        tag: u32,
        address: TetraAddress,
        pdu: BitView<'a>,
    },
    Transmission {
        receipt: Receipt,
        feedback: TxFeedback,
    },
    /// Corresponds to the application router's end-of-tick, not a wall-clock timer.
    Tick,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlcAction {
    Transmit {
        command: u32,
        address: TetraAddress,
        receipt: Option<Receipt>,
        /// Some for an automatic BL-ACK. The adapter routes it on this timeslot.
        ack_slot: Option<u8>,
        /// Group TL-DATA historically resets link_id to zero; TL-UNITDATA does not.
        group_data: bool,
        pdu: PackedBits,
    },
    Deliver {
        tag: u32,
        service: Service,
        fcs: bool,
        sdu: PackedBits,
    },
    Completed {
        command: u32,
        result: Completion,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LlcStatus {
    pub pending: u16,
    pub unacknowledged: u16,
    pub scheduled_acks: u16,
    pub actions: u16,
    pub links: u16,
    pub ignored_acks: u32,
}

struct Link {
    ssi: u32,
    next_ns: u8,
}

struct Pending {
    command: u32,
    address: TetraAddress,
    ns: u8,
    retries: u8,
    receipt: Option<Receipt>,
    done: Option<TdmaTime>,
    expired: bool,
    pdu: PackedBits,
}

struct Udata {
    command: u32,
    address: TetraAddress,
    group_data: bool,
    pdu: PackedBits,
}

struct Ack {
    address: TetraAddress,
    nr: u8,
    ts: u8,
}

pub struct LlcCore {
    config: LlcConfig,
    epoch: u32,
    next_receipt: u32,
    ignored_acks: u32,
    links: Queue<Link, { LINK_CAPACITY as usize }>,
    pending: Queue<Pending, { PENDING_CAPACITY as usize }>,
    udata: Queue<Udata, { UDATA_CAPACITY as usize }>,
    acks: Queue<Ack, { ACK_CAPACITY as usize }>,
    actions: Queue<LlcAction, { ACTION_CAPACITY as usize }>,
}

/// Byte sizes for this compilation target, available without std or an allocator.
#[derive(Debug, Clone, Copy)]
pub struct MemoryFootprint {
    pub core: u32,
    pub event: u32,
    pub action: u32,
    pub status: u32,
    pub pending_queue: u32,
    pub action_queue: u32,
    pub ack_queue: u32,
    pub udata_queue: u32,
    pub links: u32,
}

pub const MEMORY_FOOTPRINT: MemoryFootprint = MemoryFootprint {
    core: core::mem::size_of::<LlcCore>() as u32,
    event: core::mem::size_of::<LlcEvent<'_>>() as u32,
    action: core::mem::size_of::<LlcAction>() as u32,
    status: core::mem::size_of::<LlcStatus>() as u32,
    pending_queue: core::mem::size_of::<Queue<Pending, { PENDING_CAPACITY as usize }>>() as u32,
    action_queue: core::mem::size_of::<Queue<LlcAction, { ACTION_CAPACITY as usize }>>() as u32,
    ack_queue: core::mem::size_of::<Queue<Ack, { ACK_CAPACITY as usize }>>() as u32,
    udata_queue: core::mem::size_of::<Queue<Udata, { UDATA_CAPACITY as usize }>>() as u32,
    links: core::mem::size_of::<Queue<Link, { LINK_CAPACITY as usize }>>() as u32,
};

impl LlcCore {
    /// Initialize caller-owned storage without a complete temporary core on the
    /// stack. This is the preferred constructor for static embedded/FFI storage.
    /// Invalid configuration leaves the storage untouched.
    pub fn init_in_place(storage: &mut core::mem::MaybeUninit<Self>, config: LlcConfig) -> Result<&mut Self, LlcError> {
        config.validate()?;
        let out = storage.as_mut_ptr();
        // SAFETY: MaybeUninit grants exclusive, aligned storage for Self.
        // Every field is initialized exactly once before assume_init_mut.
        // Queue::init_at initializes each Option slot and its length without
        // constructing another complete queue. These fields contain no Drop state.
        unsafe {
            core::ptr::addr_of_mut!((*out).config).write(config);
            core::ptr::addr_of_mut!((*out).epoch).write(1);
            core::ptr::addr_of_mut!((*out).next_receipt).write(0);
            core::ptr::addr_of_mut!((*out).ignored_acks).write(0);
            Queue::init_at(core::ptr::addr_of_mut!((*out).links));
            Queue::init_at(core::ptr::addr_of_mut!((*out).pending));
            Queue::init_at(core::ptr::addr_of_mut!((*out).udata));
            Queue::init_at(core::ptr::addr_of_mut!((*out).acks));
            Queue::init_at(core::ptr::addr_of_mut!((*out).actions));
            Ok(storage.assume_init_mut())
        }
    }

    pub fn new(config: LlcConfig) -> Result<Self, LlcError> {
        config.validate()?;
        Ok(Self {
            config,
            epoch: 1,
            next_receipt: 0,
            ignored_acks: 0,
            links: Queue::new(),
            pending: Queue::new(),
            udata: Queue::new(),
            acks: Queue::new(),
            actions: Queue::new(),
        })
    }

    /// In-place reset. No second core is constructed. Old receipts become stale.
    /// Reset deliberately cancels queued commands; the caller owns cancellation notification.
    pub fn reset(&mut self, config: LlcConfig) -> Result<(), LlcError> {
        config.validate()?;
        let epoch = self.epoch.checked_add(1).ok_or(LlcError::GenerationExhausted)?;
        self.config = config;
        self.epoch = epoch;
        self.next_receipt = 0;
        self.ignored_acks = 0;
        self.links.clear();
        self.pending.clear();
        self.udata.clear();
        self.acks.clear();
        self.actions.clear();
        Ok(())
    }

    /// Errors never consume a command or partially modify protocol state.
    pub fn handle(&mut self, now: TdmaTime, event: &LlcEvent<'_>) -> Result<(), LlcError> {
        if !now.is_valid() {
            return Err(LlcError::InvalidTime);
        }
        match *event {
            LlcEvent::Send(request) => self.send(request),
            LlcEvent::Receive { tag, address, pdu } => self.receive(now, tag, address, pdu),
            LlcEvent::Transmission { receipt, feedback } => self.feedback(now, receipt, feedback),
            LlcEvent::Tick => self.tick(now),
        }
    }

    pub fn poll_action(&mut self) -> Option<LlcAction> {
        self.actions.pop()
    }

    pub fn status(&self) -> LlcStatus {
        LlcStatus {
            pending: self.pending.len(),
            unacknowledged: self.udata.len(),
            scheduled_acks: self.acks.len(),
            actions: self.actions.len(),
            links: self.links.len(),
            ignored_acks: self.ignored_acks,
        }
    }

    fn reserve_actions(&self, count: u16) -> Result<(), LlcError> {
        if self.actions.free() < count {
            return Err(LlcError::QueueFull);
        }
        Ok(())
    }

    fn send(&mut self, request: SendRequest<'_>) -> Result<(), LlcError> {
        if request.address.ssi > 0xff_ffff {
            return Err(LlcError::InvalidIdentity);
        }
        if self.pending.iter().any(|entry| entry.command == request.command)
            || self.udata.iter().any(|entry| entry.command == request.command)
            || self.actions.iter().any(|action| match action {
                LlcAction::Transmit {
                    command, ack_slot: None, ..
                }
                | LlcAction::Completed { command, .. } => *command == request.command,
                _ => false,
            })
        {
            return Err(LlcError::ReceiptBusy);
        }
        let group_data = request.service == Service::Acknowledged && request.address.ssi_type == SsiType::Gssi;
        if request.service == Service::Unacknowledged || group_data {
            if self.udata.free() == 0 {
                return Err(LlcError::QueueFull);
            }
            if request.sdu.len() > MAX_PDU_BITS - 4 {
                return Err(LlcError::CapacityExceeded);
            }
            let mut pdu = PackedBits::new();
            pdu.push(4, 0b0010)?;
            pdu.append(request.sdu)?;
            self.udata.push(Udata {
                command: request.command,
                address: request.address,
                group_data,
                pdu,
            });
            return Ok(());
        }
        if self.pending.free() == 0 {
            return Err(LlcError::QueueFull);
        }
        let ack_index = self.acks.iter().position(|ack| ack.address.ssi == request.address.ssi);
        let header_len = if ack_index.is_some() { 6 } else { 5 };
        if request.sdu.len() > MAX_PDU_BITS - header_len {
            return Err(LlcError::CapacityExceeded);
        }
        if !self.links.iter().any(|link| link.ssi == request.address.ssi) && self.links.free() == 0 {
            return Err(LlcError::CapacityExceeded);
        }
        if !self.links.iter().any(|link| link.ssi == request.address.ssi) {
            self.links.push(Link {
                ssi: request.address.ssi,
                next_ns: 0,
            });
        }
        let mut ns = 0;
        for link in self.links.iter_mut() {
            if link.ssi == request.address.ssi {
                ns = link.next_ns;
                link.next_ns ^= 1;
                break;
            }
        }
        let mut pdu = PackedBits::new();
        let fcs = u32::from(request.fcs) << 2;
        if let Some(ack) = ack_index.and_then(|index| self.acks.remove(index as u16)) {
            pdu.push(4, fcs)?;
            pdu.push(1, u32::from(ack.nr))?;
        } else {
            pdu.push(4, fcs | 1)?;
        }
        pdu.push(1, u32::from(ns))?;
        pdu.append(request.sdu)?;
        self.pending.push(Pending {
            command: request.command,
            address: request.address,
            ns,
            retries: 0,
            receipt: None,
            done: None,
            expired: false,
            pdu,
        });
        Ok(())
    }

    fn should_acknowledge(&self, address: TetraAddress) -> bool {
        match address.ssi_type {
            SsiType::Ssi | SsiType::Issi | SsiType::Ussi | SsiType::Smi => address.ssi == self.config.own_issi,
            _ => true,
        }
    }

    fn receive(&mut self, now: TdmaTime, tag: u32, address: TetraAddress, pdu: BitView<'_>) -> Result<(), LlcError> {
        if address.ssi > 0xff_ffff {
            return Err(LlcError::InvalidIdentity);
        }
        if pdu.len() > MAX_PDU_BITS {
            return Err(LlcError::CapacityExceeded);
        }
        let kind = pdu.read(0, 4)? as u8;
        if kind >= 8 {
            return Err(LlcError::UnsupportedPdu(kind));
        }
        let fcs_present = kind & 4 != 0;
        let kind = kind & 3;
        let (header, ns, nr) = match kind {
            0 => (6, Some(pdu.read(5, 1)? as u8), Some(pdu.read(4, 1)? as u8)),
            1 => (5, Some(pdu.read(4, 1)? as u8), None),
            2 => (4, None, None),
            _ => (5, None, Some(pdu.read(4, 1)? as u8)),
        };
        let payload = pdu.window(header, pdu.len() - header)?;
        if fcs_present && !fcs::check(payload)? {
            return Err(LlcError::InvalidFcs);
        }
        let owe_ack = ns.is_some() && self.should_acknowledge(address);
        if owe_ack && self.acks.free() == 0 {
            return Err(LlcError::QueueFull);
        }
        let deliver = kind != 3 || !payload.is_empty();
        let matched = nr.and_then(|nr| {
            self.pending
                .iter()
                .position(|entry| entry.address.ssi == address.ssi && entry.receipt.is_some())
                .filter(|&index| self.pending.iter().nth(index).is_some_and(|entry| entry.ns == nr))
        });
        self.reserve_actions(u16::from(matched.is_some()) + u16::from(deliver))?;
        if let Some(ns) = ns.filter(|_| owe_ack) {
            self.acks.push(Ack {
                address,
                nr: ns,
                ts: now.add_timeslots(-2).t,
            });
        }
        if let Some(entry) = matched.and_then(|index| self.pending.remove(index as u16)) {
            self.actions.push(LlcAction::Completed {
                command: entry.command,
                result: Completion::Acknowledged,
            });
        } else if nr.is_some() {
            self.ignored_acks = self.ignored_acks.saturating_add(1);
        }
        if deliver {
            self.actions.push(LlcAction::Deliver {
                tag,
                service: if kind == 2 {
                    Service::Unacknowledged
                } else {
                    Service::Acknowledged
                },
                fcs: fcs_present,
                sdu: PackedBits::from_view(payload)?,
            });
        }
        Ok(())
    }

    fn feedback(&mut self, now: TdmaTime, receipt: Receipt, feedback: TxFeedback) -> Result<(), LlcError> {
        let Some(entry) = self.pending.iter_mut().find(|entry| entry.receipt == Some(receipt)) else {
            return Err(LlcError::StaleReceipt);
        };
        if entry.done.is_some() {
            return Err(LlcError::InvalidTransition);
        }
        if feedback != TxFeedback::Deferred {
            entry.done = Some(now);
        }
        Ok(())
    }

    fn tick(&mut self, now: TdmaTime) -> Result<(), LlcError> {
        // Upper bound includes a completion OR transmission for each pending entry.
        self.reserve_actions(self.pending.len() + self.acks.len() + self.udata.len())?;
        self.next_receipt
            .checked_add(u32::from(self.pending.len()))
            .ok_or(LlcError::GenerationExhausted)?;
        for entry in self.pending.iter_mut() {
            if entry.done.is_some_and(|done| now.diff(done) >= T251_SLOTS) {
                if entry.retries == N252_RETRIES {
                    self.actions.push(LlcAction::Completed {
                        command: entry.command,
                        result: Completion::RetryExhausted,
                    });
                    entry.expired = true;
                } else {
                    entry.retries += 1;
                    self.next_receipt += 1;
                    let receipt = Receipt {
                        epoch: self.epoch,
                        sequence: self.next_receipt,
                    };
                    entry.receipt = Some(receipt);
                    entry.done = None;
                    self.actions.push(LlcAction::Transmit {
                        command: entry.command,
                        address: entry.address,
                        receipt: Some(receipt),
                        ack_slot: None,
                        group_data: false,
                        pdu: entry.pdu.clone(),
                    });
                }
            }
        }
        self.pending.retain(|entry| !entry.expired);
        let mut blocked = [0; PENDING_CAPACITY as usize];
        let mut blocked_len = 0;
        for entry in self.pending.iter_mut() {
            if entry.receipt.is_some() {
                blocked[blocked_len] = entry.address.ssi;
                blocked_len += 1;
            } else if !blocked[..blocked_len].contains(&entry.address.ssi) {
                self.next_receipt += 1;
                let receipt = Receipt {
                    epoch: self.epoch,
                    sequence: self.next_receipt,
                };
                entry.receipt = Some(receipt);
                self.actions.push(LlcAction::Transmit {
                    command: entry.command,
                    address: entry.address,
                    receipt: Some(receipt),
                    ack_slot: None,
                    group_data: false,
                    pdu: entry.pdu.clone(),
                });
                blocked[blocked_len] = entry.address.ssi;
                blocked_len += 1;
            }
        }
        while let Some(ack) = self.acks.pop() {
            let mut pdu = PackedBits::new();
            pdu.push(4, 3)?;
            pdu.push(1, u32::from(ack.nr))?;
            self.actions.push(LlcAction::Transmit {
                command: 0,
                address: ack.address,
                receipt: None,
                ack_slot: Some(ack.ts),
                group_data: false,
                pdu,
            });
        }
        while let Some(udata) = self.udata.pop() {
            self.actions.push(LlcAction::Transmit {
                command: udata.command,
                address: udata.address,
                receipt: None,
                ack_slot: None,
                group_data: udata.group_data,
                pdu: udata.pdu,
            });
        }
        Ok(())
    }
}

const _: () = {
    assert!(core::mem::size_of::<LlcCore>() <= 16_384);
    assert!(core::mem::size_of::<LlcEvent<'_>>() <= 64);
    assert!(core::mem::size_of::<LlcAction>() <= 384);
    assert!(core::mem::size_of::<LlcStatus>() <= 16);
    assert!(MEMORY_FOOTPRINT.pending_queue <= 3_072);
    assert!(MEMORY_FOOTPRINT.action_queue <= 9_216);
    assert!(MEMORY_FOOTPRINT.ack_queue <= 128);
    assert!(MEMORY_FOOTPRINT.udata_queue <= 3_072);
    assert!(MEMORY_FOOTPRINT.links <= 128);
    assert!(core::mem::size_of::<LlcCore>() == 14_916);
    assert!(core::mem::size_of::<LlcAction>() == 368);
};

#[cfg(target_pointer_width = "32")]
const _: [(); 28] = [(); core::mem::size_of::<LlcEvent<'_>>()];
