use tetra_core::BitBuffer;
use tetra_ms_types::BitError;

/// Compute FCS checksum for a range of bits in a BitBuffer
/// Offsets are relative to the bitbuffer window start.
pub fn compute_fcs(bitbuf: &BitBuffer, start: usize, end: usize) -> Result<u32, BitError> {
    let len = end.checked_sub(start).ok_or(BitError::InvalidValue)?;
    let start = u16::try_from(start).map_err(|_| BitError::Capacity)?;
    let len = u16::try_from(len).map_err(|_| BitError::Capacity)?;
    tetra_ms_core::fcs::compute(bitbuf.as_bit_view()?.window(start, len)?)
}

/// Computes and checks the FCS checksum
/// Computes over bitbuffer range [pos, end-32]. Checks with FCS at [end - 32, end]
pub fn check_fcs(bitbuf: &BitBuffer) -> bool {
    let check = || {
        let start = u16::try_from(bitbuf.get_pos()).map_err(|_| BitError::Capacity)?;
        let len = u16::try_from(bitbuf.get_len_remaining()).map_err(|_| BitError::Capacity)?;
        tetra_ms_core::fcs::check(bitbuf.as_bit_view()?.window(start, len)?)
    };
    match check() {
        Ok(valid) => valid,
        Err(error) => {
            tracing::warn!(?error, "LLC FCS validation failed");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use tetra_pdus::llc::pdus::bl_data::BlData;

    use super::*;

    #[test]
    fn fcs_test() {
        let testvec = "010100100111101011010111110000100110000110001011000011000000000000000011000100000001001100110011000000110010001011000011001000110000001100100011000100110001001100010011000100110101001100100011000000110010001100000011000000110001011001111010000010101011000110101";
        let mut bitbuf = BitBuffer::from_bitstr(testvec);
        bitbuf.seek(5);
        let fcs = compute_fcs(&bitbuf, 5, 5 + 224).unwrap();
        let extracted_fcs = bitbuf.peek_bits_startoffset(5 + 224, 32).unwrap() as u32;
        assert_eq!(fcs, extracted_fcs);
    }

    #[test]
    fn bldata_with_fcs() {
        let testvec = "010100100111101011010111110000100110000110001011000011000000000000000011000100000001001100110011000000110010001011000011001000110000001100100011000100110001001100010011000100110101001100100011000000110010001100000011000000110001011001111010000010101011000110101";
        let mut bitbuf = BitBuffer::from_bitstr(testvec);
        let pdu = BlData::from_bitbuf(&mut bitbuf).expect("Failed to parse BL-DATA PDU");
        assert!(pdu.has_fcs, "PDU should have FCS");
        let fcs_ok = check_fcs(&bitbuf);
        assert!(fcs_ok, "FCS check failed");
    }
}
