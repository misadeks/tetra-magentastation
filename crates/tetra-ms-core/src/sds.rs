//! MS short data and SDS-TL (clauses 13, 14.7 and 29).
//!
//! The caller supplies complete CMCE SDUs and drains actions before retrying an
//! event rejected for backpressure. No clock is needed: the historical MS SDS
//! implementation has no retransmission or delivery-report timer.
//! Report correlation deliberately retains the original reference-only key;
//! reusing a reference replaces its destination and any report clears that key.
use crate::queue::Queue;
use tetra_ms_types::{BitError, BitView, PackedBits, SsiType, TetraAddress};

pub const MAX_TYPE4_BITS: u16 = 2047;
pub const TEXT_PROTOCOL_ID: u8 = 0x82;
const ACTION_CAPACITY: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SdsStatus {
    pub outstanding: u16,
    pub queued_actions: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SdsError {
    Bits(BitError),
    InvalidValue,
    UnsupportedPdu,
    OutputFull,
}
impl From<BitError> for SdsError {
    fn from(value: BitError) -> Self {
        Self::Bits(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)] // Inline bounded payload is intentional; no allocator.
pub enum UserData {
    Type1(u16),
    Type2(u32),
    Type3(u64),
    Type4(PackedBits),
}

#[derive(Clone, Copy, Debug)]
pub enum SdsEvent<'a> {
    Downlink {
        sdu: BitView<'a>,
        group: bool,
    },
    Unitdata {
        destination: TetraAddress,
        data: &'a UserData,
    },
    Status {
        destination: TetraAddress,
        status: u16,
    },
    Message {
        destination: TetraAddress,
        protocol_id: u8,
        report_request: u8,
        reference: u8,
        data: BitView<'a>,
    },
    Report {
        destination: u32,
        reference: u8,
        status: u8,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SdsAction {
    Uplink {
        destination: TetraAddress,
        sdu: PackedBits,
    },
    Unitdata {
        source: u32,
        group: bool,
        data: UserData,
    },
    Status {
        source: u32,
        status: u16,
    },
    Message {
        source: u32,
        group: bool,
        protocol_id: u8,
        report_request: u8,
        reference: u8,
        data: PackedBits,
    },
    Report {
        source: u32,
        reference: u8,
        status: u8,
        short: bool,
    },
}

/// All 256 on-air references are representable; reuse replaces the previous
/// destination, matching the MS application's reference-keyed map.
pub struct SdsCore {
    outstanding: [Option<u32>; 256],
    actions: Queue<SdsAction, ACTION_CAPACITY>,
}
const _: () = assert!(!core::mem::needs_drop::<SdsCore>());
impl Default for SdsCore {
    fn default() -> Self {
        Self::new()
    }
}

impl SdsCore {
    /// Initialize caller-owned storage without a complete temporary SDS entity.
    pub fn init_in_place(storage: &mut core::mem::MaybeUninit<Self>) -> &mut Self {
        // SAFETY: MaybeUninit grants exclusive, aligned, sufficiently sized
        // storage; init_at initializes every field before the reference escapes.
        unsafe {
            Self::init_at(storage.as_mut_ptr());
            storage.assume_init_mut()
        }
    }

    /// # Safety
    /// `out` must be aligned, writable, exclusively owned storage for `Self`.
    /// No previous value is dropped.
    pub unsafe fn init_at(out: *mut Self) {
        // SAFETY: the caller provides the complete core; initialize every field.
        unsafe {
            let entries = core::ptr::addr_of_mut!((*out).outstanding).cast::<Option<u32>>();
            for index in 0..256 {
                entries.add(index).write(None);
            }
            Queue::init_at(core::ptr::addr_of_mut!((*out).actions));
        }
    }
    pub const fn new() -> Self {
        Self {
            outstanding: [None; 256],
            actions: Queue::new(),
        }
    }
    pub fn outstanding(&self, reference: u8) -> Option<u32> {
        self.outstanding[usize::from(reference)]
    }
    pub fn status(&self) -> SdsStatus {
        SdsStatus {
            outstanding: self.outstanding.iter().filter(|entry| entry.is_some()).count() as u16,
            queued_actions: self.actions.len(),
        }
    }
    pub fn reset(&mut self) {
        self.outstanding.fill(None);
        self.actions.clear();
    }
    pub fn cancel(&mut self, reference: u8) -> bool {
        self.outstanding[usize::from(reference)].take().is_some()
    }
    pub fn pop_action(&mut self) -> Option<SdsAction> {
        self.actions.pop()
    }

    /// Failed events leave both tracking and pending actions unchanged.
    ///
    /// Preflight retains borrowed views, not staged payloads. Out-of-line emitters
    /// bound the live stack to one final action/PDU instead of a two-action queue
    /// plus separately encoded CMCE and SDS-TL payloads.
    #[inline(never)]
    pub fn push_event(&mut self, event: SdsEvent<'_>) -> Result<(), SdsError> {
        let plan = Plan::new(event)?;
        let count = 1 + u16::from(plan.second.is_some());
        if self.actions.free() < count {
            return Err(SdsError::OutputFull);
        }
        let initial = self.actions.len();
        self.emit(plan.first)?;
        if let Some(second) = plan.second
            && let Err(error) = self.emit(second)
        {
            self.rollback(initial);
            return Err(error);
        }
        match plan.tracking {
            Tracking::None => {}
            Tracking::Set(reference, destination) => self.outstanding[usize::from(reference)] = Some(destination),
            Tracking::Clear(reference) => self.outstanding[usize::from(reference)] = None,
        }
        Ok(())
    }

    #[inline(never)]
    fn emit(&mut self, action: PendingAction<'_>) -> Result<(), SdsError> {
        match action {
            PendingAction::Uplink { destination, body } => self.emit_uplink(destination, body),
            PendingAction::Unitdata { source, group, data } => self.emit_unitdata(source, group, data),
            PendingAction::Message {
                source,
                group,
                protocol_id,
                report_request,
                reference,
                data,
            } => self.emit_message(source, group, protocol_id, report_request, reference, data),
            PendingAction::Status { source, status } => {
                self.actions.push(SdsAction::Status { source, status });
                Ok(())
            }
            PendingAction::Report {
                source,
                reference,
                status,
                short,
            } => {
                self.actions.push(SdsAction::Report {
                    source,
                    reference,
                    status,
                    short,
                });
                Ok(())
            }
        }
    }

    #[inline(never)]
    fn emit_uplink(&mut self, destination: TetraAddress, body: UplinkBody<'_>) -> Result<(), SdsError> {
        let mut action = SdsAction::Uplink {
            destination,
            sdu: PackedBits::new(),
        };
        let SdsAction::Uplink { sdu, .. } = &mut action else {
            unreachable!()
        };
        write_uplink(sdu, destination, body)?;
        self.actions.push(action);
        Ok(())
    }

    #[inline(never)]
    fn emit_unitdata(&mut self, source: u32, group: bool, data: BorrowedData<'_>) -> Result<(), SdsError> {
        let mut action = SdsAction::Unitdata {
            source,
            group,
            data: match data {
                BorrowedData::Type1(value) => UserData::Type1(value),
                BorrowedData::Type2(value) => UserData::Type2(value),
                BorrowedData::Type3(value) => UserData::Type3(value),
                BorrowedData::Type4(_) => UserData::Type4(PackedBits::new()),
            },
        };
        if let BorrowedData::Type4(view) = data {
            let SdsAction::Unitdata {
                data: UserData::Type4(bits),
                ..
            } = &mut action
            else {
                unreachable!()
            };
            bits.append(view)?;
        }
        self.actions.push(action);
        Ok(())
    }

    #[inline(never)]
    fn emit_message(
        &mut self,
        source: u32,
        group: bool,
        protocol_id: u8,
        report_request: u8,
        reference: u8,
        data: BitView<'_>,
    ) -> Result<(), SdsError> {
        let mut action = SdsAction::Message {
            source,
            group,
            protocol_id,
            report_request,
            reference,
            data: PackedBits::new(),
        };
        let SdsAction::Message { data: bits, .. } = &mut action else {
            unreachable!()
        };
        bits.append(data)?;
        self.actions.push(action);
        Ok(())
    }

    // Keep an unexpected encode failure atomic without reserving another
    // action-sized return temporary in the normal dispatcher frame.
    #[cold]
    #[inline(never)]
    fn rollback(&mut self, initial: u16) {
        while self.actions.len() > initial {
            self.actions.remove(initial);
        }
    }
}

#[derive(Clone, Copy)]
enum BorrowedData<'a> {
    Type1(u16),
    Type2(u32),
    Type3(u64),
    Type4(BitView<'a>),
}
impl<'a> From<&'a UserData> for BorrowedData<'a> {
    fn from(data: &'a UserData) -> Self {
        match data {
            UserData::Type1(value) => Self::Type1(*value),
            UserData::Type2(value) => Self::Type2(*value),
            UserData::Type3(value) => Self::Type3(*value),
            UserData::Type4(bits) => Self::Type4(bits.view()),
        }
    }
}
#[derive(Clone, Copy)]
enum UplinkBody<'a> {
    Data(BorrowedData<'a>),
    Status(u16),
    Transfer {
        protocol_id: u8,
        report_request: u8,
        reference: u8,
        data: BitView<'a>,
    },
    Receipt {
        protocol_id: u8,
        reference: u8,
        status: u8,
        ack: bool,
    },
}
#[derive(Clone, Copy)]
enum PendingAction<'a> {
    Uplink {
        destination: TetraAddress,
        body: UplinkBody<'a>,
    },
    Unitdata {
        source: u32,
        group: bool,
        data: BorrowedData<'a>,
    },
    Status {
        source: u32,
        status: u16,
    },
    Message {
        source: u32,
        group: bool,
        protocol_id: u8,
        report_request: u8,
        reference: u8,
        data: BitView<'a>,
    },
    Report {
        source: u32,
        reference: u8,
        status: u8,
        short: bool,
    },
}
enum Tracking {
    None,
    Set(u8, u32),
    Clear(u8),
}
struct Plan<'a> {
    first: PendingAction<'a>,
    second: Option<PendingAction<'a>>,
    tracking: Tracking,
}
impl<'a> Plan<'a> {
    fn one(first: PendingAction<'a>) -> Self {
        Self {
            first,
            second: None,
            tracking: Tracking::None,
        }
    }

    fn new(event: SdsEvent<'a>) -> Result<Self, SdsError> {
        let action = match event {
            SdsEvent::Downlink { sdu, group } => return Self::downlink(sdu, group),
            SdsEvent::Unitdata { destination, data } => {
                validate_destination(destination)?;
                if matches!(data, UserData::Type4(bits) if bits.len() > MAX_TYPE4_BITS) {
                    return Err(SdsError::InvalidValue);
                }
                PendingAction::Uplink {
                    destination,
                    body: UplinkBody::Data(data.into()),
                }
            }
            SdsEvent::Status { destination, status } => {
                validate_destination(destination)?;
                PendingAction::Uplink {
                    destination,
                    body: UplinkBody::Status(status),
                }
            }
            SdsEvent::Message {
                destination,
                protocol_id,
                report_request,
                reference,
                data,
            } => {
                if report_request > 3 || data.len() > MAX_TYPE4_BITS - 24 {
                    return Err(SdsError::InvalidValue);
                }
                validate_destination(destination)?;
                return Ok(Self {
                    first: PendingAction::Uplink {
                        destination,
                        body: UplinkBody::Transfer {
                            protocol_id,
                            report_request,
                            reference,
                            data,
                        },
                    },
                    second: None,
                    tracking: if report_request != 0 {
                        Tracking::Set(reference, destination.ssi)
                    } else {
                        Tracking::None
                    },
                });
            }
            SdsEvent::Report {
                destination,
                reference,
                status,
            } => {
                let destination = TetraAddress::new(destination, SsiType::Issi);
                validate_destination(destination)?;
                PendingAction::Uplink {
                    destination,
                    body: UplinkBody::Receipt {
                        protocol_id: TEXT_PROTOCOL_ID,
                        reference,
                        status,
                        ack: false,
                    },
                }
            }
        };
        Ok(Self::one(action))
    }

    fn downlink(sdu: BitView<'a>, group: bool) -> Result<Self, SdsError> {
        let mut reader = Reader::new(sdu);
        let kind = reader.read(5)?;
        if kind != 8 && kind != 15 {
            return Err(SdsError::UnsupportedPdu);
        }
        let party_type = reader.read(2)?;
        let source = if party_type == 1 || party_type == 2 { reader.read(24)? } else { 0 };
        if party_type == 2 {
            reader.read(24)?;
        }
        if kind == 8 {
            let status = reader.read(16)? as u16;
            reader.options()?;
            return Ok(if status >> 10 == 0b011111 {
                let reference = status as u8;
                let status = match (status >> 8) & 3 {
                    2 => 0,
                    3 => 2,
                    1 => 0x52,
                    _ => 0x50,
                };
                Self {
                    first: PendingAction::Report {
                        source,
                        reference,
                        status,
                        short: true,
                    },
                    second: None,
                    tracking: Tracking::Clear(reference),
                }
            } else {
                Self::one(PendingAction::Status { source, status })
            });
        }
        let data = match reader.read(2)? {
            0 => BorrowedData::Type1(reader.read(16)? as u16),
            1 => BorrowedData::Type2(reader.read(32)?),
            2 => BorrowedData::Type3((u64::from(reader.read(32)?) << 32) | u64::from(reader.read(32)?)),
            _ => {
                let len = reader.read(11)? as u16;
                BorrowedData::Type4(reader.take(len)?)
            }
        };
        reader.options()?;
        let tl = match data {
            BorrowedData::Type4(bits) => decode_tl(bits)?,
            _ => None,
        };
        Ok(match tl {
            Some(Tl::Transfer {
                protocol_id,
                report_request,
                reference,
                data,
            }) => Self {
                first: PendingAction::Message {
                    source,
                    group,
                    protocol_id,
                    report_request,
                    reference,
                    data,
                },
                second: (report_request & 1 != 0 && !group).then_some(Self::receipt(source, protocol_id, reference, 0, false)),
                tracking: Tracking::None,
            },
            Some(Tl::Report {
                protocol_id,
                reference,
                status,
                ack_required,
            }) => Self {
                first: PendingAction::Report {
                    source,
                    reference,
                    status,
                    short: false,
                },
                second: ack_required.then_some(Self::receipt(source, protocol_id, reference, status, true)),
                tracking: Tracking::Clear(reference),
            },
            None => Self::one(PendingAction::Unitdata { source, group, data }),
        })
    }

    fn receipt(destination: u32, protocol_id: u8, reference: u8, status: u8, ack: bool) -> PendingAction<'a> {
        PendingAction::Uplink {
            destination: TetraAddress::new(destination, SsiType::Issi),
            body: UplinkBody::Receipt {
                protocol_id,
                reference,
                status,
                ack,
            },
        }
    }
}

fn validate_destination(destination: TetraAddress) -> Result<(), SdsError> {
    if destination.ssi > 0xff_ffff || !matches!(destination.ssi_type, SsiType::Issi | SsiType::Gssi) {
        return Err(SdsError::InvalidValue);
    }
    Ok(())
}

fn write_uplink(bits: &mut PackedBits, destination: TetraAddress, body: UplinkBody<'_>) -> Result<(), SdsError> {
    validate_destination(destination)?;
    bits.push(5, if matches!(body, UplinkBody::Status(_)) { 8 } else { 15 })?;
    bits.push(4, 0)?;
    bits.push(2, 1)?;
    bits.push(24, destination.ssi)?;
    match body {
        UplinkBody::Status(status) => bits.push(16, u32::from(status))?,
        UplinkBody::Data(data) => match data {
            BorrowedData::Type1(value) => {
                bits.push(2, 0)?;
                bits.push(16, u32::from(value))?;
            }
            BorrowedData::Type2(value) => {
                bits.push(2, 1)?;
                bits.push(32, value)?;
            }
            BorrowedData::Type3(value) => {
                bits.push(2, 2)?;
                bits.push(32, (value >> 32) as u32)?;
                bits.push(32, value as u32)?;
            }
            BorrowedData::Type4(value) => {
                if value.len() > MAX_TYPE4_BITS {
                    return Err(SdsError::InvalidValue);
                }
                bits.push(2, 3)?;
                bits.push(11, u32::from(value.len()))?;
                bits.append(value)?;
            }
        },
        UplinkBody::Transfer {
            protocol_id,
            report_request,
            reference,
            data,
        } => {
            bits.push(2, 3)?;
            bits.push(11, u32::from(24 + data.len()))?;
            bits.push(8, u32::from(protocol_id))?;
            bits.push(
                8,
                u32::from((report_request << 2) | (u8::from(destination.ssi_type == SsiType::Gssi) << 1)),
            )?;
            bits.push(8, u32::from(reference))?;
            bits.append(data)?;
        }
        UplinkBody::Receipt {
            protocol_id,
            reference,
            status,
            ack,
        } => {
            bits.push(2, 3)?;
            bits.push(11, 32)?;
            bits.push(8, u32::from(protocol_id))?;
            bits.push(8, if ack { 0x20 } else { 0x10 })?;
            bits.push(8, u32::from(status))?;
            bits.push(8, u32::from(reference))?;
        }
    }
    bits.push(1, 0)?;
    Ok(())
}

enum Tl<'a> {
    Transfer {
        protocol_id: u8,
        report_request: u8,
        reference: u8,
        data: BitView<'a>,
    },
    Report {
        protocol_id: u8,
        reference: u8,
        status: u8,
        ack_required: bool,
    },
}
fn decode_tl(bits: BitView<'_>) -> Result<Option<Tl<'_>>, SdsError> {
    if bits.len() < 16 {
        return Ok(None);
    }
    let protocol_id = bits.read(0, 8)? as u8;
    let flags = bits.read(8, 8)? as u8;
    match flags >> 4 {
        0 if flags & 1 == 0 && bits.len() >= 24 => Ok(Some(Tl::Transfer {
            protocol_id,
            report_request: (flags >> 2) & 3,
            reference: bits.read(16, 8)? as u8,
            data: bits.window(24, bits.len() - 24)?,
        })),
        kind @ (1 | 2) if bits.len() >= 32 && (kind == 2 || flags & 1 == 0) => Ok(Some(Tl::Report {
            protocol_id,
            reference: bits.read(24, 8)? as u8,
            status: bits.read(16, 8)? as u8,
            ack_required: kind == 1 && flags & 8 != 0,
        })),
        _ => Ok(None),
    }
}

struct Reader<'a> {
    bits: BitView<'a>,
    offset: u16,
}
impl<'a> Reader<'a> {
    fn new(bits: BitView<'a>) -> Self {
        Self { bits, offset: 0 }
    }
    fn read(&mut self, width: u8) -> Result<u32, SdsError> {
        let value = self.bits.read(self.offset, width)?;
        self.offset += u16::from(width);
        Ok(value)
    }
    fn take(&mut self, len: u16) -> Result<BitView<'a>, SdsError> {
        let view = self.bits.window(self.offset, len)?;
        self.offset += len;
        Ok(view)
    }
    fn options(&mut self) -> Result<(), SdsError> {
        if self.read(1)? == 0 {
            return Ok(());
        }
        // Parse (then ignore) the same two Type-3 fields, in their original order.
        for expected in [2, 6] {
            if self.bits.read(self.offset, 1)? != 0 && self.bits.read(self.offset + 1, 4)? == expected {
                self.read(5)?;
                let len = self.read(11)? as u16;
                self.take(len)?;
            }
        }
        if self.read(1)? != 0 {
            return Err(SdsError::InvalidValue);
        }
        Ok(())
    }
}
