//! Golden vectors captured from the desktop algorithms before extraction.
use tetra_phy_core::convenc::{self, ConvEncState, RcpcPunctMode, SpeechConvEncState};
use tetra_phy_core::errorcontrol::{self, CodingError};
use tetra_phy_core::errorcontrol_params::*;
use tetra_phy_core::{crc16, interleaver, rm3014, scrambler, tch_reorder, viterbi};

const CODE: u32 = 0x3305_3907;
const SPEECH: &str = "011001011101001100011100101110001001100110101000111110111101110110110101011100000111110001110000000101101010100111001110010101101101000101001101111110100011001110111100000101011001010001100101101011000100101110111101000010001011100100001001100011110111110101001110100110000001001111010000110101101111010000010011001010010110000100011000111001101100100111010000110111011010110110100011001011010000101110000111100000101001100000011010";
const SHORT_SPEECH: &str = "111100001011110111001001110011000001011010101001110010101101010100010001111101000101100100101110000110000000010011001000111001000001001111011101000111001111100110001010101010001100010111110001000100000001111001110100001110111111100101101001111111110001010011001001100111110011010100111101011110011111011010000111001011101000000101011100110101111000111010110001101111110011111101101110011110001011101110110001011011101010100001111110";
const EMPTY_SPEECH: &str = "100110000011110111001001101001111001011010101001000110110101010100010001001001011101100100101110101111010000010011001000010000110001001111011101010101101111100110001010111000110100010111110001100001001001111001110100101011110111100101101001110100011001010011001001101100100011010100111101001000100111011010000111011101011000000101011100011001010000111010110001000010011011111101101110000101010011101110110001000001101010100001111110";
const SHORT_DECODED: &str = "0000000000000000000000000000000000011010010000110100001000001001011010010010000010001001101001011000010100001000000101101001010010000010100000000000000000000000000000000000110100100001101000010000010010110100100100000100010011010010110000101000000000001011010010100100000101";

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (((i * i + 3 * i + (i >> 2) + 1) >> 1) & 1) as u8).collect()
}

fn bits(text: &str) -> Vec<u8> {
    text.bytes().map(|b| b - b'0').collect()
}

#[test]
fn all_existing_control_channel_vectors() {
    let mut workspace = errorcontrol::Workspace::new();
    for (params, expected) in [
        (
            &BSCH_PARAMS,
            "011110011111000010110001110001110010101111110111011111001110100011000111011101000110000101011000101111001011000110111111",
        ),
        (
            &SCH_HD_PARAMS,
            "010011111101000111011010101100000111100110011010010010001011101100000000011100100011111100111101110011101110101001011011000101101111110111001111000000010011011110011001101001101010101111101010110100100101000001100011",
        ),
        (
            &SCH_F_PARAMS,
            "101011010100001100100000100100101110110011001000001011100010101111110000000100000010011111001111100010010111101000101001111101100110110100110101011000111000011101111011110101100011101000110000101100011110001010010101100110100010011110001000111001101110101000101000100001110100101111011100000101110000100001100110010000001111111111111101010100000111010001010000001111001100000110001111001000000100010101010000001110111101011010011011",
        ),
        (
            &SCH_HU_PARAMS,
            "001011110111111100001010111100001100111001101010001001000000101011010010000111001000011111001101100010010101101000010011011101001000110100011100011000011110011101001001",
        ),
    ] {
        let input = pattern(usize::from(params.type1_bits));
        let mut encoded = [7; 440];
        let len = usize::from(params.type345_bits);
        errorcontrol::encode_cp(params, &input, CODE, &mut encoded).unwrap();
        assert_eq!(&encoded[..len], bits(expected));
        assert_eq!(&encoded[len..], vec![7; encoded.len() - len]);
        let mut decoded = [7; 280];
        assert!(errorcontrol::decode_cp(params, &encoded[..len], CODE, &mut decoded, &mut workspace).unwrap());
        assert_eq!(&decoded[..input.len()], input);
        assert_eq!(&decoded[input.len()..], vec![7; decoded.len() - input.len()]);
    }
}

#[test]
fn full_short_and_empty_speech_vectors_preserve_padding_and_reordering() {
    let mut workspace = errorcontrol::Workspace::new();
    for (len, expected) in [(274, SPEECH), (137, SHORT_SPEECH), (0, EMPTY_SPEECH)] {
        let input = pattern(len);
        let mut encoded = [0; 432];
        errorcontrol::encode_tch_s(&input, CODE, &mut encoded).unwrap();
        assert_eq!(&encoded[..], bits(expected));
        let mut decoded = [0; 274];
        assert!(errorcontrol::decode_tch_s(&encoded, CODE, &mut decoded, &mut workspace).unwrap());
        match len {
            274 => assert_eq!(&decoded[..], input),
            137 => assert_eq!(&decoded[..], bits(SHORT_DECODED)),
            0 => assert_eq!(decoded, [0; 274]),
            _ => unreachable!(),
        }
    }
}

#[test]
fn aach_existing_air_vector_and_single_bit_correction() {
    let expected: [u8; 30] = bits("100100100001011110111010111011").try_into().unwrap();
    assert_eq!(errorcontrol::encode_aach(0x28a, CODE), expected);
    assert_eq!(errorcontrol::decode_aach(&expected, CODE), Ok(0x28a));
    for bit in 0..30 {
        let mut corrupted = expected;
        corrupted[bit] ^= 1;
        assert_eq!(errorcontrol::decode_aach(&corrupted, CODE), Ok(0x28a));
    }
}

#[test]
fn crc_raw_bits_packed_partial_bytes_and_original_short_input_behavior() {
    let packed = b"123456789";
    let unpacked: Vec<u8> = (0..72).map(|i| crc16::get_nth_bit(packed, i) as u8).collect();
    for (len, expected) in [(0, 0xffff), (1, 0xefdf), (7, 0xe3c1), (8, 0xc782), (63, 0xd885), (72, 0x29b1)] {
        assert_eq!(crc16::crc16_itut_bytes(0xffff, packed, len), expected);
        assert_eq!(crc16::crc16_ccitt_bits(&unpacked, len), expected);
    }
    // The historical bit-per-byte entry point consumes only available entries.
    assert_eq!(crc16::crc16_ccitt_bits(&unpacked, 80), 0x29b1);
    let noncanonical: Vec<_> = unpacked.iter().map(|b| b | 0xfe).collect();
    assert_eq!(crc16::crc16_ccitt_bits(&noncanonical, 72), 0x29b1);
}

#[test]
fn scrambling_sequence_zero_colour_and_incremental_equivalence() {
    assert_eq!(scrambler::tetra_scramb_get_init(204, 1337, 1), CODE);
    assert_eq!(scrambler::tetra_scramb_get_init(u16::MAX, u16::MAX, 0), 0);
    let expected = bits("100110000011110111001001101001111001011010101001000110110101010100010001001001011101100100101110");
    let mut sequence = [0; 96];
    scrambler::tetra_scramb_get_bits(CODE, &mut sequence);
    assert_eq!(&sequence[..], expected);
    let mut incremental = scrambler::Scrambler::new(CODE);
    for bit in &sequence {
        assert_eq!(incremental.next_bit(), *bit);
    }
    let mut input = pattern(96);
    let original = input.clone();
    scrambler::tetra_scramb_bits(CODE, &mut input);
    for i in 0..96 {
        assert_eq!(input[i], original[i] ^ sequence[i]);
    }
    scrambler::tetra_scramb_bits(CODE, &mut input);
    assert_eq!(input, original);
    scrambler::tetra_scramb_bits(0, &mut input);
    assert_eq!(input, original);
}

#[test]
fn convolutional_vectors_and_continuous_state_across_calls() {
    let mut message = pattern(12);
    message.extend_from_slice(&[0; 4]);
    let mut cch = [0; 64];
    ConvEncState::new().encode(&message, &mut cch);
    assert_eq!(&cch[..], bits("0000000011110100001001110111110001011011110100111010111100000000"));
    let mut split_cch = [0; 64];
    let mut encoder = ConvEncState::new();
    encoder.encode(&message[..7], &mut split_cch[..28]);
    encoder.encode(&message[7..], &mut split_cch[28..]);
    assert_eq!(cch, split_cch);
    encoder.reset();
    encoder.encode(&message, &mut split_cch);
    assert_eq!(cch, split_cch);
    let mut rate1_3 = [0; 48];
    ConvEncState::new().encode_rate1_3(&message, &mut rate1_3);
    for i in 0..16 {
        assert_eq!(&rate1_3[i * 3..i * 3 + 3], &cch[i * 4..i * 4 + 3]);
    }
    let mut speech = [0; 48];
    SpeechConvEncState::new().encode(&message, &mut speech);
    assert_eq!(&speech[..], bits("000000111001100010010100110110011011001111000000"));
    let mut split_speech = [0; 48];
    let mut encoder = SpeechConvEncState::new();
    encoder.encode(&message[..7], &mut split_speech[..21]);
    encoder.encode(&message[7..], &mut split_speech[21..]);
    assert_eq!(speech, split_speech);
}

#[test]
fn every_puncturing_mode_preserves_erasure_holes_and_output_suffix() {
    let mother: Vec<_> = (0..1728).map(|i| (i % 2) as u8).collect();
    for (mode, count) in [
        (RcpcPunctMode::Rate2_3, 432),
        (RcpcPunctMode::Rate1_3, 432),
        (RcpcPunctMode::Rate292_432, 432),
        (RcpcPunctMode::Rate148_432, 432),
        (RcpcPunctMode::Rate112_168, 168),
        (RcpcPunctMode::Rate72_162, 162),
        (RcpcPunctMode::Rate38_80, 80),
    ] {
        let mut punctured = [9; 432];
        convenc::get_punctured_rate(mode, &mother, &mut punctured[..count]);
        assert!(punctured[count..].iter().all(|bit| *bit == 9));
        let mut restored = [0xff; 1728];
        convenc::tetra_rcpc_depunct(mode, &punctured, count, &mut restored);
        assert_eq!(convenc::mother_memcmp(&mother, &restored), Ok(count));
        assert_eq!(restored.iter().filter(|bit| **bit != 0xff).count(), count);
    }
    let indexed: Vec<_> = (0..64).collect();
    let mut punctured = [0; 18];
    convenc::get_punctured_rate(RcpcPunctMode::Rate2_3, &indexed, &mut punctured);
    assert_eq!(punctured, [0, 1, 4, 8, 9, 12, 16, 17, 20, 24, 25, 28, 32, 33, 36, 40, 41, 44]);
    convenc::get_punctured_rate(RcpcPunctMode::Rate1_3, &indexed, &mut punctured);
    assert_eq!(punctured, [0, 1, 2, 4, 5, 6, 8, 9, 10, 12, 13, 14, 16, 17, 18, 20, 21, 22]);

    let indexed: Vec<_> = (0..256).map(|i| i as u8).collect();
    for (mode, expected) in [
        (RcpcPunctMode::Rate2_3, [160, 161, 164, 168, 169, 172, 176, 177, 180, 184, 185, 188]),
        (RcpcPunctMode::Rate1_3, [80, 81, 82, 84, 85, 86, 88, 89, 90, 92, 93, 94]),
        (
            RcpcPunctMode::Rate292_432,
            [160, 161, 164, 168, 169, 176, 177, 180, 184, 185, 188, 192],
        ),
        (RcpcPunctMode::Rate148_432, [81, 82, 84, 85, 86, 88, 89, 90, 92, 93, 96, 97]),
        (
            RcpcPunctMode::Rate112_168,
            [120, 121, 123, 126, 127, 129, 132, 133, 135, 138, 139, 141],
        ),
        (RcpcPunctMode::Rate72_162, [79, 81, 82, 84, 85, 86, 87, 88, 90, 91, 93, 94]),
        (RcpcPunctMode::Rate38_80, [84, 85, 87, 88, 90, 91, 93, 94, 96, 97, 98, 99]),
    ] {
        let mut punctured = [0; 72];
        convenc::get_punctured_rate(mode, &indexed, &mut punctured);
        assert_eq!(&punctured[60..], &expected);
    }
}

#[test]
fn interleaving_and_rm_exact_vectors() {
    let indexed: Vec<_> = (0..12).collect();
    let mut output = [0; 12];
    interleaver::block_interleave(12, 5, &indexed, &mut output);
    assert_eq!(output, [11, 4, 9, 2, 7, 0, 5, 10, 3, 8, 1, 6]);
    interleaver::matrix_interleave(4, 3, &indexed, &mut output);
    assert_eq!(output, [0, 3, 6, 9, 1, 4, 7, 10, 2, 5, 8, 11]);
    for (input, expected) in [
        (0, 0),
        (1, 0x104e7),
        (0x1234, 0x12346625),
        (0x2a3b, 0x2a3b12d7),
        (0x3fff, 0x3fff4abf),
    ] {
        assert_eq!(rm3014::tetra_rm3014_compute(input), expected);
        assert_eq!(rm3014::tetra_rm3014_decode_limited_ecc(expected), input);
    }
}

#[test]
fn viterbi_vectors_erasures_errors_ties_trailing_symbols_and_storage() {
    let expected = bits("0011110011000000");
    for (rate, encoded) in [
        (4, bits("0000000011110100001001110111110001011011110100111010111100000000")),
        (3, bits("000000111001100010010100110110011011001111000000")),
    ] {
        let mut received: Vec<i8> = encoded
            .iter()
            .enumerate()
            .map(|(i, bit)| {
                if i % 5 == 0 {
                    0
                } else if i == 9 || i == 23 {
                    if *bit == 0 { 1 } else { -1 }
                } else if *bit == 0 {
                    -1
                } else {
                    1
                }
            })
            .collect();
        received.extend_from_slice(&[1, -1]); // ignored incomplete symbol
        let mut output = [7; 20];
        let mut decisions = [0xbeef; 20];
        let count = if rate == 4 {
            viterbi::TetraViterbiDecoder::new()
                .decode_into(&received, &mut output, &mut decisions)
                .unwrap()
        } else {
            viterbi::TetraCodecViterbiDecoder::new()
                .decode_into(&received, &mut output, &mut decisions)
                .unwrap()
        };
        assert_eq!(count, 16);
        assert_eq!(&output[..count], expected);
        assert_eq!(&output[count..], &[7; 4]);
        assert_eq!(&decisions[count..], &[0xbeef; 4]);
    }
    let decoder = viterbi::TetraViterbiDecoder::new();
    let mut output = [7; 16];
    let mut decisions = [0xbeef; 16];
    assert_eq!(decoder.decode_into(&[0; 64], &mut output, &mut decisions), Ok(16));
    assert_eq!(output, [0; 16]); // original tie-breaking and zero final state
    assert_eq!(decisions, [0; 16]);
    output.fill(7);
    decisions.fill(0xbeef);
    assert_eq!(
        decoder.decode_into(&[0; 64], &mut output[..15], &mut decisions),
        Err(viterbi::DecodeError::OutputTooShort)
    );
    assert_eq!(
        decoder.decode_into(&[0; 64], &mut output, &mut decisions[..15]),
        Err(viterbi::DecodeError::TracebackTooShort)
    );
    assert_eq!(output, [7; 16]);
    assert_eq!(decisions, [0xbeef; 16]);
    assert_eq!(decoder.decode_into(&[], &mut output, &mut decisions), Ok(0));
}

#[test]
fn coding_errors_do_not_modify_output_and_crc_failure_still_returns_bits() {
    let mut workspace = errorcontrol::Workspace::new();
    let mut output = [9; 432];
    assert_eq!(
        errorcontrol::encode_cp(&BSCH_PARAMS, &[0; 59], CODE, &mut output),
        Err(CodingError::InputLength)
    );
    assert_eq!(
        errorcontrol::encode_cp(&BSCH_PARAMS, &[0; 60], CODE, &mut output[..119]),
        Err(CodingError::OutputLength)
    );
    assert_eq!(
        errorcontrol::encode_cp(&AACH_PARAMS, &[0; 14], CODE, &mut output),
        Err(CodingError::InvalidParameters)
    );
    assert_eq!(
        errorcontrol::encode_cp(&BSCH_PARAMS, &[2; 60], CODE, &mut output),
        Err(CodingError::InvalidBit)
    );
    assert_eq!(
        errorcontrol::encode_tch_s(&[0; 275], CODE, &mut output),
        Err(CodingError::InputLength)
    );
    assert_eq!(
        errorcontrol::encode_tch_s(&[2; 274], CODE, &mut output),
        Err(CodingError::InvalidBit)
    );
    assert_eq!(output, [9; 432]);
    let mut decoded = [9; 274];
    assert_eq!(
        errorcontrol::decode_cp(&BSCH_PARAMS, &[0; 119], CODE, &mut decoded, &mut workspace),
        Err(CodingError::InputLength)
    );
    assert_eq!(
        errorcontrol::decode_cp(&BSCH_PARAMS, &[0; 120], CODE, &mut decoded[..59], &mut workspace),
        Err(CodingError::OutputLength)
    );
    assert_eq!(
        errorcontrol::decode_cp(&BSCH_PARAMS, &[2; 120], CODE, &mut decoded, &mut workspace),
        Err(CodingError::InvalidBit)
    );
    assert_eq!(
        errorcontrol::decode_tch_s(&[2; 432], CODE, &mut decoded, &mut workspace),
        Err(CodingError::InvalidBit)
    );
    assert_eq!(decoded, [9; 274]);
    assert_eq!(
        errorcontrol::decode_cp(&BSCH_PARAMS, &[0; 120], 0, &mut decoded, &mut workspace),
        Ok(false)
    );
    assert_eq!(&decoded[..60], &[0; 60]);
    assert_eq!(&decoded[60..], &[9; 214]);
    assert_eq!(errorcontrol::decode_aach(&[2; 30], CODE), Err(CodingError::InvalidBit));
}

#[test]
fn speech_reordering_and_crc_class_boundaries() {
    let codec: [u8; 274] = pattern(274).try_into().unwrap();
    let channel = tch_reorder::codec_to_channel(&codec);
    let mut encoded_from_channel = [0; 432];
    errorcontrol::encode_speech_channel(&channel, CODE, &mut encoded_from_channel).unwrap();
    assert_eq!(&encoded_from_channel[..], bits(SPEECH));
    assert_eq!(errorcontrol::speech_crc(&[0; 60]), [0; 8]);
    let mut class2 = [0; 60];
    class2[0] = 1;
    assert_eq!(errorcontrol::speech_crc(&class2), [1, 0, 0, 1, 0, 0, 0, 1]);
}

#[test]
fn shared_decoder_workspace_reuses_erasure_storage_across_channels_and_failures() {
    #[repr(C)]
    struct GuardedStorage {
        before: u16,
        workspace: core::mem::MaybeUninit<errorcontrol::Workspace>,
        after: u16,
    }
    let mut storage = GuardedStorage {
        before: 0x1234,
        workspace: core::mem::MaybeUninit::uninit(),
        after: 0xabcd,
    };
    let workspace = errorcontrol::Workspace::initialize(&mut storage.workspace);
    let speech: [u8; 432] = bits(SPEECH).try_into().unwrap();
    for (pass, params) in [&SCH_F_PARAMS, &BSCH_PARAMS, &SCH_HU_PARAMS, &SCH_HD_PARAMS, &BSCH_PARAMS]
        .into_iter()
        .enumerate()
    {
        let input: Vec<_> = pattern(usize::from(params.type1_bits))
            .into_iter()
            .map(|bit| bit ^ (pass as u8 & 1))
            .collect();
        let mut encoded = vec![0; usize::from(params.type345_bits)];
        errorcontrol::encode_cp(params, &input, CODE, &mut encoded).unwrap();
        let mut decoded = [7; 274];
        assert!(errorcontrol::decode_cp(params, &encoded, CODE, &mut decoded, workspace).unwrap());
        assert_eq!(&decoded[..input.len()], input);
        assert!(decoded[input.len()..].iter().all(|bit| *bit == 7));

        assert!(errorcontrol::decode_tch_s(&speech, CODE, &mut decoded, workspace).unwrap());
        assert_eq!(&decoded[..], pattern(274));
        decoded.fill(7);
        assert_eq!(
            errorcontrol::decode_tch_s(&[2; 432], CODE, &mut decoded, workspace),
            Err(CodingError::InvalidBit)
        );
        assert_eq!(decoded, [7; 274]);
        assert_eq!(
            errorcontrol::decode_cp(&BSCH_PARAMS, &[0; 120], 0, &mut decoded, workspace),
            Ok(false)
        );
        assert_eq!(&decoded[..60], &[0; 60]);
    }
    let workspace = errorcontrol::Workspace::initialize(&mut storage.workspace);
    let mut decoded = [7; 274];
    assert!(errorcontrol::decode_tch_s(&speech, CODE, &mut decoded, workspace).unwrap());
    assert_eq!(&decoded[..], pattern(274));
    assert_eq!(storage.before, 0x1234);
    assert_eq!(storage.after, 0xabcd);
}

#[test]
fn hard_viterbi_workspace_matches_soft_decoding_and_rejects_bad_dimensions() {
    fn compare<const N: usize>(decoder: viterbi::ViterbiDecoder<N>, mut hard: Vec<u8>) {
        for (i, bit) in hard.iter_mut().enumerate() {
            if i % 5 == 0 {
                *bit = 0xff;
            } else if i == 9 || i == 23 {
                *bit ^= 1;
            }
        }
        let soft: Vec<_> = hard
            .iter()
            .map(|bit| match bit {
                0 => -1,
                1 => 1,
                _ => 0,
            })
            .collect();
        let mut expected = [7; 20];
        let mut decisions = [0; 16];
        decoder.decode_into(&soft, &mut expected, &mut decisions).unwrap();
        let mut storage = core::mem::MaybeUninit::<viterbi::Workspace<16>>::uninit();
        let workspace = viterbi::Workspace::initialize(&mut storage);
        let mut actual = [7; 20];
        // An invalid byte in an incomplete trailing symbol is not consumed.
        hard.push(2);
        assert_eq!(decoder.decode_hard_with_workspace(&hard, &mut actual, workspace), Ok(16));
        assert_eq!(actual, expected);
        actual.fill(7);
        assert_eq!(decoder.decode_with_workspace(&soft, &mut actual, workspace), Ok(16));
        assert_eq!(actual, expected);
        actual.fill(7);
        hard[0] = 2;
        assert_eq!(
            decoder.decode_hard_with_workspace(&hard, &mut actual, workspace),
            Err(viterbi::DecodeError::InvalidBit)
        );
        assert_eq!(actual, [7; 20]);
        hard[0] = 0xff;
        assert_eq!(
            decoder.decode_hard_with_workspace(&hard, &mut actual, &mut viterbi::Workspace::<15>::new()),
            Err(viterbi::DecodeError::TracebackTooShort)
        );
        assert_eq!(actual, [7; 20]);
    }

    let cch = bits("0000000011110100001001110111110001011011110100111010111100000000");
    compare(viterbi::TetraViterbiDecoder::new(), cch.clone());
    compare(
        viterbi::TetraCodecViterbiDecoder::new(),
        bits("000000111001100010010100110110011011001111000000"),
    );
    let mut workspace = viterbi::Workspace::<16>::new();
    let mut output = [7; 16];
    assert_eq!(
        viterbi::dec_sb1_with_workspace(&cch[..63], &mut output, 16, &mut workspace),
        Err(viterbi::DecodeError::InputTooShort)
    );
    assert_eq!(
        viterbi::dec_sb1_with_workspace(&cch, &mut output[..15], 16, &mut workspace),
        Err(viterbi::DecodeError::OutputTooShort)
    );
    assert_eq!(
        viterbi::dec_sb1_with_workspace(&[], &mut output, 865, &mut workspace),
        Err(viterbi::DecodeError::SymbolCountTooLarge)
    );
    assert_eq!(output, [7; 16]);
    viterbi::dec_sb1_with_workspace(&cch, &mut output, 16, &mut workspace).unwrap();
    assert_eq!(&output[..], bits("0011110011000000"));
    let mut empty = core::mem::MaybeUninit::<viterbi::Workspace<0>>::uninit();
    assert_eq!(
        viterbi::dec_sb1_with_workspace(&[], &mut [], 0, viterbi::Workspace::initialize(&mut empty)),
        Ok(())
    );
}
