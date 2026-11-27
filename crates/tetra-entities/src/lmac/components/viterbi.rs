pub use tetra_phy_core::viterbi::{SoftBit, num_states};

/// Legacy application entry point. Its large traceback workspace is heap-owned,
/// outside the portable decoder and its call stack.
pub fn dec_sb1(in_buf: &[u8], out_buf: &mut [u8], sym_count: usize) {
    assert!(sym_count <= 864, "sym_count too large");
    assert!(in_buf.len() >= sym_count * 4, "in_buf too short");
    assert!(out_buf.len() >= sym_count, "out_buf too short");
    let mut storage = Box::<tetra_phy_core::viterbi::Sb1Workspace>::new_uninit();
    let workspace = tetra_phy_core::viterbi::Sb1Workspace::initialize(storage.as_mut());
    tetra_phy_core::viterbi::dec_sb1_with_workspace(in_buf, out_buf, sym_count, workspace)
        .unwrap_or_else(|_| panic!("viterbi_dec_sb1_wrapper: invalid input"));
}

/// Host allocation adapter for the portable caller-storage decoder.
pub struct ViterbiDecoder<const N: usize>(tetra_phy_core::viterbi::ViterbiDecoder<N>);

impl<const N: usize> ViterbiDecoder<N> {
    pub fn new_with_polynomials(generator_polynomials: &[[bool; 5]; N]) -> Self {
        Self(tetra_phy_core::viterbi::ViterbiDecoder::new_with_polynomials(generator_polynomials))
    }

    pub fn decode(&self, received_bits: &[SoftBit]) -> Vec<u8> {
        let count = received_bits.len() / N;
        let mut decoded = vec![0; count];
        let mut decisions = vec![0; count];
        self.0
            .decode_into(received_bits, &mut decoded, &mut decisions)
            .expect("decoder storage");
        decoded
    }
}

pub type TetraViterbiDecoder = ViterbiDecoder<4>;
pub type TetraCodecViterbiDecoder = ViterbiDecoder<3>;

impl Default for TetraViterbiDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl TetraViterbiDecoder {
    pub fn new() -> Self {
        Self(tetra_phy_core::viterbi::TetraViterbiDecoder::new())
    }
}

impl Default for TetraCodecViterbiDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl TetraCodecViterbiDecoder {
    pub fn new() -> Self {
        Self(tetra_phy_core::viterbi::TetraCodecViterbiDecoder::new())
    }
}

#[cfg(test)]
mod tests {
    use super::super::convenc;
    use super::*;
    use rand;

    #[test]
    fn test_decoder() {
        let message: Vec<u8> = (0..288).map(|_| rand::random_range(0..2)).chain((0..4).map(|_| 0)).collect();
        let mut encoder = convenc::ConvEncState::new();
        let mut encoded = vec![0u8; message.len() * 4];
        encoder.encode(&message[..], &mut encoded[..]);
        let encoded_soft: Vec<i8> = encoded
            .into_iter()
            .enumerate()
            .map(|(i, bit)| {
                if i % 3 > 0 {
                    0
                } else if bit != 0 {
                    1
                } else {
                    -1
                }
            })
            .collect();
        let decoder = TetraViterbiDecoder::new();
        let decoded_message = decoder.decode(&encoded_soft[..]);
        assert!(decoded_message[..] == message[..]);
        let hard: Vec<_> = encoded_soft
            .iter()
            .map(|bit| match bit {
                -1 => 0,
                1 => 1,
                _ => 0xff,
            })
            .collect();
        let mut output = [9; 300];
        dec_sb1(&hard, &mut output, message.len());
        assert_eq!(&output[..message.len()], &message);
        assert!(output[message.len()..].iter().all(|bit| *bit == 9));
    }
}
