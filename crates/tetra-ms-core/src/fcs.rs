use tetra_ms_types::{BitError, BitView};

/// BlueStation LLC FCS, including its initial short-SDU left alignment.
pub fn compute(bits: BitView<'_>) -> Result<u32, BitError> {
    let len = bits.len();
    let mut crc = if len == 0 {
        0
    } else if len < 32 {
        u32::MAX << (32 - len)
    } else {
        u32::MAX
    };
    for i in 0..len {
        let feedback = bits.read(i, 1)? ^ (crc >> 31);
        crc <<= 1;
        if feedback != 0 {
            crc ^= 0x04c1_1db7;
        }
    }
    Ok(!crc)
}

pub fn check(bits: BitView<'_>) -> Result<bool, BitError> {
    if bits.len() < 32 {
        return Err(BitError::Truncated);
    }
    let len = bits.len() - 32;
    Ok(compute(bits.window(0, len)?)? == bits.read(len, 32)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tetra_ms_types::PackedBits;

    #[test]
    fn original_captured_bldata_fcs() {
        let vector = b"010100100111101011010111110000100110000110001011000011000000000000000011000100000001001100110011000000110010001011000011001000110000001100100011000100110001001100010011000100110101001100100011000000110010001100000011000000110001011001111010000010101011000110101";
        let mut pdu = PackedBits::new();
        for bit in vector {
            pdu.push(1, u32::from(bit - b'0')).unwrap();
        }
        assert_eq!(check(pdu.view().window(5, 256).unwrap()), Ok(true));
        pdu.view_mut().write(16, 1, 0).unwrap();
        assert_eq!(check(pdu.view().window(5, 256).unwrap()), Ok(false));
    }

    #[test]
    fn truncated_and_empty_fcs_are_explicit() {
        assert_eq!(check(BitView::new(&[], 0).unwrap()), Err(BitError::Truncated));
        assert_eq!(compute(BitView::new(&[], 0).unwrap()), Ok(u32::MAX));
    }
}
