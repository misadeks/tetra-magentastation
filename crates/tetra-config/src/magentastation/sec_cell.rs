use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use toml::Value;

/// Optional legacy MS seed values. Nondefault seeds survive canonical write-back;
/// the serving cell supplies current identity and RF parameters over the air.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CfgCellInfo {
    pub main_carrier: u16,
    pub freq_band: u8,
    pub freq_offset_hz: i16,
    pub duplex_spacing_id: u8,
    pub custom_duplex_spacing: Option<u32>,
    pub reverse_operation: bool,
    pub location_area: u16,
    pub colour_code: u8,
}

#[derive(Default, Deserialize, Serialize)]
pub struct CellInfoDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_carrier: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub freq_band: Option<u8>,
    pub freq_offset: Option<i16>,
    pub duplex_spacing: Option<u8>,
    pub reverse_operation: Option<bool>,
    pub custom_duplex_spacing: Option<u32>,
    pub location_area: u16,
    pub colour_code: Option<u8>,
    #[serde(flatten, skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, Value>,
}

pub fn cell_dto_to_cfg(ci: CellInfoDto) -> CfgCellInfo {
    CfgCellInfo {
        main_carrier: ci.main_carrier.unwrap_or(0),
        freq_band: ci.freq_band.unwrap_or(0),
        freq_offset_hz: ci.freq_offset.unwrap_or(0),
        duplex_spacing_id: ci.duplex_spacing.unwrap_or(0),
        custom_duplex_spacing: ci.custom_duplex_spacing,
        reverse_operation: ci.reverse_operation.unwrap_or(false),
        location_area: ci.location_area,
        colour_code: ci.colour_code.unwrap_or(0),
    }
}

pub fn cfg_to_cell_dto(cfg: &CfgCellInfo) -> Option<CellInfoDto> {
    if cfg == &CfgCellInfo::default() {
        return None;
    }
    Some(CellInfoDto {
        main_carrier: (cfg.main_carrier != 0).then_some(cfg.main_carrier),
        freq_band: (cfg.freq_band != 0).then_some(cfg.freq_band),
        freq_offset: (cfg.freq_offset_hz != 0).then_some(cfg.freq_offset_hz),
        duplex_spacing: (cfg.duplex_spacing_id != 0).then_some(cfg.duplex_spacing_id),
        reverse_operation: cfg.reverse_operation.then_some(true),
        custom_duplex_spacing: cfg.custom_duplex_spacing,
        location_area: cfg.location_area,
        colour_code: (cfg.colour_code != 0).then_some(cfg.colour_code),
        extra: HashMap::new(),
    })
}
