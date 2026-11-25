use tetra_core::{BitBuffer, PhyBlockType};
use tetra_saps::tmv::TmvUnitdataReq;
use tetra_saps::tmv::enums::logical_chans::LogicalChannel;
use tetra_saps::tp::TpUnitdataInd;

use super::{errorcontrol_params, scrambler, tch_reorder};
use tetra_phy_core::errorcontrol as coding;

/// Application primitive/buffer adapter for the portable control-channel encoder.
pub fn encode_cp(mut prim: TmvUnitdataReq) -> BitBuffer {
    let lchan = prim.logical_channel;
    assert!(lchan.is_control_channel() && lchan != LogicalChannel::Aach);
    let params = errorcontrol_params::get_params(lchan);
    let type1_len = usize::from(params.type1_bits);
    let coded_len = usize::from(params.type345_bits);
    assert_eq!(prim.mac_block.get_len(), type1_len);
    tracing::trace!("encode_cp {:?} type1 {}", lchan, prim.mac_block.dump_bin());
    prim.mac_block.seek(0);
    let mut input = [0; 274];
    prim.mac_block.to_bitarr(&mut input[..type1_len]);
    let mut output = [0; 432];
    coding::encode_cp(params, &input[..type1_len], prim.scrambling_code, &mut output).expect("control coding dimensions");
    let result = BitBuffer::from_bitarr(&output[..coded_len]);
    tracing::trace!("encode_cp {:?} type5: {:?}", lchan, result.dump_bin());
    result
}

/// Keep scrambling-code acquisition and PHY block identity outside the core.
pub fn decode_cp(lchan: LogicalChannel, prim: TpUnitdataInd, default_scramb_code: Option<u32>) -> (Option<BitBuffer>, bool) {
    assert!(lchan.is_control_channel() && lchan != LogicalChannel::Aach);
    let params = errorcontrol_params::get_params(lchan);
    let mut block = prim.block;
    tracing::trace!("decode_cp {:?} type5: {:?}", lchan, block.dump_bin());
    let code = if prim.block_type == PhyBlockType::SB1 {
        scrambler::SCRAMB_INIT
    } else if let Some(code) = default_scramb_code {
        code
    } else {
        tracing::warn!("decode_cp: no scrambling code set, need to receive SYNC first");
        return (None, false);
    };
    let mut input = [0; 432];
    let coded_len = usize::from(params.type345_bits);
    block.to_bitarr(&mut input[..coded_len]);
    let mut output = [0; 274];
    let mut storage = Box::<coding::Workspace>::new_uninit();
    let workspace = coding::Workspace::initialize(storage.as_mut());
    let crc_ok = coding::decode_cp(params, &input[..coded_len], code, &mut output, workspace).expect("control coding dimensions");
    let result = BitBuffer::from_bitarr(&output[..usize::from(params.type1_bits)]);
    (Some(result), crc_ok)
}

/// Preserve application-buffer padding, cursor reset and stolen-half-slot selection.
pub fn encode_tp(mut prim: TmvUnitdataReq, blk_num: u8) -> BitBuffer {
    let params = errorcontrol_params::get_params(prim.logical_channel);
    prim.mac_block.seek(0);
    let type1_len = prim.mac_block.get_len().min(usize::from(params.type1_bits));
    let mut input = [0; 274];
    prim.mac_block.to_bitarr(&mut input[..type1_len]);
    if prim.logical_channel == LogicalChannel::TchS && type1_len == 274 {
        input = tch_reorder::codec_to_channel(&input);
    }
    let mut output = [0; 432];
    coding::encode_speech_channel(&input, prim.scrambling_code, &mut output).expect("speech bits");
    let coded_len = usize::from(params.type345_bits);
    let start = if blk_num == 1 { 0 } else { 216 };
    BitBuffer::from_bitarr(&output[start..coded_len])
}

pub fn decode_tp(lchan: LogicalChannel, mut block: BitBuffer, scrambling_code: u32) -> (Option<BitBuffer>, bool) {
    assert_eq!(lchan, LogicalChannel::TchS);
    // Historically only the remaining window is scrambled, then the cursor is
    // reset to its start. Keep this even for a nonzero incoming cursor.
    scrambler::tetra_scramb_bits(scrambling_code, &mut block);
    block.seek(0);
    let mut input = [0; 432];
    block.to_bitarr(&mut input);
    let mut output = [0; 274];
    let mut storage = Box::<coding::Workspace>::new_uninit();
    let workspace = coding::Workspace::initialize(storage.as_mut());
    let crc_ok = coding::decode_tch_s(&input, 0, &mut output, workspace).expect("speech bits");
    (Some(BitBuffer::from_bitarr(&output)), crc_ok)
}

pub fn encode_aach(mut buf: BitBuffer, scrambling_code: u32) -> BitBuffer {
    tracing::trace!("encode_aach type1: {:?}", buf.dump_bin());
    assert_eq!(buf.get_len_remaining(), 14);
    let data = buf.read_bits(14).unwrap() as u16;
    let result = BitBuffer::from_bitarr(&coding::encode_aach(data, scrambling_code));
    tracing::trace!("encode_aach type5: {:?}", result.dump_bin());
    result
}

pub fn decode_aach(mut buf: BitBuffer, scrambling_code: u32) -> BitBuffer {
    tracing::trace!("decode_aach type5: {:?}", buf.dump_bin());
    assert_eq!(buf.get_len_remaining(), 30);
    scrambler::tetra_scramb_bits(scrambling_code, &mut buf);
    let mut bits = [0; 30];
    buf.to_bitarr(&mut bits);
    let data = coding::decode_aach(&bits, 0).expect("AACH bits");
    // Preserve the original absolute-window reset and backing buffer reuse.
    buf.set_raw_start(0);
    buf.set_raw_pos(0);
    buf.set_raw_end(14);
    buf.write_bits(u64::from(data), 14);
    buf.seek(0);
    tracing::debug!("decode_aach type1: {:?}", buf.dump_bin());
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use tetra_core::{BurstType, PhyBlockNum, TdmaTime, TrainingSequence, debug::setup_logging_verbose};
    use tetra_phy_core::errorcontrol::speech_crc;

    #[test]
    fn test_encdec_bnch() {
        let type1vec =
            "1000001111101001010000000000101001101110011000000000000000001010000101010100000000000000000000101111111111111111110100100000";
        let type5vec = "001101111110011111000110100001101110011100110000111100011000011100101011111100010101101001101001001110011100001010001101101010100000000011010001001101001010101100100110011001111100001011000001010010000011010110110110";
        let lchan = LogicalChannel::Bnch;
        let scramb_code = scrambler::tetra_scramb_get_init(204, 1337, 1);
        let type5 = encode_cp(TmvUnitdataReq {
            mac_block: BitBuffer::from_bitstr(type1vec),
            logical_channel: lchan,
            scrambling_code: scramb_code,
        });
        assert_eq!(type5vec, type5.to_bitstr());
        let prim_ind = TpUnitdataInd {
            train_type: TrainingSequence::SyncTrainSeq,
            burst_type: BurstType::SDB,
            block_type: PhyBlockType::SB2,
            block_num: PhyBlockNum::Block2,
            block: type5,
            time: TdmaTime::default(),
        };
        let (type1, crc_ok) = decode_cp(lchan, prim_ind, Some(scramb_code));
        assert!(crc_ok);
        assert_eq!(type1vec, type1.unwrap().to_bitstr());
    }

    #[test]
    fn test_encdec_bsch() {
        let type1_vec = "000100000111000010000010000000000110011000001010011100110001";
        let lchan = LogicalChannel::Bsch;
        let scramb_code = scrambler::SCRAMB_INIT;
        let type5 = encode_cp(TmvUnitdataReq {
            mac_block: BitBuffer::from_bitstr(type1_vec),
            logical_channel: lchan,
            scrambling_code: scramb_code,
        });
        let prim_ind = TpUnitdataInd {
            train_type: TrainingSequence::SyncTrainSeq,
            burst_type: BurstType::SDB,
            block_type: PhyBlockType::SB2,
            block_num: PhyBlockNum::Block2,
            block: type5,
            time: TdmaTime::default(),
        };
        let (type1, crc_ok) = decode_cp(lchan, prim_ind, Some(scramb_code));
        assert!(crc_ok);
        assert_eq!(type1_vec, type1.unwrap().to_bitstr());
    }

    #[test]
    fn test_encdec_aach() {
        let scramb_code = scrambler::tetra_scramb_get_init(204, 1337, 1);
        let type5vec = "100100100001011110111010111011";
        let type1vec = "00001010001010";
        let type1 = decode_aach(BitBuffer::from_bitstr(type5vec), scramb_code);
        let type5 = encode_aach(BitBuffer::from_bitstr(type1vec), scramb_code);
        assert_eq!(type5vec, type5.to_bitstr());
        assert_eq!(type1vec, type1.to_bitstr());
    }

    #[test]
    fn test_speech_crc() {
        assert_eq!(speech_crc(&[0; 60]), [0; 8]);
        let mut data = [0; 60];
        data[0] = 1;
        let crc = speech_crc(&data);
        assert_eq!(&crc[..7], &[1, 0, 0, 1, 0, 0, 0]);
        assert_eq!(crc[7], 1);
        let mut data2 = [0; 60];
        data2[1] = 1;
        assert_eq!(&speech_crc(&data2)[..7], &[0, 1, 0, 0, 1, 0, 0]);
    }

    #[test]
    fn test_encdec_sch_f() {
        setup_logging_verbose();
        let type1_vec = "0000000000110001000000000010011100010001000001110010000010000001000000000010011100010001010000000000001000110110011011100000100110000001011100000000110101000110011100000100000000000000000100001000000000000000000000000000000000000000000000000000000000000000000000000000";
        let lchan = LogicalChannel::SchF;
        let scramb_code = scrambler::tetra_scramb_get_init(204, 1337, 1);
        let type5 = encode_cp(TmvUnitdataReq {
            mac_block: BitBuffer::from_bitstr(type1_vec),
            logical_channel: lchan,
            scrambling_code: scramb_code,
        });
        let prim_ind = TpUnitdataInd {
            train_type: TrainingSequence::NormalTrainSeq1,
            burst_type: BurstType::NDB,
            block_type: PhyBlockType::NDB,
            block_num: PhyBlockNum::Both,
            block: type5,
            time: TdmaTime::default(),
        };
        let (type1, crc_ok) = decode_cp(lchan, prim_ind, Some(scramb_code));
        assert!(crc_ok);
        assert_eq!(type1_vec, type1.unwrap().to_bitstr());
    }

    #[test]
    fn test_encdec_tch_s() {
        let codec_bits: Vec<u8> = (0..274).map(|_| rand::random_range(0..2) as u8).collect();
        let scramb_code = scrambler::tetra_scramb_get_init(204, 1337, 1);
        let type5 = encode_tp(
            TmvUnitdataReq {
                mac_block: BitBuffer::from_bitarr(&codec_bits),
                logical_channel: LogicalChannel::TchS,
                scrambling_code: scramb_code,
            },
            1,
        );
        assert_eq!(type5.get_len(), 432);
        let (decoded, crc_ok) = decode_tp(LogicalChannel::TchS, type5, scramb_code);
        assert!(crc_ok, "CRC check failed for speech decode");
        assert_eq!(decoded.unwrap().to_bitstr(), BitBuffer::from_bitarr(&codec_bits).to_bitstr());
    }

    #[test]
    fn sync_block_scrambling_override_and_missing_cell_code() {
        let input = "000100000111000010000010000000000110011000001010011100110001";
        let encoded = encode_cp(TmvUnitdataReq {
            mac_block: BitBuffer::from_bitstr(input),
            logical_channel: LogicalChannel::Bsch,
            scrambling_code: scrambler::SCRAMB_INIT,
        });
        let mut primitive = TpUnitdataInd {
            train_type: TrainingSequence::SyncTrainSeq,
            burst_type: BurstType::SDB,
            block_type: PhyBlockType::SB2,
            block_num: PhyBlockNum::Block2,
            block: encoded.clone(),
            time: TdmaTime::default(),
        };
        let (missing, crc_ok) = decode_cp(LogicalChannel::Bsch, primitive, None);
        assert!(missing.is_none());
        assert!(!crc_ok);
        primitive = TpUnitdataInd {
            train_type: TrainingSequence::SyncTrainSeq,
            burst_type: BurstType::SDB,
            block_type: PhyBlockType::SB1,
            block_num: PhyBlockNum::Block1,
            block: encoded,
            time: TdmaTime::default(),
        };
        let (decoded, crc_ok) = decode_cp(LogicalChannel::Bsch, primitive, Some(0xdead_beef));
        assert!(crc_ok);
        assert_eq!(decoded.unwrap().to_bitstr(), input);
    }

    #[test]
    fn stolen_half_slot_and_oversized_speech_input_keep_original_selection() {
        let input: Vec<u8> = (0..280).map(|i| ((i * 3 + i / 7) % 2) as u8).collect();
        let code = scrambler::tetra_scramb_get_init(204, 1337, 1);
        let primitive = || TmvUnitdataReq {
            mac_block: BitBuffer::from_bitarr(&input),
            logical_channel: LogicalChannel::TchS,
            scrambling_code: code,
        };
        let full = encode_tp(primitive(), 1);
        let half = encode_tp(primitive(), 2);
        let mut expected = [0; 432];
        coding::encode_tch_s(&input[..274], code, &mut expected).unwrap();
        assert_eq!(full.to_bitstr(), BitBuffer::from_bitarr(&expected).to_bitstr());
        assert_eq!(half.to_bitstr(), full.to_bitstr()[216..]);
        // All non-1 block numbers historically selected the same second half.
        assert_eq!(encode_tp(primitive(), 0).to_bitstr(), half.to_bitstr());
    }
}
