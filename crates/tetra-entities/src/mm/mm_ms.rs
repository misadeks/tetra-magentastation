use crate::net_control::ControlEndpoint;
use crate::net_telemetry::channel::TelemetrySink;
use crate::{MessageQueue, TetraEntityTrait};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tetra_config::magentastation::SharedConfig;
use tetra_core::tetra_entities::TetraEntity;
use tetra_core::{BitBuffer, Layer2Service, Sap, TdmaTime, TetraAddress, TxReporter, TxState};
use tetra_saps::lmm::{LmmMleActivateConf, LmmMleCampReq, LmmMleIdentitiesReq, LmmMleScanReq, LmmMleSelectionModeReq, LmmMleUnitdataReq};
use tetra_saps::{SapMsg, SapMsgInner};

#[cfg(test)]
use tetra_pdus::mm::enums::location_update_accept_type::LocationUpdateAcceptType;
#[cfg(test)]
use tetra_pdus::mm::enums::location_update_type::LocationUpdateType;
use tetra_pdus::mm::enums::reject_cause::RejectCause;
#[cfg(test)]
use tetra_pdus::mm::pdus::d_attach_detach_group_identity_acknowledgement::DAttachDetachGroupIdentityAcknowledgement;
#[cfg(test)]
use tetra_pdus::mm::pdus::d_location_update_command::DLocationUpdateCommand;
#[cfg(test)]
use tetra_pdus::mm::pdus::d_location_update_reject::DLocationUpdateReject;
#[cfg(test)]
use tetra_pdus::mm::pdus::u_attach_detach_group_identity::UAttachDetachGroupIdentity;
#[cfg(test)]
use tetra_pdus::mm::pdus::u_location_update_demand::ULocationUpdateDemand;

use tetra_ms_core::mm::GROUP_CLASS_OF_USAGE;

const MAX_DEFERRED_CONTROL: usize = crate::net_control::channel::CONTROL_QUEUE_CAPACITY;
use tetra_ms_core::mm::{
    Action as MmAction, CellId, Config as MmConfig, Error as MmError, Event as MmEvent, GroupAttachments, GroupOperation, GroupRequest,
    MmCore, MmState, PendingGroupOp, RegState, ServiceStatus as MmServiceStatus, TransferState,
};
#[cfg(test)]
use tetra_ms_core::mm::{DETACH_DRAIN_SLOTS, N351_MAX_SYSTEM_REJECTIONS, T351_TIMEOUT_SLOTS, T353_TIMEOUT_SLOTS};

/// Application configuration/control/telemetry adapter. Protocol state lives in MmCore.
pub struct MmMs {
    config: SharedConfig,
    telemetry: Option<TelemetrySink>,
    control: Option<ControlEndpoint>,
    core: MmCore,
    detach_reporter: Option<TxReporter>,
    serving_rssi_dbfs: Option<f32>,
    mgmt_ctx: Option<ManagementContext>,
    restart_required: bool,
    active_scanlists: BTreeSet<String>,
    deferred_control: Vec<crate::net_control::ControlCommand>,
    selection_mode_manual: bool,
    camp_force_register: bool,
    pending_scan_after_detach: bool,
    last_tick: Option<TdmaTime>,
}

impl core::ops::Deref for MmMs {
    type Target = MmState;

    fn deref(&self) -> &MmState {
        self.core.state()
    }
}

/// Runtime plumbing for the Plane B management write/apply handlers
/// (**NON-STANDARD**). Carries the on-disk config path to persist to and the
/// process-level restart signalling shared with `main`.
struct ManagementContext {
    /// Path of the TOML config file that `SetConfig` writes and that the
    /// supervisor-respawned process reloads at startup.
    config_path: PathBuf,
    /// Set by `ApplyConfig` to request the supervisor-driven restart; polled by
    /// `main` after the stack run loop exits so it can exit with the documented
    /// restart exit code.
    restart_requested: Arc<AtomicBool>,
    /// Cleared by `ApplyConfig` to break the stack run loop and begin the
    /// graceful shutdown drain (the same `is_running` flag `main` owns).
    is_running: Arc<AtomicBool>,
}

impl MmMs {
    fn advance_time(&mut self, queue: &mut MessageQueue, mut elapsed: u32, detach_transfer: TransferState) {
        while elapsed > 0 {
            // One retry interval fits the core's action budget even when detach
            // and a pending group confirmation expire in the same batch.
            let mut batch = elapsed.min(tetra_ms_core::mm::T351_TIMEOUT_SLOTS + 1);
            loop {
                match self.core_event(
                    queue,
                    MmEvent::Tick {
                        elapsed_slots: batch,
                        detach_transfer,
                    },
                ) {
                    Ok(()) => break,
                    Err(MmError::ActionCapacity) if batch > 1 => batch /= 2,
                    Err(error) => {
                        tracing::error!("MM: recovered time could not be advanced: {error:?}");
                        return;
                    }
                }
            }
            elapsed -= batch;
        }
    }

    pub fn new(config: SharedConfig, telemetry: Option<TelemetrySink>, control: Option<ControlEndpoint>) -> Self {
        let serving_la = config.config().cell.location_area;
        // Seed the runtime attached-group set from config (cl. 16.8). Before any
        // registration this is the set the MS will request; after registration it
        // holds the SwMI-confirmed attached set. The desired set is the base
        // `[ms].attach_groups` union the GSSIs of every scan list whose
        // programmed default is `active` (Plane B), so default-active scan lists
        // are affiliated at the first registration.
        let attached_gssis = config.config().initial_groups().expect("validated initial affiliations");
        let active_scanlists: BTreeSet<String> = config
            .config()
            .codeplug
            .scanlists
            .iter()
            .filter(|s| s.active)
            .map(|s| s.name.clone())
            .collect();
        let cfg = config.config();
        let core = MmCore::new(
            MmConfig {
                issi: cfg.ms.as_ref().expect("MS config section required in MS mode").issi,
                mcc: cfg.net.mcc,
                mnc: cfg.net.mnc,
                initial_la: serving_la,
            },
            attached_gssis.as_slice(),
        )
        .expect("MS MM configuration exceeds portable identity/group bounds");
        Self {
            config,
            telemetry,
            control,
            core,
            detach_reporter: None,
            serving_rssi_dbfs: None,
            mgmt_ctx: None,
            restart_required: false,
            active_scanlists,
            deferred_control: Vec::new(),
            selection_mode_manual: false,
            camp_force_register: false,
            pending_scan_after_detach: false,
            last_tick: None,
        }
    }

    #[cfg(test)]
    fn seed_group(&mut self, gssi: u32) {
        let mut groups = self.attach_groups();
        groups.push(gssi);
        self.core.configure_groups(&groups).unwrap();
    }

    fn core_error_detail(error: MmError) -> String {
        match error {
            MmError::NotRegistered => "not registered; cannot attach/detach groups".to_string(),
            MmError::GroupOperationPending => "a group attach/detach is already in progress".to_string(),
            MmError::EmptyGroups => "no group identities in the request".to_string(),
            MmError::RegistrationInProgress => "registration already in progress".to_string(),
            MmError::AlreadyRegistered => "already registered".to_string(),
            MmError::Detaching => "de-registration in progress".to_string(),
            MmError::LeftSystem => "MS left the system (N351); registration requires a restart".to_string(),
            other => format!("portable MM rejected operation: {other:?}"),
        }
    }

    fn core_event(&mut self, queue: &mut MessageQueue, event: MmEvent<'_>) -> Result<(), MmError> {
        let result = self.core.handle(event);
        if let Err(error) = &result {
            tracing::warn!("MM: portable event rejected: {:?}", error);
        }
        while let Some(action) = self.core.poll_action() {
            match action {
                MmAction::Send {
                    sdu,
                    priority,
                    detach_receipt,
                } => {
                    let mut buffer = BitBuffer::new(usize::from(sdu.len()));
                    let mut pos = 0;
                    while pos < sdu.len() {
                        let width = (sdu.len() - pos).min(32) as u8;
                        buffer.write_bits(
                            u64::from(sdu.view().read(pos, width).expect("bounded core output")),
                            usize::from(width),
                        );
                        pos += u16::from(width);
                    }
                    buffer.seek(0);
                    let reporter = if detach_receipt {
                        let reporter = TxReporter::new();
                        self.detach_reporter = Some(reporter.clone());
                        Some(reporter)
                    } else {
                        None
                    };
                    tracing::info!("MM: portable uplink priority={} sdu={}", priority, buffer.dump_bin());
                    queue.push_back(SapMsg {
                        sap: Sap::LmmSap,
                        src: TetraEntity::Mm,
                        dest: TetraEntity::Mle,
                        msg: SapMsgInner::LmmMleUnitdataReq(LmmMleUnitdataReq {
                            sdu: buffer,
                            handle: 0,
                            address: TetraAddress::issi(self.own_issi()),
                            layer2service: Layer2Service::Acknowledged,
                            pdu_priority: Some(priority),
                            is_emergency: false,
                            stealing_permission: false,
                            stealing_repeats_flag: false,
                            encryption_flag: false,
                            is_null_pdu: false,
                            tx_reporter: reporter,
                        }),
                    });
                }
                MmAction::Service(status) => self.emit_service(match status {
                    MmServiceStatus::InService => crate::tnmm::ServiceStatus::InService,
                    MmServiceStatus::WaitingForRegistration => crate::tnmm::ServiceStatus::InServiceWaitingForRegistration,
                    MmServiceStatus::OutOfService => crate::tnmm::ServiceStatus::OutOfService,
                }),
                MmAction::RegistrationSuccess { attached } => self.emit_registration_success(&attached),
                MmAction::RegistrationFailure { cause } => {
                    self.emit_registration_failure(cause.and_then(|cause| RejectCause::try_from(u64::from(cause)).ok()))
                }
                MmAction::Identities { attached, detached } => {
                    queue.push_back(SapMsg {
                        sap: Sap::LmmSap,
                        src: TetraEntity::Mm,
                        dest: TetraEntity::Mle,
                        msg: SapMsgInner::LmmMleIdentitiesReq(LmmMleIdentitiesReq {
                            issi: self.own_issi(),
                            assi: None,
                            attached_gssis: attached.as_slice().to_vec(),
                            detached_gssis: detached.as_slice().to_vec(),
                        }),
                    });
                }
                MmAction::GroupConfirm { op, attached, detached } => self.emit_group_identity_confirm(&op, &attached, detached.as_slice()),
                MmAction::DetachReport { successful } => {
                    self.detach_reporter = None;
                    self.emit_report(if successful {
                        crate::tnmm::TransferResult::TransferSuccessfulDone
                    } else {
                        crate::tnmm::TransferResult::TransferFail
                    });
                }
                MmAction::DetachComplete => {
                    if std::mem::take(&mut self.pending_scan_after_detach) {
                        self.send_lmm_to_mle(queue, SapMsgInner::LmmMleScanReq(LmmMleScanReq { start: true }));
                    }
                }
            }
        }
        result
    }

    /// Install the Plane B management context (**NON-STANDARD**): the config
    /// file path to persist `SetConfig` writes to, plus the shared restart /
    /// running flags `main` uses to drive a controlled restart on `ApplyConfig`.
    ///
    /// Provided via a setter (not a `new()` parameter) so the many existing
    /// test constructors are unaffected; when it is not installed the write /
    /// apply management commands are refused gracefully.
    pub fn set_management_context(&mut self, config_path: PathBuf, restart_requested: Arc<AtomicBool>, is_running: Arc<AtomicBool>) {
        self.mgmt_ctx = Some(ManagementContext {
            config_path,
            restart_requested,
            is_running,
        });
    }

    /// Own Individual Short Subscriber Identity (ISSI) from the MS config
    /// section. This is the MS's own address used as the source of the uplink.
    fn own_issi(&self) -> u32 {
        self.config
            .config()
            .ms
            .as_ref()
            .expect("MS config section required in MS mode")
            .issi
    }

    /// Group identities (GSSIs) this MS currently intends to have attached
    /// (ETSI TS 100 392-2 cl. 16.8 group identity attachment). Returns the live
    /// runtime set (seeded from config, updated by attach/detach), sorted for
    /// deterministic PDU ordering. On an initial registration this equals the
    /// configured set; on a re-registration it reflects the current attachments.
    fn attach_groups(&self) -> Vec<u32> {
        self.attached_gssis.iter().copied().collect()
    }

    /// Base group identities the MS always affiliates to, from
    /// `[ms].attach_groups` (independent of any scan-list activation).
    fn base_attach_groups(&self) -> BTreeSet<u32> {
        self.config
            .config()
            .ms
            .as_ref()
            .map(|ms| ms.attach_groups.iter().copied().collect::<BTreeSet<u32>>())
            .unwrap_or_default()
    }

    /// Union of the GSSIs of every scan list currently marked active (looked up
    /// by name in the codeplug). Unknown names (e.g. a scan list removed by a
    /// later config edit) contribute nothing.
    fn active_scanlist_gssis(&self) -> BTreeSet<u32> {
        let cfg = self.config.config();
        let mut out = BTreeSet::new();
        for name in &self.active_scanlists {
            if let Some(sl) = cfg.codeplug.scanlist(name) {
                for &g in &sl.talkgroups {
                    out.insert(g);
                }
            }
        }
        out
    }

    /// The desired group-affiliation set: base `[ms].attach_groups` union the
    /// GSSIs of every active scan list (**NON-STANDARD**, Plane B superset control).
    fn desired_affiliation_set(&self) -> BTreeSet<u32> {
        let mut set = self.base_attach_groups();
        set.extend(self.active_scanlist_gssis());
        set
    }

    /// Own Mobile Network Identity (MNI) as the 24-bit value carried in the
    /// address extension information element (ETSI EN 300 392-1 clause 7): the
    /// 10-bit MCC in the high bits followed by the 14-bit MNC. Taken from the
    /// configured home network (`[net_info]`), which is the MNI of the MS ITSI.
    fn own_mni(&self) -> u32 {
        let net = &self.config.config().net;
        ((net.mcc as u32) << 14) | (net.mnc as u32 & 0x3FFF)
    }

    // ----------------------------------------------------------------------
    // TNMM-SAP indications (Plane A, OUTBOUND) — ETSI TS 100 392-2 cl. 15.3.
    //
    // These push standardized TNMM primitives (cl. 15.3.3 / 15.3.4) toward the
    // MS user application over the telemetry side-channel. They never touch the
    // on-air SapMsg queue and never change registration state — they only
    // report state that MM has already reached, so no air/TNMM behaviour is
    // invented.
    // ----------------------------------------------------------------------

    /// Push a TNMM indication/confirm to the user application, if a telemetry
    /// sink is wired. Fire-and-forget (lock-free).
    fn emit(&self, event: crate::net_telemetry::TelemetryEvent) {
        if let Some(sink) = &self.telemetry {
            sink.send(event);
        }
    }

    /// MCC where registered (Table 15.5). This single-network clear-mode MS only
    /// ever camps on / registers with cells of its configured network — a
    /// D-LOCATION-UPDATE-COMMAND carrying a different MNI is ignored (own_mni
    /// check) — so the registered MCC is the configured home MCC.
    fn where_registered_mcc(&self) -> u16 {
        self.config.config().net.mcc
    }

    /// MNC where registered (Table 15.5). See [`Self::where_registered_mcc`].
    fn where_registered_mnc(&self) -> u16 {
        self.config.config().net.mnc
    }

    /// Cell type where registered (Table 15.5). TNMM registration / location
    /// updating is a trunked-mode (V+D) procedure carried out on a CA cell; this
    /// stack implements no direct-mode (DA) registration path, so the cell where
    /// this MS registers is always a CA cell (`cl. 15.3.4` value "CA cell").
    fn where_registered_cell_type(&self) -> crate::tnmm::CellType {
        crate::tnmm::CellType::CaCell
    }

    /// Build the "group identities" parameter (Table 15.8) for a set of GSSIs
    /// that have just been attached at ITSI attach, as reported to the user
    /// application. GITI = Attachment, so the conditional lifetime + class of
    /// usage members are present; the detachment reason is absent (cl. 15.3.4).
    /// GTSI = MNI << 24 | GSSI (cl. 15.3.4 "GTSI").
    fn attached_group_identities(&self, groups: &GroupAttachments) -> Vec<crate::tnmm::GroupIdentity> {
        use crate::tnmm::{GroupIdentity, GroupIdentityAttachDetachTypeIdentifier, GroupIdentityLifetime};
        let mni = self.own_mni() as u64;
        groups
            .iter()
            .map(|group| GroupIdentity {
                gtsi: (mni << 24) | u64::from(group.gssi),
                group_identity_attach_detach_type_identifier: GroupIdentityAttachDetachTypeIdentifier::Attachment,
                group_identity_lifetime: Some(match group.lifetime {
                    0 => GroupIdentityLifetime::PermanentAttachmentNotNeeded,
                    1 => GroupIdentityLifetime::AttachmentNeededForNextItsiAttach,
                    2 => GroupIdentityLifetime::AttachmentNotAllowedAfterNextItsiAttach,
                    _ => GroupIdentityLifetime::AttachmentNeededForNextLocationUpdate,
                }),
                class_of_usage: Some(Self::class_of_usage_from_raw(group.class_of_usage)),
                group_identity_detachment_reason: None,
            })
            .collect()
    }

    /// Map the current MM state to the derived TNMM service status (the same
    /// vocabulary as the TNMM-SERVICE indication, Table 15.6 / cl. 15.3.4).
    ///
    /// Service availability is kept distinct from registration (cl. 18.3.3): a
    /// serving-cell radio link failure (`out_of_service`) makes the MS out of
    /// service even though it may still hold a valid registration. Only when the
    /// link is up does the status follow the registration state.
    fn derived_service_status(&self) -> crate::tnmm::ServiceStatus {
        match self.core.service_status() {
            MmServiceStatus::InService => crate::tnmm::ServiceStatus::InService,
            MmServiceStatus::WaitingForRegistration => crate::tnmm::ServiceStatus::InServiceWaitingForRegistration,
            MmServiceStatus::OutOfService => crate::tnmm::ServiceStatus::OutOfService,
        }
    }

    /// Handle an MLE-BREAK indication (LMM-SAP, cl. 18.3.3 / 18.3.4.5.3): the
    /// serving-cell downlink has failed. Declare the MS out of service and inform
    /// the TNMM user (TNMM-SERVICE "out of service", cl. 15.3.4). The registration
    /// state is intentionally left unchanged — the MS is still registered, just
    /// without a usable radio link.
    fn rx_mle_break(&mut self) {
        self.last_tick = None;
        let mut unused = MessageQueue::new();
        let _ = self.core_event(&mut unused, MmEvent::LinkBreak);
    }

    /// Handle an MLE-REOPEN indication (LMM-SAP, cl. 18.3.4.7): the serving-cell
    /// downlink has recovered. Leave out-of-service and emit a TNMM-SERVICE
    /// indication reflecting the restored state. MLE re-runs cell selection and
    /// re-fires the LMM-ACTIVATE confirmation, so any needed re-registration is
    /// evaluated there (cl. 18.3.4.7.1a); this only restores the service view.
    fn rx_mle_reopen(&mut self) {
        self.last_tick = None;
        let mut unused = MessageQueue::new();
        let _ = self.core_event(&mut unused, MmEvent::LinkReopen);
    }

    /// Emit a TNMM-SERVICE indication (Table 15.6, cl. 15.3.3.8) reflecting the
    /// MS service state. Disable status is always "enabled": the disabling
    /// procedure is defined in ETSI EN 300 392-7 (Part 7, security), which is out
    /// of scope, so this MS is never in a temporary/permanently-disabled state.
    fn emit_service(&self, service_status: crate::tnmm::ServiceStatus) {
        use crate::net_telemetry::TelemetryEvent;
        use crate::tnmm::{DisableStatus, TnmmServiceIndication};
        self.emit(TelemetryEvent::TnmmServiceIndication(TnmmServiceIndication {
            service_status,
            disable_status: DisableStatus::Enabled,
        }));
    }

    /// Map an on-air `Reject cause` (cl. 16.10.42) to the TNMM-SAP
    /// `Registration reject cause` enumeration (cl. 15.3.4). Returns `None` for
    /// the on-air causes that have no TNMM-SAP enumerant ("use CA/DA cell not
    /// permitted"), which therefore cannot be reported through this parameter.
    fn map_registration_reject_cause(cause: RejectCause) -> Option<crate::tnmm::RegistrationRejectCause> {
        use crate::tnmm::RegistrationRejectCause as T;
        Some(match cause {
            RejectCause::ItsiAtsiUnknown => T::ItsiUnknown,
            RejectCause::IllegalMs => T::IllegalMs,
            RejectCause::LaNotAllowed => T::LaNotAllowed,
            RejectCause::LaUnknown => T::LaUnknown,
            RejectCause::NetworkFailure => T::NetworkFailure,
            RejectCause::Congestion => T::Congestion,
            RejectCause::ForwardRegistrationFailure => T::ForwardRegistrationFailure,
            RejectCause::ServiceNotSubscribed => T::ServiceNotSubscribed,
            RejectCause::MandatoryElementError => T::MandatoryElementError,
            RejectCause::MessageConsistencyError => T::MessageConsistencyError,
            RejectCause::RoamingNotSupported => T::RoamingNotSupported,
            RejectCause::MigrationNotSupported => T::MigrationNotSupported,
            RejectCause::NoCipherKsg => T::NoCipherKsg,
            RejectCause::IdentifiedCipherKsgNotSupported => T::IdentifiedCipherKsgNotSupported,
            RejectCause::RequestedCipherKeyTypeNotAvailable => T::RequestedCipherKeyTypeNotAvailable,
            RejectCause::IdentifiedCipherKeyNotAvailable => T::IdentifiedCipherKeyNotAvailable,
            RejectCause::CipheringRequired => T::CipheringRequired,
            RejectCause::AuthenticationFailure => T::AuthenticationFailure,
            RejectCause::UseCaCellNotPermitted | RejectCause::UseDaCellNotPermitted => return None,
        })
    }

    /// Emit a TNMM-REGISTRATION indication reporting that MM has failed a
    /// registration procedure (Table 15.5; status = "failure" + reject cause,
    /// cl. 15.3.3.7 / NOTE 1). LA/MCC/MNC reflect the cell the attempt was made
    /// against. Followed by a TNMM-SERVICE "out of service" indication.
    fn emit_registration_failure(&self, cause: Option<RejectCause>) {
        use crate::net_telemetry::TelemetryEvent;
        use crate::tnmm::{RegistrationStatus, ServiceStatus, TnmmRegistrationIndication};
        let reject = cause.and_then(Self::map_registration_reject_cause);
        self.emit(TelemetryEvent::TnmmRegistrationIndication(Box::new(TnmmRegistrationIndication {
            registration_status: RegistrationStatus::Failure,
            registration_reject_cause: reject,
            cell_type_where_registered: self.where_registered_cell_type(),
            la_where_registered: self.serving_la,
            mcc_where_registered: self.where_registered_mcc(),
            mnc_where_registered: self.where_registered_mnc(),
            swmis_required_cell_types: None,
            energy_economy_mode: None,
            energy_economy_mode_status: None,
            group_identities: None,
            group_identity_attach_detach_mode: None,
        })));
        self.emit_service(ServiceStatus::OutOfService);
    }

    /// Emit the TNMM-SAP outbound primitives for a successfully carried-out
    /// registration (Tables 15.5 / 15.1 / 15.6):
    /// - TNMM-REGISTRATION indication and confirm (status = "success"), the
    ///   confirm informing the user application the MS is ready for use
    ///   (cl. 15.3.3.7);
    /// - TNMM-SERVICE indication "in service" (cl. 15.3.3.8);
    /// - TNMM-ATTACH DETACH GROUP IDENTITY indication reporting the groups the
    ///   SwMI attached, when any (cl. 15.3.3.1).
    ///
    /// `attached_gssis` are the GSSIs the SwMI confirmed attached in the
    /// D-LOCATION-UPDATE-ACCEPT (empty when none / group attachment failed).
    fn emit_registration_success(&self, attached_gssis: &GroupAttachments) {
        use crate::net_telemetry::TelemetryEvent;
        use crate::tnmm::{
            RegistrationStatus, ServiceStatus, TnmmAttachDetachGroupIdentityIndication, TnmmRegistrationConfirm, TnmmRegistrationIndication,
        };

        let group_identities = self.attached_group_identities(attached_gssis);
        let groups_opt = if group_identities.is_empty() {
            None
        } else {
            Some(group_identities.clone())
        };

        self.emit(TelemetryEvent::TnmmRegistrationIndication(Box::new(TnmmRegistrationIndication {
            registration_status: RegistrationStatus::Success,
            registration_reject_cause: None,
            cell_type_where_registered: self.where_registered_cell_type(),
            la_where_registered: self.serving_la,
            mcc_where_registered: self.where_registered_mcc(),
            mnc_where_registered: self.where_registered_mnc(),
            swmis_required_cell_types: None,
            energy_economy_mode: None,
            energy_economy_mode_status: None,
            group_identities: groups_opt.clone(),
            group_identity_attach_detach_mode: None,
        })));

        self.emit(TelemetryEvent::TnmmRegistrationConfirm(Box::new(TnmmRegistrationConfirm {
            registration_status: RegistrationStatus::Success,
            registration_reject_cause: None,
            cell_type_where_registered: self.where_registered_cell_type(),
            la_where_registered: self.serving_la,
            mcc_where_registered: self.where_registered_mcc(),
            mnc_where_registered: self.where_registered_mnc(),
            energy_economy_mode: None,
            energy_economy_mode_status: None,
            group_identities: groups_opt,
            group_identity_attach_detach_mode: None,
        })));

        self.emit_service(ServiceStatus::InService);

        if !group_identities.is_empty() {
            self.emit(TelemetryEvent::TnmmAttachDetachGroupIdentityIndication(
                TnmmAttachDetachGroupIdentityIndication { group_identities },
            ));
        }
    }

    /// Handle a Plane B `ActivateScanlist` management command (**NON-STANDARD**):
    /// activate or deactivate a programmed scan list and reconcile the group
    /// affiliation to the resulting desired set (base `[ms].attach_groups` union
    /// the GSSIs of every active scan list). When registered, the delta is
    /// applied on air via a standalone U-ATTACH/DETACH GROUP IDENTITY (cl. 16.8.2)
    /// amendment; otherwise the desired set is updated locally so it is
    /// affiliated at the next registration. Responds with a management `Ack`.
    fn handle_activate_scanlist(&mut self, handle: u32, name: String, active: bool, queue: &mut MessageQueue) {
        use crate::management::ManagementResponse;
        use crate::net_control::ControlResponse;

        let ack = |accepted: bool, message: String| {
            ControlResponse::Management(ManagementResponse::Ack {
                handle,
                accepted,
                restart_required: false,
                message,
            })
        };

        // The scan list must exist in the codeplug.
        if self.config.config().codeplug.scanlist(&name).is_none() {
            self.respond(ack(false, format!("unknown scanlist '{name}'")));
            return;
        }

        // Update the active-scan-list set (idempotent).
        let changed = if active {
            self.active_scanlists.insert(name.clone())
        } else {
            self.active_scanlists.remove(&name)
        };
        if !changed {
            self.respond(ack(
                true,
                format!("scanlist '{name}' already {}", if active { "active" } else { "inactive" }),
            ));
            return;
        }

        // Reconcile the desired affiliation set against the currently attached
        // set to find what must be attached/detached on air.
        let desired = self.desired_affiliation_set();
        let attached: BTreeSet<u32> = self.attached_gssis.iter().copied().collect();
        let to_attach: Vec<u32> = desired.difference(&attached).copied().collect();
        let to_detach: Vec<u32> = attached.difference(&desired).copied().collect();

        // Not registered: no on-air action; just update the desired runtime set
        // so it is affiliated at the next registration.
        if self.reg_state != RegState::Registered {
            if let Err(error) = self.core.configure_groups(&desired.iter().copied().collect::<Vec<_>>()) {
                if active {
                    self.active_scanlists.remove(&name);
                } else {
                    self.active_scanlists.insert(name.clone());
                }
                self.respond(ack(false, Self::core_error_detail(error)));
                return;
            }
            tracing::info!(
                "MM(MS): scanlist '{name}' {} (not registered); desired groups now {:?}",
                if active { "activated" } else { "deactivated" },
                self.attach_groups()
            );
            self.respond(ack(true, "scanlist state updated; will affiliate at next registration".to_string()));
            return;
        }

        // Already in the desired affiliation state on air (e.g. the scan list's
        // groups overlap other active lists / the base set): nothing to send.
        if to_attach.is_empty() && to_detach.is_empty() {
            tracing::info!(
                "MM(MS): scanlist '{name}' {}; no group affiliation change required",
                if active { "activated" } else { "deactivated" }
            );
            self.respond(ack(true, "scanlist state updated; no affiliation change required".to_string()));
            return;
        }

        // Registered: apply the delta via a standalone group attach/detach
        // amendment (cl. 16.8.2). The confirmed attached set is reconciled from
        // the SwMI acknowledgement (G3), not predicted here.
        match self.begin_group_amendment(queue, &to_attach, &to_detach, 0) {
            Ok(()) => {
                tracing::info!(
                    "MM(MS): scanlist '{name}' {}; standalone group amendment attach {:?} detach {:?} (cl. 16.8.2)",
                    if active { "activated" } else { "deactivated" },
                    to_attach,
                    to_detach
                );
                self.respond(ack(true, "scanlist activation in progress (group attach/detach)".to_string()));
            }
            Err(reason) => {
                // Roll back the active-set change so a later retry re-derives the
                // same delta.
                if active {
                    self.active_scanlists.remove(&name);
                } else {
                    self.active_scanlists.insert(name.clone());
                }
                self.respond(ack(false, format!("scanlist activation deferred: {reason}")));
            }
        }
    }

    /// Whether a manual-selection operation (survey / mode switch / camp) is
    /// permitted right now (**NON-STANDARD**, Plane B; resolved design decision
    /// #4). These operations retune the SDR / change the serving cell, which
    /// would disrupt an active call, so they must be blocked while a call is up.
    ///
    /// MM does not yet hold CMCE call state (there is no CMCE->MM call-active
    /// signalling primitive in the stack). Rather than invent one or guess, the
    /// guard is intentionally conservative-but-honest: it is a single chokepoint
    /// so that, once a CMCE->MM "call active" indication is wired, only this
    /// method changes. Today it returns `Ok(())` (no known active call visible to
    /// MM). Documented as a follow-up; the survey itself is receive-only.
    fn manual_selection_allowed(&self) -> Result<(), String> {
        Ok(())
    }

    /// Send an LMM-SAP request primitive to the MLE (**NON-STANDARD**, Plane B
    /// internal plumbing for manual cell selection; not an ETSI primitive).
    fn send_lmm_to_mle(&self, queue: &mut MessageQueue, msg: SapMsgInner) {
        queue.push_back(SapMsg {
            sap: Sap::LmmSap,
            src: TetraEntity::Mm,
            dest: TetraEntity::Mle,
            msg,
        });
    }

    /// Set the cell-selection mode (Plane B; cl. 18.3.4 policy). Relays the mode
    /// to the MLE and mirrors it for the runtime snapshot.
    fn handle_set_cell_selection_mode(&mut self, handle: u32, manual: bool, queue: &mut MessageQueue) {
        use crate::management::ManagementResponse;
        use crate::net_control::ControlResponse;
        let ack = |accepted: bool, message: String| {
            ControlResponse::Management(ManagementResponse::Ack {
                handle,
                accepted,
                restart_required: false,
                message,
            })
        };

        if let Err(reason) = self.manual_selection_allowed() {
            self.respond(ack(false, format!("cell-selection mode change rejected: {reason}")));
            return;
        }

        // Switching to manual selection abandons automatic (re)selection and
        // leaves the operator in control of the serving cell. A registration is
        // bound to the serving cell's location area (cl. 16.4), so entering
        // manual mode while registered must first cleanly de-register from the
        // SwMI (U-ITSI DETACH, cl. 16.6.1) rather than leave a stale
        // registration behind. Manual mode keeps the current camp in place (see
        // MLE `rx_lmm_mle_selection_mode_req`), so the detach transmits normally
        // over the still-usable serving-cell link during the drain.
        if manual && self.reg_state == RegState::Registered {
            tracing::info!("MM(MS): entering manual selection while registered - de-registering (U-ITSI DETACH, cl. 16.6.1)");
            self.begin_deregistration(queue);
        }

        self.selection_mode_manual = manual;
        self.send_lmm_to_mle(queue, SapMsgInner::LmmMleSelectionModeReq(LmmMleSelectionModeReq { manual }));
        tracing::info!("MM(MS): cell-selection mode -> {}", if manual { "manual" } else { "automatic" });
        self.respond(ack(
            true,
            format!("cell-selection mode set to {}", if manual { "manual" } else { "automatic" }),
        ));
    }

    /// Start or stop a receive-only cell survey (Plane B; cl. 18.3.4 policy).
    fn handle_cell_scan(&mut self, handle: u32, start: bool, queue: &mut MessageQueue) {
        use crate::management::ManagementResponse;
        use crate::net_control::ControlResponse;
        let ack = |accepted: bool, message: String| {
            ControlResponse::Management(ManagementResponse::Ack {
                handle,
                accepted,
                restart_required: false,
                message,
            })
        };

        if start {
            if let Err(reason) = self.manual_selection_allowed() {
                self.respond(ack(false, format!("cell scan rejected: {reason}")));
                return;
            }
            // A receive-only survey abandons the serving cell (it tunes across
            // candidate carriers), so a registration bound to that cell must be
            // torn down first. If the MS is registered, begin a clean
            // de-registration (U-ITSI DETACH, cl. 16.6.1) while still camped;
            // if a de-registration is already draining (e.g. triggered by the
            // preceding switch to manual selection), wait for it. In either case
            // defer the survey until the detach drain completes so the detach
            // actually reaches the air before the MS tunes away.
            if self.reg_state == RegState::Registered {
                tracing::info!("MM(MS): scan requested while registered - de-registering (U-ITSI DETACH, cl. 16.6.1) before survey");
                self.begin_deregistration(queue);
            }
            if self.deregistration_pending() {
                self.pending_scan_after_detach = true;
                tracing::info!("MM(MS): survey deferred until de-registration drain completes");
                self.respond(ack(true, "cell survey will start after de-registration".to_string()));
                return;
            }
            self.send_lmm_to_mle(queue, SapMsgInner::LmmMleScanReq(LmmMleScanReq { start }));
            tracing::info!("MM(MS): start cell survey");
            self.respond(ack(true, "cell survey started".to_string()));
            return;
        }

        // Stop: cancel any survey and drop a not-yet-launched deferred survey.
        self.pending_scan_after_detach = false;
        self.send_lmm_to_mle(queue, SapMsgInner::LmmMleScanReq(LmmMleScanReq { start }));
        tracing::info!("MM(MS): stop cell survey");
        self.respond(ack(true, "cell survey stopped".to_string()));
    }

    /// Camp on (and optionally register to) an operator-selected carrier
    /// (Plane B; cl. 18.3.4.6 camp + cl. 16.4 registration).
    fn handle_camp_on_cell(&mut self, handle: u32, carrier_hz: u32, register: bool, queue: &mut MessageQueue) {
        use crate::management::ManagementResponse;
        use crate::net_control::ControlResponse;
        let ack = |accepted: bool, message: String| {
            ControlResponse::Management(ManagementResponse::Ack {
                handle,
                accepted,
                restart_required: false,
                message,
            })
        };

        if let Err(reason) = self.manual_selection_allowed() {
            self.respond(ack(false, format!("camp rejected: {reason}")));
            return;
        }

        // Remember the operator's force-register intent; it is consumed on the
        // resulting LMM-ACTIVATE confirmation (see rx_activate_conf).
        self.camp_force_register = register;
        self.send_lmm_to_mle(queue, SapMsgInner::LmmMleCampReq(LmmMleCampReq { carrier_hz, register }));
        tracing::info!("MM(MS): camp on {} Hz (register={})", carrier_hz, register);
        self.respond(ack(true, format!("camping on {carrier_hz} Hz")));
    }

    /// Begin a standalone group-identity amendment (cl. 16.8.2) that attaches
    /// `attach` and detaches `detach` GSSIs in a single U-ATTACH/DETACH GROUP
    /// IDENTITY (amendment mode), WITHOUT emitting any control-plane response.
    /// Used internally for scan-list-driven affiliation changes. Returns `Err`
    /// with a human-readable reason when it cannot start (not registered, an op
    /// already outstanding, or an empty delta).
    fn begin_group_amendment(&mut self, queue: &mut MessageQueue, attach: &[u32], detach: &[u32], handle: u32) -> Result<(), String> {
        let requests: Vec<GroupRequest> = attach
            .iter()
            .map(|&gssi| GroupRequest {
                gssi,
                operation: GroupOperation::Attach {
                    class_of_usage: GROUP_CLASS_OF_USAGE,
                },
            })
            .chain(detach.iter().map(|&gssi| GroupRequest {
                gssi,
                operation: GroupOperation::Detach { reason: 0b10 },
            }))
            .collect();
        self.core_event(
            queue,
            MmEvent::Groups {
                handle,
                detach_all: false,
                report_requested: false,
                requests: &requests,
            },
        )
        .map_err(Self::core_error_detail)
    }

    /// LMM-ACTIVATE confirmation from MLE (cl. 17.3.2): a serving cell has been
    /// selected. Start the registration procedure if the cell requires it.
    fn rx_activate_conf(&mut self, queue: &mut MessageQueue, conf: &LmmMleActivateConf) {
        // The camp intent is consumed only when the protocol can evaluate this cell.
        let force_register = self.camp_force_register;
        if !self.left_system && !matches!(self.reg_state, RegState::Detaching | RegState::Registering) {
            self.camp_force_register = false;
        }
        let _ = self.core_event(
            queue,
            MmEvent::Activate {
                cell: CellId {
                    mcc: conf.mcc,
                    mnc: conf.mnc,
                    la: conf.la,
                },
                registration_required: conf.registration_required,
                system_wide_services: conf.system_wide_services,
                force_register,
            },
        );
    }

    /// Emit a TNMM-REPORT indication (Table 15.4, cl. 15.3.3.6) reporting the
    /// transfer result of the U-ITSI DETACH transmission to the user application.
    fn emit_report(&self, transfer_result: crate::tnmm::TransferResult) {
        use crate::net_telemetry::TelemetryEvent;
        use crate::tnmm::TnmmReportIndication;
        tracing::info!("MM: -> TNMM-REPORT (U-ITSI DETACH transfer result = {:?})", transfer_result);
        self.emit(TelemetryEvent::TnmmReportIndication(TnmmReportIndication { transfer_result }));
    }

    fn rx_lmm_mle_unitdata_ind(&mut self, queue: &mut MessageQueue, mut message: SapMsg) {
        let SapMsgInner::LmmMleUnitdataInd(prim) = &mut message.msg else {
            return;
        };
        let result = prim.sdu.as_bit_view().and_then(|view| {
            let pos = u16::try_from(prim.sdu.get_pos()).map_err(|_| tetra_ms_core::types::BitError::Capacity)?;
            let remaining = view.len().checked_sub(pos).ok_or(tetra_ms_core::types::BitError::Truncated)?;
            view.window(pos, remaining)
        });
        match result {
            Ok(bits) => {
                let _ = self.core_event(queue, MmEvent::Downlink(bits));
            }
            Err(error) => tracing::warn!("MM: invalid portable SDU: {:?}", error),
        }
    }

    /// Drain and handle inbound external-interface commands (UI -> stack).
    ///
    /// The control endpoint is polled once per downlink tick (mirroring
    /// `MmBs::tick_start`). Commands are drained first so the immutable borrow of
    /// `self.control` is released before any handler mutates MM state.
    ///
    /// Unlike the BS placeholder handler this never panics: commands originate
    /// from an external UI process and an unexpected/unsupported variant must not
    /// be able to crash the MS stack.
    fn poll_control(&mut self, queue: &mut MessageQueue) {
        // Replay, in arrival order, any commands buffered while the stack was
        // not yet ticking (deferred by `poll_control_offline`). This runs on the
        // first real tick, so a command that arrived before DL synchronization is
        // acted upon exactly as if it had arrived now — registration / on-air
        // behaviour is unchanged by the offline config-service path.
        if !self.deferred_control.is_empty() {
            let deferred = std::mem::take(&mut self.deferred_control);
            for cmd in deferred {
                self.handle_control_command(queue, cmd);
            }
        }
        let mut commands = Vec::new();
        if let Some(cep) = &self.control {
            while commands.len() < crate::net_control::channel::CONTROL_QUEUE_CAPACITY {
                let Some(cmd) = cep.try_recv() else { break };
                commands.push(cmd);
            }
        }
        for cmd in commands {
            self.handle_control_command(queue, cmd);
        }
    }

    /// Service the *offline-serviceable* external-interface command subset while
    /// the stack is not yet ticking (DL-unsynchronized: `run_stack_ms` recovers
    /// no slot, so there is no stack clock and `tick_start`/`poll_control` never
    /// run). Config read/staging and read-only state queries must work as soon as
    /// the control link is up — independent of registration/service state
    /// (including out of service) — so the UI can inspect and stage the codeplug
    /// on a radio that has never seen a base station.
    ///
    /// Only commands that touch process-local config/state (no air-interface side
    /// effects, no SAP traffic that needs a clock) are handled here; see
    /// [`Self::is_offline_serviceable`]. Every other command (TNMM requests,
    /// scan-list toggles, …) is buffered in `deferred_control` and replayed on
    /// the first real tick by [`Self::poll_control`], so nothing about
    /// registration or on-air timing changes. **NON-STANDARD** (Plane B).
    fn poll_control_offline(&mut self, queue: &mut MessageQueue) {
        let mut commands = Vec::new();
        if let Some(cep) = &self.control {
            while commands.len() < crate::net_control::channel::CONTROL_QUEUE_CAPACITY {
                let Some(cmd) = cep.try_recv() else { break };
                commands.push(cmd);
            }
        }
        for cmd in commands {
            if Self::is_offline_serviceable(&cmd) {
                self.handle_control_command(queue, cmd);
            } else if self.deferred_control.len() < MAX_DEFERRED_CONTROL {
                self.deferred_control.push(cmd);
            } else {
                tracing::warn!("MM: unsynchronized command queue full; rejecting request");
                if let Some(response) = cmd.rejection("radio not synchronized; deferred command queue full") {
                    self.respond(response);
                }
            }
        }
    }

    /// Whether a control command may be serviced before the stack is
    /// synchronized (see [`Self::poll_control_offline`]). Limited to the
    /// management config/state subset, none of which injects SAP traffic that
    /// depends on the (absent) stack clock or a serving cell:
    /// `GetConfig`/`SetConfig`/`ApplyConfig` (config read + staging + the
    /// restart request) and the read-only `GetState`/`GetInterfaceVersion`.
    /// `ActivateScanlist` is deliberately excluded because it resolves to a
    /// group attach/detach (cl. 16.8.2) that must run on a real tick.
    ///
    /// The manual cell-survey commands ARE offline-serviceable: a survey is
    /// specifically run while the MS is not camped/synchronized (that is its
    /// purpose), and the selection-mode / camp requests only stage MLE state.
    fn is_offline_serviceable(cmd: &crate::net_control::ControlCommand) -> bool {
        use crate::management::ManagementCommand;
        use crate::net_control::ControlCommand;
        matches!(
            cmd,
            ControlCommand::Management(
                ManagementCommand::GetConfig { .. }
                    | ManagementCommand::SetConfig { .. }
                    | ManagementCommand::ApplyConfig { .. }
                    | ManagementCommand::GetState { .. }
                    | ManagementCommand::GetInterfaceVersion { .. }
                    | ManagementCommand::SetCellSelectionMode { .. }
                    | ManagementCommand::StartCellScan { .. }
                    | ManagementCommand::StopCellScan { .. }
                    | ManagementCommand::CampOnCell { .. }
            )
        )
    }

    /// Send a control response back to the UI, if a control endpoint is wired.
    fn respond(&self, response: crate::net_control::ControlResponse) {
        if let Some(cep) = &self.control {
            cep.respond(response);
        }
    }

    /// Handle one inbound control command. TNMM-SAP requests (Plane A, cl. 15.3)
    /// are acted upon here; the TNMM *result* is reported asynchronously through
    /// the outbound TNMM-SAP indications/confirms on the telemetry channel
    /// (cl. 15.3.2), so the control response only acknowledges whether MM acted.
    fn handle_control_command(&mut self, queue: &mut MessageQueue, cmd: crate::net_control::ControlCommand) {
        use crate::net_control::{ControlCommand, ControlResponse};
        match cmd {
            // TNMM-REGISTRATION request (Table 15.5, cl. 15.3.3.7): initiate
            // attachment and registration of the terminal.
            ControlCommand::TnmmRegistration { handle, request } => {
                self.handle_tnmm_registration_request(queue, handle, &request);
            }
            // TNMM-DEREGISTRATION request (Table 15.2, cl. 15.3.3.2): cancel the
            // registration. Reuses the shutdown de-registration path (cl. 16.6.1),
            // which sends U-ITSI DETACH and emits TNMM-SERVICE "out of service".
            ControlCommand::TnmmDeregistration { handle, request } => {
                // Table 15.2 NOTE: with all attached ITSIs detached the ISSI/MCC/
                // MNC need not be present. When present, they must select this
                // MS's own ITSI (single-ITSI stack).
                if let Some(issi) = request.issi
                    && issi != self.own_issi()
                {
                    tracing::warn!(
                        "MM(MS): TNMM-DEREGISTRATION for ISSI {} != own ISSI {}; ignoring",
                        issi,
                        self.own_issi()
                    );
                    self.respond(ControlResponse::TnmmAck {
                        handle,
                        accepted: false,
                        detail: Some("ISSI does not match the configured ITSI".to_string()),
                    });
                    return;
                }
                let acted = self.begin_deregistration(queue);
                let detail = if acted {
                    None
                } else {
                    Some("not registered; nothing to detach".to_string())
                };
                self.respond(ControlResponse::TnmmAck {
                    handle,
                    accepted: true,
                    detail,
                });
            }
            // TNMM-ATTACH DETACH GROUP IDENTITY request (Table 15.1, cl. 15.3.3.1):
            // the standalone MS-initiated group attach/detach procedure (cl. 16.8.2).
            // Builds and sends a U-ATTACH/DETACH GROUP IDENTITY PDU, starts T353 and
            // awaits the D-ATTACH/DETACH GROUP IDENTITY ACKNOWLEDGEMENT (handled in
            // `rx_d_attach_detach_group_identity_acknowledgement`).
            ControlCommand::TnmmAttachDetachGroupIdentity { handle, request } => {
                self.handle_tnmm_attach_detach_group_identity_request(queue, handle, &request);
            }
            // TNMM-STATUS request (Table 15.7, cl. 15.3.3.9): selects direct mode
            // / dual watch / energy economy — none implemented in this stack.
            ControlCommand::TnmmStatus { handle, .. } => {
                tracing::warn!(
                    "MM(MS): TNMM-STATUS request received but direct mode / dual watch / energy \
                     economy are not implemented; ignoring"
                );
                self.respond(ControlResponse::TnmmAck {
                    handle,
                    accepted: false,
                    detail: Some("direct mode / dual watch / energy economy not implemented".to_string()),
                });
            }
            // TNMM-ENERGY SAVING request (Table 15.3, cl. 15.3.3.5): dormant —
            // the energy-economy procedure (cl. 16.7) is not implemented.
            ControlCommand::TnmmEnergySaving { handle, .. } => {
                tracing::warn!("MM(MS): TNMM-ENERGY SAVING request received but energy economy (cl. 16.7) is not implemented; ignoring");
                self.respond(ControlResponse::TnmmAck {
                    handle,
                    accepted: false,
                    detail: Some("energy economy (cl. 16.7) not implemented".to_string()),
                });
            }
            // Management / provisioning (Plane B, **NON-STANDARD**). Served here
            // because MM is the single writer of MS runtime state. See
            // `crate::management` for the standards disclaimer.
            ControlCommand::Management(mgmt) => {
                self.handle_management_command(mgmt, queue);
            }
            // Non-TNMM commands are not addressed to MM(MS); log and drop.
            other => {
                tracing::warn!(
                    "MM(MS): received non-TNMM control command with no MS handler, dropping: {:?}",
                    other
                );
            }
        }
    }

    /// Act on a TNMM-REGISTRATION request (Table 15.5). Validates the requested
    /// ITSI against the configured one (single-ITSI stack) and, when the MS is
    /// idle, initiates the ITSI-attach registration by sending a
    /// U-LOCATION-UPDATE-DEMAND — the same path taken on serving-cell selection
    /// (rx_activate_conf). The registration *result* is reported later through
    /// the TNMM-REGISTRATION indication/confirm.
    fn handle_tnmm_registration_request(&mut self, queue: &mut MessageQueue, handle: u32, request: &crate::tnmm::TnmmRegistrationRequest) {
        use crate::net_control::ControlResponse;
        let cfg = self.config.config();
        if request.issi != self.own_issi() || request.mcc_of_issi != cfg.net.mcc || request.mnc_of_issi != cfg.net.mnc {
            self.respond(ControlResponse::TnmmAck {
                handle,
                accepted: false,
                detail: Some("ISSI/MNI does not match the configured ITSI".to_string()),
            });
            return;
        }
        let result = self.core_event(queue, MmEvent::Register);
        self.respond(ControlResponse::TnmmAck {
            handle,
            accepted: result.is_ok(),
            detail: result.err().map(Self::core_error_detail),
        });
    }

    /// Act on a TNMM-ATTACH DETACH GROUP IDENTITY request (Table 15.1,
    /// cl. 15.3.3.1): initiate the MS-initiated standalone group attach/detach
    /// procedure (cl. 16.8.2). Validates preconditions, builds and sends a
    /// U-ATTACH/DETACH GROUP IDENTITY PDU, starts T353 and stores the operation
    /// as pending until the acknowledgement arrives.
    fn handle_tnmm_attach_detach_group_identity_request(
        &mut self,
        queue: &mut MessageQueue,
        handle: u32,
        request: &crate::tnmm::TnmmAttachDetachGroupIdentityRequest,
    ) {
        use crate::net_control::ControlResponse;
        use crate::tnmm::GroupIdentityAttachDetachMode;
        let requests: Vec<GroupRequest> = request
            .group_identity_request
            .iter()
            .map(Self::group_identity_request_to_uplink)
            .collect();
        let result = self.core_event(
            queue,
            MmEvent::Groups {
                handle,
                detach_all: matches!(
                    request.group_identity_attach_detach_mode,
                    GroupIdentityAttachDetachMode::DetachTheCurrentlyActiveGroupIdentities
                ),
                report_requested: matches!(
                    request.group_identity_report,
                    Some(crate::tnmm::GroupIdentityReport::ReportRequested)
                ),
                requests: &requests,
            },
        );
        self.respond(ControlResponse::TnmmAck {
            handle,
            accepted: result.is_ok(),
            detail: result.err().map(Self::core_error_detail),
        });
    }

    /// Map a TNMM `Group identity request` (Table 15.9) to an on-air Group
    /// identity uplink element (cl. 16.10.27). Attachment carries the class of
    /// usage (cl. 16.10.6); detachment carries the detachment reason
    /// (cl. 16.10.21). Addressed by GSSI only (address type 0).
    fn group_identity_request_to_uplink(g: &crate::tnmm::GroupIdentityRequest) -> GroupRequest {
        use crate::tnmm::GroupIdentityAttachDetachTypeIdentifier;
        GroupRequest {
            gssi: (g.gtsi & 0xff_ffff) as u32,
            operation: match g.group_identity_attach_detach_type_identifier {
                GroupIdentityAttachDetachTypeIdentifier::Attachment => GroupOperation::Attach {
                    class_of_usage: g.class_of_usage.map(Self::class_of_usage_to_raw).unwrap_or(GROUP_CLASS_OF_USAGE),
                },
                GroupIdentityAttachDetachTypeIdentifier::Detachment => GroupOperation::Detach {
                    reason: g
                        .group_identity_detachment_request
                        .map(Self::detachment_request_to_raw)
                        .unwrap_or(0b10),
                },
            },
        }
    }

    /// On-air `Class of usage` raw 3-bit encoding (cl. 16.10.6, Table 16.32):
    /// "Class of usage N" is transmitted as raw value (N − 1).
    fn class_of_usage_to_raw(cou: crate::tnmm::ClassOfUsage) -> u8 {
        use crate::tnmm::ClassOfUsage::*;
        match cou {
            ClassOfUsage1 => 0,
            ClassOfUsage2 => 1,
            ClassOfUsage3 => 2,
            ClassOfUsage4 => 3,
            ClassOfUsage5 => 4,
            ClassOfUsage6 => 5,
            ClassOfUsage7 => 6,
            ClassOfUsage8 => 7,
        }
    }

    /// On-air `Class of usage` decode (cl. 16.10.6, Table 16.32): raw R → "Class
    /// of usage (R + 1)". Inverse of [`Self::class_of_usage_to_raw`].
    fn class_of_usage_from_raw(raw: u8) -> crate::tnmm::ClassOfUsage {
        use crate::tnmm::ClassOfUsage::*;
        match raw & 0b111 {
            0 => ClassOfUsage1,
            1 => ClassOfUsage2,
            2 => ClassOfUsage3,
            3 => ClassOfUsage4,
            4 => ClassOfUsage5,
            5 => ClassOfUsage6,
            6 => ClassOfUsage7,
            _ => ClassOfUsage8,
        }
    }

    /// On-air `Group identity detachment uplink` raw 2-bit encoding
    /// (cl. 16.10.21, Table 16.53). The TNMM SAP only defines the
    /// user-initiated detachment reason (Table 15.9), which maps to 102.
    fn detachment_request_to_raw(_req: crate::tnmm::GroupIdentityDetachmentRequest) -> u8 {
        // GroupIdentityDetachmentRequest::UserInitiatedDetachment -> "User initiated".
        0b10
    }

    /// Emit a TNMM-ATTACH DETACH GROUP IDENTITY confirm (Table 15.1,
    /// cl. 15.3.3.1) reporting the outcome of a standalone group attach/detach.
    /// The `group_identities` list reports each acknowledged group as a GTSI
    /// (cl. 16.8.2: "using only the GTSIs since the (V)GSSIs is not known by the
    /// user application").
    fn emit_group_identity_confirm(&self, op: &PendingGroupOp, accepted_attach: &GroupAttachments, accepted_detach: &[u32]) {
        use crate::net_telemetry::TelemetryEvent;
        use crate::tnmm::{
            GroupIdentity, GroupIdentityAttachDetachMode, GroupIdentityAttachDetachTypeIdentifier, GroupIdentityDetachment,
            GroupIdentityReport, TnmmAttachDetachGroupIdentityConfirm,
        };

        let mni = self.own_mni() as u64;
        let mut group_identities = self.attached_group_identities(accepted_attach);
        for gssi in accepted_detach {
            group_identities.push(GroupIdentity {
                gtsi: (mni << 24) | (*gssi as u64),
                group_identity_attach_detach_type_identifier: GroupIdentityAttachDetachTypeIdentifier::Detachment,
                group_identity_lifetime: None,
                class_of_usage: None,
                group_identity_detachment_reason: Some(GroupIdentityDetachment::PermanentlyDetached),
            });
        }

        let group_identity_attach_detach_mode = if op.detach_all {
            GroupIdentityAttachDetachMode::DetachTheCurrentlyActiveGroupIdentities
        } else {
            GroupIdentityAttachDetachMode::Amendment
        };

        self.emit(TelemetryEvent::TnmmAttachDetachGroupIdentityConfirm(
            TnmmAttachDetachGroupIdentityConfirm {
                group_identity_attach_detach_mode,
                group_identity_report: op.report_requested.then_some(GroupIdentityReport::ReportNotRequested),
                group_identities,
            },
        ));
    }

    // ----------------------------------------------------------------------
    // Management / provisioning handlers (Plane B, **NON-STANDARD**).
    //
    // Implementation-defined stack provisioning + runtime-state reads. NOT part
    // of any ETSI standard (see `crate::management`). Carried over the reused
    // control transport in the dedicated `Management` variant.
    // ----------------------------------------------------------------------

    /// Handle one inbound management command (Plane B, non-standard).
    fn handle_management_command(&mut self, cmd: crate::management::ManagementCommand, queue: &mut MessageQueue) {
        use crate::management::{ManagementCommand, ManagementResponse};
        use crate::net_control::ControlResponse;
        match cmd {
            // Read-only runtime state snapshot; always serviceable.
            ManagementCommand::GetState { handle } => {
                let state = self.runtime_snapshot();
                self.respond(ControlResponse::Management(ManagementResponse::State {
                    handle,
                    state: Box::new(state),
                }));
            }
            // Frozen interface schema version discovery; always serviceable.
            ManagementCommand::GetInterfaceVersion { handle } => {
                self.respond(ControlResponse::Management(ManagementResponse::InterfaceVersion {
                    handle,
                    version: crate::management::MS_INTERFACE_SCHEMA_VERSION.to_string(),
                }));
            }
            // Read the active configuration as canonical TOML (always serviceable).
            // Secrets are redacted on the wire; the real values never leave the
            // process (restored on SetConfig write-back from the current config).
            ManagementCommand::GetConfig { handle } => {
                let cfg = self.config.config();
                match tetra_config::magentastation::to_toml_string_redacted(&cfg) {
                    Ok(toml) => {
                        self.respond(ControlResponse::Management(ManagementResponse::Config { handle, toml }));
                    }
                    Err(e) => {
                        tracing::error!("MM(MS): failed to serialize active config: {e}");
                        self.respond(ControlResponse::Management(ManagementResponse::Error {
                            handle,
                            message: format!("failed to serialize active config: {e}"),
                        }));
                    }
                }
            }
            // Stage a new configuration: validate through the exact startup
            // validator, then persist to disk. Structural changes take effect on
            // the next `ApplyConfig`; the process is NOT bounced here.
            ManagementCommand::SetConfig { handle, toml } => {
                self.handle_set_config(handle, toml);
            }
            // Apply staged configuration: drain-and-restart under the external
            // supervisor. Reuses the graceful de-registration drain (cl. 16.6.1).
            ManagementCommand::ApplyConfig { handle } => {
                self.handle_apply_config(handle, queue);
            }
            // Activate/deactivate a programmed scan list at runtime (Plane B).
            ManagementCommand::ActivateScanlist { handle, name, active } => {
                self.handle_activate_scanlist(handle, name, active, queue);
            }
            // Manual cell-selection mode toggle (Plane B, cl. 18.3.4 policy).
            ManagementCommand::SetCellSelectionMode { handle, manual } => {
                self.handle_set_cell_selection_mode(handle, manual, queue);
            }
            // Start a receive-only survey of the codeplug candidate carriers.
            ManagementCommand::StartCellScan { handle } => {
                self.handle_cell_scan(handle, true, queue);
            }
            // Abort an in-progress survey.
            ManagementCommand::StopCellScan { handle } => {
                self.handle_cell_scan(handle, false, queue);
            }
            // Camp on / register to an operator-selected carrier.
            ManagementCommand::CampOnCell {
                handle,
                carrier_hz,
                register,
            } => {
                self.handle_camp_on_cell(handle, carrier_hz, register, queue);
            }
        }
    }

    /// Validate + persist a staged configuration (Plane B, non-standard).
    ///
    /// The payload is parsed through [`tetra_config::magentastation::from_toml_str`]
    /// — the exact validator the stack runs at startup — so a config that would
    /// fail to load is rejected here rather than bricking the radio on restart.
    /// On success it is re-serialized to canonical form and written to the
    /// config file, and `restart_required` is set so the UI shows a pending
    /// restart. The running process keeps its current (old) config until an
    /// explicit `ApplyConfig`.
    fn handle_set_config(&mut self, handle: u32, toml: String) {
        use crate::management::ManagementResponse;
        use crate::net_control::ControlResponse;

        let ctx = match &self.mgmt_ctx {
            Some(ctx) => ctx,
            None => {
                tracing::warn!("MM(MS): SetConfig received but no management context installed; refusing");
                self.respond(ControlResponse::Management(ManagementResponse::Ack {
                    handle,
                    accepted: false,
                    restart_required: self.restart_required,
                    message: "configuration write-back is not available in this deployment".to_string(),
                }));
                return;
            }
        };

        // 1) Validate: parse through the startup validator.
        let parsed = match tetra_config::magentastation::from_toml_str(&toml) {
            Ok(cfg) => cfg,
            Err(e) => {
                let summary = tetra_config::magentastation::config_error_summary(&toml, e.as_ref());
                tracing::warn!("MM(MS): SetConfig rejected invalid configuration: {summary}");
                self.respond(ControlResponse::Management(ManagementResponse::Ack {
                    handle,
                    accepted: false,
                    restart_required: self.restart_required,
                    message: format!("configuration rejected: {summary}"),
                }));
                return;
            }
        };

        // 2) Merge redacted secrets: a UI that read the config via GetConfig sees
        //    secrets as the REDACTED_SECRET sentinel. Preserve the live on-disk
        //    value for any secret returned unchanged (sentinel), so a benign
        //    round-trip edit never clobbers a credential; genuinely-new secrets
        //    (not the sentinel) are kept as supplied.
        let current = self.config.config();
        let parsed = tetra_config::magentastation::restore_redacted_secrets(parsed, &current);

        // 3) Normalize: re-serialize so the on-disk file is canonical and
        //    provably re-parses to the validated config.
        let normalized = match tetra_config::magentastation::to_toml_string(&parsed) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("MM(MS): SetConfig failed to normalize configuration: {e}");
                self.respond(ControlResponse::Management(ManagementResponse::Ack {
                    handle,
                    accepted: false,
                    restart_required: self.restart_required,
                    message: format!("configuration could not be serialized: {e}"),
                }));
                return;
            }
        };

        // 4) Persist to the config file path (so a later restart / cold boot
        //    loads the same configuration that is applied live below).
        let path = ctx.config_path.clone();
        if let Err(e) = std::fs::write(&path, normalized) {
            tracing::error!("MM(MS): SetConfig failed to write {}: {e}", path.display());
            self.respond(ControlResponse::Management(ManagementResponse::Ack {
                handle,
                accepted: false,
                restart_required: self.restart_required,
                message: format!("failed to write configuration file: {e}"),
            }));
            return;
        }

        // 5) Classify the change (Plane B hybrid apply):
        //    - OPERATIONAL (codeplug-only: folders/talkgroups/networks/scan lists/
        //      frequency lists/carrier overrides/gateways/contacts/settings) is
        //      hot-swapped LIVE with no restart — every entity observes the new
        //      config on its next `config()` read.
        //    - STRUCTURAL (stack_mode / PHY / net / cell / ms / duplex / control /
        //      telemetry / brew) still requires a controlled restart via
        //      ApplyConfig; it is only staged to disk here.
        let operational_only = tetra_config::magentastation::is_operational_only_change(&current, &parsed);
        if operational_only {
            self.config.apply_config_live(parsed);
            tracing::info!(
                "MM(MS): SetConfig applied LIVE (operational/codeplug change) — no restart required ({})",
                path.display()
            );
            self.respond(ControlResponse::Management(ManagementResponse::Ack {
                handle,
                accepted: true,
                restart_required: self.restart_required,
                message: "configuration applied live (operational change; no restart required)".to_string(),
            }));
            return;
        }

        self.restart_required = true;
        tracing::info!(
            "MM(MS): staged new configuration to {} (structural change — restart required to apply)",
            path.display()
        );
        self.respond(ControlResponse::Management(ManagementResponse::Ack {
            handle,
            accepted: true,
            restart_required: true,
            message: "configuration staged; apply (restart) required to take effect".to_string(),
        }));
    }

    /// Apply staged configuration by draining and requesting a controlled
    /// restart (Plane B, non-standard).
    ///
    /// This reuses the existing graceful shutdown path: if the MS is registered
    /// it begins de-registration (U-ITSI DETACH, cl. 16.6.1) so the drain sends
    /// it, then clears the shared `is_running` flag to break the stack run loop
    /// and sets `restart_requested` so `main` exits with the restart exit code.
    /// The external supervisor respawns the process with the new config. It does
    /// **not** relaunch in-process (the SDR handle and thread graph are owned by
    /// `main`).
    fn handle_apply_config(&mut self, handle: u32, queue: &mut MessageQueue) {
        use crate::management::ManagementResponse;
        use crate::net_control::ControlResponse;

        let ctx = match &self.mgmt_ctx {
            Some(ctx) => ctx,
            None => {
                tracing::warn!("MM(MS): ApplyConfig received but no management context installed; refusing");
                self.respond(ControlResponse::Management(ManagementResponse::Ack {
                    handle,
                    accepted: false,
                    restart_required: self.restart_required,
                    message: "restart/apply is not available in this deployment".to_string(),
                }));
                return;
            }
        };

        // Nothing structural is staged (the last SetConfig applied live, or no
        // config change is pending): a restart would needlessly bounce the radio.
        // ApplyConfig only exists to apply a STAGED structural change, so with
        // nothing pending it is a success no-op. This makes the UI's unconditional
        // "Save then Apply" flow correct for codeplug/operational-only edits, which
        // have already taken effect live.
        if !self.restart_required {
            tracing::info!("MM(MS): ApplyConfig received but no structural change is staged — no restart (already up to date)");
            self.respond(ControlResponse::Management(ManagementResponse::Ack {
                handle,
                accepted: true,
                restart_required: false,
                message: "no restart required; configuration is already up to date (last change applied live)".to_string(),
            }));
            return;
        }

        // Queue the final response before shutdown. Main joins the worker after
        // dropping the router, which drains all queued responses before exit.
        let response = ControlResponse::Management(ManagementResponse::Ack {
            handle,
            accepted: true,
            restart_required: self.restart_required,
            message: "restart initiated; the stack will de-register and respawn with the staged configuration".to_string(),
        });
        let Some(control) = &self.control else {
            tracing::error!("MM: ApplyConfig cannot acknowledge without a control endpoint");
            return;
        };
        if control.try_respond(response).is_err() {
            tracing::error!("MM: ApplyConfig refused: acknowledgement queue full or closed");
            return;
        }
        ctx.restart_requested.store(true, Ordering::SeqCst);
        ctx.is_running.store(false, Ordering::SeqCst);

        // Reuse the graceful de-registration drain so a registered MS cleanly
        // detaches before the restart (best-effort, cl. 16.6.1).
        let detaching = self.begin_deregistration(queue);

        tracing::info!(
            "MM(MS): ApplyConfig -> restart requested (detach initiated: {}); supervisor will respawn with new config",
            detaching
        );
    }

    /// Build an [`crate::management::MsRuntimeState`] snapshot from MM's own
    /// state and the active configuration (Plane B, non-standard). MM is the
    /// single writer of MS runtime state, so no locking is required.
    fn runtime_snapshot(&self) -> crate::management::MsRuntimeState {
        use crate::management::{MsRuntimeState, RegistrationState};

        let registration_state = match self.reg_state {
            RegState::Idle => RegistrationState::Idle,
            RegState::Registering => RegistrationState::Registering,
            RegState::Registered => RegistrationState::Registered,
            RegState::Detaching => RegistrationState::Detaching,
        };
        // Derived service status: mirrors the vocabulary of the TNMM-SERVICE
        // indication (Plane A) without asserting a standardized primitive.
        // Reflects both registration state and serving-cell link status
        // (out_of_service), so a link break shows OutOfService in the snapshot.
        let service_status = self.derived_service_status();
        let cfg = self.config.config();
        MsRuntimeState {
            registration_state,
            service_status,
            own_issi: self.own_issi(),
            home_mcc: cfg.net.mcc,
            home_mnc: cfg.net.mnc,
            serving_la: self.serving_la,
            rssi_dbfs: self.serving_rssi_dbfs,
            colour_code: cfg.cell.colour_code,
            attached_groups: self.attach_groups(),
            active_scanlists: self.active_scanlists.iter().cloned().collect(),
            restart_required: self.restart_required,
            selection_mode_manual: self.selection_mode_manual,
        }
    }
}
impl TetraEntityTrait for MmMs {
    fn entity(&self) -> TetraEntity {
        TetraEntity::Mm
    }

    fn set_config(&mut self, config: SharedConfig) {
        let cfg = config.config();
        let result = self.core.configure(MmConfig {
            issi: cfg.ms.as_ref().expect("MS config section required in MS mode").issi,
            mcc: cfg.net.mcc,
            mnc: cfg.net.mnc,
            initial_la: cfg.cell.location_area,
        });
        if let Err(error) = result {
            tracing::error!("MM: rejected portable configuration: {:?}", error);
            return;
        }
        self.config = config;
    }

    /// Service the offline-serviceable management subset (config read/staging +
    /// read-only state) while the stack is not yet synchronized. The router
    /// calls this each `run_stack_ms` iteration that recovers no downlink slot,
    /// so the external interface answers config/state requests as soon as the
    /// control link is up — regardless of registration/service state. All other
    /// commands are buffered and replayed on the first real tick. See
    /// `MmMs::poll_control_offline`.
    fn drive_offline_control(&mut self, queue: &mut MessageQueue) {
        self.poll_control_offline(queue);
    }

    fn tick_start(&mut self, queue: &mut MessageQueue, ts: TdmaTime) {
        self.poll_control(queue);
        // A first/recovered clock contributes one slot, not the unsynchronized
        // interval. Duplicate or backwards clock corrections add no protocol time.
        let elapsed = self.last_tick.map_or(1, |last| ts.diff(last).max(0) as u32);
        self.last_tick = Some(ts);
        let detach_transfer = self
            .detach_reporter
            .as_ref()
            .map(|reporter| match reporter.get_state() {
                TxState::Pending => TransferState::Pending,
                TxState::Transmitted => TransferState::Transmitted,
                TxState::Acknowledged => TransferState::Acknowledged,
                TxState::Discarded => TransferState::Discarded,
                TxState::Lost => TransferState::Lost,
            })
            .unwrap_or(TransferState::Pending);
        self.advance_time(queue, elapsed, detach_transfer);
    }

    fn rx_prim(&mut self, queue: &mut MessageQueue, message: SapMsg) {
        tracing::trace!("rx_prim: {:?}", message);

        // There is only one SAP for MM
        assert!(message.sap == Sap::LmmSap);

        match &message.msg {
            SapMsgInner::LmmMleUnitdataInd(_) => {
                self.rx_lmm_mle_unitdata_ind(queue, message);
            }
            SapMsgInner::LmmMleActivateConf(conf) => {
                let conf = conf.clone();
                self.rx_activate_conf(queue, &conf);
            }
            SapMsgInner::LmmMleRssiInd(ind) => {
                // Implementation-defined (Plane B): cache the serving-cell level
                // for the management runtime state / UI receive-level meter.
                self.serving_rssi_dbfs = ind.rssi_dbfs;
            }
            SapMsgInner::LmmMleBreakInd(_) => {
                self.rx_mle_break();
            }
            SapMsgInner::LmmMleReopenInd(_) => {
                self.rx_mle_reopen();
            }
            SapMsgInner::LmmMleScanResultInd(ind) => {
                // Manual survey (Plane B): forward the found cell to the UI as a
                // telemetry event. Parsed per spec at the MLE; MM only relays.
                self.emit(crate::net_telemetry::TelemetryEvent::MsScanResult {
                    carrier_hz: ind.carrier_hz,
                    mcc: ind.mcc,
                    mnc: ind.mnc,
                    location_area: ind.location_area,
                    colour_code: ind.colour_code,
                    rssi_dbfs: ind.rssi_dbfs,
                    registration_required: ind.registration_required,
                    late_entry_supported: ind.late_entry_supported,
                });
            }
            SapMsgInner::LmmMleScanCompleteInd(ind) => {
                self.emit(crate::net_telemetry::TelemetryEvent::MsScanComplete {
                    found: ind.found,
                    scanned: ind.scanned,
                });
            }
            _ => {
                panic!();
            }
        }
    }

    /// Shutdown hook (cl. 16.6.1): if the MS is registered, emit a U-ITSI DETACH
    /// and ask the router to keep running so it can be transmitted. Returns
    /// `true` only when a detach was actually initiated.
    fn begin_deregistration(&mut self, queue: &mut MessageQueue) -> bool {
        if self.core_event(queue, MmEvent::Deregister).is_err() {
            return false;
        }
        self.core.deregistration_pending()
    }

    fn deregistration_pending(&self) -> bool {
        self.core.deregistration_pending()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net_control::ControlCommand;
    use crate::net_telemetry::TelemetryEvent;
    use tetra_config::magentastation::from_toml_str;
    use tetra_pdus::mm::pdus::d_location_update_accept::DLocationUpdateAccept;
    use tetra_pdus::mm::pdus::u_itsi_detach::UItsiDetach;
    use tetra_saps::lmm::LmmMleRssiInd;
    use tetra_saps::lmm::LmmMleUnitdataInd;
    use tetra_saps::lmm::{LmmMleScanCompleteInd, LmmMleScanResultInd};

    const MS_ISSI: u32 = 1000001;

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

    fn ms_mm() -> MmMs {
        let cfg = from_toml_str(MS_TOML).expect("valid MS test config");
        MmMs::new(SharedConfig::from_parts(cfg, None), None, None)
    }

    #[test]
    fn recovered_mm_time_covers_skips_rollover_duplicates_and_resynchronization() {
        let mut mm = ms_mm();
        let mut queue = MessageQueue::new();
        mm.rx_activate_conf(&mut queue, &activate_conf(true));
        while queue.pop_front().is_some() {}
        let first = TdmaTime::from_int(-2);
        mm.tick_start(&mut queue, first);
        let before = mm.t351_countdown;
        mm.tick_start(&mut queue, first.add_timeslots(7));
        assert_eq!(mm.t351_countdown, before - 7);
        mm.tick_start(&mut queue, first.add_timeslots(7));
        assert_eq!(mm.t351_countdown, before - 7);
        mm.rx_mle_break();
        mm.rx_mle_reopen();
        mm.tick_start(&mut queue, first.add_timeslots(1000));
        assert_eq!(mm.t351_countdown, before - 8, "recovery bootstraps one slot, not the lost interval");
        mm.tick_start(&mut queue, first.add_timeslots(1100));
        assert_eq!(mm.t351_countdown, before - 108);
    }

    #[test]
    fn long_mm_batch_preserves_all_retries_when_action_budget_requires_splitting() {
        let mut mm = ms_mm();
        let mut queue = MessageQueue::new();
        mm.rx_activate_conf(&mut queue, &activate_conf(true));
        while queue.pop_front().is_some() {}
        mm.tick_start(&mut queue, TdmaTime::default());
        mm.tick_start(&mut queue, TdmaTime::from_int(10_000));
        assert_eq!(mm.attempts, tetra_ms_core::mm::MAX_REGISTRATION_ATTEMPTS);
        let mut retries = 0;
        while queue.pop_front().is_some() {
            retries += 1;
        }
        assert_eq!(retries, 4, "every retry reached the adapter queue");
    }

    #[test]
    fn offline_request_flood_is_bounded_rejected_and_replays_once_after_recovery() {
        use crate::net_control::ControlResponse;
        use crate::net_control::channel::make_control_link;
        let mut mm = ms_mm();
        let (dispatcher, endpoint) = make_control_link();
        mm.control = Some(endpoint);
        let mut queue = MessageQueue::new();
        for handle in 0..MAX_DEFERRED_CONTROL as u32 * 3 {
            dispatcher
                .try_send(ControlCommand::TnmmStatus {
                    handle,
                    request: crate::tnmm::TnmmStatusRequest {
                        direct_mode: None,
                        dual_watch: None,
                        energy_economy_mode: None,
                    },
                })
                .unwrap();
            mm.drive_offline_control(&mut queue);
            if handle >= MAX_DEFERRED_CONTROL as u32 {
                assert!(matches!(
                    dispatcher.try_recv_response(),
                    Some(ControlResponse::TnmmAck { accepted: false, .. })
                ));
            }
        }
        assert_eq!(mm.deferred_control.len(), MAX_DEFERRED_CONTROL);
        assert!(queue.pop_front().is_none());
        mm.tick_start(&mut queue, TdmaTime::default());
        assert!(mm.deferred_control.is_empty());
        assert_eq!(dispatcher.try_recv_responses().len(), MAX_DEFERRED_CONTROL);
        mm.tick_start(&mut queue, TdmaTime::from_int(1));
        assert!(dispatcher.try_recv_response().is_none());
    }

    #[test]
    fn registration_and_group_confirm_report_the_shared_wire_class() {
        for raw in 0..8 {
            assert_eq!(MmMs::class_of_usage_to_raw(MmMs::class_of_usage_from_raw(raw)), raw);
            for lifetime in 0..4 {
                let (mut mm, source, _) = ms_mm_wired();
                let mut queue = MessageQueue::new();
                register_wired(&mut mm, &mut queue, &source);
                mm.begin_group_amendment(&mut queue, &[10], &[], 100).unwrap();
                deliver_dl(&mut mm, &mut queue, build_group_ack_metadata(&[10], &[], raw, lifetime));
                let events = drain(&source);
                let confirmation = events
                    .iter()
                    .find_map(|event| match event {
                        TelemetryEvent::TnmmAttachDetachGroupIdentityConfirm(confirm) => Some(confirm),
                        _ => None,
                    })
                    .unwrap();
                assert_eq!(
                    MmMs::class_of_usage_to_raw(confirmation.group_identities[0].class_of_usage.unwrap()),
                    raw
                );
                assert_eq!(
                    confirmation.group_identities[0].group_identity_lifetime,
                    Some(match lifetime {
                        0 => crate::tnmm::GroupIdentityLifetime::PermanentAttachmentNotNeeded,
                        1 => crate::tnmm::GroupIdentityLifetime::AttachmentNeededForNextItsiAttach,
                        2 => crate::tnmm::GroupIdentityLifetime::AttachmentNotAllowedAfterNextItsiAttach,
                        _ => crate::tnmm::GroupIdentityLifetime::AttachmentNeededForNextLocationUpdate,
                    })
                );
            }
        }
    }

    #[test]
    fn staged_legacy_cell_edit_survives_disk_reload_without_changing_live_state() {
        use crate::management::ManagementCommand;
        let (mut mm, _, dispatcher) = ms_mm_wired();
        let path = std::env::temp_dir().join(format!("tetra-ms-cell-{}.toml", uuid::Uuid::new_v4()));
        mm.set_management_context(path.clone(), Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(true)));
        dispatcher.send(ControlCommand::Management(ManagementCommand::SetConfig {
            handle: 901,
            toml: MS_TOML
                .replace("location_area = 1", "location_area = 2")
                .replace("colour_code = 1", "colour_code = 3"),
        }));
        mm.drive_offline_control(&mut MessageQueue::new());
        let (_, accepted, restart) = last_mgmt_ack(&dispatcher);
        assert!(accepted && restart);
        let restored = from_toml_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!((restored.cell.location_area, restored.cell.colour_code), (2, 3));
        assert_eq!((mm.config.config().cell.location_area, mm.config.config().cell.colour_code), (1, 1));
    }

    fn ms_mm_with_groups(groups: &[u32]) -> MmMs {
        let list = groups.iter().map(u32::to_string).collect::<Vec<_>>().join(", ");
        let toml = MS_TOML.replace("attach_groups = []", &format!("attach_groups = [{list}]"));
        let cfg = from_toml_str(&toml).expect("valid MS test config");
        MmMs::new(SharedConfig::from_parts(cfg, None), None, None)
    }

    fn activate_conf(registration_required: bool) -> LmmMleActivateConf {
        activate_conf_cell(registration_required, 901, 9999, 1)
    }

    fn activate_conf_cell(registration_required: bool, mcc: u16, mnc: u16, la: u16) -> LmmMleActivateConf {
        activate_conf_cell_sws(registration_required, mcc, mnc, la, true)
    }

    fn activate_conf_cell_sws(registration_required: bool, mcc: u16, mnc: u16, la: u16, system_wide_services: bool) -> LmmMleActivateConf {
        LmmMleActivateConf {
            registration_required,
            mcc,
            mnc,
            la,
            system_wide_services,
            cell_type: 0,
        }
    }

    /// Deliver a downlink MM PDU (already positioned at the MM PDU type) to MM as
    /// an LMM-UNITDATA indication, mirroring what MLE forwards.
    fn deliver_dl(mm: &mut MmMs, queue: &mut MessageQueue, mut sdu: BitBuffer) {
        sdu.seek(0);
        let msg = SapMsg {
            sap: Sap::LmmSap,
            src: TetraEntity::Mle,
            dest: TetraEntity::Mm,
            msg: SapMsgInner::LmmMleUnitdataInd(LmmMleUnitdataInd {
                sdu,
                handle: 0,
                received_address: TetraAddress::issi(MS_ISSI),
            }),
        };
        mm.rx_prim(queue, msg);
    }

    fn build_accept() -> BitBuffer {
        let pdu = DLocationUpdateAccept {
            location_update_accept_type: LocationUpdateAcceptType::ItsiAttach,
            ssi: Some(MS_ISSI as u64),
            address_extension: None,
            subscriber_class: None,
            energy_saving_information: None,
            scch_information_and_distribution_on_18th_frame: None,
            new_registered_area: None,
            security_downlink: None,
            group_identity_location_accept: None,
            default_group_attachment_lifetime: None,
            authentication_downlink: None,
            group_identity_security_related_information: None,
            cell_type_control: None,
            proprietary: None,
        };
        let mut sdu = BitBuffer::new_autoexpand(8);
        pdu.to_bitbuf(&mut sdu).unwrap();
        sdu
    }

    /// The serving-cell RSSI reported by MLE (implementation-defined Plane B
    /// `LmmMleRssiInd`) is cached by MM and surfaced in the `GetState` runtime
    /// snapshot the UI reads.
    #[test]
    fn test_rssi_reported_in_runtime_state() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();

        // No measurement yet.
        assert_eq!(mm.runtime_snapshot().rssi_dbfs, None);

        let msg = SapMsg {
            sap: Sap::LmmSap,
            src: TetraEntity::Mle,
            dest: TetraEntity::Mm,
            msg: SapMsgInner::LmmMleRssiInd(LmmMleRssiInd { rssi_dbfs: Some(-63.0) }),
        };
        mm.rx_prim(&mut q, msg);
        assert_eq!(mm.runtime_snapshot().rssi_dbfs, Some(-63.0));

        // A subsequent out-of-service clear (None) is reflected too.
        let clear = SapMsg {
            sap: Sap::LmmSap,
            src: TetraEntity::Mle,
            dest: TetraEntity::Mm,
            msg: SapMsgInner::LmmMleRssiInd(LmmMleRssiInd { rssi_dbfs: None }),
        };
        mm.rx_prim(&mut q, clear);
        assert_eq!(mm.runtime_snapshot().rssi_dbfs, None);
    }

    /// The demand MM emits on cell selection must be a valid, BS-acceptable
    /// U-LOCATION-UPDATE-DEMAND (ITSI attach, no ciphering, no optional fields):
    /// it round-trips through the exact parser the BS uses (cl. 16.9.3.4).
    #[test]
    fn test_registration_demand_on_activate_conf() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();

        mm.rx_activate_conf(&mut q, &activate_conf(true));
        assert_eq!(mm.reg_state, RegState::Registering);
        assert_eq!(mm.attempts, 1);

        let msg = q.pop_front().expect("a demand must be emitted");
        assert!(q.pop_front().is_none(), "exactly one message emitted");
        assert_eq!(msg.sap, Sap::LmmSap);
        assert_eq!(msg.dest, TetraEntity::Mle);
        let SapMsgInner::LmmMleUnitdataReq(mut req) = msg.msg else {
            panic!("expected LmmMleUnitdataReq");
        };
        assert_eq!(req.address.ssi, MS_ISSI);
        assert_eq!(req.sdu.to_bitstr(), "0010011000");

        // The BS parses exactly this on the uplink; it must decode cleanly.
        req.sdu.seek(0);
        let pdu = ULocationUpdateDemand::from_bitbuf(&mut req.sdu).expect("BS must parse the demand");
        assert_eq!(pdu.location_update_type, LocationUpdateType::ItsiAttach);
        assert!(!pdu.request_to_append_la);
        assert!(!pdu.cipher_control);
        assert!(pdu.ciphering_parameters.is_none());
        assert!(pdu.class_of_ms.is_none());
        assert!(pdu.group_identity_location_demand.is_none());
        assert_eq!(req.sdu.get_len_remaining(), 0, "demand fully consumed");
    }

    /// Even with groups configured, the ITSI-attach demand must carry NO group
    /// identity location demand element: bundling would force fragmentation of
    /// the first uplink. Groups are affiliated afterwards via the standalone
    /// procedure (see `test_post_registration_group_attach_sequential`).
    #[test]
    fn test_registration_demand_carries_no_group_attachment() {
        let mut mm = ms_mm_with_groups(&[91, 220]);
        let mut q = MessageQueue::new();

        mm.rx_activate_conf(&mut q, &activate_conf(true));

        let msg = q.pop_front().expect("a demand must be emitted");
        assert!(q.pop_front().is_none(), "exactly one message emitted");
        let SapMsgInner::LmmMleUnitdataReq(mut req) = msg.msg else {
            panic!("expected LmmMleUnitdataReq");
        };
        req.sdu.seek(0);
        let pdu = ULocationUpdateDemand::from_bitbuf(&mut req.sdu).expect("BS must parse the demand");

        assert!(
            pdu.group_identity_location_demand.is_none(),
            "registration demand must not bundle group attachments"
        );
        assert_eq!(req.sdu.get_len_remaining(), 0, "demand fully consumed");
    }

    /// Feature 2 (L3 PDU priority plumbing): the ITSI-attach registration demand
    /// must be handed to the MAC with PDU priority 6 (new-ITSI attach,
    /// ETSI TS 100 392-2 cl. 16, registration) and no emergency flag, so the
    /// random-access permission gate (cl. 23.5.1.4.4) sees the real L3 priority.
    #[test]
    fn test_registration_demand_carries_priority_six() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();

        mm.rx_activate_conf(&mut q, &activate_conf(true));

        let msg = q.pop_front().expect("a demand must be emitted");
        let SapMsgInner::LmmMleUnitdataReq(req) = msg.msg else {
            panic!("expected LmmMleUnitdataReq");
        };
        assert_eq!(req.pdu_priority, Some(6), "ITSI-attach registration is priority 6");
        assert!(!req.is_emergency, "MM signalling is never emergency");
    }

    /// Feature 2: the U-ITSI DETACH sent on de-registration carries PDU priority 3
    /// (cl. 16.6.1) and no emergency flag.
    #[test]
    fn test_detach_carries_priority_three() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();

        mm.rx_activate_conf(&mut q, &activate_conf(true));
        let _ = q.pop_front();
        deliver_dl(&mut mm, &mut q, build_accept());
        assert_eq!(mm.reg_state, RegState::Registered);
        drain_mle_identities(&mut q);

        assert!(mm.begin_deregistration(&mut q));
        let msg = q.pop_front().expect("a U-ITSI DETACH must be emitted");
        let SapMsgInner::LmmMleUnitdataReq(req) = msg.msg else {
            panic!("expected LmmMleUnitdataReq");
        };
        assert_eq!(req.pdu_priority, Some(3), "U-ITSI DETACH is priority 3");
        assert_eq!(req.sdu.to_bitstr(), "00010");
        assert!(!req.is_emergency);
    }

    /// Feature 2: a standalone U-ATTACH/DETACH GROUP IDENTITY carries PDU
    /// priority 3 (cl. 16.8.2) and no emergency flag.
    #[test]
    fn test_group_attach_carries_priority_three() {
        let mut mm = ms_mm_with_groups(&[91, 220]);
        let mut q = MessageQueue::new();

        mm.rx_activate_conf(&mut q, &activate_conf(true));
        let _ = q.pop_front();
        deliver_dl(&mut mm, &mut q, build_accept());
        assert_eq!(mm.reg_state, RegState::Registered);
        drain_mle_identities(&mut q);

        let msg = q.pop_front().expect("a standalone group attach must be emitted");
        let SapMsgInner::LmmMleUnitdataReq(req) = msg.msg else {
            panic!("expected LmmMleUnitdataReq");
        };
        assert_eq!(req.pdu_priority, Some(3), "group attach is priority 3");
        assert_eq!(
            req.sdu.to_bitstr(),
            "011100111000000010000100000100100000000000000000000010110110100000000000000000000110111000"
        );
        assert!(!req.is_emergency);
    }
    /// amendment mode) whose group identity uplink element repeats once per
    /// group (cl. 16.10.27). One acknowledgement completes the whole set.
    #[test]
    fn test_post_registration_group_attach_bundled() {
        let mut mm = ms_mm_with_groups(&[91, 220]);
        let mut q = MessageQueue::new();

        // Register: demand out (no groups), then accept.
        mm.rx_activate_conf(&mut q, &activate_conf(true));
        let _ = q.pop_front().expect("registration demand");
        assert!(q.pop_front().is_none());
        deliver_dl(&mut mm, &mut q, build_accept());
        assert_eq!(mm.reg_state, RegState::Registered);
        // The accept first emits MLE-IDENTITIES, then the standalone attach.
        drain_mle_identities(&mut q);

        // A single attach PDU carrying BOTH groups (amendment mode), one op in
        // flight, and nothing else queued.
        let (issi, pdu) = pop_group_pdu(&mut q);
        assert_eq!(issi, MS_ISSI);
        assert!(!pdu.group_identity_attach_detach_mode, "amendment mode");
        let ul = pdu.group_identity_uplink.expect("group elements present");
        let gssis: Vec<u32> = ul.iter().filter_map(|g| g.gssi).collect();
        assert_eq!(gssis, vec![91, 220], "all configured groups in one PDU");
        assert!(ul.iter().all(|g| g.class_of_usage == Some(GROUP_CLASS_OF_USAGE)));
        assert!(ul.iter().all(|g| g.group_identity_detachment_uplink.is_none()));
        assert!(mm.pending_group_op.is_some());
        assert!(q.pop_front().is_none(), "exactly one attach PDU");

        // One acknowledgement (both groups) completes the operation.
        deliver_dl(&mut mm, &mut q, build_group_ack(&[91, 220], &[]));
        drain_mle_identities(&mut q);
        assert!(mm.pending_group_op.is_none(), "operation complete");
        assert_eq!(mm.attach_groups(), vec![91, 220]);
        assert!(q.pop_front().is_none());
    }

    /// Own MNI for the test config (mcc=901, mnc=9999): (901<<14)|9999.
    const MS_MNI: u64 = (901 << 14) | 9999;

    /// Build a D-LOCATION-UPDATE-COMMAND (cl. 16.9.2.8) with the given group
    /// report request flag and optional MNI address extension. Minimal: no
    /// ciphering, no cell type control, no proprietary element (as this BS
    /// sends).
    fn build_command(group_report: bool, address_extension: Option<u64>) -> BitBuffer {
        let pdu = DLocationUpdateCommand {
            group_identity_report: group_report,
            cipher_control: false,
            ciphering_parameters: None,
            address_extension,
            cell_type_control: None,
            proprietary: None,
        };
        let mut sdu = BitBuffer::new_autoexpand(8);
        pdu.to_bitbuf(&mut sdu).unwrap();
        sdu
    }

    /// A D-LOCATION-UPDATE-COMMAND (infrastructure-initiated registration,
    /// cl. 16.4.3) makes MM send a U-LOCATION-UPDATE-DEMAND of type "demand
    /// location updating" that contains the MNI (address extension), the ISSI
    /// (SSI element) and the class of MS element, and it must round-trip through
    /// the exact parser the BS uses.
    #[test]
    fn test_command_triggers_demand_location_updating() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();

        deliver_dl(&mut mm, &mut q, build_command(false, None));
        assert_eq!(mm.reg_state, RegState::Registering);
        assert_eq!(mm.attempts, 1);

        let msg = q.pop_front().expect("a demand must be emitted");
        assert!(q.pop_front().is_none(), "exactly one message emitted");
        let SapMsgInner::LmmMleUnitdataReq(mut req) = msg.msg else {
            panic!("expected LmmMleUnitdataReq");
        };
        assert_eq!(req.address.ssi, MS_ISSI);

        req.sdu.seek(0);
        let pdu = ULocationUpdateDemand::from_bitbuf(&mut req.sdu).expect("BS must parse the demand");
        assert_eq!(pdu.location_update_type, LocationUpdateType::DemandLocationUpdating);
        assert!(!pdu.request_to_append_la, "cl. 16.4.3: shall not request append LA");
        assert!(pdu.la_information.is_none(), "cl. 16.4.3: LA information shall be absent");
        assert_eq!(pdu.ssi, Some(MS_ISSI as u64), "cl. 16.4.3: ISSI in SSI element");
        assert_eq!(pdu.address_extension, Some(MS_MNI), "cl. 16.4.3: MNI in address extension");
        let class = pdu.class_of_ms.expect("cl. 16.4.3: class of MS shall be present");
        assert!(class.voice, "voice-capable radio");
        assert!(class.e2e_encryption_not_supported, "no E2E encryption");
        assert!(pdu.group_report_response.is_none(), "no group report requested");
        assert!(pdu.group_identity_location_demand.is_none(), "no groups configured");
        assert_eq!(req.sdu.get_len_remaining(), 0, "demand fully consumed");
    }

    /// A command WITH a group report request makes MM regard groups as
    /// un-attached and, since all reported groups fit in this one PDU, include a
    /// "group report response = group report complete" element (cl. 16.4.3 /
    /// 16.10.27a). Configured groups are (re)attached in the same demand.
    #[test]
    fn test_command_with_group_report_request() {
        let mut mm = ms_mm_with_groups(&[91]);
        let mut q = MessageQueue::new();

        deliver_dl(&mut mm, &mut q, build_command(true, Some(MS_MNI)));
        let msg = q.pop_front().expect("a demand must be emitted");
        let SapMsgInner::LmmMleUnitdataReq(mut req) = msg.msg else {
            panic!("expected LmmMleUnitdataReq");
        };
        req.sdu.seek(0);
        let pdu = ULocationUpdateDemand::from_bitbuf(&mut req.sdu).expect("BS must parse the demand");

        let grr = pdu.group_report_response.expect("group report response present");
        assert_eq!(grr.len, 1, "group report response is 1 bit");
        assert_eq!(grr.data, vec![0], "value 0 = group report complete (Table 16.59)");

        let gild = pdu
            .group_identity_location_demand
            .expect("configured group re-attached in the demand");
        assert_eq!(gild.group_identity_attach_detach_mode, 1);
        let gssis: Vec<u32> = gild
            .group_identity_uplink
            .expect("group entries present")
            .iter()
            .filter_map(|g| g.gssi)
            .collect();
        assert_eq!(gssis, vec![91]);
        assert_eq!(req.sdu.get_len_remaining(), 0, "demand fully consumed");
    }

    /// With a group report request but no configured groups, MM sends only the
    /// "group report complete" element and no group identity location demand
    /// (cl. 16.4.3: "If the MS has no groups to attach, it shall send
    /// U-LOCATION UPDATE DEMAND PDU containing a group report response
    /// information element indicating 'group report complete'").
    #[test]
    fn test_command_group_report_no_groups() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();

        deliver_dl(&mut mm, &mut q, build_command(true, None));
        let msg = q.pop_front().expect("a demand must be emitted");
        let SapMsgInner::LmmMleUnitdataReq(mut req) = msg.msg else {
            panic!("expected LmmMleUnitdataReq");
        };
        req.sdu.seek(0);
        let pdu = ULocationUpdateDemand::from_bitbuf(&mut req.sdu).expect("BS must parse the demand");
        assert!(pdu.group_report_response.is_some(), "group report complete sent");
        assert!(pdu.group_identity_location_demand.is_none(), "no groups to attach");
    }

    /// A command whose MNI address extension does not match the MS's own MNI is
    /// not for this MS and shall be ignored (cl. 16.4.3), leaving MM idle.
    #[test]
    fn test_command_wrong_mni_ignored() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();

        deliver_dl(&mut mm, &mut q, build_command(false, Some(MS_MNI ^ 0x1)));
        assert_eq!(mm.reg_state, RegState::Idle, "command for a different MNI ignored");
        assert!(q.pop_front().is_none(), "no demand emitted for a foreign MNI");
    }

    /// A second D-LOCATION-UPDATE-COMMAND arriving while a registration is
    /// already in progress (T351 active) is coalesced: MM does not emit a
    /// duplicate U-LOCATION-UPDATE-DEMAND (cl. 16.4.3 / 22.3.2.3 stop-and-wait).
    #[test]
    fn test_command_coalesced_while_registering() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();

        // First command → registration starts, one demand emitted.
        deliver_dl(&mut mm, &mut q, build_command(false, Some(MS_MNI)));
        assert_eq!(mm.reg_state, RegState::Registering);
        assert!(q.pop_front().is_some(), "first command emits a demand");
        assert!(q.pop_front().is_none(), "exactly one demand for the first command");

        // Second command while still Registering with T351 active → coalesced.
        assert!(mm.t351_countdown > 0, "T351 running after the first command");
        deliver_dl(&mut mm, &mut q, build_command(false, Some(MS_MNI)));
        assert_eq!(mm.reg_state, RegState::Registering, "still registering");
        assert!(q.pop_front().is_none(), "duplicate command coalesced — no second demand queued");
    }

    /// If the serving cell does not require registration, MM stays idle.
    #[test]
    fn test_no_registration_when_not_required() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();
        mm.rx_activate_conf(&mut q, &activate_conf(false));
        assert_eq!(mm.reg_state, RegState::Idle);
        assert!(q.pop_front().is_none(), "no demand when registration not required");
    }

    /// A D-LOCATION-UPDATE-ACCEPT completes the registration and stops retries.
    #[test]
    fn test_registration_accept_completes() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();
        mm.rx_activate_conf(&mut q, &activate_conf(true));
        let _ = q.pop_front();

        deliver_dl(&mut mm, &mut q, build_accept());
        assert_eq!(mm.reg_state, RegState::Registered);
        drain_mle_identities(&mut q);

        // No further retransmission even after the retry window elapses.
        for slot in 0..=T351_TIMEOUT_SLOTS {
            mm.tick_start(&mut q, TdmaTime::from_int(slot as i32));
        }
        assert!(q.pop_front().is_none(), "registered MS must not retransmit");
    }

    /// With no response, MM retransmits the demand after the retry window.
    #[test]
    fn test_registration_retransmits_on_timeout() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();
        mm.rx_activate_conf(&mut q, &activate_conf(true));
        let _ = q.pop_front();
        assert_eq!(mm.attempts, 1);

        // Countdown slots: no retransmit until the tick after T351 reaches zero.
        for slot in 0..=T351_TIMEOUT_SLOTS {
            mm.tick_start(&mut q, TdmaTime::from_int(slot as i32));
        }
        let msg = q.pop_front().expect("demand retransmitted after timeout");
        assert!(matches!(msg.msg, SapMsgInner::LmmMleUnitdataReq(_)));
        assert_eq!(mm.attempts, 2);
    }

    /// Build a D-LOCATION-UPDATE-REJECT with the given raw reject cause.
    fn build_reject(cause: u8) -> BitBuffer {
        let pdu = DLocationUpdateReject {
            location_update_type: LocationUpdateType::ItsiAttach,
            reject_cause: cause,
            cipher_control: false,
            ciphering_parameters: None,
            address_extension: None,
            cell_type_control: None,
            proprietary: None,
        };
        let mut sdu = BitBuffer::new_autoexpand(8);
        pdu.to_bitbuf(&mut sdu).unwrap();
        sdu
    }

    /// A recoverable reject cause (Congestion) makes MM resend the demand
    /// (cl. 16.4.1.1: "may re-try registration"); attempt count increases.
    #[test]
    fn test_reject_recoverable_cause_retries() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();
        mm.rx_activate_conf(&mut q, &activate_conf(true));
        let _ = q.pop_front();
        assert_eq!(mm.attempts, 1);

        deliver_dl(&mut mm, &mut q, build_reject(RejectCause::Congestion as u8));
        // Reject-driven resend is deferred to the next tick (T351 set to 0).
        assert_eq!(mm.reg_state, RegState::Registering);
        mm.tick_start(&mut q, TdmaTime::default());
        let msg = q.pop_front().expect("demand resent after recoverable reject");
        assert!(matches!(msg.msg, SapMsgInner::LmmMleUnitdataReq(_)));
        assert_eq!(mm.attempts, 2);
    }

    /// An LA-rejection cause (LA not allowed) abandons the attempt → Idle,
    /// awaiting a fresh cell (re)selection (cl. 16.4.1.1).
    #[test]
    fn test_reject_la_cause_abandons() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();
        mm.rx_activate_conf(&mut q, &activate_conf(true));
        let _ = q.pop_front();

        deliver_dl(&mut mm, &mut q, build_reject(RejectCause::LaNotAllowed as u8));
        assert_eq!(mm.reg_state, RegState::Idle);
        assert!(!mm.left_system);
        // No resend on the next tick.
        mm.tick_start(&mut q, TdmaTime::default());
        assert!(q.pop_front().is_none());
    }

    /// After N351 "system rejection" causes (Illegal MS) without a successful
    /// registration, the MS leaves the system and ignores further activate-confs
    /// (cl. 16.11.2.1).
    #[test]
    fn test_reject_system_rejection_leaves_after_n351() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();
        for i in 1..=N351_MAX_SYSTEM_REJECTIONS {
            mm.rx_activate_conf(&mut q, &activate_conf(true));
            let _ = q.pop_front();
            deliver_dl(&mut mm, &mut q, build_reject(RejectCause::IllegalMs as u8));
            assert_eq!(mm.system_rejection_count, i);
        }
        assert!(mm.left_system, "MS must leave the system after N351 system rejections");
        // Further cell selection must not trigger a new registration.
        mm.rx_activate_conf(&mut q, &activate_conf(true));
        assert_eq!(mm.reg_state, RegState::Idle);
        assert!(q.pop_front().is_none(), "no registration after leaving the system");
    }

    /// A successful registration clears the accumulated system-rejection count
    /// (cl. 16.11.2.1 counts rejections "without a successful registration").
    #[test]
    fn test_accept_resets_system_rejection_count() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();
        mm.rx_activate_conf(&mut q, &activate_conf(true));
        let _ = q.pop_front();
        deliver_dl(&mut mm, &mut q, build_reject(RejectCause::IllegalMs as u8));
        assert_eq!(mm.system_rejection_count, 1);

        mm.rx_activate_conf(&mut q, &activate_conf(true));
        let _ = q.pop_front();
        deliver_dl(&mut mm, &mut q, build_accept());
        assert_eq!(mm.reg_state, RegState::Registered);
        assert_eq!(mm.system_rejection_count, 0);
    }

    /// On shutdown while registered, MM emits a valid, BS-parseable U-ITSI DETACH
    /// (cl. 16.6.1 / 16.9.3.3) and enters the bounded detach drain.
    #[test]
    fn test_detach_on_shutdown_when_registered() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();

        // Get to the Registered state.
        mm.rx_activate_conf(&mut q, &activate_conf(true));
        let _ = q.pop_front();
        deliver_dl(&mut mm, &mut q, build_accept());
        assert_eq!(mm.reg_state, RegState::Registered);
        drain_mle_identities(&mut q);

        // Shutdown: a detach must be initiated.
        assert!(mm.begin_deregistration(&mut q), "detach must be initiated when registered");
        assert_eq!(mm.reg_state, RegState::Detaching);
        assert!(mm.deregistration_pending());

        let msg = q.pop_front().expect("a U-ITSI DETACH must be emitted");
        assert!(q.pop_front().is_none(), "exactly one message emitted");
        assert_eq!(msg.sap, Sap::LmmSap);
        assert_eq!(msg.dest, TetraEntity::Mle);
        let SapMsgInner::LmmMleUnitdataReq(mut req) = msg.msg else {
            panic!("expected LmmMleUnitdataReq");
        };
        assert_eq!(req.address.ssi, MS_ISSI);
        assert_eq!(req.layer2service, Layer2Service::Acknowledged);

        // The BS parses exactly this on the uplink; it must decode cleanly as a
        // minimal detach (no MNI, no proprietary).
        req.sdu.seek(0);
        let pdu = UItsiDetach::from_bitbuf(&mut req.sdu).expect("BS must parse the detach");
        assert!(pdu.address_extension.is_none());
        assert!(pdu.proprietary.is_none());
        assert_eq!(req.sdu.get_len_remaining(), 0, "detach fully consumed");
    }

    /// The detach drain counts down over `DETACH_DRAIN_SLOTS` ticks, then reports
    /// no longer pending so the router can stop.
    #[test]
    fn test_detach_drain_bounded() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();
        mm.rx_activate_conf(&mut q, &activate_conf(true));
        let _ = q.pop_front();
        deliver_dl(&mut mm, &mut q, build_accept());
        drain_mle_identities(&mut q);
        assert!(mm.begin_deregistration(&mut q));
        let _ = q.pop_front();

        // Pending for the full drain window, then clears.
        for slot in 0..DETACH_DRAIN_SLOTS {
            assert!(mm.deregistration_pending(), "still draining");
            mm.tick_start(&mut q, TdmaTime::from_int(slot as i32));
        }
        assert!(!mm.deregistration_pending(), "drain complete");

        // No accidental uplinks emitted during the drain.
        assert!(q.pop_front().is_none(), "detach drain emits nothing further");
    }

    /// On shutdown while not registered, MM does not attempt a detach.
    #[test]
    fn test_no_detach_when_not_registered() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();
        assert!(!mm.begin_deregistration(&mut q), "no detach when idle");
        assert_eq!(mm.reg_state, RegState::Idle);
        assert!(!mm.deregistration_pending());
        assert!(q.pop_front().is_none(), "no PDU emitted when not registered");
    }

    /// On shutdown while registered but out of service (serving-cell downlink
    /// lost, MLE-BREAK), MM must NOT start a detach drain: the U-ITSI DETACH
    /// cannot be transmitted without a downlink, and the router's drain countdown
    /// (advanced only on recovered downlink slots) would stall — the stack must
    /// stop immediately instead of hanging. Regression for the "stop while not
    /// connected hangs" bug.
    #[test]
    fn test_no_detach_when_registered_but_out_of_service() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();
        mm.rx_activate_conf(&mut q, &activate_conf(true));
        let _ = q.pop_front();
        deliver_dl(&mut mm, &mut q, build_accept());
        assert_eq!(mm.reg_state, RegState::Registered);
        drain_mle_identities(&mut q);

        // Serving-cell downlink is lost: MM goes out of service, stays Registered.
        deliver_mle_break(&mut mm, &mut q);
        assert!(mm.out_of_service);
        assert_eq!(mm.reg_state, RegState::Registered);
        while q.pop_front().is_some() {}

        // Shutdown now must not start a detach and must not leave a pending drain.
        assert!(!mm.begin_deregistration(&mut q), "no detach must be attempted while out of service");
        assert_eq!(mm.reg_state, RegState::Registered, "reg state left unchanged");
        assert!(!mm.deregistration_pending(), "no drain pending — stack can stop at once");
        assert!(q.pop_front().is_none(), "no U-ITSI DETACH emitted with no downlink");
    }

    /// Helper: drive MM to Registered on the default config cell (901/9999, LA 1)
    /// via an ITSI-attach activate-conf + a D-LOCATION-UPDATE-ACCEPT, draining the
    /// emitted demand. Leaves the queue empty.
    fn register_on_default_cell(mm: &mut MmMs, q: &mut MessageQueue) {
        mm.rx_activate_conf(q, &activate_conf(true));
        let _ = q.pop_front().expect("registration demand");
        assert!(q.pop_front().is_none());
        deliver_dl(mm, q, build_accept());
        assert_eq!(mm.reg_state, RegState::Registered);
        drain_mle_identities(q);
        assert!(q.pop_front().is_none());
    }

    /// Drain (and validate) the MLE-IDENTITIES request MM emits after a
    /// successful registration accept (cl. 17.3.2): it carries the MS's own ISSI
    /// and the confirmed attached-group set so the MLE can configure the MAC
    /// downlink address filter (cl. 23.4.1.2.1).
    fn drain_mle_identities(q: &mut MessageQueue) {
        let msg = q.pop_front().expect("MLE-IDENTITIES emitted after registration accept");
        assert_eq!(msg.sap, Sap::LmmSap);
        assert_eq!(msg.dest, TetraEntity::Mle);
        assert!(
            matches!(msg.msg, SapMsgInner::LmmMleIdentitiesReq(_)),
            "expected an MLE-IDENTITIES request"
        );
    }

    /// Helper: pop exactly one U-LOCATION-UPDATE-DEMAND off the queue and return its
    /// location-update type (parsed through the BS's own decoder).
    fn pop_demand_lu_type(q: &mut MessageQueue) -> LocationUpdateType {
        let msg = q.pop_front().expect("a demand must be emitted");
        assert!(q.pop_front().is_none(), "exactly one message emitted");
        assert_eq!(msg.dest, TetraEntity::Mle);
        let SapMsgInner::LmmMleUnitdataReq(mut req) = msg.msg else {
            panic!("expected LmmMleUnitdataReq");
        };
        req.sdu.seek(0);
        ULocationUpdateDemand::from_bitbuf(&mut req.sdu)
            .expect("BS must parse the demand")
            .location_update_type
    }

    /// R1 (cl. 18.3.4.7.1a NOTE 2): re-selecting the SAME cell (same MCC/MNC/LA)
    /// while already registered must NOT trigger a new location update.
    #[test]
    fn test_same_cell_no_reregistration() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();
        register_on_default_cell(&mut mm, &mut q);

        // A repeated activate-conf for the identical cell: no LU, stays Registered.
        mm.rx_activate_conf(&mut q, &activate_conf_cell(true, 901, 9999, 1));
        assert_eq!(mm.reg_state, RegState::Registered);
        assert!(q.pop_front().is_none(), "same-cell return emits no demand");
    }

    /// R1 (cl. 18.3.4.7.1a cond 2): an LA change within the same network, when the
    /// new cell requires registration, triggers a roaming location update.
    #[test]
    fn test_la_change_triggers_roaming_lu() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();
        register_on_default_cell(&mut mm, &mut q);

        mm.rx_activate_conf(&mut q, &activate_conf_cell(true, 901, 9999, 2));
        assert_eq!(mm.reg_state, RegState::Registering);
        assert_eq!(pop_demand_lu_type(&mut q), LocationUpdateType::RoamingLocationUpdating);
    }

    /// R1 (cl. 18.3.4.7.1a cond 1): a network (MCC/MNC) change triggers a migrating
    /// location update regardless of the new cell's registration-required flag.
    #[test]
    fn test_network_change_triggers_migrating_lu() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();
        register_on_default_cell(&mut mm, &mut q);

        mm.rx_activate_conf(&mut q, &activate_conf_cell(false, 902, 1, 5));
        assert_eq!(mm.reg_state, RegState::Registering);
        assert_eq!(pop_demand_lu_type(&mut q), LocationUpdateType::MigratingLocationUpdating);
    }

    /// R1 (cl. 18.3.4.7.1a cond 2): an LA change where the new cell does NOT require
    /// registration must not trigger a location update.
    #[test]
    fn test_la_change_without_registration_required_skips() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();
        register_on_default_cell(&mut mm, &mut q);

        mm.rx_activate_conf(&mut q, &activate_conf_cell(false, 901, 9999, 2));
        assert_eq!(mm.reg_state, RegState::Registered);
        assert!(q.pop_front().is_none(), "no LU when new cell needs no registration");
    }

    /// R1 (cl. 16.4.1.0 / 18.3.4.7.1a): the registered area (LA where registration
    /// completed) is captured on the accept, so a later same-LA return is a no-op.
    #[test]
    fn test_registered_cell_captured_on_accept() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();
        register_on_default_cell(&mut mm, &mut q);

        assert_eq!(
            mm.registered_cell,
            Some(CellId {
                mcc: 901,
                mnc: 9999,
                la: 1
            })
        );
    }

    /// R2 helper: build a D-LOCATION-UPDATE-ACCEPT whose accept-type field carries
    /// the accept-type enum value (Table 16.68).
    fn build_accept_type(raw_type: LocationUpdateAcceptType) -> BitBuffer {
        let pdu = DLocationUpdateAccept {
            location_update_accept_type: raw_type,
            ssi: Some(MS_ISSI as u64),
            address_extension: None,
            subscriber_class: None,
            energy_saving_information: None,
            scch_information_and_distribution_on_18th_frame: None,
            new_registered_area: None,
            security_downlink: None,
            group_identity_location_accept: None,
            default_group_attachment_lifetime: None,
            authentication_downlink: None,
            group_identity_security_related_information: None,
            cell_type_control: None,
            proprietary: None,
        };
        let mut sdu = BitBuffer::new_autoexpand(8);
        pdu.to_bitbuf(&mut sdu).unwrap();
        sdu
    }

    /// R2 (cl. 16.4.8 / Table 16.68): an accept whose accept-type field is raw 0012
    /// means "Temporary registration" — the MS must reinterpret it through the
    /// accept-type table (not the demand table, where raw 1 = migrating) and hold
    /// the temporary-registration state. A normal ITSI-attach accept (raw 3) does
    /// not set the flag.
    #[test]
    fn test_temporary_registration_accept_sets_flag() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();
        mm.rx_activate_conf(&mut q, &activate_conf(true));
        let _ = q.pop_front();
        // Raw 1 on the wire → "Temporary registration" per the accept table.
        deliver_dl(&mut mm, &mut q, build_accept_type(LocationUpdateAcceptType::TemporaryRegistration));
        assert_eq!(mm.reg_state, RegState::Registered);
        assert!(mm.temporary_registration, "raw-1 accept is a temporary registration");

        // A normal ITSI-attach accept (raw 3) leaves the flag clear.
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();
        mm.rx_activate_conf(&mut q, &activate_conf(true));
        let _ = q.pop_front();
        deliver_dl(&mut mm, &mut q, build_accept_type(LocationUpdateAcceptType::ItsiAttach));
        assert!(!mm.temporary_registration, "raw-3 accept is a full registration");
    }

    /// R2 (cl. 16.4.8 para 3 / 16.4.1.0 NOTE): a temporarily-registered MS whose
    /// cell returns to normal mode ("system wide services" restored) performs a
    /// periodic location update on the same cell.
    #[test]
    fn test_temporary_registration_triggers_periodic_on_normal_mode() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();
        mm.rx_activate_conf(&mut q, &activate_conf(true));
        let _ = q.pop_front();
        deliver_dl(&mut mm, &mut q, build_accept_type(LocationUpdateAcceptType::TemporaryRegistration));
        assert!(mm.temporary_registration);
        drain_mle_identities(&mut q);
        assert!(q.pop_front().is_none());

        // Same cell, normal mode (system_wide_services = true): periodic LU.
        mm.rx_activate_conf(&mut q, &activate_conf_cell_sws(true, 901, 9999, 1, true));
        assert_eq!(mm.reg_state, RegState::Registering);
        assert_eq!(pop_demand_lu_type(&mut q), LocationUpdateType::PeriodicLocationUpdating);
    }

    /// R2 (cl. 16.4.1.0 cond. 5): a cell advertising "system wide services
    /// temporarily not supported" (system_wide_services = false) forces
    /// registration on the registered cell even though nothing else changed.
    #[test]
    fn test_system_wide_services_false_forces_registration() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();
        register_on_default_cell(&mut mm, &mut q);
        assert!(!mm.temporary_registration);

        // Same cell, but now restricted: roaming LU required (cond. 5).
        mm.rx_activate_conf(&mut q, &activate_conf_cell_sws(true, 901, 9999, 1, false));
        assert_eq!(mm.reg_state, RegState::Registering);
        assert_eq!(pop_demand_lu_type(&mut q), LocationUpdateType::RoamingLocationUpdating);
    }

    /// R2 (cl. 16.4.1.0 cond. 5): even when the cell does NOT set the registration
    /// flag, a restricted cell (system_wide_services = false) still requires an
    /// initial registration.
    #[test]
    fn test_restricted_cell_forces_initial_registration() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();
        // registration_required = false, but system_wide_services = false.
        mm.rx_activate_conf(&mut q, &activate_conf_cell_sws(false, 901, 9999, 1, false));
        assert_eq!(mm.reg_state, RegState::Registering);
        assert_eq!(pop_demand_lu_type(&mut q), LocationUpdateType::ItsiAttach);
    }

    // --- Phase T0: MS external-interface wiring (enabler; no TNMM behaviour) ---

    /// Build an MS MM wired to a telemetry sink and a control endpoint, and
    /// return the far ends (telemetry source + command dispatcher) so a test can
    /// act as the external UI process.
    fn ms_mm_wired() -> (MmMs, crate::net_telemetry::TelemetrySource, crate::net_control::CommandDispatcher) {
        use crate::net_control::channel::make_control_link;
        use crate::net_telemetry::telemetry_channel;
        let cfg = from_toml_str(MS_TOML).expect("valid MS test config");
        let (sink, source) = telemetry_channel();
        let (dispatcher, endpoint) = make_control_link();
        let mm = MmMs::new(SharedConfig::from_parts(cfg, None), Some(sink), Some(endpoint));
        (mm, source, dispatcher)
    }

    /// T0: `MmMs::new` accepts and stores the telemetry sink + control endpoint
    /// (mirroring `MmBs`), so `build_ms_stack` can wire the external interface.
    #[test]
    fn test_ms_mm_accepts_telemetry_and_control() {
        let (mm, _source, _dispatcher) = ms_mm_wired();
        assert!(mm.telemetry.is_some(), "telemetry sink wired");
        assert!(mm.control.is_some(), "control endpoint wired");
    }

    /// T0: MM drains the control endpoint each tick. No MS control command is
    /// defined yet, so an inbound command must be consumed and dropped without
    /// panicking or altering registration state (cl. 15.3 requests land in T2).
    #[test]
    fn test_ms_mm_drains_control_without_panic() {
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        dispatcher.send(ControlCommand::CommandA { handle: 7, parameter: 1 });
        mm.tick_start(&mut q, TdmaTime::default());

        // Command consumed (nothing left for the endpoint to receive), state
        // unchanged, and no uplink PDU produced as a side effect.
        assert_eq!(mm.reg_state, RegState::Idle, "control command must not change reg state");
        assert!(q.pop_front().is_none(), "no PDU emitted for an unhandled control command");
    }

    // -----------------------------------------------------------------------
    // T1: TNMM-SAP outbound indications (cl. 15.3.3 / 15.3.4). Drive MM state
    // transitions through a wired telemetry sink and assert the exact primitives
    // reach the (simulated) user application.
    // -----------------------------------------------------------------------

    /// Collect all telemetry events currently queued on the source.
    fn drain(source: &crate::net_telemetry::TelemetrySource) -> Vec<TelemetryEvent> {
        let mut out = Vec::new();
        while let Some(e) = source.try_recv() {
            out.push(e);
        }
        out
    }

    /// Successful registration emits, in order: a TNMM-SERVICE "in service
    /// waiting for registration" indication when the demand is sent, then on
    /// D-LOCATION-UPDATE-ACCEPT a TNMM-REGISTRATION indication + confirm
    /// (status = success) and a TNMM-SERVICE "in service" indication
    /// (cl. 15.3.3.7 / 15.3.3.8).
    #[test]
    fn test_accept_emits_registration_success_and_service() {
        use crate::tnmm::{RegistrationStatus, ServiceStatus};
        let (mut mm, source, _dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        mm.rx_activate_conf(&mut q, &activate_conf(true));
        let waiting = drain(&source);
        assert!(
            waiting.iter().any(|e| matches!(
                e,
                TelemetryEvent::TnmmServiceIndication(s)
                    if s.service_status == ServiceStatus::InServiceWaitingForRegistration
            )),
            "demand must emit 'in service waiting for registration'"
        );

        deliver_dl(&mut mm, &mut q, build_accept());
        assert_eq!(mm.reg_state, RegState::Registered);
        let events = drain(&source);

        let reg_ind = events.iter().find_map(|e| match e {
            TelemetryEvent::TnmmRegistrationIndication(i) => Some(i),
            _ => None,
        });
        let reg_ind = reg_ind.expect("TNMM-REGISTRATION indication emitted on accept");
        assert_eq!(reg_ind.registration_status, RegistrationStatus::Success);
        assert_eq!(reg_ind.registration_reject_cause, None);
        assert_eq!(reg_ind.cell_type_where_registered, crate::tnmm::CellType::CaCell);
        assert_eq!(reg_ind.la_where_registered, 1);
        assert_eq!(reg_ind.mcc_where_registered, 901);
        assert_eq!(reg_ind.mnc_where_registered, 9999);

        assert!(
            events.iter().any(|e| matches!(
                e,
                TelemetryEvent::TnmmRegistrationConfirm(c) if c.registration_status == RegistrationStatus::Success
            )),
            "TNMM-REGISTRATION confirm emitted on accept"
        );
        assert!(
            events.iter().any(|e| matches!(
                e,
                TelemetryEvent::TnmmServiceIndication(s) if s.service_status == ServiceStatus::InService
            )),
            "TNMM-SERVICE 'in service' emitted on accept"
        );
    }

    /// `emit_registration_success` builds one `GroupIdentity` per attached GSSI
    /// with GITI = Attachment and GTSI = (own MNI << 24) | GSSI (cl. 15.3.4), and
    /// emits a TNMM-ATTACH DETACH GROUP IDENTITY indication (cl. 15.3.3.1).
    #[test]
    fn test_registration_success_emits_group_identities() {
        use crate::tnmm::GroupIdentityAttachDetachTypeIdentifier as Giti;
        let (mm, source, _dispatcher) = ms_mm_wired();
        let gssi = 100u32;
        let groups =
            GroupAttachments::from_slice(&[tetra_ms_core::mm::GroupAttachment::new(gssi, GROUP_CLASS_OF_USAGE, 3).unwrap()]).unwrap();
        mm.emit_registration_success(&groups);
        let events = drain(&source);

        let group_ind = events.iter().find_map(|e| match e {
            TelemetryEvent::TnmmAttachDetachGroupIdentityIndication(i) => Some(i),
            _ => None,
        });
        let group_ind = group_ind.expect("group identity indication emitted for attached GSSIs");
        assert_eq!(group_ind.group_identities.len(), 1);
        let g = &group_ind.group_identities[0];
        assert_eq!(g.group_identity_attach_detach_type_identifier, Giti::Attachment);
        let expected_gtsi = ((mm.own_mni() as u64) << 24) | (gssi as u64);
        assert_eq!(g.gtsi, expected_gtsi);
        assert!(g.group_identity_lifetime.is_some(), "GITI=Attachment carries lifetime");
        assert!(g.class_of_usage.is_some(), "GITI=Attachment carries class of usage");
        assert!(g.group_identity_detachment_reason.is_none());
    }

    /// A terminal reject (Illegal MS → abandon) emits a TNMM-REGISTRATION
    /// indication with status = failure + the mapped reject cause, then a
    /// TNMM-SERVICE "out of service" indication (cl. 15.3.3.7 / 15.3.4).
    #[test]
    fn test_reject_emits_registration_failure_and_out_of_service() {
        use crate::tnmm::{RegistrationRejectCause, RegistrationStatus, ServiceStatus};
        let (mut mm, source, _dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        mm.rx_activate_conf(&mut q, &activate_conf(true));
        let _ = drain(&source); // discard the "waiting" service indication

        // Illegal MS (cause 2) is an abandon cause (cl. 16.4.1.1): terminal.
        deliver_dl(&mut mm, &mut q, build_reject(2));
        assert_eq!(mm.reg_state, RegState::Idle);
        let events = drain(&source);

        let reg_ind = events.iter().find_map(|e| match e {
            TelemetryEvent::TnmmRegistrationIndication(i) => Some(i),
            _ => None,
        });
        let reg_ind = reg_ind.expect("TNMM-REGISTRATION failure indication emitted on terminal reject");
        assert_eq!(reg_ind.registration_status, RegistrationStatus::Failure);
        assert_eq!(reg_ind.registration_reject_cause, Some(RegistrationRejectCause::IllegalMs));

        assert!(
            events.iter().any(|e| matches!(
                e,
                TelemetryEvent::TnmmServiceIndication(s) if s.service_status == ServiceStatus::OutOfService
            )),
            "TNMM-SERVICE 'out of service' emitted on terminal reject"
        );
    }

    /// A recoverable reject (Congestion → re-try) keeps the registration in
    /// progress, so no failure indication is emitted (cl. 16.4.1.1).
    #[test]
    fn test_recoverable_reject_emits_no_failure() {
        let (mut mm, source, _dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        mm.rx_activate_conf(&mut q, &activate_conf(true));
        let _ = drain(&source);

        // Congestion (cause 6) permits a re-try: still Registering, not terminal.
        deliver_dl(&mut mm, &mut q, build_reject(6));
        let events = drain(&source);
        assert!(
            !events.iter().any(|e| matches!(e, TelemetryEvent::TnmmRegistrationIndication(_))),
            "no failure indication while a re-try is still outstanding"
        );
    }

    /// The on-air reject causes "use CA/DA cell not permitted" (cl. 16.10.42)
    /// have no TNMM-SAP enumerant (cl. 15.3.4): the mapping returns `None`.
    #[test]
    fn test_reject_cause_without_tnmm_enumerant_maps_to_none() {
        assert_eq!(MmMs::map_registration_reject_cause(RejectCause::UseCaCellNotPermitted), None);
        assert_eq!(MmMs::map_registration_reject_cause(RejectCause::UseDaCellNotPermitted), None);
        assert_eq!(
            MmMs::map_registration_reject_cause(RejectCause::IllegalMs),
            Some(crate::tnmm::RegistrationRejectCause::IllegalMs)
        );
    }

    /// De-registration at shutdown emits a TNMM-SERVICE "out of service"
    /// indication (cl. 15.3.3.8).
    #[test]
    fn test_deregistration_emits_out_of_service() {
        use crate::tnmm::ServiceStatus;
        let (mut mm, source, _dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        mm.rx_activate_conf(&mut q, &activate_conf(true));
        deliver_dl(&mut mm, &mut q, build_accept());
        let _ = drain(&source); // discard registration/service events

        assert!(mm.begin_deregistration(&mut q));
        let events = drain(&source);
        assert!(
            events.iter().any(|e| matches!(
                e,
                TelemetryEvent::TnmmServiceIndication(s) if s.service_status == ServiceStatus::OutOfService
            )),
            "de-registration emits 'out of service'"
        );
    }

    // -----------------------------------------------------------------------
    // Service-loss indication (cl. 18.3.3 / 18.3.4.5.3 / 18.3.4.7): MLE-BREAK /
    // MLE-REOPEN over the LMM-SAP drive TNMM-SERVICE without touching reg_state.
    // -----------------------------------------------------------------------

    /// Feed an MLE-BREAK indication (LMM-SAP) into MM.
    fn deliver_mle_break(mm: &mut MmMs, q: &mut MessageQueue) {
        mm.rx_prim(
            q,
            SapMsg {
                sap: Sap::LmmSap,
                src: TetraEntity::Mle,
                dest: TetraEntity::Mm,
                msg: SapMsgInner::LmmMleBreakInd(tetra_saps::lmm::LmmMleBreakInd {}),
            },
        );
    }

    /// Feed an MLE-REOPEN indication (LMM-SAP) into MM.
    fn deliver_mle_reopen(mm: &mut MmMs, q: &mut MessageQueue) {
        mm.rx_prim(
            q,
            SapMsg {
                sap: Sap::LmmSap,
                src: TetraEntity::Mle,
                dest: TetraEntity::Mm,
                msg: SapMsgInner::LmmMleReopenInd(tetra_saps::lmm::LmmMleReopenInd {}),
            },
        );
    }

    /// A registered MS that receives MLE-BREAK declares itself out of service and
    /// emits TNMM-SERVICE(OutOfService), but stays Registered (cl. 18.3.3 — the
    /// registration/service split): the runtime snapshot then shows Registered +
    /// OutOfService.
    #[test]
    fn test_mle_break_emits_out_of_service_keeps_registered() {
        use crate::management::RegistrationState;
        use crate::tnmm::ServiceStatus;
        let (mut mm, source, _dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        register_on_default_cell(&mut mm, &mut q);
        let _ = drain(&source); // discard registration/service events

        deliver_mle_break(&mut mm, &mut q);

        assert_eq!(mm.reg_state, RegState::Registered, "reg state must be unchanged by a link break");
        assert!(mm.out_of_service, "MS must be out of service after MLE-BREAK");

        let events = drain(&source);
        assert!(
            events.iter().any(|e| matches!(
                e,
                TelemetryEvent::TnmmServiceIndication(s) if s.service_status == ServiceStatus::OutOfService
            )),
            "MLE-BREAK emits TNMM-SERVICE 'out of service'"
        );

        let snap = mm.runtime_snapshot();
        assert_eq!(snap.registration_state, RegistrationState::Registered);
        assert_eq!(snap.service_status, ServiceStatus::OutOfService);
    }

    /// MLE-BREAK is idempotent: a second break while already out of service emits
    /// no further TNMM-SERVICE indication.
    #[test]
    fn test_mle_break_idempotent() {
        let (mut mm, source, _dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        register_on_default_cell(&mut mm, &mut q);
        deliver_mle_break(&mut mm, &mut q);
        let _ = drain(&source);

        deliver_mle_break(&mut mm, &mut q);
        assert!(
            drain(&source).is_empty(),
            "a repeated MLE-BREAK must not re-emit a service indication"
        );
    }

    /// MLE-REOPEN restores service: out_of_service clears and MM emits a
    /// TNMM-SERVICE indication reflecting the (still) Registered state, so the
    /// UI sees InService again.
    #[test]
    fn test_mle_reopen_restores_service() {
        use crate::tnmm::ServiceStatus;
        let (mut mm, source, _dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        register_on_default_cell(&mut mm, &mut q);
        deliver_mle_break(&mut mm, &mut q);
        let _ = drain(&source);

        deliver_mle_reopen(&mut mm, &mut q);
        assert!(!mm.out_of_service, "MLE-REOPEN clears out-of-service");
        assert_eq!(mm.reg_state, RegState::Registered);

        let events = drain(&source);
        assert!(
            events.iter().any(|e| matches!(
                e,
                TelemetryEvent::TnmmServiceIndication(s) if s.service_status == ServiceStatus::InService
            )),
            "MLE-REOPEN emits TNMM-SERVICE 'in service' for a registered MS"
        );
    }

    /// MLE-REOPEN without a preceding break is a no-op (idempotent): no service
    /// indication is emitted.
    #[test]
    fn test_mle_reopen_without_break_is_noop() {
        let (mut mm, source, _dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        register_on_default_cell(&mut mm, &mut q);
        let _ = drain(&source);

        deliver_mle_reopen(&mut mm, &mut q);
        assert!(
            drain(&source).is_empty(),
            "MLE-REOPEN with no prior break must not emit a service indication"
        );
    }

    // -----------------------------------------------------------------------
    // T2: TNMM-SAP requests (INBOUND, cl. 15.3.3). Drive requests through the
    // wired control link (dispatcher = UI side) and assert MM's action + the
    // TnmmAck transport response.
    // -----------------------------------------------------------------------

    /// Build the MS's own valid TNMM-REGISTRATION request (matching ITSI/MNI).
    fn own_registration_request() -> crate::tnmm::TnmmRegistrationRequest {
        use crate::tnmm::{RegistrationType, TnmmRegistrationRequest};
        TnmmRegistrationRequest {
            registration_type: RegistrationType::RegistrationToIndicatedCell,
            required_cell_type_list: None,
            preferred_cell_type_list: None,
            preferred_la_list: None,
            preferred_mcc_list: None,
            preferred_mnc_list: None,
            issi: MS_ISSI,
            mcc_of_issi: 901,
            mnc_of_issi: 9999,
            energy_economy_mode: None,
            group_identity_request: None,
            group_identity_attach_detach_mode: None,
        }
    }

    fn last_ack(dispatcher: &crate::net_control::CommandDispatcher) -> (bool, Option<String>) {
        let resps = dispatcher.try_recv_responses();

        resps
            .into_iter()
            .find_map(|r| match r {
                crate::net_control::ControlResponse::TnmmAck { accepted, detail, .. } => Some((accepted, detail)),
                _ => None,
            })
            .expect("a TnmmAck response")
    }

    /// A TNMM-REGISTRATION request (Table 15.5) while idle initiates the ITSI
    /// attach: MM sends a U-LOCATION-UPDATE-DEMAND and moves to Registering, and
    /// acknowledges acceptance (cl. 15.3.3.7).
    #[test]
    fn test_tnmm_registration_request_triggers_demand() {
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        dispatcher.send(ControlCommand::TnmmRegistration {
            handle: 11,
            request: Box::new(own_registration_request()),
        });
        mm.tick_start(&mut q, TdmaTime::default());

        assert_eq!(mm.reg_state, RegState::Registering);
        let msg = q.pop_front().expect("registration demand queued");
        assert!(matches!(msg.msg, SapMsgInner::LmmMleUnitdataReq(_)));
        let (accepted, _detail) = last_ack(&dispatcher);
        assert!(accepted, "registration request accepted");
    }

    /// A TNMM-REGISTRATION request for a different ITSI is rejected (single-ITSI
    /// stack) with no state change.
    #[test]
    fn test_tnmm_registration_request_wrong_itsi_rejected() {
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        let mut req = own_registration_request();
        req.issi = MS_ISSI + 1;
        dispatcher.send(ControlCommand::TnmmRegistration {
            handle: 12,
            request: Box::new(req),
        });
        mm.tick_start(&mut q, TdmaTime::default());

        assert_eq!(mm.reg_state, RegState::Idle, "no registration for a foreign ITSI");
        assert!(q.pop_front().is_none(), "no demand for a foreign ITSI");
        let (accepted, detail) = last_ack(&dispatcher);
        assert!(!accepted);
        assert!(detail.is_some());
    }

    /// A TNMM-DEREGISTRATION request (Table 15.2) while registered runs the
    /// de-registration: MM sends U-ITSI DETACH, moves to Detaching, emits
    /// TNMM-SERVICE "out of service", and acknowledges (cl. 15.3.3.2 / 16.6.1).
    #[test]
    fn test_tnmm_deregistration_request_detaches() {
        use crate::tnmm::{ServiceStatus, TnmmDeregistrationRequest};
        let (mut mm, source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        // Register first.
        mm.rx_activate_conf(&mut q, &activate_conf(true));
        deliver_dl(&mut mm, &mut q, build_accept());
        while q.pop_front().is_some() {}
        let _ = drain(&source);

        dispatcher.send(ControlCommand::TnmmDeregistration {
            handle: 21,
            request: TnmmDeregistrationRequest {
                issi: Some(MS_ISSI),
                mcc: None,
                mnc: None,
            },
        });
        mm.tick_start(&mut q, TdmaTime::default());

        assert_eq!(mm.reg_state, RegState::Detaching);
        let msg = q.pop_front().expect("U-ITSI DETACH queued");
        assert!(matches!(msg.msg, SapMsgInner::LmmMleUnitdataReq(_)));
        let events = drain(&source);
        assert!(
            events.iter().any(|e| matches!(
                e,
                TelemetryEvent::TnmmServiceIndication(s) if s.service_status == ServiceStatus::OutOfService
            )),
            "de-registration emits 'out of service'"
        );
        let (accepted, _detail) = last_ack(&dispatcher);
        assert!(accepted);

        // The bounded drain (cl. 16.6.1) counts down while Detaching. U-ITSI
        // DETACH is unacknowledged (cl. 16.9.3.3): once the drain completes the
        // MS must return to Idle rather than staying stuck in Detaching (which
        // previously rejected any later registration with "de-registration in
        // progress").
        for slot in 1..=DETACH_DRAIN_SLOTS + 1 {
            mm.tick_start(&mut q, TdmaTime::from_int(slot as i32));
        }
        assert_eq!(mm.reg_state, RegState::Idle, "MS returns to Idle after the de-registration drain");

        // A subsequent TNMM-REGISTRATION request is now accepted (was rejected
        // with 'de-registration in progress' while stuck in Detaching).
        dispatcher.send(ControlCommand::TnmmRegistration {
            handle: 23,
            request: Box::new(own_registration_request()),
        });
        mm.tick_start(&mut q, TdmaTime::default());
        assert_eq!(mm.reg_state, RegState::Registering);
        let (accepted, _detail) = last_ack(&dispatcher);
        assert!(accepted, "re-registration accepted after returning to Idle");
    }
    /// acknowledged with a documented detail.
    #[test]
    fn test_tnmm_deregistration_request_when_not_registered() {
        use crate::tnmm::TnmmDeregistrationRequest;
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        dispatcher.send(ControlCommand::TnmmDeregistration {
            handle: 22,
            request: TnmmDeregistrationRequest {
                issi: None,
                mcc: None,
                mnc: None,
            },
        });
        mm.tick_start(&mut q, TdmaTime::default());

        assert_eq!(mm.reg_state, RegState::Idle);
        assert!(q.pop_front().is_none());
        let (accepted, detail) = last_ack(&dispatcher);
        assert!(accepted);
        assert!(detail.is_some(), "documented 'nothing to detach' detail");
    }

    /// Build a D-ATTACH/DETACH GROUP IDENTITY ACKNOWLEDGEMENT (cl. 16.9.2.2) that
    /// accepts the given attached / detached GSSIs, exactly as this BS emits it.
    fn build_group_ack(attached: &[u32], detached: &[u32]) -> BitBuffer {
        build_group_ack_metadata(attached, detached, GROUP_CLASS_OF_USAGE, 3)
    }

    fn build_group_ack_metadata(attached: &[u32], detached: &[u32], class: u8, lifetime: u8) -> BitBuffer {
        use tetra_pdus::mm::fields::group_identity_attachment::GroupIdentityAttachment;
        use tetra_pdus::mm::fields::group_identity_downlink::GroupIdentityDownlink;

        let mut downlink = Vec::new();
        for &g in attached {
            downlink.push(GroupIdentityDownlink {
                group_identity_attachment: Some(GroupIdentityAttachment {
                    group_identity_attachment_lifetime: lifetime,
                    class_of_usage: class,
                }),
                group_identity_detachment_uplink: None,
                gssi: Some(g),
                address_extension: None,
                vgssi: None,
            });
        }
        for &g in detached {
            downlink.push(GroupIdentityDownlink {
                group_identity_attachment: None,
                group_identity_detachment_uplink: Some(0b10),
                gssi: Some(g),
                address_extension: None,
                vgssi: None,
            });
        }
        let pdu = DAttachDetachGroupIdentityAcknowledgement {
            group_identity_accept_reject: 0,
            reserved: false,
            proprietary: None,
            group_identity_downlink: if downlink.is_empty() { None } else { Some(downlink) },
            group_identity_security_related_information: None,
        };
        let mut sdu = BitBuffer::new_autoexpand(8);
        pdu.to_bitbuf(&mut sdu).unwrap();
        sdu
    }

    /// Register a wired MS on the default cell and discard the registration
    /// telemetry / MLE-IDENTITIES so a group-op test starts from a clean queue.
    fn register_wired(mm: &mut MmMs, q: &mut MessageQueue, source: &crate::net_telemetry::TelemetrySource) {
        mm.rx_activate_conf(q, &activate_conf(true));
        let _ = q.pop_front().expect("registration demand");
        deliver_dl(mm, q, build_accept());
        assert_eq!(mm.reg_state, RegState::Registered);
        drain_mle_identities(q);
        assert!(q.pop_front().is_none());
        let _ = drain(source);
    }

    /// Pop the U-ATTACH/DETACH GROUP IDENTITY PDU MM queues and decode it through
    /// the exact parser the BS uses (cl. 16.9.3.1), returning (issi, pdu).
    fn pop_group_pdu(q: &mut MessageQueue) -> (u32, UAttachDetachGroupIdentity) {
        let msg = q.pop_front().expect("group identity PDU queued");
        assert_eq!(msg.dest, TetraEntity::Mle);
        let SapMsgInner::LmmMleUnitdataReq(mut req) = msg.msg else {
            panic!("expected LmmMleUnitdataReq");
        };
        assert_eq!(req.layer2service, Layer2Service::Acknowledged);
        let issi = req.address.ssi;
        req.sdu.seek(0);
        let pdu = UAttachDetachGroupIdentity::from_bitbuf(&mut req.sdu).expect("BS must parse the group PDU");
        assert_eq!(req.sdu.get_len_remaining(), 0, "group PDU fully consumed");
        (issi, pdu)
    }

    fn find_group_confirm(events: &[TelemetryEvent]) -> Option<crate::tnmm::TnmmAttachDetachGroupIdentityConfirm> {
        events.iter().find_map(|e| match e {
            TelemetryEvent::TnmmAttachDetachGroupIdentityConfirm(c) => Some(c.clone()),
            _ => None,
        })
    }

    // -----------------------------------------------------------------------
    // TNMM-REPORT (Table 15.4 / cl. 15.3.3.6): MM shares a transmit receipt with
    // the shutdown U-ITSI DETACH and emits a precise transfer-result indication
    // reflecting the real on-air outcome (cl. 16.6.1), not a fixed timeout bound.
    // -----------------------------------------------------------------------

    /// Pop the queued U-ITSI DETACH and return the transmit receipt MM shares
    /// with it (a clone of `mm.detach_reporter` — driving it drives MM's copy).
    fn take_detach_reporter(q: &mut MessageQueue) -> tetra_core::TxReporter {
        let msg = q.pop_front().expect("U-ITSI DETACH queued");
        let SapMsgInner::LmmMleUnitdataReq(req) = msg.msg else {
            panic!("expected LmmMleUnitdataReq");
        };
        req.tx_reporter.expect("detach carries a shared tx reporter")
    }

    /// Collect the transfer results of every TNMM-REPORT indication emitted.
    fn find_reports(events: &[TelemetryEvent]) -> Vec<crate::tnmm::TransferResult> {
        events
            .iter()
            .filter_map(|e| match e {
                TelemetryEvent::TnmmReportIndication(r) => Some(r.transfer_result),
                _ => None,
            })
            .collect()
    }

    /// Acknowledged detach transfer -> exactly one TNMM-REPORT(SuccessfulDone),
    /// the drain ends early, and no duplicate report is emitted afterwards.
    #[test]
    fn test_detach_report_success_on_acknowledged() {
        use crate::tnmm::TransferResult;
        let (mut mm, source, _d) = ms_mm_wired();
        let mut q = MessageQueue::new();
        register_wired(&mut mm, &mut q, &source);

        assert!(mm.begin_deregistration(&mut q));
        let reporter = take_detach_reporter(&mut q);

        // MAC sends the burst, then the BS acknowledges the acknowledged-mode
        // basic-link transfer (cl. 16.6.1 / 22.3.2.3).
        reporter.mark_transmitted();
        reporter.mark_acknowledged();

        mm.tick_start(&mut q, TdmaTime::default());

        assert_eq!(find_reports(&drain(&source)), vec![TransferResult::TransferSuccessfulDone]);
        assert!(!mm.deregistration_pending(), "drain ended early once the result was known");
        assert_eq!(mm.reg_state, RegState::Idle);

        // Idempotent: the report is emitted exactly once.
        mm.tick_start(&mut q, TdmaTime::default());
        assert!(find_reports(&drain(&source)).is_empty(), "no duplicate TNMM-REPORT");
    }

    /// The MAC discards the detach (congestion) -> TNMM-REPORT(TransferFail),
    /// drain ends early.
    #[test]
    fn test_detach_report_fail_on_discarded() {
        use crate::tnmm::TransferResult;
        let (mut mm, source, _d) = ms_mm_wired();
        let mut q = MessageQueue::new();
        register_wired(&mut mm, &mut q, &source);

        assert!(mm.begin_deregistration(&mut q));
        let reporter = take_detach_reporter(&mut q);
        reporter.mark_discarded();

        mm.tick_start(&mut q, TdmaTime::default());

        assert_eq!(find_reports(&drain(&source)), vec![TransferResult::TransferFail]);
        assert!(!mm.deregistration_pending());
        assert_eq!(mm.reg_state, RegState::Idle);
    }

    /// Transmitted on air but the ack never arrives within the drain window:
    /// best-effort success (cl. 16.6.1 — MM proceeds once the PDU is on air).
    #[test]
    fn test_detach_report_success_on_drain_timeout_after_transmit() {
        use crate::tnmm::TransferResult;
        let (mut mm, source, _d) = ms_mm_wired();
        let mut q = MessageQueue::new();
        register_wired(&mut mm, &mut q, &source);

        assert!(mm.begin_deregistration(&mut q));
        let reporter = take_detach_reporter(&mut q);
        reporter.mark_transmitted(); // on air, ack withheld for the whole drain

        for slot in 0..DETACH_DRAIN_SLOTS {
            mm.tick_start(&mut q, TdmaTime::from_int(slot as i32));
        }

        assert_eq!(find_reports(&drain(&source)), vec![TransferResult::TransferSuccessfulDone]);
        assert!(!mm.deregistration_pending(), "drain complete");
    }

    /// Never transmitted (still Pending at the end of the drain): the detach
    /// never left the MAC -> TNMM-REPORT(TransferFail).
    #[test]
    fn test_detach_report_fail_on_drain_timeout_never_transmitted() {
        use crate::tnmm::TransferResult;
        let (mut mm, source, _d) = ms_mm_wired();
        let mut q = MessageQueue::new();
        register_wired(&mut mm, &mut q, &source);

        assert!(mm.begin_deregistration(&mut q));
        let _reporter = take_detach_reporter(&mut q); // leave it Pending

        for slot in 0..DETACH_DRAIN_SLOTS {
            mm.tick_start(&mut q, TdmaTime::from_int(slot as i32));
        }

        assert_eq!(find_reports(&drain(&source)), vec![TransferResult::TransferFail]);
        assert!(!mm.deregistration_pending());
    }

    /// G2 (cl. 16.8.2): a standalone group attach/detach requested before the MS
    /// is registered is refused (the SwMI must already know the ITSI) — no PDU,
    /// no pending operation.
    #[test]
    fn test_tnmm_group_attach_rejected_when_not_registered() {
        use crate::tnmm::{
            ClassOfUsage, GroupIdentityAttachDetachMode, GroupIdentityAttachDetachTypeIdentifier, GroupIdentityRequest,
            TnmmAttachDetachGroupIdentityRequest,
        };
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        dispatcher.send(ControlCommand::TnmmAttachDetachGroupIdentity {
            handle: 31,
            request: TnmmAttachDetachGroupIdentityRequest {
                group_identity_attach_detach_mode: GroupIdentityAttachDetachMode::Amendment,
                group_identity_request: vec![GroupIdentityRequest {
                    gtsi: 300,
                    group_identity_attach_detach_type_identifier: GroupIdentityAttachDetachTypeIdentifier::Attachment,
                    class_of_usage: Some(ClassOfUsage::ClassOfUsage4),
                    group_identity_detachment_request: None,
                }],
                group_identity_report: None,
            },
        });
        mm.tick_start(&mut q, TdmaTime::default());

        assert!(q.pop_front().is_none(), "no PDU while unregistered");
        assert!(mm.pending_group_op.is_none());
        let (accepted, detail) = last_ack(&dispatcher);
        assert!(!accepted);
        assert!(detail.is_some());
    }

    /// G2 (cl. 16.8.2): once registered, an amendment attach builds a
    /// U-ATTACH/DETACH GROUP IDENTITY PDU (report = not-report-request, mode =
    /// amendment, one Group identity uplink attachment with the class of usage),
    /// which round-trips through the BS parser; T353 is started and the request
    /// is acknowledged accepted.
    #[test]
    fn test_tnmm_group_attach_sends_pdu_and_starts_t353() {
        use crate::tnmm::{
            ClassOfUsage, GroupIdentityAttachDetachMode, GroupIdentityAttachDetachTypeIdentifier, GroupIdentityRequest,
            TnmmAttachDetachGroupIdentityRequest,
        };
        let (mut mm, source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();
        register_wired(&mut mm, &mut q, &source);

        dispatcher.send(ControlCommand::TnmmAttachDetachGroupIdentity {
            handle: 32,
            request: TnmmAttachDetachGroupIdentityRequest {
                group_identity_attach_detach_mode: GroupIdentityAttachDetachMode::Amendment,
                group_identity_request: vec![GroupIdentityRequest {
                    gtsi: 300,
                    group_identity_attach_detach_type_identifier: GroupIdentityAttachDetachTypeIdentifier::Attachment,
                    class_of_usage: Some(ClassOfUsage::ClassOfUsage4),
                    group_identity_detachment_request: None,
                }],
                group_identity_report: None,
            },
        });
        mm.tick_start(&mut q, TdmaTime::default());

        let (issi, pdu) = pop_group_pdu(&mut q);
        assert!(q.pop_front().is_none(), "exactly one PDU emitted");
        assert_eq!(issi, MS_ISSI);
        assert!(!pdu.group_identity_report, "cl. 16.8.2: report = not report request");
        assert!(!pdu.group_identity_attach_detach_mode, "amendment mode");
        assert!(pdu.group_report_response.is_none());
        let ul = pdu.group_identity_uplink.expect("one uplink entry");
        assert_eq!(ul.len(), 1);
        assert_eq!(ul[0].gssi, Some(300));
        // ClassOfUsage4 -> raw 3 (cl. 16.10.6 Table 16.32: "Class of usage N" = N-1).
        assert_eq!(ul[0].class_of_usage, Some(3));
        assert!(ul[0].group_identity_detachment_uplink.is_none());

        let op = mm.pending_group_op.as_ref().expect("operation pending");
        assert!(!op.detach_all);
        // One tick decrement has already run in this tick_start.
        assert_eq!(op.t353_countdown, T353_TIMEOUT_SLOTS - 1);

        let (accepted, _d) = last_ack(&dispatcher);
        assert!(accepted, "request accepted");
    }

    /// G3/G4 (cl. 16.8.2): the D-ATTACH/DETACH GROUP IDENTITY ACKNOWLEDGEMENT
    /// stops T353, reconciles the attached-group set, pushes it to the MLE via
    /// MLE-IDENTITIES, and emits a TNMM-ATTACH DETACH GROUP IDENTITY confirm.
    #[test]
    fn test_group_ack_reconciles_and_confirms() {
        use crate::tnmm::{
            ClassOfUsage, GroupIdentityAttachDetachMode, GroupIdentityAttachDetachTypeIdentifier, GroupIdentityRequest,
            TnmmAttachDetachGroupIdentityRequest,
        };
        let (mut mm, source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();
        register_wired(&mut mm, &mut q, &source);

        dispatcher.send(ControlCommand::TnmmAttachDetachGroupIdentity {
            handle: 33,
            request: TnmmAttachDetachGroupIdentityRequest {
                group_identity_attach_detach_mode: GroupIdentityAttachDetachMode::Amendment,
                group_identity_request: vec![GroupIdentityRequest {
                    gtsi: 300,
                    group_identity_attach_detach_type_identifier: GroupIdentityAttachDetachTypeIdentifier::Attachment,
                    class_of_usage: Some(ClassOfUsage::ClassOfUsage4),
                    group_identity_detachment_request: None,
                }],
                group_identity_report: None,
            },
        });
        mm.tick_start(&mut q, TdmaTime::default());
        let _ = pop_group_pdu(&mut q);
        let _ = drain(&source);

        // SwMI accepts the attachment of GSSI 300.
        deliver_dl(&mut mm, &mut q, build_group_ack(&[300], &[]));

        assert!(mm.attached_gssis.contains(&300), "attached set reconciled");
        assert!(mm.pending_group_op.is_none(), "T353 stopped");
        drain_mle_identities(&mut q);
        assert!(q.pop_front().is_none());

        let confirm = find_group_confirm(&drain(&source)).expect("a group identity confirm");
        assert!(matches!(
            confirm.group_identity_attach_detach_mode,
            GroupIdentityAttachDetachMode::Amendment
        ));
        assert_eq!(confirm.group_identities.len(), 1);
        let gid = &confirm.group_identities[0];
        assert_eq!(gid.gtsi, (MS_MNI << 24) | 300);
        assert!(matches!(
            gid.group_identity_attach_detach_type_identifier,
            GroupIdentityAttachDetachTypeIdentifier::Attachment
        ));
    }

    /// G3 (cl. 16.8.2, amendment): an acknowledged detachment removes the group
    /// from the attached set and the detachment reason is carried on air.
    #[test]
    fn test_group_amendment_detach_removes_group() {
        use crate::tnmm::{
            GroupIdentityAttachDetachMode, GroupIdentityAttachDetachTypeIdentifier, GroupIdentityDetachmentRequest, GroupIdentityRequest,
            TnmmAttachDetachGroupIdentityRequest,
        };
        let (mut mm, source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();
        register_wired(&mut mm, &mut q, &source);
        // Pretend GSSI 91 was previously attached.
        mm.seed_group(91);

        dispatcher.send(ControlCommand::TnmmAttachDetachGroupIdentity {
            handle: 34,
            request: TnmmAttachDetachGroupIdentityRequest {
                group_identity_attach_detach_mode: GroupIdentityAttachDetachMode::Amendment,
                group_identity_request: vec![GroupIdentityRequest {
                    gtsi: 91,
                    group_identity_attach_detach_type_identifier: GroupIdentityAttachDetachTypeIdentifier::Detachment,
                    class_of_usage: None,
                    group_identity_detachment_request: Some(GroupIdentityDetachmentRequest::UserInitiatedDetachment),
                }],
                group_identity_report: None,
            },
        });
        mm.tick_start(&mut q, TdmaTime::default());

        let (_issi, pdu) = pop_group_pdu(&mut q);
        let ul = pdu.group_identity_uplink.expect("one uplink entry");
        assert_eq!(ul[0].gssi, Some(91));
        assert!(ul[0].class_of_usage.is_none());
        // "User initiated" detachment reason (cl. 16.10.21 Table 16.53 = 102).
        assert_eq!(ul[0].group_identity_detachment_uplink, Some(0b10));
        let _ = drain(&source);

        deliver_dl(&mut mm, &mut q, build_group_ack(&[], &[91]));
        assert!(!mm.attached_gssis.contains(&91), "group detached from attached set");
        assert!(mm.pending_group_op.is_none());
    }

    /// G3 (cl. 16.8.2, detach-all): the attached set is replaced by exactly the
    /// acknowledged attachments.
    #[test]
    fn test_group_detach_all_replaces_set() {
        use crate::tnmm::{
            ClassOfUsage, GroupIdentityAttachDetachMode, GroupIdentityAttachDetachTypeIdentifier, GroupIdentityRequest,
            TnmmAttachDetachGroupIdentityRequest,
        };
        let (mut mm, source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();
        register_wired(&mut mm, &mut q, &source);
        mm.seed_group(91);
        mm.seed_group(220);

        dispatcher.send(ControlCommand::TnmmAttachDetachGroupIdentity {
            handle: 35,
            request: TnmmAttachDetachGroupIdentityRequest {
                group_identity_attach_detach_mode: GroupIdentityAttachDetachMode::DetachTheCurrentlyActiveGroupIdentities,
                group_identity_request: vec![GroupIdentityRequest {
                    gtsi: 500,
                    group_identity_attach_detach_type_identifier: GroupIdentityAttachDetachTypeIdentifier::Attachment,
                    class_of_usage: Some(ClassOfUsage::ClassOfUsage4),
                    group_identity_detachment_request: None,
                }],
                group_identity_report: None,
            },
        });
        mm.tick_start(&mut q, TdmaTime::default());
        let (_issi, pdu) = pop_group_pdu(&mut q);
        assert!(pdu.group_identity_attach_detach_mode, "detach-all mode set");
        let _ = drain(&source);

        deliver_dl(&mut mm, &mut q, build_group_ack(&[500], &[]));
        let attached: Vec<u32> = mm.attached_gssis.iter().copied().collect();
        assert_eq!(attached, vec![500], "old groups replaced by the acknowledged set");
    }

    /// G2 (cl. 16.8.2): only one standalone group operation may be outstanding —
    /// a second request while one is pending is refused.
    #[test]
    fn test_second_group_op_rejected_while_pending() {
        use crate::tnmm::{
            ClassOfUsage, GroupIdentityAttachDetachMode, GroupIdentityAttachDetachTypeIdentifier, GroupIdentityRequest,
            TnmmAttachDetachGroupIdentityRequest,
        };
        let (mut mm, source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();
        register_wired(&mut mm, &mut q, &source);

        let mk = |gssi: u64, handle: u32| ControlCommand::TnmmAttachDetachGroupIdentity {
            handle,
            request: TnmmAttachDetachGroupIdentityRequest {
                group_identity_attach_detach_mode: GroupIdentityAttachDetachMode::Amendment,
                group_identity_request: vec![GroupIdentityRequest {
                    gtsi: gssi,
                    group_identity_attach_detach_type_identifier: GroupIdentityAttachDetachTypeIdentifier::Attachment,
                    class_of_usage: Some(ClassOfUsage::ClassOfUsage4),
                    group_identity_detachment_request: None,
                }],
                group_identity_report: None,
            },
        };

        dispatcher.send(mk(300, 36));
        mm.tick_start(&mut q, TdmaTime::default());
        let _ = pop_group_pdu(&mut q);
        let _ = dispatcher.try_recv_responses();

        // Second request while the first is still awaiting its acknowledgement.
        dispatcher.send(mk(301, 37));
        mm.tick_start(&mut q, TdmaTime::default());
        assert!(q.pop_front().is_none(), "no second PDU while one is pending");
        let (accepted, detail) = last_ack(&dispatcher);
        assert!(!accepted);
        assert!(detail.is_some());
    }

    /// G3 (cl. 16.8.5): if T353 expires with no acknowledgement, the operation is
    /// treated as failed — the pending op is cleared and a confirm is emitted.
    #[test]
    fn test_t353_expiry_fails_group_op() {
        use crate::tnmm::{
            ClassOfUsage, GroupIdentityAttachDetachMode, GroupIdentityAttachDetachTypeIdentifier, GroupIdentityRequest,
            TnmmAttachDetachGroupIdentityRequest,
        };
        let (mut mm, source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();
        register_wired(&mut mm, &mut q, &source);

        dispatcher.send(ControlCommand::TnmmAttachDetachGroupIdentity {
            handle: 38,
            request: TnmmAttachDetachGroupIdentityRequest {
                group_identity_attach_detach_mode: GroupIdentityAttachDetachMode::Amendment,
                group_identity_request: vec![GroupIdentityRequest {
                    gtsi: 300,
                    group_identity_attach_detach_type_identifier: GroupIdentityAttachDetachTypeIdentifier::Attachment,
                    class_of_usage: Some(ClassOfUsage::ClassOfUsage4),
                    group_identity_detachment_request: None,
                }],
                group_identity_report: None,
            },
        });
        mm.tick_start(&mut q, TdmaTime::default());
        let _ = pop_group_pdu(&mut q);
        let _ = drain(&source);

        // Drive enough ticks for T353 to expire.
        for slot in 1..(T353_TIMEOUT_SLOTS + 3) {
            mm.tick_start(&mut q, TdmaTime::from_int(slot as i32));
        }

        assert!(mm.pending_group_op.is_none(), "operation cleared on T353 expiry");
        let confirm = find_group_confirm(&drain(&source)).expect("a confirm on failure");
        assert!(confirm.group_identities.is_empty(), "no groups acknowledged on failure");
    }

    /// Dormant primitives (STATUS / ENERGY SAVING requests) are acknowledged as
    /// not-accepted with a documented reason and change no state.
    #[test]
    fn test_tnmm_status_and_energy_saving_requests_are_dormant() {
        use crate::tnmm::{EnergyEconomyMode, TnmmEnergySavingRequest, TnmmStatusRequest};
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        dispatcher.send(ControlCommand::TnmmStatus {
            handle: 41,
            request: TnmmStatusRequest {
                direct_mode: None,
                dual_watch: None,
                energy_economy_mode: None,
            },
        });
        dispatcher.send(ControlCommand::TnmmEnergySaving {
            handle: 42,
            request: TnmmEnergySavingRequest {
                energy_economy_mode: EnergyEconomyMode::EnergyEconomyMode1,
            },
        });
        mm.tick_start(&mut q, TdmaTime::default());

        assert_eq!(mm.reg_state, RegState::Idle);
        let resps = dispatcher.try_recv_responses();
        let acks: Vec<_> = resps
            .into_iter()
            .filter_map(|r| match r {
                crate::net_control::ControlResponse::TnmmAck { handle, accepted, .. } => Some((handle, accepted)),
                _ => None,
            })
            .collect();
        assert_eq!(acks.len(), 2);
        assert!(acks.iter().all(|(_, accepted)| !*accepted), "dormant requests not accepted");
    }

    // -----------------------------------------------------------------------
    // T3: management / provisioning (Plane B, NON-STANDARD). Read-path slice:
    // GetState returns an MS runtime-state snapshot built from MM state + config.
    // -----------------------------------------------------------------------

    /// Extract the last `Management(State{..})` response from a dispatcher.
    fn last_state(dispatcher: &crate::net_control::CommandDispatcher) -> (u32, crate::management::MsRuntimeState) {
        use crate::management::ManagementResponse;
        use crate::net_control::ControlResponse;
        dispatcher
            .try_recv_responses()
            .into_iter()
            .find_map(|r| match r {
                ControlResponse::Management(ManagementResponse::State { handle, state }) => Some((handle, *state)),
                _ => None,
            })
            .expect("a Management State response")
    }

    /// GetState before any cell selection reports Idle / OutOfService and echoes
    /// the configured identity/network/cell fields (non-standard Plane B).
    #[test]
    fn test_management_get_state_idle_snapshot() {
        use crate::management::{ManagementCommand, RegistrationState};
        use crate::tnmm::ServiceStatus;
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        dispatcher.send(ControlCommand::Management(ManagementCommand::GetState { handle: 71 }));
        mm.tick_start(&mut q, TdmaTime::default());

        let (handle, state) = last_state(&dispatcher);
        assert_eq!(handle, 71);
        assert_eq!(state.registration_state, RegistrationState::Idle);
        assert_eq!(state.service_status, ServiceStatus::OutOfService);
        assert_eq!(state.own_issi, 1000001);
        assert_eq!(state.home_mcc, 901);
        assert_eq!(state.home_mnc, 9999);
        assert_eq!(state.serving_la, 1);
        assert_eq!(state.colour_code, 1);
        assert!(state.attached_groups.is_empty());
        // GetState is read-only: registration state must be untouched.
        assert_eq!(mm.reg_state, RegState::Idle);
        assert!(q.pop_front().is_none(), "GetState must not emit any PDU");
    }

    /// After successful registration, GetState reflects Registered / InService.
    #[test]
    fn test_management_get_state_registered_snapshot() {
        use crate::management::{ManagementCommand, RegistrationState};
        use crate::tnmm::ServiceStatus;
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        mm.rx_activate_conf(&mut q, &activate_conf(true));
        deliver_dl(&mut mm, &mut q, build_accept());
        assert_eq!(mm.reg_state, RegState::Registered);
        while q.pop_front().is_some() {}

        dispatcher.send(ControlCommand::Management(ManagementCommand::GetState { handle: 72 }));
        mm.tick_start(&mut q, TdmaTime::default());

        let (_handle, state) = last_state(&dispatcher);
        assert_eq!(state.registration_state, RegistrationState::Registered);
        assert_eq!(state.service_status, ServiceStatus::InService);
    }

    // -----------------------------------------------------------------------
    // Scan lists (Plane B, NON-STANDARD): programmed talkgroup sets that are
    // resolved into the desired group-affiliation superset and activated /
    // deactivated live via `ManagementCommand::ActivateScanlist`. Runtime
    // changes reuse the standalone group attach/detach amendment (cl. 16.8.2).
    // -----------------------------------------------------------------------

    /// Build an MS config whose codeplug carries three talkgroups (100/200/300)
    /// and two scan lists: "Alpha" (100, 200) and "Bravo" (200, 300). `alpha_active`
    /// controls Alpha's programmed-default `active` flag; Bravo is always inactive.
    fn scanlist_toml(alpha_active: bool) -> String {
        format!(
            "{MS_TOML}\n\
[[folder]]\n\
id = \"main\"\n\
name = \"Main\"\n\
order = 1\n\
\n\
[[talkgroup]]\n\
gssi = 100\n\
name = \"TG100\"\n\
folder = \"main\"\n\
order = 1\n\
\n\
[[talkgroup]]\n\
gssi = 200\n\
name = \"TG200\"\n\
folder = \"main\"\n\
order = 2\n\
\n\
[[talkgroup]]\n\
gssi = 300\n\
name = \"TG300\"\n\
folder = \"main\"\n\
order = 3\n\
\n\
[[scanlist]]\n\
name = \"Alpha\"\n\
talkgroups = [100, 200]\n\
active = {alpha_active}\n\
order = 1\n\
\n\
[[scanlist]]\n\
name = \"Bravo\"\n\
talkgroups = [200, 300]\n\
active = false\n\
order = 2\n"
        )
    }

    /// Wired MM built from an arbitrary config TOML (mirrors `ms_mm_wired`).
    fn ms_mm_wired_from(toml: &str) -> (MmMs, crate::net_telemetry::TelemetrySource, crate::net_control::CommandDispatcher) {
        use crate::net_control::channel::make_control_link;
        use crate::net_telemetry::telemetry_channel;
        let cfg = from_toml_str(toml).expect("valid MS test config");
        let (sink, source) = telemetry_channel();
        let (dispatcher, endpoint) = make_control_link();
        let mm = MmMs::new(SharedConfig::from_parts(cfg, None), Some(sink), Some(endpoint));
        (mm, source, dispatcher)
    }

    fn activate_scanlist_cmd(handle: u32, name: &str, active: bool) -> ControlCommand {
        use crate::management::ManagementCommand;
        ControlCommand::Management(ManagementCommand::ActivateScanlist {
            handle,
            name: name.to_string(),
            active,
        })
    }

    /// A scan list whose programmed default is `active` seeds its GSSIs into the
    /// desired affiliation set at construction, so they are affiliated at the
    /// first registration; the runtime snapshot reflects the active-list name.
    #[test]
    fn test_scanlist_default_active_seeded() {
        let cfg = from_toml_str(&scanlist_toml(true)).expect("valid scanlist config");
        let mm = MmMs::new(SharedConfig::from_parts(cfg, None), None, None);

        assert_eq!(mm.attach_groups(), vec![100, 200], "Alpha's GSSIs seeded");
        assert!(mm.active_scanlists.contains("Alpha"));
        assert!(!mm.active_scanlists.contains("Bravo"));

        let snap = mm.runtime_snapshot();
        assert_eq!(snap.active_scanlists, vec!["Alpha".to_string()]);
        assert_eq!(snap.attached_groups, vec![100, 200]);
    }

    /// Activating an unknown scan list is refused (`accepted = false`) with no
    /// state change and no on-air PDU.
    #[test]
    fn test_activate_scanlist_unknown_rejected() {
        let (mut mm, _source, dispatcher) = ms_mm_wired_from(&scanlist_toml(false));
        let mut q = MessageQueue::new();

        dispatcher.send(activate_scanlist_cmd(90, "Ghost", true));
        mm.tick_start(&mut q, TdmaTime::default());

        let (handle, accepted, _restart) = last_mgmt_ack(&dispatcher);
        assert_eq!(handle, 90);
        assert!(!accepted, "unknown scanlist refused");
        assert!(mm.active_scanlists.is_empty());
        assert!(q.pop_front().is_none(), "no PDU emitted");
    }

    /// Activating a scan list while NOT registered updates the desired
    /// affiliation set locally (affiliated at the next registration) and emits no
    /// on-air PDU; the request is acknowledged accepted.
    #[test]
    fn test_activate_scanlist_not_registered_updates_desired_set() {
        let (mut mm, _source, dispatcher) = ms_mm_wired_from(&scanlist_toml(false));
        let mut q = MessageQueue::new();
        assert_eq!(mm.reg_state, RegState::Idle);

        dispatcher.send(activate_scanlist_cmd(91, "Alpha", true));
        mm.tick_start(&mut q, TdmaTime::default());

        let (handle, accepted, _restart) = last_mgmt_ack(&dispatcher);
        assert_eq!(handle, 91);
        assert!(accepted);
        assert!(mm.active_scanlists.contains("Alpha"));
        assert_eq!(mm.attach_groups(), vec![100, 200], "desired set updated locally");
        assert!(mm.pending_group_op.is_none());
        assert!(q.pop_front().is_none(), "no PDU while unregistered");
    }

    /// Activating a scan list while registered applies the delta on air via a
    /// standalone U-ATTACH/DETACH GROUP IDENTITY amendment (cl. 16.8.2) carrying
    /// the to-attach GSSIs; T353 is armed and the request is acknowledged. The
    /// confirmed set is reconciled only from the SwMI acknowledgement (G3), then
    /// deactivating detaches those groups.
    #[test]
    fn test_activate_scanlist_registered_attaches_then_detaches() {
        let (mut mm, source, dispatcher) = ms_mm_wired_from(&scanlist_toml(false));
        let mut q = MessageQueue::new();
        // No default-active scan lists and no base groups, so registration seeds
        // nothing and `register_wired`'s "queue empty" assertion holds.
        register_wired(&mut mm, &mut q, &source);

        // Activate Alpha -> amendment attach of {100, 200}.
        dispatcher.send(activate_scanlist_cmd(92, "Alpha", true));
        mm.tick_start(&mut q, TdmaTime::default());

        let (_issi, pdu) = pop_group_pdu(&mut q);
        assert!(q.pop_front().is_none(), "exactly one PDU emitted");
        assert!(!pdu.group_identity_attach_detach_mode, "amendment mode");
        let ul = pdu.group_identity_uplink.expect("uplink entries");
        let mut attached: Vec<u32> = ul
            .iter()
            .filter(|e| e.group_identity_detachment_uplink.is_none())
            .filter_map(|e| e.gssi)
            .collect();
        attached.sort_unstable();
        assert_eq!(attached, vec![100, 200], "both Alpha GSSIs attached");
        assert!(mm.pending_group_op.is_some(), "T353 armed");
        let (handle, accepted, _restart) = last_mgmt_ack(&dispatcher);
        assert_eq!(handle, 92);
        assert!(accepted);

        // SwMI confirms the attachment -> attached set reconciled from the ACK.
        deliver_dl(&mut mm, &mut q, build_group_ack(&[100, 200], &[]));
        assert!(mm.attached_gssis.contains(&100) && mm.attached_gssis.contains(&200));
        assert!(mm.pending_group_op.is_none());
        drain_mle_identities(&mut q);
        while q.pop_front().is_some() {}
        let _ = drain(&source);

        // Deactivate Alpha -> amendment detach of {100, 200}.
        dispatcher.send(activate_scanlist_cmd(93, "Alpha", false));
        mm.tick_start(&mut q, TdmaTime::default());

        let (_issi, pdu) = pop_group_pdu(&mut q);
        let ul = pdu.group_identity_uplink.expect("uplink entries");
        let mut detached: Vec<u32> = ul
            .iter()
            .filter(|e| e.group_identity_detachment_uplink.is_some())
            .filter_map(|e| e.gssi)
            .collect();
        detached.sort_unstable();
        assert_eq!(detached, vec![100, 200], "both Alpha GSSIs detached");
        assert!(!mm.active_scanlists.contains("Alpha"));

        deliver_dl(&mut mm, &mut q, build_group_ack(&[], &[100, 200]));
        assert!(!mm.attached_gssis.contains(&100) && !mm.attached_gssis.contains(&200));
    }

    /// Re-activating an already-active scan list is an idempotent no-op: accepted,
    /// no PDU, no pending operation.
    #[test]
    fn test_activate_scanlist_idempotent() {
        let (mut mm, source, dispatcher) = ms_mm_wired_from(&scanlist_toml(false));
        let mut q = MessageQueue::new();
        register_wired(&mut mm, &mut q, &source);
        mm.active_scanlists.insert("Alpha".to_string());

        dispatcher.send(activate_scanlist_cmd(94, "Alpha", true));
        mm.tick_start(&mut q, TdmaTime::default());

        let (_handle, accepted, _restart) = last_mgmt_ack(&dispatcher);
        assert!(accepted, "idempotent re-activate accepted");
        assert!(mm.pending_group_op.is_none());
        assert!(q.pop_front().is_none(), "no PDU for a no-op");
    }

    // -----------------------------------------------------------------------
    // Manual cell survey / register-to-cell (Plane B, NON-STANDARD): the four
    // management commands relay internal LMM-SAP requests to the MLE, forward
    // the survey result/complete indications up as telemetry, and CampOnCell's
    // force-register intent is consumed on the next LMM-ACTIVATE confirmation.
    // -----------------------------------------------------------------------

    /// The first LMM request of a given kind forwarded to the MLE, if any.
    fn forwarded_lmm(queue: &MessageQueue) -> Option<&SapMsgInner> {
        queue
            .iter()
            .find(|m| {
                m.dest == TetraEntity::Mle
                    && matches!(
                        m.msg,
                        SapMsgInner::LmmMleSelectionModeReq(_) | SapMsgInner::LmmMleScanReq(_) | SapMsgInner::LmmMleCampReq(_)
                    )
            })
            .map(|m| &m.msg)
    }

    /// Deliver an MLE->MM survey result indication (as MLE forwards it).
    fn deliver_scan_result(mm: &mut MmMs, q: &mut MessageQueue, ind: LmmMleScanResultInd) {
        mm.rx_prim(
            q,
            SapMsg {
                sap: Sap::LmmSap,
                src: TetraEntity::Mle,
                dest: TetraEntity::Mm,
                msg: SapMsgInner::LmmMleScanResultInd(ind),
            },
        );
    }

    /// SetCellSelectionMode relays the mode to the MLE, mirrors it in the runtime
    /// snapshot, and acknowledges the request.
    #[test]
    fn test_set_cell_selection_mode_forwards_and_snapshots() {
        use crate::management::ManagementCommand;
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        dispatcher.send(ControlCommand::Management(ManagementCommand::SetCellSelectionMode {
            handle: 40,
            manual: true,
        }));
        mm.tick_start(&mut q, TdmaTime::default());

        let (handle, accepted, _restart) = last_mgmt_ack(&dispatcher);
        assert_eq!(handle, 40);
        assert!(accepted);
        assert!(mm.runtime_snapshot().selection_mode_manual, "snapshot mirrors manual mode");
        match forwarded_lmm(&q).expect("selection-mode req forwarded to MLE") {
            SapMsgInner::LmmMleSelectionModeReq(r) => assert!(r.manual),
            other => panic!("expected LmmMleSelectionModeReq, got {other:?}"),
        }
    }

    /// StartCellScan / StopCellScan relay an LmmMleScanReq to the MLE with the
    /// matching start flag, and are acknowledged.
    #[test]
    fn test_cell_scan_commands_forward_scan_req() {
        use crate::management::ManagementCommand;
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        dispatcher.send(ControlCommand::Management(ManagementCommand::StartCellScan { handle: 41 }));
        mm.tick_start(&mut q, TdmaTime::default());
        let (_h, accepted, _r) = last_mgmt_ack(&dispatcher);
        assert!(accepted);
        match forwarded_lmm(&q).expect("scan req forwarded") {
            SapMsgInner::LmmMleScanReq(r) => assert!(r.start, "StartCellScan -> start=true"),
            other => panic!("expected LmmMleScanReq, got {other:?}"),
        }

        q = MessageQueue::new();
        dispatcher.send(ControlCommand::Management(ManagementCommand::StopCellScan { handle: 42 }));
        mm.tick_start(&mut q, TdmaTime::default());
        let (_h, accepted, _r) = last_mgmt_ack(&dispatcher);
        assert!(accepted);
        match forwarded_lmm(&q).expect("scan req forwarded") {
            SapMsgInner::LmmMleScanReq(r) => assert!(!r.start, "StopCellScan -> start=false"),
            other => panic!("expected LmmMleScanReq, got {other:?}"),
        }
    }

    /// The four manual-selection commands are serviceable before registration
    /// (the survey is receive-only and must work while camped-less/unregistered).
    #[test]
    fn test_cell_scan_serviceable_while_unregistered() {
        use crate::management::ManagementCommand;
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();
        assert_eq!(mm.reg_state, RegState::Idle);

        dispatcher.send(ControlCommand::Management(ManagementCommand::StartCellScan { handle: 43 }));
        mm.tick_start(&mut q, TdmaTime::default());

        let (_h, accepted, _r) = last_mgmt_ack(&dispatcher);
        assert!(accepted, "scan accepted while unregistered");
        assert!(forwarded_lmm(&q).is_some(), "scan req forwarded even when Idle");
    }

    /// Entering manual cell selection while registered must first cleanly
    /// de-register from the SwMI (U-ITSI DETACH, cl. 16.6.1): a registration is
    /// bound to the serving cell and manual selection abandons automatic
    /// (re)selection. Manual mode keeps the current camp, so the detach can
    /// still be transmitted over the serving-cell link during the drain, and the
    /// mode is still relayed to the MLE.
    #[test]
    fn test_manual_selection_deregisters_when_registered() {
        use crate::management::ManagementCommand;
        let (mut mm, source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();
        register_wired(&mut mm, &mut q, &source);

        dispatcher.send(ControlCommand::Management(ManagementCommand::SetCellSelectionMode {
            handle: 50,
            manual: true,
        }));
        mm.tick_start(&mut q, TdmaTime::default());

        let (_h, accepted, _r) = last_mgmt_ack(&dispatcher);
        assert!(accepted);
        assert_eq!(mm.reg_state, RegState::Detaching, "manual mode while registered de-registers");
        assert!(mm.deregistration_pending());

        // A U-ITSI DETACH is emitted (parseable by the BS) and the manual mode is
        // still relayed to the MLE.
        let mut saw_detach = false;
        let mut saw_manual = false;
        for m in q.iter() {
            if m.dest != TetraEntity::Mle {
                continue;
            }
            match &m.msg {
                SapMsgInner::LmmMleUnitdataReq(req) => {
                    let mut sdu = req.sdu.clone();
                    sdu.seek(0);
                    assert!(UItsiDetach::from_bitbuf(&mut sdu).is_ok(), "detach must parse");
                    saw_detach = true;
                }
                SapMsgInner::LmmMleSelectionModeReq(r) if r.manual => saw_manual = true,
                _ => {}
            }
        }
        assert!(saw_detach, "U-ITSI DETACH emitted when entering manual mode registered");
        assert!(saw_manual, "manual selection still relayed to the MLE");
        assert!(mm.runtime_snapshot().selection_mode_manual, "snapshot mirrors manual mode");
    }

    /// Switching to manual selection while NOT registered simply relays the mode
    /// (no detach): there is nothing to de-register.
    #[test]
    fn test_manual_selection_no_detach_when_unregistered() {
        use crate::management::ManagementCommand;
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();
        assert_eq!(mm.reg_state, RegState::Idle);

        dispatcher.send(ControlCommand::Management(ManagementCommand::SetCellSelectionMode {
            handle: 52,
            manual: true,
        }));
        mm.tick_start(&mut q, TdmaTime::default());

        assert_eq!(mm.reg_state, RegState::Idle, "no de-registration while unregistered");
        assert!(
            !q.iter().any(|m| matches!(m.msg, SapMsgInner::LmmMleUnitdataReq(_))),
            "no U-ITSI DETACH emitted while unregistered"
        );
        match forwarded_lmm(&q).expect("selection-mode req forwarded") {
            SapMsgInner::LmmMleSelectionModeReq(r) => assert!(r.manual),
            other => panic!("expected LmmMleSelectionModeReq, got {other:?}"),
        }
    }

    /// Starting a receive-only survey while registered first de-registers
    /// (U-ITSI DETACH, cl. 16.6.1) and DEFERS the survey until that detach drains
    /// - the survey tunes away from the serving cell, so the detach must reach
    ///   the air first. Once the drain completes the deferred survey is launched.
    #[test]
    fn test_scan_while_registered_deregisters_then_starts_survey() {
        use crate::management::ManagementCommand;
        let (mut mm, source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();
        register_wired(&mut mm, &mut q, &source);

        dispatcher.send(ControlCommand::Management(ManagementCommand::StartCellScan { handle: 51 }));
        mm.tick_start(&mut q, TdmaTime::default());

        let (_h, accepted, _r) = last_mgmt_ack(&dispatcher);
        assert!(accepted);
        assert_eq!(mm.reg_state, RegState::Detaching, "scan while registered first de-registers");
        assert!(mm.pending_scan_after_detach, "survey deferred until detach drains");
        assert!(
            !q.iter().any(|m| matches!(m.msg, SapMsgInner::LmmMleScanReq(_))),
            "survey not started before the detach drains"
        );

        // The BS acknowledges the U-ITSI DETACH: the drain ends and the deferred
        // survey is launched on the completing tick.
        let reporter = take_detach_reporter(&mut q);
        reporter.mark_transmitted();
        reporter.mark_acknowledged();
        let mut q2 = MessageQueue::new();
        mm.tick_start(&mut q2, TdmaTime::from_int(1));

        assert_eq!(mm.reg_state, RegState::Idle);
        assert!(!mm.pending_scan_after_detach, "deferred-survey flag cleared once launched");
        match forwarded_lmm(&q2).expect("deferred survey started after detach") {
            SapMsgInner::LmmMleScanReq(r) => assert!(r.start, "deferred survey starts (start=true)"),
            other => panic!("expected LmmMleScanReq, got {other:?}"),
        }
    }

    /// Cancelling a survey (`StopCellScan`) that was deferred behind a
    /// de-registration drops the pending survey so it is not launched when the
    /// detach completes.
    #[test]
    fn test_stop_scan_clears_deferred_survey() {
        use crate::management::ManagementCommand;
        let (mut mm, source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();
        register_wired(&mut mm, &mut q, &source);

        dispatcher.send(ControlCommand::Management(ManagementCommand::StartCellScan { handle: 53 }));
        mm.tick_start(&mut q, TdmaTime::default());
        assert!(mm.pending_scan_after_detach, "survey deferred");

        // Operator cancels before the detach drains.
        let reporter = take_detach_reporter(&mut q);
        let mut q2 = MessageQueue::new();
        dispatcher.send(ControlCommand::Management(ManagementCommand::StopCellScan { handle: 54 }));
        mm.tick_start(&mut q2, TdmaTime::from_int(1));
        assert!(!mm.pending_scan_after_detach, "deferred survey cancelled");

        // Complete the detach: no survey is launched.
        reporter.mark_transmitted();
        reporter.mark_acknowledged();
        let mut q3 = MessageQueue::new();
        mm.tick_start(&mut q3, TdmaTime::from_int(2));
        assert_eq!(mm.reg_state, RegState::Idle);
        assert!(
            !q3.iter().any(|m| matches!(&m.msg, SapMsgInner::LmmMleScanReq(r) if r.start)),
            "no survey launched after a cancelled deferral"
        );
    }

    /// CampOnCell relays an LmmMleCampReq to the MLE with the chosen carrier +
    /// register flag, records the operator's force-register intent, and acks.
    #[test]
    fn test_camp_on_cell_forwards_and_records_force_register() {
        use crate::management::ManagementCommand;
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        dispatcher.send(ControlCommand::Management(ManagementCommand::CampOnCell {
            handle: 44,
            carrier_hz: 396_000_000,
            register: true,
        }));
        mm.tick_start(&mut q, TdmaTime::default());

        let (handle, accepted, _r) = last_mgmt_ack(&dispatcher);
        assert_eq!(handle, 44);
        assert!(accepted);
        assert!(mm.camp_force_register, "force-register intent recorded for the next activate-conf");
        match forwarded_lmm(&q).expect("camp req forwarded") {
            SapMsgInner::LmmMleCampReq(r) => {
                assert_eq!(r.carrier_hz, 396_000_000);
                assert!(r.register);
            }
            other => panic!("expected LmmMleCampReq, got {other:?}"),
        }
    }

    /// A survey result/complete indication from the MLE is forwarded to the UI as
    /// `MsScanResult` / `MsScanComplete` telemetry (MM only relays; the MLE parsed
    /// the cell per spec).
    #[test]
    fn test_scan_result_and_complete_emit_telemetry() {
        let (mut mm, source, _dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        deliver_scan_result(
            &mut mm,
            &mut q,
            LmmMleScanResultInd {
                carrier_hz: 390_000_000,
                mcc: 901,
                mnc: 9999,
                location_area: Some(1),
                colour_code: None,
                rssi_dbfs: Some(-72.0),
                registration_required: Some(true),
                late_entry_supported: true,
            },
        );
        mm.rx_prim(
            &mut q,
            SapMsg {
                sap: Sap::LmmSap,
                src: TetraEntity::Mle,
                dest: TetraEntity::Mm,
                msg: SapMsgInner::LmmMleScanCompleteInd(LmmMleScanCompleteInd { found: 1, scanned: 2 }),
            },
        );

        let events = drain(&source);
        let result = events
            .iter()
            .find_map(|e| match e {
                TelemetryEvent::MsScanResult {
                    carrier_hz,
                    mcc,
                    mnc,
                    location_area,
                    registration_required,
                    ..
                } => Some((*carrier_hz, *mcc, *mnc, *location_area, *registration_required)),
                _ => None,
            })
            .expect("MsScanResult telemetry emitted");
        assert_eq!(result, (390_000_000, 901, 9999, Some(1), Some(true)));

        let complete = events
            .iter()
            .find_map(|e| match e {
                TelemetryEvent::MsScanComplete { found, scanned } => Some((*found, *scanned)),
                _ => None,
            })
            .expect("MsScanComplete telemetry emitted");
        assert_eq!(complete, (1, 2));
    }

    /// Register-to-cell (decision #2): with the operator's force-register intent
    /// set, MM performs an ITSI attach on the next LMM-ACTIVATE confirmation even
    /// when the cell advertises registration-not-required.
    #[test]
    fn test_camp_force_register_forces_lu_when_not_required() {
        let mut mm = ms_mm();
        let mut q = MessageQueue::new();

        // Operator armed "register on camp"; MM would normally skip an LU on a
        // cell that does not require registration.
        mm.camp_force_register = true;
        mm.rx_activate_conf(&mut q, &activate_conf(false));

        assert_eq!(mm.reg_state, RegState::Registering, "forced ITSI attach despite reg-not-required");
        let msg = q.pop_front().expect("a demand must be emitted");
        let SapMsgInner::LmmMleUnitdataReq(mut req) = msg.msg else {
            panic!("expected LmmMleUnitdataReq");
        };
        req.sdu.seek(0);
        let pdu = ULocationUpdateDemand::from_bitbuf(&mut req.sdu).expect("BS must parse the forced demand");
        assert_eq!(pdu.location_update_type, LocationUpdateType::ItsiAttach);
        assert!(!mm.camp_force_register, "force-register intent consumed once");
    }

    // -----------------------------------------------------------------------
    // T3: management / provisioning (Plane B, NON-STANDARD). Write-path slice:
    // GetConfig / SetConfig (validate+persist) / ApplyConfig (drain+restart).
    // -----------------------------------------------------------------------

    /// Extract the last `Management(Config{..})` response from a dispatcher.
    fn last_config(dispatcher: &crate::net_control::CommandDispatcher) -> (u32, String) {
        use crate::management::ManagementResponse;
        use crate::net_control::ControlResponse;
        dispatcher
            .try_recv_responses()
            .into_iter()
            .find_map(|r| match r {
                ControlResponse::Management(ManagementResponse::Config { handle, toml }) => Some((handle, toml)),
                _ => None,
            })
            .expect("a Management Config response")
    }

    /// Extract the last `Management(Ack{..})` response from a dispatcher.
    fn last_mgmt_ack(dispatcher: &crate::net_control::CommandDispatcher) -> (u32, bool, bool) {
        use crate::management::ManagementResponse;
        use crate::net_control::ControlResponse;
        dispatcher
            .try_recv_responses()
            .into_iter()
            .find_map(|r| match r {
                ControlResponse::Management(ManagementResponse::Ack {
                    handle,
                    accepted,
                    restart_required,
                    ..
                }) => Some((handle, accepted, restart_required)),
                _ => None,
            })
            .expect("a Management Ack response")
    }

    /// GetInterfaceVersion returns the frozen schema-version string (live).
    #[test]
    fn test_management_get_interface_version() {
        use crate::management::{MS_INTERFACE_SCHEMA_VERSION, ManagementCommand, ManagementResponse};
        use crate::net_control::ControlResponse;
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        dispatcher.send(ControlCommand::Management(ManagementCommand::GetInterfaceVersion { handle: 90 }));
        mm.tick_start(&mut q, TdmaTime::default());

        let (handle, version) = dispatcher
            .try_recv_responses()
            .into_iter()
            .find_map(|r| match r {
                ControlResponse::Management(ManagementResponse::InterfaceVersion { handle, version }) => Some((handle, version)),
                _ => None,
            })
            .expect("an InterfaceVersion response");
        assert_eq!(handle, 90);
        assert_eq!(version, MS_INTERFACE_SCHEMA_VERSION);
        assert_eq!(mm.reg_state, RegState::Idle);
    }

    /// GetConfig returns canonical TOML that re-parses through the validator.
    #[test]
    fn test_management_get_config_roundtrips() {
        use crate::management::ManagementCommand;
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        dispatcher.send(ControlCommand::Management(ManagementCommand::GetConfig { handle: 80 }));
        mm.tick_start(&mut q, TdmaTime::default());

        let (handle, toml) = last_config(&dispatcher);
        assert_eq!(handle, 80);
        let reparsed = from_toml_str(&toml).expect("GetConfig TOML must re-parse");
        assert_eq!(reparsed.net.mcc, 901);
        assert_eq!(reparsed.net.mnc, 9999);
        assert_eq!(reparsed.ms.as_ref().unwrap().issi, 1000001);
        // Read-only: no PDU emitted, state untouched.
        assert!(q.pop_front().is_none());
        assert_eq!(mm.reg_state, RegState::Idle);
    }

    /// SetConfig without an installed management context is refused gracefully
    /// (accepted = false), never panics, and does not touch state.
    #[test]
    fn test_management_set_config_without_context_refused() {
        use crate::management::ManagementCommand;
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        dispatcher.send(ControlCommand::Management(ManagementCommand::SetConfig {
            handle: 81,
            toml: MS_TOML.to_string(),
        }));
        mm.tick_start(&mut q, TdmaTime::default());

        let (handle, accepted, restart_required) = last_mgmt_ack(&dispatcher);
        assert_eq!(handle, 81);
        assert!(!accepted, "no context => refused");
        assert!(!restart_required);
        assert_eq!(mm.reg_state, RegState::Idle);
    }

    /// SetConfig with a context validates + persists to disk and flags a pending
    /// restart; the runtime snapshot then reports `restart_required = true`.
    #[test]
    fn test_management_set_config_persists_and_flags_restart() {
        use crate::management::ManagementCommand;
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        let path = std::env::temp_dir().join(format!("tetra-ms-setcfg-{}.toml", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let restart_requested = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let is_running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        mm.set_management_context(path.clone(), restart_requested.clone(), is_running.clone());

        dispatcher.send(ControlCommand::Management(ManagementCommand::SetConfig {
            handle: 82,
            // A STRUCTURAL change ([ms].issi) must still require a restart.
            toml: MS_TOML.replace("issi = 1000001", "issi = 1000002"),
        }));
        mm.tick_start(&mut q, TdmaTime::default());

        let (handle, accepted, restart_required) = last_mgmt_ack(&dispatcher);
        assert_eq!(handle, 82);
        assert!(accepted, "valid config accepted");
        assert!(restart_required, "restart flagged after staging");
        // File written and re-parses.
        let written = std::fs::read_to_string(&path).expect("config file written");
        assert!(from_toml_str(&written).is_ok(), "persisted config must re-parse");
        // SetConfig must NOT bounce the process.
        assert!(
            is_running.load(std::sync::atomic::Ordering::SeqCst),
            "SetConfig must not stop the loop"
        );
        assert!(
            !restart_requested.load(std::sync::atomic::Ordering::SeqCst),
            "SetConfig must not request restart"
        );
        // Snapshot reflects the pending restart.
        let snap = mm.runtime_snapshot();
        assert!(snap.restart_required);

        let _ = std::fs::remove_file(&path);
    }

    /// SetConfig with a codeplug-only (operational) change applies LIVE: it is
    /// accepted, `restart_required` stays false, the process is not bounced, and
    /// the running config immediately reflects the new codeplug data.
    #[test]
    fn test_management_set_config_codeplug_applies_live() {
        use crate::management::ManagementCommand;
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        let path = std::env::temp_dir().join(format!("tetra-ms-livecfg-{}.toml", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let restart_requested = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let is_running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        mm.set_management_context(path.clone(), restart_requested.clone(), is_running.clone());

        // Sanity: the running config has no contacts yet.
        assert!(mm.config.config().codeplug.contacts.is_empty());

        // A codeplug-only change: add a contact. Everything structural is
        // identical to the running config.
        let toml = format!("{MS_TOML}\n[[contact]]\nname = \"Dispatch\"\nissi = 4242\n");
        dispatcher.send(ControlCommand::Management(ManagementCommand::SetConfig { handle: 90, toml }));
        mm.tick_start(&mut q, TdmaTime::default());

        let (handle, accepted, restart_required) = last_mgmt_ack(&dispatcher);
        assert_eq!(handle, 90);
        assert!(accepted, "codeplug change accepted");
        assert!(!restart_required, "codeplug change applies live — no restart");
        // Process must NOT be bounced.
        assert!(
            is_running.load(std::sync::atomic::Ordering::SeqCst),
            "live apply must not stop the loop"
        );
        assert!(
            !restart_requested.load(std::sync::atomic::Ordering::SeqCst),
            "live apply must not request restart"
        );
        // The running config now reflects the new contact — live, no restart.
        let cfg = mm.config.config();
        assert_eq!(cfg.codeplug.contacts.len(), 1, "contact applied live");
        assert_eq!(
            cfg.codeplug.contacts[0].target,
            tetra_config::magentastation::CfgContactTarget::Issi(4242)
        );
        // Snapshot still shows no pending restart.
        assert!(!mm.runtime_snapshot().restart_required);

        let _ = std::fs::remove_file(&path);
    }
    #[test]
    fn test_management_set_config_rejects_invalid() {
        use crate::management::ManagementCommand;
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        let path = std::env::temp_dir().join(format!("tetra-ms-badcfg-{}.toml", uuid::Uuid::new_v4()));
        let restart_requested = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let is_running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        mm.set_management_context(path.clone(), restart_requested, is_running);

        let credential_section = "\n[command]\nhost = \"127.0.0.1\"\nport = 9102\nusername = \"test\"\n";
        let invalid = [
            format!("{MS_TOML}{credential_section}password = \"credential-log-test-secret\n"),
            format!("{MS_TOML}{credential_section}password = [\"credential-log-test-secret\"]\n"),
            MS_TOML.replace("stack_mode = \"Ms\"", "stack_mode = \"credential-log-test-secret\""),
            MS_TOML.replace("config_version = \"0.6\"", "config_version = \"credential-log-test-secret\""),
        ];
        for level in [tracing::Level::WARN, tracing::Level::DEBUG] {
            for (index, toml) in invalid.iter().enumerate() {
                let logs = crate::test_utils::capture_logs(level, || {
                    dispatcher.send(ControlCommand::Management(ManagementCommand::SetConfig {
                        handle: 83,
                        toml: toml.clone(),
                    }));
                    mm.tick_start(&mut q, TdmaTime::default());
                });
                assert!(logs.contains("WARN"));
                assert!(logs.contains("SetConfig rejected invalid configuration"));
                assert!(!logs.contains("credential-log-test-secret"));
                let response = dispatcher.try_recv_responses().into_iter().next().expect("rejection response");
                let crate::net_control::ControlResponse::Management(crate::management::ManagementResponse::Ack {
                    handle,
                    accepted,
                    restart_required,
                    message,
                }) = response
                else {
                    panic!("expected management acknowledgement");
                };
                assert_eq!(handle, 83);
                assert!(!accepted, "invalid config rejected");
                assert!(!restart_required);
                assert!(!message.contains("credential-log-test-secret"));
                if index < 3 {
                    assert!(message.contains("invalid TOML"));
                    assert!(message.contains("line "));
                    assert!(message.contains("column "));
                } else {
                    assert!(message.contains("configuration validation failed"));
                }
            }
        }
        assert!(!path.exists(), "no file written for rejected config");
        assert!(!mm.runtime_snapshot().restart_required);
    }

    // -----------------------------------------------------------------------
    // Offline (pre-synchronization) management servicing: config read/staging
    // and read-only state must work as soon as the control link is up, before
    // the MS has synced/registered. Driven via `drive_offline_control` (what
    // `MessageRouter::run_stack_ms` calls on every iteration that recovers no
    // downlink slot), NOT `tick_start`.
    // -----------------------------------------------------------------------

    /// GetConfig is answered while unsynchronized (no tick): the full active
    /// codeplug is returned and re-parses through the startup validator. This
    /// is the fix for "GetConfig returns empty before registration" — the
    /// command was simply never serviced until the stack started ticking.
    #[test]
    fn test_offline_get_config_before_sync() {
        use crate::management::ManagementCommand;
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        dispatcher.send(ControlCommand::Management(ManagementCommand::GetConfig { handle: 200 }));
        // No slot recovered => router services only the offline subset.
        mm.drive_offline_control(&mut q);

        let (handle, toml) = last_config(&dispatcher);
        assert_eq!(handle, 200);
        let reparsed = from_toml_str(&toml).expect("offline GetConfig TOML must re-parse");
        assert_eq!(reparsed.net.mcc, 901);
        assert_eq!(reparsed.net.mnc, 9999);
        assert_eq!(reparsed.ms.as_ref().unwrap().issi, 1000001);
        // Purely a read: never touches registration state or emits a PDU.
        assert_eq!(mm.reg_state, RegState::Idle);
        assert!(q.pop_front().is_none());
    }

    /// SetConfig validates + persists while unsynchronized and flags a pending
    /// restart, exactly as it does on a real tick. The staged file re-parses;
    /// the process is not bounced.
    #[test]
    fn test_offline_set_config_persists_before_sync() {
        use crate::management::ManagementCommand;
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        let path = std::env::temp_dir().join(format!("tetra-ms-offcfg-{}.toml", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let restart_requested = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let is_running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        mm.set_management_context(path.clone(), restart_requested.clone(), is_running.clone());

        dispatcher.send(ControlCommand::Management(ManagementCommand::SetConfig {
            handle: 201,
            // A STRUCTURAL change ([ms].issi) still stages + flags a restart.
            toml: MS_TOML.replace("issi = 1000001", "issi = 1000002"),
        }));
        mm.drive_offline_control(&mut q);

        let (handle, accepted, restart_required) = last_mgmt_ack(&dispatcher);
        assert_eq!(handle, 201);
        assert!(accepted, "offline SetConfig accepted while unregistered");
        assert!(restart_required, "restart flagged after offline staging");
        let written = std::fs::read_to_string(&path).expect("offline SetConfig persisted");
        assert!(from_toml_str(&written).is_ok(), "persisted config must re-parse");
        assert!(
            is_running.load(std::sync::atomic::Ordering::SeqCst),
            "SetConfig must not stop the loop"
        );
        assert!(
            !restart_requested.load(std::sync::atomic::Ordering::SeqCst),
            "SetConfig must not request restart"
        );
        assert_eq!(mm.reg_state, RegState::Idle);

        // A subsequent offline GetConfig is still serviced (returns the live
        // active config; staged changes apply on the next ApplyConfig restart).
        dispatcher.send(ControlCommand::Management(ManagementCommand::GetConfig { handle: 202 }));
        mm.drive_offline_control(&mut q);
        let (h2, toml2) = last_config(&dispatcher);
        assert_eq!(h2, 202);
        assert!(from_toml_str(&toml2).is_ok());
        assert!(mm.runtime_snapshot().restart_required, "snapshot reflects the pending restart");

        let _ = std::fs::remove_file(&path);
    }

    /// GetConfig offline still redacts secrets on the wire (no regression in the
    /// ******** sentinel handling for the pre-sync path).
    #[test]
    fn test_offline_get_config_redacts_secret() {
        use crate::management::ManagementCommand;
        use tetra_config::magentastation::REDACTED_SECRET;
        let (mut mm, _source, dispatcher) = ms_mm_wired_from(MS_TOML_WITH_SECRET);
        let mut q = MessageQueue::new();

        dispatcher.send(ControlCommand::Management(ManagementCommand::GetConfig { handle: 203 }));
        mm.drive_offline_control(&mut q);

        let (_handle, toml) = last_config(&dispatcher);
        assert!(!toml.contains("supersecret"), "offline GetConfig must not leak the password");
        assert!(toml.contains(REDACTED_SECRET), "offline GetConfig carries the redaction sentinel");
    }

    /// Non-management commands received while unsynchronized are NOT acted upon
    /// offline: a TNMM-REGISTRATION request is buffered and only takes effect on
    /// the first real tick, so registration / on-air behaviour is unchanged by
    /// the offline config-service path. GetConfig sent alongside is still served.
    #[test]
    fn test_offline_defers_tnmm_registration_until_tick() {
        use crate::management::ManagementCommand;
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        // Both arrive before synchronization.
        dispatcher.send(ControlCommand::Management(ManagementCommand::GetConfig { handle: 204 }));
        dispatcher.send(ControlCommand::TnmmRegistration {
            handle: 205,
            request: Box::new(own_registration_request()),
        });
        mm.drive_offline_control(&mut q);

        // GetConfig serviced offline; registration deferred (no state change, no
        // demand emitted, no ack yet).
        let (h, _toml) = last_config(&dispatcher);
        assert_eq!(h, 204);
        assert_eq!(mm.reg_state, RegState::Idle, "registration must not run offline");
        assert!(q.pop_front().is_none(), "no registration demand emitted offline");
        assert_eq!(mm.deferred_control.len(), 1, "registration buffered for the first tick");

        // First real tick replays the buffered registration exactly as if it had
        // just arrived: MS transitions to Registering and queues the demand.
        mm.tick_start(&mut q, TdmaTime::default());
        assert_eq!(mm.reg_state, RegState::Registering, "deferred registration acted on at first tick");
        let msg = q.pop_front().expect("registration demand queued on first tick");
        assert!(matches!(msg.msg, SapMsgInner::LmmMleUnitdataReq(_)));
        assert!(mm.deferred_control.is_empty(), "deferred buffer drained");
    }

    // An MS config that additionally configures a control endpoint with HTTP
    // Basic credentials, so the secret redaction/restore paths carry a secret.
    const MS_TOML_WITH_SECRET: &str = r#"
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

[command]
host = "127.0.0.1"
port = 9000
username = "ui-operator"
password = "supersecret"
"#;

    fn ms_mm_wired_with_secret() -> (MmMs, crate::net_telemetry::TelemetrySource, crate::net_control::CommandDispatcher) {
        use crate::net_control::channel::make_control_link;
        use crate::net_telemetry::telemetry_channel;
        let cfg = from_toml_str(MS_TOML_WITH_SECRET).expect("valid MS test config with secret");
        let (sink, source) = telemetry_channel();
        let (dispatcher, endpoint) = make_control_link();
        let mm = MmMs::new(SharedConfig::from_parts(cfg, None), Some(sink), Some(endpoint));
        (mm, source, dispatcher)
    }

    /// End-to-end secret handling (Plane B, non-standard):
    /// - GetConfig redacts the control password on the wire (never leaks it).
    /// - SetConfig with the redacted document back preserves the real on-disk
    ///   secret (a benign round-trip must not clobber the credential).
    /// - SetConfig with a genuinely new password overwrites it.
    #[test]
    fn test_management_config_redacts_and_preserves_secret() {
        use crate::management::ManagementCommand;
        use tetra_config::magentastation::REDACTED_SECRET;

        let (mut mm, _source, dispatcher) = ms_mm_wired_with_secret();
        let mut q = MessageQueue::new();

        let path = std::env::temp_dir().join(format!("tetra-ms-secret-{}.toml", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let restart_requested = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let is_running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        mm.set_management_context(path.clone(), restart_requested, is_running);

        // 1) GetConfig must redact the secret on the wire.
        dispatcher.send(ControlCommand::Management(ManagementCommand::GetConfig { handle: 84 }));
        mm.tick_start(&mut q, TdmaTime::default());
        let (_h, wire_toml) = last_config(&dispatcher);
        assert!(!wire_toml.contains("supersecret"), "GetConfig must not leak the password");
        assert!(wire_toml.contains(REDACTED_SECRET), "GetConfig carries the redaction sentinel");
        assert!(wire_toml.contains("ui-operator"), "non-secret username still travels");

        // 2) UI posts the redacted document back unchanged => on-disk secret preserved.
        dispatcher.send(ControlCommand::Management(ManagementCommand::SetConfig {
            handle: 85,
            toml: wire_toml.clone(),
        }));
        mm.tick_start(&mut q, TdmaTime::default());
        let (_h, accepted, _rr) = last_mgmt_ack(&dispatcher);
        assert!(accepted, "redacted round-trip config accepted");
        let written = std::fs::read_to_string(&path).expect("config written");
        assert!(written.contains("supersecret"), "real secret preserved on disk");
        assert!(!written.contains(REDACTED_SECRET), "sentinel never persisted to disk");

        // 3) A genuinely new password is written through.
        let rotated = wire_toml.replace(REDACTED_SECRET, "rotated-pass");
        dispatcher.send(ControlCommand::Management(ManagementCommand::SetConfig {
            handle: 86,
            toml: rotated,
        }));
        mm.tick_start(&mut q, TdmaTime::default());
        let (_h, accepted, _rr) = last_mgmt_ack(&dispatcher);
        assert!(accepted, "rotated config accepted");
        let written = std::fs::read_to_string(&path).expect("config written");
        assert!(written.contains("rotated-pass"), "new secret written to disk");
        assert!(!written.contains("supersecret"), "old secret replaced");

        let _ = std::fs::remove_file(&path);
    }

    /// ApplyConfig requests the controlled restart: sets the shared
    /// `restart_requested` flag and clears `is_running` so `main` exits with the
    /// restart code, and (when registered) initiates the detach drain.
    #[test]
    fn test_management_apply_config_requests_restart() {
        use crate::management::ManagementCommand;
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        // Register first so ApplyConfig exercises the detach drain.
        mm.rx_activate_conf(&mut q, &activate_conf(true));
        deliver_dl(&mut mm, &mut q, build_accept());
        assert_eq!(mm.reg_state, RegState::Registered);
        while q.pop_front().is_some() {}

        let path = std::env::temp_dir().join(format!("tetra-ms-apply-{}.toml", std::process::id()));
        let restart_requested = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let is_running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        mm.set_management_context(path, restart_requested.clone(), is_running.clone());

        // A structural change is staged and pending a restart.
        mm.restart_required = true;

        dispatcher.send(ControlCommand::Management(ManagementCommand::ApplyConfig { handle: 84 }));
        mm.tick_start(&mut q, TdmaTime::default());

        let (handle, accepted, _restart_required) = last_mgmt_ack(&dispatcher);
        assert_eq!(handle, 84);
        assert!(accepted, "apply accepted");
        assert!(restart_requested.load(std::sync::atomic::Ordering::SeqCst), "restart requested");
        assert!(!is_running.load(std::sync::atomic::Ordering::SeqCst), "run loop stopped");
        // Registered MS begins de-registration (U-ITSI DETACH) on apply.
        assert_eq!(mm.reg_state, RegState::Detaching);
    }

    /// ApplyConfig is a success no-op when nothing structural is staged
    /// (`restart_required == false`): it must NOT bounce the radio. This is the
    /// UI's common "Save then Apply" flow after a codeplug/operational-only edit
    /// that already took effect live.
    #[test]
    fn test_management_apply_config_noop_when_nothing_staged() {
        use crate::management::ManagementCommand;
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        mm.rx_activate_conf(&mut q, &activate_conf(true));
        deliver_dl(&mut mm, &mut q, build_accept());
        assert_eq!(mm.reg_state, RegState::Registered);
        while q.pop_front().is_some() {}

        let path = std::env::temp_dir().join(format!("tetra-ms-apply-noop-{}.toml", std::process::id()));
        let restart_requested = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let is_running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        mm.set_management_context(path, restart_requested.clone(), is_running.clone());

        // Nothing staged.
        assert!(!mm.restart_required);

        dispatcher.send(ControlCommand::Management(ManagementCommand::ApplyConfig { handle: 87 }));
        mm.tick_start(&mut q, TdmaTime::default());

        let (handle, accepted, restart_required) = last_mgmt_ack(&dispatcher);
        assert_eq!(handle, 87);
        assert!(accepted, "no-op apply is accepted");
        assert!(!restart_required);
        // Radio must NOT be bounced and must NOT de-register.
        assert!(
            !restart_requested.load(std::sync::atomic::Ordering::SeqCst),
            "no restart requested when nothing staged"
        );
        assert!(is_running.load(std::sync::atomic::Ordering::SeqCst), "run loop keeps running");
        assert_eq!(mm.reg_state, RegState::Registered, "no de-registration on no-op apply");
    }

    /// ApplyConfig without a management context is refused gracefully.
    #[test]
    fn test_management_apply_config_without_context_refused() {
        use crate::management::ManagementCommand;
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();

        dispatcher.send(ControlCommand::Management(ManagementCommand::ApplyConfig { handle: 85 }));
        mm.tick_start(&mut q, TdmaTime::default());

        let (handle, accepted, _r) = last_mgmt_ack(&dispatcher);
        assert_eq!(handle, 85);
        assert!(!accepted, "no context => refused");
        assert_eq!(mm.reg_state, RegState::Idle);
    }

    /// T4 (mock-transport CI): a management command survives a full JSON
    /// encode -> decode -> MmMs handle -> response encode -> decode loop, proving
    /// the wire path is portable end to end over the mock control transport.
    #[test]
    fn test_management_end_to_end_over_json_codec() {
        use crate::management::{ManagementCommand, ManagementResponse};
        use crate::net_control::ControlResponse;
        use crate::net_control::codec::ControlCodecJson;
        let (mut mm, _source, dispatcher) = ms_mm_wired();
        let mut q = MessageQueue::new();
        let codec = ControlCodecJson;

        // UI side: build a command, serialize it, and (mock transport) deserialize.
        let wire = codec.encode_command(&ControlCommand::Management(ManagementCommand::GetInterfaceVersion { handle: 7 }));
        let cmd = codec.decode_command(&wire).expect("decode command");
        dispatcher.send(cmd);

        // Stack side: MM handles it and emits a response.
        mm.tick_start(&mut q, TdmaTime::default());

        // Response travels back over the mock transport: serialize + deserialize.
        let resp = dispatcher.try_recv_responses().into_iter().next().expect("a response");
        let round = codec.decode_response(&codec.encode_response(&resp)).expect("decode response");
        let ControlResponse::Management(ManagementResponse::InterfaceVersion { handle, version }) = round else {
            panic!("expected InterfaceVersion response");
        };
        assert_eq!(handle, 7);
        assert_eq!(version, crate::management::MS_INTERFACE_SCHEMA_VERSION);
    }
}
