//! Checked downlink wire fields and payload boundaries. Unsupported enhanced
//! allocations are explicit errors, never silently treated as basic grants.
use super::{Allocation, AllocationType, Grant, GrantCapacity};
use crate::random_access::{self, AccessCode, AccessField, RandomAccessParams, SlotAccessAssign};
use crate::types::{BitError, BitView, SsiType, TdmaTime, TetraAddress};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Unsupported {
    ExtendedAllocation,
    AugmentedAllocation,
    Supplementary,
    USignal,
    Broadcast,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    Bits(BitError),
    Unsupported(Unsupported),
}

/// Per-block concatenation state. A non-advancing decoder ends the block;
/// fewer than 16 trailing bits cannot contain another MAC null PDU.
pub struct BlockCursor {
    remaining: u16,
}

impl BlockCursor {
    pub fn new(bits: u16) -> Self {
        Self { remaining: bits }
    }

    pub fn advance(&mut self, consumed: u16) -> Result<bool, BitError> {
        if consumed > self.remaining {
            return Err(BitError::Truncated);
        }
        self.remaining -= consumed;
        Ok(consumed != 0 && self.remaining >= 16)
    }
}

impl From<BitError> for DecodeError {
    fn from(error: BitError) -> Self {
        Self::Bits(error)
    }
}

struct Reader<'a> {
    bits: BitView<'a>,
    pos: u16,
}

impl Reader<'_> {
    fn read(&mut self, width: u8) -> Result<u32, DecodeError> {
        let value = self.bits.read(self.pos, width)?;
        self.pos += u16::from(width);
        Ok(value)
    }

    fn expect(&mut self, width: u8, value: u32) -> Result<(), DecodeError> {
        if self.read(width)? == value {
            Ok(())
        } else {
            Err(BitError::InvalidValue.into())
        }
    }

    fn grant(&mut self) -> Result<Grant, DecodeError> {
        const SLOTS: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 8, 10, 13, 17, 24, 34, 51, 68, 0];
        let capacity = match self.read(4)? {
            0 => GrantCapacity::FirstSubslot,
            15 => GrantCapacity::SecondSubslot,
            value => GrantCapacity::Slots(SLOTS[value as usize]),
        };
        Ok(Grant {
            capacity,
            next_opportunity: self.read(4)? == 0,
        })
    }

    fn allocation(&mut self, owner: u32) -> Result<Allocation, DecodeError> {
        let kind = if self.read(2)? == 1 {
            AllocationType::Additional
        } else {
            AllocationType::Replace
        };
        let bitmap = self.read(4)?;
        let assignment = self.read(2)?;
        self.read(1)?; // CLCH permission
        let cell_change = self.read(1)? != 0;
        let carrier = self.read(12)? as u16;
        if self.read(1)? != 0 {
            return Err(DecodeError::Unsupported(Unsupported::ExtendedAllocation));
        }
        if self.read(2)? == 0 {
            self.read(2)?; // frame-18 monitoring pattern
        }
        if assignment == 0 {
            return Err(DecodeError::Unsupported(Unsupported::AugmentedAllocation));
        }
        Ok(Allocation {
            carrier,
            extended: false,
            cell_change,
            downlink: assignment != 2,
            kind,
            slots: [bitmap & 8 != 0, bitmap & 4 != 0, bitmap & 2 != 0, bitmap & 1 != 0],
            owner,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Resource {
    pub header_bits: u16,
    pub fill_bits: bool,
    pub encryption_mode: u8,
    pub random_access_flag: bool,
    pub length_ind: u8,
    pub addr: Option<TetraAddress>,
    pub event_label: Option<u16>,
    pub usage_marker: Option<u8>,
    pub grant: Option<Grant>,
    pub allocation: Option<Allocation>,
}

pub fn resource(bits: BitView<'_>) -> Result<Resource, DecodeError> {
    let mut reader = Reader { bits, pos: 0 };
    reader.expect(2, 0)?;
    let mut header = Resource {
        header_bits: 0,
        fill_bits: reader.read(1)? != 0,
        encryption_mode: {
            reader.read(1)?;
            reader.read(2)? as u8
        },
        random_access_flag: reader.read(1)? != 0,
        length_ind: reader.read(6)? as u8,
        addr: None,
        event_label: None,
        usage_marker: None,
        grant: None,
        allocation: None,
    };
    let address_type = reader.read(3)?;
    if address_type == 0 {
        header.fill_bits = false;
        header.encryption_mode = 0;
        header.random_access_flag = false;
        header.header_bits = reader.pos;
        return Ok(header);
    }
    if address_type != 2 {
        let ssi_type = match address_type {
            3 => SsiType::Ussi,
            4 | 7 => SsiType::Smi,
            _ => SsiType::Ssi,
        };
        header.addr = Some(TetraAddress {
            ssi: reader.read(24)?,
            ssi_type,
        });
    }
    if matches!(address_type, 2 | 5 | 7) {
        header.event_label = Some(reader.read(10)? as u16);
    }
    if address_type == 6 {
        header.usage_marker = Some(reader.read(6)? as u8);
    }
    if reader.read(1)? != 0 {
        reader.read(4)?; // power control, not acted on by the historical MS
    }
    if reader.read(1)? != 0 {
        header.grant = Some(reader.grant()?);
    }
    if reader.read(1)? != 0 {
        header.allocation = Some(reader.allocation(header.addr.map_or(0, |addr| addr.ssi))?);
    }
    header.header_bits = reader.pos;
    Ok(header)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Payload {
    pub start: u16,
    /// Exclusive SDU end, relative to the beginning of the MAC PDU.
    pub end: u16,
    /// Octet-declared/clamped boundary at which a concatenated PDU starts.
    pub next: u16,
    pub fill: u16,
    pub truncated: bool,
}

/// `None` length means the PDU fills the physical block. Under-declared grant
/// headers carry an empty SDU but retain their declared concatenation boundary.
pub fn payload(bits: BitView<'_>, header: u16, declared: Option<u16>, has_fill: bool) -> Result<Payload, BitError> {
    if header > bits.len() {
        return Err(BitError::Truncated);
    }
    let requested = declared.unwrap_or(bits.len());
    let next = requested.min(bits.len());
    let mut end = next;
    let mut fill = 0;
    if has_fill {
        while end > header {
            end -= 1;
            if bits.read(end, 1)? != 0 {
                fill = next - end;
                break;
            }
        }
        if fill == 0 {
            end = next;
        }
    }
    Ok(Payload {
        start: header,
        end: end.max(header),
        next,
        fill,
        truncated: requested > bits.len(),
    })
}

impl Resource {
    pub fn payload(self, bits: BitView<'_>) -> Result<Payload, BitError> {
        let length = match self.length_ind {
            0 => Some(self.header_bits),
            1..=57 => Some(u16::from(self.length_ind) * 8),
            62 | 63 => None,
            _ => return Err(BitError::InvalidValue),
        };
        payload(bits, self.header_bits, length, self.fill_bits)
    }
}

pub fn fragment(bits: BitView<'_>) -> Result<Payload, DecodeError> {
    let mut reader = Reader { bits, pos: 0 };
    reader.expect(3, 0b010)?;
    let fill = reader.read(1)? != 0;
    Ok(payload(bits, reader.pos, None, fill)?)
}

pub fn end(bits: BitView<'_>) -> Result<Payload, DecodeError> {
    let mut reader = Reader { bits, pos: 0 };
    reader.expect(3, 0b011)?;
    let fill = reader.read(1)? != 0;
    reader.read(1)?; // position of grant
    let length = reader.read(6)? as u16;
    if length == 0 {
        return Err(BitError::InvalidValue.into());
    }
    if reader.read(1)? != 0 {
        reader.grant()?;
    }
    if reader.read(1)? != 0 {
        reader.allocation(0)?;
    }
    Ok(payload(bits, reader.pos, Some(length * 8), fill)?)
}

#[derive(Debug, Clone, Copy)]
pub struct Sync {
    pub colour_code: u8,
    pub time: TdmaTime,
    pub sdu_offset: u16,
}

pub fn sync(bits: BitView<'_>) -> Result<Sync, DecodeError> {
    let mut reader = Reader { bits, pos: 0 };
    reader.read(4)?; // system code
    let colour_code = reader.read(6)? as u8;
    let time = TdmaTime {
        t: reader.read(2)? as u8 + 1,
        f: reader.read(5)? as u8,
        m: reader.read(6)? as u8,
        h: 0,
    };
    reader.read(8)?; // sharing, reserved frames, DTX, frame-18 extension, reserved
    if !time.is_valid() {
        return Err(BitError::InvalidValue.into());
    }
    Ok(Sync {
        colour_code,
        time,
        sdu_offset: reader.pos,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PduKind {
    Resource,
    Fragment,
    End,
    Sysinfo,
    AccessDefine,
}

pub fn kind(bits: BitView<'_>, stolen_channel: bool) -> Result<PduKind, DecodeError> {
    match bits.read(0, 2)? {
        0 => Ok(PduKind::Resource),
        1 if bits.read(2, 1)? == 0 => Ok(PduKind::Fragment),
        1 => Ok(PduKind::End),
        2 => match bits.read(2, 2)? {
            0 => Ok(PduKind::Sysinfo),
            1 => Ok(PduKind::AccessDefine),
            _ => Err(DecodeError::Unsupported(Unsupported::Broadcast)),
        },
        _ => Err(DecodeError::Unsupported(if stolen_channel {
            Unsupported::USignal
        } else {
            Unsupported::Supplementary
        })),
    }
}

#[derive(Debug)]
pub struct Aach {
    pub traffic_marker: Option<u8>,
    pub access: Option<SlotAccessAssign>,
}

pub fn aach(bits: BitView<'_>, frame18: bool) -> Result<Aach, DecodeError> {
    let mut reader = Reader { bits, pos: 0 };
    let header = reader.read(2)?;
    let first = reader.read(6)? as u8;
    let second = reader.read(6)? as u8;
    if frame18 {
        if header == 3 && first < 4 {
            return Err(BitError::InvalidValue.into());
        }
        return Ok(Aach {
            traffic_marker: None,
            access: None,
        });
    }
    if header == 3 && (1..=3).contains(&second) {
        return Err(BitError::InvalidValue.into());
    }
    let field = |value: u8| AccessField {
        access_code: value >> 4,
        base_frame_len: value & 15,
    };
    let assignment = random_access::AccessAssign {
        ul_usage: if header == 2 {
            random_access::AccessAssignUlUsage::AssignedOnly
        } else {
            random_access::AccessAssignUlUsage::Other
        },
        f1_af1: (header == 0).then(|| field(first)),
        f2_af2: (header == 0).then(|| field(second)),
        f2_af: matches!(header, 1 | 2).then(|| field(second)),
    };
    Ok(Aach {
        traffic_marker: (header != 0 && first >= 4).then_some(first),
        access: Some(random_access::interpret_access_assign(&assignment, true)),
    })
}

#[derive(Debug)]
pub struct AccessDefinition {
    pub assigned_control: bool,
    pub code: AccessCode,
    pub params: RandomAccessParams,
}

impl AccessDefinition {
    pub fn for_common_channel(self) -> Option<(AccessCode, RandomAccessParams)> {
        (!self.assigned_control).then_some((self.code, self.params))
    }
}

fn access_params(reader: &mut Reader<'_>) -> Result<RandomAccessParams, DecodeError> {
    Ok(RandomAccessParams {
        imm: reader.read(4)? as u8,
        wt: reader.read(4)? as u8,
        nu: reader.read(4)? as u8,
        fl_factor: reader.read(1)? != 0,
        ts_ptr: reader.read(4)? as u8,
        min_pdu_prio: reader.read(3)? as u8,
        subscriber_class: None,
        gssi: None,
    })
}

pub fn access_define(bits: BitView<'_>) -> Result<AccessDefinition, DecodeError> {
    let mut reader = Reader { bits, pos: 0 };
    reader.expect(4, 0b1001)?;
    let assigned_control = reader.read(1)? != 0;
    let code = AccessCode::from_raw(reader.read(2)? as u8).ok_or(BitError::InvalidValue)?;
    let mut params = access_params(&mut reader)?;
    match reader.read(2)? {
        1 => params.subscriber_class = Some(reader.read(16)? as u16),
        2 => params.gssi = Some(reader.read(24)?),
        _ => {}
    }
    reader.expect(3, 0b100)?;
    Ok(AccessDefinition {
        assigned_control,
        code,
        params,
    })
}

#[derive(Debug)]
pub struct Sysinfo {
    pub main_carrier: u16,
    pub freq_band: u8,
    pub freq_offset_index: u8,
    pub duplex_spacing: u8,
    pub reverse_operation: bool,
    pub hyperframe: Option<u16>,
    pub default_access: Option<RandomAccessParams>,
    pub option: u8,
    pub option_bits: u32,
    pub sdu_offset: u16,
}

pub fn sysinfo(bits: BitView<'_>) -> Result<Sysinfo, DecodeError> {
    let mut reader = Reader { bits, pos: 0 };
    reader.expect(4, 0b1000)?;
    let main_carrier = reader.read(12)? as u16;
    let freq_band = reader.read(4)? as u8;
    let freq_offset_index = reader.read(2)? as u8;
    let duplex_spacing = reader.read(3)? as u8;
    let reverse_operation = reader.read(1)? != 0;
    reader.read(17)?; // CSCH count, cell power/access thresholds and timeout
    let cck_field = reader.read(1)? != 0;
    let number = reader.read(16)? as u16;
    let option = reader.read(2)? as u8;
    let option_bits = reader.bits.read(reader.pos, 20)?;
    let default_access = if option == 2 {
        Some(access_params(&mut reader)?)
    } else {
        reader.read(20)?;
        None
    };
    Ok(Sysinfo {
        main_carrier,
        freq_band,
        freq_offset_index,
        duplex_spacing,
        reverse_operation,
        hyperframe: (!cck_field).then_some(number),
        default_access,
        option,
        option_bits,
        sdu_offset: reader.pos,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_underdeclared_truncated_and_missing_fill_are_explicit() {
        let bits = BitView::new(&[0; 34], 268).unwrap();
        let short = payload(bits, 51, Some(40), true).unwrap();
        assert_eq!((short.start, short.end, short.next, short.fill), (51, 51, 40, 0));
        let long = payload(bits, 43, Some(272), false).unwrap();
        assert_eq!((long.end, long.next, long.truncated), (268, 268, true));
        let no_marker = payload(bits, 43, Some(80), true).unwrap();
        assert_eq!((no_marker.end, no_marker.fill), (80, 0));
        assert_eq!(payload(bits, 269, None, false), Err(BitError::Truncated));
    }

    #[test]
    fn arbitrary_truncated_headers_never_panic() {
        let mut bytes = [0; 34];
        let mut random = 0x1234_5678_u32;
        for _ in 0..32 {
            for byte in &mut bytes {
                random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                *byte = (random >> 24) as u8;
            }
            for length in 0..=268 {
                let bits = BitView::new(&bytes, length).unwrap();
                let _ = resource(bits).and_then(|header| header.payload(bits).map_err(DecodeError::from));
                let _ = fragment(bits);
                let _ = end(bits);
                let _ = sync(bits);
                let _ = kind(bits, false);
                let _ = aach(bits, false);
                let _ = aach(bits, true);
                let _ = access_define(bits);
                let _ = sysinfo(bits);
            }
        }
    }
}
