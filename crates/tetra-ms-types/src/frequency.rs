//! Checked TETRA carrier and duplex arithmetic, independent of RF hardware.
use core::fmt;

/// ETSI TS 100 392-15 V1.5.1 (2011-02), clause 6: Duplex spacing
const TETRA_DUPLEX_SPACING: [[Option<u32>; 16]; 8] = [
    [
        None,
        Some(1600),
        Some(10000),
        Some(10000),
        Some(10000),
        Some(10000),
        Some(10000),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    ],
    [
        None,
        Some(4500),
        None,
        Some(36000),
        Some(7000),
        None,
        None,
        None,
        Some(45000),
        Some(45000),
        None,
        None,
        None,
        None,
        None,
        None,
    ],
    [
        Some(0),
        Some(0),
        Some(0),
        Some(0),
        Some(0),
        Some(0),
        Some(0),
        Some(0),
        Some(0),
        Some(0),
        Some(0),
        Some(0),
        Some(0),
        Some(0),
        Some(0),
        Some(0),
    ],
    [
        None,
        None,
        None,
        Some(8000),
        Some(8000),
        None,
        None,
        None,
        Some(18000),
        Some(18000),
        None,
        None,
        None,
        None,
        None,
        None,
    ],
    [
        None,
        None,
        None,
        Some(18000),
        Some(5000),
        None,
        Some(30000),
        Some(30000),
        None,
        Some(39000),
        None,
        None,
        None,
        None,
        None,
        None,
    ],
    [
        None,
        None,
        None,
        None,
        Some(9500),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    ],
    [
        None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None,
    ],
    [
        None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None,
    ],
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrequencyError {
    Band,
    Carrier,
    Offset,
    DuplexIndex,
    OutOfRange,
}

impl fmt::Display for FrequencyError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(match self {
            Self::Band => "invalid frequency band",
            Self::Carrier => "invalid carrier number",
            Self::Offset => "invalid frequency offset",
            Self::DuplexIndex => "invalid duplex index",
            Self::OutOfRange => "frequency outside the u32 Hz range",
        })
    }
}

impl core::error::Error for FrequencyError {}

pub const fn offset_hz(index: u8) -> Option<i16> {
    match index {
        0 => Some(0),
        1 => Some(6250),
        2 => Some(-6250),
        3 => Some(12500),
        _ => None,
    }
}

pub const fn offset_index(hz: i16) -> Option<u8> {
    match hz {
        0 => Some(0),
        6250 => Some(1),
        -6250 => Some(2),
        12500 => Some(3),
        _ => None,
    }
}

pub fn default_duplex_spacing(band: u8, index: u8) -> Result<Option<u32>, FrequencyError> {
    if index >= 8 {
        return Err(FrequencyError::DuplexIndex);
    }
    if band >= 16 {
        return Err(FrequencyError::Band);
    }
    Ok(TETRA_DUPLEX_SPACING[usize::from(index)][usize::from(band)].map(|khz| khz * 1000))
}

/// Resolve the original precedence: per-channel override, programmed index,
/// then the standardized table. An undefined standardized entry remains None.
pub fn duplex_spacing(
    band: u8,
    index: u8,
    channel_override: Option<u32>,
    programmed: &[Option<u32>; 8],
) -> Result<Option<u32>, FrequencyError> {
    if index >= 8 {
        return Err(FrequencyError::DuplexIndex);
    }
    if band >= 16 {
        return Err(FrequencyError::Band);
    }
    if let Some(hz) = channel_override.or(programmed[usize::from(index)]) {
        return Ok(Some(hz));
    }
    default_duplex_spacing(band, index)
}

pub fn frequencies(band: u8, carrier: u16, offset: i16, reverse: bool, spacing_hz: u32) -> Result<(u32, u32), FrequencyError> {
    if band > 8 {
        return Err(FrequencyError::Band);
    }
    if carrier >= 4000 {
        return Err(FrequencyError::Carrier);
    }
    if offset_index(offset).is_none() {
        return Err(FrequencyError::Offset);
    }
    let downlink = (100_000_000 * u32::from(band) + 25_000 * u32::from(carrier))
        .checked_add_signed(i32::from(offset))
        .ok_or(FrequencyError::OutOfRange)?;
    let uplink = if reverse {
        downlink.checked_add(spacing_hz)
    } else {
        downlink.checked_sub(spacing_hz)
    }
    .ok_or(FrequencyError::OutOfRange)?;
    Ok((downlink, uplink))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_carrier_and_programmed_duplex_vectors() {
        let mut table = [None; 8];
        assert_eq!(duplex_spacing(4, 5, None, &table), Ok(Some(9_500_000)));
        table[5] = Some(9_400_000);
        assert_eq!(duplex_spacing(4, 5, None, &table), Ok(Some(9_400_000)));
        assert_eq!(duplex_spacing(4, 5, Some(9_500_000), &table), Ok(Some(9_500_000)));
        assert_eq!(frequencies(4, 1001, 0, false, 9_400_000), Ok((425_025_000, 415_625_000)));
        assert_eq!(frequencies(4, 1001, -6250, true, 9_400_000), Ok((425_018_750, 434_418_750)));
    }

    #[test]
    fn malformed_fields_and_frequency_overflow_are_explicit() {
        assert_eq!(default_duplex_spacing(16, 0), Err(FrequencyError::Band));
        assert_eq!(default_duplex_spacing(4, 8), Err(FrequencyError::DuplexIndex));
        assert_eq!(frequencies(9, 0, 0, false, 0), Err(FrequencyError::Band));
        assert_eq!(frequencies(4, 4000, 0, false, 0), Err(FrequencyError::Carrier));
        assert_eq!(frequencies(4, 0, 1, false, 0), Err(FrequencyError::Offset));
        assert_eq!(frequencies(0, 0, -6250, false, 0), Err(FrequencyError::OutOfRange));
        assert_eq!(frequencies(0, 0, 0, false, 1), Err(FrequencyError::OutOfRange));
        assert_eq!(frequencies(4, 0, 0, true, u32::MAX), Err(FrequencyError::OutOfRange));
    }
}
