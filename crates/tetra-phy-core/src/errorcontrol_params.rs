/// Existing logical-channel coding parameters, independent of desktop primitives.
#[derive(Debug, Eq, PartialEq)]
pub struct ErrorControlParams {
    // pub name:           &'static str,
    pub type345_bits: u16,
    pub type2_bits: u16,
    pub type1_bits: u16,
    pub interleave_a: u16,
    pub have_crc16: bool,
}

/// Parameters for the BSCH (Broadcast Synchronization Channel)
pub const BSCH_PARAMS: ErrorControlParams = ErrorControlParams {
    type345_bits: 120,
    type2_bits: 80,
    type1_bits: 60,
    interleave_a: 11,
    have_crc16: true,
};

/// Parameters for the SCH/HD (half slot) signalling channel, also for STCH and BNCH
pub const SCH_HD_PARAMS: ErrorControlParams = ErrorControlParams {
    type345_bits: 216,
    type2_bits: 144,
    type1_bits: 124,
    interleave_a: 101,
    have_crc16: true,
};

/// Parameters for the BBK (Broadcast Block) channel, used for AACH
pub const AACH_PARAMS: ErrorControlParams = ErrorControlParams {
    type345_bits: 30,
    type2_bits: 30,
    type1_bits: 14,
    interleave_a: 0, // No interleaving
    have_crc16: false,
};

/// Parameters for the SCH/F channel
pub const SCH_F_PARAMS: ErrorControlParams = ErrorControlParams {
    type345_bits: 432,
    type2_bits: 288,
    type1_bits: 268,
    interleave_a: 103,
    have_crc16: true,
};

/// Parameters for the SCH/HU (half slot uplink, Control Uplink Burst) channel
pub const SCH_HU_PARAMS: ErrorControlParams = ErrorControlParams {
    type345_bits: 168,
    type2_bits: 112,
    type1_bits: 92,
    interleave_a: 13,
    have_crc16: true,
};

/// Parameters for TCH/S (full-rate 7.2 kbit/s speech).
///
/// These legacy generic dimensions are retained for desktop channel lookup.
/// Speech coding uses the per-class CRC and continuous encoder in
/// [`crate::errorcontrol`], not the generic control-channel pipeline.
pub const TCH_S_PARAMS: ErrorControlParams = ErrorControlParams {
    type345_bits: 432,
    type2_bits: 288,
    type1_bits: 274,
    interleave_a: 103,
    have_crc16: false,
};
