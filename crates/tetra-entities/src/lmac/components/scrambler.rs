use tetra_core::BitBuffer;

pub use tetra_phy_core::scrambler::{SCRAMB_INIT, Scrambler, tetra_scramb_get_bits, tetra_scramb_get_init};

/// XOR the remaining window and restore the original cursor.
pub fn tetra_scramb_bits(seed: u32, buf: &mut BitBuffer) {
    let num_bits = buf.get_len_remaining() as isize;
    let mut scrambler = Scrambler::new(seed);
    for _ in 0..num_bits {
        buf.xor_bit(scrambler.next_bit());
    }
    buf.seek_rel(-num_bits);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_scrambling_preserves_prefix_and_cursor() {
        let mut buffer = BitBuffer::from_bitstr("101000101110001101010100");
        buffer.seek(3);
        buffer.set_raw_start(3);
        buffer.seek(2);
        let original = buffer.to_bitstr();
        let mut expected = [0; 21];
        let mut reader = buffer.clone();
        reader.seek(0);
        reader.to_bitarr(&mut expected);
        let code = tetra_scramb_get_init(204, 1337, 1);
        tetra_phy_core::scrambler::tetra_scramb_bits(code, &mut expected[2..]);
        tetra_scramb_bits(code, &mut buffer);
        assert_eq!(buffer.get_pos(), 2);
        assert_eq!(buffer.to_bitstr(), BitBuffer::from_bitarr(&expected).to_bitstr());
        assert_eq!(buffer.to_bitstr(), "001100010001010111010");
        tetra_scramb_bits(code, &mut buffer);
        assert_eq!(buffer.get_pos(), 2);
        assert_eq!(buffer.to_bitstr(), original);
    }
}
