//! MM wire layouts shared with the application PDU representations.

use super::{Config, Error, GROUP_CLASS_OF_USAGE, GroupList, GroupOperation, GroupRequest, GroupSet, LocationUpdateType, MAX_GROUPS};
use tetra_ms_types::bits::{BitView, PackedBits};

pub(super) enum Downlink<'a> {
    Accept {
        temporary: bool,
        groups: Option<GroupElements<'a>>,
    },
    Command {
        report: bool,
        mni: Option<u32>,
    },
    Reject {
        cause: u8,
    },
    GroupAck {
        groups: GroupElements<'a>,
    },
}

/// Validated, borrowed repetitions. Do not copy two complete group arrays through
/// the decoder and dispatcher before the procedure actually needs their values.
pub(super) struct GroupElements<'a> {
    bits: Option<BitView<'a>>,
}

impl<'a> GroupElements<'a> {
    fn empty() -> Self {
        Self { bits: None }
    }

    fn parse(element: Reader<'a>) -> Result<Self, Error> {
        let groups = Self { bits: Some(element.bits) };
        groups.visit(|_, _| Ok(()))?;
        Ok(groups)
    }

    pub(super) fn attachments(&self, attached: &mut GroupList) -> Result<(), Error> {
        self.visit(|detachment, gssi| if detachment { Ok(()) } else { attached.push(gssi) })
    }

    pub(super) fn collect(&self, attached: &mut GroupList, detached: &mut GroupList) -> Result<(), Error> {
        self.visit(|detachment, gssi| if detachment { detached.push(gssi) } else { attached.push(gssi) })
    }

    fn visit(&self, mut consume: impl FnMut(bool, u32) -> Result<(), Error>) -> Result<(), Error> {
        let Some(bits) = self.bits else { return Ok(()) };
        let mut element = Reader::new(bits);
        let count = element.read(6)?;
        for _ in 0..count {
            let detachment = element.read(1)? != 0;
            element.skip(if detachment { 2 } else { 5 })?;
            let address_type = element.read(2)?;
            let gssi = if address_type != 2 { Some(element.read(24)?) } else { None };
            if address_type == 1 || address_type == 3 {
                element.skip(24)?;
            }
            if address_type == 2 || address_type == 3 {
                element.skip(24)?;
            }
            if let Some(gssi) = gssi {
                consume(detachment, gssi)?;
            }
        }
        element.exact_end()
    }
}

struct Reader<'a> {
    bits: BitView<'a>,
    pos: u16,
}

impl<'a> Reader<'a> {
    fn new(bits: BitView<'a>) -> Self {
        Self { bits, pos: 0 }
    }

    fn read(&mut self, width: u8) -> Result<u32, Error> {
        let value = self.bits.read(self.pos, width)?;
        self.pos += u16::from(width);
        Ok(value)
    }

    fn skip(&mut self, width: u16) -> Result<(), Error> {
        self.bits.window(self.pos, width)?;
        self.pos += width;
        Ok(())
    }

    fn type2(&mut self, obit: bool, width: u8) -> Result<Option<u32>, Error> {
        if obit && self.read(1)? != 0 {
            Ok(Some(self.read(width)?))
        } else {
            Ok(None)
        }
    }

    fn element(&mut self, obit: bool, id: u32) -> Result<Option<Reader<'a>>, Error> {
        if !obit || self.bits.read(self.pos, 1)? == 0 {
            return Ok(None);
        }
        if self.bits.read(self.pos + 1, 4)? != id {
            return Ok(None);
        }
        self.skip(5)?;
        let len = self.read(11)? as u16;
        let bits = self.bits.window(self.pos, len)?;
        self.pos += len;
        Ok(Some(Reader::new(bits)))
    }

    fn generic(&mut self, obit: bool, id: u32, repeated: bool) -> Result<(), Error> {
        if let Some(mut item) = self.element(obit, id)?
            && repeated
        {
            item.read(6)?;
        }
        Ok(())
    }

    fn end(&mut self, obit: bool) -> Result<(), Error> {
        if obit && self.read(1)? != 0 {
            return Err(Error::InvalidPdu);
        }
        Ok(())
    }

    fn exact_end(&self) -> Result<(), Error> {
        if self.pos == self.bits.len() {
            Ok(())
        } else {
            Err(Error::InvalidPdu)
        }
    }
}

#[inline(never)]
pub(super) fn decode(bits: BitView<'_>) -> Result<Downlink<'_>, Error> {
    let mut r = Reader::new(bits);
    match r.read(4)? as u8 {
        5 => {
            let temporary = r.read(3)? == 1;
            let obit = r.read(1)? != 0;
            for width in [24, 24, 16, 14, 6] {
                r.type2(obit, width)?;
            }
            r.generic(obit, 2, true)?;
            r.generic(obit, 3, false)?;
            let groups = if let Some(mut item) = r.element(obit, 5)? {
                item.skip(2)?;
                let nested_obit = item.read(1)? != 0;
                let result = if let Some(groups) = item.element(nested_obit, 7)? {
                    GroupElements::parse(groups)?
                } else {
                    GroupElements::empty()
                };
                item.end(nested_obit)?;
                item.exact_end()?;
                Some(result)
            } else {
                None
            };
            r.generic(obit, 1, false)?;
            r.generic(obit, 10, false)?;
            r.generic(obit, 12, true)?;
            r.generic(obit, 13, false)?;
            r.generic(obit, 15, false)?;
            r.end(obit)?;
            Ok(Downlink::Accept { temporary, groups })
        }
        6 | 7 => {
            let kind = bits.read(0, 4)?;
            let (report, cause) = if kind == 6 {
                (r.read(1)? != 0, 0)
            } else {
                r.skip(3)?;
                (false, r.read(5)? as u8)
            };
            if r.read(1)? != 0 {
                r.skip(10)?;
            }
            let obit = r.read(1)? != 0;
            let mni = r.type2(obit, 24)?;
            r.generic(obit, 13, false)?;
            r.generic(obit, 15, false)?;
            r.end(obit)?;
            if kind == 6 {
                Ok(Downlink::Command { report, mni })
            } else {
                Ok(Downlink::Reject { cause })
            }
        }
        11 => {
            r.skip(2)?;
            let obit = r.read(1)? != 0;
            r.generic(obit, 15, false)?;
            let groups = if let Some(groups) = r.element(obit, 7)? {
                GroupElements::parse(groups)?
            } else {
                GroupElements::empty()
            };
            r.generic(obit, 12, true)?;
            r.end(obit)?;
            Ok(Downlink::GroupAck { groups })
        }
        other => Err(Error::UnsupportedPdu(other)),
    }
}

fn element_header(bits: &mut PackedBits, id: u32, payload_len: u16) -> Result<(), Error> {
    if payload_len > 2047 {
        return Err(Error::GroupCapacity);
    }
    bits.push(1, 1)?;
    bits.push(4, id)?;
    bits.push(11, u32::from(payload_len))?;
    Ok(())
}

pub(super) fn groups(bits: &mut PackedBits, detach_all: bool, requests: &[GroupRequest]) -> Result<(), Error> {
    if requests.len() > MAX_GROUPS {
        return Err(Error::GroupCapacity);
    }
    // Six-bit count; attach entries are 1+3+2+24 bits, detach 1+2+2+24.
    let payload_len = 6 + requests
        .iter()
        .map(|request| match request.operation {
            GroupOperation::Attach { .. } => 30,
            GroupOperation::Detach { .. } => 29,
        })
        .sum::<u16>();
    bits.push(4, 7)?;
    bits.push(1, 0)?;
    bits.push(1, u32::from(detach_all))?;
    bits.push(1, 1)?;
    element_header(bits, 8, payload_len)?;
    bits.push(6, requests.len() as u32)?;
    for request in requests {
        if request.gssi > 0xff_ffff {
            return Err(Error::InvalidIdentity);
        }
        match request.operation {
            GroupOperation::Attach { class_of_usage } => {
                bits.push(1, 0)?;
                bits.push(3, u32::from(class_of_usage))?;
            }
            GroupOperation::Detach { reason } => {
                bits.push(1, 1)?;
                bits.push(2, u32::from(reason))?;
            }
        }
        bits.push(2, 0)?;
        bits.push(24, request.gssi)?;
    }
    bits.push(1, 0)?;
    Ok(())
}

fn attachments(bits: &mut PackedBits, groups: &[u32]) -> Result<(), Error> {
    if groups.len() > MAX_GROUPS {
        return Err(Error::GroupCapacity);
    }
    element_header(bits, 8, 6 + 30 * groups.len() as u16)?;
    bits.push(6, groups.len() as u32)?;
    for &gssi in groups {
        bits.push(1, 0)?;
        bits.push(3, u32::from(GROUP_CLASS_OF_USAGE))?;
        bits.push(2, 0)?;
        bits.push(24, gssi)?;
    }
    Ok(())
}

pub(super) fn attach_set(bits: &mut PackedBits, groups: &GroupSet) -> Result<(), Error> {
    bits.push(4, 7)?;
    bits.push(3, 0b001)?; // No report, amendment, optional group list present.
    attachments(bits, groups.as_slice())?;
    bits.push(1, 0)?;
    Ok(())
}

pub(super) fn location_update(
    bits: &mut PackedBits,
    lu: LocationUpdateType,
    identity: Option<Config>,
    groups: &[u32],
    report: bool,
) -> Result<(), Error> {
    bits.push(4, 2)?;
    bits.push(3, lu as u32)?;
    bits.push(2, 0)?; // No appended LA or cipher control.
    bits.push(1, u32::from(identity.is_some()))?;
    let Some(identity) = identity else { return Ok(()) };
    bits.push(1, 1)?;
    bits.push(24, 0x18_0060)?; // Original clear-mode class of MS, AI version 3.
    bits.push(2, 0)?; // No energy saving mode or LA information.
    bits.push(1, 1)?;
    bits.push(24, identity.issi)?;
    bits.push(1, 1)?;
    bits.push(24, u32::from(identity.mcc) << 14 | u32::from(identity.mnc))?;
    if !groups.is_empty() {
        if groups.len() > MAX_GROUPS {
            return Err(Error::GroupCapacity);
        }
        // Mandatory 3 bits + nested header 16 + count 6 + final m-bit.
        element_header(bits, 3, 26 + 30 * groups.len() as u16)?;
        bits.push(3, 0b011)?; // Reserved, detach-all mode, optional elements.
        attachments(bits, groups)?;
        bits.push(1, 0)?;
    }
    if report {
        element_header(bits, 4, 1)?;
        bits.push(1, 0)?;
    }
    bits.push(1, 0)?;
    Ok(())
}

pub(super) fn detach() -> Result<PackedBits, Error> {
    let mut bits = PackedBits::new();
    bits.push(4, 1)?;
    bits.push(1, 0)?;
    Ok(bits)
}
