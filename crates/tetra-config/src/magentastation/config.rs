use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use tetra_core::freqs::DuplexTable;

use crate::magentastation::{CfgCellInfo, CfgCodeplug, CfgControl, CfgMs, CfgNetInfo, CfgPhyIo, PhyBackend, StackState};

use super::sec_telemetry::CfgTelemetry;

/// Wrapper for a string that should be treated as a secret. Display and Debug will redact the actual value,
/// to prevent accidental logging of secrets.
#[derive(Clone)]
pub struct SecretField {
    pub val: String,
}

impl From<String> for SecretField {
    fn from(val: String) -> Self {
        Self { val }
    }
}

impl From<SecretField> for String {
    fn from(secret: SecretField) -> Self {
        secret.val
    }
}

impl AsRef<str> for SecretField {
    fn as_ref(&self) -> &str {
        &self.val
    }
}

impl std::fmt::Display for SecretField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "********")
    }
}

impl std::fmt::Debug for SecretField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretField").field("val", &"********").finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum StackMode {
    Ms,
}

#[derive(Debug, Clone)]
pub struct StackConfig {
    pub stack_mode: StackMode,
    pub debug_log: Option<String>,

    pub phy_io: CfgPhyIo,
    pub net: CfgNetInfo,
    pub cell: CfgCellInfo,

    /// Programmable 8-entry duplex-spacing table (TS 100 392-15 cl. 6). Defaults
    /// to the ETSI spec table when no `[duplex_table]` section is configured.
    pub duplex_table: DuplexTable,

    /// Radio-style codeplug (folders/talkgroups/networks/carrier_overrides/scan) — **Plane B**.
    /// Empty for legacy MS configs. Data only: read by the
    /// management/TNMM UI and the cell-selection engine; does not itself change
    /// on-air behaviour.
    pub codeplug: CfgCodeplug,

    /// Mobile Station configuration. Required when `stack_mode == Ms`.
    pub ms: Option<CfgMs>,

    /// Telemetry endpoint configuration
    pub telemetry: Option<CfgTelemetry>,

    /// Control endpoint configuration
    pub control: Option<CfgControl>,
}

impl StackConfig {
    /// Initial affiliation set shared by startup validation and the MM adapter.
    pub fn initial_groups(&self) -> Result<tetra_ms_core::mm::GroupSet, &'static str> {
        let mut groups = tetra_ms_core::mm::GroupSet::new();
        let base = self.ms.as_ref().map(|ms| ms.attach_groups.as_slice()).unwrap_or_default();
        for gssi in base.iter().copied().chain(self.codeplug.default_active_scanlist_gssis()) {
            groups.insert(gssi).map_err(|error| match error {
                tetra_ms_core::mm::Error::GroupCapacity => "initial affiliation exceeds 63 unique groups",
                _ => "initial group identity must fit in 24 bits",
            })?;
        }
        Ok(groups)
    }

    /// Validate that all required configuration fields are properly set.
    pub fn validate(&self) -> Result<(), &str> {
        // Check input device settings
        match self.phy_io.backend {
            PhyBackend::SoapySdr => {
                if self.phy_io.soapysdr.is_none() {
                    return Err("soapysdr configuration must be provided for Soapysdr backend");
                };
            }
            PhyBackend::None => {} // For testing
            PhyBackend::Undefined => {
                return Err("phy_io backend must be defined");
            }
        };

        // Sanity check on main carrier property fields in SYSINFO
        if self.phy_io.backend == PhyBackend::SoapySdr {
            let soapy_cfg = self
                .phy_io
                .soapysdr
                .as_ref()
                .expect("SoapySdr config must be set for SoapySdr PhyIo");

            let has_rx_source = soapy_cfg.dl_freq > 0.0 || !self.codeplug.scan_candidate_frequencies().is_empty();
            if !has_rx_source {
                return Err("MS mode requires an RX source: program at least one [[frequency_list]] \
                     or set [phy_io.soapysdr] tx_freq");
            }
        }

        // Validate the codeplug (Plane B). Empty codeplug is a no-op.
        // Note: kept as an owned String on the error path, mapped to &str below.
        if let Err(_e) = self.codeplug.validate() {
            return Err("Invalid codeplug configuration");
        }

        // MS mode requires an [ms] section with a valid own ISSI.
        if self.stack_mode == StackMode::Ms {
            let Some(ref ms) = self.ms else {
                return Err("[ms] configuration section is required when stack_mode = \"Ms\"");
            };
            if ms.issi == 0 || ms.issi > 0xFF_FFFF {
                return Err("ms.issi must be a valid 24-bit ISSI (1..=16777215)");
            }
            if ms.subscriber_class < 1 || ms.subscriber_class > 16 {
                return Err("ms.subscriber_class must be in range 1-16");
            }
            tetra_ms_core::mm::Config {
                issi: ms.issi,
                mcc: self.net.mcc,
                mnc: self.net.mnc,
                initial_la: self.cell.location_area,
            }
            .validate()
            .map_err(|_| "MS MCC/MNC/location area exceeds portable wire bounds")?;
            self.initial_groups()?;
        }

        Ok(())
    }
}

/// Global shared configuration: hot-swappable config + mutable state.
///
/// `cfg` is held behind a `RwLock<Arc<..>>` so the whole configuration can be
/// atomically hot-swapped at runtime (Plane B `SetConfig` live apply) while every
/// `SharedConfig` clone — held by all entities — observes the new value on its
/// next [`SharedConfig::config`] call. Reads clone the inner `Arc` (cheap); the
/// swap is rare (only on an operational `SetConfig`). Structural changes still
/// require a restart; only operational (codeplug) changes are hot-swapped.
#[derive(Clone)]
pub struct SharedConfig {
    /// Hot-swappable configuration snapshot.
    cfg: Arc<RwLock<Arc<StackConfig>>>,
    /// Mutable state guarded with RwLock (write by the stack, read by others).
    state: Arc<RwLock<StackState>>,
}

impl SharedConfig {
    pub fn from_parts(cfg: StackConfig, state: Option<StackState>) -> Self {
        // Check config for validity before returning the SharedConfig object
        match cfg.validate() {
            Ok(_) => {}
            Err(e) => panic!("Invalid stack configuration: {}", e),
        }

        Self {
            cfg: Arc::new(RwLock::new(Arc::new(cfg))),
            state: Arc::new(RwLock::new(state.unwrap_or_default())),
        }
    }

    /// Access the current configuration snapshot. Returns an `Arc` so callers get
    /// a stable view even if a concurrent live apply swaps the config mid-read.
    pub fn config(&self) -> Arc<StackConfig> {
        Arc::clone(&self.cfg.read().expect("StackConfig RwLock poisoned"))
    }

    /// Hot-swap the whole configuration (Plane B `SetConfig` live apply). Every
    /// `SharedConfig` clone observes `new` on its next [`config`](Self::config)
    /// call. Used only for operational (codeplug) changes; structural changes
    /// must go through a restart. Panics if `new` fails validation (the caller is
    /// expected to have validated it already).
    pub fn apply_config_live(&self, new: StackConfig) {
        if let Err(e) = new.validate() {
            panic!("apply_config_live: refusing to install invalid configuration: {}", e);
        }
        *self.cfg.write().expect("StackConfig RwLock poisoned") = Arc::new(new);
    }

    /// Read guard for mutable state.
    pub fn state_read(&self) -> std::sync::RwLockReadGuard<'_, StackState> {
        self.state.read().expect("StackState RwLock blocked")
    }

    /// Write guard for mutable state.
    pub fn state_write(&self) -> std::sync::RwLockWriteGuard<'_, StackState> {
        self.state.write().expect("StackState RwLock blocked")
    }
}
