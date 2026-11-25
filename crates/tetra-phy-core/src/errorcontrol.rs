//! The type-1/type-5 bit pipelines, without application buffer ownership.
//!
//! Input and output lengths are checked before either output is modified.
//! Failed CRCs still return decoded bits, matching the application implementation.
//! These functions do not choose channels, acquire scrambling codes, or schedule
//! blocks; those decisions remain with their callers.

use crate::convenc::{self, ConvEncState, RcpcPunctMode, SpeechConvEncState};
use crate::errorcontrol_params::{BSCH_PARAMS, ErrorControlParams, SCH_F_PARAMS, SCH_HD_PARAMS, SCH_HU_PARAMS};
use crate::{crc16, interleaver, rm3014, scrambler, tch_reorder, viterbi};

/// Invalid caller-provided coding dimensions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodingError {
    InputLength,
    OutputLength,
    InvalidParameters,
    InvalidBit,
}

/// Reusable caller-owned control/speech decoder scratch.
///
/// The same storage supports every existing control-channel block and TCH/S.
/// Initialize once in caller-owned static storage with `const { Workspace::new() }`
/// on constrained targets, or use [`Self::initialize`] for runtime initialization
/// without a second workspace. Each decode overwrites its used regions in place;
/// no reset is required between calls.
#[repr(C, align(4))]
pub struct Workspace {
    stage: [u8; 432],
    type3: [u8; 432],
    mother: [u8; 288 * 4],
    decoded: [u8; 288],
    viterbi: viterbi::Workspace<288>,
}

#[allow(clippy::new_without_default, reason = "runtime initialization must use the in-place initializer")]
impl Workspace {
    /// Const initializer for application-owned static storage.
    ///
    /// Runtime construction by value can require a workspace-sized temporary;
    /// use [`Self::initialize`] when initializing runtime storage.
    pub const fn new() -> Self {
        Self {
            stage: [0; 432],
            type3: [0; 432],
            mother: [0; 288 * 4],
            decoded: [0; 288],
            viterbi: viterbi::Workspace::new(),
        }
    }

    /// Initialize caller-owned storage in place, without a workspace-sized
    /// temporary. The returned borrow prevents storage reuse during decoding.
    #[allow(unsafe_code, reason = "initialize private integer-array scratch without a large stack temporary")]
    pub fn initialize(storage: &mut core::mem::MaybeUninit<Self>) -> &mut Self {
        // SAFETY: every field, including the private Viterbi workspace, contains
        // only integer arrays for which all-zero bytes are a valid value. The
        // exclusive, correctly aligned storage borrow covers the whole object.
        unsafe {
            storage.as_mut_ptr().write_bytes(0, 1);
            storage.assume_init_mut()
        }
    }
}

fn check_bits(input: &[u8]) -> Result<(), CodingError> {
    if input.iter().any(|bit| *bit > 1) {
        return Err(CodingError::InvalidBit);
    }
    Ok(())
}

fn check_cp_params(params: &ErrorControlParams) -> Result<(), CodingError> {
    if ![&BSCH_PARAMS, &SCH_HD_PARAMS, &SCH_F_PARAMS, &SCH_HU_PARAMS].contains(&params) {
        return Err(CodingError::InvalidParameters);
    }
    Ok(())
}

/// Encode a supported control block (BSCH, SCH/HD, SCH/F, or SCH/HU).
pub fn encode_cp(params: &ErrorControlParams, input: &[u8], scrambling_code: u32, output: &mut [u8]) -> Result<(), CodingError> {
    check_cp_params(params)?;
    let type1 = usize::from(params.type1_bits);
    let type2 = usize::from(params.type2_bits);
    let coded = usize::from(params.type345_bits);
    if input.len() != type1 {
        return Err(CodingError::InputLength);
    }
    if output.len() < coded {
        return Err(CodingError::OutputLength);
    }
    check_bits(input)?;

    let mut type2_arr = [0; 288];
    type2_arr[..type1].copy_from_slice(input);
    let crc = !crc16::crc16_ccitt_bits(input, type1);
    for i in 0..16 {
        type2_arr[type1 + i] = ((crc >> (15 - i)) & 1) as u8;
    }
    let mut mother = [0; 432 * 4];
    ConvEncState::new().encode(&type2_arr[..type2], &mut mother);
    let mut type3 = [0; 432];
    convenc::get_punctured_rate(RcpcPunctMode::Rate2_3, &mother, &mut type3[..coded]);
    interleaver::block_interleave(coded, usize::from(params.interleave_a), &type3, output);
    scrambler::tetra_scramb_bits(scrambling_code, &mut output[..coded]);
    Ok(())
}

/// Decode a control block using caller-owned scratch, writing decoded bits even
/// when the CRC fails. No large local arrays or replacement workspace are made.
pub fn decode_cp(
    params: &ErrorControlParams,
    input: &[u8],
    scrambling_code: u32,
    output: &mut [u8],
    workspace: &mut Workspace,
) -> Result<bool, CodingError> {
    check_cp_params(params)?;
    let type1 = usize::from(params.type1_bits);
    let type2 = usize::from(params.type2_bits);
    let coded = usize::from(params.type345_bits);
    if input.len() != coded {
        return Err(CodingError::InputLength);
    }
    if output.len() < type1 {
        return Err(CodingError::OutputLength);
    }
    check_bits(input)?;

    workspace.stage[..coded].copy_from_slice(input);
    scrambler::tetra_scramb_bits(scrambling_code, &mut workspace.stage[..coded]);
    interleaver::block_deinterleave(coded, usize::from(params.interleave_a), &workspace.stage, &mut workspace.type3);
    workspace.mother[..type2 * 4].fill(0xff);
    convenc::tetra_rcpc_depunct(RcpcPunctMode::Rate2_3, &workspace.type3, coded, &mut workspace.mother);
    viterbi::dec_sb1_with_workspace(
        &workspace.mother[..type2 * 4],
        &mut workspace.decoded,
        type2,
        &mut workspace.viterbi,
    )
    .expect("fixed decoder storage");
    output[..type1].copy_from_slice(&workspace.decoded[..type1]);
    Ok(crc16::crc16_ccitt_bits(&workspace.decoded, type1 + 16) == crc16::TETRA_CRC_OK)
}

/// Existing speech CRC-7 plus overall parity, in the original bit order.
pub fn speech_crc(class2_bits: &[u8; 60]) -> [u8; 8] {
    let mut w = [0; 67];
    for k in 0..60 {
        w[k + 7] = class2_bits[k] & 1;
    }
    for d in (7..67).rev() {
        if w[d] == 1 {
            w[d] ^= 1;
            w[d - 4] ^= 1;
            w[d - 7] ^= 1;
        }
    }
    let mut result = [0; 8];
    result[..7].copy_from_slice(&w[..7]);
    let mut parity = 0;
    for bit in class2_bits {
        parity ^= bit & 1;
    }
    for bit in &result[..7] {
        parity ^= bit;
    }
    result[7] = parity;
    result
}

/// Encode the 274 speech bits already in channel sensitivity-class order.
///
/// The encoder state remains continuous across Class 1 and Class 2.
pub fn encode_speech_channel(input: &[u8; 274], scrambling_code: u32, output: &mut [u8; 432]) -> Result<(), CodingError> {
    check_bits(input)?;
    let mut type3 = [0; 432];
    type3[..102].copy_from_slice(&input[..102]);
    let mut encoder = SpeechConvEncState::new();
    let mut class1_mother = [0; 112 * 3];
    encoder.encode(&input[102..214], &mut class1_mother);
    convenc::get_punctured_rate(RcpcPunctMode::Rate112_168, &class1_mother, &mut type3[102..270]);

    let mut class2 = [0; 72];
    class2[..60].copy_from_slice(&input[214..]);
    let crc = speech_crc((&input[214..]).try_into().unwrap());
    class2[60..68].copy_from_slice(&crc);
    let mut class2_mother = [0; 72 * 3];
    encoder.encode(&class2, &mut class2_mother);
    convenc::get_punctured_rate(RcpcPunctMode::Rate72_162, &class2_mother, &mut type3[270..]);
    interleaver::matrix_interleave(24, 18, &type3, output);
    scrambler::tetra_scramb_bits(scrambling_code, output);
    Ok(())
}

/// Encode a TCH/S frame in codec order.
///
/// Compatibility: short inputs are zero-padded without reordering, exactly as
/// the application encoder did. Full 274-bit inputs are reordered before coding.
/// A stolen half-slot is the caller-selected second 216 bits of the full output.
pub fn encode_tch_s(input: &[u8], scrambling_code: u32, output: &mut [u8; 432]) -> Result<(), CodingError> {
    if input.len() > 274 {
        return Err(CodingError::InputLength);
    }
    let mut channel = [0; 274];
    channel[..input.len()].copy_from_slice(input);
    if input.len() == 274 {
        channel = tch_reorder::codec_to_channel(&channel);
    }
    encode_speech_channel(&channel, scrambling_code, output)
}

/// Decode a full 432-bit TCH/S block into 274 codec-order bits.
///
/// The CRC result covers Class 2 only. Class 0 remains unprotected, and the
/// existing Class 1/Class 2 puncturing and single-pass traceback are preserved.
pub fn decode_tch_s(
    input: &[u8; 432],
    scrambling_code: u32,
    output: &mut [u8; 274],
    workspace: &mut Workspace,
) -> Result<bool, CodingError> {
    check_bits(input)?;
    workspace.stage.copy_from_slice(input);
    scrambler::tetra_scramb_bits(scrambling_code, &mut workspace.stage);
    interleaver::matrix_deinterleave(24, 18, &workspace.stage, &mut workspace.type3);

    workspace.mother[..184 * 3].fill(0xff);
    convenc::tetra_rcpc_depunct(
        RcpcPunctMode::Rate112_168,
        &workspace.type3[102..270],
        168,
        &mut workspace.mother[..336],
    );
    convenc::tetra_rcpc_depunct(
        RcpcPunctMode::Rate72_162,
        &workspace.type3[270..],
        162,
        &mut workspace.mother[336..552],
    );
    viterbi::TetraCodecViterbiDecoder::new()
        .decode_hard_with_workspace(&workspace.mother[..552], &mut workspace.decoded[..184], &mut workspace.viterbi)
        .expect("fixed decoder storage");
    let data: &[u8; 60] = (&workspace.decoded[112..172]).try_into().unwrap();
    let crc_ok = speech_crc(data) == workspace.decoded[172..180];
    let channel: &mut [u8; 274] = (&mut workspace.stage[..274]).try_into().unwrap();
    channel[..102].copy_from_slice(&workspace.type3[..102]);
    channel[102..214].copy_from_slice(&workspace.decoded[..112]);
    channel[214..].copy_from_slice(data);
    tch_reorder::channel_to_codec_into(channel, output);
    Ok(crc_ok)
}

/// Encode the low 14 bits using the existing RM(30,14) code and scrambler.
pub fn encode_aach(input: u16, scrambling_code: u32) -> [u8; 30] {
    let codeword = rm3014::tetra_rm3014_compute(input);
    let mut output = core::array::from_fn(|i| ((codeword >> (29 - i)) & 1) as u8);
    scrambler::tetra_scramb_bits(scrambling_code, &mut output);
    output
}

/// Decode AACH with the original single-bit-only error correction.
pub fn decode_aach(input: &[u8; 30], scrambling_code: u32) -> Result<u16, CodingError> {
    check_bits(input)?;
    let mut bits = *input;
    scrambler::tetra_scramb_bits(scrambling_code, &mut bits);
    let codeword = bits.iter().fold(0, |word, bit| (word << 1) | u32::from(*bit));
    Ok(rm3014::tetra_rm3014_decode_limited_ecc(codeword))
}
