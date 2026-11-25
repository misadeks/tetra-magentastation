use tetra_config::magentastation::{CfgCellInfo, CfgMs, CfgNetInfo, CfgPhyIo, PhyBackend, StackConfig, StackMode};
use tetra_core::freqs::FreqInfo;

/// Creates a default config for testing. It can still be modified as needed
/// before passing it to the ComponentTest constructor
pub fn default_test_config_ms() -> StackConfig {
    let phy_io = default_phy_io();
    let net_info = default_net_info();
    let freq_info = FreqInfo::from_components(4, 1521, 0, false, 4, None).unwrap();
    let cell_info = default_cell_info(freq_info);

    // Put together components and return this proto config
    StackConfig {
        stack_mode: StackMode::Ms,
        debug_log: None,
        phy_io,
        net: net_info,
        cell: cell_info,
        duplex_table: Default::default(),
        codeplug: Default::default(),
        ms: Some(default_ms_info()),
        telemetry: None,
        control: None,
    }
}

pub fn default_phy_io() -> CfgPhyIo {
    CfgPhyIo {
        backend: PhyBackend::None,
        soapysdr: None,
    }
}

pub fn default_net_info() -> CfgNetInfo {
    CfgNetInfo { mcc: 204, mnc: 1337 }
}

pub fn default_cell_info(freq_info: FreqInfo) -> CfgCellInfo {
    CfgCellInfo {
        colour_code: 1,
        location_area: 2,
        main_carrier: freq_info.carrier,
        freq_band: freq_info.band,
        freq_offset_hz: freq_info.freq_offset_hz,
        duplex_spacing_id: freq_info.duplex_spacing_id,
        custom_duplex_spacing: None,
        reverse_operation: freq_info.reverse_operation,
    }
}

pub fn default_ms_info() -> CfgMs {
    CfgMs {
        issi: 1000001,
        subscriber_class: 1,
        attach_groups: vec![],
    }
}
