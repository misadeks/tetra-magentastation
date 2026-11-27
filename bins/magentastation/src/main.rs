use clap::Parser;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use tetra_core::tetra_entities::TetraEntity;
use tetra_entities::net_control::channel::build_all_control_links;
use tetra_entities::net_control::{
    CONTROL_HEARTBEAT_INTERVAL, CONTROL_HEARTBEAT_TIMEOUT, CONTROL_PROTOCOL_VERSION, CommandDispatcher, ControlWorker,
};

use tetra_config::magentastation::{PhyBackend, SharedConfig, StackConfig, parsing};
use tetra_core::{TdmaTime, debug};
use tetra_entities::MessageRouter;
use tetra_entities::net_telemetry::worker::TelemetryWorker;
use tetra_entities::net_telemetry::{
    TELEMETRY_HEARTBEAT_INTERVAL, TELEMETRY_HEARTBEAT_TIMEOUT, TELEMETRY_PROTOCOL_VERSION, TelemetrySource, telemetry_channel,
};
use tetra_entities::network::transports::websocket::{WebSocketTransport, WebSocketTransportConfig};
use tetra_entities::{
    cmce::cmce_ms::CmceMs,
    llc::llc_ms::Llc,
    lmac::lmac_ms::LmacMs,
    mle::mle_ms::MleMs,
    mm::mm_ms::MmMs,
    phy::{components::soapy_dev::RxTxDevSoapySdr, phy_ms::PhyMs},
    sndcp::Sndcp,
    umac::umac_ms::UmacMs,
};

/// Process exit code requesting a **controlled restart** by an external
/// supervisor (**NON-STANDARD**, Plane B config-apply). When the MS management
/// interface applies a staged configuration (`ApplyConfig`), the stack performs
/// its graceful de-registration drain and then exits with this code; a
/// supervisor (systemd `RestartForceExitStatus=75`, or a wrapper-script loop)
/// respawns the process so it reloads the new configuration. In-process
/// relaunch is deliberately avoided (the SDR device handle and thread graph are
/// owned by `main`). A normal shutdown (Ctrl+C) exits 0 and is not restarted.
const EXIT_RESTART: i32 = 75;

/// Load configuration file
fn load_config_from_toml(cfg_path: &str) -> StackConfig {
    let contents = std::fs::read_to_string(cfg_path).unwrap_or_else(|error| {
        eprintln!("Failed to read configuration: {:?}", error.kind());
        std::process::exit(1);
    });
    match parsing::from_toml_str(&contents) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "Failed to load configuration: {}",
                parsing::config_error_summary(&contents, e.as_ref())
            );
            std::process::exit(1);
        }
    }
}

fn start_telemetry_worker(cfg: SharedConfig, telemetry_source: TelemetrySource) -> thread::JoinHandle<()> {
    let config = cfg.config();
    let tcfg = config.telemetry.as_ref().unwrap();

    let custom_root_certs = tcfg.ca_cert.as_ref().map(|path| {
        let der_bytes = std::fs::read(path).unwrap_or_else(|e| {
            eprintln!("Failed to read CA certificate from '{}': {}", path, e);
            std::process::exit(1);
        });
        vec![rustls::pki_types::CertificateDer::from(der_bytes)]
    });

    let ws_config = WebSocketTransportConfig {
        host: tcfg.host.clone(),
        port: tcfg.port,
        use_tls: tcfg.use_tls,
        digest_auth_credentials: None,
        basic_auth_credentials: tcfg.credentials.clone(),
        endpoint_path: "/".to_string(),
        subprotocol: Some(TELEMETRY_PROTOCOL_VERSION.to_string()),
        user_agent: format!("MagentaStation/{}", tetra_core::STACK_VERSION),
        heartbeat_interval: TELEMETRY_HEARTBEAT_INTERVAL,
        heartbeat_timeout: TELEMETRY_HEARTBEAT_TIMEOUT,
        custom_root_certs,
    };

    thread::spawn(move || {
        let transport = WebSocketTransport::new(ws_config);
        let mut worker = TelemetryWorker::new(telemetry_source, transport);
        worker.run();
    })
}

fn start_control_worker(
    cfg: SharedConfig,
    command_dispatchers: HashMap<TetraEntity, CommandDispatcher>,
) -> thread::JoinHandle<Result<(), tetra_entities::network::transports::NetworkError>> {
    let config = cfg.config();
    let ccfg = config.control.as_ref().unwrap();

    let custom_root_certs = ccfg.ca_cert.as_ref().map(|path| {
        let der_bytes = std::fs::read(path).unwrap_or_else(|e| {
            eprintln!("Failed to read CA certificate from '{}': {}", path, e);
            std::process::exit(1);
        });
        vec![rustls::pki_types::CertificateDer::from(der_bytes)]
    });

    let ws_config = WebSocketTransportConfig {
        host: ccfg.host.clone(),
        port: ccfg.port,
        use_tls: ccfg.use_tls,
        digest_auth_credentials: None,
        basic_auth_credentials: ccfg.credentials.clone(),
        endpoint_path: "/".to_string(),
        subprotocol: Some(CONTROL_PROTOCOL_VERSION.to_string()),
        user_agent: format!("MagentaStation/{}", tetra_core::STACK_VERSION),
        heartbeat_interval: CONTROL_HEARTBEAT_INTERVAL,
        heartbeat_timeout: CONTROL_HEARTBEAT_TIMEOUT,
        custom_root_certs,
    };

    thread::spawn(move || {
        let transport = WebSocketTransport::new(ws_config);
        let mut worker = ControlWorker::new(command_dispatchers, transport);
        worker.run()
    })
}

/// Start mobile station stack.
///
/// MS protocol time is recovered from RX. The explicit None backend registers
/// no PHY, so only offline management is serviced and no protocol ticks occur.
fn build_ms_stack(
    cfg: &mut SharedConfig,
    config_path: &str,
    restart_requested: Arc<AtomicBool>,
    is_running: Arc<AtomicBool>,
) -> (MessageRouter, Option<TelemetrySource>, HashMap<TetraEntity, CommandDispatcher>) {
    let mut router = MessageRouter::new(cfg.clone());

    // Add suitable Phy component based on PhyIo type
    match cfg.config().phy_io.backend {
        PhyBackend::SoapySdr => {
            // soapyio already maps StackMode::Ms to RX=downlink / TX=uplink.
            let mut rxdev = RxTxDevSoapySdr::new(cfg);
            // Give the SDR RX loop the shutdown flag so a Ctrl+C is acted on
            // promptly even while the MS is not synchronized to a base station
            // (otherwise the unsynchronized RX loop blocks until a downlink
            // appears — the process would "await connection" before exiting).
            rxdev.set_run_flag(is_running.clone());
            let phy = PhyMs::new(cfg.clone(), rxdev);
            router.register_entity(Box::new(phy));
        }
        PhyBackend::None => {
            tracing::warn!("MS PHY backend None: offline management only; no radio or TDMA clock");
        }
        _ => {
            panic!("Unsupported PhyIo type: {:?}", cfg.config().phy_io.backend);
        }
    }

    // Build telemetry sink/source, if enabled
    let (tsink, tsource) = if cfg.config().telemetry.is_some() {
        let (a, b) = telemetry_channel();
        (Some(a), Some(b))
    } else {
        (None, None)
    };

    // Build control links, if enabled
    let (mut c_d, mut c_e) = if cfg.config().control.is_some() {
        build_all_control_links()
    } else {
        (HashMap::new(), HashMap::new())
    };

    // Add remaining components
    let lmac = LmacMs::new(cfg.clone());
    let umac = UmacMs::new(cfg.clone());
    let llc = Llc::new(cfg.clone());
    let mle = MleMs::new(cfg.clone());
    let mm = {
        let mut mm = MmMs::new(cfg.clone(), tsink.clone(), c_e.remove(&TetraEntity::Mm));
        // Install the Plane B management context (NON-STANDARD) so the MS
        // management interface can persist staged configs and drive a
        // supervisor-based restart on ApplyConfig. Harmless when control is
        // disabled (no command source reaches the write handlers).
        mm.set_management_context(PathBuf::from(config_path), restart_requested, is_running);
        mm
    };
    let sndcp = Sndcp::new(cfg.clone());
    let cmce = CmceMs::new(cfg.clone(), tsink.clone(), c_e.remove(&TetraEntity::Cmce));
    router.register_entity(Box::new(lmac));
    router.register_entity(Box::new(umac));
    router.register_entity(Box::new(llc));
    router.register_entity(Box::new(mle));
    router.register_entity(Box::new(mm));
    router.register_entity(Box::new(sndcp));
    router.register_entity(Box::new(cmce));

    // Drop all command links that were not given to a TetraEntity
    for (entity, dispatcher) in c_e.into_iter() {
        drop(dispatcher);
        c_d.remove(&entity);
    }

    // Initial router value only; RX must supply time before protocol ticks run.
    router.set_dl_time(TdmaTime::default());

    (router, tsource, c_d)
}

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "TETRA MagentaStation mobile station stack",
    long_about = "Runs the MS-only TETRA MagentaStation stack using a TOML configuration. \
                  Only stack_mode = \"Ms\" is supported. Use backend = \"None\" for offline management."
)]
struct Args {
    /// Config file (required)
    #[arg(help = "TOML mobile-station configuration")]
    config: String,
}

fn main() {
    eprintln!("MagentaStation - TETRA mobile station");
    eprintln!("  Derived from BlueStation by Wouter Bokslag / Midnight Blue");
    eprintln!("  Version: {}", tetra_core::STACK_VERSION);

    // Parse command-line arguments
    let args = Args::parse();

    // Validate before constructing any radio device.
    let stack_cfg = load_config_from_toml(&args.config);
    let mut cfg = SharedConfig::from_parts(stack_cfg, None);

    let _log_guards = debug::setup_logging_default(cfg.config().debug_log.clone());

    // Shared shutdown/restart flags. `is_running` gates the stack run loop (also
    // cleared by Ctrl+C and by an MS ApplyConfig); `restart_requested` is set by
    // an MS ApplyConfig so we exit with EXIT_RESTART for the supervisor to
    // respawn with the new config. Both are created before building the stack so
    // the MS management context can share them.
    let is_running = Arc::new(AtomicBool::new(true));
    let restart_requested = Arc::new(AtomicBool::new(false));

    let (mut router, tsource, cdispatchers) = build_ms_stack(&mut cfg, &args.config, restart_requested.clone(), is_running.clone());

    // Start Telemetry and Control threads, if enabled
    let telemetry_worker = tsource.map(|source| start_telemetry_worker(cfg.clone(), source));
    let control_worker = cfg
        .config()
        .control
        .as_ref()
        .map(|_| start_control_worker(cfg.clone(), cdispatchers));

    // Set up Ctrl+C handler for graceful shutdown
    let is_running_clone = is_running.clone();
    ctrlc::set_handler(move || {
        is_running_clone.store(false, Ordering::SeqCst);
    })
    .expect("failed to set Ctrl+C handler");

    // Start the stack
    router.run_stack(None, Some(is_running));

    // router drops here → entities are dropped, networked entities disconnect.
    drop(router);
    if let Some(worker) = control_worker {
        match worker.join() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::error!("Control shutdown completed with response delivery failure: {error}"),
            Err(_) => tracing::error!("Control worker panicked during shutdown"),
        }
    }
    if let Some(worker) = telemetry_worker
        && worker.join().is_err()
    {
        tracing::error!("Telemetry worker panicked during shutdown");
    }

    // If the MS management interface requested a config-apply restart, exit with
    // the documented restart code so the external supervisor respawns the stack
    // with the newly staged configuration (NON-STANDARD, Plane B).
    if restart_requested.load(Ordering::SeqCst) {
        eprintln!("Restart requested by management interface; exiting with code {EXIT_RESTART} for supervisor respawn.");
        std::process::exit(EXIT_RESTART);
    }
}
