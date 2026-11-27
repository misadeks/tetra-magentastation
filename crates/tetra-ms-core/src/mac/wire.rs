//! Uplink MAC type-1 blocks (TS 100 392-2, clauses 21.4 and 23.4).
use crate::types::{BitError, BitStorage, BitView, FragmentBits};

pub type MacBlock = BitStorage<34>;
pub const SCH_HU_BITS: u16 = 92;
pub const SCH_F_BITS: u16 = 268;
pub const STCH_BITS: u16 = 124;
pub const END_HU_SDU_BITS: u16 = 81;
pub const END_UL_SDU_BITS: u16 = 254;
pub const FRAG_UL_SDU_BITS: u16 = 264;
pub const MAX_UL_FRAG_SLOTS: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FragEndKind {
    MacEndHu,
    MacEndUl,
    MacFragUl { total_slots: u8 },
}

fn zeroes(block: &mut MacBlock, mut count: u16) -> Result<(), BitError> {
    while count != 0 {
        let n = count.min(32) as u8;
        block.push(n, 0)?;
        count -= u16::from(n);
    }
    Ok(())
}

fn finish(block: &mut MacBlock, fill: u16, capacity: u16) -> Result<(), BitError> {
    if fill != 0 {
        block.push(1, 1)?;
        zeroes(block, fill - 1)?;
    }
    zeroes(block, capacity - block.len())
}

/// Complete MAC-ACCESS, without a length indication.
pub fn access(issi: u32, sdu: BitView<'_>) -> Result<MacBlock, BitError> {
    let content = 30_u16.checked_add(sdu.len()).ok_or(BitError::Capacity)?;
    if content > SCH_HU_BITS {
        return Err(BitError::Capacity);
    }
    let fill = SCH_HU_BITS - content;
    let mut block = MacBlock::new();
    block.push(1, 0)?;
    block.push(1, u32::from(fill != 0))?;
    block.push(3, 0)?; // unencrypted ISSI
    block.push(24, issi)?;
    block.push(1, 0)?; // no optional field
    block.append(sdu)?;
    finish(&mut block, fill, SCH_HU_BITS)?;
    Ok(block)
}

/// MAC-DATA on a stolen half-slot, with an octet length indication.
pub fn stolen_data(issi: u32, sdu: BitView<'_>) -> Result<MacBlock, BitError> {
    if sdu.len() > STCH_BITS - 37 {
        return Err(BitError::Capacity);
    }
    let content = 37_u16.checked_add(sdu.len()).ok_or(BitError::Capacity)?;
    let total = content.div_ceil(8) * 8;
    if total > STCH_BITS {
        return Err(BitError::Capacity);
    }
    let fill = total - content;
    let mut block = MacBlock::new();
    block.push(2, 0)?;
    block.push(1, u32::from(fill != 0))?;
    block.push(3, 0)?;
    block.push(24, issi)?;
    block.push(1, 0)?;
    block.push(6, u32::from(total / 8))?;
    block.append(sdu)?;
    finish(&mut block, fill, STCH_BITS)?;
    Ok(block)
}

pub fn fragment_slots(remainder: u16) -> Result<u8, BitError> {
    if remainder <= END_UL_SDU_BITS {
        return Err(BitError::InvalidValue);
    }
    let slots = (remainder - END_UL_SDU_BITS).div_ceil(FRAG_UL_SDU_BITS) + 1;
    if usize::from(slots) > MAX_UL_FRAG_SLOTS {
        Err(BitError::Capacity)
    } else {
        Ok(slots as u8)
    }
}

pub fn access_fragment(issi: u32, sdu: BitView<'_>) -> Result<(MacBlock, FragmentBits, FragEndKind), BitError> {
    const FIRST: u16 = SCH_HU_BITS - 36;
    if sdu.len() <= FIRST {
        return Err(BitError::InvalidValue);
    }
    let remaining = sdu.len() - FIRST;
    let (reservation, kind) = if remaining <= END_HU_SDU_BITS {
        (0, FragEndKind::MacEndHu)
    } else if remaining <= END_UL_SDU_BITS {
        (1, FragEndKind::MacEndUl)
    } else {
        let total_slots = fragment_slots(remaining)?;
        FragmentBlocks::build(sdu.window(FIRST, remaining)?, total_slots)?;
        (u32::from(total_slots), FragEndKind::MacFragUl { total_slots })
    };
    let mut block = MacBlock::new();
    block.push(5, 0)?; // MAC-ACCESS, no fill, unencrypted ISSI
    block.push(24, issi)?;
    block.push(3, 0b111)?; // optional capacity request, fragmentation start
    block.push(4, reservation)?;
    block.append(sdu.window(0, FIRST)?)?;
    let remainder = FragmentBits::from_view(sdu.window(FIRST, remaining)?)?;
    Ok((block, remainder, kind))
}

/// MAC-FRAG-UL, with a fill delimiter for a short continuation (21.4.2.4).
pub fn fragment(sdu: BitView<'_>) -> Result<MacBlock, BitError> {
    if sdu.is_empty() || sdu.len() > FRAG_UL_SDU_BITS {
        return Err(BitError::InvalidValue);
    }
    let fill = FRAG_UL_SDU_BITS - sdu.len();
    let mut block = MacBlock::new();
    block.push(3, 0b010)?;
    block.push(1, u32::from(fill != 0))?;
    block.append(sdu)?;
    finish(&mut block, fill, SCH_F_BITS)?;
    Ok(block)
}

pub fn end_hu(sdu: BitView<'_>) -> Result<MacBlock, BitError> {
    end(sdu, true)
}

pub fn end_ul(sdu: BitView<'_>) -> Result<MacBlock, BitError> {
    end(sdu, false)
}

fn end(sdu: BitView<'_>, half: bool) -> Result<MacBlock, BitError> {
    let (header, capacity) = if half { (7, SCH_HU_BITS) } else { (10, SCH_F_BITS) };
    if sdu.len() > capacity - header {
        return Err(BitError::Capacity);
    }
    let content = sdu.len().checked_add(header).ok_or(BitError::Capacity)?;
    let total = content.div_ceil(8) * 8;
    if total > capacity {
        return Err(BitError::Capacity);
    }
    let fill = total - content;
    let mut block = MacBlock::new();
    if half {
        block.push(1, 1)?;
        block.push(1, u32::from(fill != 0))?;
        block.push(1, 0)?;
        block.push(4, u32::from(total / 8))?;
    } else {
        block.push(3, 0b011)?;
        block.push(1, u32::from(fill != 0))?;
        block.push(6, u32::from(total / 8))?;
    }
    block.append(sdu)?;
    finish(&mut block, fill, capacity)?;
    Ok(block)
}

/// Bounded, ordered full-slot fragments. Short continuations use fill so the
/// final MAC-END always contains data. Rejected sizes never yield a partial plan.
#[derive(Debug, Clone)]
pub struct FragmentBlocks {
    blocks: [MacBlock; MAX_UL_FRAG_SLOTS],
    len: u8,
}

impl FragmentBlocks {
    pub fn build(remainder: BitView<'_>, slots: u8) -> Result<Self, BitError> {
        let count = usize::from(slots);
        if !(1..=MAX_UL_FRAG_SLOTS).contains(&count) {
            return Err(BitError::Capacity);
        }
        let expected = if remainder.len() <= END_UL_SDU_BITS {
            1
        } else {
            fragment_slots(remainder.len())?
        };
        if slots != expected || remainder.is_empty() {
            return Err(BitError::InvalidValue);
        }
        let mut blocks = core::array::from_fn(|_| MacBlock::new());
        let mut prefix = 0;
        for (i, block) in blocks[..count - 1].iter_mut().enumerate() {
            // MAC-FRAG permits fill (21.4.2.4). Keep at least one SDU bit
            // for MAC-END rather than creating a zero/negative final tail.
            let len = if i == count - 2 {
                (remainder.len() - prefix - 1).min(FRAG_UL_SDU_BITS)
            } else {
                FRAG_UL_SDU_BITS
            };
            *block = fragment(remainder.window(prefix, len)?)?;
            prefix += len;
        }
        let tail = remainder.len() - prefix;
        blocks[count - 1] = end_ul(remainder.window(prefix, tail)?)?;
        Ok(Self { blocks, len: slots })
    }

    pub fn as_slice(&self) -> &[MacBlock] {
        &self.blocks[..usize::from(self.len)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(block: BitView<'_>, header: u16, end: u16, fill_offset: u16) -> BitView<'_> {
        let mut stop = end;
        if block.read(fill_offset, 1).unwrap() != 0 {
            while block.read(stop - 1, 1).unwrap() == 0 {
                stop -= 1;
            }
            stop -= 1; // fill delimiter
        }
        block.window(header, stop - header).unwrap()
    }

    #[test]
    fn every_fragmented_sdu_length_roundtrips_with_consistent_reservations() {
        // Independent field widths from MAC-ACCESS / MAC-FRAG / MAC-END
        // (21.4.3.1, 21.4.2.4, 21.4.2.5), including all ten-bit holes.
        let bytes = [0xa5; 204];
        for len in 57..=1630 {
            let source = BitView::new(&bytes, len).unwrap();
            let (first, remainder, kind) = access_fragment(0x123456, source).unwrap();
            assert_eq!(first.view().read(0, 5), Ok(0));
            assert_eq!(first.view().read(5, 24), Ok(0x123456));
            assert_eq!(first.view().read(29, 3), Ok(7));
            let mut recovered = FragmentBits::new();
            recovered.append(first.view().window(36, 56).unwrap()).unwrap();
            let reservation = first.view().read(32, 4).unwrap();
            match kind {
                FragEndKind::MacEndHu => {
                    assert_eq!(reservation, 0);
                    let end = end_hu(remainder.view()).unwrap();
                    assert_eq!(end.view().read(0, 1), Ok(1));
                    let length = end.view().read(3, 4).unwrap() as u16 * 8;
                    recovered.append(payload(end.view(), 7, length, 1)).unwrap();
                }
                FragEndKind::MacEndUl => {
                    assert_eq!(reservation, 1);
                    let end = end_ul(remainder.view()).unwrap();
                    assert_eq!(end.view().read(0, 3), Ok(3));
                    let length = end.view().read(4, 6).unwrap() as u16 * 8;
                    recovered.append(payload(end.view(), 10, length, 3)).unwrap();
                }
                FragEndKind::MacFragUl { total_slots } => {
                    assert_eq!(reservation, u32::from(total_slots));
                    let plan = FragmentBlocks::build(remainder.view(), total_slots).unwrap();
                    assert_eq!(plan.as_slice().len(), usize::from(total_slots));
                    for (index, block) in plan.as_slice().iter().enumerate() {
                        let last = index == usize::from(total_slots) - 1;
                        assert_eq!(block.len(), 268);
                        assert_eq!(block.view().read(0, 3), Ok(if last { 3 } else { 2 }));
                        let (header, end) = if last {
                            (10, block.view().read(4, 6).unwrap() as u16 * 8)
                        } else {
                            (4, 268)
                        };
                        recovered.append(payload(block.view(), header, end, 3)).unwrap();
                    }
                }
            }
            assert_eq!(recovered.len(), len, "SDU length {len}");
            for offset in 0..len {
                assert_eq!(recovered.view().read(offset, 1), source.read(offset, 1), "SDU {len}, bit {offset}");
            }
        }
        assert_eq!(
            access_fragment(1, BitView::new(&bytes, 1631).unwrap()).err(),
            Some(BitError::Capacity)
        );
    }

    #[test]
    fn short_fragment_fill_vector_and_invalid_plans_are_atomic() {
        let bytes = [0xa5; 33];
        let fragment = fragment(BitView::new(&bytes, 263).unwrap()).unwrap();
        assert_eq!(fragment.view().read(0, 8), Ok(0x5a));
        assert_eq!(fragment.view().read(267, 1), Ok(1)); // one-bit fill
        for len in [255, 264, 519, 528] {
            let bytes = [0x80; 66];
            let bits = BitView::new(&bytes, len).unwrap();
            let slots = fragment_slots(len).unwrap();
            assert!(FragmentBlocks::build(bits, slots).is_ok());
            assert!(FragmentBlocks::build(bits, slots - 1).is_err());
            assert!(FragmentBlocks::build(bits, slots + 1).is_err());
        }
    }
}
