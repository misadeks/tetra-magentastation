use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use toml::Value;

use crate::magentastation::{CfgSoapySdr, SoapySdrDto, cfg_to_soapy_dto};

/// The PHY layer backend type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum PhyBackend {
    Undefined,
    /// No radio transport. The MS process can service offline management only.
    None,
    SoapySdr,
}

/// PHY layer I/O configuration
#[derive(Debug, Clone)]
pub struct CfgPhyIo {
    /// Radio transport, or None for offline MS management and entity tests.
    pub backend: PhyBackend,

    /// For Soapysdr backend: SoapySDR configuration
    pub soapysdr: Option<CfgSoapySdr>,
}

#[derive(Deserialize, Serialize)]
pub struct PhyIoDto {
    pub backend: PhyBackend,

    pub soapysdr: Option<SoapySdrDto>,

    #[serde(flatten, skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, Value>,
}

pub fn phy_dto_to_cfg(src: PhyIoDto) -> Result<CfgPhyIo, String> {
    let soapysdr = src
        .soapysdr
        .map(|soapy_dto| {
            Ok::<_, String>(CfgSoapySdr {
                ul_freq: soapy_dto.rx_freq.unwrap_or(0.0),
                dl_freq: soapy_dto.tx_freq.unwrap_or(0.0),
                dl_freq_seeded: false,
                ppm_err: soapy_dto.ppm_err.unwrap_or(0.0),
                device: soapy_dto.device,
                fs: soapy_dto.sample_rate,
                rx_ch: soapy_dto.rx_channel,
                tx_ch: soapy_dto.tx_channel,
                rx_ant: soapy_dto.rx_antenna,
                tx_ant: soapy_dto.tx_antenna,
                rx_gains: soapy_dto
                    .extra
                    .iter()
                    .filter(|(key, _)| key.starts_with("rx_gain_"))
                    .map(|(key, value)| gain_element(&key["rx_gain_".len()..], value))
                    .collect::<Result<_, _>>()?,
                tx_gains: soapy_dto
                    .extra
                    .iter()
                    .filter(|(key, _)| key.starts_with("tx_gain_"))
                    .map(|(key, value)| gain_element(&key["tx_gain_".len()..], value))
                    .collect::<Result<_, _>>()?,
            })
        })
        .transpose()?;

    Ok(CfgPhyIo {
        backend: src.backend,
        soapysdr,
    })
}

fn gain_element(name: &str, value: &Value) -> Result<(String, f64), String> {
    let number = match value {
        Value::Integer(value) => *value as f64,
        Value::Float(value) => *value,
        _ => return Err("SoapySDR gain must be a number".to_string()),
    };
    Ok((name.to_lowercase(), number))
}

/// Inverse of [`phy_dto_to_cfg`] for TOML write-back (Plane B, non-standard).
pub fn cfg_to_phy_dto(cfg: &CfgPhyIo) -> PhyIoDto {
    PhyIoDto {
        backend: cfg.backend,
        soapysdr: cfg.soapysdr.as_ref().map(cfg_to_soapy_dto),
        extra: HashMap::new(),
    }
}
