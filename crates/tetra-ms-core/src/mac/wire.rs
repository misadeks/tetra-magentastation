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

pub fn fragment(sdu: BitView<'_>) -> Result<MacBlock, BitError> {
    if sdu.len() != FRAG_UL_SDU_BITS {
        return Err(BitError::InvalidValue);
    }
    let mut block = MacBlock::new();
    block.push(4, 0b0100)?;
    block.append(sdu)?;
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

/// Bounded, ordered full-slot fragments. Rejected sizes never yield a partial plan.
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
        let prefix = (slots - 1) as u16 * FRAG_UL_SDU_BITS;
        let tail = remainder.len().checked_sub(prefix).ok_or(BitError::InvalidValue)?;
        if tail == 0 || tail > END_UL_SDU_BITS {
            return Err(BitError::InvalidValue);
        }
        let mut blocks = core::array::from_fn(|_| MacBlock::new());
        for (i, block) in blocks[..count - 1].iter_mut().enumerate() {
            *block = fragment(remainder.window(i as u16 * FRAG_UL_SDU_BITS, FRAG_UL_SDU_BITS)?)?;
        }
        blocks[count - 1] = end_ul(remainder.window(prefix, tail)?)?;
        Ok(Self { blocks, len: slots })
    }

    pub fn as_slice(&self) -> &[MacBlock] {
        &self.blocks[..usize::from(self.len)]
    }
}
