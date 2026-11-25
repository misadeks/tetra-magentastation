//! Checked MSB-first packed bits. Byte order never depends on the CPU.

pub const MAX_PDU_BITS: u16 = 2688;
pub const MAX_PDU_BYTES: u16 = MAX_PDU_BITS / 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitError {
    Truncated,
    Capacity,
    InvalidWidth,
    InvalidValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitView<'a> {
    bytes: &'a [u8],
    start: u16,
    len: u16,
}

impl<'a> BitView<'a> {
    pub fn new(bytes: &'a [u8], len: u16) -> Result<Self, BitError> {
        if usize::from(len).div_ceil(8) > bytes.len() {
            return Err(BitError::Truncated);
        }
        Ok(Self { bytes, start: 0, len })
    }

    pub const fn len(self) -> u16 {
        self.len
    }

    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    pub fn window(self, offset: u16, len: u16) -> Result<Self, BitError> {
        check_range(self.len, offset, len)?;
        Ok(Self {
            bytes: self.bytes,
            start: self.start + offset,
            len,
        })
    }

    #[inline]
    pub fn read(self, offset: u16, width: u8) -> Result<u32, BitError> {
        if width > 32 {
            return Err(BitError::InvalidWidth);
        }
        check_range(self.len, offset, u16::from(width))?;
        if width == 1 {
            let pos = self.start + offset;
            return Ok(u32::from((self.bytes[usize::from(pos / 8)] >> (7 - pos % 8)) & 1));
        }
        let mut value = 0;
        let mut pos = self.start + offset;
        let mut remaining = width;
        while remaining != 0 {
            let available = 8 - (pos % 8) as u8;
            let count = available.min(remaining);
            let shift = available - count;
            let mask = (1_u16 << count) - 1;
            value = (value << count) | ((u32::from(self.bytes[usize::from(pos / 8)]) >> shift) & u32::from(mask));
            pos += u16::from(count);
            remaining -= count;
        }
        Ok(value)
    }

    /// Reference bit-at-a-time path for equivalence tests and measurements.
    #[inline]
    pub fn read_reference(self, offset: u16, width: u8) -> Result<u32, BitError> {
        if width > 32 {
            return Err(BitError::InvalidWidth);
        }
        check_range(self.len, offset, u16::from(width))?;
        let mut value = 0;
        for i in 0..u16::from(width) {
            let pos = self.start + offset + i;
            value = (value << 1) | u32::from((self.bytes[usize::from(pos / 8)] >> (7 - pos % 8)) & 1);
        }
        Ok(value)
    }
}

#[derive(Debug)]
pub struct BitViewMut<'a> {
    bytes: &'a mut [u8],
    start: u16,
    len: u16,
}

impl<'a> BitViewMut<'a> {
    pub fn new(bytes: &'a mut [u8], len: u16) -> Result<Self, BitError> {
        if usize::from(len).div_ceil(8) > bytes.len() {
            return Err(BitError::Truncated);
        }
        Ok(Self { bytes, start: 0, len })
    }

    pub fn window(&mut self, offset: u16, len: u16) -> Result<BitViewMut<'_>, BitError> {
        check_range(self.len, offset, len)?;
        Ok(BitViewMut {
            bytes: self.bytes,
            start: self.start + offset,
            len,
        })
    }

    pub fn write(&mut self, offset: u16, width: u8, value: u32) -> Result<(), BitError> {
        if width > 32 {
            return Err(BitError::InvalidWidth);
        }
        check_range(self.len, offset, u16::from(width))?;
        if width < 32 && value >> width != 0 {
            return Err(BitError::InvalidValue);
        }
        for i in 0..u16::from(width) {
            let pos = self.start + offset + i;
            let mask = 1 << (7 - pos % 8);
            let target = &mut self.bytes[usize::from(pos / 8)];
            if value & (1 << (u16::from(width) - i - 1)) == 0 {
                *target &= !mask;
            } else {
                *target |= mask;
            }
        }
        Ok(())
    }

    pub fn xor(&mut self, source: BitView<'_>) -> Result<(), BitError> {
        if self.len != source.len() {
            return Err(BitError::InvalidValue);
        }
        for i in 0..self.len {
            let pos = self.start + i;
            if source.read(i, 1)? != 0 {
                self.bytes[usize::from(pos / 8)] ^= 1 << (7 - pos % 8);
            }
        }
        Ok(())
    }
}

fn check_range(len: u16, offset: u16, width: u16) -> Result<(), BitError> {
    if offset > len || width > len - offset {
        return Err(BitError::Truncated);
    }
    Ok(())
}

/// Bounded packed bytes, aligned for native 32-bit copies, plus a fixed-width bit length.
/// Unused bytes and unused low bits of the final byte are always zero.
#[derive(Debug, Clone, PartialEq, Eq)]
#[repr(align(4))]
pub struct BitStorage<const BYTES: usize> {
    bytes: [u8; BYTES],
    len: u16,
}

pub type PackedBits = BitStorage<{ MAX_PDU_BYTES as usize }>;
pub type FragmentBits = BitStorage<512>;

impl<const BYTES: usize> Default for BitStorage<BYTES> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const BYTES: usize> BitStorage<BYTES> {
    pub const fn new() -> Self {
        assert!(BYTES <= u16::MAX as usize / 8);
        Self { bytes: [0; BYTES], len: 0 }
    }

    pub fn from_view(view: BitView<'_>) -> Result<Self, BitError> {
        let mut bits = Self::new();
        bits.append(view)?;
        Ok(bits)
    }

    pub const fn len(&self) -> u16 {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len).div_ceil(8)]
    }

    pub fn view(&self) -> BitView<'_> {
        BitView {
            bytes: &self.bytes,
            start: 0,
            len: self.len,
        }
    }

    pub fn view_mut(&mut self) -> BitViewMut<'_> {
        BitViewMut {
            bytes: &mut self.bytes,
            start: 0,
            len: self.len,
        }
    }

    pub fn clear(&mut self) {
        self.bytes.fill(0);
        self.len = 0;
    }

    pub fn push(&mut self, width: u8, value: u32) -> Result<(), BitError> {
        if width > 32 {
            return Err(BitError::InvalidWidth);
        }
        if width < 32 && value >> width != 0 {
            return Err(BitError::InvalidValue);
        }
        if u16::from(width) > (BYTES * 8) as u16 - self.len {
            return Err(BitError::Capacity);
        }
        let len = self.len;
        BitViewMut {
            bytes: &mut self.bytes,
            start: len,
            len: u16::from(width),
        }
        .write(0, width, value)?;
        self.len += u16::from(width);
        Ok(())
    }

    pub fn append(&mut self, view: BitView<'_>) -> Result<(), BitError> {
        if view.len() > (BYTES * 8) as u16 - self.len {
            return Err(BitError::Capacity);
        }
        let mut offset = 0;
        while offset < view.len() {
            let width = (view.len() - offset).min(32) as u8;
            self.push(width, view.read(offset, width)?)?;
            offset += u16::from(width);
        }
        Ok(())
    }

    /// Explicit big-endian word conversion; no casts or unaligned accesses.
    pub fn write_words(&self, out: &mut [u32]) -> Result<u16, BitError> {
        let count = self.len.div_ceil(32);
        if out.len() < usize::from(count) {
            return Err(BitError::Capacity);
        }
        for i in 0..count {
            let offset = i * 32;
            let width = (self.len - offset).min(32) as u8;
            let value = self.view().read(offset, width)?;
            out[usize::from(i)] = if width == 32 { value } else { value << (32 - width) };
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_windows_and_word_order() {
        let bytes = [0x12, 0x34, 0x56, 0x78, 0xa0];
        let view = BitView::new(&bytes, 36).unwrap();
        assert_eq!(view.read(0, 32), Ok(0x1234_5678));
        assert_eq!(view.window(4, 28).unwrap().read(0, 28), Ok(0x234_5678));
        let bits = PackedBits::from_view(view).unwrap();
        let mut words = [0; 2];
        assert_eq!(bits.write_words(&mut words), Ok(2));
        assert_eq!(words, [0x1234_5678, 0xa000_0000]);
        assert_eq!(bits.as_bytes(), bytes);
    }

    #[test]
    fn byte_path_matches_reference_at_every_offset_and_width() {
        let bytes = [0x12, 0xa5, 0xff, 0x00, 0x81, 0x3c, 0xee, 0x65];
        for start in 0..32 {
            let view = BitView::new(&bytes, 64).unwrap().window(start, 64 - start).unwrap();
            for offset in 0..=view.len() {
                for width in 0..=32 {
                    assert_eq!(view.read(offset, width), view.read_reference(offset, width));
                }
            }
        }
    }

    #[test]
    fn writes_and_xor_preserve_bits_outside_mutable_window() {
        let mut bytes = [0xff, 0xff];
        let mut all = BitViewMut::new(&mut bytes, 16).unwrap();
        all.window(3, 10).unwrap().write(0, 10, 0x12).unwrap();
        assert_eq!(bytes, [0xe0, 0x97]);
        let mask = [0xaa, 0x80];
        BitViewMut::new(&mut bytes, 16)
            .unwrap()
            .window(3, 9)
            .unwrap()
            .xor(BitView::new(&mask, 9).unwrap())
            .unwrap();
        assert_eq!(bytes, [0xf5, 0xc7]);
    }

    #[test]
    fn full_buffer_and_invalid_operations_are_atomic() {
        let mut bits = PackedBits::new();
        for _ in 0..MAX_PDU_BITS / 32 {
            bits.push(32, u32::MAX).unwrap();
        }
        let before = bits.clone();
        assert_eq!(bits.push(1, 1), Err(BitError::Capacity));
        assert_eq!(bits, before);
        assert_eq!(bits.view().read(u16::MAX, 1), Err(BitError::Truncated));
        assert_eq!(bits.view().read(0, 33), Err(BitError::InvalidWidth));
        assert_eq!(bits.view_mut().write(0, 1, 2), Err(BitError::InvalidValue));
        assert_eq!(bits, before);
        bits.clear();
        assert!(bits.is_empty());
        assert_eq!(core::mem::size_of::<PackedBits>(), 340);
        assert_eq!(core::mem::align_of::<PackedBits>(), 4);
    }
}
