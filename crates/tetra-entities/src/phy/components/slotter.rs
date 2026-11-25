use tetra_core::TrainingSequence;

use crate::phy::components::burst_consts::*;

#[allow(non_upper_case_globals)]
pub mod bitseq {
    /// Clause 9.4.4.3.2 Normal Training Sequence 1.
    pub const n: [u8; 22] = [1, 1, 0, 1, 0, 0, 0, 0, 1, 1, 1, 0, 1, 0, 0, 1, 1, 1, 0, 1, 0, 0];
    /// Clause 9.4.4.3.2 Normal Training Sequence 2.
    pub const p: [u8; 22] = [0, 1, 1, 1, 1, 0, 1, 0, 0, 1, 0, 0, 0, 0, 1, 1, 0, 1, 1, 1, 1, 0];
    /// Clause 9.4.4.3.3 Extended Training Sequence.
    pub const x: [u8; 30] = [
        1, 0, 0, 1, 1, 1, 0, 1, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 1, 1, 1, 0, 1, 0, 0, 0, 0, 1, 1,
    ];
    /// Clause 9.4.4.3.5 Tail Bits.
    pub const t: [u8; 4] = [1, 1, 0, 0];
}

/// Constructs a Normal Uplink Burst from two type-5 blocks
/// (ETSI TS 100 392-2 clause 9.4.4.2.4, Table 9.5).
pub fn build_nub(train_seq: TrainingSequence, blk1: &[u8; NUB_BLK_BITS], blk2: &[u8; NUB_BLK_BITS]) -> [u8; NUB_BURST_BITS] {
    let mut type5 = [0u8; NUB_BURST_BITS];
    type5[0..NUB_BLK1_OFFSET].copy_from_slice(&bitseq::t);
    type5[NUB_BLK1_OFFSET..NUB_BLK1_OFFSET + NUB_BLK_BITS].copy_from_slice(blk1);
    match train_seq {
        TrainingSequence::NormalTrainSeq1 => {
            type5[NUB_TRAINING_OFFSET..NUB_BLK2_OFFSET].copy_from_slice(&bitseq::n);
        }
        TrainingSequence::NormalTrainSeq2 => {
            type5[NUB_TRAINING_OFFSET..NUB_BLK2_OFFSET].copy_from_slice(&bitseq::p);
        }
        _ => panic!("build_nub: invalid training sequence {:?}", train_seq),
    }
    type5[NUB_BLK2_OFFSET..NUB_BLK2_OFFSET + NUB_BLK_BITS].copy_from_slice(blk2);
    type5[NUB_TAILBITS_OFFSET..NUB_BURST_BITS].copy_from_slice(&bitseq::t);
    type5
}

/// Constructs a Control Uplink Burst from one 168-bit type-5 block
/// (ETSI TS 100 392-2 clause 9.4.4.2.1, Table 9.3).
pub fn build_cub(blk: &[u8; CUB_BLK_BITS * 2]) -> [u8; CUB_BURST_BITS] {
    let mut type5 = [0u8; CUB_BURST_BITS];
    type5[0..CUB_BLK1_OFFSET].copy_from_slice(&bitseq::t);
    type5[CUB_BLK1_OFFSET..CUB_BLK1_OFFSET + CUB_BLK_BITS].copy_from_slice(&blk[..CUB_BLK_BITS]);
    type5[CUB_TRAINING_OFFSET..CUB_BLK2_OFFSET].copy_from_slice(&bitseq::x);
    type5[CUB_BLK2_OFFSET..CUB_BLK2_OFFSET + CUB_BLK_BITS].copy_from_slice(&blk[CUB_BLK_BITS..]);
    type5[CUB_TAILBITS_OFFSET..CUB_BURST_BITS].copy_from_slice(&bitseq::t);
    type5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_nub_roundtrip() {
        let blk1: [u8; NUB_BLK_BITS] = std::array::from_fn(|i| (i % 2) as u8);
        let blk2: [u8; NUB_BLK_BITS] = std::array::from_fn(|i| ((i + 1) % 2) as u8);
        let burst = build_nub(TrainingSequence::NormalTrainSeq1, &blk1, &blk2);
        assert_eq!(burst.len(), NUB_BURST_BITS);
        assert_eq!(&burst[0..NUB_BLK1_OFFSET], &bitseq::t);
        assert_eq!(&burst[NUB_BLK1_OFFSET..NUB_BLK1_OFFSET + NUB_BLK_BITS], &blk1);
        assert_eq!(&burst[NUB_TRAINING_OFFSET..NUB_BLK2_OFFSET], &bitseq::n);
        assert_eq!(&burst[NUB_BLK2_OFFSET..NUB_BLK2_OFFSET + NUB_BLK_BITS], &blk2);
        assert_eq!(&burst[NUB_TAILBITS_OFFSET..NUB_BURST_BITS], &bitseq::t);
        let burst2 = build_nub(TrainingSequence::NormalTrainSeq2, &blk1, &blk2);
        assert_eq!(&burst2[NUB_TRAINING_OFFSET..NUB_BLK2_OFFSET], &bitseq::p);
    }

    #[test]
    fn test_build_cub_roundtrip() {
        let blk: [u8; CUB_BLK_BITS * 2] = std::array::from_fn(|i| (i % 2) as u8);
        let burst = build_cub(&blk);
        assert_eq!(burst.len(), CUB_BURST_BITS);
        assert_eq!(&burst[0..CUB_BLK1_OFFSET], &bitseq::t);
        assert_eq!(&burst[CUB_BLK1_OFFSET..CUB_BLK1_OFFSET + CUB_BLK_BITS], &blk[..CUB_BLK_BITS]);
        assert_eq!(&burst[CUB_TRAINING_OFFSET..CUB_BLK2_OFFSET], &bitseq::x);
        assert_eq!(&burst[CUB_BLK2_OFFSET..CUB_BLK2_OFFSET + CUB_BLK_BITS], &blk[CUB_BLK_BITS..]);
        assert_eq!(&burst[CUB_TAILBITS_OFFSET..CUB_BURST_BITS], &bitseq::t);
    }
}
