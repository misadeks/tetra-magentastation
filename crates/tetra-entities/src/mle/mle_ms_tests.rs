use super::*;
use tetra_config::magentastation::from_toml_str;
use tetra_pdus::mle::pdus::{d_mle_sync::DMleSync, d_mle_sysinfo::DMleSysinfo};
use tetra_saps::lmm::LmmMleIdentitiesReq;

const MS_TOML: &str = r#"
config_version = "0.6"
stack_mode = "Ms"

[phy_io]
backend = "SoapySdr"

[phy_io.soapysdr]
tx_freq = 439825000
rx_freq = 430425000
ppm_err = 0
device = "driver=sx"
sample_rate = 600000
rx_antenna = "RX"
tx_antenna = "TX"
rx_gain_lna = 48.0
rx_gain_pga = 8.0
tx_gain_dac = 0.0
tx_gain_mixer = 0.0

[net_info]
mcc = 901
mnc = 9999

[cell_info]
freq_band = 4
main_carrier = 1593
duplex_spacing = 7
custom_duplex_spacing = 9400000
freq_offset = 0
reverse_operation = false
location_area = 1
colour_code = 1

[ms]
issi = 1000001
subscriber_class = 1
attach_groups = []
"#;

fn ms_mle() -> MleMs {
    let cfg = from_toml_str(MS_TOML).expect("valid MS test config");
    MleMs::new(SharedConfig::from_parts(cfg, None))
}

/// Put the MLE into the "camped and confirmed" state a real run reaches
/// after cell selection.
fn camp(mle: &mut MleMs) {
    let mut queue = MessageQueue::new();
    mle.rx_tlmb_prim(&mut queue, sync_ind(901, 9999));
    mle.rx_tlmb_prim(&mut queue, sysinfo_ind(1, 1));
}

/// A downlink failure indication makes the MLE declare out-of-service, emit
/// MLE-BREAK to CMCE, clear the serving cell, and re-arm cell selection
/// (cl. 18.3.4.5.3).
#[test]
fn test_downlink_failure_declares_out_of_service_and_breaks() {
    let mut mle = ms_mle();
    camp(&mut mle);
    let mut queue = MessageQueue::new();

    mle.rx_tlmb_monitor_ind(&mut queue, false);

    assert!(mle.core.state().out_of_service, "declared out of service");
    assert!(mle.core.state().serving_cell.is_none(), "serving cell cleared");
    assert!(!mle.core.state().activate_confirmed, "activate confirmation re-armed");

    let breaks: Vec<_> = queue.iter().filter(|m| matches!(m.msg, SapMsgInner::LcmcMleBreakInd(_))).collect();
    assert_eq!(breaks.len(), 1, "exactly one MLE-BREAK");
    assert_eq!(breaks[0].dest, TetraEntity::Cmce, "MLE-BREAK addressed to CMCE");

    // The break is ALSO routed to MM over the LMM-SAP so MM can go out of
    // service and inform the TNMM user (cl. 18.3.3 / 15.3.4).
    let mm_breaks: Vec<_> = queue.iter().filter(|m| matches!(m.msg, SapMsgInner::LmmMleBreakInd(_))).collect();
    assert_eq!(mm_breaks.len(), 1, "exactly one LMM MLE-BREAK");
    assert_eq!(mm_breaks[0].dest, TetraEntity::Mm, "LMM MLE-BREAK addressed to MM");
}

/// A second failure indication while already out of service is idempotent:
/// no further MLE-BREAK.
#[test]
fn test_repeated_failure_is_idempotent() {
    let mut mle = ms_mle();
    camp(&mut mle);
    let mut queue = MessageQueue::new();

    mle.rx_tlmb_monitor_ind(&mut queue, false);
    mle.rx_tlmb_monitor_ind(&mut queue, false);

    let breaks = queue.iter().filter(|m| matches!(m.msg, SapMsgInner::LcmcMleBreakInd(_))).count();
    assert_eq!(breaks, 1, "MLE-BREAK emitted only once");
}

/// Recovery after a failure emits MLE-REOPEN to CMCE and clears the
/// out-of-service state (cl. 18.3.4.7).
#[test]
fn test_recovery_emits_reopen() {
    let mut mle = ms_mle();
    camp(&mut mle);
    let mut queue = MessageQueue::new();

    mle.rx_tlmb_monitor_ind(&mut queue, false);
    mle.rx_tlmb_monitor_ind(&mut queue, true);

    assert!(!mle.core.state().out_of_service, "back in service after recovery");
    let reopens: Vec<_> = queue.iter().filter(|m| matches!(m.msg, SapMsgInner::LcmcMleReopenInd(_))).collect();
    assert_eq!(reopens.len(), 1, "exactly one MLE-REOPEN");
    assert_eq!(reopens[0].dest, TetraEntity::Cmce, "MLE-REOPEN addressed to CMCE");

    // Reopen is ALSO routed to MM over the LMM-SAP (restores service view).
    let mm_reopens: Vec<_> = queue.iter().filter(|m| matches!(m.msg, SapMsgInner::LmmMleReopenInd(_))).collect();
    assert_eq!(mm_reopens.len(), 1, "exactly one LMM MLE-REOPEN");
    assert_eq!(mm_reopens[0].dest, TetraEntity::Mm, "LMM MLE-REOPEN addressed to MM");
}

/// A monitoring refresh carrying a signal level caches it as the serving-cell
/// RSSI (reselection input, cl. 18.3.4 / management-UI readout); a subsequent
/// downlink failure clears it (no serving-cell signal while out of service).
#[test]
fn test_serving_cell_rssi_cached_and_cleared() {
    use tetra_saps::tlmb::TlmbMonitorInd;
    let mut mle = ms_mle();
    camp(&mut mle);
    let mut queue = MessageQueue::new();

    let monitor = |downlink_available, rssi_dbfs| SapMsg {
        sap: Sap::TlmbSap,
        src: TetraEntity::Phy,
        dest: TetraEntity::Mle,
        msg: SapMsgInner::TlmbMonitorInd(TlmbMonitorInd {
            downlink_available,
            rssi_dbfs,
        }),
    };

    mle.rx_tlmb_prim(&mut queue, monitor(true, Some(-12.5)));
    assert_eq!(mle.core.state().serving_cell_rssi_dbfs, Some(-12.5), "level cached from refresh");

    mle.rx_tlmb_prim(&mut queue, monitor(false, None));
    assert_eq!(mle.core.state().serving_cell_rssi_dbfs, None, "cleared on out of service");
}

/// An "available" indication while in service (initial acquisition) is a
/// no-op: no MLE-REOPEN, still in service.
#[test]
fn test_available_while_in_service_is_noop() {
    let mut mle = ms_mle();
    camp(&mut mle);
    let mut queue = MessageQueue::new();

    mle.rx_tlmb_monitor_ind(&mut queue, true);

    assert!(!mle.core.state().out_of_service);
    assert!(mle.core.state().serving_cell.is_some(), "serving cell untouched");
    assert_eq!(
        queue.iter().filter(|m| matches!(m.msg, SapMsgInner::LcmcMleReopenInd(_))).count(),
        0,
        "no reopen when never broken"
    );
}

/// G1 (cl. 17.3.2 / 23.4.1.2.1): an MLE-IDENTITIES request from MM makes the
/// MLE configure the MAC downlink address filter, forwarding the own ISSI and
/// the full attached-group set to UMAC in a TL-CONFIGURE.
#[test]
fn test_mle_identities_configures_mac_filter() {
    let mut mle = ms_mle();
    let mut queue = MessageQueue::new();

    let msg = SapMsg {
        sap: Sap::LmmSap,
        src: TetraEntity::Mm,
        dest: TetraEntity::Mle,
        msg: SapMsgInner::LmmMleIdentitiesReq(LmmMleIdentitiesReq {
            issi: 1000001,
            assi: None,
            attached_gssis: vec![91, 220],
            detached_gssis: vec![],
        }),
    };
    mle.rx_lmm_prim(&mut queue, msg);

    let out = queue.pop_front().expect("a TL-CONFIGURE must be emitted");
    assert_eq!(out.sap, Sap::TlmcSap);
    assert_eq!(out.dest, TetraEntity::Umac);
    let SapMsgInner::TlmcConfigureReq(req) = out.msg else {
        panic!("expected TlmcConfigureReq");
    };
    let va = req.valid_addresses.expect("valid addresses set");
    assert_eq!(va.individual_ssi, Some(1000001));
    assert_eq!(va.group_ssis, Some(vec![91, 220]));
    // MCC/MNC are the configured home network so the element stays well-formed.
    assert_eq!(va.mcc, 901);
    assert_eq!(va.mnc, 9999);
}

use tetra_pdus::mle::fields::bs_service_details::BsServiceDetails;
use tetra_saps::tlmb::{TlmbSyncInd, TlmbSysinfoInd};

/// Build a D-MLE-SYNC monitoring indication (cl. 18.4.2.1) for a cell.
fn sync_ind(mcc: u16, mnc: u16) -> SapMsg {
    let mut tl_sdu = BitBuffer::new(64);
    DMleSync {
        mcc,
        mnc,
        neighbor_cell_broadcast: 0,
        cell_load_ca: 0,
        late_entry_supported: false,
    }
    .to_bitbuf(&mut tl_sdu);
    tl_sdu.seek(0);
    SapMsg {
        sap: Sap::TlmbSap,
        src: TetraEntity::Phy,
        dest: TetraEntity::Mle,
        msg: SapMsgInner::TlmbSyncInd(TlmbSyncInd { endpoint_id: 0, tl_sdu }),
    }
}

/// Build a D-MLE-SYSINFO monitoring indication (cl. 18.4.2.2) carrying a
/// subscriber-class bitmap; all BS service-detail flags off except
/// registration + system-wide-services.
fn sysinfo_ind(location_area: u16, subscriber_class: u16) -> SapMsg {
    let bs_service_details = BsServiceDetails {
        registration: true,
        deregistration: false,
        priority_cell: false,
        no_minimum_mode: false,
        migration: false,
        system_wide_services: true,
        voice_service: false,
        circuit_mode_data_service: false,
        sndcp_service: false,
        aie_service: false,
        advanced_link: false,
    };
    let mut tl_sdu = BitBuffer::new(64);
    DMleSysinfo {
        location_area,
        subscriber_class,
        bs_service_details,
    }
    .to_bitbuf(&mut tl_sdu);
    tl_sdu.seek(0);
    SapMsg {
        sap: Sap::TlmbSap,
        src: TetraEntity::Phy,
        dest: TetraEntity::Mle,
        msg: SapMsgInner::TlmbSysinfoInd(TlmbSysinfoInd {
            endpoint_id: 0,
            tl_sdu,
            mac_broadcast_info: None,
        }),
    }
}

/// Suitability (cl. 18.3.4): a SYNC for the home network is selected â€” the
/// serving cell is adopted and the MAC is configured with its scrambling.
#[test]
fn test_allowed_home_network_selected() {
    let mut mle = ms_mle();
    let mut queue = MessageQueue::new();

    mle.rx_tlmb_prim(&mut queue, sync_ind(901, 9999));

    assert!(mle.core.state().serving_cell.is_some(), "home cell selected");
    assert_eq!(
        queue.iter().filter(|m| matches!(m.msg, SapMsgInner::TlmcConfigureReq(_))).count(),
        1,
        "MAC configured for the selected cell"
    );
}

/// Suitability (cl. 18.3.4): a SYNC for a foreign, non-programmed network is
/// rejected â€” no cell is selected and the MAC is not configured.
#[test]
fn test_disallowed_network_not_selected() {
    let mut mle = ms_mle();
    let mut queue = MessageQueue::new();

    mle.rx_tlmb_prim(&mut queue, sync_ind(238, 6));

    assert!(mle.core.state().serving_cell.is_none(), "foreign cell not selected");
    assert_eq!(
        queue.iter().filter(|m| matches!(m.msg, SapMsgInner::TlmcConfigureReq(_))).count(),
        0,
        "MAC not configured for a disallowed network"
    );
}

/// Suitability (cl. 18.4.2.2): once camped, a SYSINFO whose subscriber-class
/// bitmap admits our class (1 => bit 0) confirms the cell to MM.
#[test]
fn test_subscriber_class_permitted_confirms() {
    let mut mle = ms_mle();
    let mut queue = MessageQueue::new();
    mle.rx_tlmb_prim(&mut queue, sync_ind(901, 9999));

    // class 1 permitted => bit 0 set.
    mle.rx_tlmb_prim(&mut queue, sysinfo_ind(1, 0b1));

    assert!(mle.core.state().activate_confirmed, "cell confirmed");
    assert_eq!(
        queue.iter().filter(|m| matches!(m.msg, SapMsgInner::LmmMleActivateConf(_))).count(),
        1,
        "LMM-ACTIVATE confirmation sent to MM"
    );
}

/// Suitability (cl. 18.4.2.2): a SYSINFO whose subscriber-class bitmap does
/// NOT admit our class leaves the cell unconfirmed â€” no registration.
#[test]
fn test_subscriber_class_not_permitted_not_confirmed() {
    let mut mle = ms_mle();
    let mut queue = MessageQueue::new();
    mle.rx_tlmb_prim(&mut queue, sync_ind(901, 9999));

    // class 1 NOT permitted (bit 0 clear; only classes 2 and 3 allowed).
    mle.rx_tlmb_prim(&mut queue, sysinfo_ind(1, 0b110));

    assert!(!mle.core.state().activate_confirmed, "cell not confirmed for a barred class");
    assert_eq!(
        queue.iter().filter(|m| matches!(m.msg, SapMsgInner::LmmMleActivateConf(_))).count(),
        0,
        "no LMM-ACTIVATE confirmation for a barred subscriber class"
    );
}

/// D-2 (tune plumbing): the MLE `request_tune` helper emits a TMC-SAP
/// TlmcTuneReq addressed to UMAC carrying the requested carrier, so the
/// scanning engine can step the receiver across candidate carriers.
#[test]
fn test_request_tune_emits_tlmc_tune_req() {
    let mut mle = ms_mle();
    let mut queue = MessageQueue::new();

    mle.rx_lmm_prim(&mut queue, camp_req(396_000_000, false));

    let out = queue.pop_front().expect("a TLMC-TUNE must be emitted");
    assert_eq!(out.sap, Sap::TlmcSap);
    assert_eq!(out.src, TetraEntity::Mle);
    assert_eq!(out.dest, TetraEntity::Umac);
    let SapMsgInner::TlmcTuneReq(req) = out.msg else {
        panic!("expected TlmcTuneReq");
    };
    assert_eq!(req.carrier_hz, 396_000_000);
}

// ----------------------------------------------------------------------
// Phase D-3: scanning cell-selection engine (ETSI TS 100 392-2 cl. 18.3.4)
// ----------------------------------------------------------------------

/// A List-mode scan codeplug programming two candidate downlink carriers.
/// (Home network 901/9999 stays always-allowed; no extra allowed networks.)
const MS_TOML_SCAN: &str = r#"
config_version = "0.6"
stack_mode = "Ms"

[phy_io]
backend = "SoapySdr"

[phy_io.soapysdr]
tx_freq = 439825000
rx_freq = 430425000
ppm_err = 0
device = "driver=sx"
sample_rate = 600000
rx_antenna = "RX"
tx_antenna = "TX"
rx_gain_lna = 48.0
rx_gain_pga = 8.0
tx_gain_dac = 0.0
tx_gain_mixer = 0.0

[net_info]
mcc = 901
mnc = 9999

[cell_info]
freq_band = 4
main_carrier = 1593
duplex_spacing = 7
custom_duplex_spacing = 9400000
freq_offset = 0
reverse_operation = false
location_area = 1
colour_code = 1

[ms]
issi = 1000001
subscriber_class = 1
attach_groups = []

[[frequency_list]]
name = "scan"
mode = "List"
frequencies = [390000000, 396000000]
dwell_ms = 500
"#;

/// Two candidate carriers programmed in the scan config above.
const SCAN_F0: u32 = 390_000_000;
const SCAN_F1: u32 = 396_000_000;

fn ms_mle_scan() -> MleMs {
    let cfg = from_toml_str(MS_TOML_SCAN).expect("valid MS scan test config");
    MleMs::new(SharedConfig::from_parts(cfg, None))
}

/// A PHY scan-dwell-elapsed heartbeat (no carrier on the current candidate).
fn scan_dwell_ind() -> SapMsg {
    use tetra_saps::tlmb::TlmbScanDwellInd;
    SapMsg {
        sap: Sap::TlmbSap,
        src: TetraEntity::Phy,
        dest: TetraEntity::Mle,
        msg: SapMsgInner::TlmbScanDwellInd(TlmbScanDwellInd { rssi_dbfs: Some(-95.0) }),
    }
}

/// A PHY downlink-monitoring indication (carrier locked / lock lost). The PHY
/// emits these once demod-synchronized instead of the scan-dwell heartbeat.
fn monitor_ind(downlink_available: bool) -> SapMsg {
    use tetra_saps::tlmb::TlmbMonitorInd;
    SapMsg {
        sap: Sap::TlmbSap,
        src: TetraEntity::Phy,
        dest: TetraEntity::Mle,
        msg: SapMsgInner::TlmbMonitorInd(TlmbMonitorInd {
            downlink_available,
            rssi_dbfs: Some(-70.0),
        }),
    }
}

/// The single tune carrier emitted since the queue was last drained, or
/// `None` if no TlmcTuneReq is present.
fn tuned_carrier(queue: &MessageQueue) -> Option<u32> {
    queue.iter().find_map(|m| match &m.msg {
        SapMsgInner::TlmcTuneReq(t) => Some(t.carrier_hz),
        _ => None,
    })
}

/// D-3: the first scan-dwell heartbeat with no serving cell starts the scan
/// and retunes to the first programmed candidate (cl. 18.3.4).
#[test]
fn test_scan_starts_on_first_dwell() {
    let mut mle = ms_mle_scan();
    let mut queue = MessageQueue::new();

    mle.rx_tlmb_prim(&mut queue, scan_dwell_ind());

    assert!(mle.core.state().scanning, "scan started");
    assert_eq!(tuned_carrier(&queue), Some(SCAN_F0), "tuned to first candidate");
}

/// D-3: subsequent dwell heartbeats step through the candidate list and wrap.
#[test]
fn test_scan_advances_and_wraps_on_dwell() {
    let mut mle = ms_mle_scan();
    let mut queue = MessageQueue::new();

    mle.rx_tlmb_prim(&mut queue, scan_dwell_ind()); // start -> F0
    queue = MessageQueue::new();
    mle.rx_tlmb_prim(&mut queue, scan_dwell_ind()); // -> F1
    assert_eq!(tuned_carrier(&queue), Some(SCAN_F1), "advanced to second candidate");
    queue = MessageQueue::new();
    mle.rx_tlmb_prim(&mut queue, scan_dwell_ind()); // wrap -> F0
    assert_eq!(tuned_carrier(&queue), Some(SCAN_F0), "wrapped back to first candidate");
}

/// D-3: a candidate carrying a cell on a disallowed network is unsuitable, so
/// the scan advances to the next candidate (cl. 18.3.4).
#[test]
fn test_scan_advances_on_disallowed_network() {
    let mut mle = ms_mle_scan();
    let mut queue = MessageQueue::new();

    mle.rx_tlmb_prim(&mut queue, scan_dwell_ind()); // start -> F0
    queue = MessageQueue::new();

    // A foreign cell appears on F0: reject and step to F1.
    mle.rx_tlmb_prim(&mut queue, sync_ind(238, 6));

    assert!(mle.core.state().serving_cell.is_none(), "foreign cell not adopted");
    assert!(mle.core.state().scanning, "still scanning");
    assert_eq!(tuned_carrier(&queue), Some(SCAN_F1), "advanced past the disallowed cell");
}

/// D-3: a candidate whose cell bars our subscriber class is unsuitable â€” the
/// adopted cell is dropped and the scan advances (cl. 18.4.2.2 / 18.3.4).
#[test]
fn test_scan_advances_on_barred_class() {
    let mut mle = ms_mle_scan();
    let mut queue = MessageQueue::new();

    mle.rx_tlmb_prim(&mut queue, scan_dwell_ind()); // start -> F0
    queue = MessageQueue::new();

    // Home cell adopted from SYNC (network allowed)...
    mle.rx_tlmb_prim(&mut queue, sync_ind(901, 9999));
    assert!(mle.core.state().serving_cell.is_some(), "home cell adopted pending SYSINFO");
    queue = MessageQueue::new();

    // ...but its SYSINFO bars our class (only classes 2/3): drop and advance.
    mle.rx_tlmb_prim(&mut queue, sysinfo_ind(1, 0b110));

    assert!(mle.core.state().serving_cell.is_none(), "unsuitable cell dropped");
    assert!(mle.core.state().scanning, "still scanning");
    assert!(!mle.core.state().activate_confirmed, "not confirmed");
    assert_eq!(tuned_carrier(&queue), Some(SCAN_F1), "advanced past the barred cell");
}

/// D-3: a suitable candidate ends the scan â€” the cell is confirmed to MM and
/// no further candidate retune happens on a later stray dwell (cl. 18.3.4.6).
#[test]
fn test_scan_camps_on_suitable_cell_and_stops() {
    let mut mle = ms_mle_scan();
    let mut queue = MessageQueue::new();

    mle.rx_tlmb_prim(&mut queue, scan_dwell_ind()); // start -> F0
    mle.rx_tlmb_prim(&mut queue, sync_ind(901, 9999)); // adopt home cell
    mle.rx_tlmb_prim(&mut queue, sysinfo_ind(1, 0b1)); // class permitted -> camp

    assert!(mle.core.state().activate_confirmed, "cell confirmed");
    assert!(!mle.core.state().scanning, "scan stopped after camping");
    assert_eq!(
        queue.iter().filter(|m| matches!(m.msg, SapMsgInner::LmmMleActivateConf(_))).count(),
        1,
        "LMM-ACTIVATE confirmation sent to MM"
    );

    // A later stray dwell must not retune (we are camped).
    queue = MessageQueue::new();
    mle.rx_tlmb_prim(&mut queue, scan_dwell_ind());
    assert_eq!(tuned_carrier(&queue), None, "no retune while camped");
}

/// D-3: with no scan programmed (default codeplug), a dwell heartbeat is a
/// no-op â€” the radio stays on its single configured carrier.
#[test]
fn test_no_scan_when_not_programmed() {
    let mut mle = ms_mle();
    let mut queue = MessageQueue::new();

    mle.rx_tlmb_prim(&mut queue, scan_dwell_ind());

    assert!(!mle.core.state().scanning, "scanning not enabled without a scan codeplug");
    assert_eq!(tuned_carrier(&queue), None, "no retune when not scanning");
}

// ----------------------------------------------------------------------
// Manual cell survey + register-to-cell (UI-driven, receive-only)
// ETSI TS 100 392-2 cl. 18.3.4 (survey), 18.3.4.6 (camp), 16.4 (register).
// ----------------------------------------------------------------------

/// Build an operator survey start/stop request (MM -> MLE, LMM SAP).
fn scan_req(start: bool) -> SapMsg {
    use tetra_saps::lmm::LmmMleScanReq;
    SapMsg {
        sap: Sap::LmmSap,
        src: TetraEntity::Mm,
        dest: TetraEntity::Mle,
        msg: SapMsgInner::LmmMleScanReq(LmmMleScanReq { start }),
    }
}

/// Build a selection-mode request (MM -> MLE, LMM SAP).
fn selection_mode_req(manual: bool) -> SapMsg {
    use tetra_saps::lmm::LmmMleSelectionModeReq;
    SapMsg {
        sap: Sap::LmmSap,
        src: TetraEntity::Mm,
        dest: TetraEntity::Mle,
        msg: SapMsgInner::LmmMleSelectionModeReq(LmmMleSelectionModeReq { manual }),
    }
}

/// Build an operator camp request (MM -> MLE, LMM SAP).
fn camp_req(carrier_hz: u32, register: bool) -> SapMsg {
    use tetra_saps::lmm::LmmMleCampReq;
    SapMsg {
        sap: Sap::LmmSap,
        src: TetraEntity::Mm,
        dest: TetraEntity::Mle,
        msg: SapMsgInner::LmmMleCampReq(LmmMleCampReq { carrier_hz, register }),
    }
}

/// All scan-result indications emitted to MM since the queue was drained.
fn scan_results(queue: &MessageQueue) -> Vec<&LmmMleScanResultInd> {
    queue
        .iter()
        .filter_map(|m| match &m.msg {
            SapMsgInner::LmmMleScanResultInd(r) => Some(r),
            _ => None,
        })
        .collect()
}

/// The single scan-complete indication emitted to MM, if any.
fn scan_complete(queue: &MessageQueue) -> Option<&LmmMleScanCompleteInd> {
    queue.iter().find_map(|m| match &m.msg {
        SapMsgInner::LmmMleScanCompleteInd(c) => Some(c),
        _ => None,
    })
}

/// Survey: `StartCellScan` tunes to the first candidate and enters the
/// receive-only survey mode without camping.
#[test]
fn test_survey_starts_and_tunes_first_candidate() {
    let mut mle = ms_mle_scan();
    let mut queue = MessageQueue::new();

    mle.rx_lmm_prim(&mut queue, scan_req(true));

    assert!(mle.core.state().survey, "survey mode entered");
    assert!(!mle.core.state().scanning, "automatic scan not running during a survey");
    assert!(mle.core.state().serving_cell.is_none(), "no cell camped at survey start");
    assert_eq!(tuned_carrier(&queue), Some(SCAN_F0), "tuned to first candidate");
}

/// Survey: visits each candidate once (no wrap), reporting one result per
/// found cell and a single completion, and never camps/registers.
#[test]
fn test_survey_reports_each_cell_and_completes() {
    let mut mle = ms_mle_scan();
    let mut queue = MessageQueue::new();

    mle.rx_lmm_prim(&mut queue, scan_req(true)); // start -> tune F0
    queue = MessageQueue::new();

    // A cell appears on F0: SYNC then SYSINFO characterise it and advance.
    mle.rx_tlmb_prim(&mut queue, sync_ind(901, 9999));
    assert!(mle.core.state().serving_cell.is_none(), "survey never adopts a serving cell");
    mle.rx_tlmb_prim(&mut queue, sysinfo_ind(1, 0b1));
    assert_eq!(tuned_carrier(&queue), Some(SCAN_F1), "advanced to second candidate");

    // A different cell on F1.
    mle.rx_tlmb_prim(&mut queue, sync_ind(238, 6));
    mle.rx_tlmb_prim(&mut queue, sysinfo_ind(2, 0b1));

    let results = scan_results(&queue);
    assert_eq!(results.len(), 2, "one result per found cell");
    assert_eq!(results[0].carrier_hz, SCAN_F0);
    assert_eq!(results[0].mcc, 901);
    assert_eq!(results[0].mnc, 9999);
    assert_eq!(results[0].location_area, Some(1));
    assert_eq!(results[0].registration_required, Some(true));
    assert_eq!(results[1].carrier_hz, SCAN_F1);
    assert_eq!(results[1].mcc, 238);
    assert_eq!(results[1].location_area, Some(2));

    let complete = scan_complete(&queue).expect("survey completion emitted");
    assert_eq!(complete.found, 2);
    assert_eq!(complete.scanned, 2);
    assert!(!mle.core.state().survey, "survey finished after the last candidate");
    assert!(mle.core.state().serving_cell.is_none(), "never camped during survey");
    assert!(!mle.core.state().activate_confirmed, "never confirmed/registered during survey");
    assert_eq!(
        queue.iter().filter(|m| matches!(m.msg, SapMsgInner::LmmMleActivateConf(_))).count(),
        0,
        "survey emits no LMM-ACTIVATE (no registration)"
    );
}

/// Survey (regression): a candidate carrier that demod-locks a signal but
/// never yields a decodable D-MLE-SYNC produces only PHY monitoring
/// indications (never the unsynchronized scan-dwell heartbeat). The survey
/// must still advance off it after the monitor-tick backstop, must not report
/// a cell, and must never enter the serving-cell break/reopen path.
#[test]
fn test_survey_advances_off_locked_undecodable_carrier() {
    let mut mle = ms_mle_scan();
    let mut queue = MessageQueue::new();

    mle.rx_lmm_prim(&mut queue, scan_req(true)); // start -> tune F0
    queue = MessageQueue::new();

    // F0 locks a signal but no SYNC decodes: only monitor indications arrive.
    // Below the backstop nothing happens; at the limit the survey advances.
    for _ in 0..MleMs::SURVEY_MONITOR_TICK_LIMIT - 1 {
        mle.rx_tlmb_prim(&mut queue, monitor_ind(true));
    }
    assert_eq!(tuned_carrier(&queue), None, "no advance before the backstop limit");
    mle.rx_tlmb_prim(&mut queue, monitor_ind(true)); // reaches the limit
    assert_eq!(tuned_carrier(&queue), Some(SCAN_F1), "advanced off the undecodable carrier");
    assert!(scan_results(&queue).is_empty(), "no cell reported for an undecodable carrier");
    assert!(!mle.core.state().out_of_service, "survey never declares out of service");
    assert_eq!(
        queue.iter().filter(|m| matches!(m.msg, SapMsgInner::LmmMleBreakInd(_))).count(),
        0,
        "survey emits no MLE-BREAK to MM"
    );
    assert_eq!(
        queue.iter().filter(|m| matches!(m.msg, SapMsgInner::LcmcMleBreakInd(_))).count(),
        0,
        "survey emits no MLE-BREAK to CMCE"
    );

    // F1 also locks-but-undecodable: the survey completes (no cells found).
    queue = MessageQueue::new();
    for _ in 0..MleMs::SURVEY_MONITOR_TICK_LIMIT {
        mle.rx_tlmb_prim(&mut queue, monitor_ind(true));
    }
    let complete = scan_complete(&queue).expect("survey completes over the candidate set");
    assert_eq!(complete.found, 0);
    assert_eq!(complete.scanned, 2);
    assert!(!mle.core.state().survey, "survey finished");
}

/// Survey: monitoring indications interleaved with a SYNC must not cut the
/// carrier short — a decodable SYNC resets the backstop so the cell is still
/// characterised and reported.
#[test]
fn test_survey_monitor_ticks_do_not_preempt_a_synced_cell() {
    let mut mle = ms_mle_scan();
    let mut queue = MessageQueue::new();

    mle.rx_lmm_prim(&mut queue, scan_req(true)); // start -> F0
    queue = MessageQueue::new();

    // A few monitor indications, then a SYNC (progress) resets the backstop.
    for _ in 0..MleMs::SURVEY_MONITOR_TICK_LIMIT - 1 {
        mle.rx_tlmb_prim(&mut queue, monitor_ind(true));
    }
    mle.rx_tlmb_prim(&mut queue, sync_ind(901, 9999));
    assert_eq!(tuned_carrier(&queue), None, "SYNC reset the backstop; no premature advance");

    // A couple more monitor ticks (still below the reset limit), then SYSINFO
    // finalizes and reports the cell before advancing.
    mle.rx_tlmb_prim(&mut queue, monitor_ind(true));
    mle.rx_tlmb_prim(&mut queue, sysinfo_ind(1, 0b1));

    let results = scan_results(&queue);
    assert_eq!(results.len(), 1, "the synced cell is reported, not skipped");
    assert_eq!(results[0].carrier_hz, SCAN_F0);
    assert_eq!(results[0].mcc, 901);
    assert_eq!(tuned_carrier(&queue), Some(SCAN_F1), "advanced after finalizing the cell");
}

/// Survey: a candidate that yields no cell within the dwell is recorded as an
/// empty carrier (no result) and the survey advances (cl. 18.3.4).
#[test]
fn test_survey_empty_carrier_advances_on_dwell() {
    let mut mle = ms_mle_scan();
    let mut queue = MessageQueue::new();

    mle.rx_lmm_prim(&mut queue, scan_req(true)); // start -> F0
    queue = MessageQueue::new();

    // No cell on F0: the dwell heartbeat advances to F1.
    mle.rx_tlmb_prim(&mut queue, scan_dwell_ind());
    assert!(scan_results(&queue).is_empty(), "empty carrier yields no result");
    assert_eq!(tuned_carrier(&queue), Some(SCAN_F1), "advanced past the empty carrier");

    // No cell on F1 either: the survey completes with zero found cells.
    queue = MessageQueue::new();
    mle.rx_tlmb_prim(&mut queue, scan_dwell_ind());
    let complete = scan_complete(&queue).expect("survey completion emitted");
    assert_eq!(complete.found, 0);
    assert_eq!(complete.scanned, 2);
    assert!(!mle.core.state().survey, "survey finished");
}

/// Survey: a SYNC-only cell (SYSINFO never arrives) is still reported after
/// the bounded wait, with LA/registration left unknown.
#[test]
fn test_survey_reports_sync_only_cell_after_bound() {
    let mut mle = ms_mle_scan();
    let mut queue = MessageQueue::new();

    mle.rx_lmm_prim(&mut queue, scan_req(true)); // start -> F0
    queue = MessageQueue::new();

    // Repeated SYNC without SYSINFO: bounded by SURVEY_SYSINFO_SYNC_LIMIT.
    for _ in 0..MleMs::SURVEY_SYSINFO_SYNC_LIMIT + 1 {
        mle.rx_tlmb_prim(&mut queue, sync_ind(901, 9999));
    }

    let results = scan_results(&queue);
    assert_eq!(results.len(), 1, "SYNC-only cell reported once");
    assert_eq!(results[0].mcc, 901);
    assert_eq!(results[0].location_area, None, "LA unknown without SYSINFO");
    assert_eq!(results[0].registration_required, None, "reg flag unknown without SYSINFO");
    assert_eq!(tuned_carrier(&queue), Some(SCAN_F1), "advanced after the bound");
}

/// Survey: a foreign/disallowed-network cell is still characterised and
/// reported (a survey observes, it does not filter by allowed-network).
#[test]
fn test_survey_reports_disallowed_network_cell() {
    let mut mle = ms_mle_scan();
    let mut queue = MessageQueue::new();

    mle.rx_lmm_prim(&mut queue, scan_req(true)); // start -> F0
    queue = MessageQueue::new();

    mle.rx_tlmb_prim(&mut queue, sync_ind(238, 6)); // foreign network
    mle.rx_tlmb_prim(&mut queue, sysinfo_ind(7, 0b1));

    let results = scan_results(&queue);
    assert_eq!(results.len(), 1, "disallowed cell still reported by the survey");
    assert_eq!(results[0].mcc, 238);
    assert_eq!(results[0].mnc, 6);
    assert!(mle.core.state().serving_cell.is_none(), "survey never adopts the foreign cell");
}

/// Survey: `StopCellScan` cancels an in-progress survey and reports a
/// completion for the carriers visited so far.
#[test]
fn test_survey_cancel_reports_completion() {
    let mut mle = ms_mle_scan();
    let mut queue = MessageQueue::new();

    mle.rx_lmm_prim(&mut queue, scan_req(true)); // start -> F0
    queue = MessageQueue::new();

    mle.rx_lmm_prim(&mut queue, scan_req(false)); // cancel

    assert!(!mle.core.state().survey, "survey cancelled");
    assert!(scan_complete(&queue).is_some(), "cancellation reports a completion");
}

/// Manual mode: a SYNC for a suitable cell is NOT adopted (auto-camp is
/// suppressed) until the operator arms a camp.
#[test]
fn test_manual_mode_suppresses_auto_camp() {
    let mut mle = ms_mle_scan();
    let mut queue = MessageQueue::new();

    mle.rx_lmm_prim(&mut queue, selection_mode_req(true));
    assert!(mle.core.state().selection_mode_manual, "manual mode set");
    queue = MessageQueue::new();

    mle.rx_tlmb_prim(&mut queue, sync_ind(901, 9999));
    assert!(mle.core.state().serving_cell.is_none(), "manual mode does not auto-camp");
    assert!(
        !mle.core.state().activate_confirmed,
        "no registration in manual mode without a camp"
    );
}

/// Register-to-cell: an operator `CampOnCell` on a valid candidate arms a
/// camp, tunes the carrier, and the next suitable SYNC+SYSINFO adopts the
/// cell and confirms to MM (cl. 18.3.4.6) even in manual mode.
#[test]
fn test_camp_request_adopts_and_confirms() {
    let mut mle = ms_mle_scan();
    let mut queue = MessageQueue::new();

    mle.rx_lmm_prim(&mut queue, selection_mode_req(true)); // manual
    queue = MessageQueue::new();

    mle.rx_lmm_prim(&mut queue, camp_req(SCAN_F1, true));
    assert!(mle.core.state().camp_armed, "camp armed");
    assert!(mle.core.state().camp_force_register, "force-register recorded");
    assert_eq!(tuned_carrier(&queue), Some(SCAN_F1), "tuned to the chosen carrier");
    queue = MessageQueue::new();

    // The cell on the camped carrier is adopted despite manual mode.
    mle.rx_tlmb_prim(&mut queue, sync_ind(901, 9999));
    assert!(mle.core.state().serving_cell.is_some(), "camped cell adopted");
    mle.rx_tlmb_prim(&mut queue, sysinfo_ind(1, 0b1));

    assert!(mle.core.state().activate_confirmed, "cell confirmed to MM");
    assert!(!mle.core.state().camp_armed, "camp disarmed after honouring it");
    assert_eq!(
        queue.iter().filter(|m| matches!(m.msg, SapMsgInner::LmmMleActivateConf(_))).count(),
        1,
        "LMM-ACTIVATE confirmation sent to MM"
    );
}

/// Register-to-cell: a camp request for a carrier that is not a codeplug
/// candidate is rejected (no arm, no retune).
#[test]
fn test_camp_request_rejects_unknown_carrier() {
    let mut mle = ms_mle_scan();
    let mut queue = MessageQueue::new();

    mle.rx_lmm_prim(&mut queue, camp_req(123_000_000, true));

    assert!(!mle.core.state().camp_armed, "camp not armed for an unknown carrier");
    assert_eq!(tuned_carrier(&queue), None, "no retune for an unknown carrier");
}

#[test]
fn characterization_activate_only_on_first_sysinfo_la_or_service_change() {
    let mut mle = ms_mle();
    let mut queue = MessageQueue::new();
    mle.rx_tlmb_prim(&mut queue, sync_ind(901, 9999));
    mle.rx_tlmb_prim(&mut queue, sysinfo_ind(1, 1));
    mle.rx_tlmb_prim(&mut queue, sysinfo_ind(1, 1));
    mle.rx_tlmb_prim(&mut queue, sysinfo_ind(2, 1));
    assert_eq!(
        queue.iter().filter(|m| matches!(m.msg, SapMsgInner::LmmMleActivateConf(_))).count(),
        2
    );
}

#[test]
fn characterization_cancel_discards_unfinished_survey_cell() {
    let mut mle = ms_mle_scan();
    let mut queue = MessageQueue::new();
    mle.rx_lmm_prim(&mut queue, scan_req(true));
    mle.rx_tlmb_prim(&mut queue, sync_ind(901, 9999));
    mle.rx_lmm_prim(&mut queue, scan_req(false));
    assert!(scan_results(&queue).is_empty());
    let complete = scan_complete(&queue).unwrap();
    assert_eq!((complete.found, complete.scanned), (0, 0));
}

#[test]
fn characterization_internal_mle_and_reserved_protocols_are_not_delivered() {
    use tetra_saps::tla::TlaTlUnitdataIndBl;
    for protocol in [0_u64, 3, 5, 6, 7] {
        let mut mle = ms_mle();
        let mut queue = MessageQueue::new();
        let mut sdu = BitBuffer::new(6);
        sdu.write_bits(protocol, 3);
        sdu.write_bits(2, 3);
        sdu.seek(0);
        mle.rx_tla_prim(
            &mut queue,
            SapMsg {
                sap: Sap::TlaSap,
                src: TetraEntity::Llc,
                dest: TetraEntity::Mle,
                msg: SapMsgInner::TlaTlUnitdataIndBl(TlaTlUnitdataIndBl {
                    main_address: tetra_core::TetraAddress::new(1, tetra_core::SsiType::Issi),
                    endpoint_id: 0,
                    link_id: 0,
                    tl_sdu: Some(sdu),
                    new_endpoint_id: None,
                    css_endpoint_id: None,
                    scrambling_code: 0,
                    fcs_flag: false,
                    air_interface_encryption: 0,
                    chan_change_resp_req: false,
                    chan_change_handle: None,
                    chan_info: None,
                    report: None,
                }),
            },
        );
        assert!(queue.iter().next().is_none());
    }
}

#[test]
fn portable_downlink_adapter_preserves_routes_metadata_and_bitbuffer_cursor() {
    use tetra_core::SsiType;
    use tetra_saps::tla::{TlaTlDataIndBl, TlaTlUnitdataIndBl};

    for acknowledged in [false, true] {
        for protocol in [1, 2, 4] {
            let mut mle = ms_mle();
            let mut queue = MessageQueue::new();
            let address = TetraAddress::new(0x123456, SsiType::Gssi);
            let mut sdu = BitBuffer::new(13);
            sdu.write_bits(protocol, 3);
            sdu.write_bits(0x2a5, 10);
            sdu.seek(0);
            let msg = if acknowledged {
                SapMsgInner::TlaTlDataIndBl(TlaTlDataIndBl {
                    main_address: address,
                    endpoint_id: 0x1234,
                    link_id: 7,
                    new_endpoint_id: None,
                    css_endpoint_id: None,
                    tl_sdu: Some(sdu),
                    scrambling_code: 0,
                    fcs_flag: false,
                    air_interface_encryption: 0,
                    chan_change_resp_req: true,
                    chan_change_handle: Some(13),
                    chan_info: None,
                    req_handle: 19,
                })
            } else {
                SapMsgInner::TlaTlUnitdataIndBl(TlaTlUnitdataIndBl {
                    main_address: address,
                    endpoint_id: 0x1234,
                    link_id: 7,
                    new_endpoint_id: None,
                    css_endpoint_id: None,
                    tl_sdu: Some(sdu),
                    scrambling_code: 0,
                    fcs_flag: false,
                    air_interface_encryption: 0,
                    chan_change_resp_req: true,
                    chan_change_handle: Some(13),
                    chan_info: None,
                    report: None,
                })
            };
            mle.rx_tla_prim(
                &mut queue,
                SapMsg {
                    sap: Sap::TlaSap,
                    src: TetraEntity::Llc,
                    dest: TetraEntity::Mle,
                    msg,
                },
            );
            if protocol == 4 {
                assert!(queue.iter().next().is_none(), "unsupported SNDCP never reaches CMCE");
                assert!(mle.core.state().serving_cell.is_none());
                continue;
            }
            let out = queue.pop_front().unwrap();
            assert_eq!(out.dest, if protocol == 1 { TetraEntity::Mm } else { TetraEntity::Cmce });
            let mut sdu = match out.msg {
                SapMsgInner::LmmMleUnitdataInd(prim) => {
                    assert_eq!(protocol, 1);
                    assert_eq!(prim.received_address, address);
                    assert_eq!(prim.handle, 0);
                    prim.sdu
                }
                SapMsgInner::LcmcMleUnitdataInd(prim) => {
                    assert_eq!(protocol, 2);
                    assert_eq!(prim.received_tetra_address, address);
                    assert_eq!((prim.endpoint_id, prim.link_id, prim.handle), (0x1234, 7, 0));
                    assert!(!prim.chan_change_resp_req);
                    assert!(prim.chan_change_handle.is_none());
                    prim.sdu
                }
                other => panic!("unexpected delivery: {other:?}"),
            };
            assert_eq!((sdu.get_len(), sdu.get_pos(), sdu.get_len_remaining()), (13, 3, 10));
            assert_eq!(sdu.read_bits(10), Some(0x2a5));
            assert!(queue.iter().next().is_none());
        }
    }
}

#[test]
fn portable_mm_uplink_preserves_priority_and_reports_explicit_capacity_rejection() {
    use tetra_core::{TxReporter, sap_fields::Layer2Service};
    use tetra_saps::lmm::LmmMleUnitdataReq;
    for oversized in [false, true] {
        let mut mle = ms_mle();
        let mut queue = MessageQueue::new();
        let reporter = TxReporter::new();
        let mut sdu = BitBuffer::new(if oversized { 2688 } else { 10 });
        sdu.write_bits(0x2a5, 10);
        sdu.seek(0);
        mle.rx_lmm_prim(
            &mut queue,
            SapMsg {
                sap: Sap::LmmSap,
                src: TetraEntity::Mm,
                dest: TetraEntity::Mle,
                msg: SapMsgInner::LmmMleUnitdataReq(LmmMleUnitdataReq {
                    sdu,
                    handle: 17,
                    address: TetraAddress::issi(123),
                    layer2service: Layer2Service::Acknowledged,
                    pdu_priority: Some(6),
                    is_emergency: true,
                    stealing_permission: true,
                    stealing_repeats_flag: true,
                    encryption_flag: true,
                    is_null_pdu: false,
                    tx_reporter: Some(reporter.clone()),
                }),
            },
        );
        if oversized {
            assert!(queue.iter().next().is_none());
            assert!(reporter.is_discarded());
            continue;
        }
        let out = queue.pop_front().unwrap();
        let SapMsgInner::TlaTlDataReqBl(mut prim) = out.msg else {
            panic!("uplink to LLC")
        };
        assert_eq!(out.dest, TetraEntity::Llc);
        assert_eq!(prim.main_address, TetraAddress::issi(123));
        assert_eq!((prim.endpoint_id, prim.link_id, prim.pdu_priority), (0, 0, Some(6)));
        assert!(prim.is_emergency);
        assert!(!prim.stealing_permission);
        assert!(prim.air_interface_encryption.is_none());
        assert!(prim.stealing_repeats_flag.is_none());
        assert_eq!(prim.tl_sdu.read_bits(3), Some(1));
        assert_eq!(prim.tl_sdu.read_bits(10), Some(0x2a5));
        prim.tx_reporter.take().unwrap().try_mark_discarded();
        assert!(reporter.is_discarded(), "reporter correlation retained");
    }
}

#[test]
fn portable_cmce_uplink_retains_channel_allocation_and_link() {
    use tetra_core::sap_fields::Layer2Service;
    use tetra_saps::lcmc::{
        LcmcMleUnitdataReq,
        enums::{alloc_type::ChanAllocType, ul_dl_assignment::UlDlAssignment},
        fields::chan_alloc_req::CmceChanAllocReq,
    };
    let mut mle = ms_mle();
    let mut queue = MessageQueue::new();
    let mut sdu = BitBuffer::new(10);
    sdu.write_bits(0x2a5, 10);
    sdu.seek(0);
    mle.rx_lcmc_prim(
        &mut queue,
        SapMsg {
            sap: Sap::LcmcSap,
            src: TetraEntity::Cmce,
            dest: TetraEntity::Mle,
            msg: SapMsgInner::LcmcMleUnitdataReq(LcmcMleUnitdataReq {
                sdu,
                handle: 18,
                endpoint_id: 1234,
                link_id: 7,
                layer2service: Layer2Service::Unacknowledged,
                pdu_prio: 99,
                layer2_qos: 0,
                stealing_permission: true,
                stealing_repeats_flag: true,
                main_address: TetraAddress::issi(42),
                tx_reporter: None,
                chan_alloc: Some(CmceChanAllocReq {
                    usage: Some(4),
                    carrier: Some(1600),
                    timeslots: [true, false, true, false],
                    alloc_type: ChanAllocType::Replace,
                    ul_dl_assigned: UlDlAssignment::Both,
                }),
            }),
        },
    );
    let out = queue.pop_front().unwrap();
    let SapMsgInner::TlaTlDataReqBl(mut prim) = out.msg else {
        panic!("uplink to LLC")
    };
    assert_eq!(
        (prim.endpoint_id, prim.link_id, prim.main_address),
        (1234, 7, TetraAddress::issi(42))
    );
    assert!(prim.pdu_priority.is_none());
    assert!(!prim.is_emergency);
    assert!(prim.stealing_permission);
    assert!(prim.stealing_repeats_flag.is_none());
    assert_eq!(prim.tl_sdu.read_bits(3), Some(2));
    assert_eq!(prim.tl_sdu.read_bits(10), Some(0x2a5));
    let alloc = prim.chan_alloc.unwrap();
    assert_eq!(
        (alloc.usage, alloc.carrier, alloc.timeslots),
        (Some(4), Some(1600), [true, false, true, false])
    );
}
