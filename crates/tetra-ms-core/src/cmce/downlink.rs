//! Allocation-free relocation of the MS downlink PDU decoders (14.7.1).
use super::{BasicService, CmceError};
use tetra_ms_types::BitView;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub offset: u16,
    pub len: u16,
}
impl Span {
    pub fn view(self, bits: BitView<'_>) -> Result<BitView<'_>, CmceError> {
        Ok(bits.window(self.offset, self.len)?)
    }
}

/// Scalar fields plus offsets into the original SDU for variable-length IEs.
/// Ignored optional fields are still parsed and validated in their original order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Downlink {
    pub kind: u8,
    pub id: u16,
    pub timeout: Option<u8>,
    pub setup_timeout: Option<u8>,
    pub hook: bool,
    pub duplex: bool,
    pub basic: Option<BasicService>,
    pub grant: u8,
    pub permission_denied: bool,
    pub priority: Option<u8>,
    pub ownership: Option<bool>,
    pub speaker: Option<u32>,
    pub extension: Option<u32>,
    pub notification: Option<u8>,
    pub call_status: Option<u8>,
    pub new_id: Option<u16>,
    pub reset: bool,
    pub poll: bool,
    pub poll_percentage: Option<u8>,
    pub poll_number: Option<u8>,
    pub queued: bool,
    pub resume: bool,
    pub encrypted: bool,
    pub cause: u8,
    pub external: Option<Span>,
    pub dtmf: Option<Span>,
}

struct Reader<'a> {
    bits: BitView<'a>,
    pos: u16,
}
impl Reader<'_> {
    fn read(&mut self, width: u8) -> Result<u32, CmceError> {
        let value = self.bits.read(self.pos, width)?;
        self.pos += u16::from(width);
        Ok(value)
    }
    fn flag(&mut self) -> Result<bool, CmceError> {
        Ok(self.read(1)? != 0)
    }
    fn optional(&mut self, present: bool, width: u8) -> Result<Option<u32>, CmceError> {
        if present && self.flag()? {
            Ok(Some(self.read(width)?))
        } else {
            Ok(None)
        }
    }
    fn small(&mut self, present: bool, width: u8) -> Result<Option<u8>, CmceError> {
        Ok(self.optional(present, width)?.map(|value| value as u8))
    }
    fn basic(&mut self) -> Result<BasicService, CmceError> {
        Ok(BasicService {
            circuit_mode: self.read(3)? as u8,
            encrypted: self.flag()?,
            communication: self.read(2)? as u8,
            parameter: self.read(2)? as u8,
        })
    }
    fn optional_basic(&mut self, present: bool) -> Result<Option<BasicService>, CmceError> {
        if present && self.flag()? {
            Ok(Some(self.basic()?))
        } else {
            Ok(None)
        }
    }
    fn field(&mut self, present: bool, expected: u32) -> Result<Option<Span>, CmceError> {
        if !present || self.bits.read(self.pos, 1)? == 0 {
            return Ok(None);
        }
        if self.bits.read(self.pos + 1, 4)? != expected {
            return Ok(None);
        }
        self.read(5)?;
        let len = self.read(11)? as u16;
        let result = Span { offset: self.pos, len };
        self.bits.window(self.pos, len)?;
        self.pos += len;
        Ok(Some(result))
    }
    fn end(&mut self, present: bool) -> Result<(), CmceError> {
        if present && self.flag()? {
            return Err(CmceError::InvalidValue);
        }
        Ok(())
    }
}

pub fn decode(bits: BitView<'_>) -> Result<Downlink, CmceError> {
    let mut r = Reader { bits, pos: 0 };
    let kind = r.read(5)? as u8;
    if !matches!(kind, 0..=7 | 9..=14) {
        return Err(CmceError::Unsupported);
    }
    let mut p = Downlink {
        kind,
        id: r.read(14)? as u16,
        ..Downlink::default()
    };
    match kind {
        7 => {
            p.timeout = Some(r.read(4)? as u8);
            p.hook = r.flag()?;
            p.duplex = r.flag()?;
            p.basic = Some(r.basic()?);
            p.grant = r.read(2)? as u8;
            p.permission_denied = r.flag()?;
            p.priority = Some(r.read(4)? as u8);
        }
        0 | 1 => {
            p.setup_timeout = Some(r.read(3)? as u8);
            p.hook = r.flag()?; // D-ALERT's reserved bit is not interpreted.
            p.duplex = r.flag()?;
            if kind == 0 {
                p.queued = r.flag()?;
            }
        }
        2 => {
            p.timeout = Some(r.read(4)? as u8);
            p.hook = r.flag()?;
            p.duplex = r.flag()?;
            p.grant = r.read(2)? as u8;
            p.permission_denied = r.flag()?;
            p.ownership = Some(r.flag()?);
        }
        3 => {
            p.timeout = Some(r.read(4)? as u8);
            p.grant = r.read(2)? as u8;
            p.permission_denied = r.flag()?;
        }
        4 | 6 => {
            p.cause = r.read(5)? as u8;
            if p.cause > 23 {
                return Err(CmceError::InvalidValue);
            }
        }
        5 => {
            p.reset = r.flag()?;
            p.poll = r.flag()?;
        }
        9 | 12 => p.permission_denied = r.flag()?,
        10 => {
            p.resume = r.flag()?;
            p.permission_denied = r.flag()?;
        }
        11 | 13 => {
            p.grant = r.read(2)? as u8;
            p.permission_denied = r.flag()?;
            p.encrypted = r.flag()?;
            r.read(1)?;
        }
        14 => {
            p.grant = r.read(2)? as u8;
            p.permission_denied = r.flag()?;
            p.reset = r.flag()?;
        }
        _ => return Err(CmceError::Unsupported),
    }
    let o = r.flag()?;
    match kind {
        7 => {
            p.notification = r.small(o, 6)?;
            r.optional(o, 24)?;
            let party = r.optional(o, 2)?;
            if matches!(party, Some(1 | 2)) {
                p.speaker = Some(r.read(24)?);
            }
            if party == Some(2) {
                p.extension = Some(r.read(24)?);
            }
        }
        0 | 1 => {
            p.basic = r.optional_basic(o)?;
            if kind == 1 {
                p.call_status = r.small(o, 3)?;
                if p.call_status.is_some_and(|value| value > 4) {
                    return Err(CmceError::InvalidValue);
                }
            }
            p.notification = r.small(o, 6)?;
        }
        2 => {
            p.priority = r.small(o, 4)?;
            p.basic = r.optional_basic(o)?;
            r.optional(o, 24)?;
            p.notification = r.small(o, 6)?;
        }
        3 | 4 | 6 | 9 | 10 | 12 => p.notification = r.small(o, 6)?,
        5 => {
            p.new_id = r.optional(o, 14)?.map(|value| value as u16);
            p.timeout = r.small(o, 4)?;
            p.setup_timeout = r.small(o, 3)?;
            p.ownership = r.optional(o, 1)?.map(|value| value != 0);
            r.optional(o, 9)?;
            p.call_status = r.small(o, 3)?;
            r.optional(o, 24)?;
            p.notification = r.small(o, 6)?;
            p.poll_percentage = r.small(o, 6)?;
            p.poll_number = r.small(o, 6)?;
        }
        11 => {
            p.notification = r.small(o, 6)?;
            let party = r.optional(o, 2)?;
            if matches!(party, Some(1 | 2)) {
                p.speaker = Some(r.read(24)?);
            }
            if party == Some(2) {
                p.extension = Some(r.read(24)?);
            }
        }
        13 => {
            p.notification = r.small(o, 6)?;
            r.optional(o, 2)?;
            // The authoritative D-TX-INTERRUPT uses independently present
            // Type-2 address fields, unlike D-TX-GRANTED's conditional address.
            p.speaker = r.optional(o, 24)?;
            p.extension = r.optional(o, 24)?;
        }
        14 => {
            p.new_id = r.optional(o, 14)?.map(|value| value as u16);
            p.timeout = r.small(o, 4)?;
            p.call_status = r.small(o, 3)?;
            r.optional(o, 9)?;
            p.notification = r.small(o, 6)?;
        }
        _ => {}
    }
    if kind == 5 {
        p.dtmf = r.field(o, 1)?;
    }
    if matches!(kind, 7 | 11 | 13) {
        p.external = r.field(o, 2)?;
    }
    r.field(o, 3)?;
    if kind == 5 {
        r.field(o, 4)?;
    }
    if kind == 14 {
        r.field(o, 5)?;
    }
    if matches!(kind, 7 | 9..=14) {
        r.field(o, 6)?;
    }
    r.field(o, 15)?;
    r.end(o)?;
    Ok(p)
}
